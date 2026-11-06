//! # camcuts-infer
//!
//! Los modelos de CAMCUTS en Rust con ONNX Runtime (DirectML en la GPU, CPU de respaldo). Implementa el
//! trait [`camcuts_core::Detector`] con el mismo pre- y posprocesado que la app de Python
//! (`camcuts_next/camcuts/jewel.py`, `classify.py`, `audio.py`), para que el núcleo dé los mismos selects.
//!
//! Módulos:
//! - [`runtime`]: carga de `runtime/onnxruntime.dll` (+ `DirectML.dll`) y creación de sesiones.
//! - [`imgops`]: redimensionados y letterbox idénticos a OpenCV / PIL / Ultralytics.
//! - [`yolo`]: YOLO-World y yolo11n-pose (letterbox, NMS, reescalado).
//! - [`sam`]: MobileSAM (codificador + decodificador de caja, máscara al tamaño del cuadro).
//! - [`clip`]: verificador CLIP de detecciones débiles.
//! - [`vad`]: Silero VAD.
//! - [`moments`]: edición Bodas, momento de la boda del cuadro entero (CLIP zero-shot).
//! - [`trained`]: detector propio (YOLO11 entrenado con material de la empresa), alternativa a
//!   YOLO-World elegible con `InferConfig::detector` (`--detector propio|world`, `CAMCUTS_DETECTOR`).
//!
//! Los modelos se abren al primer uso (así un análisis sin voz no carga Silero, etc.). Los archivos y
//! su preparación exacta están descritos en `models/README.md`.

pub mod clip;
pub mod imgops;
pub mod moments;
pub mod runtime;
pub mod sam;
pub mod trained;
pub mod vad;
pub mod yolo;

pub use trained::DetectorKind;

use std::path::PathBuf;

use anyhow::{bail, Result};
use camcuts_core::{Detection, Detector, Frame, Pose};

pub use runtime::Device;

/// Prompts de YOLO-World en el orden en que se exportaron (= índice de clase) y su etiqueta en español,
/// copiados de `JEWEL_PROMPTS` + `PEOPLE_PROMPTS` de `jewel.py`.
pub const PROMPTS: [(&str, &str); 10] = [
    ("ring", "anillo"),
    ("necklace", "collar"),
    ("chain necklace", "cadena"),
    ("earring", "arete"),
    ("stud earring", "topo"),
    ("bracelet", "pulsera"),
    ("pendant", "dije"),
    ("jewelry", "joya"),
    ("person", "persona"),
    ("face", "cara"),
];

/// Confianza mínima con la que se devuelven detecciones: `VERIFY_MIN` de `__main__.py`
/// (con el verificador CLIP activo, Python detecta a 0.02 y CLIP decide sobre las débiles).
pub const DETECT_CONF: f32 = 0.02;
/// Confianza de la pose (`PoseFinder.keypoints`).
pub const POSE_CONF: f32 = 0.4;

/// Qué pesos de YOLO-World usar (Python elige según la VRAM libre, ver `hardware.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldSize {
    /// yolov8x-worldv2: perfil normal (GPU con ≥ 6 GB y ≥ 3 GB libres).
    X,
    /// yolov8m-worldv2: perfil ligero (poca VRAM libre, p. ej. con DaVinci abierto).
    M,
}

/// Configuración de los modelos.
#[derive(Debug, Clone)]
pub struct InferConfig {
    /// Carpeta con los .onnx y `clip_text.json` (por defecto `models/` del workspace).
    pub models_dir: PathBuf,
    /// Carpeta con `onnxruntime.dll` y `DirectML.dll` (por defecto `runtime/` del workspace).
    pub runtime_dir: PathBuf,
    pub device: Device,
    pub world: WorldSize,
    /// Versión FP16 de cada modelo (entradas y salidas siguen en f32). Los valores por defecto son los
    /// que pasan las tolerancias de `fixtures/README.md` frente a Python.
    pub fp16_world: bool,
    pub fp16_sam: bool,
    pub fp16_pose: bool,
    pub fp16_clip: bool,
    /// Confianza mínima de `detect` (por defecto [`DETECT_CONF`]).
    pub detect_conf: f32,
    /// Cuadros por lote de YOLO-World en `detect_batch` (1 = cuadro a cuadro, lo más rápido con DirectML).
    pub batch_world: usize,
    /// Detector de joyas/personas/caras: el propio entrenado (`models/jewel_det.onnx`, ver [`trained`]) o
    /// YOLO-World. Por defecto lo que diga `CAMCUTS_DETECTOR` y, sin ella, el propio
    /// (`DetectorKind::default`). Las pruebas y medidas de YOLO-World lo fijan a `World` a mano.
    pub detector: DetectorKind,
    /// Versión FP16 del detector propio.
    pub fp16_own: bool,
    /// Edición de joyas: si se pide YOLO-World y el grande no cabe en la GPU (modo ligero, o no se pudo
    /// abrir), analizar con el propio en vez de con el mediano. El mediano confunde las joyas de la marca:
    /// en 33 clips de anillos ("Videos Yo Manos Detalle", 2026-11-05) casi todo salía "pulsera" y aprobaba
    /// 1318 s, frente a 2083 s del grande y 2134 s del propio. Por defecto sí; el respaldo automático del
    /// propio lo apaga (ese pase tiene que ser con YOLO-World).
    pub propio_si_no_cabe: bool,
    /// Sesiones de forma fija que guarda cada modelo (ver `runtime::ShapedSession`). 1 por defecto: una
    /// sola copia del modelo en la VRAM. [`fabrica`] pone [`MAX_FIJAS_COMPARTIDO`] al detector propio
    /// compartido, que recibe cuadros de varios clips de formas distintas a la vez.
    pub max_fijas: usize,
}

