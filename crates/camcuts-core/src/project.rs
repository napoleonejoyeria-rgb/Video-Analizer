//! Archivos de la carpeta de resultados, con el mismo esquema que Python (port de `project.py` y de las
//! partes de `__main__.py`/`gui.py` que leen y escriben JSON):
//!
//! - `cache/<clip>_<sha1>.json`: las medidas de un clip ([`ClipEntry`]);
//! - `*.camcuts`: el proyecto (formato `camcuts-project`, versión 1), rutas absolutas y relativas;
//! - `colores.json`: conversión de color por clip (ruta en minúsculas → `{"id", "cube"}`);
//! - `clip_types.json`: tipo de joya forzado por clip (ruta en minúsculas → tipo);
//! - `clips_meta.json`: códec, contenedor, resolución, fps y duración por clip.
//!
//! Así una carpeta hecha por Python se abre en Rust y al revés.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::jewel::Sample;
use crate::pyfmt;
use crate::sampler::{ClipInfo, Fraction};

/// Extensión del proyecto.
pub const EXT: &str = ".camcuts";
/// Marca de formato del proyecto.
pub const FORMAT: &str = "camcuts-project";
pub const VERSION: i64 = 1;
/// Nombre de la app (de `perfil.py`).
pub const APP_NAME: &str = "CAMCUTS";

/// Conversión automática (sin entrada en `colores.json`).
pub fn auto_color() -> Value {
    serde_json::json!({"id": "auto"})
}

/// Dos elecciones de conversión de color son la misma (sin elección = automático).
pub fn same_color(a: Option<&Value>, b: Option<&Value>) -> bool {
    let auto = auto_color();
    let a = a.filter(|v| !v.is_null()).unwrap_or(&auto);
    let b = b.filter(|v| !v.is_null()).unwrap_or(&auto);
    let cube = |v: &Value| v.get("cube").filter(|c| truthy(c)).cloned();
    a.get("id") == b.get("id") && cube(a) == cube(b)
}

/// Verdad al estilo de Python (`None`, `""`, `0`, `[]`, `{}` son falsos).
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Medidas de un clip tal como se guardan en `cache/*.json` (mismo orden de claves que Python).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipEntry {
    pub path: String,
    #[serde(default)]
    pub decode_path: Option<String>,
    /// Falta en cachés anteriores al timecode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_tc: Option<String>,
    pub fps: [i64; 2],
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub per_second: f64,
    #[serde(default)]
    pub speech: Vec<[f64; 2]>,
    pub samples: Vec<Sample>,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default = "default_decode")]
    pub decode: String,
    #[serde(default = "default_one")]
    pub lap_scale: f64,
    #[serde(default)]
    pub reused: i64,
    /// `{"id": ..., "cube": ...}`; falta en cachés antiguas (= automático).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Value>,
    /// Claves que esta versión no conoce: se conservan.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_decode() -> String {
    "cpu".into()
}
fn default_one() -> f64 {
    1.0
}

impl ClipEntry {
    /// Datos del clip (sin volver a sondear, salvo cachés sin timecode cuyo clip exista).
    pub fn clip_info(&self) -> ClipInfo {
        let mut info = ClipInfo {
            path: PathBuf::from(&self.path),
            width: self.width,
            height: self.height,
            fps: Fraction::new(self.fps[0], self.fps[1]),
            duration: self.duration,
            decode_path: self.decode_path.as_ref().filter(|s| !s.is_empty()).map(PathBuf::from),
            start_tc: self.start_tc.clone().unwrap_or_default(),
            rotation: 0,
            // solo para exportar: no se decodifica
            saltar_b: false,
        };
        if self.start_tc.is_none() && info.path.exists() {
            // cachés escritas antes del soporte de timecode
            if let Ok(p) = crate::sampler::probe(&info.path) {
                info.start_tc = p.start_tc;
            }
        }
        info
    }
}

/// Lee una caché de clip.
pub fn read_entry(path: &Path) -> Result<ClipEntry> {
    let bytes = std::fs::read(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("caché inválida: {}", path.display()))
}

