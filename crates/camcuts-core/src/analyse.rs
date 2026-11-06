//! Bucle de análisis de un clip (port de `analyse` y `work` de `__main__.py`).
//!
//! Solo las muestras que cambiaron desde la última que pasó por la GPU se vuelven a detectar; las
//! quietas reutilizan sus detecciones (y las siluetas, en `measure`). El resultado es exactamente el
//! mismo que el de Python por lotes: la decisión de "fresca o quieta" depende solo de la miniatura de
//! la última muestra detectada, así que hacerlo muestra a muestra da las mismas detecciones.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::{Map, Value};

use crate::color::{self, Converter};
use crate::image;
use crate::jewel::{self, Det, MeasureOpts, MeasureState, Sample};
use crate::metrics;
use crate::project::{self, ClipEntry};
use crate::sampler::{self, ClipInfo};
use crate::types::{Detector, Frame, Pose};

/// Detección más débil que aún merece una segunda opinión de CLIP.
pub const VERIFY_MIN: f64 = 0.02;
/// Probabilidad de "joya" de CLIP necesaria para conservar una detección débil.
pub const VERIFY_ACCEPT: f32 = 0.6;
/// Confianza mínima de la segunda mirada con zoom (la de `detector.detect` por defecto en Python).
pub const ZOOM_CONF: f64 = 0.05;
/// Confianza mínima de las poses (la del `PoseFinder` de Python).
pub const POSE_CONF: f32 = 0.4;

/// Opciones del análisis de un clip.
#[derive(Debug, Clone)]
pub struct AnalyseOpts {
    /// Muestras por segundo (`--fps`, 4 por defecto).
    pub per_second: f64,
    /// Confianza mínima de una joya (la del preset).
    pub min_conf: f64,
    /// Medir sobre la silueta (MobileSAM) en vez de la caja.
    pub use_sam: bool,
    /// Segunda mirada a 1920 px en orejas, cuello y muñecas.
    pub zoom: bool,
    /// Escribir el proxy de vista previa aquí en la misma pasada.
    pub proxy: Option<PathBuf>,
    /// Conversión de color elegida (`{"id": ..., "cube": ...}`; `None` = automática).
    pub color: Option<Value>,
}

impl Default for AnalyseOpts {
    fn default() -> Self {
        AnalyseOpts { per_second: 4.0, min_conf: 0.15, use_sam: true, zoom: false, proxy: None, color: None }
    }
}

/// Qué pasó en el análisis (`stats` de Python).
#[derive(Debug, Clone, Copy, Default)]
pub struct AnalyseStats {
    pub gpu: bool,
    pub lap_scale: f64,
    /// Muestras quietas que reutilizaron la detección anterior.
    pub reused: usize,
    /// El archivo está cortado: solo se pudo analizar hasta este segundo.
    pub cut_at: Option<f64>,
    /// Siluetas de SAM que fallaron (esas cajas se midieron enteras; ver `MeasureState::sam_fallos`).
    pub sam_fallos: usize,
}

/// Clave de la caché de un clip cuyo archivo estaba cortado: `{"hasta": s, "bytes": tamaño, "mtime": s}`.
/// Con ella la CLI avisa en cada pasada y, si el archivo cambia (se copió entero), lo vuelve a analizar.
pub const CLAVE_CORTADO: &str = "cortado";

/// Clave de la caché que dice que SAM falló en alguna caja (`false`): se midió sobre la caja.
pub const CLAVE_SAM: &str = "sam";

/// Tamaño y fecha de modificación (segundos desde 1970) de un archivo, para saber si cambió.
pub fn huella_archivo(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), mtime))
}

/// Marca de archivo cortado para la caché ([`CLAVE_CORTADO`]).
fn marca_cortado(path: &Path, hasta: f64) -> Value {
    let (bytes, mtime) = huella_archivo(path).unwrap_or((0, 0));
    serde_json::json!({"hasta": hasta, "bytes": bytes, "mtime": mtime})
}

