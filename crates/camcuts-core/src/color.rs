//! Conversiones de color de cámara LOG / HDR a Rec.709 sobre cuadros BGR de 8 bits (port de `color.py`).
//!
//! Tubería de las transformaciones de cámara (todo precalculado):
//!  1. Deshacer la matriz Y'CbCr del decodificador si no es la del archivo (3x3 de 8 bits).
//!  2. Tabla de 256 `f32` por canal: valor de 8 bits → código de cámara → luz lineal de escena
//!     (curva inversa publicada por el fabricante; 0.18 = gris 18 %, 0.9 = blanco 90 %).
//!  3. Una matriz 3x3: gama de la cámara → primarios Rec.709 (todo D65).
//!  4. Tabla de 65536 entradas sobre la luz lineal cuantizada: hombro suave en altas luces y OETF Rec.709.
//!
//! Fuentes de fórmulas y matrices: las mismas que en `color.py` (Sony, Panasonic, Canon, Apple,
//! ITU-R BT.2100/BT.709/BT.2020); las constantes están copiadas tal cual.
//!
//! Cómo llegan los cuadros: el sampler usa ffmpeg, que respeta las etiquetas del archivo (rango completo
//! en los Sony "pc", la matriz etiquetada o BT.601 sin etiqueta). [`decoding_for`] devuelve los parámetros.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, bail, Result};
use rayon::prelude::*;

use crate::image;
use crate::metrics;

/// Identificadores de transformación y su nombre en la interfaz (mismo orden que Python).
pub const TRANSFORMS: &[(&str, &str)] = &[
    ("none", "Ninguna (tal cual)"),
    ("auto", "Automático (estirar contraste)"),
    ("slog3_sgamut3cine", "Sony S-Log3 / S-Gamut3.Cine → Rec.709"),
    ("slog3_sgamut3", "Sony S-Log3 / S-Gamut3 → Rec.709"),
    ("slog2_sgamut", "Sony S-Log2 / S-Gamut → Rec.709"),
    ("hlg", "HLG / Rec.2020 (Sony HLG, iPhone HDR) → Rec.709"),
    ("vlog", "Panasonic V-Log / V-Gamut → Rec.709"),
    ("clog3", "Canon C-Log3 / Cinema Gamut → Rec.709"),
    ("applelog", "Apple Log → Rec.709"),
    ("cube", "LUT .cube del usuario…"),
];

/// Nivel lineal donde empieza el hombro de altas luces (blanco 90 % = 0.9).
pub const SHOULDER_START: f64 = 0.6;
/// Luz lineal que cubre la tabla de salida (~6.5 pasos sobre el gris); por encima → 255.
pub const LINEAR_MAX: f64 = 16.0;
const OUT_SCALE: f64 = 65535.0 / LINEAR_MAX;

// ------------------------------------------------------------------------------------------------
// Curvas inversas: valor en el dominio del fabricante → luz lineal de escena.
// Dominio "code" = código de 10 bits / 1023; dominio "signal" = (código - 64) / 876.
// ------------------------------------------------------------------------------------------------

/// Sony S-Log3 (Technical Summary V1.00).
pub fn slog3_to_linear(c: f64) -> f64 {
    let cv = c * 1023.0;
    if cv >= 171.2102946929 {
        10f64.powf((cv - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
    } else {
        (cv - 95.0) * 0.01125000 / (171.2102946929 - 95.0)
    }
}

/// Sony S-Log2 (S-Log2 Technical Paper): S-Log sobre x*155/219, reflectancia = 0.9 * entrada.
pub fn slog2_to_linear(c: f64) -> f64 {
    let y = (c * 1023.0 - 64.0) / 876.0;
    let x = if y >= 0.030001222851889303 {
        10f64.powf((y - 0.616596 - 0.03) / 0.432699) - 0.037584
    } else {
        (y - 0.030001222851889303) / 5.0
    };
    0.9 * x * 219.0 / 155.0
}

/// Panasonic V-Log (VARICAM V-Log/V-Gamut, 2014).
pub fn vlog_to_linear(v: f64) -> f64 {
    if v < 0.181 { (v - 0.125) / 5.6 } else { 10f64.powf((v - 0.598206) / 0.241514) - 0.00873 }
}

/// Canon Log 3 (Input Transform 202007, constantes v1.2 sobre código normalizado); reflectancia = 0.9 x.
pub fn clog3_to_linear(v: f64) -> f64 {
    let x = if v < 0.097465473 {
        -(10f64.powf((0.12783901 - v) / 0.36726845) - 1.0) / 14.98325
    } else if v <= 0.15277891 {
        (v - 0.12512219) / 1.9754798
    } else {
        (10f64.powf((v - 0.12240537) / 0.36726845) - 1.0) / 14.98325
    };
    0.9 * x
}

const AL_R0: f64 = -0.05641088;
const AL_RT: f64 = 0.01;
const AL_C: f64 = 47.28711236;
const AL_B: f64 = 0.00964052;
const AL_G: f64 = 0.08550479;
const AL_D: f64 = 0.69336945;

/// Apple Log (Apple Log Profile White Paper, 2023).
pub fn applelog_to_linear(p: f64) -> f64 {
    let pt = AL_C * (AL_RT - AL_R0).powi(2);
    if p >= pt {
        2f64.powf((p - AL_D) / AL_G) - AL_B
    } else if p >= 0.0 {
        (p.max(0.0) / AL_C).sqrt() + AL_R0
    } else {
        AL_R0
    }
}

const HLG_A: f64 = 0.17883277;

/// ITU-R BT.2100 HLG, OETF inversa: señal E' en [0,1] → luz de escena E en [0,1].
fn hlg_inverse_oetf(e: f64) -> f64 {
    let b = 1.0 - 4.0 * HLG_A;
    let c = 0.5 - HLG_A * (4.0 * HLG_A).ln();
    let e = e.max(0.0);
    if e <= 0.5 { e * e / 3.0 } else { (((e - c) / HLG_A).exp() + b) / 12.0 }
}

/// HLG escalado para que el blanco de referencia BT.2408 (75 % HLG) sea 1.0 (gris 38 % → 0.18).
pub fn hlg_to_linear(e: f64) -> f64 {
    hlg_inverse_oetf(e) * (1.0 / hlg_inverse_oetf(0.75))
}

// ------------------------------------------------------------------------------------------------
// Gamas (RGB → XYZ)
// ------------------------------------------------------------------------------------------------

type M3 = [[f64; 3]; 3];

fn matmul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn inv3(m: &M3) -> M3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |i: usize, j: usize| {
        let r: Vec<usize> = (0..3).filter(|&x| x != i).collect();
        let k: Vec<usize> = (0..3).filter(|&x| x != j).collect();
        let minor = m[r[0]][k[0]] * m[r[1]][k[1]] - m[r[0]][k[1]] * m[r[1]][k[0]];
        if (i + j) % 2 == 0 { minor } else { -minor }
    };
    std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det))
}

