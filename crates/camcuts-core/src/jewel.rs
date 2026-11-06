//! Medidas de cada muestra centradas en la joya (port de `jewel.py`).
//!
//! El detector (YOLO-World) vive en `camcuts-infer`; aquí están los prompts y su traducción, cómo se
//! agrupan las detecciones, la silueta (y su reutilización mientras la joya no se mueve), la nitidez,
//! la exposición y el movimiento de cada joya, y la estructura [`Sample`] que se guarda en la caché.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::image;
use crate::metrics::{self, CameraShift, EdgeMaps, Region};
use crate::types::{Detection, Detector, Frame, Pose};

/// Prompts en inglés (el codificador de texto de YOLO-World es CLIP) → tipo en español.
pub const JEWEL_PROMPTS: &[(&str, &str)] = &[
    ("ring", "anillo"),
    ("necklace", "collar"),
    ("chain necklace", "cadena"),
    ("earring", "arete"),
    ("stud earring", "topo"),
    ("bracelet", "pulsera"),
    ("pendant", "dije"),
    ("jewelry", "joya"),
];
/// Se detectan en la misma pasada para distinguir producto de una modelo que lleva la joya.
pub const PEOPLE_PROMPTS: &[(&str, &str)] = &[("person", "persona"), ("face", "cara")];

/// ¿Es un tipo de joya (en español)?
pub fn is_jewel_label(label: &str) -> bool {
    JEWEL_PROMPTS.iter().any(|(_, es)| *es == label)
}

/// Traduce el prompt en inglés al tipo en español; deja tal cual lo que ya está en español.
pub fn spanish_label(label: &str) -> String {
    JEWEL_PROMPTS.iter().chain(PEOPLE_PROMPTS).find(|(en, _)| *en == label).map(|(_, es)| es.to_string()).unwrap_or_else(|| label.to_string())
}

/// Una zona de cara/mano más grande que esta fracción del cuadro no se amplía: sus joyas ya se ven.
pub const ZOOM_MAX_AREA: f64 = 0.15;

// Puntos COCO para saber dónde se lleva la joya
pub const NOSE: usize = 0;
pub const L_EYE: usize = 1;
pub const R_EYE: usize = 2;
pub const L_EAR: usize = 3;
pub const R_EAR: usize = 4;
pub const L_SH: usize = 5;
pub const R_SH: usize = 6;
pub const L_WRIST: usize = 9;
pub const R_WRIST: usize = 10;
pub const KP_MIN: f32 = 0.5;

/// ¿Alguna persona muestra la cara (nariz y un ojo visibles)?
pub fn has_face(people: &[Pose]) -> bool {
    people.iter().any(|p| {
        let k = &p.keypoints;
        k[NOSE][2] > KP_MIN && (k[L_EYE][2] > KP_MIN || k[R_EYE][2] > KP_MIN)
    })
}

