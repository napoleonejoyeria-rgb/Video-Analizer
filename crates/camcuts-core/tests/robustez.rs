//! Pruebas de robustez con clips sintéticos (ffmpeg `lavfi`), sin modelos ni material de D:.
//!
//! Cubren los fallos encontrados al pasar el análisis por todo el material de 2026:
//! - proxies de clips verticales que conservaban la matriz de rotación de la cámara (se veían girados o
//!   estirados en el visor) — la decodificación por GPU y por CPU debe dar el mismo cuadro derecho;
//! - archivos vacíos, cortados o solo de audio: error claro por clip, no "RAW sin proxy" ni un pánico;
//! - un lote con un archivo dañado sigue con los demás y lo anota en `errores.txt`;
//! - clips muy cortos, sin audio y a frecuencias raras (20, 25, 23.976, 119.88);
//! - timecode a 23.976 sin drop-frame.
//!
//! Usan ffmpeg (y la GPU un instante si la hay): se saltan si ffmpeg no está instalado.
//! `cargo test -p camcuts-core --test robustez`

use std::path::{Path, PathBuf};
use std::process::Command;

use camcuts_core::analyse::{self, AnalyseOpts};
use camcuts_core::{cli, sampler, Detection, Detector, Frame, Pose};

/// Detector sin modelos: no ve nada.
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

/// Carpeta temporal propia de cada prueba (dentro de `target/`).
fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("robustez").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hay_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Ejecuta ffmpeg en silencio y falla la prueba si termina mal.
fn ffmpeg(args: &[&str]) {
    let o = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().unwrap();
    assert!(o.status.success(), "ffmpeg {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
}

/// Clip de prueba con un patrón asimétrico (testsrc2), sin audio salvo que se pida.
fn clip(dir: &Path, name: &str, size: &str, rate: &str, secs: &str, extra: &[&str]) -> PathBuf {
    let p = dir.join(name);
    let src = format!("testsrc2=size={size}:rate={rate}");
    let mut a: Vec<&str> = vec!["-f", "lavfi", "-i", &src, "-t", secs];
    a.extend_from_slice(extra);
    a.extend_from_slice(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "ultrafast"]);
    let ps = p.to_string_lossy().to_string();
    a.push(&ps);
    ffmpeg(&a);
    p
}

/// Copia `src` con una matriz de rotación de display (como un vertical de Sony o de iPhone).
fn rotated(src: &Path, deg: &str) -> PathBuf {
    let out = src.with_file_name(format!("rot{deg}.mp4"));
    ffmpeg(&["-display_rotation", deg, "-i", &src.to_string_lossy(), "-c", "copy", &out.to_string_lossy()]);
    out
}

