//! Carga de ONNX Runtime y creación de sesiones, con uno de dos motores de GPU por proceso:
//!
//! - **CUDA** (`runtime_cuda/`: ONNX Runtime GPU 1.24.4 + CUDA 12.9 + cuDNN 9.20, carpeta compartida por las
//!   dos ediciones). Se elige solo si sus DLL cargan y la GPU NVIDIA responde ([`engine`]).
//! - **DirectML** (`runtime/`: la compilación DirectML de ONNX Runtime), el de siempre y el de respaldo.
//!
//! - La DLL se carga en tiempo de ejecución (`ort` con `load-dynamic`), así que el crate compila con la
//!   toolchain GNU sin enlazar nada de Microsoft. Solo se puede cargar UNA por proceso: el motor se decide
//!   antes de abrir ONNX Runtime (variable `CAMCUTS_MOTOR` = `cuda` | `directml` | `auto`).
//! - Con DirectML se precarga `runtime/DirectML.dll`: Windows trae en System32 una versión antigua que ONNX
//!   Runtime no acepta, y la DLL de ORT la buscaría primero allí. Con CUDA se precargan igual sus DLL desde
//!   `runtime_cuda/`, sin tocar el PATH ni instalar nada.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

/// Dónde se ejecuta un modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// La GPU, con el motor del proceso ([`engine`]: CUDA o DirectML). El nombre es histórico.
    DirectMl,
    /// Solo CPU.
    Cpu,
}

/// Entorno global de ONNX Runtime: la DLL solo se puede cargar una vez por proceso.
static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();

/// Motor de la GPU del proceso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Cuda,
    DirectMl,
}

/// El motor elegido al abrir ONNX Runtime (antes de abrirlo: DirectML).
static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Motor de la GPU de este proceso.
pub fn engine() -> Engine {
    ENGINE.get().copied().unwrap_or(Engine::DirectMl)
}

/// Lo que pide `CAMCUTS_MOTOR` (`--motor` la fija): `cuda`, `auto` (`None`: CUDA si `runtime_cuda` está y
/// la GPU responde, si no DirectML) o, sin variable, DirectML.
///
/// DirectML es el motor por defecto porque ganó: medido el 2026-11-03 en Octubre 27 (19 clips, 2 en
/// paralelo), DirectML 164-166 s; CUDA FP32 308 s; CUDA TF32 191 s. Con CUDA solo YOLO-World x va algo más
/// rápido (54 frente a 63 ms por cuadro con TF32); SAM, CLIP y la pose, modelos de muchas operaciones
/// pequeñas, van 1.5-3 veces más lentos.
pub fn requested_engine() -> Option<Engine> {
    match std::env::var("CAMCUTS_MOTOR").unwrap_or_default().to_ascii_lowercase().as_str() {
        "cuda" => Some(Engine::Cuda),
        "auto" => None,
        _ => Some(Engine::DirectMl),
    }
}

/// DLL de CUDA y cuDNN que carga ONNX Runtime GPU, en orden de dependencias (las de cuDNN las abre
/// `cudnn64_9.dll` por su nombre al necesitarlas: ya cargadas, Windows usa estas). `cudnn_adv64_9.dll`
/// (RNN y atención de cuDNN) no hace falta para estos modelos y no se instala.
const CUDA_DLLS: &[&str] = &[
    "cudart64_12.dll",
    "cublasLt64_12.dll",
    "cublas64_12.dll",
    "cufft64_11.dll",
    "cudnn_graph64_9.dll",
    "cudnn_engines_precompiled64_9.dll",
    "cudnn_engines_runtime_compiled64_9.dll",
    "cudnn_heuristic64_9.dll",
    "cudnn_ops64_9.dll",
    "cudnn_cnn64_9.dll",
    "cudnn64_9.dll",
];

