//! `jewel::measure` y la voz contra `fixtures/` (salidas de Python escritas por el agente de modelos).
//!
//! No hace falta ningún modelo: un detector de reproducción devuelve las siluetas de MobileSAM que
//! guardó Python, y la medida se hace con las detecciones de Python ya verificadas por CLIP.
//! Tolerancias (fixtures/README.md): `lap_p99` ±1 nivel; el resto de medidas casi exactas.

use std::path::{Path, PathBuf};

use camcuts_core::audio;
use camcuts_core::jewel::{self, Det, MeasureOpts, MeasureState};
use camcuts_core::{Detection, Detector, Frame, Pose};
use serde_json::Value;

fn root() -> PathBuf {
    // fixtures/ está en la raíz del workspace
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("fixtures");
    if d.is_dir() { d } else { PathBuf::from(r"C:\Users\Usuario\Tools\camcuts_rs\fixtures") }
}

fn reference() -> Option<Value> {
    let p = root().join("python_ref.json");
    Some(serde_json::from_slice(&std::fs::read(p).ok()?).unwrap())
}

/// Cuadro BGR desde un PNG RGB (como `cv2.imread`).
fn load_frame(p: &Path) -> Frame {
    let img = image::open(p).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_exact_mut(3) {
        px.swap(0, 2);
    }
    Frame { width: w, height: h, data }
}

/// Detector que solo reproduce las siluetas guardadas por Python (por caja).
struct Replay {
    sils: Vec<([i64; 4], Option<Vec<u8>>)>,
}

impl Detector for Replay {
    fn detect(&mut self, _: &Frame) -> anyhow::Result<Vec<Detection>> {
        anyhow::bail!("no usado")
    }
    fn silhouette(&mut self, _: &Frame, b: [f32; 4]) -> anyhow::Result<Vec<u8>> {
        let key = b.map(|v| v as i64);
        match self.sils.iter().find(|(k, _)| *k == key) {
            Some((_, Some(m))) => Ok(m.clone()),
            _ => anyhow::bail!("sin silueta para {key:?}"),
        }
    }
    fn pose(&mut self, _: &Frame) -> anyhow::Result<Vec<Pose>> {
        Ok(vec![])
    }
    fn verify(&mut self, _: &Frame, _: [f32; 4]) -> anyhow::Result<f32> {
        anyhow::bail!("no usado")
    }
    fn speech(&mut self, _: &[f32]) -> anyhow::Result<Vec<f32>> {
        anyhow::bail!("no usado")
    }
}