/// Zonas cuadradas (píxeles del cuadro de análisis) alrededor de orejas, cuello y muñecas.
///
/// El tamaño sigue el ancho de hombros (o la distancia entre orejas), así una modelo lejana da zonas
/// pequeñas que la pasada con zoom amplía; las que ya son grandes en cuadro se saltan.
pub fn zoom_regions(people: &[Pose], w: u32, h: u32, max_regions: usize) -> Vec<[i64; 4]> {
    let mut regions = Vec::new();
    for p in people {
        let k = &p.keypoints;
        let ok = |i: usize| k[i][2] > KP_MIN;
        let dist = |a: usize, b: usize| if ok(a) && ok(b) { ((k[a][0] - k[b][0]) as f64).hypot((k[a][1] - k[b][1]) as f64) } else { 0.0 };
        let or = |a: f64, b: f64| if a != 0.0 { a } else { b };
        let shoulders = dist(L_SH, R_SH);
        // tamaño de cabeza con lo que se vea: las dos orejas, nariz-oreja de perfil, los ojos
        let head = or(or(or(dist(L_EAR, R_EAR), dist(NOSE, L_EAR).max(dist(NOSE, R_EAR)) * 1.2), dist(L_EYE, R_EYE) * 2.5), shoulders * 0.4);
        let body = or(shoulders, head * 2.5);
        if head <= 0.0 {
            continue;
        }
        let mut spots: Vec<(f64, f64, f64)> = Vec::new();
        for ear in [L_EAR, R_EAR] {
            if ok(ear) {
                // el arete cuelga del lóbulo hacia abajo
                spots.push((k[ear][0] as f64, k[ear][1] as f64 + 0.15 * head, 0.6 * head));
            }
        }
        if shoulders != 0.0 {
            // collar / dije entre los hombros, un poco por debajo
            let mx = (k[L_SH][0] + k[R_SH][0]) / 2.0;
            let my = (k[L_SH][1] + k[R_SH][1]) / 2.0;
            spots.push((mx as f64, my as f64 + 0.05 * body, 0.9 * body));
        }
        for wr in [L_WRIST, R_WRIST] {
            if ok(wr) {
                // pulsera en la muñeca, anillos justo después
                spots.push((k[wr][0] as f64, k[wr][1] as f64, 0.55 * body));
            }
        }
        for (cx, cy, side) in spots {
            let r = [
                ((cx - side / 2.0) as i64).max(0),
                ((cy - side / 2.0) as i64).max(0),
                ((cx + side / 2.0) as i64).min(w as i64),
                ((cy + side / 2.0) as i64).min(h as i64),
            ];
            let area = ((r[2] - r[0]) * (r[3] - r[1])) as f64 / (w as f64 * h as f64);
            if r[2] - r[0] >= 8 && r[3] - r[1] >= 8 && area <= ZOOM_MAX_AREA {
                regions.push(r);
            }
        }
    }
    regions.truncate(max_regions);
    regions
}

/// Detección con caja entera (`int()` de Python sobre las coordenadas del modelo).
#[derive(Debug, Clone, PartialEq)]
pub struct Det {
    pub bbox: [i64; 4],
    pub conf: f64,
    pub label: String,
    /// Encontrada por la segunda mirada a resolución de detalle.
    pub zoom: bool,
    /// Detección débil confirmada por CLIP.
    pub verified: bool,
}

impl Det {
    /// Desde la detección del modelo: caja truncada hacia cero y tipo en español.
    pub fn from_detection(d: &Detection) -> Det {
        Det { bbox: [d.x1 as i64, d.y1 as i64, d.x2 as i64, d.y2 as i64], conf: d.conf as f64, label: spanish_label(&d.label), zoom: false, verified: false }
    }
}

/// Una joya medida dentro de una muestra (diccionario de `s.jewels` en Python).
///
/// Los campos opcionales pueden faltar en cachés antiguas; se respetan al leer y escribir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Jewel {
    pub label: String,
    #[serde(rename = "with", default, skip_serializing_if = "Option::is_none")]
    pub with_labels: Option<Vec<String>>,
    #[serde(default)]
    pub conf: f64,
    #[serde(rename = "box")]
    pub bbox: [i64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_frac: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_frac: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lap_var: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lap_p99: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anisotropy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touches_edge: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worn: Option<bool>,
    /// Tipo que dio el detector en este cuadro, antes de estabilizar por pista.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_label: Option<String>,
    /// Campos que esta versión no conoce (p. ej. `kind` del clasificador de catálogo): se conservan.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> f64 {
    1.0
}

