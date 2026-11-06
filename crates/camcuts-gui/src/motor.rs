//! Puente entre la ventana y `camcuts-core`.
//!
//! La ventana solo necesita una vista sencilla de cada clip analizado (muestras con su tiempo, cajas de
//! joya y nitidez) y de cada tramo aprobado. Este módulo traduce los tipos del núcleo a esa vista, de
//! modo que el resto de la interfaz no depende de los detalles internos de `camcuts-core`.
//!
//! Todo lo que es "cálculo de CAMCUTS" (criterios, puntuación, tramos, exportar, convertir color,
//! detectar perfil) se delega al núcleo; aquí no se reimplementa.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use camcuts_core as nucleo;

// ---------------------------------------------------------------------------------------------
// Vista de los datos analizados
// ---------------------------------------------------------------------------------------------

/// Conversión de color asignada a un clip (formato de `colores.json`: `{"id": ..., "cube": ...}`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorSpec {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cube: Option<String>,
}

impl ColorSpec {
    /// La conversión automática (la que se usa si el editor no eligió otra).
    pub fn auto() -> Self {
        Self { id: "auto".into(), cube: None }
    }
    /// ¿Es la misma elección? (igual que `project.same_color`: ausente = automática).
    pub fn igual(a: Option<&ColorSpec>, b: Option<&ColorSpec>) -> bool {
        let auto = ColorSpec::auto();
        let (a, b) = (a.unwrap_or(&auto), b.unwrap_or(&auto));
        a.id == b.id && a.cube.as_deref().filter(|s| !s.is_empty()) == b.cube.as_deref().filter(|s| !s.is_empty())
    }
}

/// Una joya (o, en la edición Bodas, un sujeto) detectada en una muestra, en píxeles de análisis
/// (lado largo 960).
#[derive(Clone, Debug)]
pub struct JoyaVista {
    /// Etiqueta estable del clip (tras seguir el objeto entre cuadros o fijarla el editor).
    pub etiqueta: String,
    /// La etiqueta que dio el detector a ESTA caja en ESTE cuadro (la que se dibuja sobre la caja).
    pub propia: String,
    /// Confianza del detector para esta caja.
    pub conf: f32,
    pub caja: [f32; 4],
    /// Nitidez de 0 a 1 (componente "sharp" del criterio).
    pub nitidez: f32,
    /// ¿Es la joya principal de la muestra?
    pub principal: bool,
}

/// Lo que la ventana necesita de cada muestra analizada.
#[derive(Clone, Debug)]
pub struct MuestraVista {
    pub t: f64,
    pub joya: bool,
    pub log: bool,
    pub lap_p99: f64,
    pub joyas: Vec<JoyaVista>,
    /// Edición Bodas: momento estable de la muestra (vacío en joyas o en cachés sin momentos).
    pub momento: String,
    /// Edición Bodas: hay una cara a la vista.
    pub cara: bool,
}

/// Un clip analizado (una entrada de `cache/*.json`).
pub struct Entrada {
    pub ruta: PathBuf,
    pub ruta_decodificar: Option<PathBuf>,
    pub tc_inicio: String,
    pub fps: (u32, u32),
    pub ancho: u32,
    pub alto: u32,
    pub duracion: f64,
    pub por_segundo: f64,
    pub color: Option<ColorSpec>,
    /// Detector de joyas con el que se analizó (clave `detector` de la caché; sin ella, YOLO-World).
    pub detector: nucleo::detector::DetectorKind,
    /// Proxy de vista previa (`cache/proxies/<nombre>.mp4`), exista o no todavía.
    pub proxy: PathBuf,
    pub muestras: Vec<MuestraVista>,
    /// La entrada completa tal como la entiende el núcleo (para puntuar y exportar).
    pub nucleo: nucleo::project::ClipEntry,
}