/// Escribe un archivo de forma atómica: primero `<archivo>.tmp` y luego se renombra encima (en Windows,
/// `rename` reemplaza el destino). Si el proceso muere a mitad (Detener usa `taskkill /F`), queda el
/// archivo anterior entero o ninguno, nunca uno cortado que haga fallar la lectura.
///
/// Si el destino está abierto un momento por otro programa (antivirus, la app de Python leyéndolo),
/// `rename` falla: se reintenta unas veces y, si sigue sin poder, el `.tmp` se CONSERVA (con el trabajo
/// recién hecho) y el error dice dónde está, en vez de borrarlo y perder el análisis.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, bytes).with_context(|| format!("no se pudo escribir {}", tmp.display()))?;
    rename_retrying(&tmp, path, RENAME_TRIES, std::time::Duration::from_millis(100))
}

/// Intentos de `rename` en [`write_atomic`] (con 100 ms entre ellos: ~1 s en total).
const RENAME_TRIES: usize = 10;

/// `rename` con reintentos; si no lo consigue, deja `tmp` donde está y lo dice en el error.
fn rename_retrying(tmp: &Path, path: &Path, tries: usize, pause: std::time::Duration) -> Result<()> {
    let mut last = None;
    for k in 0..tries.max(1) {
        match std::fs::rename(tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if k + 1 < tries {
            std::thread::sleep(pause);
        }
    }
    Err(last.unwrap()).with_context(|| format!("no se pudo reemplazar {} (el resultado nuevo queda en {})", path.display(), tmp.display()))
}

/// Escribe una caché de clip como `json.dumps(entry)` de Python (de forma atómica).
pub fn write_entry(path: &Path, e: &ClipEntry) -> Result<()> {
    write_atomic(path, pyfmt::dumps(e)?)
}

/// Cachés legibles de una carpeta. Las dañadas (p. ej. cortadas por un análisis interrumpido antes de que
/// la escritura fuera atómica) se saltan con un aviso en stderr: cuentan como clips sin analizar.
pub fn read_entries_lenient(cache: &Path) -> Vec<ClipEntry> {
    let mut out = Vec::new();
    for p in cache_files(cache) {
        match read_entry(&p) {
            Ok(e) => out.push(e),
            Err(e) => eprintln!("aviso: se ignora una caché ilegible ({e:#}); ese clip se volverá a analizar"),
        }
    }
    out
}

/// Ruta normalizada para comparar como `WindowsPath ==` de Python: por componentes (da igual `/` o `\`)
/// y sin distinguir mayúsculas.
pub fn path_key(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()
}

/// `Path.resolve()` de Python: ruta absoluta canónica (sin prefijo `\\?\`), o absoluta si no existe.
pub fn resolve(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => {
            let s = c.to_string_lossy().to_string();
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                PathBuf::from(format!(r"\\{rest}"))
            } else if let Some(rest) = s.strip_prefix(r"\\?\") {
                PathBuf::from(rest)
            } else {
                c
            }
        }
        Err(_) => std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    }
}

/// Archivo de caché de un clip. El hash de la ruta separa clips con el mismo nombre de carpetas
/// distintas; las cachés antiguas (solo el nombre) se siguen encontrando si son de este mismo clip.
pub fn cache_file(cache: &Path, path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let legacy = cache.join(format!("{stem}.json"));
    if legacy.exists() {
        if let Ok(bytes) = std::fs::read(&legacy) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                // `WindowsPath ==` de Python no distingue mayúsculas: "D:\Contenido\…" y "D:\CONTENIDO\…"
                // son el mismo clip y deben dar la misma caché (si no, se reanaliza y se duplica)
                if v.get("path").and_then(|p| p.as_str()).map(|p| path_key(Path::new(p)) == path_key(path)).unwrap_or(false) {
                    return legacy;
                }
            }
        }
    }
    let key = resolve(path).to_string_lossy().to_lowercase();
    let digest = sha1_smol::Sha1::from(key.as_bytes()).digest().to_string();
    cache.join(format!("{stem}_{}.json", &digest[..8]))
}