/// Medidas de una muestra (dataclass `Sample` de Python, mismo orden de campos en el JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Sample {
    pub t: f64,
    // cuadro completo (equivalente a Cutdown)
    #[serde(default)]
    pub edge_density: f64,
    #[serde(default)]
    pub lap_var: f64,
    #[serde(default)]
    pub mean_luma: f64,
    #[serde(default)]
    pub clip_black: f64,
    #[serde(default)]
    pub clip_white: f64,
    #[serde(default)]
    pub cam_dx: f64,
    #[serde(default)]
    pub cam_dy: f64,
    // joya
    #[serde(default)]
    pub jewel: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub conf: f64,
    #[serde(rename = "box", default)]
    pub bbox: Vec<i64>,
    #[serde(default)]
    pub area_frac: f64,
    #[serde(default)]
    pub touches_edge: bool,
    #[serde(default)]
    pub jewel_edge_density: f64,
    #[serde(default)]
    pub bg_edge_density: f64,
    #[serde(default)]
    pub focus_ratio: f64,
    #[serde(default)]
    pub jewel_lap_var: f64,
    /// Detalle más nítido de la joya (medida principal de nitidez).
    #[serde(default)]
    pub jewel_lap_p99: f64,
    #[serde(default = "one")]
    pub anisotropy: f64,
    /// Diagonales de caja por segundo, en la imagen.
    #[serde(default)]
    pub jewel_speed: f64,
    #[serde(default)]
    pub n_jewels: i64,
    /// Medidas de la joya tomadas sobre la silueta y no sobre la caja.
    #[serde(default)]
    pub masked: bool,
    /// Área de la silueta / área de la caja.
    #[serde(default)]
    pub mask_frac: f64,
    #[serde(default)]
    pub person: bool,
    #[serde(default)]
    pub face: bool,
    /// Caja de persona más grande / cuadro.
    #[serde(default)]
    pub person_frac: f64,
    /// La joya está dentro de la caja de una persona.
    #[serde(default)]
    pub worn: bool,
    /// Piezas que se solapan con la principal (dije → cadena).
    #[serde(default)]
    pub with_labels: Vec<String>,
    /// Cada grupo de joyas del cuadro, medido por separado.
    #[serde(default)]
    pub jewels: Vec<Jewel>,
    /// Joyas cuya propia silueta está nítida.
    #[serde(default)]
    pub n_sharp: i64,
    /// El clip se trató como LOG (contraste estirado o convertido).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<bool>,
    /// Alguien habla con la cara visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking: Option<bool>,
    /// Campos desconocidos: se conservan tal cual.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Cuando las detecciones se solapan (un dije en su cadena), la pieza más específica es la principal.
pub fn priority(label: &str) -> i32 {
    match label {
        "dije" => 0,
        "anillo" | "arete" | "topo" | "candonga" | "piercing" => 1,
        "pulsera" | "tobillera" | "rosario" => 2,
        "collar" => 3,
        "cadena" => 4,
        "joya" => 5,
        _ => 9,
    }
}

/// Intersección sobre la caja más pequeña: 1.0 cuando una caja está dentro de la otra.
pub fn overlap(a: &[i64; 4], b: &[i64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0);
    let small = ((a[2] - a[0]) * (a[3] - a[1])).min((b[2] - b[0]) * (b[3] - b[1]));
    if small > 0 { (ix * iy) as f64 / small as f64 } else { 0.0 }
}

/// Intersección sobre unión de dos cajas.
pub fn box_iou(a: &[i64; 4], b: &[i64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0);
    let inter = ix * iy;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union > 0 { inter as f64 / union as f64 } else { 0.0 }
}

pub const MAX_JEWELS: usize = 6;
/// Siluetas por cuadro; el resto de joyas se miden sobre su caja.
pub const SAM_MAX: usize = 3;
/// |Laplaciano| p99 desde el que una joya cuenta como nítida (iPhone + Sony 4K etiquetados a mano).
pub const SHARP_LAP: f64 = 90.0;
/// Una silueta se reutiliza (desplazada) hasta este tiempo mientras su joya no se mueva.
pub const SIL_REUSE_S: f64 = 1.0;
pub const SIL_REUSE_IOU: f64 = 0.8;