impl Entrada {
    /// Cadencia del clip en fps.
    pub fn cadencia(&self) -> f64 {
        self.fps.0 as f64 / self.fps.1.max(1) as f64
    }
    /// Muestra más cercana a un instante.
    pub fn muestra_en(&self, t: f64) -> Option<&MuestraVista> {
        if self.muestras.is_empty() {
            return None;
        }
        let i = (t * self.por_segundo).round().max(0.0) as usize;
        self.muestras.get(i.min(self.muestras.len() - 1))
    }
    /// Ancho del cuadro de análisis (las cajas de joya están en estos píxeles).
    pub fn ancho_analisis(&self) -> f32 {
        let lado = self.ancho.max(self.alto).max(1) as f32;
        (self.ancho as f32 * 960.0 / lado).round()
    }
    /// ¿El clip es LOG (más de la mitad de sus muestras planas)?
    pub fn es_log(&self) -> bool {
        self.muestras.iter().filter(|m| m.log).count() * 2 > self.muestras.len()
    }
    /// Parte de las muestras con joya visible.
    pub fn parte_joya(&self) -> f64 {
        self.muestras.iter().filter(|m| m.joya).count() as f64 / self.muestras.len().max(1) as f64
    }
}

/// Un tramo aprobado.
#[derive(Clone, Debug)]
pub struct Tramo {
    pub inicio: f64,
    pub fin: f64,
    pub media: f64,
    pub pico: f64,
    pub etiqueta: String,
    pub plano: String,
    pub presentes: Vec<String>,
    /// Edición Bodas: momento dominante del tramo.
    pub momento: String,
    /// El tramo tal como lo devuelve el núcleo (para exportar).
    pub nucleo: nucleo::scoring::Segment,
}

// ---------------------------------------------------------------------------------------------
// Criterio (preset + ajustes del editor)
// ---------------------------------------------------------------------------------------------

/// Nombres de los criterios y su etiqueta en la ventana (igual que `CRITERIA` de Python).
pub const CRITERIOS: [(&str, &str); 3] = [("joyas", "Equilibrado"), ("social", "Redes · cortes cortos"), ("todo", "Permisivo")];

/// Los pesos en el orden y con los nombres de la ventana de Python.
pub const PESOS: [(&str, &str); 6] = [
    ("sharp", "Nitidez de la joya"),
    ("size", "Tamaño en cuadro"),
    ("exposure", "Exposición"),
    ("still", "Quietud"),
    ("ratio", "Joya más nítida que el fondo"),
    ("aniso", "Sin barrido de movimiento"),
];

/// Tipos de joya para filtros y para "Joya de este clip" (igual que `perfil.SUBJECT_TYPES`).
pub const JOYAS: [&str; 12] =
    ["anillo", "dije", "cadena", "collar", "arete", "topo", "candonga", "pulsera", "tobillera", "rosario", "piercing", "joya"];

/// Tipos de plano (igual que `perfil.SHOTS`).
pub const PLANOS: [(&str, &str); 3] = [("producto", "Producto"), ("modelo", "Modelo"), ("hablando", "Hablando")];

// --- edición Bodas ---------------------------------------------------------------------------------

/// Criterios de la edición Bodas.
pub const CRITERIOS_BODAS: [(&str, &str); 3] = [("bodas", "Equilibrado"), ("social", "Highlight · cortes cortos"), ("todo", "Permisivo")];
/// Pesos de la edición Bodas (la nitidez de las caras manda; cámara estable y exposición importan).
pub const PESOS_BODAS: [(&str, &str); 7] = [
    ("sharp", "Nitidez del sujeto"),
    ("exposure", "Exposición (caras, vestido)"),
    ("steady", "Cámara estable"),
    ("size", "Tamaño en cuadro"),
    ("aniso", "Sin barrido de movimiento"),
    ("still", "Sujeto quieto"),
    ("ratio", "Sujeto más nítido que el fondo"),
];
/// Planos de la edición Bodas.
pub const PLANOS_BODAS: [(&str, &str); 5] =
    [("hablando", "Hablando"), ("primer_plano", "Primer plano"), ("plano_medio", "Plano medio"), ("plano_general", "Plano general"), ("detalle", "Detalle")];