/// Carpeta `runtime_cuda` (compartida): `CAMCUTS_RUNTIME_CUDA`, junto al ejecutable, en la carpeta de
/// encima (`dist\runtime_cuda` sirve a `dist\CAMCUTS\` y a `dist\CAMCUTS Bodas\`) o la de `dist\` del
/// workspace (pruebas y ejemplos).
pub fn cuda_runtime_dir() -> Option<PathBuf> {
    let ok = |p: PathBuf| p.join("onnxruntime.dll").exists().then_some(p);
    if let Some(p) = std::env::var_os("CAMCUTS_RUNTIME_CUDA").map(PathBuf::from).and_then(ok) {
        return Some(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        for d in [Some(dir.join("runtime_cuda")), dir.parent().map(|p| p.join("runtime_cuda"))].into_iter().flatten() {
            if let Some(p) = ok(d) {
                return Some(p);
            }
        }
    }
    ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("dist").join("runtime_cuda"))
}

/// Precarga las DLL de CUDA desde `dir` y comprueba con cudart que hay una GPU NVIDIA con un driver que
/// la sirve. Devuelve su "compute capability" (p. ej. `sm_120`).
fn cuda_ready(dir: &Path) -> Result<String> {
    let mut libs = Vec::new();
    for name in CUDA_DLLS {
        // SAFETY: DLL oficiales de NVIDIA de `runtime_cuda/`; se quedan cargadas todo el proceso
        let lib = unsafe { libloading::Library::new(dir.join(name)) }.with_context(|| format!("cargando {name}"))?;
        libs.push(lib);
    }
    // SAFETY: firmas de cudart 12 (cuda_runtime_api.h); los punteros apuntan a variables locales
    let info = unsafe {
        let count: libloading::Symbol<unsafe extern "C" fn(*mut i32) -> i32> = libs[0].get(b"cudaGetDeviceCount")?;
        let attr: libloading::Symbol<unsafe extern "C" fn(*mut i32, i32, i32) -> i32> = libs[0].get(b"cudaDeviceGetAttribute")?;
        let mut n = 0;
        let e = count(&mut n);
        if e != 0 || n < 1 {
            return Err(anyhow!("sin GPU NVIDIA utilizable (cudaGetDeviceCount: error {e}, {n} GPU)"));
        }
        // 75/76 = cudaDevAttrComputeCapabilityMajor/Minor
        let (mut ma, mut mi) = (0, 0);
        attr(&mut ma, 75, 0);
        attr(&mut mi, 76, 0);
        format!("sm_{ma}{mi}")
    };
    for l in libs {
        std::mem::forget(l);
    }
    Ok(info)
}

/// "Compute capability" de la GPU si el motor es CUDA (para el registro).
static CUDA_SM: OnceLock<String> = OnceLock::new();

/// Descripción del motor para el registro, p. ej. "CUDA (sm_120, TF32 no)" o "DirectML".
pub fn engine_label() -> String {
    match engine() {
        Engine::Cuda => format!("CUDA ({}, TF32 {})", CUDA_SM.get().map(String::as_str).unwrap_or("?"), if tf32() { "sí" } else { "no" }),
        Engine::DirectMl => "DirectML".into(),
    }
}

/// `CAMCUTS_TF32=1`: cuDNN/cuBLAS pueden usar TF32 en las operaciones FP32 (solo CUDA).
fn tf32() -> bool {
    std::env::var("CAMCUTS_TF32").map(|v| v == "1").unwrap_or(false)
}

/// El proveedor CUDA con sus opciones (`CAMCUTS_CUDNN_BUSQUEDA` = `exhaustive` | `heuristic` | `default`,
/// para comparar).
fn cuda_ep() -> ort::ep::ExecutionProviderDispatch {
    use ort::ep::cuda::ConvAlgorithmSearch;
    let busqueda = match std::env::var("CAMCUTS_CUDNN_BUSQUEDA").unwrap_or_default().as_str() {
        "heuristic" => ConvAlgorithmSearch::Heuristic,
        "default" => ConvAlgorithmSearch::Default,
        _ => ConvAlgorithmSearch::Exhaustive,
    };
    ort::ep::CUDA::default()
        .with_device_id(0)
        .with_tf32(tf32())
        .with_conv_algorithm_search(busqueda)
        // la memoria de trabajo crece lo que se pide, no al doble (cada sesión tiene la suya)
        .with_arena_extend_strategy(ort::ep::ArenaExtendStrategy::SameAsRequested)
        .build()
        .error_on_failure()
}

