//! Compara camcuts-infer con las salidas del pipeline de PYTHON guardadas en `fixtures/python_ref.json`
//! (generadas por `fixtures/make_fixtures.py`). Tolerancias en `fixtures/README.md`:
//!
//! - detecciones: toda caja de Python con conf ≥ 0.10 debe tener pareja en Rust (misma etiqueta, IoU ≥ 0.5)
//!   y las parejas deben tener IoU ≥ 0.9;
//! - siluetas de MobileSAM: IoU ≥ 0.9 con la máscara de Python;
//! - pose: caja con IoU ≥ 0.9 y puntos visibles (conf > 0.5) a ≤ 4 px;
//! - CLIP: |p_rust − p_python| ≤ 0.05;
//! - Silero VAD: |p_rust − p_python| ≤ 0.05 por ventana.
//!
//! Usan la GPU (DirectML): correr con `cargo test -p camcuts-infer -- --test-threads=1 --nocapture`.
//! Cada prueba imprime las cifras para FP16 y FP32 y comprueba la configuración por defecto.

use std::path::{Path, PathBuf};

use camcuts_core::{Detector, Frame};
use camcuts_infer::{InferConfig, OnnxDetector, WorldSize};
use serde_json::Value;

/// Carpeta `fixtures/` del workspace.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("fixtures")
}

fn reference() -> Value {
    let raw = std::fs::read_to_string(fixtures().join("python_ref.json")).expect("falta fixtures/python_ref.json");
    serde_json::from_str(&raw).unwrap()
}

/// Lee un PNG RGB y lo devuelve como `Frame` BGR (lo que da cv2.imread en Python).
fn load_frame(name: &str) -> Frame {
    let img = image::open(fixtures().join(name)).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for p in data.chunks_mut(3) {
        p.swap(0, 2);
    }
    Frame { width: w, height: h, data }
}

/// Lee un PNG de máscara (0/255) en escala de grises.
fn load_mask(name: &str) -> Vec<u8> {
    image::open(fixtures().join(name)).unwrap().to_luma8().into_raw()
}

fn bx(v: &Value) -> [f32; 4] {
    let a = v.as_array().unwrap();
    [0, 1, 2, 3].map(|i| a[i].as_f64().unwrap() as f32)
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    camcuts_infer::yolo::iou(a, b)
}

fn mask_iou(a: &[u8], b: &[u8]) -> f32 {
    let (mut i, mut u) = (0u64, 0u64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x > 127, *y > 127);
        i += (x && y) as u64;
        u += (x || y) as u64;
    }
    if u == 0 { 1.0 } else { i as f32 / u as f32 }
}

/// Configuración de las pruebas de paridad. El detector va fijado a YOLO-World: las referencias de Python
/// son de YOLO-World y el predeterminado es ahora el propio (`DetectorKind::default`), con el que
/// `detecciones_yolo_world` mediría el YOLO11 sin decirlo.
fn config(fp16: bool) -> InferConfig {
    InferConfig { fp16_world: fp16, fp16_sam: fp16, fp16_pose: fp16, fp16_clip: fp16, detector: camcuts_infer::DetectorKind::World, ..InferConfig::default() }
}

/// Cifras de la comparación de detecciones de un modelo.
#[derive(Debug)]
struct DetStats {
    /// IoU mínima de las parejas cuya caja de Python tiene conf ≥ 0.05 (la que se exige ≥ 0.9)
    min_iou_05: f32,
    /// IoU mínima de todas las parejas (incluidas las de 0.02–0.05, ruidosas)
    min_iou_all: f32,
    /// parte de las cajas de Python con conf ≥ 0.10 que tienen pareja
    recall_10: f32,
    /// parte de todas las cajas de Python (≥ 0.02) que tienen pareja
    recall_all: f32,
    /// diferencia máxima de confianza en las parejas
    max_dconf: f32,
}