/// Grupos de joyas, el más seguro primero: (pieza principal, tipos de las piezas solapadas).
///
/// Detecciones solapadas son una misma joya vista dos veces (un dije y su cadena); gana el tipo más
/// específico (dije sobre cadena).
pub fn groups(dets: &[Det], min_conf: f64) -> Vec<(Det, Vec<String>)> {
    let mut left: Vec<&Det> = dets.iter().filter(|d| d.conf >= min_conf && is_jewel_label(&d.label)).collect();
    left.sort_by(|a, b| b.conf.total_cmp(&a.conf)); // estable, como sorted(reverse=True)
    let mut out = Vec::new();
    while !left.is_empty() && out.len() < MAX_JEWELS {
        let seed = left.remove(0);
        let mut group = vec![seed];
        group.extend(left.iter().copied().filter(|d| overlap(&d.bbox, &seed.bbox) > 0.6));
        // `d not in group` compara por valor en Python
        left.retain(|d| !group.iter().any(|g| *g == *d));
        let mut main = group[0];
        for d in &group[1..] {
            let (kd, km) = ((priority(&d.label), -d.conf), (priority(&main.label), -main.conf));
            if kd.0 < km.0 || (kd.0 == km.0 && kd.1 < km.1) {
                main = d;
            }
        }
        let mut extras: Vec<String> = group.iter().filter(|d| d.label != main.label).map(|d| d.label.clone()).collect();
        extras.sort();
        extras.dedup();
        out.push((main.clone(), extras));
    }
    out
}

/// Siluetas recientes de un clip, para reutilizarlas mientras las joyas no se mueven.
#[derive(Default)]
pub struct SilCache {
    t: f64,
    boxes: Vec<[i64; 4]>,
    /// Máscaras del tamaño del cuadro (1 = joya).
    sils: Vec<Option<Vec<u8>>>,
    valid: bool,
}

impl SilCache {
    /// Siluetas recién calculadas en el instante `t` (la edición Bodas las guarda igual que [`measure_with`]).
    pub(crate) fn new(t: f64, boxes: Vec<[i64; 4]>, sils: Vec<Option<Vec<u8>>>) -> Self {
        SilCache { t, boxes, sils, valid: true }
    }
}

/// Desplaza una máscara (relleno con ceros), como `_shift` de Python.
fn shift_mask(mask: &[u8], w: usize, h: usize, dx: i64, dy: i64) -> Vec<u8> {
    let mut out = vec![0u8; mask.len()];
    for y in 0..h as i64 {
        let sy = y - dy;
        if sy < 0 || sy >= h as i64 {
            continue;
        }
        let (x0, x1) = (dx.max(0), (w as i64 + dx).min(w as i64));
        if x0 >= x1 {
            continue;
        }
        let d = y as usize * w;
        let s = sy as usize * w;
        out[d + x0 as usize..d + x1 as usize].copy_from_slice(&mask[s + (x0 - dx) as usize..s + (x1 - dx) as usize]);
    }
    out
}

/// Las últimas siluetas, movidas con sus cajas, si todas las joyas siguen donde estaban (si no, `None`).
pub(crate) fn reuse_silhouettes(cache: &SilCache, t: f64, boxes: &[[i64; 4]], w: usize, h: usize) -> Option<Vec<Option<Vec<u8>>>> {
    if !cache.valid || t - cache.t > SIL_REUSE_S || cache.boxes.len() != boxes.len() {
        return None;
    }
    let mut out = Vec::new();
    for ((b, cb), m) in boxes.iter().zip(&cache.boxes).zip(&cache.sils) {
        if box_iou(b, cb) < SIL_REUSE_IOU {
            return None;
        }
        out.push(m.as_ref().map(|m| {
            let dx = (((b[0] + b[2] - cb[0] - cb[2]) as f64) / 2.0).round_ties_even() as i64;
            let dy = (((b[1] + b[3] - cb[1] - cb[3]) as f64) / 2.0).round_ties_even() as i64;
            shift_mask(m, w, h, dx, dy)
        }));
    }
    Some(out)
}

/// Dilatación 5x5 de la silueta (crece 2 px para medir también el contorno de la joya), recortada a la caja.
/// Devuelve la máscara local de la caja.
pub(crate) fn grow_in_rect(sil: &[u8], w: usize, h: usize, r: [usize; 4]) -> Vec<u8> {
    let [x0, y0, x1, y1] = r;
    let (rw, rh) = (x1 - x0, y1 - y0);
    // máximo horizontal en las filas que pueden tocar la caja
    let ya = y0.saturating_sub(2);
    let yb = (y1 + 2).min(h);
    let mut hor = vec![0u8; (yb - ya) * rw];
    for y in ya..yb {
        let row = &sil[y * w..(y + 1) * w];
        for x in x0..x1 {
            let (a, b) = (x.saturating_sub(2), (x + 3).min(w));
            hor[(y - ya) * rw + (x - x0)] = row[a..b].iter().any(|&v| v != 0) as u8;
        }
    }
    let mut out = vec![0u8; rw * rh];
    for y in y0..y1 {
        let (a, b) = (y.saturating_sub(2).max(ya), (y + 3).min(yb));
        for x in 0..rw {
            out[(y - y0) * rw + x] = (a..b).any(|yy| hor[(yy - ya) * rw + x] != 0) as u8;
        }
    }
    out
}

