//! Cronometra las medidas de CPU de una muestra (sin modelos) sobre un cuadro BGR crudo de 960x540.
//!
//! Uso: `cargo run --release -p camcuts-core --example bench_measure -- cuadro.bgr [ancho alto]`
//! (el cuadro se saca con `ffmpeg ... -vf scale=960:540 -frames:v 1 -f rawvideo -pix_fmt bgr24 cuadro.bgr`).

use std::time::Instant;

use camcuts_core::jewel::{self, Det, MeasureOpts, MeasureState};
use camcuts_core::{image, metrics, Frame};

/// Ejecuta `f` `n` veces y devuelve los milisegundos por llamada.
fn time<T>(n: usize, mut f: impl FnMut() -> T) -> f64 {
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(f());
    }
    t.elapsed().as_secs_f64() * 1000.0 / n as f64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("cuadro");
    let (w, h) = (args.get(2).map(|s| s.parse().unwrap()).unwrap_or(960usize), args.get(3).map(|s| s.parse().unwrap()).unwrap_or(540usize));
    let frame = Frame { width: w as u32, height: h as u32, data };
    // huella de los resultados: debe ser idéntica antes y después de optimizar
    {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        let g = image::bgr_to_gray(&frame.data, w, h);
        let m = metrics::edges_and_laplacian(&g, w, h);
        m.mask.hash(&mut hs);
        m.lap.data.iter().for_each(|v| v.to_bits().hash(&mut hs));
        metrics::normalize_contrast(&frame.data, w, h, 0.5, 99.5).hash(&mut hs);
        let mut cs = metrics::CameraShift::new();
        cs.shift(&g, w, h);
        // segundo cuadro desplazado 7 px para que la correlación de fase tenga algo que medir
        let g2: Vec<u8> = (0..w * h).map(|i| g[(i + 7) % (w * h)]).collect();
        let (dx, dy, r) = cs.shift(&g2, w, h);
        println!("shift = ({dx}, {dy}, {r})");
        dx.to_bits().hash(&mut hs);
        dy.to_bits().hash(&mut hs);
        r.to_bits().hash(&mut hs);
        println!("huella = {:016x}", hs.finish());
    }
    let n = 200;
    let gray = image::bgr_to_gray(&frame.data, w, h);
    println!("bgr_to_gray        {:7.3} ms", time(n, || image::bgr_to_gray(&frame.data, w, h)));
    println!("thumb              {:7.3} ms", time(n, || metrics::thumb(&gray, w, h)));
    println!("is_flat            {:7.3} ms", time(n, || metrics::is_flat(&frame.data, w, h, 150.0)));
    println!("normalize_contrast {:7.3} ms", time(n, || metrics::normalize_contrast(&frame.data, w, h, 0.5, 99.5)));
    println!("edges_and_lap      {:7.3} ms", time(n, || metrics::edges_and_laplacian(&gray, w, h)));
    println!("exposure           {:7.3} ms", time(n, || metrics::exposure(&gray)));
    let mut cs = metrics::CameraShift::new();
    println!("camera_shift       {:7.3} ms", time(n, || cs.shift(&gray, w, h)));
    let maps = metrics::edges_and_laplacian(&gray, w, h);
    println!("density_full       {:7.3} ms", time(n, || metrics::density_full(&maps)));
    println!("lap_var full       {:7.3} ms", time(n, || metrics::laplacian_variance(&maps.lap, None)));
    println!("anisotropy 200px   {:7.3} ms", time(n, || metrics::blur_anisotropy(&gray, w, [300, 200, 500, 400])));
    let dets = vec![
        Det { bbox: [300, 200, 500, 400], conf: 0.6, label: "anillo".into(), zoom: false, verified: false },
        Det { bbox: [600, 100, 700, 250], conf: 0.4, label: "arete".into(), zoom: false, verified: false },
        Det { bbox: [100, 0, 900, 540], conf: 0.9, label: "persona".into(), zoom: false, verified: false },
    ];
    let mo = MeasureOpts { dt: 0.25, min_conf: 0.15, lap_scale: 1.0, use_sam: false };
    let mut st = MeasureState::default();
    let mut t = 0.0;
    println!(
        "measure (sin SAM)  {:7.3} ms",
        time(n, || {
            t += 0.25;
            jewel::measure(t, &frame, &dets, &mut st, &mo, None, false)
        })
    );
}