/// Empareja uno a uno (misma etiqueta, IoU ≥ 0.5, de mayor a menor IoU) las detecciones de Python y Rust.
fn compare_detections(det: &mut OnnxDetector, key: &str) -> DetStats {
    let r = reference();
    let mut s = DetStats { min_iou_05: 1.0, min_iou_all: 1.0, recall_10: 0.0, recall_all: 0.0, max_dconf: 0.0 };
    let (mut n10, mut f10, mut nall, mut fall) = (0, 0, 0, 0);
    for (name, f) in r["frames"].as_object().unwrap() {
        let frame = load_frame(f["png"].as_str().unwrap());
        let ours = det.detect(&frame).unwrap();
        let theirs = f[key].as_array().unwrap();
        // todas las parejas posibles, de mayor a menor IoU
        let mut pairs = vec![];
        for (i, d) in theirs.iter().enumerate() {
            let pb = bx(&d["box"]);
            for (j, o) in ours.iter().enumerate() {
                if o.label == d["label"].as_str().unwrap() {
                    let v = iou(&pb, &[o.x1, o.y1, o.x2, o.y2]);
                    if v >= 0.5 {
                        pairs.push((v, i, j));
                    }
                }
            }
        }
        pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let (mut used_p, mut used_r) = (vec![None; theirs.len()], vec![false; ours.len()]);
        for (v, i, j) in pairs {
            if used_p[i].is_none() && !used_r[j] {
                used_p[i] = Some((v, j));
                used_r[j] = true;
            }
        }
        for (i, d) in theirs.iter().enumerate() {
            let pc = d["conf"].as_f64().unwrap() as f32;
            let label = d["label"].as_str().unwrap();
            nall += 1;
            n10 += (pc >= 0.10) as usize;
            match used_p[i] {
                Some((v, j)) => {
                    fall += 1;
                    f10 += (pc >= 0.10) as usize;
                    s.min_iou_all = s.min_iou_all.min(v);
                    if pc >= 0.05 {
                        s.min_iou_05 = s.min_iou_05.min(v);
                    }
                    s.max_dconf = s.max_dconf.max((ours[j].conf - pc).abs());
                    if v < 0.9 {
                        println!("  {name}: pareja floja {label} python {pc:.3} rust {:.3} IoU {v:.3}", ours[j].conf);
                    }
                }
                None => println!("  {name}: sin pareja {label} python {pc:.3}"),
            }
        }
    }
    s.recall_10 = f10 as f32 / n10.max(1) as f32;
    s.recall_all = fall as f32 / nall.max(1) as f32;
    s
}

#[test]
fn detecciones_yolo_world() {
    let mut fails = vec![];
    for (world, key) in [(WorldSize::X, "detections_x"), (WorldSize::M, "detections_m")] {
        for fp16 in [true, false] {
            let mut det = OnnxDetector::new(InferConfig { world, ..config(fp16) }).unwrap();
            let s = compare_detections(&mut det, key);
            println!("YOLO-World {world:?} fp16={fp16}: {s:?}");
            if fp16 == InferConfig::default().fp16_world && (s.min_iou_05 < 0.9 || s.recall_10 < 1.0) {
                fails.push(format!("{world:?}: {s:?}"));
            }
        }
    }
    assert!(fails.is_empty(), "{fails:?}");
}

#[test]
fn siluetas_mobile_sam() {
    let r = reference();
    for fp16 in [true, false] {
        let mut det = OnnxDetector::new(config(fp16)).unwrap();
        let (mut min_iou, mut n, mut sum) = (1.0f32, 0, 0.0f32);
        for (name, f) in r["frames"].as_object().unwrap() {
            let frame = load_frame(f["png"].as_str().unwrap());
            for s in f["silhouettes"].as_array().unwrap() {
                let Some(png) = s["png"].as_str() else { continue };
                let theirs = load_mask(png);
                let ours = det.silhouette(&frame, bx(&s["box"])).unwrap();
                let v = mask_iou(&ours, &theirs);
                println!("  {name} {png}: IoU {v:.4}");
                min_iou = min_iou.min(v);
                sum += v;
                n += 1;
            }
        }
        println!("MobileSAM fp16={fp16}: {n} siluetas, IoU mínima {min_iou:.4}, media {:.4}", sum / n as f32);
        if fp16 == InferConfig::default().fp16_sam {
            assert!(min_iou >= 0.9, "IoU {min_iou}");
        }
    }
}