/// Sesiones de forma fija de cada modelo en un detector compartido ([`Compartido`]): dos clips en paralelo
/// de forma distinta (apaisado y vertical, 16:9 y DCI) recompilaban la sesión única en casi cada llamada.
pub const MAX_FIJAS_COMPARTIDO: usize = 2;

impl Default for InferConfig {
    fn default() -> Self {
        Self {
            models_dir: runtime::default_models_dir(),
            runtime_dir: runtime::default_runtime_dir(),
            // `CAMCUTS_DISPOSITIVO=cpu`: todo en la CPU (pruebas con la GPU ocupada, equipos sin GPU)
            device: if std::env::var("CAMCUTS_DISPOSITIVO").map(|v| v.eq_ignore_ascii_case("cpu")).unwrap_or(false) { Device::Cpu } else { Device::DirectMl },
            world: WorldSize::X,
            // YOLO-World en FP32: en FP16 la confianza se movía hasta 0.025 (cruza los umbrales 0.02 y 0.15);
            // en FP32 se mueve ≤ 0.002 y las cajas coinciden (IoU ≥ 0.99). Ver fixtures/README.md.
            // `CAMCUTS_WORLD_FP16=1`: el FP16 (pruebas de velocidad y calidad de punta a punta)
            fp16_world: std::env::var("CAMCUTS_WORLD_FP16").map(|v| v == "1").unwrap_or(false),
            fp16_sam: true,
            fp16_pose: true,
            fp16_clip: true,
            detect_conf: DETECT_CONF,
            batch_world: 1,
            detector: DetectorKind::from_env(),
            fp16_own: true,
            propio_si_no_cabe: true,
            max_fijas: 1,
        }
    }
}

impl InferConfig {
    /// ¿Se puede analizar con el propio en lugar de YOLO-World cuando el grande no cabe? Solo en joyas,
    /// si se pidió YOLO-World, la opción está puesta y el modelo propio está instalado.
    pub fn puede_usar_propio(&self) -> bool {
        self.detector == DetectorKind::World && self.propio_si_no_cabe && !camcuts_core::edicion::es_bodas() && trained::disponible(&self.models_dir)
    }

    /// Ruta de un modelo, en su versión FP16 si se pide.
    fn model(&self, stem: &str, fp16: bool) -> PathBuf {
        self.models_dir.join(if fp16 { format!("{stem}_fp16.onnx") } else { format!("{stem}.onnx") })
    }
}

/// Deja listo el motor de la GPU antes de analizar (lo llaman `camcuts.exe --analizar`, `camcuts-bodas.exe
/// --analizar` y `examples/analizar.rs`):
///
/// 1. `motor` (`--motor`) pasa a `CAMCUTS_MOTOR`;
/// 2. abre ONNX Runtime con su motor ([`runtime::engine`]: CUDA si `runtime_cuda` está y la GPU responde);
/// 3. con CUDA, pasa un cuadro negro por el modelo de pose para comprobar que la GPU calcula de verdad
///    (DLL que cargan pero sin núcleos para la GPU, driver viejo…).
///
/// Si CUDA se eligió solo (automático) y falla, vuelve a lanzar este mismo programa, con los mismos
/// argumentos y DirectML, y devuelve su código de salida (`Some`): ONNX Runtime no se puede cambiar dentro
/// de un proceso y así el usuario no ve ningún error. `None` = seguir en este proceso.
pub fn motor_listo(motor: Option<&str>, cfg: &InferConfig) -> Option<i32> {
    // en el proceso relanzado manda DirectML, diga lo que diga `--motor`
    let respaldo = std::env::var_os("CAMCUTS_RESPALDO_DIRECTML").is_some();
    if let (Some(m), false) = (motor, respaldo) {
        std::env::set_var("CAMCUTS_MOTOR", m);
    }
    let auto = runtime::requested_engine().is_none();
    let fallo = match runtime::environment(&cfg.runtime_dir) {
        Err(e) => Some(format!("{e:#}")),
        Ok(env) if runtime::engine() == runtime::Engine::Cuda => prueba_cuda(&env, cfg).err().map(|e| format!("{e:#}")),
        Ok(_) => None,
    };
    match fallo {
        None => {
            println!("motor de la GPU: {}", runtime::engine_label());
            None
        }
        Some(e) if auto && runtime::engine() == runtime::Engine::Cuda => {
            eprintln!("aviso: CUDA no responde ({e}); se analiza con DirectML");
            let mut c = std::process::Command::new(std::env::current_exe().ok()?);
            c.args(std::env::args_os().skip(1)).env("CAMCUTS_MOTOR", "directml").env("CAMCUTS_RESPALDO_DIRECTML", "1");
            camcuts_core::sampler::no_window(&mut c);
            Some(c.status().ok().and_then(|s| s.code()).unwrap_or(1))
        }
        // forzado o DirectML: el error sale (explicado) al abrir el primer modelo, como siempre
        Some(_) => None,
    }
}

/// Una inferencia de pose (5 MB, todo convoluciones de cuDNN) con el motor CUDA.
fn prueba_cuda(env: &ort::environment::Environment, cfg: &InferConfig) -> Result<()> {
    let mut s = runtime::session(env, &cfg.model("yolo11n_pose", true), Device::DirectMl, None)?;
    let x = runtime::tensor_f32(vec![1, 3, 384, 640], vec![0.0; 3 * 384 * 640])?;
    runtime::run_f32(&mut s, vec![("images", x)])?;
    Ok(())
}