/// ¿Es la edición Bodas?
pub fn bodas() -> bool {
    nucleo::edicion::es_bodas()
}
/// Nombre de la app de esta edición (el campo `app` de los proyectos).
pub fn nombre_app() -> &'static str {
    nucleo::edicion::actual().nombre_app()
}
/// Criterios de esta edición: (clave del preset, nombre en la ventana).
pub fn criterios() -> &'static [(&'static str, &'static str)] {
    if bodas() { &CRITERIOS_BODAS } else { &CRITERIOS }
}
/// Criterio por defecto de esta edición ("joyas" o "bodas").
pub fn criterio_defecto() -> &'static str {
    criterios()[0].0
}
/// Pesos de esta edición: (clave, nombre en la ventana).
pub fn pesos() -> &'static [(&'static str, &'static str)] {
    if bodas() { &PESOS_BODAS } else { &PESOS }
}
/// Tipos del filtro y de "… de este clip": joyas, o los momentos de la boda.
pub fn tipos() -> &'static [&'static str] {
    if bodas() { &nucleo::bodas::MOMENTOS } else { &JOYAS }
}
/// Planos de esta edición.
pub fn planos() -> &'static [(&'static str, &'static str)] {
    if bodas() { &PLANOS_BODAS } else { &PLANOS }
}
/// Clave del filtro de tipos en el proyecto (`joyas` como Python; `momentos` en Bodas).
pub fn clave_tipos() -> &'static str {
    if bodas() { "momentos" } else { "joyas" }
}

/// Detectores de joyas que se ofrecen en la edición de joyas, con su nombre en la ventana (el propio,
/// el predeterminado, primero).
pub const DETECTORES: [(nucleo::detector::DetectorKind, &str); 2] =
    [(nucleo::detector::DetectorKind::Propio, "Propio"), (nucleo::detector::DetectorKind::World, "Clásico")];

/// Ayuda del selector de detector.
pub const AYUDA_DETECTOR: &str =
    "Propio (predeterminado): entrenado con el material de la marca, más preciso y más rápido. Si en un clip casi nunca da la joya nítida (tomas macro con poco foco), ese clip se analiza también con el clásico. Clásico: el detector general de siempre; con la GPU ocupada (DaVinci abierto) se analiza con el propio, porque el clásico grande no cabe";

/// Nombre en la ventana de un detector ("Propio" o "Clásico").
pub fn nombre_detector(k: nucleo::detector::DetectorKind) -> &'static str {
    DETECTORES.iter().find(|d| d.0 == k).map(|d| d.1).unwrap_or("Clásico")
}

/// Modos de exportación (igual que `EXPORT_MODES` de Python).
pub const MODOS_EXPORTAR: [(&str, &str); 3] = [
    ("v1v2", "V2 aprobados · V1 el resto (sin duplicar)"),
    ("aprobados", "Solo los tramos aprobados"),
    ("marcadores", "V1 clips completos con marcadores"),
];

/// Lo que el editor ajusta en la columna "Pesos": un preset y sus valores tocados.
#[derive(Clone, Debug, PartialEq)]
pub struct Criterio {
    pub nombre: String,
    pub umbral: f64,
    pub min_s: f64,
    pub max_s: f64,
    pub por_clip: u32,
    pub tolerancia: f64,
    pub suavizado: f64,
    pub pesos: BTreeMap<String, f64>,
    pub joyas: Vec<String>,
    pub planos: Vec<String>,
}

impl Criterio {
    /// Los valores de un preset del núcleo (con los mismos topes que la ventana de Python).
    pub fn base(nombre: &str) -> Self {
        let p = preset_nucleo(nombre);
        Self {
            nombre: p.name.clone(),
            umbral: p.score_threshold,
            min_s: p.min_segment_seconds,
            max_s: p.max_segment_seconds.min(120.0),
            por_clip: p.max_segments_per_clip.min(99) as u32,
            tolerancia: p.gap_tolerance_seconds,
            suavizado: p.smooth_seconds,
            pesos: pesos().iter().map(|(k, _)| (k.to_string(), p.weight(k))).collect(),
            joyas: Vec::new(),
            planos: Vec::new(),
        }
    }

    /// El preset del núcleo con los ajustes del editor (igual que `_preset()` de Python).
    pub fn preset(&self) -> nucleo::scoring::Preset {
        let mut p = preset_nucleo(&self.nombre);
        p.score_threshold = self.umbral;
        p.min_segment_seconds = self.min_s;
        p.max_segment_seconds = self.max_s;
        p.max_segments_per_clip = self.por_clip as usize;
        p.gap_tolerance_seconds = self.tolerancia;
        p.smooth_seconds = self.suavizado;
        for (k, v) in &self.pesos {
            p.set_weight(k, *v);
        }
        p.jewel_types = self.joyas.clone();
        p.shot_types = self.planos.clone();
        p
    }
}