/// Resuelve P·x = w para un 3x3 (eliminación de Gauss con pivote parcial, como `np.linalg.solve`).
fn solve3(p: &M3, w: [f64; 3]) -> [f64; 3] {
    let mut a = [[0f64; 4]; 3];
    for i in 0..3 {
        a[i][..3].copy_from_slice(&p[i]);
        a[i][3] = w[i];
    }
    for col in 0..3 {
        let piv = (col..3).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs())).unwrap();
        a.swap(col, piv);
        for r in col + 1..3 {
            let f = a[r][col] / a[col][col];
            for k in col..4 {
                a[r][k] -= f * a[col][k];
            }
        }
    }
    let mut x = [0f64; 3];
    for i in (0..3).rev() {
        let s: f64 = (i + 1..3).map(|k| a[i][k] * x[k]).sum();
        x[i] = (a[i][3] - s) / a[i][i];
    }
    x
}

const D65: (f64, f64) = (0.3127, 0.3290);

/// Matriz primaria normalizada RGB→XYZ (SMPTE RP 177).
fn npm(prim: [(f64, f64); 3]) -> M3 {
    let pt: M3 = std::array::from_fn(|i| {
        let (x, y) = prim[i];
        [x / y, 1.0, (1.0 - x - y) / y]
    });
    // P = filas de primarios traspuestas
    let p: M3 = std::array::from_fn(|i| std::array::from_fn(|j| pt[j][i]));
    let w = [D65.0 / D65.1, 1.0, (1.0 - D65.0 - D65.1) / D65.1];
    let s = solve3(&p, w);
    std::array::from_fn(|i| std::array::from_fn(|j| p[i][j] * s[j]))
}

fn to_xyz(gamut: &str) -> M3 {
    match gamut {
        // Sony S-Gamut3.Cine → XYZ (S-Gamut3_S-Gamut3Cine_Matrix.xlsx)
        "sgamut3cine" => [[0.5990839208, 0.2489255161, 0.1024464902], [0.2150758201, 0.8850685017, -0.1001443219], [-0.0320658495, -0.0276583907, 1.1487819910]],
        // Sony S-Gamut3 (y S-Gamut, mismos primarios) → XYZ
        "sgamut3" => [[0.7064827132, 0.1288010498, 0.1151721641], [0.2709796708, 0.7866064112, -0.0575860820], [-0.0096778454, 0.0046000375, 1.0941355587]],
        // Panasonic V-Gamut → XYZ
        "vgamut" => [[0.679644, 0.152211, 0.118600], [0.260686, 0.774894, -0.035580], [-0.009310, -0.004612, 1.102980]],
        "cinemagamut" => npm([(0.740, 0.270), (0.170, 1.140), (0.080, -0.100)]),
        "rec2020" => npm([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)]),
        _ => npm([(0.640, 0.330), (0.300, 0.600), (0.150, 0.060)]), // rec709
    }
}

/// RGB lineal (gama) → RGB lineal Rec.709, filas renormalizadas para que el neutro siga neutro.
pub fn gamut_to_709(gamut: &str) -> M3 {
    let m = matmul(&inv3(&to_xyz("rec709")), &to_xyz(gamut));
    std::array::from_fn(|i| {
        let s: f64 = m[i].iter().sum();
        std::array::from_fn(|j| m[i][j] / s)
    })
}

/// Datos de cada transformación de cámara: curva, dominio, gama y matriz Y'CbCr con la que codifica.
fn camera(tid: &str) -> Option<(fn(f64) -> f64, &'static str, &'static str, &'static str)> {
    Some(match tid {
        "slog3_sgamut3cine" => (slog3_to_linear, "code", "sgamut3cine", "bt709"),
        "slog3_sgamut3" => (slog3_to_linear, "code", "sgamut3", "bt709"),
        "slog2_sgamut" => (slog2_to_linear, "code", "sgamut3", "bt709"),
        "hlg" => (hlg_to_linear, "signal", "rec2020", "bt2020"),
        "vlog" => (vlog_to_linear, "code", "vgamut", "bt709"),
        "clog3" => (clog3_to_linear, "code", "cinemagamut", "bt709"),
        "applelog" => (applelog_to_linear, "signal", "rec2020", "bt2020"),
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------------------
// Decodificador
// ------------------------------------------------------------------------------------------------

fn krb(matrix: &str) -> Option<(f64, f64)> {
    match matrix {
        "bt601" => Some((0.299, 0.114)),
        "bt709" => Some((0.2126, 0.0722)),
        "bt2020" => Some((0.2627, 0.0593)),
        _ => None,
    }
}

fn rgb_to_ycc(matrix: &str) -> M3 {
    let (kr, kb) = krb(matrix).unwrap();
    let kg = 1.0 - kr - kb;
    let y = [kr, kg, kb];
    [
        y,
        std::array::from_fn(|i| ([0.0, 0.0, 1.0][i] - y[i]) / (2.0 * (1.0 - kb))),
        std::array::from_fn(|i| ([1.0, 0.0, 0.0][i] - y[i]) / (2.0 * (1.0 - kr))),
    ]
}

/// Cambia el orden RGB ↔ BGR de una matriz (REV · M · REV).
fn rev(m: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| m[2 - i][2 - j]))
}

/// 3x3 BGR que convierte el R'G'B' decodificado con la matriz `decoded` en el R'G'B' real del archivo.
fn matrix_fix(decoded: &str, source: &str) -> Option<[[f32; 3]; 3]> {
    if decoded == source {
        return None;
    }
    let m = matmul(&inv3(&rgb_to_ycc(source)), &rgb_to_ycc(decoded));
    let r = rev(&m);
    Some(std::array::from_fn(|i| std::array::from_fn(|j| r[i][j] as f32)))
}

/// Para cada valor decodificado de 8 bits: (código/1023, señal de rango de video (código-64)/876).
fn domain_values(decoded_range: &str) -> (Vec<f64>, Vec<f64>) {
    let v: Vec<f64> = (0..256).map(|i| i as f64 / 255.0).collect();
    if decoded_range == "full" {
        let sig = v.iter().map(|x| (x * 1023.0 - 64.0) / 876.0).collect();
        (v, sig)
    } else {
        let code = v.iter().map(|x| (64.0 + 876.0 * x) / 1023.0).collect();
        (code, v)
    }
}

// ------------------------------------------------------------------------------------------------
// Etapa de salida
// ------------------------------------------------------------------------------------------------

/// Identidad hasta k, luego k + (1-k)(1 - exp(-(x-k)/(1-k))): continua en C1, asíntota 1.0.
pub fn tonemap(x: f64, k: f64) -> f64 {
    let x = x.max(0.0);
    if x <= k { x } else { k + (1.0 - k) * (1.0 - (-(x - k) / (1.0 - k)).exp()) }
}

/// OETF Rec.709 (ITU-R BT.709-6).
pub fn rec709_oetf(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    if l < 0.018 { 4.5 * l } else { 1.099 * l.powf(0.45) - 0.099 }
}

/// Tabla de salida (65536 niveles de luz lineal cuantizada → 8 bits), calculada una vez.
pub fn out_lut() -> &'static [u8] {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    LUT.get_or_init(|| {
        (0..65536)
            .map(|i| {
                let x = i as f64 / OUT_SCALE;
                (rec709_oetf(tonemap(x, SHOULDER_START)) * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
            })
            .collect()
    })
}

