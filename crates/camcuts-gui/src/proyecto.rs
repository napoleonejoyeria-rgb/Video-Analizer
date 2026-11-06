//! Proyectos `.camcuts` y preferencias de la app.
//!
//! El formato es el de `camcuts/project.py` (formato `camcuts-project`, versión 1): un JSON con el
//! estado de la ventana, las rutas del material guardadas absolutas y relativas al proyecto, y la
//! carpeta de resultados igual. Un proyecto guardado por Python se abre aquí y al revés.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::datos::ruta_relativa;

/// Extensión de los proyectos.
pub const EXT: &str = ".camcuts";
/// Marca de formato que Python comprueba al abrir.
pub const FORMATO: &str = "camcuts-project";
/// Versión del formato.
pub const VERSION: i64 = 1;

/// `{"abs": ..., "rel": ...}` de una ruta respecto a la carpeta del proyecto.
fn par(ruta: &str, base: &Path) -> Value {
    json!({"abs": ruta, "rel": ruta_relativa(Path::new(ruta), base)})
}

/// Ruta guardada: la absoluta si existe; si no, la relativa al proyecto; si tampoco, la absoluta.
fn resolver(v: &Value, base: &Path) -> String {
    let abs = v.get("abs").and_then(Value::as_str).unwrap_or("");
    if !abs.is_empty() && Path::new(abs).exists() {
        return abs.to_owned();
    }
    if let Some(rel) = v.get("rel").and_then(Value::as_str) {
        let c = base.join(rel);
        if c.exists() {
            // normaliza "..\" sin depender de canonicalize (que añade el prefijo \\?\)
            return normalizar(&c).to_string_lossy().to_string();
        }
    }
    abs.to_owned()
}

/// Quita los componentes "." y ".." de una ruta.
fn normalizar(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            o => out.push(o.as_os_str()),
        }
    }
    out
}

/// Fecha y hora local "AAAA-MM-DD HH:MM:SS" (para el campo `saved`), sin dependencias extra.
pub fn ahora_texto() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct SystemTime {
            y: u16,
            m: u16,
            _dow: u16,
            d: u16,
            h: u16,
            mi: u16,
            s: u16,
            _ms: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(st: *mut SystemTime);
        }
        let mut st = SystemTime { y: 0, m: 0, _dow: 0, d: 0, h: 0, mi: 0, s: 0, _ms: 0 };
        // SAFETY: GetLocalTime solo escribe en la estructura que le pasamos.
        unsafe { GetLocalTime(&mut st) };
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", st.y, st.m, st.d, st.h, st.mi, st.s)
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

/// Hora local "HH:MM:SS" para el aviso "guardado a las…".
pub fn hora_texto() -> String {
    ahora_texto().split(' ').nth(1).unwrap_or("").to_owned()
}

/// Guarda el estado como proyecto. Escribe a un temporal y lo reemplaza: un corte a mitad de
/// guardado nunca deja el proyecto roto (igual que `project.save`).
pub fn guardar(ruta: &Path, estado: &Map<String, Value>) -> std::io::Result<()> {
    let base = ruta.parent().unwrap_or(Path::new("."));
    let mut d = estado.clone();
    d.insert("format".into(), json!(FORMATO));
    d.insert("version".into(), json!(VERSION));
    d.insert("app".into(), json!(crate::motor::nombre_app()));
    d.insert("saved".into(), json!(ahora_texto()));
    let fuentes: Vec<Value> = estado
        .get("sources")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|s| par(s, base)).collect())
        .unwrap_or_default();
    d.insert("sources".into(), Value::Array(fuentes));
    let out = estado.get("results_dir").and_then(Value::as_str).unwrap_or("");
    d.insert(
        "results_dir".into(),
        json!({"abs": out, "rel": if out.is_empty() { Value::Null } else { json!(ruta_relativa(Path::new(out), base)) }}),
    );
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", ruta.display()));
    // mismo sangrado que Python (indent=1) para que los diffs entre versiones sean limpios
    let texto = a_json_sangrado(&Value::Object(d));
    std::fs::write(&tmp, texto)?;
    std::fs::rename(&tmp, ruta)
}

/// JSON con sangría de un espacio y sin escapar acentos (como `json.dumps(indent=1, ensure_ascii=False)`).
pub fn a_json_sangrado(v: &Value) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(v, &mut ser).expect("serializar JSON");
    String::from_utf8(buf).unwrap_or_default()
}