#[test]
fn pose_yolo11n() {
    let r = reference();
    for fp16 in [true, false] {
        let mut det = OnnxDetector::new(config(fp16)).unwrap();
        let (mut min_iou, mut max_px, mut max_dc, mut n) = (1.0f32, 0.0f32, 0.0f32, 0);
        for (name, f) in r["frames"].as_object().unwrap() {
            let frame = load_frame(f["png"].as_str().unwrap());
            let ours = det.pose(&frame).unwrap();
            let theirs = f["pose"].as_array().unwrap();
            println!("  {name}: python {} / rust {} personas", theirs.len(), ours.len());
            for p in theirs {
                let pb = bx(&p["bbox"]);
                let Some(o) = ours.iter().max_by(|a, b| iou(&pb, &a.bbox).partial_cmp(&iou(&pb, &b.bbox)).unwrap()) else {
                    min_iou = 0.0;
                    continue;
                };
                min_iou = min_iou.min(iou(&pb, &o.bbox));
                for (k, kp) in p["keypoints"].as_array().unwrap().iter().enumerate() {
                    let kp: Vec<f32> = kp.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
                    // en estos cuadros solo se ven manos/cuerpo parcial: todos los puntos cuentan
                    let d = ((kp[0] - o.keypoints[k][0]).powi(2) + (kp[1] - o.keypoints[k][1]).powi(2)).sqrt();
                    max_px = max_px.max(d);
                    max_dc = max_dc.max((kp[2] - o.keypoints[k][2]).abs());
                }
                n += 1;
            }
        }
        println!("Pose fp16={fp16}: {n} personas, IoU mínima {min_iou:.3}, error máx de punto {max_px:.2} px, Δconf punto máx {max_dc:.3}");
        if fp16 == InferConfig::default().fp16_pose {
            assert!(min_iou >= 0.9 && max_px <= 4.0 && max_dc <= 0.05);
        }
    }
}

#[test]
fn verificador_clip() {
    let r = reference();
    for fp16 in [true, false] {
        let mut det = OnnxDetector::new(config(fp16)).unwrap();
        let (mut max_d, mut n, mut flips) = (0.0f32, 0, 0);
        for (name, f) in r["frames"].as_object().unwrap() {
            let frame = load_frame(f["png"].as_str().unwrap());
            let items = f["clip"].as_array().unwrap();
            // mismo recorte que Python (la caja agrandada un 15 %), en un lote como verify_weak
            let rects: Vec<[usize; 4]> = items.iter().map(|c| bx(&c["crop"]).map(|v| v as usize)).collect();
            let ours = det.verify_rects(&frame, &rects).unwrap();
            for (c, p) in items.iter().zip(&ours) {
                let pp = c["p"].as_f64().unwrap() as f32;
                println!("  {name} {}: python {pp:.4} rust {p:.4}", c["kind"].as_str().unwrap());
                max_d = max_d.max((p - pp).abs());
                flips += ((p >= &0.6) != (pp >= 0.6)) as usize;
                n += 1;
            }
            // la API del trait (caja de la detección, relleno interno) debe dar lo mismo
            if let Some(c) = items.first() {
                let p = det.verify(&frame, bx(&c["box"])).unwrap();
                let (h, w) = (frame.height as usize, frame.width as usize);
                let rect = camcuts_infer::clip::verify_crop_rect(bx(&c["box"]), w, h);
                assert_eq!(rect, rects[0], "el recorte de verify() no es el de Python");
                // misma entrada; solo cambia el tamaño del lote (en FP16 puede mover la 3.ª–4.ª cifra)
                assert!((p - ours[0]).abs() < 2e-3, "verify() {p} y verify_rects() {} difieren", ours[0]);
            }
        }
        println!("CLIP fp16={fp16}: {n} recortes, |Δp| máx {max_d:.4}, cambios de decisión (p≥0.6) {flips}");
        if fp16 == InferConfig::default().fp16_clip {
            assert!(max_d <= 0.05, "Δp {max_d}");
        }
    }
}

#[test]
fn voz_silero() {
    let r = reference();
    let probs: Vec<f32> = r["speech"]["probs"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    // WAV PCM 16 bits mono de 16 kHz: cabecera de 44 bytes
    let raw = std::fs::read(fixtures().join(r["speech"]["wav"].as_str().unwrap())).unwrap();
    let pos = raw.windows(4).position(|w| w == b"data").unwrap() + 8;
    let audio: Vec<f32> = raw[pos..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    let mut det = OnnxDetector::new(config(false)).unwrap();
    let ours = det.speech(&audio).unwrap();
    assert_eq!(ours.len(), probs.len());
    let max_d = ours.iter().zip(&probs).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    let flips = ours.iter().zip(&probs).filter(|(a, b)| (**a >= 0.5) != (**b >= 0.5)).count();
    println!("Silero VAD: {} ventanas, |Δp| máx {max_d:.5}, cambios de decisión (p≥0.5) {flips}", ours.len());
    assert!(max_d <= 0.05);
}