/// Cómo se decodificó el cuadro (argumentos de `make_converter` en Python).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decoding {
    /// "limited" (código 64..940 → 0..255) o "full" (0..1023 → 0..255).
    pub decoded_range: String,
    /// Matriz Y'CbCr que usó el decodificador: "bt601", "bt709" o "bt2020".
    pub decoded_matrix: String,
    /// Matriz real del archivo; `None` = la habitual de la cámara.
    pub source_matrix: Option<String>,
    /// Qué espera un `.cube` cuya cabecera no lo dice: "full" o "legal".
    pub cube_input: Option<String>,
}

impl Default for Decoding {
    /// Lo que hace `cv2.VideoCapture` con cualquier archivo (valores por defecto de Python).
    fn default() -> Self {
        Decoding { decoded_range: "limited".into(), decoded_matrix: "bt601".into(), source_matrix: None, cube_input: None }
    }
}

/// Conversor de cuadros BGR de 8 bits (sin cambiar el tamaño).
pub trait Converter: Send + Sync {
    /// Convierte un cuadro BGR (`width*height*3` bytes).
    fn convert(&self, bgr: &[u8], width: usize, height: usize) -> Vec<u8>;
    /// Descripción corta en español.
    fn description(&self) -> String;
}

struct Identity;
impl Converter for Identity {
    fn convert(&self, bgr: &[u8], _: usize, _: usize) -> Vec<u8> {
        bgr.to_vec()
    }
    fn description(&self) -> String {
        describe("none", None)
    }
}

struct Auto;
impl Converter for Auto {
    fn convert(&self, bgr: &[u8], w: usize, h: usize) -> Vec<u8> {
        metrics::normalize_contrast(bgr, w, h, 0.5, 99.5)
    }
    fn description(&self) -> String {
        describe("auto", None)
    }
}

/// Conversor de cámara (curva + gama + hombro + OETF).
pub struct CameraConverter {
    tid: String,
    fix: Option<[[f32; 3]; 3]>,
    /// Tabla de entrada: valor de 8 bits → luz lineal (en la gama de la cámara).
    pub lut_in: [f32; 256],
    mat: [[f32; 3]; 3],
}

impl CameraConverter {
    fn new(tid: &str, d: &Decoding) -> Result<Self> {
        let (curve, domain, gamut, coded) = camera(tid).ok_or_else(|| anyhow!("Transformación desconocida: {tid:?}"))?;
        let fix = matrix_fix(&d.decoded_matrix, d.source_matrix.as_deref().unwrap_or(coded));
        let (code, signal) = domain_values(&d.decoded_range);
        let src = if domain == "code" { code } else { signal };
        let lut_in: [f32; 256] = std::array::from_fn(|i| curve(src[i]) as f32);
        let g = rev(&gamut_to_709(gamut));
        let mat = std::array::from_fn(|i| std::array::from_fn(|j| (g[i][j] * OUT_SCALE) as f32));
        Ok(CameraConverter { tid: tid.into(), fix, lut_in, mat })
    }
}

impl Converter for CameraConverter {
    fn convert(&self, bgr: &[u8], _w: usize, _h: usize) -> Vec<u8> {
        let fixed;
        let src = match &self.fix {
            Some(f) => {
                fixed = image::transform_u8(bgr, f); // 8 bits, saturado
                &fixed[..]
            }
            None => bgr,
        };
        let lut_out = out_lut();
        let (lut, m) = (&self.lut_in, &self.mat);
        let mut out = vec![0u8; src.len()];
        out.par_chunks_mut(3 * 4096).zip(src.par_chunks(3 * 4096)).for_each(|(o, s)| {
            for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
                let (b, g, r) = (lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]);
                for c in 0..3 {
                    // cv2.transform en float32 y cv2.add(..., dtype=CV_16U): redondeo al par y saturación
                    let v = m[c][0] * b + m[c][1] * g + m[c][2] * r;
                    let idx = v.round_ties_even().clamp(0.0, 65535.0) as usize;
                    d[c] = lut_out[idx];
                }
            }
        });
        out
    }
    fn description(&self) -> String {
        describe(&self.tid, None)
    }
}