/// Cachés de una carpeta en el orden de Python (`sorted(cache.glob("*.json"))`, sin distinguir mayúsculas).
pub fn cache_files(cache: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(cache)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file() && p.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false)).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    v
}

/// Lee un JSON de diccionario de la carpeta de resultados (vacío si falta o está roto).
pub fn read_dict(path: &Path) -> Map<String, Value> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// `colores.json` con las claves en minúsculas.
pub fn read_colors(path: &Path) -> Map<String, Value> {
    read_dict(path).into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect()
}

/// Escribe `colores.json` o `clip_types.json` como la ventana de Python (`indent=1`, sin escapar acentos).
pub fn write_dict_indented(path: &Path, data: &Map<String, Value>) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    write_atomic(path, pyfmt::dumps_with(data, Some(1), false)?)
}

/// Escribe `clips_meta.json` como Python (`json.dumps(meta)`).
pub fn write_meta(path: &Path, data: &Map<String, Value>) -> Result<()> {
    write_atomic(path, pyfmt::dumps(data)?)
}

/// Códec, contenedor, resolución, fps y duración de un clip para la tabla de clips (`probe_meta`).
pub fn probe_meta(path: &Path) -> Value {
    let Ok(out) = crate::sampler::run_quiet("ffprobe", &["-v", "error", "-print_format", "json", "-show_streams", "-show_format", &path.to_string_lossy()]) else {
        return serde_json::json!({"error": true});
    };
    let Ok(data) = serde_json::from_slice::<Value>(&out) else { return serde_json::json!({"error": true}) };
    let empty = Value::Object(Map::new());
    let v = data["streams"].as_array().and_then(|a| a.iter().find(|s| s["codec_type"] == "video")).unwrap_or(&empty);
    let (mut w, mut h) = (v["width"].as_i64().unwrap_or(0), v["height"].as_i64().unwrap_or(0));
    if v["side_data_list"].as_array().into_iter().flatten().any(|sd| matches!(sd["rotation"].as_i64().map(|r| r.abs()), Some(90) | Some(270))) {
        std::mem::swap(&mut w, &mut h);
    }
    let fps = v["r_frame_rate"].as_str().and_then(|s| Fraction::parse(s).ok()).map(|f| f.as_f64()).unwrap_or(0.0);
    let codec = match v["codec_name"].as_str() {
        Some("h264") => "H.264".to_string(),
        Some("hevc") => "H.265".to_string(),
        Some("prores") => "ProRes".to_string(),
        Some(o) => o.to_string(),
        None => "?".to_string(),
    };
    let container = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let duration = data["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    serde_json::json!({"codec": codec, "container": container, "res": format!("{w}×{h}"), "fps": fps, "duration": duration})
}

// ------------------------------------------------------------------------------------------------
// Proyecto .camcuts
// ------------------------------------------------------------------------------------------------

/// Componentes normalizados de una ruta absoluta para compararlos sin distinguir mayúsculas (Windows).
fn norm_parts(p: &Path) -> (String, Vec<String>) {
    let mut prefix = String::new();
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::Prefix(x) => prefix = x.as_os_str().to_string_lossy().to_lowercase(),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
        }
    }
    (prefix, parts)
}

/// `os.path.relpath(path, base)` de Windows; `None` si están en unidades distintas.
pub fn relpath(path: &str, base: &Path) -> Option<String> {
    let a = std::path::absolute(path).ok()?;
    let b = std::path::absolute(base).ok()?;
    let (pa, xa) = norm_parts(&a);
    let (pb, xb) = norm_parts(&b);
    if pa != pb {
        return None;
    }
    let common = xa.iter().zip(&xb).take_while(|(x, y)| x.to_lowercase() == y.to_lowercase()).count();
    let mut out: Vec<String> = vec!["..".into(); xb.len() - common];
    out.extend(xa[common..].iter().cloned());
    if out.is_empty() {
        return Some(".".into());
    }
    Some(out.join(std::path::MAIN_SEPARATOR_STR))
}