/// Rotación y tamaño de la pista de video según ffprobe ("ancho,alto[,rotación]").
fn probe_rot(p: &Path) -> String {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height:stream_side_data=rotation", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Primer cuadro decodificado por un comando de `sampler` (el de GPU o el de CPU).
fn first_frame(cmd: &[String], bytes: usize) -> Option<Vec<u8>> {
    let o = Command::new(&cmd[0]).args(&cmd[1..]).output().ok()?;
    (o.status.success() && o.stdout.len() >= bytes).then(|| o.stdout[..bytes].to_vec())
}

#[test]
fn proxy_vertical_sin_matriz_y_derecho() {
    if !hay_ffmpeg() {
        return;
    }
    let d = tmp("rotacion");
    let base = clip(&d, "base.mp4", "320x180", "30000/1001", "1", &[]);
    for deg in ["90", "-90"] {
        let src = rotated(&base, deg);
        let info = sampler::probe(&src).unwrap();
        assert_eq!((info.width, info.height), (180, 320), "tamaño mostrado ({deg})");
        assert_eq!(info.rotation.abs(), 90);

        // análisis + proxy en la misma pasada (GPU si la hay, CPU si no)
        let proxy = d.join(format!("proxy{deg}.mp4"));
        let mut n = 0;
        sampler::sample(&info, 4.0, false, Some(&proxy), |s, _| {
            assert_eq!((s.frame.width, s.frame.height), (540, 960));
            n += 1;
            Ok(true)
        })
        .unwrap();
        assert!(n >= 4, "muestras: {n}");
        assert_eq!(probe_rot(&proxy), "540,960", "el proxy no debe llevar matriz de rotación ({deg})");

        // proxy suelto (como "crear vistas previas"): todas las variantes, sin matriz
        for (k, cmd) in sampler::proxy_commands(&info, &d.join(format!("solo{deg}.mp4"))).iter().enumerate() {
            let out = PathBuf::from(cmd.last().unwrap());
            if Command::new(&cmd[0]).args(&cmd[1..]).output().map(|o| o.status.success()).unwrap_or(false) {
                assert_eq!(probe_rot(&out), "540,960", "proxy_commands[{k}] ({deg})");
            }
        }

        // GPU y CPU deben girar igual: mismo primer cuadro (salvo el escalador)
        let bytes = 540 * 960 * 3;
        let cpu = first_frame(&sampler::cpu_cmd(&info, 4.0, (540, 960), None), bytes).expect("CPU");
        if let Some(gpu) = sampler::gpu_cmd(&info, 4.0, (540, 960), None).and_then(|c| first_frame(&c, bytes)) {
            let mad = cpu.iter().zip(&gpu).map(|(a, b)| (*a as i32 - *b as i32).abs() as f64).sum::<f64>() / bytes as f64;
            assert!(mad < 12.0, "GPU y CPU difieren (diferencia media {mad:.1}): rotación distinta ({deg})");
        }
    }
}

#[test]
fn archivos_rotos_dan_error_claro() {
    if !hay_ffmpeg() {
        return;
    }
    let d = tmp("rotos");
    // vacío
    let vacio = d.join("vacio.mp4");
    std::fs::write(&vacio, b"").unwrap();
    let e = sampler::open_clip(&vacio, &[]).unwrap_err().to_string();
    assert!(e.contains("vacío"), "{e}");
    // cortado: mp4 con el índice (moov) al final, del que solo queda el principio
    let bueno = clip(&d, "bueno.mp4", "320x180", "25", "2", &[]);
    let bytes = std::fs::read(&bueno).unwrap();
    let cortado = d.join("cortado.mp4");
    std::fs::write(&cortado, &bytes[..bytes.len() / 3]).unwrap();
    let e = sampler::open_clip(&cortado, &[]).unwrap_err().to_string();
    assert!(e.contains("dañado"), "{e}");
    // solo audio (como "WhatsApp Audio ….mp4" o un "Timeline.mp4" exportado sin video)
    let audio = d.join("audio.mp4");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "aac", &audio.to_string_lossy()]);
    let e = sampler::open_clip(&audio, &[]).unwrap_err().to_string();
    assert!(e.contains("sin pista de video"), "{e}");
    // no existe
    assert!(sampler::open_clip(&d.join("no_existe.mp4"), &[]).is_err());
    // cortado con el índice al principio (faststart): se analiza lo que hay y se avisa de hasta dónde
    let fs = clip(&d, "fs.mp4", "320x180", "25", "4", &["-movflags", "+faststart", "-g", "10"]);
    let bytes = std::fs::read(&fs).unwrap();
    let mitad = d.join("fs_mitad.mp4");
    std::fs::write(&mitad, &bytes[..bytes.len() / 2]).unwrap();
    let cache = d.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let (e, st) = analyse::analyse_clip(&mitad, &cache, &[], &mut Nada, &AnalyseOpts::default(), true, None, |_| {}).unwrap().unwrap();
    let cut = st.cut_at.expect("debe avisar de que está cortado");
    assert!(cut < 3.5 && !e.samples.is_empty(), "cortado en {cut}");
    let (_, st) = analyse::analyse_clip(&fs, &cache, &[], &mut Nada, &AnalyseOpts::default(), true, None, |_| {}).unwrap().unwrap();
    assert_eq!(st.cut_at, None, "el entero no está cortado");
    // un RAW sin proxy sigue siendo `None` (lo anota necesitan_proxy.txt)
    let braw = d.join("clip.braw");
    std::fs::write(&braw, b"x").unwrap();
    assert!(sampler::open_clip(&braw, &[]).unwrap().is_none());
}