// ------------------------------------------------------------------------------------------------
// LUT .cube (formato Resolve / Adobe): 1D, 3D o 1D de preparación + 3D
// ------------------------------------------------------------------------------------------------

/// Contenido de un `.cube`. La tabla 3D se guarda indexada `[r][g][b]` → RGB.
#[derive(Debug, Clone)]
pub struct CubeInfo {
    pub title: String,
    pub size1d: usize,
    pub size3d: usize,
    /// Rango de entrada declarado en la cabecera ("full"/"legal") o `None`.
    pub input: Option<String>,
    pub output: Option<String>,
    pub dmin1d: [f64; 3],
    pub dmax1d: [f64; 3],
    pub dmin3d: [f64; 3],
    pub dmax3d: [f64; 3],
    pub lut1d: Option<Vec<[f32; 3]>>,
    /// `size3d³` tripletas en orden `[r][g][b]`.
    pub lut3d: Option<Vec<[f32; 3]>>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Busca `\b(full|legal|video|data)\s+<tail>\b` en un comentario en minúsculas (como la regex de Python).
fn range_hint(low: &str, tail: &str) -> Option<String> {
    let chars: Vec<char> = low.chars().collect();
    for i in 0..chars.len() {
        if i > 0 && is_word(chars[i - 1]) {
            continue;
        }
        for w in ["full", "legal", "video", "data"] {
            let wl: Vec<char> = w.chars().collect();
            if chars.len() < i + wl.len() || chars[i..i + wl.len()] != wl[..] {
                continue;
            }
            let mut j = i + wl.len();
            let ws = j;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j == ws {
                continue;
            }
            let tl: Vec<char> = tail.chars().collect();
            if chars.len() >= j + tl.len() && chars[j..j + tl.len()] == tl[..] && (j + tl.len() == chars.len() || !is_word(chars[j + tl.len()])) {
                return Some(if w == "full" || w == "data" { "full".into() } else { "legal".into() });
            }
        }
    }
    None
}

/// Lee un `.cube`. Errores con el mismo texto que Python.
pub fn read_cube(path: &Path) -> Result<CubeInfo> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut info = CubeInfo {
        title: String::new(), size1d: 0, size3d: 0, input: None, output: None,
        dmin1d: [0.0; 3], dmax1d: [1.0; 3], dmin3d: [0.0; 3], dmax3d: [1.0; 3], lut1d: None, lut3d: None,
    };
    let mut nums: Vec<f32> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            let low = line.to_lowercase();
            if let Some(v) = range_hint(&low, "in") {
                info.input = Some(v);
            }
            if let Some(v) = range_hint(&low, "out") {
                info.output = Some(v);
            }
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let head = parts[0].to_uppercase();
        let first = head.chars().next().unwrap();
        if first.is_ascii_digit() || "-+.".contains(first) {
            let data = line.split('#').next().unwrap_or("");
            for t in data.split_whitespace() {
                // numpy convierte texto → float64 → float32
                let v: f64 = t.parse().map_err(|_| anyhow!("{name}: valor no numérico {t:?}"))?;
                nums.push(v as f32);
            }
            continue;
        }
        let f = |i: usize| -> Result<f64> { Ok(parts.get(i).ok_or_else(|| anyhow!("{name}: línea incompleta"))?.parse::<f64>()?) };
        match head.as_str() {
            "TITLE" => info.title = line.get(5..).unwrap_or("").trim().trim_matches('"').to_string(),
            "LUT_1D_SIZE" => info.size1d = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
            "LUT_3D_SIZE" => info.size3d = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
            "DOMAIN_MIN" | "DOMAIN_MAX" => {
                let v = [f(1)?, f(2)?, f(3)?];
                // spec de Adobe: se aplica a la LUT presente
                if head == "DOMAIN_MIN" {
                    info.dmin1d = v;
                    info.dmin3d = v;
                } else {
                    info.dmax1d = v;
                    info.dmax3d = v;
                }
            }
            "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                let (lo, hi) = (f(1)?, f(2)?);
                if head.starts_with("LUT_1D") {
                    info.dmin1d = [lo; 3];
                    info.dmax1d = [hi; 3];
                } else {
                    info.dmin3d = [lo; 3];
                    info.dmax3d = [hi; 3];
                }
            }
            "LUT_IN_VIDEO_RANGE" => info.input = Some("legal".into()), // LUT exportadas por Resolve
            "LUT_OUT_VIDEO_RANGE" => info.output = Some("legal".into()),
            _ => {}
        }
    }
    let (n1, n3) = (info.size1d, info.size3d);
    if n1 == 0 && n3 == 0 {
        bail!("{name}: falta LUT_1D_SIZE / LUT_3D_SIZE");
    }
    let rows: Vec<[f32; 3]> = nums.chunks(3).map(|c| [c[0], *c.get(1).unwrap_or(&0.0), *c.get(2).unwrap_or(&0.0)]).collect();
    if nums.len() % 3 != 0 || rows.len() != n1 + n3 * n3 * n3 {
        bail!("{name}: {} filas de datos, se esperaban {}", rows.len(), n1 + n3 * n3 * n3);
    }
    if n1 > 0 {
        info.lut1d = Some(rows[..n1].to_vec());
    }
    if n3 > 0 {
        // el archivo va con el rojo más rápido: fila i = r + g*n + b*n² → reordenar a [r][g][b]
        let src = &rows[n1..];
        let mut t = vec![[0f32; 3]; n3 * n3 * n3];
        for b in 0..n3 {
            for g in 0..n3 {
                for r in 0..n3 {
                    t[(r * n3 + g) * n3 + b] = src[(b * n3 + g) * n3 + r];
                }
            }
        }
        info.lut3d = Some(t);
    }
    Ok(info)
}

/// Preparación 1D opcional (interpolación lineal, dominio acotado) sobre un canal (`np.interp`).
fn axis_lut(values: &[f64], lut1d: Option<&Vec<[f32; 3]>>, dmin: &[f64; 3], dmax: &[f64; 3], ch: usize) -> Vec<f64> {
    let Some(l) = lut1d else { return values.to_vec() };
    let n = l.len();
    values
        .iter()
        .map(|&v| {
            let t = ((v - dmin[ch]) / (dmax[ch] - dmin[ch])).clamp(0.0, 1.0) * (n - 1) as f64;
            let i = (t.floor() as usize).min(n.saturating_sub(1));
            if i + 1 >= n {
                return l[n - 1][ch] as f64;
            }
            let (a, b) = (l[i][ch] as f64, l[i + 1][ch] as f64);
            let f = t - i as f64;
            // np.interp: a + (b-a)*f
            a + (b - a) * f
        })
        .collect()
}

