//! Utilidades de datos de la ventana: buscar clips en carpetas, leer sus metadatos con ffprobe,
//! formatear tiempos y códigos de tiempo. Replica las funciones de ayuda de `gui.py`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Bandera de Windows para que ffmpeg/ffprobe no abran una consola (CREATE_NO_WINDOW).
pub const SIN_VENTANA: u32 = 0x0800_0000;
/// Prioridad "por debajo de lo normal" para trabajos de fondo (BELOW_NORMAL_PRIORITY_CLASS).
pub const PRIORIDAD_BAJA: u32 = 0x0000_4000;

/// Extensiones que ffmpeg decodifica directamente (igual que `sampler.FFMPEG_EXT`).
const EXT_FFMPEG: &[&str] = &[".mp4", ".mov", ".mxf", ".m4v", ".mts", ".m2ts", ".avi", ".mkv", ".webm", ".3gp", ".dng", ".crm"];
/// RAW de cámara que se analiza por su proxy (igual que `sampler.RAW_EXT`).
const EXT_RAW: &[&str] = &[".braw", ".r3d", ".nev", ".ari", ".arx", ".mxr", ".nraw"];
/// Subcarpetas que nunca se recorren: son resultados de CAMCUTS (de cualquier edición), no material.
const CARPETAS_OMITIDAS: &[&str] = &["CAMCUTS", "CAMCUTS Bodas", "cache", "thumbs"];

/// Prepara un `Command` para que no muestre consola en Windows.
pub fn sin_consola(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(SIN_VENTANA);
    }
    cmd
}

/// Igual que `sin_consola`, y además con prioridad baja (proxies, miniaturas).
pub fn sin_consola_baja(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(SIN_VENTANA | PRIORIDAD_BAJA);
    }
    cmd
}

/// ¿La extensión es de video que CAMCUTS sabe analizar?
pub fn es_video(p: &Path) -> bool {
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
    EXT_FFMPEG.contains(&ext.as_str()) || EXT_RAW.contains(&ext.as_str())
}

/// Extensiones para el filtro del diálogo "Archivos" (sin el punto).
pub fn extensiones_video() -> Vec<String> {
    EXT_FFMPEG.iter().chain(EXT_RAW).map(|e| e.trim_start_matches('.').to_owned()).collect()
}

/// Recorre carpeta (con o sin subcarpetas) y añade los videos, saltando las carpetas de resultados.
fn recorrer(raiz: &Path, dir: &Path, recursivo: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if recursivo && !CARPETAS_OMITIDAS.iter().any(|c| p.file_name().map(|n| n == *c).unwrap_or(false)) {
                recorrer(raiz, &p, recursivo, out);
            }
        } else if ft.is_file() && es_video(&p) && !camcuts_core::sampler::es_basura_mac(&p) {
            // ("._C0001.MP4" de macOS: tiene extensión de video pero no lo es)
            out.push(p);
        }
    }
}

/// Lista de clips de las fuentes (archivos sueltos o carpetas), ordenada y sin duplicados
/// (comparando en minúsculas, como hace Windows). Igual que `collect_clips` de Python.
pub fn reunir_clips(fuentes: &[String], recursivo: bool) -> Vec<PathBuf> {
    let mut clips = Vec::new();
    for s in fuentes {
        let p = PathBuf::from(s);
        if p.is_file() && es_video(&p) && !camcuts_core::sampler::es_basura_mac(&p) {
            clips.push(p);
        } else if p.is_dir() {
            recorrer(&p, &p, recursivo, &mut clips);
        }
    }
    clips.sort();
    let mut vistos = HashSet::new();
    clips.into_iter().filter(|c| vistos.insert(clave(c))).collect()
}

