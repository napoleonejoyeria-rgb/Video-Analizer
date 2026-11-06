//! Compara cómo se ve un cuadro del visor con cada manera de graduarlo.
//!
//! `cargo run --release --example vista_color -- <clip original> <fuente a decodificar> <segundo> <salida.png>`
//!
//! Decodifica un cuadro de la fuente (proxy u original) igual que el visor (ffmpeg → rgb24), lo gradúa
//! de cuatro formas y guarda una tira: tal cual · "auto" (estirar contraste) · perfil detectado con la
//! decodificación de OpenCV · perfil detectado con la decodificación real de ffmpeg.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};
use camcuts_core::color::{self, Decoding};

fn medida(fuente: &str) -> Result<(usize, usize)> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", fuente])
        .output()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().split(',').map(|v| v.trim().parse::<usize>());
    Ok((it.next().ok_or(anyhow!("sin ancho"))??, it.next().ok_or(anyhow!("sin alto"))??))
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (original, fuente, t, salida) = (&a[1], &a[2], &a[3], &a[4]);
    let es_proxy = fuente != original;
    let (mut w, mut h) = medida(fuente)?;
    let mut c = Command::new("ffmpeg");
    c.args(["-v", "error", "-nostdin"]);
    if es_proxy {
        c.arg("-noautorotate");
    } else if w > h {
        // el original vertical lleva la marca de 90°: ffmpeg lo entrega ya girado
        std::mem::swap(&mut w, &mut h);
    }
    let rgb = c
        .args(["-ss", t, "-i", fuente, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stderr(Stdio::inherit())
        .output()?
        .stdout;
    if rgb.len() != w * h * 3 {
        return Err(anyhow!("cuadro de {} bytes, se esperaban {}", rgb.len(), w * h * 3));
    }
    let bgr: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[2], p[1], p[0]]).collect();
    let perfil = color::detect_profile(original).unwrap_or_else(|| "none".into());
    let dec = color::decoding_for(original, "ffmpeg")?;
    println!("perfil detectado: {perfil} · decodificación ffmpeg: {dec:?}");
    let variantes = [
        ("tal cual", bgr.clone()),
        ("auto", color::make_converter("auto", None, &Decoding::default())?.convert(&bgr, w, h)),
        ("perfil+opencv", color::make_converter(&perfil, None, &Decoding::default())?.convert(&bgr, w, h)),
        ("perfil+ffmpeg", color::make_converter(&perfil, None, &dec)?.convert(&bgr, w, h)),
    ];
    // tira horizontal en PPM, que ffmpeg pasa a PNG
    let n = variantes.len();
    let mut ppm = format!("P6\n{} {}\n255\n", w * n, h).into_bytes();
    for y in 0..h {
        for (_, v) in &variantes {
            for x in 0..w {
                let i = (y * w + x) * 3;
                ppm.extend_from_slice(&[v[i + 2], v[i + 1], v[i]]);
            }
        }
    }
    for (nombre, v) in &variantes {
        let media: f64 = v.iter().map(|&b| b as f64).sum::<f64>() / v.len() as f64;
        let sat: f64 = v.chunks_exact(3).map(|p| (*p.iter().max().unwrap() as f64) - (*p.iter().min().unwrap() as f64)).sum::<f64>() / (w * h) as f64;
        println!("{nombre:>14}: brillo medio {media:5.1} · saturación media {sat:5.1}");
    }
    let tmp = Path::new(salida).with_extension("ppm");
    std::fs::write(&tmp, ppm)?;
    let ok = Command::new("ffmpeg").args(["-v", "error", "-y", "-i"]).arg(&tmp).arg(salida).status()?.success();
    let _ = std::fs::remove_file(&tmp);
    if !ok {
        return Err(anyhow!("ffmpeg no pudo escribir {salida}"));
    }
    Ok(())
}