/// Pesos de interpolación por eje: índice bajo, índice alto y fracción (float32).
fn interp_weights(coord: &[f64], n: usize) -> (Vec<usize>, Vec<usize>, Vec<f32>) {
    let mut i0 = Vec::with_capacity(coord.len());
    let mut i1 = Vec::with_capacity(coord.len());
    let mut f = Vec::with_capacity(coord.len());
    for &c in coord {
        let c = c.clamp(0.0, (n - 1) as f64);
        let lo = if n > 1 { (c.floor() as usize).min(n - 2) } else { 0 };
        i0.push(lo);
        i1.push((lo + 1).min(n - 1));
        f.push((c - lo as f64) as f32);
    }
    (i0, i1, f)
}

/// Conversor `.cube`.
pub struct CubeConverter {
    desc: String,
    fix: Option<[[f32; 3]; 3]>,
    /// LUT 1D pura: tabla por canal en orden BGR.
    table1d: Option<[[u8; 3]; 256]>,
    /// LUT 3D: tabla densa 256³ indexada por `R<<16 | G<<8 | B` → BGR.
    dense: Option<Vec<[u8; 3]>>,
}

impl CubeConverter {
    fn new(path: &Path, d: &Decoding) -> Result<Self> {
        let info = read_cube(path)?;
        let expects = info.input.clone().or(d.cube_input.clone()).unwrap_or_else(|| "legal".into());
        let (code, signal) = domain_values(&d.decoded_range);
        let x_in = if expects == "full" { code } else { signal }; // lo que espera la LUT por valor de 8 bits
        let legal_out = info.output.as_deref() == Some("legal");
        let fix = matrix_fix(&d.decoded_matrix, d.source_matrix.as_deref().unwrap_or("bt709"));
        let shaped: Vec<Vec<f64>> = (0..3).map(|c| axis_lut(&x_in, info.lut1d.as_ref(), &info.dmin1d, &info.dmax1d, c)).collect();
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Some(lut) = info.lut3d.as_ref() else {
            // LUT 1D pura: tres tablas (en f64, como en Python)
            let to_u8 = |v: f64| {
                let v = if legal_out { (v * 1023.0 - 64.0) / 876.0 } else { v };
                (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            };
            let table: [[u8; 3]; 256] = std::array::from_fn(|i| [to_u8(shaped[2][i]), to_u8(shaped[1][i]), to_u8(shaped[0][i])]);
            return Ok(CubeConverter { desc: format!("LUT 1D {}: {name}", info.size1d), fix, table1d: Some(table), dense: None });
        };
        // Tabla densa 256³ de la interpolación trilineal, hecha por ejes (trilineal = tres lineales).
        let n = info.size3d;
        let w: Vec<(Vec<usize>, Vec<usize>, Vec<f32>)> = (0..3)
            .map(|c| {
                let coord: Vec<f64> = shaped[c].iter().map(|v| (v - info.dmin3d[c]) / (info.dmax3d[c] - info.dmin3d[c]) * (n - 1) as f64).collect();
                interp_weights(&coord, n)
            })
            .collect();
        let (r0, r1, rf) = &w[0];
        let (g0, g1, gf) = &w[1];
        let (b0, b1, bf) = &w[2];
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|k| a[k] * (1.0 - t) + b[k] * t) };
        // A[r][g][b8]: interpolado en azul
        let a: Vec<[f32; 3]> = (0..n * n)
            .into_par_iter()
            .flat_map_iter(|rg| (0..256).map(move |b8| lerp(lut[rg * n + b0[b8]], lut[rg * n + b1[b8]], bf[b8])))
            .collect();
        // S_j[g8][b8]: interpolado en verde, para cada índice rojo j del cubo
        let slabs: Vec<Vec<[f32; 3]>> = (0..n)
            .into_par_iter()
            .map(|j| {
                let mut s = vec![[0f32; 3]; 256 * 256];
                for g8 in 0..256 {
                    let (ra, rb) = ((j * n + g0[g8]) * 256, (j * n + g1[g8]) * 256);
                    for b8 in 0..256 {
                        s[g8 * 256 + b8] = lerp(a[ra + b8], a[rb + b8], gf[g8]);
                    }
                }
                s
            })
            .collect();
        drop(a);
        let to_u8 = |v: f32| {
            let v = if legal_out { (v * 1023.0f32 - 64.0f32) / 876.0f32 } else { v };
            (v * 255.0f32 + 0.5f32).clamp(0.0, 255.0) as u8
        };
        let mut dense = vec![[0u8; 3]; 1 << 24];
        dense.par_chunks_mut(1 << 16).enumerate().for_each(|(r8, out)| {
            let (sa, sb, t) = (&slabs[r0[r8]], &slabs[r1[r8]], rf[r8]);
            for i in 0..(1 << 16) {
                let rgb = lerp(sa[i], sb[i], t);
                out[i] = [to_u8(rgb[2]), to_u8(rgb[1]), to_u8(rgb[0])];
            }
        });
        let extra = if info.size1d > 0 { format!(" + 1D {}", info.size1d) } else { String::new() };
        Ok(CubeConverter { desc: format!("LUT 3D {n}³{extra}: {name}"), fix, table1d: None, dense: Some(dense) })
    }
}