/// Clave de un clip en los JSON de Python: la ruta en minúsculas.
pub fn clave(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Carpeta del material de una fuente: la propia carpeta, o la carpeta del archivo. Una fuente sin
/// conexión (unidad desconectada) no se puede mirar en el disco: con extensión de video es un clip y
/// si no, una carpeta (`D:\Boda` → `D:\Boda`, `D:\Boda\C0001.MP4` → `D:\Boda`).
pub fn carpeta_de_fuente(p: &Path) -> PathBuf {
    let es_archivo = if p.exists() { !p.is_dir() } else { es_video(p) };
    if es_archivo {
        p.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        p.to_path_buf()
    }
}

/// Componentes de una ruta en minúsculas (sin "."), para comparar como Windows.
fn componentes(p: &Path) -> Vec<String> {
    use std::path::Component;
    p.components().filter(|c| !matches!(c, Component::CurDir)).map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()
}

/// ¿Está `ruta` dentro de `base` (o es ella)? Compara componentes enteros y sin mayúsculas: `D:\Boda`
/// contiene `D:\boda\C0001.MP4` pero no `D:\Boda 2\C0001.MP4`. Con `recursivo = false`, solo lo que
/// cuelga directamente de `base`.
pub fn dentro_de(ruta: &Path, base: &Path, recursivo: bool) -> bool {
    let (r, b) = (componentes(ruta), componentes(base));
    if b.is_empty() || !r.starts_with(&b) {
        return false;
    }
    recursivo || r.len() <= b.len() + 1
}

/// Metadatos de un clip para la tabla (formato de `clips_meta.json`, igual que Python).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub res: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

/// Lee códec, contenedor, resolución, fps y duración con ffprobe (como `probe_meta` de Python).
pub fn sondear_meta(ruta: &Path) -> Meta {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"]).arg(ruta);
    sin_consola(&mut cmd);
    let Ok(out) = cmd.output() else { return Meta { error: Some(true), ..Default::default() } };
    // ffprobe sobre un archivo que no existe escribe "{  }" y sale con código 1: el JSON se lee, pero no
    // hay nada. Antes quedaba "? · 0×0 · 0.00" para siempre (con códec puesto no se vuelve a sondear)
    if !out.status.success() {
        return Meta { error: Some(true), ..Default::default() };
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Meta { error: Some(true), ..Default::default() };
    };
    let Some(video) = v["streams"].as_array().and_then(|s| s.iter().find(|s| s["codec_type"] == "video")) else {
        // sin pista de video (solo audio, dañado): no es un clip que se pueda ver
        return Meta { error: Some(true), ..Default::default() };
    };
    let (mut w, mut h) = (video["width"].as_u64().unwrap_or(0), video["height"].as_u64().unwrap_or(0));
    // los iPhone guardan la rotación aparte: en vertical, ancho y alto se intercambian
    let rotado = video["side_data_list"]
        .as_array()
        .map(|l| l.iter().any(|sd| matches!(sd["rotation"].as_i64().map(|r| r.abs()), Some(90) | Some(270))))
        .unwrap_or(false);
    if rotado {
        std::mem::swap(&mut w, &mut h);
    }
    let fps = video["r_frame_rate"].as_str().and_then(fraccion).unwrap_or(0.0);
    let codec = match video["codec_name"].as_str() {
        Some("h264") => "H.264".to_owned(),
        Some("hevc") => "H.265".to_owned(),
        Some("prores") => "ProRes".to_owned(),
        Some(o) => o.to_owned(),
        None => "?".to_owned(),
    };
    let duracion = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    Meta {
        codec: Some(codec),
        container: Some(ruta.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default()),
        res: Some(format!("{w}×{h}")),
        fps: Some(fps),
        duration: Some(duracion),
        error: None,
    }
}

/// "30000/1001" -> 29.97
fn fraccion(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b != 0.0).then_some(a / b)
}