/// Implementación de [`Detector`] con ONNX Runtime. Los modelos se cargan al primer uso.
pub struct OnnxDetector {
    cfg: InferConfig,
    env: ort::environment::Environment,
    world: Option<yolo::Yolo>,
    /// detector propio (solo con `cfg.detector == Propio`; entonces YOLO-World no se carga)
    own: Option<trained::TrainedDetector>,
    pose: Option<yolo::Yolo>,
    sam: Option<sam::MobileSam>,
    clip: Option<clip::ClipVerifier>,
    vad: Option<vad::SileroVad>,
    /// Edición Bodas: textos de los momentos (`clip_moments.json`).
    momentos: Option<moments::Moments>,
    /// Se pidió YOLO-World y analiza el propio porque el grande no cabe en la GPU (ver
    /// [`InferConfig::propio_si_no_cabe`]); va a la caché de cada clip.
    en_lugar_de: Option<DetectorKind>,
    /// YOLO-World x ya dio al menos un resultado en una llamada real (no en `prepare`). Si después falla
    /// (sesión dinámica de `--zoom`, VRAM a mitad de clip), pasar al propio y marcar la caché "propio en
    /// lugar de world" mezclaría dos detectores en un clip que diría venir del propio entero.
    world_dio_resultado: bool,
    /// Error con el que no se pudo abrir SAM: se recuerda para no reintentar abrirlo en cada caja (hasta
    /// `SAM_MAX` veces por muestra, bloqueando a los demás hilos de un detector compartido).
    sam_fallo: Option<String>,
    /// Igual para CLIP.
    clip_fallo: Option<String>,
    /// Detector de prueba de `prepare`: sus fallos no se recuerdan ni se avisan (el de verdad lo vuelve
    /// a intentar en su primer uso real, como siempre).
    de_prueba: bool,
}

/// Modelos sin los que el análisis sigue (peor): SAM (se mide sobre la caja) y CLIP (las detecciones
/// débiles no se confirman).
#[derive(Clone, Copy)]
enum Opcional {
    Sam,
    Clip,
}

/// ¿Pasa el detector al propio (con la marca `en_lugar_de`) cuando YOLO-World x falla? Solo si se puede
/// ([`InferConfig::puede_usar_propio`]) y x aún no dio ningún resultado: así todo el clip sale del propio.
fn sustituir_por_propio(puede: bool, x_dio_resultado: bool) -> bool {
    puede && !x_dio_resultado
}

/// Primera línea de un error, recortada, para un aviso de una línea.
fn primera_linea(e: &str) -> String {
    // 300: los errores de ONNX Runtime empiezan por la ruta completa del modelo, que ya ocupa ~150
    e.lines().next().unwrap_or_default().chars().take(300).collect()
}

/// Comprueba que un cuadro tiene el tamaño que dice.
fn check(frame: &Frame) -> Result<(usize, usize)> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if frame.data.len() != w * h * 3 {
        bail!("cuadro de {}x{} con {} bytes (se esperaban {})", w, h, frame.data.len(), w * h * 3);
    }
    Ok((h, w))
}

impl OnnxDetector {
    /// Carga ONNX Runtime (no abre aún ningún modelo).
    pub fn new(mut cfg: InferConfig) -> Result<Self> {
        // modo ligero decidido por la línea de comandos (poca VRAM libre o `--gpu ligero`): el detector
        // mediano, como `hardware.detect` de Python. Sin modo publicado (pruebas, la ventana) manda `cfg`.
        // En joyas, si se pidió YOLO-World, el propio en su lugar (el mediano confunde las joyas).
        let mut en_lugar_de = None;
        if camcuts_core::hardware::gpu_mode() == Some(camcuts_core::hardware::GpuMode::Ligero) {
            cfg.world = WorldSize::M;
            if cfg.puede_usar_propio() {
                cfg.detector = DetectorKind::Propio;
                en_lugar_de = Some(DetectorKind::World);
            }
        }
        let env = runtime::environment(&cfg.runtime_dir)?;
        Ok(Self::vacio(cfg, env, en_lugar_de))
    }

    /// Un detector sin ningún modelo abierto.
    fn vacio(cfg: InferConfig, env: ort::environment::Environment, en_lugar_de: Option<DetectorKind>) -> Self {
        Self {
            cfg,
            env,
            world: None,
            own: None,
            pose: None,
            sam: None,
            clip: None,
            vad: None,
            momentos: None,
            en_lugar_de,
            world_dio_resultado: false,
            sam_fallo: None,
            clip_fallo: None,
            de_prueba: false,
        }
    }

    /// Edición Bodas: un modelo propio de la edición, junto a los demás (`models/`, como en la app
    /// instalada) o en `models/bodas/` (el workspace).
    fn modelo_bodas(&self, archivo: &str) -> PathBuf {
        let plano = self.cfg.models_dir.join(archivo);
        if plano.exists() { plano } else { self.cfg.models_dir.join("bodas").join(archivo) }
    }

    /// Edición Bodas: los textos de los momentos, abiertos al primer uso.
    fn momentos(&mut self) -> Result<&moments::Moments> {
        if self.momentos.is_none() {
            let p = self.modelo_bodas("clip_moments.json");
            self.momentos = Some(moments::Moments::open(&p)?);
        }
        Ok(self.momentos.as_ref().unwrap())
    }