impl Converter for CubeConverter {
    fn convert(&self, bgr: &[u8], _w: usize, _h: usize) -> Vec<u8> {
        let fixed;
        let src = match &self.fix {
            Some(f) => {
                fixed = image::transform_u8(bgr, f);
                &fixed[..]
            }
            None => bgr,
        };
        let mut out = vec![0u8; src.len()];
        if let Some(t) = &self.table1d {
            out.par_chunks_mut(3 * 4096).zip(src.par_chunks(3 * 4096)).for_each(|(o, s)| {
                for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
                    for c in 0..3 {
                        d[c] = t[p[c] as usize][c];
                    }
                }
            });
        } else if let Some(dense) = &self.dense {
            out.par_chunks_mut(3 * 4096).zip(src.par_chunks(3 * 4096)).for_each(|(o, s)| {
                for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
                    let key = (p[2] as usize) << 16 | (p[1] as usize) << 8 | p[0] as usize;
                    d.copy_from_slice(&dense[key]);
                }
            });
        }
        out
    }
    fn description(&self) -> String {
        self.desc.clone()
    }
}

// ------------------------------------------------------------------------------------------------
// API pública
// ------------------------------------------------------------------------------------------------

type CubeKey = (String, u128, u64, Decoding);

fn camera_cache() -> &'static Mutex<HashMap<(String, Decoding), Arc<dyn Converter>>> {
    static C: OnceLock<Mutex<HashMap<(String, Decoding), Arc<dyn Converter>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn cube_cache() -> &'static Mutex<Vec<(CubeKey, Arc<dyn Converter>)>> {
    static C: OnceLock<Mutex<Vec<(CubeKey, Arc<dyn Converter>)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Cada LUT 3D guarda una tabla densa de 48 MB: se conservan las 4 últimas.
const CUBE_KEEP: usize = 4;

fn check(name: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if !allowed.contains(&value) {
        let mut a: Vec<&str> = allowed.to_vec();
        a.sort();
        bail!("{name}={value:?}; esperado uno de {a:?}");
    }
    Ok(())
}

/// Conversor BGR→BGR de 8 bits para `transform_id` (una clave de [`TRANSFORMS`]). Se guarda en caché.
pub fn make_converter(transform_id: &str, cube_path: Option<&str>, d: &Decoding) -> Result<Arc<dyn Converter>> {
    let tid = if transform_id.is_empty() { "none" } else { transform_id };
    if !TRANSFORMS.iter().any(|(k, _)| *k == tid) {
        bail!("Transformación desconocida: {transform_id:?}");
    }
    check("decoded_range", &d.decoded_range, &["limited", "full"])?;
    check("decoded_matrix", &d.decoded_matrix, &["bt601", "bt709", "bt2020"])?;
    if let Some(s) = &d.source_matrix {
        check("source_matrix", s, &["bt601", "bt709", "bt2020"])?;
    }
    if let Some(s) = &d.cube_input {
        check("cube_input", s, &["full", "legal"])?;
    }
    match tid {
        "none" => return Ok(Arc::new(Identity)),
        "auto" => return Ok(Arc::new(Auto)),
        "cube" => {
            let p = cube_path.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("La transformación 'cube' necesita la ruta de un archivo .cube"))?;
            let p = std::path::absolute(p)?;
            let meta = std::fs::metadata(&p)?;
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            let key: CubeKey = (p.to_string_lossy().to_lowercase(), mtime, meta.len(), d.clone());
            {
                let mut c = cube_cache().lock().unwrap();
                if let Some(pos) = c.iter().position(|(k, _)| *k == key) {
                    let e = c.remove(pos);
                    let conv = e.1.clone();
                    c.push(e);
                    return Ok(conv);
                }
            }
            let conv: Arc<dyn Converter> = Arc::new(CubeConverter::new(&p, d)?);
            let mut c = cube_cache().lock().unwrap();
            c.push((key, conv.clone()));
            while c.len() > CUBE_KEEP {
                c.remove(0);
            }
            Ok(conv)
        }
        _ => {
            let key = (tid.to_string(), d.clone());
            let mut c = camera_cache().lock().unwrap();
            if let Some(v) = c.get(&key) {
                return Ok(v.clone());
            }
            let conv: Arc<dyn Converter> = Arc::new(CameraConverter::new(tid, d)?);
            c.insert(key, conv.clone());
            Ok(conv)
        }
    }
}

/// Descripción corta en español para la interfaz.
pub fn describe(transform_id: &str, cube_path: Option<&str>) -> String {
    let tid = if transform_id.is_empty() { "none" } else { transform_id };
    if tid == "cube" {
        let Some(p) = cube_path.filter(|s| !s.is_empty()) else { return "LUT .cube: elige un archivo.".into() };
        let abs = std::path::absolute(p).map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some((_, conv)) = cube_cache().lock().unwrap().iter().find(|(k, _)| k.0 == abs) {
            return conv.description();
        }
        let name = Path::new(p).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return format!("LUT .cube: {name}");
    }
    match tid {
        "none" => "Sin conversión: el cuadro tal como sale del decodificador.".into(),
        "auto" => "Estira el contraste de la luma (percentiles 0.5–99.5); no convierte el color.".into(),
        "slog3_sgamut3cine" => "Curva S-Log3 y matriz S-Gamut3.Cine oficiales de Sony a Rec.709, con hombro suave en altas luces.".into(),
        "slog3_sgamut3" => "Curva S-Log3 y matriz S-Gamut3 oficiales de Sony a Rec.709, con hombro suave en altas luces.".into(),
        "slog2_sgamut" => "Curva S-Log2 y matriz S-Gamut oficiales de Sony a Rec.709, con hombro suave en altas luces.".into(),
        "hlg" => "HLG (BT.2100) y Rec.2020 a Rec.709 SDR: blanco de referencia HLG 75 % ≈ blanco, hombro suave.".into(),
        "vlog" => "Curva V-Log y matriz V-Gamut oficiales de Panasonic a Rec.709, con hombro suave en altas luces.".into(),
        "clog3" => "Curva Canon Log 3 y Cinema Gamut a Rec.709, con hombro suave en altas luces.".into(),
        "applelog" => "Curva Apple Log (white paper de Apple) y Rec.2020 a Rec.709, con hombro suave en altas luces.".into(),
        _ => format!("Transformación desconocida: {transform_id}"),
    }
}

// ------------------------------------------------------------------------------------------------
// Metadatos: detect_profile / decoding_for
// ------------------------------------------------------------------------------------------------

/// Bytes de cada punta del archivo donde se busca el XML de Sony incrustado.
const XML_INCRUSTADO_BYTES: u64 = 256 * 1024;