/// Estado por clip que necesita `measure` entre muestras.
pub struct MeasureState {
    pub shift: CameraShift,
    pub sil: SilCache,
    /// Cajas de las joyas de la muestra anterior (velocidad).
    pub prev_boxes: Vec<[i64; 4]>,
    pub has_prev: bool,
    /// Siluetas pedidas a SAM que fallaron (modelo que no abre, error de la GPU): esas cajas se midieron
    /// enteras. El análisis lo apunta en la caché (`"sam": false`) para que se vea y se pueda reanalizar.
    pub sam_fallos: usize,
}

impl Default for MeasureState {
    fn default() -> Self {
        MeasureState { shift: CameraShift::new(), sil: SilCache::default(), prev_boxes: Vec::new(), has_prev: false, sam_fallos: 0 }
    }
}

/// Parámetros fijos de la medida de un clip.
#[derive(Debug, Clone, Copy)]
pub struct MeasureOpts {
    /// Segundos entre muestras.
    pub dt: f64,
    pub min_conf: f64,
    /// Corrige la nitidez del Laplaciano según el escalador del decodificador.
    pub lap_scale: f64,
    /// Pedir siluetas (MobileSAM) al detector.
    pub use_sam: bool,
}

/// Medidas del cuadro entero que no dependen de las detecciones: gris, bordes/Laplaciano, exposición y
/// movimiento de cámara. Se separan de [`measure`] para calcularlas en la CPU mientras la GPU detecta.
pub struct FrameMaps {
    pub gray: Vec<u8>,
    pub maps: EdgeMaps,
    pub exposure: metrics::Exposure,
    /// Desplazamiento de cámara respecto a la muestra anterior (píxeles).
    pub dx: f64,
    pub dy: f64,
}

/// Calcula las medidas de cuadro entero. `shift` es el de [`MeasureState`]: guarda el espectro de
/// esta muestra para la siguiente, así que hay que llamarla una vez por muestra y en orden.
pub fn frame_maps(frame: &Frame, shift: &mut CameraShift) -> FrameMaps {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let gray = image::bgr_to_gray(&frame.data, w, h);
    let maps: EdgeMaps = metrics::edges_and_laplacian(&gray, w, h);
    let exposure = metrics::exposure(&gray);
    let (dx, dy, _) = shift.shift(&gray, w, h);
    FrameMaps { gray, maps, exposure, dx, dy }
}

/// Mide una muestra.
///
/// `segmenter`: el detector, si hay que pedirle siluetas. `face_seen`: la pose vio una cara.
pub fn measure(t: f64, frame: &Frame, dets: &[Det], state: &mut MeasureState, opts: &MeasureOpts, segmenter: Option<&mut dyn Detector>, face_seen: bool) -> Sample {
    let pre = frame_maps(frame, &mut state.shift);
    measure_with(t, frame, pre, dets, state, opts, segmenter, face_seen)
}