/// Un archivo cortado deja la marca en su caché, no deja proxy (se acabaría antes de tiempo) y, cuando el
/// archivo se vuelve a copiar entero, el siguiente análisis lo rehace.
#[test]
fn cortado_se_marca_y_se_rehace_al_cambiar() {
    if !hay_ffmpeg() || camcuts_core::edicion::es_bodas() {
        return;
    }
    use clap::Parser;
    let d = tmp("cortado");
    let fuente = clip(&d, "fuente.mp4", "320x180", "25", "4", &["-movflags", "+faststart", "-g", "10"]);
    let entero = std::fs::read(&fuente).unwrap();
    let clips = d.join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    let p = clips.join("X.mp4");
    std::fs::write(&p, &entero[..entero.len() / 2]).unwrap();

    // con proxy: el del archivo cortado no se queda
    let cache_proxy = d.join("cache_proxy");
    std::fs::create_dir_all(&cache_proxy).unwrap();
    let (e, st) = analyse::analyse_clip(&p, &cache_proxy, &[], &mut Nada, &AnalyseOpts::default(), false, None, |_| {}).unwrap().unwrap();
    assert!(st.cut_at.is_some(), "debe verse cortado");
    let proxies: Vec<_> = std::fs::read_dir(cache_proxy.join("proxies")).map(|r| r.flatten().map(|x| x.path()).collect()).unwrap_or_default();
    assert!(proxies.is_empty(), "no debe quedar proxy de un archivo cortado: {proxies:?}");
    assert!(e.proxy.is_none());

    // por la línea de comandos: la caché lleva la marca con la huella del archivo
    let out = d.join("out");
    let args = || cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--workers", "1", "--no-proxy", "--detector", "world"]).unwrap();
    let factory = || -> anyhow::Result<Box<dyn Detector>> { Ok(Box::new(Nada)) };
    cli::run(args(), Some(&factory)).unwrap();
    let leer = || -> serde_json::Value { serde_json::from_slice(&std::fs::read(camcuts_core::project::cache_file(&out.join("cache"), &p)).unwrap()).unwrap() };
    let c = leer();
    assert!(c["cortado"]["hasta"].as_f64().unwrap() < 3.5, "{}", c["cortado"]);
    assert_eq!(c["cortado"]["bytes"].as_u64(), Some((entero.len() / 2) as u64));
    // el mismo archivo: se reutiliza (sin fábrica no podría analizar)
    cli::run(args(), None).expect("la caché del cortado vale mientras el archivo no cambie");
    // copiado entero: se vuelve a analizar y la marca desaparece
    std::fs::write(&p, &entero).unwrap();
    cli::run(args(), Some(&factory)).unwrap();
    let c = leer();
    assert!(c.get("cortado").is_none(), "reanalizado entero: {}", c);
}

/// `-skip_frame noref` deja los cuadros que sí decodifica idénticos a una decodificación completa, también
/// con las B de referencia de x264 por defecto (b-pyramid normal). `bidir` las descartaba y los P que las
/// usan salían con errores ocultados.
#[test]
fn saltar_no_referencia_da_los_mismos_cuadros() {
    if !hay_ffmpeg() {
        return;
    }
    let d = tmp("noref");
    let p = d.join("pyramid.mp4");
    // x264 por defecto (medium): 3 B con b-pyramid normal
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25", "-t", "4", "-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p", &p.to_string_lossy()]);
    // (pts, md5) de cada cuadro decodificado
    let cuadros = |skip: Option<&str>| -> std::collections::BTreeMap<String, String> {
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error"]);
        if let Some(s) = skip {
            c.args(["-skip_frame", s]);
        }
        let o = c.arg("-i").arg(&p).args(["-fps_mode", "passthrough", "-f", "framemd5", "-"]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let f: Vec<&str> = l.split(',').map(str::trim).collect();
                (f.len() >= 6).then(|| (f[2].to_string(), f[5].to_string()))
            })
            .collect()
    };
    let todos = cuadros(None);
    let noref = cuadros(Some("noref"));
    assert!(!noref.is_empty() && noref.len() < todos.len(), "noref decodifica menos: {} de {}", noref.len(), todos.len());
    for (pts, md5) in &noref {
        assert_eq!(todos.get(pts), Some(md5), "cuadro {pts} distinto con -skip_frame noref");
    }
    // informativo: con bidir, cuántos de sus cuadros salían distintos
    let bidir = cuadros(Some("bidir"));
    let distintos = bidir.iter().filter(|(pts, md5)| todos.get(*pts) != Some(*md5)).count();
    println!("noref: {} cuadros, todos iguales; bidir: {} cuadros, {distintos} distintos", noref.len(), bidir.len());
}