/// Una caché marcada como cortada ¿sigue valiendo? Solo si el archivo es el mismo que se analizó (mismo
/// tamaño y fecha); si cambió (se volvió a copiar entero), hay que analizarlo otra vez. Sin marca, sí.
pub fn cortado_vigente(entry: &ClipEntry) -> bool {
    let Some(c) = entry.extra.get(CLAVE_CORTADO) else { return true };
    let quiero = (c.get("bytes").and_then(Value::as_u64), c.get("mtime").and_then(Value::as_u64));
    huella_archivo(Path::new(&entry.path)).map(|(b, m)| (Some(b), Some(m)) == quiero).unwrap_or(true)
}

/// Perfil de tiempos por etapa (solo con la variable de entorno `CAMCUTS_PROFILE`): se imprime en stderr
/// al terminar cada clip, para ver dónde se va el tiempo sin herramientas externas.
#[derive(Default, Debug, Clone)]
pub struct Profile {
    /// (nombre de etapa, segundos acumulados, llamadas)
    pub stages: Vec<(&'static str, f64, usize)>,
}

impl Profile {
    /// ¿Está activado el perfil?
    pub fn enabled() -> bool {
        std::env::var_os("CAMCUTS_PROFILE").is_some()
    }
    /// Suma `secs` a la etapa `name`.
    pub fn add(&mut self, name: &'static str, secs: f64) {
        match self.stages.iter_mut().find(|s| s.0 == name) {
            Some(s) => {
                s.1 += secs;
                s.2 += 1;
            }
            None => self.stages.push((name, secs, 1)),
        }
    }
    /// Una línea por etapa: "etapa: 1.23 s (45 llamadas)".
    pub fn report(&self, title: &str) -> String {
        let mut s = format!("[perfil] {title}");
        for (n, t, c) in &self.stages {
            s.push_str(&format!("\n  {n:<14} {t:7.2} s  ({c})"));
        }
        s
    }
}

/// Envoltorio de un detector que cronometra cada llamada a los modelos.
struct Timed<'a> {
    inner: &'a mut dyn Detector,
    prof: &'a std::sync::Mutex<Profile>,
}

