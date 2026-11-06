//! Medida rápida de velocidad de cada modelo (ms por cuadro) con DirectML, en tandas cortas para no
//! acaparar la GPU mientras el usuario edita.
//!
//! `cargo run -p camcuts-infer --release --example bench [-- cpu]`
//!
//! Usa el cuadro `fixtures/f2_dije_pulsera.png` (960×540, como los de la app).

use std::path::Path;
use std::time::Instant;

use camcuts_core::{Detector, Frame};
use camcuts_infer::{Device, InferConfig, OnnxDetector, WorldSize};

/// Repeticiones medidas por modelo (tras 2 de calentamiento, que compilan los grafos de DirectML).
const REPS: usize = 15;

fn load_frame(p: &Path) -> Frame {
    let img = image::open(p).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_mut(3) {
        px.swap(0, 2);
    }
    Frame { width: w, height: h, data }
}

/// Milisegundos medios de `f` tras 2 llamadas de calentamiento.
fn ms(mut f: impl FnMut()) -> f64 {
    f();
    f();
    let t = Instant::now();
    for _ in 0..REPS {
        f();
    }
    t.elapsed().as_secs_f64() * 1000.0 / REPS as f64
}

fn main() {
    let device = if std::env::args().any(|a| a == "cpu") { Device::Cpu } else { Device::DirectMl };
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let frame = load_frame(&fx.join("f2_dije_pulsera.png"));
    let six: Vec<Frame> = (0..6).map(|_| frame.clone()).collect();
    let six_ref: Vec<&Frame> = six.iter().collect();
    let boxx = [436.0, 182.0, 581.0, 330.0];
    println!("dispositivo: {device:?}");
    // lo que hace la CPU antes de cada llamada (letterbox + tensor CHW) y después de SAM (máscara al cuadro)
    {
        use camcuts_infer::imgops::{bgr_to_chw, bilinear_threshold, letterbox, letterbox_params};
        let (h, w) = (frame.height as usize, frame.width as usize);
        let sam_lb = letterbox_params(h, w, (1024, 1024), false, 32, false);
        let yolo_lb = letterbox_params(h, w, (960, 960), true, 32, true);
        let pre = |lb: &camcuts_infer::imgops::LetterboxInfo, mean: [f32; 3], std: [f32; 3], scale: f32| {
            let img = letterbox(&frame.data, lb, 114);
            let mut v = Vec::new();
            bgr_to_chw(&img, lb.input.0, lb.input.1, scale, mean, std, &mut v);
            v
        };
        let s = ms(|| {
            std::hint::black_box(pre(&sam_lb, [123.675, 116.28, 103.53], [58.395, 57.12, 57.375], 1.0));
        });
        let y = ms(|| {
            std::hint::black_box(pre(&yolo_lb, [0.0; 3], [1.0; 3], 255.0));
        });
        let m = vec![0.5f32; 256 * 256];
        let b = ms(|| {
            std::hint::black_box(bilinear_threshold(&m, 256, 256, 144.0, 256.0, h, w, 0.0));
        });
        println!("CPU: preparar SAM {s:.1} ms, preparar YOLO {y:.1} ms, máscara de SAM al cuadro {b:.1} ms");
    }
    for fp16 in [true, false] {
        for world in [WorldSize::X, WorldSize::M] {
            // YOLO-World a mano: el detector por defecto es ahora el propio y esto mide YOLO-World
            let mut d = OnnxDetector::new(InferConfig { device, world, fp16_world: fp16, batch_world: 6, detector: camcuts_infer::DetectorKind::World, ..InferConfig::default() }).unwrap();
            let b1 = ms(|| {
                d.detect(&frame).unwrap();
            });
            let b6 = ms(|| {
                d.detect_batch(&six_ref).unwrap();
            }) / 6.0;
            println!("YOLO-World {world:?} fp16={fp16}: lote 1 {b1:.1} ms/cuadro, lote 6 {b6:.1} ms/cuadro");
        }
        let cfg = InferConfig { device, fp16_sam: fp16, fp16_pose: fp16, fp16_clip: fp16, ..InferConfig::default() };
        let mut d = OnnxDetector::new(cfg).unwrap();
        let p = ms(|| {
            d.pose(&frame).unwrap();
        });
        println!("pose fp16={fp16}: {p:.1} ms/cuadro");
        // SAM: cuadro distinto en cada llamada para que el codificador corra siempre (sin caché)
        let mut k = 0u8;
        let sam1 = ms(|| {
            let mut f = frame.clone();
            k = k.wrapping_add(1);
            f.data[0] = k;
            d.silhouettes(&f, &[boxx]).unwrap();
        });
        let sam3 = ms(|| {
            let mut f = frame.clone();
            k = k.wrapping_add(1);
            f.data[0] = k;
            d.silhouettes(&f, &[boxx, boxx, boxx]).unwrap();
        });
        let dec = ms(|| {
            d.silhouettes(&frame, &[boxx]).unwrap();
        });
        println!("MobileSAM fp16={fp16}: 1 caja {sam1:.1} ms, 3 cajas {sam3:.1} ms, solo decodificador (embedding guardado) {dec:.1} ms");
        let c4 = ms(|| {
            d.verify_batch(&frame, &[boxx, boxx, boxx, boxx]).unwrap();
        }) / 4.0;
        println!("CLIP fp16={fp16}: {c4:.1} ms/recorte (lote 4)");
    }
    let mut d = OnnxDetector::new(InferConfig { detector: camcuts_infer::DetectorKind::World, ..InferConfig::default() }).unwrap();
    let audio = vec![0.01f32; 512 * 200];
    let v = ms(|| {
        d.speech(&audio).unwrap();
    }) / 200.0;
    println!("Silero VAD (CPU): {v:.3} ms por ventana de 512");
}