/// [`measure`] con las medidas de cuadro entero ya calculadas por [`frame_maps`] (con `state.shift`).
#[allow(clippy::too_many_arguments)]
pub fn measure_with(
    t: f64,
    frame: &Frame,
    pre: FrameMaps,
    dets: &[Det],
    state: &mut MeasureState,
    opts: &MeasureOpts,
    segmenter: Option<&mut dyn Detector>,
    face_seen: bool,
) -> Sample {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let FrameMaps { gray, maps, exposure: ex, dx, dy } = pre;
    let ls = opts.lap_scale;
    let mut s = Sample {
        t,
        edge_density: metrics::density_full(&maps),
        lap_var: metrics::laplacian_variance(&maps.lap, None) * ls * ls,
        mean_luma: ex.mean_luma,
        clip_black: ex.clip_black,
        clip_white: ex.clip_white,
        cam_dx: dx,
        cam_dy: dy,
        anisotropy: 1.0,
        ..Default::default()
    };

    let people: Vec<&Det> = dets.iter().filter(|d| d.label == "persona" && d.conf >= 0.25).collect();
    s.person = !people.is_empty();
    s.face = face_seen || dets.iter().any(|d| d.label == "cara" && d.conf >= 0.25);
    if !people.is_empty() {
        let m = people.iter().map(|d| (d.bbox[2] - d.bbox[0]) * (d.bbox[3] - d.bbox[1])).max().unwrap();
        s.person_frac = m as f64 / (w * h) as f64;
    }

    let mut gs: Vec<(Det, Vec<String>, [i64; 4])> = Vec::new();
    for (main, extras) in groups(dets, opts.min_conf) {
        let b = main.bbox;
        let r = [b[0].max(0), b[1].max(0), b[2].min(w as i64), b[3].min(h as i64)];
        if r[2] - r[0] >= 4 && r[3] - r[1] >= 4 {
            gs.push((main, extras, r));
        }
    }
    s.n_jewels = gs.len() as i64;
    let prev_boxes = std::mem::take(&mut state.prev_boxes);
    let had_prev = state.has_prev;
    state.has_prev = true;
    if gs.is_empty() {
        return s;
    }
    let ur = |r: &[i64; 4]| [r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize];

    // fondo = todo lo que queda fuera de cada caja agrandada un 25 %, para que el halo de ninguna joya cuente
    let mut outside = vec![1u8; w * h];
    for (_, _, r) in &gs {
        let (gx, gy) = (((r[2] - r[0]) as f64 * 0.25) as i64, ((r[3] - r[1]) as f64 * 0.25) as i64);
        let (ya, yb) = ((r[1] - gy).max(0) as usize, (r[3] + gy).min(h as i64) as usize);
        let (xa, xb) = ((r[0] - gx).max(0) as usize, (r[2] + gx).min(w as i64) as usize);
        for y in ya..yb {
            outside[y * w + xa..y * w + xb].fill(0);
        }
    }
    let bg = if outside.iter().any(|&v| v != 0) { metrics::density(&maps, Region::Mask(&outside)) } else { 0.0 };

    // Siluetas solo donde pueden cambiar el veredicto: una caja sin detalle nítido no puede tener una
    // joya nítida y se mide tal cual. Los grupos más seguros primero, como mucho SAM_MAX por cuadro.
    let boxes: Vec<[i64; 4]> = gs.iter().map(|g| g.2).collect();
    let mut sils: Vec<Option<Vec<u8>>> = vec![None; gs.len()];
    let reused = if segmenter.is_some() { reuse_silhouettes(&state.sil, t, &boxes, w, h) } else { None };
    let fresh = reused.is_none();
    if let Some(r) = reused {
        sils = r;
    } else if let Some(seg) = segmenter {
        let mut want = Vec::new();
        for (i, (_, _, r)) in gs.iter().enumerate() {
            if want.len() < SAM_MAX && metrics::laplacian_peak(&maps.lap, Region::Rect(ur(r)), 99.0) * ls >= SHARP_LAP / 2.0 {
                want.push(i);
            }
        }
        // una caja por llamada: con DirectML el decodificador de SAM con 2-3 cajas tarda ~4 veces más que
        // con una (el embedding del cuadro se calcula una sola vez y se reutiliza entre cajas)
        for i in want {
            let r = gs[i].2;
            // un fallo de SAM no para el análisis (la caja se mide entera, como antes), pero se cuenta:
            // antes se tragaba sin dejar rastro
            let m = match seg.silhouette(frame, [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]) {
                Ok(m) => Some(m).filter(|m| m.len() == w * h),
                Err(_) => {
                    state.sam_fallos += 1;
                    None
                }
            };
            sils[i] = m.and_then(|m| {
                // la silueta solo cuenta dentro de su caja
                let [x0, y0, x1, y1] = ur(&r);
                let mut cut = vec![0u8; w * h];
                let mut n = 0usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        if m[y * w + x] != 0 {
                            cut[y * w + x] = 1;
                            n += 1;
                        }
                    }
                }
                (n >= 30).then_some(cut)
            });
        }
        state.sil = SilCache { t, boxes: boxes.clone(), sils: sils.clone(), valid: true };
    }
    let _ = fresh;

    for ((main, extras, r), sil) in gs.iter().zip(&sils) {
        let rr = ur(r);
        let [x0, y0, x1, y1] = rr;
        let grown;
        let (region, masked, mask_frac) = match sil {
            Some(sil) => {
                grown = grow_in_rect(sil, w, h, rr);
                let n = grown.iter().filter(|&&v| v != 0).count();
                (Region::RectMask(rr, &grown), true, n as f64 / ((x1 - x0) * (y1 - y0)) as f64)
            }
            None => (Region::Rect(rr), false, 0.0),
        };
        let edge = metrics::density(&maps, region);
        let (cx, cy) = ((r[0] + r[2]) as f64 / 2.0, (r[1] + r[3]) as f64 / 2.0);
        let diag = ((r[2] - r[0]) as f64).hypot((r[3] - r[1]) as f64);
        let mut speed = 0.0;
        if had_prev && !prev_boxes.is_empty() && opts.dt > 0.0 {
            // la misma joya en la muestra anterior = el centro de caja más cercano
            let mut best: Option<(f64, f64, f64)> = None;
            for b in &prev_boxes {
                let c = ((b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0);
                let d = (c.0 - cx).powi(2) + (c.1 - cy).powi(2);
                if best.map(|x| d < x.2).unwrap_or(true) {
                    best = Some((c.0, c.1, d));
                }
            }
            let (px, py, _) = best.unwrap();
            speed = (cx - px).hypot(cy - py) / diag.max(1.0) / opts.dt;
        }
        let worn = people.iter().any(|p| p.bbox[0] as f64 <= cx && cx <= p.bbox[2] as f64 && p.bbox[1] as f64 <= cy && cy <= p.bbox[3] as f64);
        s.jewels.push(Jewel {
            label: main.label.clone(),
            with_labels: Some(extras.clone()),
            conf: main.conf,
            bbox: *r,
            zoom: Some(main.zoom),
            area_frac: Some(((r[2] - r[0]) * (r[3] - r[1])) as f64 / (w * h) as f64),
            edge: Some(edge),
            ratio: Some(edge / bg.max(1e-3)),
            masked: Some(masked),
            mask_frac: Some(mask_frac),
            lap_var: Some(metrics::laplacian_variance(&maps.lap, Some(region)) * ls * ls),
            lap_p99: Some(metrics::laplacian_peak(&maps.lap, region, 99.0) * ls),
            anisotropy: Some(metrics::blur_anisotropy(&gray, w, rr)),
            speed: Some(speed),
            touches_edge: Some(r[0] <= 2 || r[1] <= 2 || r[2] >= w as i64 - 2 || r[3] >= h as i64 - 2),
            worn: Some(worn),
            ..Default::default()
        });
    }
    state.prev_boxes = s.jewels.iter().map(|j| j.bbox).collect();
    s.n_sharp = s.jewels.iter().filter(|j| j.lap_p99.unwrap_or(0.0) >= SHARP_LAP).count() as i64;

    // protagonista = la joya que mejor sostiene el plano: nítida, segura, no diminuta
    let key = |j: &Jewel| j.lap_p99.unwrap_or(0.0) * j.conf * (j.area_frac.unwrap_or(0.0) / 0.01).min(1.0);
    let mut hero = &s.jewels[0];
    for j in &s.jewels[1..] {
        if key(j) > key(hero) {
            hero = j;
        }
    }
    let hero = hero.clone();
    s.jewel = true;
    s.label = hero.label.clone();
    s.with_labels = hero.with_labels.clone().unwrap_or_default();
    s.conf = hero.conf;
    s.bbox = hero.bbox.to_vec();
    s.area_frac = hero.area_frac.unwrap_or(0.0);
    s.touches_edge = hero.touches_edge.unwrap_or(false);
    s.worn = hero.worn.unwrap_or(false);
    s.jewel_edge_density = hero.edge.unwrap_or(0.0);
    s.bg_edge_density = bg;
    s.focus_ratio = hero.ratio.unwrap_or(0.0);
    s.jewel_lap_var = hero.lap_var.unwrap_or(0.0);
    s.anisotropy = hero.anisotropy.unwrap_or(1.0);
    s.jewel_speed = hero.speed.unwrap_or(0.0);
    s.jewel_lap_p99 = hero.lap_p99.unwrap_or(0.0);
    s.masked = hero.masked.unwrap_or(false);
    s.mask_frac = hero.mask_frac.unwrap_or(0.0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(b: [i64; 4], conf: f64, label: &str) -> Det {
        Det { bbox: b, conf, label: label.into(), zoom: false, verified: false }
    }

    /// Detector cuyo SAM siempre falla (modelo que no abre).
    struct SinSam;

    impl Detector for SinSam {
        fn detect(&mut self, _: &Frame) -> anyhow::Result<Vec<crate::types::Detection>> {
            Ok(vec![])
        }
        fn silhouette(&mut self, _: &Frame, _: [f32; 4]) -> anyhow::Result<Vec<u8>> {
            anyhow::bail!("SAM no disponible")
        }
        fn pose(&mut self, _: &Frame) -> anyhow::Result<Vec<crate::types::Pose>> {
            Ok(vec![])
        }
        fn verify(&mut self, _: &Frame, _: [f32; 4]) -> anyhow::Result<f32> {
            Ok(0.0)
        }
        fn speech(&mut self, _: &[f32]) -> anyhow::Result<Vec<f32>> {
            Ok(vec![])
        }
    }

    /// Si SAM falla, la muestra se mide sobre la caja (sin máscara) y el fallo queda contado.
    #[test]
    fn fallo_de_sam_se_cuenta_y_se_mide_la_caja() {
        let (w, h) = (320usize, 180usize);
        // tablero de ajedrez de 2 px dentro de la caja: detalle nítido, así que se pide silueta
        let mut data = vec![128u8; w * h * 3];
        for y in 40..140 {
            for x in 100..220 {
                let v = if (x / 2 + y / 2) % 2 == 0 { 20 } else { 235 };
                data[(y * w + x) * 3..(y * w + x) * 3 + 3].fill(v);
            }
        }
        let frame = Frame { width: w as u32, height: h as u32, data };
        let dets = vec![det([100, 40, 220, 140], 0.9, "anillo")];
        let mut state = MeasureState::default();
        let opts = MeasureOpts { dt: 0.25, min_conf: 0.15, lap_scale: 1.0, use_sam: true };
        let mut sam = SinSam;
        let s = measure(0.0, &frame, &dets, &mut state, &opts, Some(&mut sam), false);
        assert_eq!(s.jewels.len(), 1);
        assert_eq!(s.jewels[0].masked, Some(false), "sin silueta: medida sobre la caja");
        assert!(state.sam_fallos >= 1, "el fallo de SAM queda contado");
    }

    #[test]
    fn grupos_como_python() {
        // dije sobre cadena: gana el dije (más específico), la cadena queda en "with"
        let d = vec![det([10, 10, 50, 50], 0.5, "cadena"), det([20, 20, 40, 40], 0.3, "dije"), det([100, 100, 120, 120], 0.4, "anillo"), det([0, 0, 5, 5], 0.9, "persona")];
        let g = groups(&d, 0.15);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].0.label, "dije");
        assert_eq!(g[0].1, vec!["cadena".to_string()]);
        assert_eq!(g[1].0.label, "anillo");
    }

    #[test]
    fn desplazar_mascara() {
        let m = vec![1, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(shift_mask(&m, 3, 3, 1, 1), vec![0, 0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(shift_mask(&m, 3, 3, -1, 0), vec![0; 9]);
    }
}