/// El XML de Sony (NonRealTimeMeta) que la cámara también graba dentro del MP4, al final del archivo
/// (en un A7 IV, a ~700 bytes del final) o, si el `moov` va delante, al principio. Sirve cuando falta el
/// `M01.XML` de al lado: en "Videos Yo Manos Detalle" C9254 no lo tenía y su S-Log3 se veía gris.
fn xml_incrustado(video: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let ext = video.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !["mp4", "mov", "m4v", "mxf"].contains(&ext.as_str()) {
        return None;
    }
    let mut f = std::fs::File::open(video).ok()?;
    let len = f.metadata().ok()?.len();
    let n = XML_INCRUSTADO_BYTES.min(len);
    let mut leer = |desde: u64| -> Option<String> {
        f.seek(SeekFrom::Start(desde)).ok()?;
        let mut buf = vec![0u8; n as usize];
        f.read_exact(&mut buf).ok()?;
        let texto = String::from_utf8_lossy(&buf);
        texto.contains("CaptureGammaEquation").then(|| texto.into_owned())
    };
    leer(len - n).or_else(|| leer(0))
}

/// Texto del XML de Sony de un video: el `<stem>M01.XML` de al lado o, si falta, el incrustado en el
/// video ([`xml_incrustado`]). Lo usan el color ([`sony_xml`]) y el timecode
/// (`sampler::sony_drop_frame`), para que los dos lean la misma fuente.
pub(crate) fn texto_xml_sony(video: &Path) -> Option<String> {
    let stem = video.file_stem()?.to_string_lossy().to_string();
    match std::fs::read(video.with_file_name(format!("{stem}M01.XML"))) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(_) => xml_incrustado(video),
    }
}

/// XML de Sony ([`texto_xml_sony`]): CaptureGammaEquation / CaptureColorPrimaries / CodingEquations.
pub fn sony_xml(video: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(text) = texto_xml_sony(video) else { return out };
    for name in ["CaptureGammaEquation", "CaptureColorPrimaries", "CodingEquations"] {
        // equivalente a la regex `name="X"\s+value="([^"]*)"`
        let pat = format!("name=\"{name}\"");
        let mut from = 0;
        while let Some(pos) = text[from..].find(&pat) {
            let mut rest = &text[from + pos + pat.len()..];
            let trimmed = rest.trim_start();
            if trimmed.len() < rest.len() && trimmed.starts_with("value=\"") {
                rest = &trimmed[7..];
                if let Some(end) = rest.find('"') {
                    out.insert(name.to_string(), rest[..end].trim().to_lowercase());
                    break;
                }
            }
            from += pos + pat.len();
        }
    }
    out
}

/// Datos de color de ffprobe de la primera pista de video (en caché por ruta, fecha y tamaño).
#[derive(Debug, Clone, Default)]
pub struct ProbeColor {
    pub codec_name: Option<String>,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    /// Etiquetas del contenedor y de la pista (las de la pista ganan).
    pub tags: Vec<(String, String)>,
}

/// ffprobe del color de un video (Python `_ffprobe`).
pub fn ffprobe_color(video: &Path) -> ProbeColor {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u128, u64), ProbeColor>>> = OnceLock::new();
    let Ok(meta) = std::fs::metadata(video) else { return ProbeColor::default() };
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    let key = (video.to_path_buf(), mtime, meta.len());
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap().get(&key) {
        return v.clone();
    }
    let data: serde_json::Value = crate::sampler::run_quiet(
        "ffprobe",
        &["-v", "error", "-select_streams", "v:0", "-print_format", "json", "-show_streams", "-show_format", &video.to_string_lossy()],
    )
    .ok()
    .and_then(|o| serde_json::from_slice(&o).ok())
    .unwrap_or(serde_json::Value::Null);
    let s = data.get("streams").and_then(|v| v.get(0)).cloned().unwrap_or(serde_json::Value::Null);
    let get = |k: &str| s.get(k).and_then(|v| v.as_str()).map(String::from);
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut put = |obj: Option<&serde_json::Value>| {
        if let Some(m) = obj.and_then(|v| v.as_object()) {
            for (k, v) in m {
                let val = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
                if let Some(e) = tags.iter_mut().find(|(kk, _)| kk == k) {
                    e.1 = val;
                } else {
                    tags.push((k.clone(), val));
                }
            }
        }
    };
    put(data.get("format").and_then(|f| f.get("tags")));
    put(s.get("tags"));
    let info = ProbeColor {
        codec_name: get("codec_name"),
        color_range: get("color_range"),
        color_space: get("color_space"),
        color_transfer: get("color_transfer"),
        color_primaries: get("color_primaries"),
        tags,
    };
    cache.lock().unwrap().insert(key, info.clone());
    info
}

/// Busca una marca de Apple Log en el átomo `moov` de QuickTime.
fn moov_says_apple_log(video: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(video) else { return false };
    let Ok(size) = f.metadata().map(|m| m.len()) else { return false };
    let mut pos = 0u64;
    while pos + 8 <= size {
        if f.seek(SeekFrom::Start(pos)).is_err() {
            return false;
        }
        let mut hdr = [0u8; 16];
        let got = f.read(&mut hdr).unwrap_or(0);
        if got < 8 {
            return false;
        }
        let mut n = u32::from_be_bytes(hdr[..4].try_into().unwrap()) as u64;
        let typ = &hdr[4..8];
        let mut hl = 8u64;
        if n == 1 {
            n = u64::from_be_bytes(hdr[8..16].try_into().unwrap());
            hl = 16;
        } else if n == 0 {
            n = size - pos;
        }
        if n < hl {
            return false;
        }
        if typ == b"moov" {
            let len = (n - hl).min(64 << 20) as usize;
            let mut blob = vec![0u8; len];
            if f.seek(SeekFrom::Start(pos + hl)).is_err() {
                return false;
            }
            let got = f.read(&mut blob).unwrap_or(0);
            let low: Vec<u8> = blob[..got].iter().map(|b| b.to_ascii_lowercase()).collect();
            let has = |pat: &[u8]| low.windows(pat.len()).any(|w| w == pat);
            return has(b"applelog") || has(b"apple log");
        }
        pos += n;
    }
    false
}