/// Preset del núcleo con la calibración del editor (`models/calibration.json`), como los PRESETS de Python.
pub fn preset_nucleo(nombre: &str) -> nucleo::scoring::Preset {
    let cal = nucleo::cli::load_calibration();
    nucleo::scoring::preset(nombre, cal.as_ref())
        .or_else(|| nucleo::scoring::preset(criterio_defecto(), cal.as_ref()))
        .expect("el núcleo siempre trae el preset por defecto de su edición")
}

// ---------------------------------------------------------------------------------------------
// Cargar la caché y puntuar
// ---------------------------------------------------------------------------------------------

/// Convierte una entrada del núcleo a la vista de la ventana.
fn vista(e: nucleo::project::ClipEntry, proxy: PathBuf) -> Entrada {
    let p = preset_nucleo(criterio_defecto());
    let muestras = e
        .samples
        .iter()
        .map(|s| {
            // la caja principal de la muestra (vacía si no hay joya)
            let principal = &s.bbox;
            let joyas = s
                .jewels
                .iter()
                .map(|j| JoyaVista {
                    etiqueta: j.label.clone(),
                    // lo que el detector dijo de esta caja en este cuadro (antes de estabilizar o forzar)
                    propia: j.raw_label.clone().unwrap_or_else(|| j.label.clone()),
                    conf: j.conf as f32,
                    caja: [j.bbox[0] as f32, j.bbox[1] as f32, j.bbox[2] as f32, j.bbox[3] as f32],
                    nitidez: nucleo::scoring::sharpness_of(j.lap_p99.unwrap_or(0.0), &p) as f32,
                    principal: !principal.is_empty() && principal.as_slice() == j.bbox.as_slice(),
                })
                .collect();
            MuestraVista {
                t: s.t,
                joya: s.jewel,
                log: s.log.unwrap_or(false),
                lap_p99: s.jewel_lap_p99,
                joyas,
                momento: nucleo::bodas::momento(s).to_string(),
                cara: s.face,
            }
        })
        .collect();
    Entrada {
        ruta: PathBuf::from(&e.path),
        ruta_decodificar: e.decode_path.as_ref().map(PathBuf::from),
        tc_inicio: e.start_tc.clone().unwrap_or_default(),
        fps: (e.fps[0] as u32, e.fps[1] as u32),
        ancho: e.width,
        alto: e.height,
        duracion: e.duration,
        por_segundo: e.per_second,
        color: e.color.as_ref().and_then(|c| serde_json::from_value(c.clone()).ok()),
        detector: nucleo::detector::DetectorKind::de_cache(&e),
        proxy,
        muestras,
        nucleo: e,
    }
}

/// Lee todas las entradas de `cache/*.json` de una carpeta de resultados, aplicando las mismas
/// correcciones que la ventana de Python (refinar joyas, estabilizar etiquetas, tipo forzado).
/// Devuelve un mapa ruta-en-minúsculas -> entrada.
///
/// Las cachés de la otra edición (marca `edition`) no se cargan: sus clips salen como pendientes y
/// Analizar los vuelve a analizar con esta edición, en vez de mostrar joyas en una boda o al revés.
pub fn cargar_cache(out: &Path, tipos: &BTreeMap<String, String>) -> HashMap<String, Entrada> {
    let cache = out.join("cache");
    let mut r: HashMap<String, Entrada> = HashMap::new();
    // fecha de cada caché elegida: si dos archivos son del mismo clip (p. ej. una caché antigua sin hash
    // y la nueva, o rutas que solo difieren en mayúsculas), gana la más reciente, no la última leída
    let mut fechas: HashMap<String, std::time::SystemTime> = HashMap::new();
    let Ok(rd) = std::fs::read_dir(&cache) else { return r };
    for f in rd.flatten() {
        let ruta = f.path();
        if ruta.extension().map(|e| !e.eq_ignore_ascii_case("json")).unwrap_or(true) {
            continue;
        }
        let Ok(mut e) = nucleo::project::read_entry(&ruta) else { continue };
        if !nucleo::edicion::cache_vigente(&e) {
            continue;
        }
        let clave = e.path.to_lowercase();
        let fecha = f.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if fechas.get(&clave).map(|&vieja| vieja >= fecha).unwrap_or(false) {
            continue;
        }
        nucleo::scoring::refine_jewels(&mut e.samples, e.width, e.height);
        nucleo::scoring::stabilize_labels(&mut e.samples, e.per_second, 1.0);
        if let Some(forzado) = tipos.get(&clave) {
            nucleo::scoring::force_type(&mut e.samples, forzado);
        }
        let stem = ruta.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let proxy = cache.join("proxies").join(format!("{stem}.mp4"));
        fechas.insert(clave.clone(), fecha);
        r.insert(clave, vista(e, proxy));
    }
    r
}

