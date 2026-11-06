//! Comprobación de la decodificación sin modelos: abre cada clip de una lista, lo muestrea a 4/s como el
//! análisis y comprueba que no se cuelga ni falla, que los cuadros salen del tamaño de análisis (rotación
//! aplicada) y que el número de muestras cuadra con la duración. Con `CAMCUTS_DECODE=cpu` prueba la ruta
//! de CPU (la de los equipos sin NVIDIA) sin tocar la GPU.
//!
//! `cargo run -p camcuts-core --release --example decode_check -- <lista.txt> <salida.tsv> [hilos]`

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use camcuts_core::sampler;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let text = std::fs::read_to_string(&a[1]).unwrap();
    let paths: Vec<PathBuf> = text.lines().map(|l| l.trim_start_matches('\u{feff}').trim()).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    let threads: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    camcuts_core::hardware::run_in_background();
    let queue = Mutex::new(paths.into_iter());
    let out = Mutex::new(std::fs::File::create(&a[2]).unwrap());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let Some(p) = queue.lock().unwrap().next() else { return };
                let t0 = Instant::now();
                let row = match sampler::open_clip(&p, &[]) {
                    Err(e) => format!("error_abrir\t{e:#}"),
                    Ok(None) => "sin_proxy".into(),
                    Ok(Some(info)) => {
                        let want = sampler::scaled_size(&info);
                        let mut n = 0usize;
                        let mut bad = 0usize;
                        let mut last = 0.0;
                        let r = sampler::sample(&info, 4.0, false, None, |sm, _| {
                            if (sm.frame.width, sm.frame.height) != want {
                                bad += 1;
                            }
                            n += 1;
                            last = sm.t;
                            Ok(true)
                        });
                        let expect = (info.duration * 4.0).round() as i64;
                        match r {
                            Err(e) => format!("error_decodificar\t{e:#}"),
                            Ok(st) => format!(
                                "{}\tmuestras={n}\tesperadas~{expect}\tdif={}\tmal_tamaño={bad}\t{}x{}\trot={}\tultima_t={last:.2}\tdur={:.2}\t{}",
                                if bad == 0 && (n as i64 - expect).abs() <= 2 { "ok" } else { "REVISAR" },
                                n as i64 - expect,
                                want.0,
                                want.1,
                                info.rotation,
                                info.duration,
                                if st.gpu { "gpu" } else { "cpu" }
                            ),
                        }
                    }
                };
                let line = format!("{}\t{:.1}s\t{row}", p.display(), t0.elapsed().as_secs_f64());
                println!("{line}");
                writeln!(out.lock().unwrap(), "{line}").unwrap();
            });
        }
    });
}