    /// Etiqueta en español de una clase del detector YOLO-World de la edición en curso.
    fn etiqueta_clase(&self, class: usize) -> String {
        if camcuts_core::edicion::es_bodas() {
            camcuts_core::bodas::SUJETOS.get(class).copied().unwrap_or("persona").to_string()
        } else {
            PROMPTS[class].1.to_string()
        }
    }

    /// El detector propio, abierto al primer uso.
    fn own(&mut self) -> Result<&mut trained::TrainedDetector> {
        if self.own.is_none() {
            let mut d = trained::TrainedDetector::open(&self.env, &self.cfg.models_dir, self.cfg.device, self.cfg.fp16_own)?;
            d.set_max_fijas(self.cfg.max_fijas);
            self.own = Some(d);
        }
        Ok(self.own.as_mut().unwrap())
    }

    fn world(&mut self) -> Result<&mut yolo::Yolo> {
        if self.world.is_none() {
            if camcuts_core::edicion::es_bodas() {
                // edición Bodas: YOLO-World con los 12 sujetos de boda incrustados (solo FP32)
                let stem = match self.cfg.world {
                    WorldSize::X => "yolo_world_bodas_x.onnx",
                    WorldSize::M => "yolo_world_bodas_m.onnx",
                };
                let p = self.modelo_bodas(stem);
                let mut y = yolo::Yolo::open(&self.env, &p, self.cfg.device, 960, camcuts_core::bodas::SUJETOS.len())?;
                y.set_max_fijas(self.cfg.max_fijas);
                self.world = Some(y);
                return Ok(self.world.as_mut().unwrap());
            }
            let stem = match self.cfg.world {
                WorldSize::X => "yolo_world_x",
                WorldSize::M => "yolo_world_m",
            };
            let p = self.cfg.model(stem, self.cfg.fp16_world);
            let mut y = yolo::Yolo::open(&self.env, &p, self.cfg.device, 960, PROMPTS.len())?;
            y.set_max_fijas(self.cfg.max_fijas);
            self.world = Some(y);
        }
        Ok(self.world.as_mut().unwrap())
    }

    fn pose_model(&mut self) -> Result<&mut yolo::Yolo> {
        if self.pose.is_none() {
            let p = self.cfg.model("yolo11n_pose", self.cfg.fp16_pose);
            let mut y = yolo::Yolo::open(&self.env, &p, self.cfg.device, 640, 1)?;
            y.set_max_fijas(self.cfg.max_fijas);
            self.pose = Some(y);
        }
        Ok(self.pose.as_mut().unwrap())
    }

    /// Recuerda (y avisa una vez) que un modelo opcional no se pudo abrir, salvo en los detectores de
    /// prueba de `prepare`. Devuelve el error tal cual para quien lo pidió.
    fn recordar_fallo(&mut self, cual: Opcional, err: anyhow::Error) -> anyhow::Error {
        if !self.de_prueba {
            let msg = format!("{err:#}");
            let (nombre, consecuencia, hueco) = match cual {
                Opcional::Sam => ("SAM", "se mide sobre la caja", &mut self.sam_fallo),
                Opcional::Clip => ("CLIP", "las detecciones débiles no se verifican", &mut self.clip_fallo),
            };
            eprintln!("aviso: {nombre} no disponible ({}): {consecuencia}", primera_linea(&msg));
            *hueco = Some(msg);
        }
        err
    }

    fn sam(&mut self) -> Result<&mut sam::MobileSam> {
        if self.sam.is_none() {
            // ya falló al abrirse: no se reintenta en cada caja
            if let Some(e) = &self.sam_fallo {
                bail!("SAM no disponible: {e}");
            }
            let e = self.cfg.model("mobile_sam_encoder", self.cfg.fp16_sam);
            let d = self.cfg.model("mobile_sam_decoder", self.cfg.fp16_sam);
            match sam::MobileSam::open(&self.env, &e, &d, self.cfg.device) {
                Ok(mut s) => {
                    s.set_max_fijas(self.cfg.max_fijas);
                    self.sam = Some(s);
                }
                Err(err) => return Err(self.recordar_fallo(Opcional::Sam, err)),
            }
        }
        Ok(self.sam.as_mut().unwrap())
    }

    fn clip(&mut self) -> Result<&mut clip::ClipVerifier> {
        if self.clip.is_none() {
            if let Some(e) = &self.clip_fallo {
                bail!("CLIP no disponible: {e}");
            }
            let m = self.cfg.model("clip_visual", self.cfg.fp16_clip);
            let t = self.cfg.models_dir.join("clip_text.json");
            match clip::ClipVerifier::open(&self.env, &m, &t, self.cfg.device) {
                Ok(mut c) => {
                    c.set_max_fijas(self.cfg.max_fijas);
                    self.clip = Some(c);
                }
                Err(err) => return Err(self.recordar_fallo(Opcional::Clip, err)),
            }
        }
        Ok(self.clip.as_mut().unwrap())
    }

    fn vad(&mut self) -> Result<&mut vad::SileroVad> {
        if self.vad.is_none() {
            let m = self.cfg.models_dir.join("silero_vad.onnx");
            self.vad = Some(vad::SileroVad::open(&self.env, &m)?);
        }
        Ok(self.vad.as_mut().unwrap())
    }

