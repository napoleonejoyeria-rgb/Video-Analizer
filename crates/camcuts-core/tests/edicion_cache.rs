//! La línea de comandos solo reutiliza las cachés de SU edición.
//!
//! Una caché marcada con la otra edición (`"edition": "bodas"` vista desde joyas, o una sin marca vista
//! desde bodas) cuenta como vieja: el clip se vuelve a analizar. Aquí no hay modelos (`factory = None`),
//! así que "volver a analizar" se ve como el error de "clip(s) sin analizar". Las cachés de joyas sin
//! marca (las de Python y las antiguas) siguen valiendo para joyas.

use std::path::{Path, PathBuf};

use camcuts_core::cli::{self, Args};
use camcuts_core::edicion::{self, Edicion};
use clap::Parser;

/// Carpeta de resultados temporal y vacía.
fn carpeta(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("camcuts_prueba_edicion_{nombre}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("cache")).unwrap();
    d
}

/// Caché mínima de `clip` (nombre antiguo `<clip>.json`, que la CLI encuentra por su ruta).
fn escribir_cache(out: &Path, clip: &Path, marca: Option<&str>) {
    // hecha con el detector predeterminado (el propio): aquí solo se prueba la edición (Bodas lo ignora)
    let mut e = serde_json::json!({"path": clip.to_string_lossy(), "fps": [30, 1], "width": 1920, "height": 1080,
                                   "duration": 2.0, "per_second": 4.0, "samples": [], "start_tc": "00:00:00:00",
                                   "detector": "propio"});
    if let Some(m) = marca {
        e["edition"] = serde_json::json!(m);
    }
    let stem = clip.file_stem().unwrap().to_string_lossy().to_string();
    std::fs::write(out.join("cache").join(format!("{stem}.json")), e.to_string()).unwrap();
}

/// Marca de una caché de esta edición y de la otra.
fn marcas() -> (Option<&'static str>, Option<&'static str>) {
    match edicion::actual() {
        Edicion::Joyas => (None, Some("bodas")),
        Edicion::Bodas => (Some("bodas"), None),
    }
}

fn correr(out: &Path, clip: &Path) -> anyhow::Result<()> {
    let args = Args::parse_from(["camcuts", &clip.to_string_lossy(), "--out", &out.to_string_lossy()]);
    cli::run(args, None)
}

#[test]
fn cache_de_esta_edicion_se_reutiliza() {
    let (propia, _) = marcas();
    let out = carpeta("propia");
    let clip = out.join("material").join("C0001.MP4");
    escribir_cache(&out, &clip, propia);
    correr(&out, &clip).expect("la caché de esta edición se reutiliza (no hace falta analizar)");
    let base = edicion::actual().nombre_selects(if edicion::es_bodas() { "bodas" } else { "joyas" });
    assert!(out.join(format!("{base}.otio")).exists(), "exporta {base}.otio");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn cache_de_otra_edicion_se_vuelve_a_analizar() {
    let (_, ajena) = marcas();
    let out = carpeta("ajena");
    let clip = out.join("material").join("C0001.MP4");
    escribir_cache(&out, &clip, ajena);
    let e = correr(&out, &clip).expect_err("la caché de la otra edición no debe reutilizarse");
    assert!(format!("{e:#}").contains("sin analizar"), "{e:#}");
    let _ = std::fs::remove_dir_all(&out);
}