#[test]
fn cortos_sin_audio_y_frecuencias_raras() {
    if !hay_ffmpeg() {
        return;
    }
    let d = tmp("raros");
    let cache = d.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    // menos de media muestra (0.1 s a 4/s): ninguna muestra posible, error que lo dice
    let p = clip(&d, "f60.mp4", "320x240", "60", "0.1", &[]);
    let e = analyse::analyse_clip(&p, &cache, &[], &mut Nada, &AnalyseOpts::default(), true, None, |_| {}).unwrap_err().to_string();
    assert!(e.contains("demasiado corto"), "{e}");
    for (name, rate, secs) in [("f20.mp4", "20", "0.3"), ("f25.mp4", "25", "1"), ("f2398.mp4", "24000/1001", "1"), ("f11988.mp4", "120000/1001", "0.5")] {
        let p = clip(&d, name, "320x240", rate, secs, &[]);
        let (e, _) = analyse::analyse_clip(&p, &cache, &[], &mut Nada, &AnalyseOpts::default(), true, None, |_| {}).unwrap().unwrap();
        assert!(!e.samples.is_empty(), "{name}: sin muestras");
        assert!(e.speech.is_empty());
        assert_eq!((e.width, e.height), (320, 240));
        // muestras a 4 por segundo dentro del clip
        assert!(e.samples.last().unwrap().t <= e.duration + 0.25, "{name}");
        let want = sampler::Fraction::parse(rate).unwrap();
        assert_eq!(e.fps, [want.num, want.den], "{name}");
    }
}

#[test]
fn timecode_23976_no_es_drop_frame() {
    if !hay_ffmpeg() {
        return;
    }
    let d = tmp("timecode");
    let p = clip(&d, "tc2398.mov", "320x180", "24000/1001", "0.5", &["-timecode", "01:00:00:00"]);
    let info = sampler::probe(&p).unwrap();
    assert_eq!(info.start_tc, "01:00:00:00");
    assert_eq!(sampler::start_frames(&info), (3600 * 24) as f64);
    let p = clip(&d, "tc2997.mov", "320x180", "30000/1001", "0.5", &["-timecode", "01:00:00;00"]);
    let info = sampler::probe(&p).unwrap();
    assert_eq!(info.start_tc, "01:00:00;00");
    // 1 h a 29.97 DF: 108000 - 2 * 54 cuadros saltados
    assert_eq!(sampler::start_frames(&info), (108000 - 108) as f64);
}

/// Detector sin modelos que dice ser el propio (el entrenado).
struct NadaPropio;

impl Detector for NadaPropio {
    fn detect(&mut self, f: &Frame) -> anyhow::Result<Vec<Detection>> {
        Nada.detect(f)
    }
    fn silhouette(&mut self, f: &Frame, b: [f32; 4]) -> anyhow::Result<Vec<u8>> {
        Nada.silhouette(f, b)
    }
    fn pose(&mut self, f: &Frame) -> anyhow::Result<Vec<Pose>> {
        Nada.pose(f)
    }
    fn verify(&mut self, f: &Frame, b: [f32; 4]) -> anyhow::Result<f32> {
        Nada.verify(f, b)
    }
    fn speech(&mut self, a: &[f32]) -> anyhow::Result<Vec<f32>> {
        Nada.speech(a)
    }
    fn kind(&self) -> camcuts_core::detector::DetectorKind {
        camcuts_core::detector::DetectorKind::Propio
    }
}

/// La caché de cada clip dice con qué detector se analizó, y al pedir el otro el clip se rehace.
#[test]
fn caches_marcan_su_detector() {
    if !hay_ffmpeg() || camcuts_core::edicion::es_bodas() {
        return;
    }
    use clap::Parser;
    let d = tmp("detector");
    let clips = d.join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    let p = clip(&clips, "A.mp4", "320x180", "25", "1", &[]);
    let out = d.join("out");
    let leer = || -> serde_json::Value {
        let f = camcuts_core::project::cache_file(&out.join("cache"), &p);
        serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap()
    };
    // por defecto (YOLO-World): "detector": "world"
    let a = cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--workers", "1", "--no-proxy", "--detector", "world"]).unwrap();
    let f_world = || -> anyhow::Result<Box<dyn Detector>> { Ok(Box::new(Nada)) };
    cli::run(a, Some(&f_world)).unwrap();
    assert_eq!(leer()["detector"], "world");
    // con el propio: la caché de YOLO-World es vieja, se rehace y queda marcada "propio"
    let b = cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--workers", "1", "--no-proxy", "--detector", "propio"]).unwrap();
    let f_propio = || -> anyhow::Result<Box<dyn Detector>> { Ok(Box::new(NadaPropio)) };
    cli::run(b, Some(&f_propio)).unwrap();
    assert_eq!(leer()["detector"], "propio");
    // y otra vez con el propio: se reutiliza (sin modelos ni fábrica no podría analizar)
    let c = cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--no-proxy", "--detector", "propio"]).unwrap();
    cli::run(c, None).expect("la caché del propio vale para el propio");
}

