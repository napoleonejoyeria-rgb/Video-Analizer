//! Línea de comandos de CAMCUTS con los modelos de `camcuts-infer` y la opción `--detector propio|world`
//! (pruebas de punta a punta del detector propio sin depender de la ventana).
//!
//! ```text
//! camcuts-e2e <clips o carpetas...> --out <dir> --detector propio [resto de opciones de CAMCUTS]
//! camcuts-e2e bench <frames.bin> <frames.json> <models_dir> propio|world [fp16|fp32] [dets.json]
//! ```
//!
//! `bench` cronometra `Detector::detect` cuadro a cuadro (DirectML) sobre cuadros BGR crudos que escribe
//! `training/scripts/dump_frames.py`, y guarda las detecciones para compararlas con las de Python.

use std::time::Instant;

use camcuts_core::{Detector, Frame};
use clap::Parser;

/// Cronometra el detector elegido sobre los cuadros de `bin` (descritos en `json`).
fn bench(args: &[String]) -> anyhow::Result<()> {
    let (bin, meta, models, kind) = (&args[0], &args[1], &args[2], &args[3]);
    let fp16 = args.get(4).map(|s| s == "fp16").unwrap_or(false);
    let out = args.get(5);
    let data = std::fs::read(bin)?;
    let list: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(meta)?)?;
    let kind = camcuts_infer::DetectorKind::parse(kind).ok_or_else(|| anyhow::anyhow!("detector: propio|world"))?;
    // BENCH_CPU=1: en CPU (para comparar con Python sin ocupar la GPU)
    let device = if std::env::var_os("BENCH_CPU").is_some() { camcuts_infer::Device::Cpu } else { camcuts_infer::Device::DirectMl };
    let cfg = camcuts_infer::InferConfig {
        models_dir: models.into(),
        detector: kind,
        fp16_own: fp16,
        fp16_world: fp16,
        device,
        ..Default::default()
    };
    let mut det = camcuts_infer::OnnxDetector::new(cfg)?;
    let frames: Vec<(String, Frame)> = list
        .iter()
        .map(|m| {
            let (w, h, off) = (m["w"].as_u64().unwrap() as u32, m["h"].as_u64().unwrap() as u32, m["offset"].as_u64().unwrap() as usize);
            let n = (w * h * 3) as usize;
            (m["id"].as_str().unwrap().to_string(), Frame { width: w, height: h, data: data[off..off + n].to_vec() })
        })
        .collect();
    // calentamiento: la primera inferencia de cada forma compila los núcleos de DirectML
    for (_, f) in frames.iter().take(4) {
        det.detect(f)?;
    }
    let mut times = Vec::new();
    let mut dump = serde_json::Map::new();
    for (id, f) in &frames {
        let t = Instant::now();
        let d = det.detect(f)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        dump.insert(
            id.clone(),
            serde_json::Value::Array(d.iter().map(|x| serde_json::json!({"box": [x.x1, x.y1, x.x2, x.y2], "conf": x.conf, "label": x.label})).collect()),
        );
    }
    let mut s = times.clone();
    s.sort_by(|a, b| a.total_cmp(b));
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    println!("{kind:?} fp16={fp16}: {} cuadros, media {mean:.1} ms, mediana {:.1} ms, p90 {:.1} ms", times.len(), s[s.len() / 2], s[s.len() * 9 / 10]);
    if let Some(o) = out {
        std::fs::write(o, serde_json::to_vec(&dump)?)?;
    }
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "bench").unwrap_or(false) {
        if let Err(e) = bench(&args[2..]) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // `--detector` no es del parser del núcleo: se quita y queda en CAMCUTS_DETECTOR
    let kind = camcuts_infer::DetectorKind::take_arg(&mut args).unwrap_or_else(camcuts_infer::DetectorKind::from_env);
    eprintln!("detector: {kind:?}");
    let parsed = camcuts_core::cli::Args::parse_from(args);
    let cfg = camcuts_infer::InferConfig { detector: kind, ..Default::default() };
    // cada hilo de análisis pide su detector a esta fábrica
    let factory = move || -> anyhow::Result<Box<dyn camcuts_core::Detector>> { Ok(Box::new(camcuts_infer::OnnxDetector::new(cfg.clone())?)) };
    if let Err(e) = camcuts_core::cli::run(parsed, Some(&factory)) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