/// Curva de puntuación (t, puntuación) y tramos aprobados de un clip con un criterio.
pub fn puntuar(e: &Entrada, p: &nucleo::scoring::Preset) -> (Vec<(f64, f64)>, Vec<Tramo>) {
    let s = &e.nucleo.samples;
    let curva = nucleo::scoring::scored_samples(s, p, e.por_segundo).into_iter().map(|r| (r.0, r.1)).collect();
    let tramos = nucleo::scoring::build_segments(s, p, e.por_segundo, e.duracion)
        .into_iter()
        .map(|g| Tramo {
            inicio: g.start,
            fin: g.end,
            media: g.mean_score,
            pico: g.peak_score,
            etiqueta: g.label.clone(),
            plano: g.shot.clone(),
            presentes: g.present.clone(),
            momento: g.moment.clone(),
            nucleo: g,
        })
        .collect();
    (curva, tramos)
}

// ---------------------------------------------------------------------------------------------
// Exportar
// ---------------------------------------------------------------------------------------------

/// Escribe el OTIO y el CSV de los clips analizados. Devuelve el número de tramos exportados y, si el
/// OTIO se escribió pero el CSV no se pudo reemplazar (abierto en Excel), el motivo: son dos cosas
/// distintas y el editor debe saber que la línea de tiempo sí está.
pub fn exportar(clips: &[(&Entrada, &[Tramo])], ruta: &Path, nombre: &str, cadencia: f64, modo: &str) -> anyhow::Result<(usize, Option<String>)> {
    let resultados: Vec<(nucleo::sampler::ClipInfo, Vec<nucleo::scoring::Segment>)> = clips
        .iter()
        .map(|(e, t)| (e.nucleo.clip_info(), t.iter().map(|x| x.nucleo.clone()).collect()))
        .collect();
    // con audio, como `export.write_otio` de Python por defecto
    let n = nucleo::export::write_otio(&resultados, ruta, nombre, Some(cadencia), modo, true)?;
    let csv = nucleo::export::write_csv(&resultados, &ruta.with_extension("csv")).err().map(|e| format!("{e:#}"));
    Ok((n, csv))
}

// ---------------------------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------------------------

