//! Prueba de punta a punta del muestreador y del bucle de análisis con un clip real (solo lectura en D:).
//!
//! Decodifica C9182.MP4 (1.5 s, Sony 4K 4:2:2 10 bits) por la misma ruta que Python (GPU con
//! `scale_cuda`, o CPU si no hay) con un detector vacío, y compara las medidas de cuadro completo con
//! la caché que escribió Python (`bench_oct27`). Usa la GPU unos segundos: va marcada `#[ignore]`.
//! `cargo test -p camcuts-core --test real_clip -- --ignored --nocapture`

use std::path::Path;

use camcuts_core::analyse::{self, AnalyseOpts};
use camcuts_core::{project, sampler, Detection, Detector, Frame, Pose};

/// Detector sin modelos: no ve nada (las medidas de cuadro completo no dependen de él).
struct Nada;

impl Detector for Nada {
    fn detect(&mut self, _: &Frame) -> anyhow::Result<Vec<Detection>> {
        Ok(vec![])
    }
    fn silhouette(&mut self, _: &Frame, _: [f32; 4]) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("sin modelos")
    }
    fn pose(&mut self, _: &Frame) -> anyhow::Result<Vec<Pose>> {
        Ok(vec![])
    }
    fn verify(&mut self, _: &Frame, _: [f32; 4]) -> anyhow::Result<f32> {
        Ok(0.0)
    }
    fn speech(&mut self, _: &[f32]) -> anyhow::Result<Vec<f32>> {
        Ok(vec![])
    }
}

#[test]
#[ignore]
fn clip_real_como_python() {
    let clip = Path::new(r"D:\CONTENIDO NAPO\2026\Octubre\27\videos\C9182.MP4");
    let cache = Path::new(r"C:\Users\Usuario\Tools\camcuts_next\bench_oct27\cache\C9182_f6531214.json");
    if !clip.exists() || !cache.exists() {
        eprintln!("sin clip o caché: prueba saltada");
        return;
    }
    let py = project::read_entry(cache).unwrap();
    let info = sampler::probe(clip).unwrap();
    assert_eq!(info.start_tc, py.start_tc.clone().unwrap(), "timecode");
    assert_eq!([info.fps.num, info.fps.den], py.fps, "fps");
    assert_eq!((info.width, info.height), (py.width, py.height));
    assert_eq!(info.duration, py.duration);
    let (samples, stats) = analyse::analyse(&info, &mut Nada, &AnalyseOpts { proxy: None, ..Default::default() }, None, |_| {}).unwrap();
    println!("decodificación {} (Python {}), lap_scale {} (Python {})", if stats.gpu { "gpu" } else { "cpu" }, py.decode, stats.lap_scale, py.lap_scale);
    assert_eq!(samples.len(), py.samples.len(), "muestras");
    for (r, p) in samples.iter().zip(&py.samples) {
        println!(
            "t={:.2} edge {:.6}/{:.6} lap_var {:.3}/{:.3} luma {:.6}/{:.6} cam ({:.2},{:.2})/({:.2},{:.2}) log {:?}/{:?}",
            r.t, r.edge_density, p.edge_density, r.lap_var, p.lap_var, r.mean_luma, p.mean_luma, r.cam_dx, r.cam_dy, p.cam_dx, p.cam_dy, r.log, p.log
        );
        assert_eq!(r.log, p.log, "LOG");
        if stats.gpu == (py.decode == "gpu") {
            assert!((r.edge_density - p.edge_density).abs() < 1e-3, "edge_density");
            assert!((r.lap_var - p.lap_var).abs() / p.lap_var < 1e-2, "lap_var");
            assert!((r.mean_luma - p.mean_luma).abs() < 1e-3, "mean_luma");
            assert!((r.cam_dx - p.cam_dx).abs() < 0.5 && (r.cam_dy - p.cam_dy).abs() < 0.5, "cámara");
        }
    }
}