    /// Detecta en varios cuadros, como `JewelDetector.detect` con una lista. Las cajas salen truncadas a
    /// enteros (Python hace `int(x)`), ordenadas por confianza.
    ///
    /// Con DirectML, los lotes de más de un cuadro resultaron MÁS lentos por cuadro (36 ms frente a 24 ms
    /// con yolov8x en FP16), así que por defecto se infiere cuadro a cuadro; `batch_world` > 1 agrupa
    /// cuadros del mismo tamaño en un solo lote (útil en CPU o en otras GPU).
    pub fn detect_batch(&mut self, frames: &[&Frame]) -> Result<Vec<Vec<Detection>>> {
        let conf = self.cfg.detect_conf;
        let max_batch = self.cfg.batch_world.max(1);
        let mut out: Vec<Vec<Detection>> = vec![vec![]; frames.len()];
        // agrupa por tamaño: un lote de ONNX necesita la misma forma
        let mut done = vec![false; frames.len()];
        for i in 0..frames.len() {
            if done[i] {
                continue;
            }
            let (h, w) = check(frames[i])?;
            let idx: Vec<usize> = (i..frames.len())
                .filter(|&j| !done[j] && frames[j].width as usize == w && frames[j].height as usize == h)
                .take(max_batch)
                .collect();
            let batch: Vec<(&[u8], usize, usize)> = idx.iter().map(|&j| (frames[j].data.as_slice(), h, w)).collect();
            // detector propio: ya devuelve etiquetas en español, umbrales por clase y cajas truncadas
            if self.cfg.detector == DetectorKind::Propio && !camcuts_core::edicion::es_bodas() {
                for (&j, dets) in idx.iter().zip(self.own()?.detect(&batch)?) {
                    done[j] = true;
                    out[j] = dets;
                }
                continue;
            }
            let res = match self.world().and_then(|w| w.predict(&batch, conf)) {
                Ok(r) => {
                    self.world_dio_resultado = true;
                    r
                }
                // YOLO-World x no cabe en la VRAM que dejan las otras apps (DaVinci): DirectML no puede
                // compilarlo ("El parámetro no es correcto"). Antes cada clip salía con error y el análisis
                // no daba nada; ahora sigue con el mediano, como el modo ligero, y los detectores que se
                // creen después también (`hardware::bajar_a_ligero`)
                Err(e) if self.cfg.world == WorldSize::X && self.cfg.device == Device::DirectMl => {
                    // al propio solo si x aún no dio nada en este detector: si ya dio resultados (el fallo
                    // llegó a mitad de clip, p. ej. la sesión dinámica de `--zoom` o la VRAM), la caché
                    // diría "propio en lugar de world" con un análisis mezclado. Entonces se sigue con el
                    // mediano (el mismo YOLO-World) y, desde el clip siguiente, el detector nuevo de modo
                    // ligero decide (`cli::analizar_lote` lo recrea al ver el modo ligero)
                    let propio = sustituir_por_propio(self.cfg.puede_usar_propio(), self.world_dio_resultado);
                    if camcuts_core::hardware::bajar_a_ligero() {
                        // "equipo:" para que la ventana lo muestre en la barra de estado (en rojo: "OCUPADA")
                        let motivo = primera_linea(&format!("{e:#}"));
                        let sigue = if propio { "el detector propio" } else { "el mediano" };
                        println!("equipo: GPU OCUPADA — el detector grande no cabe en la memoria libre de la GPU (¿DaVinci?); se sigue con {sigue} (modo ligero)");
                        eprintln!("aviso: no se pudo usar YOLO-World x: {motivo}");
                    }
                    self.cfg.world = WorldSize::M;
                    self.world = None;
                    if propio {
                        // x aún no dio ningún resultado en este detector: el clip entero sale del propio
                        self.cfg.detector = DetectorKind::Propio;
                        self.en_lugar_de = Some(DetectorKind::World);
                        for (&j, dets) in idx.iter().zip(self.own()?.detect(&batch)?) {
                            done[j] = true;
                            out[j] = dets;
                        }
                        continue;
                    }
                    self.world()?.predict(&batch, conf)?
                }
                Err(e) => return Err(e),
            };
            for (&j, dets) in idx.iter().zip(res) {
                done[j] = true;
                out[j] = dets
                    .into_iter()
                    .map(|d| Detection {
                        x1: d.xyxy[0].trunc(),
                        y1: d.xyxy[1].trunc(),
                        x2: d.xyxy[2].trunc(),
                        y2: d.xyxy[3].trunc(),
                        conf: d.conf,
                        label: self.etiqueta_clase(d.class),
                    })
                    .collect();
            }
        }
        Ok(out)
    }

    /// Pose en varios cuadros del mismo tamaño (un lote), como `PoseFinder.keypoints`.
    pub fn pose_batch(&mut self, frames: &[&Frame]) -> Result<Vec<Vec<Pose>>> {
        let mut out = Vec::with_capacity(frames.len());
        for f in frames {
            check(f)?;
        }
        // por simplicidad, lotes por tamaño igual que detect
        let mut groups: Vec<(usize, usize, Vec<usize>)> = vec![];
        for (i, f) in frames.iter().enumerate() {
            let (h, w) = (f.height as usize, f.width as usize);
            match groups.iter_mut().find(|g| g.0 == h && g.1 == w) {
                Some(g) => g.2.push(i),
                None => groups.push((h, w, vec![i])),
            }
        }
        let mut res: Vec<Vec<Pose>> = vec![vec![]; frames.len()];
        for (h, w, idx) in groups {
            let batch: Vec<(&[u8], usize, usize)> = idx.iter().map(|&j| (frames[j].data.as_slice(), h, w)).collect();
            let dets = self.pose_model()?.predict(&batch, POSE_CONF)?;
            for (&j, ds) in idx.iter().zip(dets) {
                res[j] = ds
                    .into_iter()
                    .map(|d| {
                        let mut kp = [[0f32; 3]; 17];
                        for (k, p) in d.extra.chunks(3).take(17).enumerate() {
                            kp[k] = [p[0], p[1], p[2]];
                        }
                        Pose { keypoints: kp, bbox: d.xyxy, conf: d.conf }
                    })
                    .collect();
            }
        }
        out.extend(res);
        Ok(out)
    }

