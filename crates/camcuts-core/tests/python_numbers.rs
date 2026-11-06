//! Métricas y color contra números calculados con Python (venv de CAMCUTS, OpenCV 5, solo CPU).
//!
//! Las referencias las escribe `tests/data/gen_python_numbers.py` en `tests/data/numbers/`.
//! Tolerancias: aritmética de 8 bits idéntica (0 diferencias); varianzas y percentiles con 1e-5
//! relativo (numpy acumula en float32); desplazamiento de cámara a 0.1 px de cuadro: el pico es el mismo,
//! pero el centroide subpíxel de `cv2.phaseCorrelate` de OpenCV 5 no se reprodujo exactamente (~0.06 px).
//! Solo afecta al umbral de 40 px de recorrido de cámara del deduplicado (desactivado en "joyas").

use std::path::{Path, PathBuf};

use camcuts_core::color::{self, Decoding};
use camcuts_core::{image, metrics};
use serde_json::Value;

fn dir() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data");
    let d = if d.is_dir() { d } else { Path::new(file!()).parent().unwrap().join("data") };
    d.join("numbers")
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(format!("{name}.bin"))).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn nums() -> Value {
    serde_json::from_slice(&std::fs::read(dir().join("numbers.json")).unwrap()).unwrap()
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn close(name: &str, a: f64, b: f64, rel: f64) {
    let tol = rel * a.abs().max(b.abs()).max(1e-12);
    assert!((a - b).abs() <= tol, "{name}: rust {a} vs python {b} (dif {:.3e})", (a - b).abs());
    println!("{name}: rust {a} python {b} (dif rel {:.1e})", (a - b).abs() / b.abs().max(1e-300));
}

/// (píxeles distintos, diferencia máxima) entre dos imágenes de 8 bits.
fn diff(a: &[u8], b: &[u8]) -> (usize, u8) {
    assert_eq!(a.len(), b.len());
    let n = a.iter().zip(b).filter(|(x, y)| x != y).count();
    let m = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    (n, m)
}

#[test]
fn metricas_como_python() {
    let n = nums();
    let (w, h) = (n["size"][0].as_u64().unwrap() as usize, n["size"][1].as_u64().unwrap() as usize);
    let a = bin("frame_a");
    let b = bin("frame_b");
    let ga = image::bgr_to_gray(&a, w, h);
    assert_eq!(diff(&ga, &bin("gray_a")), (0, 0), "gris");
    let maps = metrics::edges_and_laplacian(&ga, w, h);
    assert_eq!(diff(&maps.mask, &bin("mask_a")), (0, 0), "máscara de bordes nítidos");
    close("edge_density", metrics::density_full(&maps), f(&n["edge_density"]), 1e-12);
    close("lap_var", metrics::laplacian_variance(&maps.lap, None), f(&n["lap_var"]), 1e-5);
    let r = &n["rect"];
    let rect = [0, 1, 2, 3].map(|i| r[i].as_u64().unwrap() as usize);
    let reg = metrics::Region::Rect(rect);
    close("rect_density", metrics::density(&maps, reg), f(&n["rect_density"]), 1e-12);
    close("rect_lap_var", metrics::laplacian_variance(&maps.lap, Some(reg)), f(&n["rect_lap_var"]), 1e-5);
    close("rect_lap_p99", metrics::laplacian_peak(&maps.lap, reg, 99.0), f(&n["rect_lap_p99"]), 1e-5);
    close("rect_anisotropy", metrics::blur_anisotropy(&ga, w, rect), f(&n["rect_anisotropy"]), 1e-5);
    let ex = metrics::exposure(&ga);
    close("mean_luma", ex.mean_luma, f(&n["exposure"]["mean_luma"]), 1e-12);
    close("clip_black", ex.clip_black, f(&n["exposure"]["clip_black"]), 1e-12);
    close("clip_white", ex.clip_white, f(&n["exposure"]["clip_white"]), 1e-12);

    // movimiento de cámara (correlación de fase a media resolución)
    let gb = image::bgr_to_gray(&b, w, h);
    let mut cs = metrics::CameraShift::new();
    cs.shift(&ga, w, h);
    let (dx, dy, _) = cs.shift(&gb, w, h);
    let py = &n["camera_shift"];
    println!("camera_shift: rust ({dx:.4}, {dy:.4}) python ({:.4}, {:.4})", f(&py[0]), f(&py[1]));
    assert!((dx - f(&py[0])).abs() < 0.1 && (dy - f(&py[1])).abs() < 0.1, "desplazamiento");

    // tamaño real de análisis (960x540): desplazamiento, miniatura y medidas de cuadro
    let (bw, bh) = (960, 540);
    let g1 = image::bgr_to_gray(&bin("big_a"), bw, bh);
    let g2 = image::bgr_to_gray(&bin("big_b"), bw, bh);
    let mut cs = metrics::CameraShift::new();
    cs.shift(&g1, bw, bh);
    let (dx, dy, _) = cs.shift(&g2, bw, bh);
    let py = &n["big_camera_shift"];
    println!("big camera_shift: rust ({dx:.4}, {dy:.4}) python ({:.4}, {:.4})", f(&py[0]), f(&py[1]));
    assert!((dx - f(&py[0])).abs() < 0.1 && (dy - f(&py[1])).abs() < 0.1, "desplazamiento 960");
    let th = metrics::thumb(&g1, bw, bh);
    let (nd, md) = diff(&th, &bin("big_thumb"));
    println!("miniatura 96x96: {nd} píxeles distintos, máx {md}");
    assert!(md <= 1 && nd < 50, "miniatura");
    let m2 = metrics::edges_and_laplacian(&g1, bw, bh);
    close("big_edge_density", metrics::density_full(&m2), f(&n["big_edge_density"]), 1e-12);
    close("big_lap_var", metrics::laplacian_variance(&m2.lap, None), f(&n["big_lap_var"]), 1e-5);
    let (nd, md) = diff(&metrics::thumb(&ga, w, h), &bin("thumb_a"));
    assert!(md <= 1 && nd < 50, "miniatura pequeña: {nd} / {md}");

    // LOG: detección de plano y estirado de contraste
    let flat = bin("flat");
    assert_eq!(metrics::is_flat(&a, w, h, 150.0), n["is_flat_a"].as_bool().unwrap());
    assert_eq!(metrics::is_flat(&flat, w, h, 150.0), n["is_flat_flat"].as_bool().unwrap());
    let nc = metrics::normalize_contrast(&flat, w, h, 0.5, 99.5);
    let (nd, md) = diff(&nc, &bin("normalized_flat"));
    println!("normalize_contrast: {nd} valores distintos, máx {md}");
    assert_eq!(nd, 0, "normalize_contrast");
}

#[test]
fn color_como_python() {
    let n = nums();
    let c = bin("color_in");
    let (w, h) = (160, 90);
    let lut = color::out_lut();
    assert_eq!(diff(lut, &bin("out_lut")), (0, 0), "tabla de salida");
    let ffmpeg = Decoding { decoded_range: "full".into(), decoded_matrix: "bt601".into(), source_matrix: Some("bt709".into()), cube_input: None };
    for tid in ["slog3_sgamut3cine", "slog3_sgamut3", "slog2_sgamut", "hlg", "vlog", "clog3", "applelog"] {
        let g = color::gamut_to_709(match tid {
            "slog3_sgamut3cine" => "sgamut3cine",
            "slog3_sgamut3" | "slog2_sgamut" => "sgamut3",
            "hlg" | "applelog" => "rec2020",
            "vlog" => "vgamut",
            _ => "cinemagamut",
        });
        for i in 0..3 {
            for j in 0..3 {
                close(&format!("gamut {tid} [{i}][{j}]"), g[i][j], f(&n[format!("gamut_{tid}")][i][j]), 1e-12);
            }
        }
        for (dec, name) in [(ffmpeg.clone(), "ffmpeg"), (Decoding::default(), "cv2")] {
            let conv = color::make_converter(tid, None, &dec).unwrap();
            let out = conv.convert(&c, w, h);
            let (nd, md) = diff(&out, &bin(&format!("color_{tid}_{name}")));
            println!("{tid} ({name}): {nd} de {} valores distintos, máx {md}", out.len());
            assert!(md <= 1 && nd * 1000 <= out.len(), "{tid} ({name}): {nd} distintos, máx {md}");
        }
    }
    // LUT .cube 3D con preparación 1D, y 1D pura con salida legal
    let p3 = dir().join("test3d.cube");
    for (dec, name) in [(Decoding::default(), "cube3d_out"), (ffmpeg.clone(), "cube3d_out_ffmpeg")] {
        let conv = color::make_converter("cube", Some(p3.to_str().unwrap()), &dec).unwrap();
        let (nd, md) = diff(&conv.convert(&c, w, h), &bin(name));
        println!("{name}: {nd} valores distintos, máx {md}");
        assert!(md <= 1 && nd * 1000 <= c.len(), "{name}");
    }
    let p1 = dir().join("test1d.cube");
    let conv = color::make_converter("cube", Some(p1.to_str().unwrap()), &Decoding::default()).unwrap();
    let (nd, md) = diff(&conv.convert(&c, w, h), &bin("cube1d_out"));
    println!("cube1d_out: {nd} valores distintos, máx {md}");
    assert_eq!(nd, 0, "cube 1D");
}