#[test]
fn measure_como_python() {
    let Some(r) = reference() else {
        eprintln!("sin fixtures/: prueba saltada");
        return;
    };
    let min_conf = r["min_conf"].as_f64().unwrap();
    for (name, fr) in r["frames"].as_object().unwrap() {
        let frame = load_frame(&root().join(fr["png"].as_str().unwrap()));
        let dets: Vec<Det> = fr["detections_x_verified"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| Det {
                bbox: [0, 1, 2, 3].map(|i| d["box"][i].as_i64().unwrap()),
                conf: d["conf"].as_f64().unwrap(),
                label: d["label"].as_str().unwrap().to_string(),
                zoom: false,
                verified: d.get("verified").and_then(|v| v.as_bool()).unwrap_or(false),
            })
            .collect();
        let sils = fr["silhouettes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let b = [0, 1, 2, 3].map(|i| s["box"][i].as_i64().unwrap());
                let m = s["png"].as_str().map(|p| image::open(root().join(p)).unwrap().to_luma8().into_raw());
                (b, m)
            })
            .collect();
        let mut det = Replay { sils };
        // pose de Python: cara vista si nariz y un ojo superan 0.5
        let poses: Vec<Pose> = fr["pose"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Pose {
                keypoints: std::array::from_fn(|i| std::array::from_fn(|j| p["keypoints"][i][j].as_f64().unwrap() as f32)),
                bbox: std::array::from_fn(|i| p["bbox"][i].as_f64().unwrap() as f32),
                conf: p["conf"].as_f64().unwrap() as f32,
            })
            .collect();
        let mut state = MeasureState::default();
        let opts = MeasureOpts { dt: 0.25, min_conf, lap_scale: 1.0, use_sam: true };
        let s = jewel::measure(0.0, &frame, &dets, &mut state, &opts, Some(&mut det), jewel::has_face(&poses));
        let m = &fr["measure"];
        assert_eq!(s.jewel, m["jewel"].as_bool().unwrap(), "{name}: jewel");
        assert_eq!(s.label, m["label"].as_str().unwrap(), "{name}: label");
        assert_eq!(s.n_jewels, m["n_jewels"].as_i64().unwrap(), "{name}: n_jewels");
        assert_eq!(s.n_sharp, m["n_sharp"].as_i64().unwrap(), "{name}: n_sharp");
        assert_eq!(s.person, m["person"].as_bool().unwrap(), "{name}: person");
        assert_eq!(s.face, m["face"].as_bool().unwrap(), "{name}: face");
        let pj = m["jewels"].as_array().unwrap();
        assert_eq!(s.jewels.len(), pj.len(), "{name}: número de joyas");
        let mut worst = 0f64;
        for (j, p) in s.jewels.iter().zip(pj) {
            assert_eq!(j.label, p["label"].as_str().unwrap(), "{name}: tipo");
            assert_eq!(j.bbox.to_vec(), p["box"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect::<Vec<_>>(), "{name}: caja");
            assert_eq!(j.masked.unwrap(), p["masked"].as_bool().unwrap(), "{name}: masked");
            let lp = p["lap_p99"].as_f64().unwrap();
            let d = (j.lap_p99.unwrap() - lp).abs();
            worst = worst.max(d);
            assert!(d <= 1.0, "{name}: lap_p99 rust {} python {lp}", j.lap_p99.unwrap());
            let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-9);
            assert!(rel(j.mask_frac.unwrap(), p["mask_frac"].as_f64().unwrap()) < 1e-9 || p["mask_frac"].as_f64().unwrap() == 0.0, "{name}: mask_frac");
            // cv2.magnitude de OpenCV 5 no redondea la raíz exacta (60.000004 para √3600): en empates exactos
            // lap == 0.2·G cambia 1 píxel de cada ~250 000; de ahí la tolerancia absoluta de 1e-4 en la densidad
            assert!((j.edge.unwrap() - p["edge"].as_f64().unwrap()).abs() < 1e-4 || p["edge"].as_f64().unwrap() == 0.0, "{name}: edge rust {} python {}", j.edge.unwrap(), p["edge"]);
            assert!(rel(j.lap_var.unwrap(), p["lap_var"].as_f64().unwrap()) < 1e-5, "{name}: lap_var {} vs {}", j.lap_var.unwrap(), p["lap_var"]);
            assert!(rel(j.area_frac.unwrap(), p["area_frac"].as_f64().unwrap()) < 1e-12, "{name}: area_frac");
        }
        println!("{name}: {} joyas ({} con silueta), lap_p99 dif máx {worst}", s.jewels.len(), s.jewels.iter().filter(|j| j.masked == Some(true)).count());
    }
}

#[test]
fn voz_como_python() {
    let Some(r) = reference() else { return };
    let sp = &r["speech"];
    let probs: Vec<f32> = sp["probs"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    // largo del audio en muestras: el bloque "data" del WAV PCM16
    let wav = std::fs::read(root().join(sp["wav"].as_str().unwrap())).unwrap();
    let pos = wav.windows(4).position(|w| w == b"data").unwrap();
    let n = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().unwrap()) as usize / 2;
    let p = audio::VadParams { min_speech_duration_ms: 400.0, ..Default::default() };
    let got: Vec<[f64; 2]> = audio::speech_timestamps(&probs, n, &p).into_iter().map(|[a, b]| [camcuts_core::pyfmt::round_ndigits(a, 2), camcuts_core::pyfmt::round_ndigits(b, 2)]).collect();
    let want: Vec<[f64; 2]> = sp["intervals"].as_array().unwrap().iter().map(|v| [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]).collect();
    println!("voz: rust {got:?} python {want:?}");
    assert_eq!(got, want);
}