    /// Siluetas de varias cajas de un cuadro en una sola llamada, con la semántica exacta de
    /// `JewelSegmenter.masks` (incluido el corrimiento si SAM descarta una máscara con puntuación ≤ 0.25).
    pub fn silhouettes(&mut self, frame: &Frame, boxes: &[[f32; 4]]) -> Result<Vec<Option<Vec<u8>>>> {
        let (h, w) = check(frame)?;
        self.sam()?.masks(&frame.data, h, w, boxes)
    }

    /// Probabilidad de joya de varias cajas de detección de un cuadro, en un lote (como `verify_weak`,
    /// que manda hasta 4 recortes juntos). Cada caja se agranda un 15 % antes de recortar.
    pub fn verify_batch(&mut self, frame: &Frame, boxes: &[[f32; 4]]) -> Result<Vec<f32>> {
        let rects: Vec<[usize; 4]> = {
            let (h, w) = check(frame)?;
            boxes.iter().map(|b| clip::verify_crop_rect(*b, w, h)).collect()
        };
        self.verify_rects(frame, &rects)
    }

    /// Probabilidad de joya de recortes ya calculados [x0, y0, x1, y1] (sin relleno extra).
    /// Un recorte vacío da 0.
    pub fn verify_rects(&mut self, frame: &Frame, rects: &[[usize; 4]]) -> Result<Vec<f32>> {
        let (_, w) = check(frame)?;
        let crops: Vec<(Vec<u8>, usize, usize)> = rects
            .iter()
            .filter(|r| r[2] > r[0] && r[3] > r[1])
            .map(|r| (imgops::crop(&frame.data, w, r[0], r[1], r[2], r[3]), r[3] - r[1], r[2] - r[0]))
            .collect();
        let probs = self.clip()?.probabilities(&crops)?;
        let mut it = probs.into_iter();
        Ok(rects.iter().map(|r| if r[2] > r[0] && r[3] > r[1] { it.next().unwrap_or(0.0) } else { 0.0 }).collect())
    }
}

impl Detector for OnnxDetector {
    /// YOLO-World a 960 con los prompts de Python, confianza ≥ 0.02 (estricta), etiquetas en español.
    fn detect(&mut self, frame: &Frame) -> Result<Vec<Detection>> {
        Ok(self.detect_batch(&[frame])?.pop().unwrap_or_default())
    }