impl Detector for Timed<'_> {
    fn detect(&mut self, frame: &Frame) -> Result<Vec<crate::types::Detection>> {
        let t = std::time::Instant::now();
        let r = self.inner.detect(frame);
        self.prof.lock().unwrap().add("yolo-world", t.elapsed().as_secs_f64());
        r
    }
    fn silhouette(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<Vec<u8>> {
        let t = std::time::Instant::now();
        let r = self.inner.silhouette(frame, bbox);
        self.prof.lock().unwrap().add("sam", t.elapsed().as_secs_f64());
        r
    }
    fn pose(&mut self, frame: &Frame) -> Result<Vec<Pose>> {
        let t = std::time::Instant::now();
        let r = self.inner.pose(frame);
        self.prof.lock().unwrap().add("pose", t.elapsed().as_secs_f64());
        r
    }
    fn verify(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<f32> {
        let t = std::time::Instant::now();
        let r = self.inner.verify(frame, bbox);
        self.prof.lock().unwrap().add("clip", t.elapsed().as_secs_f64());
        r
    }
    fn speech(&mut self, audio_16k: &[f32]) -> Result<Vec<f32>> {
        let t = std::time::Instant::now();
        let r = self.inner.speech(audio_16k);
        self.prof.lock().unwrap().add("vad-model", t.elapsed().as_secs_f64());
        r
    }
    fn moments(&mut self, frame: &Frame) -> Result<Vec<(String, f32)>> {
        let t = std::time::Instant::now();
        let r = self.inner.moments(frame);
        self.prof.lock().unwrap().add("clip-momento", t.elapsed().as_secs_f64());
        r
    }
    /// El detector de verdad (el envoltorio solo cronometra).
    fn kind(&self) -> crate::detector::DetectorKind {
        self.inner.kind()
    }
    fn en_lugar_de(&self) -> Option<crate::detector::DetectorKind> {
        self.inner.en_lugar_de()
    }
}

/// Promueve detecciones débiles de joya que CLIP confirma (en sitio). Salta las ya cubiertas.
pub fn verify_weak(frame: &Frame, dets: &mut [Det], min_conf: f64, verifier: &mut dyn Detector) {
    let strong: Vec<[i64; 4]> = dets.iter().filter(|d| jewel::is_jewel_label(&d.label) && d.conf >= min_conf).map(|d| d.bbox).collect();
    let mut weak: Vec<usize> = (0..dets.len())
        .filter(|&i| {
            let d = &dets[i];
            jewel::is_jewel_label(&d.label) && VERIFY_MIN <= d.conf && d.conf < min_conf && !strong.iter().any(|b| jewel::overlap(&d.bbox, b) > 0.5)
        })
        .collect();
    weak.sort_by(|&a, &b| dets[b].conf.total_cmp(&dets[a].conf));
    weak.truncate(4);
    // Un recorte por llamada a propósito: Python los manda juntos, pero con DirectML un lote de 2-4
    // recortes tarda ~10 veces más que uno solo (5 ms con 1, 52-62 ms con 2-4; medido en la RTX 5060).
    for i in weak {
        // caja cruda de la detección: `Detector::verify` (camcuts-infer) añade él mismo el 15 % de
        // margen que Python pone al recorte antes de pasárselo a CLIP
        let b = dets[i].bbox;
        if b[2] <= b[0] || b[3] <= b[1] {
            continue;
        }
        if let Ok(p) = verifier.verify(frame, [b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32]) {
            if p >= VERIFY_ACCEPT {
                dets[i].conf = dets[i].conf.max(min_conf);
                dets[i].verified = true;
            }
        }
    }
}

/// Recorta una región de un cuadro BGR.
fn crop(f: &Frame, x0: usize, y0: usize, x1: usize, y1: usize) -> Frame {
    let (x1, y1) = (x1.min(f.width as usize), y1.min(f.height as usize));
    let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let mut data = Vec::with_capacity(cw * ch * 3);
    for y in y0..y0 + ch {
        let row = y * f.width as usize * 3;
        data.extend_from_slice(&f.data[row + x0 * 3..row + (x0 + cw) * 3]);
    }
    Frame { width: cw as u32, height: ch as u32, data }
}

/// Mide todas las muestras de un clip con el detector dado.
///
/// `cancel` corta el análisis (devuelve lo medido hasta ahí); `progress` recibe la fracción hecha.
pub fn analyse(info: &ClipInfo, detector: &mut dyn Detector, opts: &AnalyseOpts, cancel: Option<&AtomicBool>, progress: impl FnMut(f64)) -> Result<(Vec<Sample>, AnalyseStats)> {
    let prof = std::sync::Mutex::new(Profile::default());
    analyse_profiled(info, detector, opts, cancel, progress, &prof, None, &mut || {})
}

/// [`analyse`] anotando en `prof` el tiempo de cada etapa propia (espera del decodificador, color,
/// medidas). Los modelos se cronometran aparte si `detector` es un envoltorio [`Timed`].
#[allow(clippy::too_many_arguments)]
fn analyse_profiled(
    info: &ClipInfo,
    detector: &mut dyn Detector,
    opts: &AnalyseOpts,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(f64),
    prof: &std::sync::Mutex<Profile>,
    prepared: Option<sampler::Prepared>,
    on_eof: &mut dyn FnMut(),
) -> Result<(Vec<Sample>, AnalyseStats)> {
    use std::time::Instant;
    // fin de la última muestra: lo que pasa hasta la siguiente es espera de ffmpeg
    let mut last_end = Instant::now();
    let mut samples: Vec<Sample> = Vec::new();
    let mut flat: Vec<bool> = Vec::new();
    let mut stats = AnalyseStats { gpu: false, lap_scale: 1.0, reused: 0, cut_at: None, sam_fallos: 0 };
    let mut state = MeasureState::default();
    // la última muestra que pasó por la GPU
    let mut anchor_thumb: Option<Vec<u8>> = None;
    let mut anchor_dets: Vec<Det> = Vec::new();
    let mut anchor_kps: Vec<Pose> = Vec::new();
    // edición Bodas: además del sujeto, el momento de la boda (CLIP) de la última muestra detectada
    let bodas = crate::edicion::es_bodas();
    let mut anchor_mom: Map<String, Value> = Map::new();

    // Color: la conversión elegida por el editor (S-Log3 → Rec.709, una LUT .cube…), o "auto" (estirar
    // el contraste solo si el clip se ve plano), o "none". Detección y nitidez van sobre el cuadro
    // convertido, así que cambiar la conversión obliga a reanalizar el clip.
    let color = opts.color.clone().filter(|c| !c.is_null()).unwrap_or_else(project::auto_color);
    let cid = color.get("id").and_then(|v| v.as_str()).unwrap_or("auto").to_string();
    let auto = cid == "auto";
    let convert: Option<std::sync::Arc<dyn Converter>> = if auto || cid == "none" {
        None
    } else {
        // el sampler decodifica con ffmpeg, que respeta las etiquetas de rango/matriz del archivo
        let d = color::decoding_for(&info.path.to_string_lossy(), "ffmpeg")?;
        Some(color::make_converter(&cid, color.get("cube").and_then(|v| v.as_str()), &d)?)
    };
    let dt = 1.0 / opts.per_second;
    let duration = info.duration.max(1e-9);

    let dec = sampler::sample_with(info, opts.per_second, opts.zoom, opts.proxy.as_deref(), prepared, on_eof, |sm, ds| {
        // la espera hasta la primera muestra es el arranque de ffmpeg (CUDA, NVDEC, primer GOP)
        prof.lock().unwrap().add(if samples.is_empty() { "decode-first" } else { "decode-wait" }, last_end.elapsed().as_secs_f64());
        if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            return Ok(false);
        }
        stats.gpu = ds.gpu;
        stats.lap_scale = ds.lap_scale;
        let t_pre = Instant::now();
        let (w, h) = (sm.frame.width as usize, sm.frame.height as usize);
        let th = metrics::thumb(&image::bgr_to_gray(&sm.frame.data, w, h), w, h);
        let mut frame = sm.frame;
        let mut detail = sm.detail;
        let is_log;
        if auto {
            // el LOG es una propiedad de la grabación: se decide en las primeras muestras y se mantiene
            is_log = if flat.len() < 4 { metrics::is_flat(&frame.data, w, h, 150.0) } else { flat[..4].iter().filter(|&&b| b).count() >= 2 };
            if is_log {
                frame.data = metrics::normalize_contrast(&frame.data, w, h, 0.5, 99.5);
                if let Some(d) = detail.as_mut() {
                    d.data = metrics::normalize_contrast(&d.data, d.width as usize, d.height as usize, 0.5, 99.5);
                }
            }
        } else {
            is_log = convert.is_some();
            if let Some(c) = &convert {
                frame.data = c.convert(&frame.data, w, h);
                if let Some(d) = detail.as_mut() {
                    d.data = c.convert(&d.data, d.width as usize, d.height as usize);
                }
            }
        }
        flat.push(is_log);
        prof.lock().unwrap().add("thumb+color", t_pre.elapsed().as_secs_f64());

        // medidas de cuadro entero (gris, bordes, exposición, cámara): no dependen de las detecciones
        let frame_maps = |frame: &Frame, shift: &mut metrics::CameraShift| {
            let t = Instant::now();
            let pre = jewel::frame_maps(frame, shift);
            prof.lock().unwrap().add("frame-maps", t.elapsed().as_secs_f64());
            pre
        };

        // ¿necesita la GPU? solo si cambió desde la última muestra que pasó por ella
        // (probado en Octubre 27: una puerta que compensa paneos ahorraba un 15 % de detecciones pero
        // perdía joyas pequeñas nítidas que YOLO solo ve en algunos cuadros; ver `examples/gate_sim.rs`)
        let (pre, (dets, kps, mom)) = if metrics::changed(anchor_thumb.as_deref(), &th, 2.5) {
            anchor_thumb = Some(th);
            // la CPU mide el cuadro en otro hilo mientras este espera a la GPU (detección, CLIP, pose):
            // el resultado es el mismo que en serie, solo cambia cuándo se calcula
            let (fr, shift) = (&frame, &mut state.shift);
            let (pre, gpu) = std::thread::scope(|sc| {
                let cpu = sc.spawn(move || frame_maps(fr, shift));
                let gpu = if bodas {
                    crate::bodas::detect_sample(&frame, detail.as_ref(), opts.zoom, &mut *detector)
                } else {
                    detect_sample(&frame, detail.as_ref(), opts, &mut *detector).map(|(d, k)| (d, k, Map::new()))
                };
                (cpu.join().unwrap_or_else(|e| std::panic::resume_unwind(e)), gpu)
            });
            let (dets, kps, mom) = gpu?;
            anchor_dets = dets.clone();
            anchor_kps = kps.clone();
            anchor_mom = mom.clone();
            (pre, (dets, kps, mom))
        } else {
            // muestra quieta: toma el último resultado de la GPU anterior a ella
            stats.reused += 1;
            (frame_maps(&frame, &mut state.shift), (anchor_dets.clone(), anchor_kps.clone(), anchor_mom.clone()))
        };

        let mo = MeasureOpts { dt, min_conf: opts.min_conf, lap_scale: stats.lap_scale, use_sam: opts.use_sam };
        let seg: Option<&mut dyn Detector> = if opts.use_sam { Some(&mut *detector) } else { None };
        let t_m = Instant::now();
        let mut s = if bodas {
            // caras que el detector no vio, sacadas de la pose; el momento en bruto va a la muestra
            let mut d = dets.clone();
            d.extend(crate::bodas::pose_faces(&kps, &dets));
            let mut s = crate::bodas::measure(sm.t, &frame, pre, &d, &mut state, &mo, seg, jewel::has_face(&kps), &crate::bodas::face_points(&kps), &crate::bodas::back_points(&kps));
            s.extra.extend(mom);
            s
        } else {
            jewel::measure_with(sm.t, &frame, pre, &dets, &mut state, &mo, seg, jewel::has_face(&kps))
        };
        // incluye las llamadas a SAM, que `Timed` también anota aparte
        prof.lock().unwrap().add("measure+sam", t_m.elapsed().as_secs_f64());
        s.log = Some(is_log);
        samples.push(s);
        progress((sm.t / duration).min(1.0));
        last_end = Instant::now();
        Ok(true)
    })?;
    // tras la última muestra: ffmpeg termina el proxy (cola del codificador y `+faststart`)
    prof.lock().unwrap().add("decode-tail", last_end.elapsed().as_secs_f64());
    stats.gpu = dec.gpu;
    stats.lap_scale = dec.lap_scale;
    stats.cut_at = dec.cut_at;
    stats.sam_fallos = state.sam_fallos;
    Ok((samples, stats))
}

