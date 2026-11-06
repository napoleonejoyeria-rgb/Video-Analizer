//! La línea de comandos solo reutiliza las cachés hechas con el detector elegido (`--detector`).
//!
//! Igual que con el color o la edición: una caché del otro detector cuenta como vieja y el clip se
//! vuelve a analizar. Las cachés sin la clave `detector` (Python, anteriores a la opción) son de
//! YOLO-World. Aquí no hay modelos (`factory = None`), así que "volver a analizar" se ve como el error de
//! "clip(s) sin analizar". Solo aplica a la edición de joyas (Bodas usa siempre su detector).

use std::path::{Path, PathBuf};

use camcuts_core::cli::{self, Args};
use clap::Parser;

/// Carpeta de resultados temporal y vacía.
fn carpeta(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("camcuts_prueba_detector_{nombre}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("cache")).unwrap();
    d
}

/// Caché mínima de `clip` con (o sin) la clave `detector`.
fn escribir_cache(out: &Path, clip: &Path, detector: Option<&str>) {
    let mut e = serde_json::json!({"path": clip.to_string_lossy(), "fps": [30, 1], "width": 1920, "height": 1080,
                                   "duration": 2.0, "per_second": 4.0, "samples": [], "start_tc": "00:00:00:00"});
    if let Some(d) = detector {
        e["detector"] = serde_json::json!(d);
    }
    let stem = clip.file_stem().unwrap().to_string_lossy().to_string();
    std::fs::write(out.join("cache").join(format!("{stem}.json")), e.to_string()).unwrap();
}

/// Corre la CLI sobre un clip con argumentos extra (sin modelos).
fn correr(out: &Path, clip: &Path, extra: &[&str]) -> anyhow::Result<()> {
    let mut argv = vec!["camcuts".to_string(), clip.to_string_lossy().to_string(), "--out".into(), out.to_string_lossy().to_string()];
    argv.extend(extra.iter().map(|s| s.to_string()));
    cli::run(Args::parse_from(argv), None)
}

/// (marca de la caché, argumentos, ¿se reutiliza?)
const CASOS: [(Option<&str>, &[&str], bool); 6] = [
    (None, &["--detector", "world"], true),
    (Some("world"), &["--detector", "world"], true),
    (Some("propio"), &["--detector", "propio"], true),
    (None, &["--detector", "propio"], false),
    (Some("world"), &["--detector", "propio"], false),
    (Some("propio"), &["--detector", "world"], false),
];

#[test]
fn cache_de_otro_detector_se_vuelve_a_analizar() {
    if camcuts_core::edicion::es_bodas() {
        return; // la elección de detector es solo de la edición de joyas
    }
    for (i, (marca, extra, reutiliza)) in CASOS.iter().enumerate() {
        let out = carpeta(&format!("caso{i}"));
        let clip = out.join("material").join("C0001.MP4");
        escribir_cache(&out, &clip, *marca);
        let r = correr(&out, &clip, extra);
        if *reutiliza {
            r.unwrap_or_else(|e| panic!("caché {marca:?} con {extra:?} debe reutilizarse: {e:#}"));
            assert!(out.join("selects_joyas.otio").exists());
        } else {
            let e = r.expect_err(&format!("caché {marca:?} con {extra:?} no debe reutilizarse"));
            assert!(format!("{e:#}").contains("sin analizar"), "{e:#}");
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}

/// La opción se entiende en la CLI (también la de `camcuts.exe --analizar`, que usa este parser) y un
/// valor desconocido es un error de la línea de comandos.
#[test]
fn opcion_detector_en_la_cli() {
    use camcuts_core::detector::DetectorKind;
    let a = Args::try_parse_from(["camcuts", "x.mp4", "--out", "o", "--detector", "propio"]).unwrap();
    let b = Args::try_parse_from(["camcuts", "x.mp4", "--out", "o", "--detector=world"]).unwrap();
    assert_eq!(a.detector.as_deref(), Some("propio"));
    assert_eq!(b.detector.as_deref(), Some("world"));
    if !camcuts_core::edicion::es_bodas() {
        assert_eq!(a.detector_kind(), DetectorKind::Propio);
        assert_eq!(b.detector_kind(), DetectorKind::World);
    }
    assert!(Args::try_parse_from(["camcuts", "x.mp4", "--out", "o", "--detector", "otro"]).is_err());
}

/// Sin `--detector` (ni `CAMCUTS_DETECTOR`) manda el propio (predeterminado desde el 2026-11-05): una
/// caché del propio vale y una antigua sin clave (YOLO-World) se vuelve a analizar.
#[test]
fn sin_opcion_manda_el_propio() {
    if camcuts_core::edicion::es_bodas() || std::env::var_os("CAMCUTS_DETECTOR").is_some() {
        return;
    }
    let out = carpeta("defecto");
    let clip = out.join("material").join("C0001.MP4");
    escribir_cache(&out, &clip, Some("propio"));
    correr(&out, &clip, &[]).expect("una caché del propio se reutiliza por defecto");
    escribir_cache(&out, &clip, None);
    let e = correr(&out, &clip, &[]).expect_err("una caché antigua (YOLO-World) se vuelve a analizar");
    assert!(format!("{e:#}").contains("sin analizar"), "{e:#}");
    let _ = std::fs::remove_dir_all(&out);
}

/// `--rescore` no vuelve a analizar: usa las cachés de cualquier detector.
#[test]
fn rescore_usa_todas_las_caches() {
    if camcuts_core::edicion::es_bodas() {
        return;
    }
    let out = carpeta("rescore");
    let clip = out.join("material").join("C0001.MP4");
    escribir_cache(&out, &clip, Some("propio"));
    let args = Args::parse_from(["camcuts", "--out", &out.to_string_lossy(), "--rescore", "--detector", "world"]);
    cli::run(args, None).expect("--rescore con una caché del detector propio");
    let csv = std::fs::read_to_string(out.join("selects_joyas.csv")).unwrap();
    assert!(!csv.is_empty());
    let _ = std::fs::remove_dir_all(&out);
}
