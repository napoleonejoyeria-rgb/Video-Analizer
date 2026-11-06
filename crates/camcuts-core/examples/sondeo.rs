//! Sondeo masivo sin GPU: abre cada clip de una lista como lo haría el análisis (`open_clip`) y comprueba
//! tamaño, rotación, fps y timecode. En los clips de Sony compara el timecode de inicio con el del XML
//! de la cámara (`<stem>M01.XML`, la fuente de verdad que también lee Resolve).
//!
//! `cargo run -p camcuts-core --release --example sondeo -- <lista.txt> <salida.tsv>`

use std::io::Write;
use std::path::{Path, PathBuf};

use camcuts_core::sampler;
use rayon::prelude::*;

/// Timecode de inicio del XML de Sony en cuadros a la frecuencia del clip, o `None` sin XML.
///
/// `LtcChange value` = 4 bytes BCD (cuadros, segundos, minutos, horas) contados a `tcFps` (30): bit 0x40
/// del de cuadros = drop-frame; bit 0x80 del de segundos = medio paso (segundo cuadro de cada par a 59.94).
fn sony_start(video: &Path, nominal: i64) -> Option<(String, i64)> {
    let stem = video.file_stem()?.to_string_lossy().to_string();
    let xml = std::fs::read_to_string(video.with_file_name(format!("{stem}M01.XML"))).ok()?;
    let at = xml.find("<LtcChange ")?;
    let rest = &xml[at..];
    let v = rest.find("value=\"")? + 7;
    let hex = rest.get(v..v + 8)?;
    let b: Vec<u8> = (0..4).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect();
    let bcd = |x: u8| ((x >> 4) & 0x7) as i64 * 10 + (x & 0xF) as i64;
    let (f, s, m, h) = (bcd(b[0] & 0x3F), bcd(b[1] & 0x7F), bcd(b[2] & 0x7F), bcd(b[3] & 0x3F));
    let df = b[0] & 0x40 != 0;
    let half = (b[1] & 0x80 != 0) as i64;
    let mut f30 = (h * 3600 + m * 60 + s) * 30 + f;
    if df {
        let minutes = h * 60 + m;
        f30 -= 2 * (minutes - minutes / 10);
    }
    let k = (nominal / 30).max(1);
    let tc = format!("{h:02}:{m:02}:{s:02}{}{f:02}{}", if df { ';' } else { ':' }, if half == 1 { "+½" } else { "" });
    Some((tc, f30 * k + half))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let text = std::fs::read_to_string(&a[1]).unwrap();
    let paths: Vec<PathBuf> = text.lines().map(|l| l.trim_start_matches('\u{feff}').trim()).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    let rows: Vec<String> = paths
        .par_iter()
        .map(|p| {
            let name = p.to_string_lossy();
            match sampler::open_clip(p, &[]) {
                Err(e) => format!("{name}\terror\t{}", format!("{e:#}").replace(['\t', '\n'], " ")),
                Ok(None) => format!("{name}\tsin_proxy"),
                Ok(Some(i)) => {
                    let nominal = i.fps.as_f64().round() as i64;
                    let ours = sampler::start_frames(&i) as i64;
                    let (xtc, verdict) = match sony_start(p, nominal) {
                        Some((tc, f)) => (tc, if f == ours { "tc_ok".to_string() } else { format!("TC_MAL({})", ours - f) }),
                        None => (String::new(), "sin_xml".into()),
                    };
                    let (sw, sh) = sampler::scaled_size(&i);
                    format!(
                        "{name}\tok\t{}x{}\trot={}\t{}/{}\tdur={:.3}\tframes={}\ttc={}\tstart={}\txml={}\t{}\tanalisis={}x{}",
                        i.width, i.height, i.rotation, i.fps.num, i.fps.den, i.duration, i.frame_count(), i.start_tc, ours, xtc, verdict, sw, sh
                    )
                }
            }
        })
        .collect();
    let mut f = std::fs::File::create(&a[2]).unwrap();
    for r in &rows {
        writeln!(f, "{r}").unwrap();
    }
    let count = |s: &str| rows.iter().filter(|r| r.contains(s)).count();
    println!("{} clips: ok {} · error {} · tc_ok {} · TC_MAL {} · sin_xml {}", rows.len(), count("\tok\t"), count("\terror\t"), count("tc_ok"), count("TC_MAL"), count("sin_xml"));
}