/// Lo que hace la GPU con una muestra que cambió: detección, verificación CLIP de las débiles, pose si
/// hay alguien y, con `zoom`, la segunda mirada a resolución de detalle. Devuelve detecciones y poses.
fn detect_sample(frame: &Frame, detail: Option<&Frame>, opts: &AnalyseOpts, detector: &mut dyn Detector) -> Result<(Vec<Det>, Vec<Pose>)> {
    let mut dets: Vec<Det> = detector.detect(frame)?.iter().map(Det::from_detection).filter(|d| d.conf >= VERIFY_MIN).collect();
    verify_weak(frame, &mut dets, opts.min_conf, &mut *detector);
    // pose solo si hay alguien en cuadro: dice dónde se lleva la joya y si se ve una cara
    let kps: Vec<Pose> = if dets.iter().any(|d| d.label == "persona" && d.conf >= 0.4) {
        detector.pose(frame)?.into_iter().filter(|p| p.conf >= POSE_CONF).collect()
    } else {
        Vec::new()
    };
    // segunda mirada opcional a resolución de detalle, donde se esconden joyas pequeñas
    if opts.zoom {
        let det_frame = detail.unwrap_or(frame);
        let k = det_frame.width as f64 / frame.width as f64;
        for r in jewel::zoom_regions(&kps, frame.width, frame.height, 5) {
            let c = crop(det_frame, (r[0] as f64 * k) as usize, (r[1] as f64 * k) as usize, (r[2] as f64 * k) as usize, (r[3] as f64 * k) as usize);
            if c.width == 0 || c.height == 0 {
                continue;
            }
            for d in detector.detect(&c)? {
                let mut d = Det::from_detection(&d);
                if d.conf < ZOOM_CONF || !jewel::is_jewel_label(&d.label) {
                    continue;
                }
                // píxeles del recorte a resolución de detalle → píxeles del cuadro
                let b = d.bbox;
                d.bbox = [
                    (r[0] as f64 + b[0] as f64 / k) as i64,
                    (r[1] as f64 + b[1] as f64 / k) as i64,
                    (r[0] as f64 + b[2] as f64 / k) as i64,
                    (r[1] as f64 + b[3] as f64 / k) as i64,
                ];
                d.zoom = true;
                dets.push(d);
            }
        }
    }
    Ok((dets, kps))
}