/// Busca la carpeta `nombre` primero junto al ejecutable (la app instalada la trae al lado) y, si no
/// existe, en el workspace donde se compiló (desarrollo).
fn beside_exe_or_workspace(nombre: &str) -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join(nombre);
        if p.is_dir() {
            return p;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(nombre)
}

/// Carpeta `runtime/` (DLL de ONNX Runtime y DirectML): junto al ejecutable o en el workspace.
pub fn default_runtime_dir() -> PathBuf {
    beside_exe_or_workspace("runtime")
}

/// Carpeta `models/` (los .onnx y `clip_text.json`): junto al ejecutable o en el workspace.
pub fn default_models_dir() -> PathBuf {
    beside_exe_or_workspace("models")
}

/// Busca `onnxruntime.dll`: variable ORT_DYLIB_PATH, junto al ejecutable, o en `runtime_dir`.
fn find_dll(runtime_dir: &Path) -> Result<PathBuf> {
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Ok(p);
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join("onnxruntime.dll");
        if p.exists() {
            return Ok(p);
        }
    }
    let p = runtime_dir.join("onnxruntime.dll");
    if p.exists() {
        return Ok(p);
    }
    Err(anyhow!("no encuentro onnxruntime.dll (ni en ORT_DYLIB_PATH, ni junto al ejecutable, ni en {})", runtime_dir.display()))
}

/// Inicializa (una sola vez) ONNX Runtime desde `runtime_dir` y devuelve el entorno compartido.
pub fn environment(runtime_dir: &Path) -> Result<Environment> {
    let r = ENV.get_or_init(|| {
        let init = || -> Result<Environment> {
            // motor: CUDA si se pide o, en automático, si su carpeta está y la GPU responde
            let pedido = requested_engine();
            let cuda = match pedido {
                Some(Engine::DirectMl) => None,
                _ => match cuda_runtime_dir().ok_or_else(|| anyhow!("no está la carpeta runtime_cuda")).and_then(|d| cuda_ready(&d).map(|sm| (d, sm))) {
                    Ok((d, sm)) => {
                        let _ = CUDA_SM.set(sm);
                        Some(d)
                    }
                    Err(e) if pedido == Some(Engine::Cuda) => return Err(e.context("motor CUDA pedido (--motor cuda / CAMCUTS_MOTOR)")),
                    Err(_) => None,
                },
            };
            let _ = ENGINE.set(if cuda.is_some() { Engine::Cuda } else { Engine::DirectMl });
            let dll = match &cuda {
                Some(d) => d.join("onnxruntime.dll"),
                None => find_dll(runtime_dir)?,
            };
            // DirectML.dll al lado de onnxruntime.dll: se carga antes para que gane a la de System32
            let dml = dll.with_file_name("DirectML.dll");
            if cuda.is_none() && dml.exists() {
                // SAFETY: cargar una DLL ejecuta su DllMain; es la DLL oficial de Microsoft que trae ORT.
                let lib = unsafe { libloading::Library::new(&dml) }.with_context(|| format!("cargando {}", dml.display()))?;
                // se mantiene cargada durante todo el proceso
                std::mem::forget(lib);
            }
            let env = ort::init_from(&dll)
                .map_err(|e| anyhow!("cargando {}: {e}", dll.display()))?
                .with_name("camcuts")
                .build()
                .map_err(|e| anyhow!("creando el entorno de ONNX Runtime: {e}"))?;
            Ok(env)
        };
        init().map_err(|e| format!("{e:#}"))
    });
    r.clone().map_err(|e| anyhow!(e))
}