    /// Silueta de MobileSAM (0/255, tamaño del cuadro). Si SAM la descarta (puntuación ≤ 0.25) devuelve
    /// una máscara vacía (todo 0), que `measure` trata igual que `None` (menos de 30 píxeles).
    fn silhouette(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<Vec<u8>> {
        let (h, w) = check(frame)?;
        Ok(self.silhouettes(frame, &[bbox])?.pop().flatten().unwrap_or_else(|| vec![0; h * w]))
    }

    /// yolo11n-pose a 640, confianza 0.4.
    fn pose(&mut self, frame: &Frame) -> Result<Vec<Pose>> {
        Ok(self.pose_batch(&[frame])?.pop().unwrap_or_default())
    }

    /// `bbox` es la caja de la DETECCIÓN: se agranda un 15 % y se recorta como en `verify_weak`.
    fn verify(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<f32> {
        Ok(self.verify_batch(frame, &[bbox])?.pop().unwrap_or(0.0))
    }

    /// Probabilidad por ventana de 512 muestras (32 ms) desde el principio del audio.
    fn speech(&mut self, audio_16k: &[f32]) -> Result<Vec<f32>> {
        self.vad()?.probabilities(audio_16k)
    }

    /// El detector de joyas en uso: el propio solo en la edición de joyas (en Bodas `detect_batch` usa
    /// siempre el YOLO-World de boda). El núcleo lo guarda en la caché de cada clip.
    fn kind(&self) -> DetectorKind {
        if self.cfg.detector == DetectorKind::Propio && !camcuts_core::edicion::es_bodas() {
            DetectorKind::Propio
        } else {
            DetectorKind::World
        }
    }

    /// YOLO-World, si el propio está analizando en su lugar porque el grande no cabe en la GPU.
    fn en_lugar_de(&self) -> Option<DetectorKind> {
        self.en_lugar_de.filter(|_| self.kind() == DetectorKind::Propio)
    }

    /// Abre los modelos de la GPU a la vez, cada uno en su hilo, y pasa por cada uno un cuadro negro del
    /// tamaño dado: así se compilan sus grafos de DirectML (unos segundos de CPU por modelo) en paralelo y
    /// mientras arranca el ffmpeg del primer clip, en vez de uno tras otro en las primeras muestras.
    /// Silero VAD (CPU, rápido) se sigue abriendo al primer uso. Un modelo que falle aquí queda sin abrir y
    /// se vuelve a intentar (y se informa) en su primer uso real.
    fn prepare(&mut self, width: u32, height: u32) -> Result<()> {
        // ya preparado (un detector compartido lo prepara el primer hilo): abrir otra vez los modelos en
        // detectores de prueba solo ocuparía VRAM de más un rato
        if (self.own.is_some() || self.world.is_some()) && self.pose.is_some() && self.sam.is_some() && self.clip.is_some() {
            return Ok(());
        }
        let (w, h) = (width.max(64), height.max(64));
        let frame = Frame { width: w, height: h, data: vec![0; (w * h * 3) as usize] };
        let caja = [0.0, 0.0, 64.0, 64.0];
        // detectores de prueba: sus fallos no se recuerdan (el de verdad reintenta en su primer uso) y su
        // cuadro negro no cuenta como resultado de YOLO-World x
        let vacio = |s: &Self| Self { de_prueba: true, ..Self::vacio(s.cfg.clone(), s.env.clone(), s.en_lugar_de) };
        let (mut det, mut pose, mut sil, mut ver) = (vacio(self), vacio(self), vacio(self), vacio(self));
        std::thread::scope(|s| {
            s.spawn(|| det.detect(&frame).map(drop));
            s.spawn(|| pose.pose(&frame).map(drop));
            if camcuts_core::edicion::es_bodas() {
                // en Bodas CLIP también da el momento de cada muestra
                s.spawn(|| ver.moments(&frame).map(drop));
            } else {
                s.spawn(|| ver.verify(&frame, caja).map(drop));
            }
            s.spawn(|| sil.silhouette(&frame, caja).map(drop));
        });
        // si el grande no cupo y el hilo de prueba pasó al mediano, este detector también
        if det.cfg.world == WorldSize::M {
            self.cfg.world = WorldSize::M;
        }
        // y si en su lugar pasó al propio, también
        if det.cfg.detector != self.cfg.detector {
            self.cfg.detector = det.cfg.detector;
            self.en_lugar_de = det.en_lugar_de;
        }
        // lo ya abierto (si lo hubiera) manda; lo nuevo solo rellena huecos
        self.world = self.world.take().or(det.world);
        self.own = self.own.take().or(det.own);
        self.pose = self.pose.take().or(pose.pose);
        self.sam = self.sam.take().or(sil.sam);
        self.clip = self.clip.take().or(ver.clip);
        self.momentos = self.momentos.take().or(ver.momentos);
        Ok(())
    }

    /// Edición Bodas: probabilidad de cada momento de la boda (CLIP sobre el cuadro entero, rellenado a
    /// cuadrado). En la edición de joyas no hay momentos.
    fn moments(&mut self, frame: &Frame) -> Result<Vec<(String, f32)>> {
        if !camcuts_core::edicion::es_bodas() {
            return Ok(Vec::new());
        }
        let (h, w) = check(frame)?;
        self.momentos()?;
        // dos préstamos a la vez (textos y sesión de CLIP): se saca un momento del campo y se devuelve
        let textos = self.momentos.take().unwrap();
        let r = self.clip().and_then(|c| textos.classify(c, &frame.data, h, w));
        self.momentos = Some(textos);
        r
    }
}

/// Un detector compartido por varios hilos de análisis: sus modelos están UNA vez en la GPU y cada
/// llamada lo toma un momento. Con el detector propio el análisis lo limita el decodificador de video,
/// no los modelos, así que turnarse casi no cuesta; a cambio, cada clip en paralelo solo necesita la
/// memoria de su ffmpeg (~0.96 GB) en vez de ~1.7 GB, y con DaVinci abierto caben 2 clips.
pub struct Compartido(std::sync::Arc<std::sync::Mutex<OnnxDetector>>);

impl Compartido {
    fn con<R>(&self, f: impl FnOnce(&mut OnnxDetector) -> R) -> R {
        // un pánico a mitad de una llamada no inutiliza los modelos para los demás hilos
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }
}

impl Detector for Compartido {
    fn detect(&mut self, frame: &Frame) -> Result<Vec<Detection>> {
        self.con(|d| d.detect(frame))
    }
    fn silhouette(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<Vec<u8>> {
        self.con(|d| d.silhouette(frame, bbox))
    }
    fn pose(&mut self, frame: &Frame) -> Result<Vec<Pose>> {
        self.con(|d| d.pose(frame))
    }
    fn verify(&mut self, frame: &Frame, bbox: [f32; 4]) -> Result<f32> {
        self.con(|d| d.verify(frame, bbox))
    }
    fn speech(&mut self, audio_16k: &[f32]) -> Result<Vec<f32>> {
        self.con(|d| d.speech(audio_16k))
    }
    fn moments(&mut self, frame: &Frame) -> Result<Vec<(String, f32)>> {
        self.con(|d| d.moments(frame))
    }
    fn kind(&self) -> DetectorKind {
        self.con(|d| d.kind())
    }
    fn en_lugar_de(&self) -> Option<DetectorKind> {
        self.con(|d| d.en_lugar_de())
    }
    fn prepare(&mut self, width: u32, height: u32) -> Result<()> {
        self.con(|d| d.prepare(width, height))
    }
}

/// Fábrica de detectores para los hilos de análisis (`cli::DetectorFactory`). Cuando el que sale es el
/// propio, todos los hilos comparten uno ([`Compartido`]); con YOLO-World (también el de boda), uno por
/// hilo como siempre: es la GPU la que lo limita y turnarse sí costaría. Si el análisis pasa a modo ligero
/// a mitad (`hardware::bajar_a_ligero`), el siguiente detector se crea de nuevo con el modo nuevo.
/// `CAMCUTS_COMPARTIR=0` vuelve a uno por hilo (pruebas).
pub fn fabrica(cfg: InferConfig) -> impl Fn() -> Result<Box<dyn Detector>> + Send + Sync {
    let f = Fabrica::nueva(cfg);
    move || f.crear()
}

/// El estado de [`fabrica`]: la configuración y el detector compartido que está en uso.
struct Fabrica {
    cfg: InferConfig,
    compartir: bool,
    /// (¿modo ligero?, detector compartido). Se guarda sin retenerlo (`Weak`): cuando lo sueltan todos
    /// los hilos (fin del pase), sus modelos (~0.75 GB) salen de la VRAM. Antes la fábrica guardaba un
    /// `Arc` y, como vive todo `cli::run_con_respaldo`, los modelos seguían ocupando la GPU durante el
    /// respaldo con YOLO-World y falseaban la VRAM libre que este mide para decidir su modo.
    hecho: std::sync::Mutex<Option<(bool, std::sync::Weak<std::sync::Mutex<OnnxDetector>>)>>,
}

impl Fabrica {
    fn nueva(cfg: InferConfig) -> Self {
        let compartir = std::env::var("CAMCUTS_COMPARTIR").map(|v| v != "0").unwrap_or(true);
        Self { cfg, compartir, hecho: std::sync::Mutex::new(None) }
    }

    fn crear(&self) -> Result<Box<dyn Detector>> {
        use std::sync::{Arc, Mutex, PoisonError};
        let ligero = camcuts_core::hardware::gpu_mode() == Some(camcuts_core::hardware::GpuMode::Ligero);
        let mut h = self.hecho.lock().unwrap_or_else(PoisonError::into_inner);
        // el compartido sigue vivo mientras algún hilo tenga el suyo (también tras un pánico: el hilo pide
        // uno nuevo con el anterior aún en la mano)
        if let Some(d) = h.as_ref().filter(|(l, _)| *l == ligero).and_then(|(_, w)| w.upgrade()) {
            return Ok(Box::new(Compartido(d)) as Box<dyn Detector>);
        }
        let mut det = OnnxDetector::new(self.cfg.clone())?;
        if !self.compartir || det.kind() != DetectorKind::Propio {
            return Ok(Box::new(det) as Box<dyn Detector>);
        }
        // varios clips a la vez, quizá de forma distinta: una sesión fija por forma (ver `ShapedSession`)
        det.cfg.max_fijas = det.cfg.max_fijas.max(MAX_FIJAS_COMPARTIDO);
        let d = Arc::new(Mutex::new(det));
        *h = Some((ligero, Arc::downgrade(&d)));
        Ok(Box::new(Compartido(d)) as Box<dyn Detector>)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// La fábrica no retiene el detector compartido: mientras un hilo tiene el suyo, los demás reciben
    /// el mismo; cuando lo sueltan todos, se libera (y sus modelos salen de la VRAM).
    #[test]
    fn fabrica_no_retiene_el_compartido() {
        if camcuts_core::edicion::es_bodas() || std::env::var("CAMCUTS_COMPARTIR").map(|v| v == "0").unwrap_or(false) {
            return;
        }
        // `OnnxDetector::new` solo carga ONNX Runtime (sin abrir modelos ni tocar la GPU)
        let f = Fabrica::nueva(InferConfig { detector: DetectorKind::Propio, ..Default::default() });
        let a = f.crear().expect("detector");
        let debil = f.hecho.lock().unwrap().as_ref().map(|(_, w)| w.clone()).expect("compartido guardado");
        assert_eq!(debil.strong_count(), 1);
        let b = f.crear().expect("detector");
        assert_eq!(debil.strong_count(), 2, "el segundo hilo comparte el mismo");
        // el compartido guarda una sesión fija por forma para dos clips a la vez
        assert_eq!(debil.upgrade().unwrap().lock().unwrap().cfg.max_fijas, MAX_FIJAS_COMPARTIDO);
        drop(a);
        drop(b);
        assert_eq!(debil.strong_count(), 0, "soltado por todos: liberado");
        // el siguiente pide uno nuevo
        let c = f.crear().expect("detector");
        assert_eq!(c.kind(), DetectorKind::Propio);
        assert_eq!(debil.strong_count(), 0);
    }

    /// YOLO-World x que falla: al propio (con la marca) solo si aún no dio nada en ese detector.
    #[test]
    fn al_propio_solo_si_x_no_dio_nada() {
        assert!(sustituir_por_propio(true, false), "primer cuadro: el clip entero sale del propio");
        assert!(!sustituir_por_propio(true, true), "a mitad de clip: sigue con el mediano, sin marca");
        assert!(!sustituir_por_propio(false, false));
    }

    /// Un detector nuevo no tiene fallos recordados; el de prueba no los guarda.
    #[test]
    fn fallo_de_sam_se_recuerda() {
        // en la CPU: la prueba no debe tocar la GPU
        let mut d = match OnnxDetector::new(InferConfig { models_dir: std::env::temp_dir().join("camcuts_sin_modelos"), device: Device::Cpu, ..Default::default() }) {
            Ok(d) => d,
            Err(_) => return, // sin ONNX Runtime en este equipo
        };
        assert!(d.sam().is_err());
        let primero = d.sam_fallo.clone().expect("fallo recordado");
        // la segunda vez no reabre: devuelve el mismo error
        let e = d.sam().err().unwrap().to_string();
        assert!(e.contains("SAM no disponible") && e.contains(&primero), "{e}");
        let mut p = OnnxDetector { de_prueba: true, ..OnnxDetector::vacio(d.cfg.clone(), d.env.clone(), None) };
        assert!(p.clip().is_err());
        assert!(p.clip_fallo.is_none(), "el de prueba no guarda el fallo");
        // la silueta falla (el núcleo mide sobre la caja), sin pánico
        let f = Frame { width: 64, height: 64, data: vec![0; 64 * 64 * 3] };
        assert!(d.silhouette(&f, [0.0, 0.0, 32.0, 32.0]).is_err());
    }
}