/// Analiza un clip completo y escribe su caché, como `work` en Python: muestras, voz (si hay persona o
/// cara), `speaking` por muestra y la entrada de caché. Devuelve `None` si es un RAW sin proxy.
#[allow(clippy::too_many_arguments)]
pub fn analyse_clip(
    path: &Path,
    cache_dir: &Path,
    proxy_dirs: &[PathBuf],
    detector: &mut dyn Detector,
    opts: &AnalyseOpts,
    no_proxy: bool,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(f64),
) -> Result<Option<(ClipEntry, AnalyseStats)>> {
    let p = prepare_clip(path, cache_dir, proxy_dirs, opts, no_proxy, false)?;
    analyse_prepared(p, cache_dir, detector, opts, cancel, progress, &mut || {})
}

/// Un clip abierto (ffprobe hecho) y, si se pidió, con su ffmpeg ya decodificando: se prepara mientras
/// se analiza el anterior para que el arranque de ffmpeg (CUDA, NVDEC, primer GOP) no espere a nadie.
pub struct PreparedClip {
    pub path: PathBuf,
    /// `None` = RAW sin proxy.
    info: Option<ClipInfo>,
    /// Proxy de vista previa del clip (el que queda en la caché).
    proxy: Option<PathBuf>,
    /// El proxy a escribir en esta pasada (`None` si ya hay uno al día o con `--no-proxy`).
    escribir: Option<PathBuf>,
    dec: Option<sampler::Prepared>,
    probe_secs: f64,
}