/// Abre un modelo ONNX en el dispositivo pedido.
///
/// DirectML exige ejecución secuencial y sin patrón de memoria; en CPU se dejan los hilos por defecto
/// salvo `cpu_threads`, que limita los hilos (útil para modelos pequeños como Silero VAD).
pub fn session(env: &Environment, model: &Path, device: Device, cpu_threads: Option<usize>) -> Result<Session> {
    session_dims(env, model, device, cpu_threads, &[])
}

/// [`session`] con las dimensiones libres del modelo fijadas (`("batch", 1)`, `("height", 544)`…).
///
/// DirectML rinde mucho mejor con formas conocidas al crear la sesión: con todas fijas compila el grafo
/// entero de una vez (fusiona operaciones) en vez de lanzar cada nodo por separado y recalcular formas en
/// cada llamada. Una sesión así solo acepta esa forma exacta (ver [`ShapedSession`]).
pub fn session_dims(env: &Environment, model: &Path, device: Device, cpu_threads: Option<usize>, dims: &[(&str, i64)]) -> Result<Session> {
    let build = || -> ort::Result<Session> {
        let mut b = Session::builder(env)?.with_optimization_level(GraphOptimizationLevel::Level3)?;
        for (name, size) in dims {
            b = b.with_dimension_override(*name, *size)?;
        }
        if let Some(n) = cpu_threads {
            b = b.with_intra_threads(n)?.with_inter_threads(1)?;
        }
        if device == Device::DirectMl {
            // Con DirectML la CPU solo hace operaciones de forma sueltas: pocos hilos y sin espera activa,
            // para que el grupo de hilos de ORT (por defecto uno por núcleo, girando tras cada llamada) no
            // le quite CPU a las medidas de imagen ni a ffmpeg.
            if cpu_threads.is_none() {
                b = b.with_intra_threads(2)?.with_intra_op_spinning(false)?;
            }
            if engine() == Engine::Cuda {
                // CUDA: si no se registra, error (nada de seguir en la CPU sin avisar, iría 10 veces más lento)
                return b.with_execution_providers([cuda_ep()])?.commit_from_file(model);
            }
            b = b
                .with_memory_pattern(false)?
                .with_parallel_execution(false)?
                .with_execution_providers([ort::ep::DirectML::default().with_device_id(0).build()])?;
        }
        b.commit_from_file(model)
    };
    build().map_err(|e| anyhow!("abriendo {}: {e}", model.display()))
}

