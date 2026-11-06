//! Comprobación de los modelos de la edición Bodas (detector de sujetos y momentos) en CPU.
//!
//! Uso: `cargo run -p camcuts-infer --example bodas_check -- <imagen.png>...`
//! Imprime, por imagen, las detecciones (≥ 0.05) y los tres momentos más probables, para compararlas con
//! las de Python (`models/export_bodas.py` exporta los mismos sujetos y textos).

use camcuts_core::{edicion, Detector, Frame};
use camcuts_infer::{Device, InferConfig, OnnxDetector};

fn main() -> anyhow::Result<()> {
    edicion::fijar(edicion::Edicion::Bodas);
    let cfg = InferConfig { device: Device::Cpu, detect_conf: 0.05, ..Default::default() };
    let mut det = OnnxDetector::new(cfg)?;
    for ruta in std::env::args().skip(1) {
        let img = image::open(&ruta)?.to_rgb8();
        let (w, h) = img.dimensions();
        // RGB → BGR, como los cuadros del muestreador
        let data: Vec<u8> = img.pixels().flat_map(|p| [p[2], p[1], p[0]]).collect();
        let frame = Frame { width: w, height: h, data };
        println!("== {ruta} ({w}x{h})");
        for d in det.detect(&frame)? {
            println!("  {:<10} {:.3}  [{:.0}, {:.0}, {:.0}, {:.0}]", d.label, d.conf, d.x1, d.y1, d.x2, d.y2);
        }
        let mut m = det.moments(&frame)?;
        m.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top: Vec<String> = m.iter().take(3).map(|(k, p)| format!("{k} {p:.3}")).collect();
        println!("  momentos: {}", top.join(", "));
    }
    Ok(())
}