/// Abre un clip para [`analyse_prepared`]; con `start`, lanza ya su ffmpeg (ver [`PreparedClip`]).
pub fn prepare_clip(path: &Path, cache_dir: &Path, proxy_dirs: &[PathBuf], opts: &AnalyseOpts, no_proxy: bool, start: bool) -> Result<PreparedClip> {
    let t0 = std::time::Instant::now();
    let info = sampler::open_clip(path, proxy_dirs)?;
    let probe_secs = t0.elapsed().as_secs_f64();
    let cfile = project::cache_file(cache_dir, path);
    let stem = cfile.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let proxy = if no_proxy { None } else { Some(cache_dir.join("proxies").join(format!("{stem}.mp4"))) };
    // Un proxy ya hecho con los ajustes de ahora no se repite (reanalizar por color o detector): así el
    // análisis no escribe proxy y puede decodificar solo los cuadros I/P (`sampler::saltar_b`). Los de
    // antes (con bloques) sí se rehacen.
    let escribir = proxy.clone().filter(|p| !sampler::proxy_vigente(p));
    // si el arranque adelantado falla, el análisis lo vuelve a intentar a su hora (y explica el error)
    let dec = match (&info, start) {
        (Some(i), true) => sampler::prepare(i, opts.per_second, opts.zoom, escribir.as_deref()).ok(),
        _ => None,
    };
    Ok(PreparedClip { path: path.to_path_buf(), info, proxy, escribir, dec, probe_secs })
}