/// Id de [`TRANSFORMS`] sugerido por los metadatos, o `None` si no se puede saber.
///
/// Devuelve "none" cuando los metadatos dicen que el clip ya está listo para display.
/// Sony: XML. Otros: etiquetas de color de ffprobe (HLG = arib-std-b67). Apple Log: marca en `moov`,
/// o primarios BT.2020 con transferencia sin especificar en un archivo de Apple / ProRes.
pub fn detect_profile(video_path: &str) -> Option<String> {
    let video = Path::new(video_path);
    let sony = sony_xml(video);
    let gamma = sony.get("CaptureGammaEquation").cloned().unwrap_or_default();
    let prim = sony.get("CaptureColorPrimaries").cloned().unwrap_or_default();
    if !gamma.is_empty() {
        if gamma.starts_with("s-log3") {
            if prim.contains("cine") || (prim.is_empty() && gamma.ends_with("cine")) {
                return Some("slog3_sgamut3cine".into());
            }
            return Some("slog3_sgamut3".into());
        }
        if gamma.starts_with("s-log2") {
            return Some("slog2_sgamut".into());
        }
        if gamma.contains("hlg") {
            return Some("hlg".into());
        }
        if ["rec709", "itu709", "s-cinetone", "still", "cine", "rec709-xvycc", "709"].iter().any(|p| gamma.starts_with(p)) {
            return Some("none".into());
        }
    }
    let info = ffprobe_color(video);
    let trc = info.color_transfer.as_deref();
    if trc == Some("arib-std-b67") {
        return Some("hlg".into());
    }
    let tag_text = info.tags.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(" ").to_lowercase();
    if tag_text.contains("apple log") || tag_text.contains("applelog") {
        return Some("applelog".into());
    }
    if let Some(t) = trc {
        if ["bt709", "smpte170m", "bt470bg", "bt470m", "iec61966-2-1", "bt2020-10", "bt2020-12", "gamma22"].contains(&t) {
            return Some("none".into());
        }
        if t == "smpte2084" {
            return None; // PQ: no soportado
        }
    }
    let unspec = matches!(trc, None | Some("") | Some("unknown") | Some("unspecified") | Some("reserved"));
    let ext = video.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if info.color_primaries.as_deref() == Some("bt2020") && unspec && (ext == "mov" || ext == "mp4") {
        if moov_says_apple_log(video) {
            return Some("applelog".into());
        }
        if info.codec_name.as_deref() == Some("prores") || tag_text.contains("apple") || tag_text.contains("iphone") {
            return Some("applelog".into());
        }
    }
    None
}

/// Parámetros de decodificación de `video_path` según el decodificador ("cv2" o "ffmpeg").
///
/// "cv2": siempre BT.601 y rango limitado. "ffmpeg": respeta `color_range` y la matriz etiquetada.
pub fn decoding_for(video_path: &str, decoder: &str) -> Result<Decoding> {
    check("decoder", decoder, &["cv2", "ffmpeg"])?;
    let video = Path::new(video_path);
    let info = ffprobe_color(video);
    let file_full = info.color_range.as_deref() == Some("pc");
    let tagged = match info.color_space.as_deref().unwrap_or("") {
        "bt709" => Some("bt709"),
        "bt2020nc" | "bt2020c" => Some("bt2020"),
        "smpte170m" | "bt470bg" => Some("bt601"),
        _ => None,
    };
    let coding = match sony_xml(video).get("CodingEquations").map(String::as_str).unwrap_or("") {
        "rec709" => Some("bt709"),
        "rec2020ncl" | "rec2020" => Some("bt2020"),
        "rec601" => Some("bt601"),
        _ => None,
    };
    let source_matrix = coding.or(tagged).map(String::from);
    let cube_input = Some(if file_full { "full" } else { "legal" }.to_string());
    Ok(if decoder == "cv2" {
        Decoding { decoded_range: "limited".into(), decoded_matrix: "bt601".into(), source_matrix, cube_input }
    } else {
        Decoding {
            decoded_range: if file_full { "full" } else { "limited" }.into(),
            decoded_matrix: tagged.unwrap_or("bt601").into(),
            source_matrix,
            cube_input,
        }
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const XML: &str = r#"<NonRealTimeMeta><Group name="CameraUnitMetadataSet"><Item name="CaptureGammaEquation" value="s-log3-cine"/><Item name="CaptureColorPrimaries" value="s-gamut3-cine"/><Item name="CodingEquations" value="rec709"/></Group></NonRealTimeMeta>"#;

    /// Sin `M01.XML` al lado, el perfil sale del XML que la cámara grabó dentro del MP4 (al final o al
    /// principio); con él al lado, manda el de al lado.
    #[test]
    fn perfil_del_xml_incrustado() {
        let dir = std::env::temp_dir().join(format!("camcuts_xml_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let relleno = vec![0u8; 600 * 1024];
        // al final (como un A7 IV), a ~700 bytes del final
        let fin = dir.join("C0001.MP4");
        std::fs::write(&fin, [relleno.as_slice(), XML.as_bytes(), &[0u8; 700]].concat()).unwrap();
        assert_eq!(detect_profile(&fin.to_string_lossy()).as_deref(), Some("slog3_sgamut3cine"));
        assert_eq!(sony_xml(&fin).get("CodingEquations").map(String::as_str), Some("rec709"));
        // al principio (moov delante)
        let ini = dir.join("C0002.MP4");
        std::fs::write(&ini, [XML.as_bytes(), relleno.as_slice()].concat()).unwrap();
        assert_eq!(detect_profile(&ini.to_string_lossy()).as_deref(), Some("slog3_sgamut3cine"));
        // en medio de un archivo grande no se busca (solo las puntas)
        let medio = dir.join("C0003.MP4");
        std::fs::write(&medio, [relleno.as_slice(), XML.as_bytes(), relleno.as_slice()].concat()).unwrap();
        assert!(sony_xml(&medio).is_empty());
        // el de al lado manda
        std::fs::write(dir.join("C0001M01.XML"), XML.replace("s-log3-cine", "hlg")).unwrap();
        assert_eq!(detect_profile(&fin.to_string_lossy()).as_deref(), Some("hlg"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