#[test]
fn lote_sigue_tras_un_archivo_danado() {
    if !hay_ffmpeg() {
        return;
    }
    use clap::Parser;
    let d = tmp("lote");
    let clips = d.join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    clip(&clips, "A_bueno.mp4", "320x180", "25", "1", &[]);
    std::fs::write(clips.join("B_vacio.mp4"), b"").unwrap();
    clip(&clips, "C_bueno.mp4", "320x180", "25", "1", &[]);
    std::fs::write(clips.join("._C_bueno.mp4"), b"\x00\x05\x16\x07").unwrap(); // basura de macOS: se ignora
    let out = d.join("out");
    let args = cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--workers", "1", "--no-proxy"]).unwrap();
    let factory = || -> anyhow::Result<Box<dyn Detector>> { Ok(Box::new(Nada)) };
    cli::run(args, Some(&factory)).unwrap();
    let errores = std::fs::read_to_string(out.join("errores.txt")).unwrap();
    assert!(errores.contains("B_vacio.mp4") && errores.contains("vacío"), "{errores}");
    assert!(!errores.contains("._C_bueno"), "los ._ de macOS no son clips: {errores}");
    let n = std::fs::read_dir(out.join("cache")).unwrap().filter(|e| e.as_ref().unwrap().path().extension().map(|x| x == "json").unwrap_or(false)).count();
    assert_eq!(n, 2, "los dos clips buenos analizados");

    // el lote abre cada clip siguiente por adelantado: las muestras deben ser las mismas que analizándolo solo
    let solo = d.join("solo");
    std::fs::create_dir_all(&solo).unwrap();
    for name in ["A_bueno.mp4", "C_bueno.mp4"] {
        let p = clips.join(name);
        let (e, _) = analyse::analyse_clip(&p, &solo, &[], &mut Nada, &AnalyseOpts::default(), true, None, |_| {}).unwrap().unwrap();
        let lote = camcuts_core::project::read_entry(&camcuts_core::project::cache_file(&out.join("cache"), &p)).unwrap();
        assert_eq!(serde_json::to_value(&lote.samples).unwrap(), serde_json::to_value(&e.samples).unwrap(), "{name}: muestras distintas");
    }

    // una caché cortada (análisis matado a mitad de escribir) no tumba --rescore ni el siguiente análisis
    let caches: Vec<PathBuf> = std::fs::read_dir(out.join("cache")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
    let victima = caches.iter().find(|p| p.file_name().unwrap().to_string_lossy().starts_with("C_bueno")).unwrap().clone();
    let bytes = std::fs::read(&victima).unwrap();
    std::fs::write(&victima, &bytes[..bytes.len() / 2]).unwrap();
    let rescore = cli::Args::try_parse_from(["camcuts", "--out", out.to_str().unwrap(), "--rescore"]).unwrap();
    cli::run(rescore, None).expect("--rescore con una caché cortada");
    let csv = std::fs::read_to_string(out.join("selects_joyas.csv")).unwrap_or_default();
    assert!(!csv.contains("C_bueno"), "la caché cortada no debe exportarse");
    // al analizar otra vez, la cortada cuenta como sin analizar y se reescribe entera
    let args = cli::Args::try_parse_from(["camcuts", clips.to_str().unwrap(), "--out", out.to_str().unwrap(), "--workers", "1", "--no-proxy"]).unwrap();
    cli::run(args, Some(&factory)).unwrap();
    assert!(camcuts_core::project::read_entry(&victima).is_ok(), "caché reescrita");
}