/// [`analyse_clip`] de un clip ya preparado. `on_eof` se llama cuando su ffmpeg terminó de entregar
/// cuadros (buen momento para preparar el siguiente).
#[allow(clippy::too_many_arguments)]
pub fn analyse_prepared(
    prepared: PreparedClip,
    cache_dir: &Path,
    detector: &mut dyn Detector,
    opts: &AnalyseOpts,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(f64),
    on_eof: &mut dyn FnMut(),
) -> Result<Option<(ClipEntry, AnalyseStats)>> {
    use std::time::Instant;
    let prof = std::sync::Mutex::new(Profile::default());
    let mut timed = Timed { inner: detector, prof: &prof };
    let detector: &mut dyn Detector = &mut timed;
    let t0 = Instant::now();
    let PreparedClip { path, info, proxy, escribir, dec: ready, probe_secs } = prepared;
    let path = path.as_path();
    let Some(info) = info else { return Ok(None) };
    prof.lock().unwrap().add("probe", probe_secs);
    let cfile = project::cache_file(cache_dir, path);
    let mut o = opts.clone();
    o.proxy = escribir;
    // el audio se extrae en paralelo con el video (~0.6 s de un 4K de Sony que si no van al final); solo
    // se usa si aparece alguien. El resultado es el mismo que extraerlo después.
    let audio_path = path.to_path_buf();
    let (audio_tx, audio_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = audio_tx.send(sampler::audio_16k(&audio_path));
    });
    let t_a = Instant::now();
    let (mut samples, stats) = analyse_profiled(&info, detector, &o, cancel, progress, &prof, ready, on_eof)?;
    prof.lock().unwrap().add("sampling-total", t_a.elapsed().as_secs_f64());
    if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
        anyhow::bail!("análisis cancelado");
    }
    let t_v = Instant::now();
    // como mucho 2 minutos más: un audio que no se deja leer no debe colgar el lote (queda sin voz)
    let audio = audio_rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap_or_default();
    let speech = if samples.iter().any(|s| s.person || s.face) { crate::audio::speech_intervals_from(&audio, detector, 0.4) } else { Vec::new() };
    drop(audio);
    prof.lock().unwrap().add("vad-total", t_v.elapsed().as_secs_f64());
    for s in samples.iter_mut() {
        s.speaking = Some(s.face && crate::audio::is_speaking(s.t, &speech));
    }
    let mut extra = Map::new();
    if crate::edicion::es_bodas() {
        // la voz (aunque nadie hable a cámara) es una pista de votos y discursos; con ella, el momento estable
        for s in samples.iter_mut() {
            s.extra.insert("voice".into(), Value::Bool(crate::audio::is_speaking(s.t, &speech)));
        }
        crate::bodas::stabilize_moments(&mut samples, opts.per_second);
        extra.insert("edition".into(), Value::String("bodas".into()));
    } else {
        // joyas: con qué detector se analizó (si luego se elige el otro, la caché cuenta como vieja)
        extra.insert(crate::detector::CLAVE_CACHE.into(), Value::String(detector.kind().as_str().into()));
        // el propio en lugar del pedido (modo ligero): la caché vale también para el pedido
        if let Some(k) = detector.en_lugar_de() {
            extra.insert(crate::detector::CLAVE_EN_LUGAR.into(), Value::String(k.as_str().into()));
        }
    }
    // SAM falló en alguna caja: se midió sobre la caja entera. Que quede en la caché (sin cambiar nada
    // cuando todo va bien) para verlo y poder reanalizar
    if stats.sam_fallos > 0 {
        extra.insert(CLAVE_SAM.into(), Value::Bool(false));
        eprintln!("aviso: {}: SAM falló en {} caja(s); se midieron sobre la caja", path.display(), stats.sam_fallos);
    }
    // archivo cortado: la caché lo recuerda con la huella del archivo (ver `cortado_vigente`)
    if let Some(t) = stats.cut_at {
        extra.insert(CLAVE_CORTADO.into(), marca_cortado(path, t));
    }
    let color = opts.color.clone().filter(|c| !c.is_null()).unwrap_or_else(project::auto_color);
    let entry = ClipEntry {
        path: path.to_string_lossy().to_string(),
        decode_path: info.decode_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        start_tc: Some(info.start_tc.clone()),
        fps: [info.fps.num, info.fps.den],
        width: info.width,
        height: info.height,
        duration: info.duration,
        per_second: opts.per_second,
        speech,
        samples,
        proxy: proxy.filter(|p| p.exists()).map(|p| p.to_string_lossy().to_string()),
        decode: if stats.gpu { "gpu".into() } else { "cpu".into() },
        lap_scale: stats.lap_scale,
        reused: stats.reused as i64,
        color: Some(color),
        extra,
    };
    let t_w = Instant::now();
    project::write_entry(&cfile, &entry)?;
    prof.lock().unwrap().add("json", t_w.elapsed().as_secs_f64());
    if Profile::enabled() {
        prof.lock().unwrap().add("clip-total", t0.elapsed().as_secs_f64());
        eprintln!("{}", prof.lock().unwrap().report(&path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
    }
    Ok(Some((entry, stats)))
}