/// La ruta absoluta guardada si aún existe; si no, la relativa al proyecto; si no, la absoluta.
fn resolve_entry(entry: &Value, base: &Path) -> String {
    let absolute = entry.get("abs").and_then(|v| v.as_str()).unwrap_or("");
    if !absolute.is_empty() && Path::new(absolute).exists() {
        return absolute.to_string();
    }
    if let Some(rel) = entry.get("rel").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        let cand = resolve(&base.join(rel));
        if cand.exists() {
            return cand.to_string_lossy().to_string();
        }
    }
    absolute.to_string()
}

/// Fecha y hora local "AAAA-MM-DD HH:MM:SS" (`time.strftime` de Python).
pub fn local_timestamp() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            dow: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            ms: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(st: *mut SystemTime);
        }
        let mut st = SystemTime::default();
        // SAFETY: GetLocalTime solo escribe en la estructura que le pasamos
        unsafe { GetLocalTime(&mut st) };
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", st.year, st.month, st.day, st.hour, st.minute, st.second)
    }
    #[cfg(not(windows))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
        let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
        // civil_from_days (UTC)
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem / 60 % 60, rem % 60)
    }
}

/// Guarda un proyecto de forma atómica (temporal + reemplazo): un fallo a mitad nunca lo deja roto.
///
/// `state` es el diccionario de la ventana (`sources`, `results_dir`, criterio, pesos, filtros, tipos y
/// colores por clip, modo de exportación, dónde estaba el editor…). Se añaden `format`, `version`,
/// `app` y `saved`, y las rutas se guardan absolutas y relativas al proyecto.
pub fn save(path: &Path, state: &Map<String, Value>) -> Result<()> {
    let base = path.parent().unwrap_or(Path::new("."));
    let mut data = state.clone();
    data.insert("format".into(), FORMAT.into());
    data.insert("version".into(), VERSION.into());
    data.insert("app".into(), APP_NAME.into());
    data.insert("saved".into(), local_timestamp().into());
    let sources: Vec<Value> = state.get("sources").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter()
        .map(|s| {
            let s = s.as_str().unwrap_or("");
            serde_json::json!({"abs": s, "rel": relpath(s, base)})
        })
        .collect();
    data.insert("sources".into(), Value::Array(sources));
    let out = state.get("results_dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rel = if out.is_empty() { None } else { relpath(&out, base) };
    data.insert("results_dir".into(), serde_json::json!({"abs": out, "rel": rel}));
    write_atomic(path, pyfmt::dumps_with(&Value::Object(data), Some(1), false)?)
}

/// Abre un proyecto: las rutas vuelven como texto (la absoluta si existe, si no la relativa).
pub fn load(path: &Path) -> Result<Map<String, Value>> {
    let bytes = std::fs::read(path)?;
    let mut data: Map<String, Value> = serde_json::from_slice::<Value>(&bytes)?.as_object().cloned().unwrap_or_default();
    if data.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
        bail!("{} no es un proyecto de CAMCUTS", path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    }
    let base = path.parent().unwrap_or(Path::new("."));
    let sources: Vec<Value> = data.get("sources").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter()
        .map(|s| if s.is_object() { Value::String(resolve_entry(s, base)) } else { s.clone() })
        .collect();
    data.insert("sources".into(), Value::Array(sources));
    let rd = data.get("results_dir").cloned().unwrap_or(Value::Null);
    let rd = if rd.is_object() { Value::String(resolve_entry(&rd, base)) } else if truthy(&rd) { rd } else { Value::String(String::new()) };
    data.insert("results_dir".into(), rd);
    Ok(data)
}

/// Dónde va el proyecto de un trabajo nuevo: en su carpeta de resultados, con el nombre de la carpeta del material.
pub fn default_path(results_dir: &str, sources: &[String]) -> Option<PathBuf> {
    if results_dir.is_empty() {
        return None;
    }
    let name = sources.first().and_then(|s| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string())).filter(|n| !n.is_empty()).unwrap_or_else(|| "proyecto".into());
    Some(Path::new(results_dir).join(format!("{name}{EXT}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colores_iguales() {
        assert!(same_color(None, Some(&serde_json::json!({"id": "auto"}))));
        assert!(same_color(Some(&serde_json::json!({"id": "cube", "cube": ""})), Some(&serde_json::json!({"id": "cube"}))));
        assert!(!same_color(Some(&serde_json::json!({"id": "hlg"})), None));
    }

    #[test]
    fn rutas_relativas() {
        assert_eq!(relpath(r"C:\a\b\c.mp4", Path::new(r"C:\a\x")).as_deref(), Some(r"..\b\c.mp4"));
        assert_eq!(relpath(r"D:\a\b.mp4", Path::new(r"C:\a")), None);
    }

    #[test]
    fn nombre_de_cache_como_python() {
        // Python: cache_file(Path("x"), Path(r"D:\CONTENIDO NAPO\2026\Octubre\27\videos\C9182.MP4")) → C9182_f6531214.json
        let p = cache_file(Path::new("no_existe"), Path::new(r"D:\CONTENIDO NAPO\2026\Octubre\27\videos\C9182.MP4"));
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "C9182_f6531214.json");
    }

    /// Carpeta temporal de una prueba (dentro de `target/`).
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::current_exe().unwrap().parent().unwrap().join("camcuts_core_tests").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn cache_antigua_sin_distinguir_mayusculas() {
        // Python compara con `WindowsPath ==`: mayúsculas y separadores no importan
        let d = tmp("legacy");
        std::fs::write(d.join("C0001.json"), r#"{"path": "D:\\Contenido Napo\\C0001.MP4"}"#).unwrap();
        for p in [r"D:\CONTENIDO NAPO\C0001.MP4", r"d:\contenido napo\C0001.mp4", "D:/Contenido Napo/C0001.MP4"] {
            assert_eq!(cache_file(&d, Path::new(p)), d.join("C0001.json"), "{p}");
        }
        // otro clip con el mismo nombre en otra carpeta: caché propia con hash
        assert_ne!(cache_file(&d, Path::new(r"D:\Otra\C0001.MP4")), d.join("C0001.json"));
    }

    #[test]
    fn escritura_atomica_y_caches_cortadas() {
        let d = tmp("atomica");
        let f = d.join("x.json");
        write_atomic(&f, b"{\"a\": 1}").unwrap();
        write_atomic(&f, b"{\"a\": 2}").unwrap(); // reemplaza el que ya existe
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{\"a\": 2}");
        assert!(!d.join("x.json.tmp").exists(), "no deja el temporal");
        // una caché cortada (análisis matado a mitad con la escritura antigua) se salta, no tumba el lote
        std::fs::write(d.join("cortada.json"), b"{\"path\": \"D:\\\\a.mp4\", \"samp").unwrap();
        assert!(read_entry(&d.join("cortada.json")).is_err());
        assert!(read_entries_lenient(&d).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn destino_bloqueado_no_pierde_el_resultado() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tmp("bloqueado");
        let f = d.join("c.json");
        std::fs::write(&f, "viejo").unwrap();
        let tmpf = d.join("c.json.tmp");
        // otro programa lo tiene abierto sin compartir (como un antivirus): el rename falla siempre
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&f).unwrap();
        std::fs::write(&tmpf, "nuevo").unwrap();
        let e = rename_retrying(&tmpf, &f, 3, std::time::Duration::from_millis(10)).unwrap_err();
        assert!(format!("{e:#}").contains("c.json.tmp"), "el error dice dónde quedó: {e:#}");
        assert_eq!(std::fs::read_to_string(&tmpf).unwrap(), "nuevo", "el análisis nuevo se conserva");
        // lo suelta enseguida: el reintento lo consigue
        let suelta = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(lock);
        });
        rename_retrying(&tmpf, &f, 20, std::time::Duration::from_millis(50)).unwrap();
        suelta.join().unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "nuevo");
        assert!(!tmpf.exists());
    }
}