/// Lee `clips_meta.json` de la carpeta de resultados (vacío si no existe).
pub fn leer_metas(out: &Path) -> BTreeMap<String, Meta> {
    std::fs::read_to_string(out.join("clips_meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Escribe `clips_meta.json` (mismo formato que Python: un objeto ruta-en-minúsculas -> meta). Devuelve
/// si se pudo: la ventana avisa (una vez) cuando la carpeta de resultados no deja escribir.
pub fn escribir_metas(out: &Path, metas: &BTreeMap<String, Meta>) -> bool {
    std::fs::create_dir_all(out).is_ok() && serde_json::to_string(metas).map(|s| std::fs::write(out.join("clips_meta.json"), s).is_ok()).unwrap_or(false)
}

/// Tiempo legible: "1:05.3" o "1:02:05.3" (igual que `fmt_time` de Python).
pub fn fmt_tiempo(s: f64) -> String {
    let s = s.max(0.0);
    if s >= 3600.0 {
        format!("{}:{:02}:{:04.1}", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64, s % 60.0)
    } else {
        format!("{}:{:04.1}", (s / 60.0) as u64, s % 60.0)
    }
}

/// Tiempo corto para el transporte: "0:21" o "1:02:05".
pub fn fmt_corto(s: f64) -> String {
    let t = s.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

/// Código de tiempo HH:MM:SS:FF a una cadencia dada (igual que `timecode` de Python).
pub fn codigo_tiempo(segundos: f64, cadencia: f64) -> String {
    let n = (cadencia.round() as i64).max(1);
    let n = if cadencia.round() == 0.0 { 30 } else { n };
    let f = (segundos * cadencia).round() as i64;
    format!("{:02}:{:02}:{:02}:{:02}", f / (n * 3600), f / (n * 60) % 60, f / n % 60, f % n)
}

/// Ruta relativa como `os.path.relpath` (None si están en unidades distintas).
pub fn ruta_relativa(destino: &Path, base: &Path) -> Option<String> {
    use std::path::Component;
    let partes = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect()
    };
    let (d, b) = (partes(destino), partes(base));
    // en Windows el primer componente es la unidad: si difiere no hay ruta relativa
    if d.first().map(|s| s.to_lowercase()) != b.first().map(|s| s.to_lowercase()) {
        return None;
    }
    let comun = d.iter().zip(&b).take_while(|(x, y)| x.to_lowercase() == y.to_lowercase()).count();
    let mut r: Vec<String> = std::iter::repeat_n("..".to_owned(), b.len() - comun).collect();
    r.extend(d[comun..].iter().cloned());
    Some(if r.is_empty() { ".".to_owned() } else { r.join("\\") })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn tiempos_como_python() {
        assert_eq!(fmt_tiempo(65.3), "1:05.3");
        assert_eq!(codigo_tiempo(1.5, 30000.0 / 1001.0), "00:00:01:15");
        assert_eq!(fmt_corto(48.9), "0:48");
    }

    /// Una fuente sin conexión se compara por componentes enteros: `D:\Boda` no recoge los clips de
    /// `D:\Boda 2`, y sin subcarpetas solo los de su nivel.
    #[test]
    fn fuente_por_componentes() {
        let base = Path::new("Z:\\Boda");
        assert!(dentro_de(Path::new("z:\\boda\\C0001.MP4"), base, true));
        assert!(dentro_de(Path::new("Z:\\Boda\\Cam1\\C0002.MP4"), base, true));
        assert!(!dentro_de(Path::new("Z:\\Boda\\Cam1\\C0002.MP4"), base, false));
        assert!(!dentro_de(Path::new("Z:\\Boda 2\\C0003.MP4"), base, true));
        assert!(!dentro_de(Path::new("Z:\\Bodas\\C0003.MP4"), base, true));
        // una fuente que es un clip suelto solo se contiene a sí misma
        assert!(dentro_de(Path::new("Z:\\Boda\\C0001.MP4"), Path::new("Z:\\boda\\c0001.mp4"), true));
    }

    /// La carpeta del material de una fuente sin conexión sale de su extensión, no de `is_dir()`.
    #[test]
    fn carpeta_de_fuente_sin_conexion() {
        assert_eq!(carpeta_de_fuente(Path::new("Z:\\no hay\\Boda")), PathBuf::from("Z:\\no hay\\Boda"));
        assert_eq!(carpeta_de_fuente(Path::new("Z:\\no hay\\Boda 2.0")), PathBuf::from("Z:\\no hay\\Boda 2.0"));
        assert_eq!(carpeta_de_fuente(Path::new("Z:\\no hay\\Boda\\C0001.MP4")), PathBuf::from("Z:\\no hay\\Boda"));
    }

    /// Las copias "._" de macOS no se listan, ni en una carpeta ni sueltas.
    #[test]
    fn sin_basura_de_macos() {
        let dir = std::env::temp_dir().join(format!("camcuts_mac_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bueno = dir.join("C0001.MP4");
        let basura = dir.join("._C0001.MP4");
        std::fs::write(&bueno, b"x").unwrap();
        std::fs::write(&basura, b"\x00\x05\x16\x07").unwrap();
        assert_eq!(reunir_clips(&[dir.to_string_lossy().to_string()], true), vec![bueno.clone()]);
        assert!(reunir_clips(&[basura.to_string_lossy().to_string()], true).is_empty(), "suelta tampoco");
        assert_eq!(camcuts_core::cli::find_clips(&[basura.to_string_lossy().to_string(), bueno.to_string_lossy().to_string()]), vec![bueno.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un clip que no existe sale con error (la lista dice "Sin conexión"), no con "? · 0×0".
    #[test]
    fn sondeo_de_un_clip_que_no_existe() {
        let m = sondear_meta(Path::new("C:\\no_existe_camcuts.mp4"));
        assert_eq!(m.error, Some(true));
        assert_eq!(m.codec, None);
    }

    #[test]
    fn relativa() {
        let r = ruta_relativa(Path::new("C:\\a\\b\\c"), Path::new("C:\\a\\x"));
        assert_eq!(r.as_deref(), Some("..\\b\\c"));
        assert_eq!(ruta_relativa(Path::new("D:\\a"), Path::new("C:\\a")), None);
    }
}