/// Abre un proyecto: comprueba el formato y resuelve `sources` y `results_dir` a rutas simples.
pub fn abrir(ruta: &Path) -> anyhow::Result<Map<String, Value>> {
    let texto = std::fs::read_to_string(ruta)?;
    let mut d: Map<String, Value> = serde_json::from_str(texto.trim_start_matches('\u{feff}'))?;
    if d.get("format").and_then(Value::as_str) != Some(FORMATO) {
        anyhow::bail!("{} no es un proyecto de CAMCUTS", ruta.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
    }
    let base = ruta.parent().unwrap_or(Path::new("."));
    let fuentes: Vec<Value> = d
        .get("sources")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|s| if s.is_object() { json!(resolver(s, base)) } else { s.clone() }).collect())
        .unwrap_or_default();
    d.insert("sources".into(), Value::Array(fuentes));
    let rd = match d.get("results_dir") {
        Some(v) if v.is_object() => resolver(v, base),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    d.insert("results_dir".into(), json!(rd));
    Ok(d)
}

/// Edición de un proyecto abierto, por su campo `app` (sin campo: joyas, como los de Python).
pub fn edicion_de(d: &Map<String, Value>) -> camcuts_core::edicion::Edicion {
    camcuts_core::edicion::Edicion::de_app(d.get("app").and_then(Value::as_str))
}

/// Si en `ruta` ya hay un proyecto de la OTRA edición, el nombre de su app (p. ej. "CAMCUTS Bodas").
/// El autoguardado no debe pisarlo: una sesión de bodas reemplazaría en silencio un proyecto de joyas.
pub fn app_ajena(ruta: &Path) -> Option<String> {
    let texto = std::fs::read_to_string(ruta).ok()?;
    let d: Map<String, Value> = serde_json::from_str(texto.trim_start_matches('\u{feff}')).ok()?;
    let suya = edicion_de(&d);
    (suya != camcuts_core::edicion::actual()).then(|| suya.nombre_app().to_owned())
}

/// Dónde va el proyecto de un trabajo nuevo: en su carpeta de resultados, con el nombre de la
/// carpeta del material (igual que `project.default_path`).
pub fn ruta_por_defecto(resultados: &str, fuentes: &[String]) -> Option<PathBuf> {
    if resultados.is_empty() {
        return None;
    }
    let nombre = fuentes
        .first()
        .and_then(|s| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "proyecto".to_owned());
    Some(Path::new(resultados).join(format!("{nombre}{EXT}")))
}

/// Archivo de preferencias de la app (último proyecto y estado), en `%APPDATA%\CAMCUTS-rs`.
pub fn ruta_preferencias() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // cada edición recuerda su propio último proyecto
    let carpeta = if camcuts_core::edicion::es_bodas() { "CAMCUTS-Bodas" } else { "CAMCUTS-rs" };
    base.join(carpeta).join("settings.json")
}

/// Lee las preferencias (vacías si no hay).
pub fn leer_preferencias() -> Map<String, Value> {
    std::fs::read_to_string(ruta_preferencias()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Escribe las preferencias.
pub fn guardar_preferencias(d: &Map<String, Value>) {
    let ruta = ruta_preferencias();
    if let Some(dir) = ruta.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(ruta, a_json_sangrado(&Value::Object(d.clone())));
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn ida_y_vuelta() {
        let dir = std::env::temp_dir().join("camcuts_gui_prueba_proyecto");
        let _ = std::fs::create_dir_all(dir.join("material"));
        let mut e = Map::new();
        e.insert("sources".into(), json!([dir.join("material").to_string_lossy()]));
        e.insert("results_dir".into(), json!(dir.join("material").join("CAMCUTS").to_string_lossy()));
        e.insert("criterion".into(), json!("joyas"));
        let p = dir.join("x.camcuts");
        guardar(&p, &e).unwrap();
        let d = abrir(&p).unwrap();
        assert_eq!(d["criterion"], "joyas");
        assert_eq!(d["format"], FORMATO);
        assert_eq!(d["sources"][0].as_str().unwrap(), dir.join("material").to_string_lossy());
        // el proyecto lleva la app de esta edición y no cuenta como ajeno
        assert_eq!(edicion_de(&d), camcuts_core::edicion::actual());
        assert_eq!(app_ajena(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un proyecto de la otra edición se reconoce por su campo `app`.
    #[test]
    fn proyecto_de_otra_edicion() {
        use camcuts_core::edicion::{actual, Edicion};
        let otra = if actual() == Edicion::Bodas { Edicion::Joyas } else { Edicion::Bodas };
        let dir = std::env::temp_dir().join("camcuts_gui_prueba_proyecto_ajeno");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("x.camcuts");
        std::fs::write(&p, format!(r#"{{"format": "camcuts-project", "version": 1, "app": "{}"}}"#, otra.nombre_app())).unwrap();
        assert_eq!(app_ajena(&p).as_deref(), Some(otra.nombre_app()));
        assert_eq!(app_ajena(&dir.join("no existe.camcuts")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