/// Sesiones de forma fija compiladas en este proceso (todas las [`ShapedSession`]). Sirve para ver en el
/// perfil (`CAMCUTS_PROFILE`) si un modelo se está recompilando una y otra vez.
static COMPILACIONES_FIJAS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Cuántas sesiones de forma fija se han compilado en este proceso.
pub fn compilaciones_fijas() -> usize {
    COMPILACIONES_FIJAS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Busca `clave` en `lista` (la usada más recientemente al final) y la deja al final. Si no está, suelta
/// primero las más viejas hasta que quepa una más sin pasar de `max` y crea la nueva con `crear`: así
/// nunca hay más de `max` copias del modelo a la vez en la VRAM.
fn lru_tomar<K: PartialEq, T>(lista: &mut Vec<(K, T)>, clave: K, max: usize, crear: impl FnOnce() -> Result<T>) -> Result<&mut T> {
    match lista.iter().position(|(k, _)| *k == clave) {
        Some(i) => {
            let e = lista.remove(i);
            lista.push(e);
        }
        None => {
            while lista.len() >= max.max(1) {
                lista.remove(0);
            }
            let v = crear()?;
            lista.push((clave, v));
        }
    }
    Ok(&mut lista.last_mut().expect("recién puesta").1)
}

/// Un modelo con sus sesiones de forma fija (las de los cuadros que se están analizando; una sola por
/// defecto) y, si hace falta, la dinámica de siempre para lo demás.
///
/// Con DirectML, [`ShapedSession::get`] con `fixed = true` usa una sesión con las dimensiones fijadas
/// ([`session_dims`]); si llega otra forma fija (un clip vertical tras uno apaisado) se cierra la anterior
/// y se compila la nueva: una sola copia del modelo en la VRAM. Con una sesión por forma, una carpeta de
/// clips verticales (tras la preparación en apaisado) llevó la VRAM a 7.6 de 8 GB con 2 clips y la GPU se
/// quedó al 100 % paginando memoria. Lo que no es un cuadro de análisis (recortes de `--zoom`, lotes) va a
/// la sesión dinámica. En CPU siempre la dinámica (allí fijar formas no aporta).
///
/// Excepción: un detector compartido por varios hilos (`camcuts_infer::Compartido`, el propio) recibe
/// cuadros de varios clips intercalados, que pueden tener forma distinta (apaisado y vertical, o 16:9 y
/// DCI). Con una sola sesión fija se cerraba y recompilaba en casi cada llamada (segundos por muestra),
/// así que allí se guardan hasta 2 ([`ShapedSession::set_max_fijas`]); son modelos pequeños (YOLO11s y la
/// pose), no los 7.6 GB de YOLO-World, que sigue con una por hilo.
pub struct ShapedSession {
    env: Environment,
    model: PathBuf,
    device: Device,
    /// (dimensiones fijadas, sesión), la usada más recientemente al final
    fixed: Vec<(Vec<(String, i64)>, Session)>,
    /// cuántas sesiones fijas se guardan a la vez (1 salvo en un detector compartido)
    max_fijas: usize,
    dynamic: Option<Session>,
}

impl ShapedSession {
    /// Prepara el modelo; cada sesión se compila al primer cuadro de su forma. Si no se fijan formas
    /// (CPU) la dinámica se abre ya, así que un modelo dañado falla aquí como antes.
    pub fn open(env: &Environment, model: &Path, device: Device) -> Result<Self> {
        if !model.exists() {
            return Err(anyhow!("abriendo {}: no existe", model.display()));
        }
        let mut s = Self { env: env.clone(), model: model.to_path_buf(), device, fixed: Vec::new(), max_fijas: 1, dynamic: None };
        if !s.fixes_shapes() {
            s.dynamic = Some(session(env, model, device, None)?);
        }
        Ok(s)
    }

    /// Fijar formas solo se hace con DirectML (variable `CAMCUTS_FORMA_DINAMICA=1`: siempre dinámica,
    /// para comparar).
    fn fixes_shapes(&self) -> bool {
        self.device == Device::DirectMl && std::env::var_os("CAMCUTS_FORMA_DINAMICA").is_none()
    }

    /// Cuántas sesiones fijas se guardan a la vez (mínimo 1). Si ya hay más, se sueltan las más viejas.
    pub fn set_max_fijas(&mut self, n: usize) {
        self.max_fijas = n.max(1);
        while self.fixed.len() > self.max_fijas {
            self.fixed.remove(0);
        }
    }

    /// La sesión para estas dimensiones. `fixed`: es la forma de los cuadros de análisis (la que se repite
    /// en cada muestra) y merece su sesión fija; si no, la dinámica.
    pub fn get(&mut self, dims: &[(&str, i64)], fixed: bool) -> Result<&mut Session> {
        if fixed && self.fixes_shapes() {
            let key: Vec<(String, i64)> = dims.iter().map(|(n, v)| (n.to_string(), *v)).collect();
            let (env, model, device) = (&self.env, &self.model, self.device);
            // si no está, primero se suelta la más vieja (nunca más de `max_fijas` copias fijas a la vez)
            return lru_tomar(&mut self.fixed, key, self.max_fijas, || {
                let n = COMPILACIONES_FIJAS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if std::env::var_os("CAMCUTS_PROFILE").is_some() {
                    eprintln!("[perfil] compilando sesión fija de {} {dims:?} ({n} en el proceso)", model.file_name().map(|f| f.to_string_lossy()).unwrap_or_default());
                }
                session_dims(env, model, device, None, dims)
            });
        }
        if self.dynamic.is_none() {
            self.dynamic = Some(session(&self.env, &self.model, self.device, None)?);
        }
        Ok(self.dynamic.as_mut().unwrap())
    }
}

/// Ejecuta una sesión con entradas f32/i64 ya preparadas y devuelve cada salida como (forma, datos f32).
pub fn run_f32(session: &mut Session, inputs: Vec<(&str, ort::value::DynValue)>) -> Result<Vec<(Vec<i64>, Vec<f32>)>> {
    let named: Vec<(std::borrow::Cow<'_, str>, ort::session::SessionInputValue<'_>)> =
        inputs.into_iter().map(|(n, v)| (std::borrow::Cow::from(n), ort::session::SessionInputValue::from(v))).collect();
    let outputs = session.run(named).map_err(|e| anyhow!("inferencia: {e}"))?;
    let mut res = Vec::with_capacity(outputs.len());
    for (_, v) in outputs.iter() {
        let (shape, data) = v.try_extract_tensor::<f32>().map_err(|e| anyhow!("salida no f32: {e}"))?;
        res.push((shape.iter().copied().collect(), data.to_vec()));
    }
    Ok(res)
}

/// Tensor f32 con la forma dada.
pub fn tensor_f32(shape: Vec<i64>, data: Vec<f32>) -> Result<ort::value::DynValue> {
    Ok(ort::value::Tensor::from_array((shape, data)).map_err(|e| anyhow!("{e}"))?.into_dyn())
}

/// Tensor i64 con la forma dada.
pub fn tensor_i64(shape: Vec<i64>, data: Vec<i64>) -> Result<ort::value::DynValue> {
    Ok(ort::value::Tensor::from_array((shape, data)).map_err(|e| anyhow!("{e}"))?.into_dyn())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Con 2 huecos, dos formas que se alternan (dos clips de un detector compartido, uno apaisado y otro
    /// vertical) se compilan una vez cada una; con 1 hueco (lo de siempre) se recompilaba en cada cambio.
    #[test]
    fn dos_formas_alternas_no_se_recompilan() {
        let formas = ["apaisado", "vertical", "apaisado", "vertical", "apaisado", "vertical"];
        let compilar = |max: usize| {
            let mut lista: Vec<(&str, usize)> = Vec::new();
            let mut n = 0usize;
            for f in formas {
                let v = *lru_tomar(&mut lista, f, max, || {
                    n += 1;
                    Ok(n)
                })
                .unwrap();
                assert!(lista.len() <= max, "nunca más de {max} copias");
                assert_eq!(lista.last().unwrap().0, f, "la usada queda la más reciente");
                let _ = v;
            }
            n
        };
        assert_eq!(compilar(2), 2, "con 2 huecos: una compilación por forma");
        assert_eq!(compilar(1), formas.len(), "con 1 hueco: una por cambio de forma");
        // una tercera forma suelta la menos usada recientemente
        let mut lista: Vec<(&str, u8)> = Vec::new();
        for f in ["a", "b", "a", "c"] {
            lru_tomar(&mut lista, f, 2, || Ok(0)).unwrap();
        }
        assert_eq!(lista.iter().map(|e| e.0).collect::<Vec<_>>(), vec!["a", "c"]);
        // un fallo al crear no deja nada a medias
        let mut lista: Vec<(&str, u8)> = Vec::new();
        assert!(lru_tomar(&mut lista, "x", 1, || Err(anyhow!("sin VRAM"))).is_err());
        assert!(lista.is_empty());
    }
}
