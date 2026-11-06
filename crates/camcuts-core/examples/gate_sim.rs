//! Simulación (sin GPU) de variantes de la puerta de movimiento sobre los proxies de una carpeta de
//! resultados: cuántas muestras irían a la GPU con la puerta actual (la de Python) y con alternativas.
//!
//! `cargo run -p camcuts-core --release --example gate_sim -- <carpeta de resultados>`

use std::process::{Command, Stdio};

use camcuts_core::{image, metrics, project};

/// Diferencia media absoluta entre dos miniaturas de 96x96 con `b` desplazada (sx, sy), en la zona común.
fn mad_shift(a: &[u8], b: &[u8], sx: i32, sy: i32) -> f64 {
    let n = 96i32;
    let (mut s, mut c) = (0u64, 0u64);
    for y in 0.max(-sy)..n.min(n - sy) {
        for x in 0.max(-sx)..n.min(n - sx) {
            let pa = a[(y * n + x) as usize] as i32;
            let pb = b[((y + sy) * n + x + sx) as usize] as i32;
            s += (pa - pb).unsigned_abs() as u64;
            c += 1;
        }
    }
    s as f64 / c.max(1) as f64
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut tot = [0usize; 7];
    println!("{:<14} {:>5} {:>6} {:>6} {:>7} {:>7} {:>7} {:>7}", "clip", "n", "cache", "base", "shift3", "tol3.5", "tol5", "brillo");
    for f in project::cache_files(&out.join("cache")) {
        let e = project::read_entry(&f).unwrap();
        let Some(proxy) = e.proxy.clone().filter(|p| std::path::Path::new(p).exists()) else { continue };
        let (w, h) = camcuts_core::sampler::scaled_size(&e.clip_info());
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-i", &proxy, "-vf", &format!("fps={}", e.per_second), "-f", "rawvideo", "-pix_fmt", "bgr24", "pipe:1"]).stdout(Stdio::piped());
        let raw = c.output().unwrap().stdout;
        let fb = (w * h * 3) as usize;
        let thumbs: Vec<Vec<u8>> = raw.chunks_exact(fb).map(|fr| metrics::thumb(&image::bgr_to_gray(fr, w as usize, h as usize), w as usize, h as usize)).collect();
        // variantes: (tolerancia, desplazamiento máximo, compensar brillo)
        let variants: [(f64, i32, bool); 5] = [(2.5, 0, false), (2.5, 3, false), (3.5, 0, false), (5.0, 0, false), (2.5, 0, true)];
        let mut fresh = [0usize; 5];
        for (k, &(tol, r, bright)) in variants.iter().enumerate() {
            let mut anchor: Option<&Vec<u8>> = None;
            for th in &thumbs {
                let still = anchor.map(|a| {
                    if bright {
                        let ma = a.iter().map(|&v| v as f64).sum::<f64>() / a.len() as f64;
                        let mb = th.iter().map(|&v| v as f64).sum::<f64>() / th.len() as f64;
                        let d = mb - ma;
                        a.iter().zip(th).map(|(&x, &y)| (y as f64 - x as f64 - d).abs()).sum::<f64>() / a.len() as f64 <= tol
                    } else {
                        let mut best = f64::MAX;
                        for sy in -r..=r {
                            for sx in -r..=r {
                                best = best.min(mad_shift(a, th, sx, sy));
                            }
                        }
                        best <= tol
                    }
                });
                if still != Some(true) {
                    anchor = Some(th);
                    fresh[k] += 1;
                }
            }
        }
        let n = thumbs.len();
        let name = std::path::Path::new(&e.path).file_name().unwrap().to_string_lossy().to_string();
        println!("{:<14} {:>5} {:>6} {:>6} {:>7} {:>7} {:>7} {:>7}", name, n, e.samples.len() as i64 - e.reused, fresh[0], fresh[1], fresh[2], fresh[3], fresh[4]);
        tot[0] += n;
        tot[1] += (e.samples.len() as i64 - e.reused) as usize;
        for k in 0..5 {
            tot[2 + k] += fresh[k];
        }
    }
    println!("{:<14} {:>5} {:>6} {:>6} {:>7} {:>7} {:>7} {:>7}", "TOTAL", tot[0], tot[1], tot[2], tot[3], tot[4], tot[5], tot[6]);
}