/// (id, etiqueta) de cada conversión de color que ofrece el núcleo.
pub fn transformaciones() -> Vec<(String, String)> {
    nucleo::color::TRANSFORMS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// Etiqueta de una conversión.
pub fn etiqueta_color(id: &str) -> String {
    transformaciones().into_iter().find(|(i, _)| i == id).map(|(_, l)| l).unwrap_or_else(|| id.to_owned())
}

/// Nombre corto para la tabla y el transporte: "Sony S-Log3 / S-Gamut3.Cine → Rec.709" -> "S-Log3".
pub fn color_corto(id: &str) -> String {
    let l = etiqueta_color(id);
    let l = l.split('→').next().unwrap_or("").split('/').next().unwrap_or("").trim().to_owned();
    if l.contains(' ') {
        l.split_whitespace().last().unwrap_or("").to_owned()
    } else {
        l
    }
}

/// Lee el perfil de color de la cámara (XML de Sony, ffprobe, Apple Log).
pub fn detectar_perfil(ruta: &Path) -> Option<String> {
    nucleo::color::detect_profile(&ruta.to_string_lossy())
}

/// Función que gradúa un cuadro RGB en su sitio, con la conversión asignada al clip.
pub type Graduador = Box<dyn FnMut(&mut [u8], u32, u32) + Send>;

/// Conversión que de verdad se usa en el visor: con "auto", la que dicen los metadatos de la cámara
/// (XML de Sony, etiquetas HLG, Apple Log); si no dicen nada, "auto" (estirar contraste si es LOG).
///
/// Solo para ver: el análisis sigue usando "auto" tal cual, porque los umbrales de nitidez se
/// calibraron con él. Antes el visor estiraba el contraste de un S-Log3 sin convertirlo y la imagen
/// salía gris y lavada (2026-11-05).
pub fn conversion_visor(spec: &ColorSpec, ruta_clip: &Path) -> String {
    if spec.id == "auto" {
        if let Some(p) = detectar_perfil(ruta_clip) {
            if p != "auto" && p != "cube" {
                return p;
            }
        }
    }
    spec.id.clone()
}

/// Texto de la píldora de color del transporte: `elegido` es la conversión asignada al clip y `conv` la
/// que aplica el visor ([`conversion_visor`]). Con "auto" y un perfil de cámara, "S-Log3 → 709 (auto)".
pub fn texto_pildora_color(elegido: &str, conv: &str, es_log: bool, automatico: &str) -> String {
    match (elegido, conv) {
        ("auto", "auto") if es_log => "LOG → contraste".to_owned(),
        ("auto", "auto") => automatico.to_owned(),
        ("auto", "none") | ("none", _) => "Sin conversión".to_owned(),
        ("auto", c) => format!("{} → 709 (auto)", color_corto(c)),
        ("cube", _) => "LUT .cube".to_owned(),
        (id, _) => format!("{} → 709", color_corto(id)),
    }
}

/// Prepara el graduador del visor: "none" lo deja tal cual, "auto" usa el perfil de la cámara si se
/// conoce (si no, estira el contraste solo si el clip es LOG) y el resto usa el conversor del núcleo.
///
/// El visor decodifica con ffmpeg (el proxy conserva el rango y la matriz del original), así que la
/// conversión parte de cómo decodifica ffmpeg ese clip, no de los valores por defecto de OpenCV: con
/// un S-Log3 de rango completo de Sony, suponer rango limitado dejaba la imagen más plana.
pub fn graduador(spec: &ColorSpec, clip_log: bool, ruta_clip: &Path) -> Option<Graduador> {
    let id = conversion_visor(spec, ruta_clip);
    let id = match id.as_str() {
        "none" | "" => return None,
        "auto" if !clip_log => return None,
        id => id,
    };
    // "auto" no mira la decodificación; las demás sí
    let dec = if id == "auto" { nucleo::color::Decoding::default() } else { nucleo::color::decoding_for(&ruta_clip.to_string_lossy(), "ffmpeg").unwrap_or_default() };
    let conv = nucleo::color::make_converter(id, spec.cube.as_deref(), &dec).ok()?;
    Some(Box::new(move |rgb: &mut [u8], w: u32, h: u32| {
        let bgr = rgb_a_bgr(rgb);
        let salida = conv.convert(&bgr, w as usize, h as usize);
        if salida.len() == rgb.len() {
            bgr_a_rgb(&salida, rgb);
        }
    }))
}

/// El núcleo trabaja en BGR (como OpenCV); el visor en RGB.
fn rgb_a_bgr(rgb: &[u8]) -> Vec<u8> {
    let mut v = rgb.to_vec();
    for px in v.chunks_exact_mut(3) {
        px.swap(0, 2);
    }
    v
}

fn bgr_a_rgb(bgr: &[u8], rgb: &mut [u8]) {
    for (d, s) in rgb.chunks_exact_mut(3).zip(bgr.chunks_exact(3)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// "Automático" en el visor = el perfil que dice la cámara; lo elegido a mano se respeta.
    #[test]
    fn el_visor_usa_el_perfil_de_la_camara() {
        let dir = std::env::temp_dir().join(format!("camcuts_visor_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("C0001.MP4");
        std::fs::write(&clip, b"").unwrap();
        std::fs::write(
            dir.join("C0001M01.XML"),
            r#"<Item name="CaptureGammaEquation" value="s-log3-cine"/><Item name="CaptureColorPrimaries" value="s-gamut3-cine"/>"#,
        )
        .unwrap();
        let spec = |id: &str| ColorSpec { id: id.into(), cube: None };
        assert_eq!(conversion_visor(&ColorSpec::auto(), &clip), "slog3_sgamut3cine");
        assert_eq!(conversion_visor(&spec("none"), &clip), "none");
        assert_eq!(conversion_visor(&spec("hlg"), &clip), "hlg");
        // sin metadatos se queda en "auto"
        let otro = dir.join("sin_nada.mp4");
        assert_eq!(conversion_visor(&ColorSpec::auto(), &otro), "auto");
        // y el graduador del S-Log3 existe aunque el clip no se haya marcado como LOG
        assert!(graduador(&ColorSpec::auto(), false, &clip).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
