//! Trabajos de fondo de la ventana: el análisis (proceso hijo), los proxies que faltan, la lectura
//! de metadatos y la detección de perfiles de color.
//!
//! Como en Python, el análisis corre como un proceso aparte (la línea de comandos de CAMCUTS): la
//! ventana sigue respondiendo y un fallo del análisis no la tumba. Todos los hilos avisan a la
//! interfaz con `Mensaje` por un canal y piden repintar.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::datos::{self, sin_consola, sin_consola_baja, Meta};
use crate::motor::{self, ColorSpec};

/// Avisos de los hilos de fondo a la interfaz.
pub enum Mensaje {
    /// Una línea de la salida del análisis.
    Linea(String),
    /// El análisis terminó con este código.
    Fin(i32),
    /// Metadatos leídos con ffprobe y si se pudieron guardar en `clips_meta.json`.
    Metas(BTreeMap<String, Meta>, bool),
    /// Perfiles de color detectados (ruta en minúsculas -> conversión).
    Colores(HashMap<String, ColorSpec>),
    /// Progreso de los proxies de vista previa ("3/10" o "done").
    Proxy(String),
}

/// Primer argumento que pone a `camcuts.exe` en modo análisis (sin ventana).
pub const ARG_ANALIZAR: &str = "--analizar";

/// Ejecutable y argumentos previos del análisis: `CAMCUTS_CLI` si está definida (otra CLI con las
/// mismas opciones que Python), si no el propio `camcuts.exe --analizar`.
pub fn ruta_cli() -> Option<(PathBuf, Vec<String>)> {
    if let Some(p) = std::env::var_os("CAMCUTS_CLI").map(PathBuf::from).filter(|p| p.exists()) {
        return Some((p, Vec::new()));
    }
    Some((std::env::current_exe().ok()?, vec![ARG_ANALIZAR.to_owned()]))
}

/// Carpeta junto al ejecutable si existe (así la app instalada lleva `runtime/` y `models/` a su lado);
/// si no, la del workspace que conoce `camcuts-infer`.
fn junto_al_exe(nombre: &str, respaldo: PathBuf) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(nombre)))
        .filter(|p| p.exists())
        .unwrap_or(respaldo)
}

/// Modo análisis de `camcuts.exe`: las mismas opciones que `python -m camcuts` (las lee el parser de
/// la CLI del núcleo) y los modelos de `camcuts-infer` (ONNX Runtime con DirectML). Devuelve el código
/// de salida. La salida (`[3/10] clip...`) va a stdout, que la ventana lee por una tubería.
///
/// `--detector propio|world` lo entiende el propio parser del núcleo (hace lo mismo que
/// `DetectorKind::take_arg`): el elegido va a los detectores y a la regla de caché vieja del núcleo.
pub fn ejecutar_analisis(args: Vec<String>) -> i32 {
    use clap::Parser;
    let todos = std::iter::once("camcuts".to_owned()).chain(args);
    let args = match camcuts_core::cli::Args::try_parse_from(todos) {
        Ok(a) => a,
        Err(e) => {
            // --help y --version no son errores
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    let cfg = camcuts_infer::InferConfig {
        models_dir: junto_al_exe("models", camcuts_infer::runtime::default_models_dir()),
        runtime_dir: junto_al_exe("runtime", camcuts_infer::runtime::default_runtime_dir()),
        // `--detector`, si no `CAMCUTS_DETECTOR`, si no el propio (en Bodas siempre el de boda)
        detector: args.detector_kind(),
        ..Default::default()
    };
    // motor de la GPU (CUDA si está `runtime_cuda`, si no DirectML); si CUDA falla, se relanza con DirectML
    if let Some(codigo) = camcuts_infer::motor_listo(args.motor.as_deref(), &cfg) {
        return codigo;
    }
    // con el detector propio, los clips en los que casi nunca da la joya nítida se analizan otra vez con
    // el clásico (YOLO-World) con detectores de esta otra fábrica (aunque no quepa el grande: ese pase
    // tiene que ser con YOLO-World, no con el propio otra vez)
    let cfg_respaldo = camcuts_infer::InferConfig { detector: camcuts_infer::DetectorKind::World, propio_si_no_cabe: false, ..cfg.clone() };
    let respaldo = move || -> anyhow::Result<Box<dyn camcuts_core::Detector>> { Ok(Box::new(camcuts_infer::OnnxDetector::new(cfg_respaldo.clone())?)) };
    // cada hilo de análisis pide su detector a esta fábrica (con el propio, todos comparten uno)
    let fabrica = camcuts_infer::fabrica(cfg);
    match camcuts_core::cli::run_con_respaldo(args, Some(&fabrica), Some(&respaldo)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

/// Trabajo de Windows (Job Object) con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: cuando se cierra su último
/// handle (al soltar el `Trabajo`, o al morir la ventana por un cuelgue o desde el Administrador de
/// tareas, porque Windows cierra los handles del proceso), Windows mata todos los procesos del trabajo:
/// el `--analizar` y los ffmpeg que lance, que heredan el trabajo. Sin esto seguían vivos y ocupando la
/// GPU tras un cierre brusco.
#[cfg(windows)]
pub struct Trabajo(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: el handle de un trabajo es un identificador del núcleo; se puede usar y cerrar desde
// cualquier hilo.
#[cfg(windows)]
unsafe impl Send for Trabajo {}
#[cfg(windows)]
unsafe impl Sync for Trabajo {}

#[cfg(windows)]
impl Trabajo {
    /// Crea un trabajo anónimo que mata sus procesos al cerrarse (None si Windows no lo permite).
    pub fn nuevo() -> Option<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: llamadas a la API de Windows con punteros válidos; la estructura es POD (ceros = sin
        // límites) y solo se activa KILL_ON_JOB_CLOSE. Si algo falla se cierra el handle.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(h);
                return None;
            }
            Some(Self(h))
        }
    }

    /// Mete un proceso (y lo que lance después) en el trabajo. `false` si Windows no lo deja.
    pub fn asignar(&self, hijo: &Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: los dos handles son válidos mientras `self` y `hijo` existan.
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.0, hijo.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE) != 0 }
    }
}

#[cfg(windows)]
impl Drop for Trabajo {
    fn drop(&mut self) {
        // SAFETY: el handle es nuestro y se cierra una sola vez; cerrarlo mata lo que quede en el trabajo
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Fuera de Windows no hay Job Objects: el análisis se detiene como siempre (Detener / al cerrar).
#[cfg(not(windows))]
pub struct Trabajo;

#[cfg(not(windows))]
impl Trabajo {
    pub fn nuevo() -> Option<Self> {
        None
    }
    pub fn asignar(&self, _hijo: &Child) -> bool {
        false
    }
}

/// Un análisis en curso.
pub struct Analisis {
    hijo: Arc<Mutex<Child>>,
    pub pid: u32,
    /// El trabajo de Windows del análisis: al soltarlo (fin del análisis o muerte de la ventana) no
    /// queda vivo ningún proceso suyo.
    _trabajo: Option<Trabajo>,
}

impl Analisis {
    /// Lanza el análisis con las mismas opciones que la ventana de Python pasa a `python -m camcuts`.
    pub fn lanzar(cli: &Path, args: &[String], tx: Sender<Mensaje>, despertar: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let mut c = Command::new(cli);
        c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        c.env("PYTHONIOENCODING", "utf-8");
        // el avance de cada clip (`progreso<TAB>37<TAB>ruta`) para el porcentaje de la lista; una CLI que no
        // lo conozca lo ignora
        c.env(camcuts_core::cli::VARIABLE_PROGRESO, "1");
        sin_consola(&mut c);
        let mut hijo = c.spawn()?;
        let pid = hijo.id();
        // el hijo va al trabajo nada más nacer (antes de cargar modelos y lanzar ningún ffmpeg)
        let trabajo = Trabajo::nuevo().filter(|t| t.asignar(&hijo));
        if trabajo.is_none() {
            eprintln!("aviso: no se pudo atar el análisis a la ventana (Job Object); si la ventana muere, usa Detener");
        }
        let salida = hijo.stdout.take();
        let errores = hijo.stderr.take();
        let hijo = Arc::new(Mutex::new(hijo));
        let despertar = Arc::new(despertar);
        // stderr también va a la barra de estado (avisos del análisis)
        if let Some(e) = errores {
            let tx = tx.clone();
            let d = despertar.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(e).lines().map_while(Result::ok) {
                    let _ = tx.send(Mensaje::Linea(l));
                    d();
                }
            });
        }
        let h2 = hijo.clone();
        std::thread::spawn(move || {
            if let Some(s) = salida {
                for l in BufReader::new(s).lines().map_while(Result::ok) {
                    let _ = tx.send(Mensaje::Linea(l));
                    despertar();
                }
            }
            // esperar sin bloquear el mutex mucho tiempo (Detener lo necesita)
            let codigo = loop {
                if let Ok(mut h) = h2.lock() {
                    if let Ok(Some(st)) = h.try_wait() {
                        break st.code().unwrap_or(-1);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            let _ = tx.send(Mensaje::Fin(codigo));
            despertar();
        });
        Ok(Self { hijo, pid, _trabajo: trabajo })
    }

    /// Detiene el análisis y todos sus hijos (ffmpeg), como `taskkill /T /F` en Python.
    pub fn detener(&self) {
        let mut c = Command::new("taskkill");
        c.args(["/T", "/F", "/PID", &self.pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
        sin_consola(&mut c);
        if c.status().is_err() {
            if let Ok(mut h) = self.hijo.lock() {
                let _ = h.kill();
            }
        }
    }
}

/// Interpreta "[3/10] C9172.MP4: ..." -> (3, 10, resto).
pub fn progreso(linea: &str) -> Option<(usize, usize, String)> {
    let r = linea.strip_prefix('[')?;
    let (dentro, resto) = r.split_once(']')?;
    let (a, b) = dentro.split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, resto.trim().to_owned()))
}

/// Interpreta "progreso\t37\tD:\...\C9243.MP4" -> (Some(37), ruta); con `error` en vez del número,
/// (None, ruta).
pub fn avance(linea: &str) -> Option<(Option<u8>, &str)> {
    let mut partes = linea.strip_prefix("progreso\t")?.splitn(2, '\t');
    let valor = partes.next()?;
    let ruta = partes.next().filter(|r| !r.is_empty())?;
    match valor {
        "error" => Some((None, ruta)),
        v => Some((Some(v.parse::<u8>().ok()?.min(100)), ruta)),
    }
}

/// Lee metadatos de los clips que no los tienen y los guarda en `clips_meta.json`.
pub fn sondear(rutas: Vec<PathBuf>, out: PathBuf, tx: Sender<Mensaje>, despertar: impl Fn() + Send + Sync + 'static) {
    std::thread::spawn(move || {
        let mut metas = datos::leer_metas(&out);
        for p in rutas {
            metas.insert(datos::clave(&p), datos::sondear_meta(&p));
        }
        // si no se puede escribir, la ventana se queda con lo sondeado en memoria (y avisa una vez)
        let escrito = datos::escribir_metas(&out, &metas);
        let _ = tx.send(Mensaje::Metas(metas, escrito));
        despertar();
    });
}

/// Detecta el perfil de color de cámara de cada clip (XML de Sony, ffprobe…).
pub fn detectar_colores(rutas: Vec<PathBuf>, tx: Sender<Mensaje>, despertar: impl Fn() + Send + Sync + 'static) {
    std::thread::spawn(move || {
        let mut encontrados = HashMap::new();
        for p in rutas {
            if let Some(id) = motor::detectar_perfil(&p) {
                encontrados.insert(datos::clave(&p), ColorSpec { id, cube: None });
            }
        }
        let _ = tx.send(Mensaje::Colores(encontrados));
        despertar();
    });
}

/// Trabajo que crea los proxies de vista previa que faltan (clips analizados antes de que existieran).
pub struct TrabajoProxies {
    pub parar: Arc<AtomicBool>,
    hijo: Arc<Mutex<Option<Child>>>,
    vivo: Arc<AtomicBool>,
}

/// Tamaño de análisis: lado largo 960, medidas pares (como `scaled_size` de Python).
pub fn medida_analisis(w: u32, h: u32) -> (u32, u32) {
    let k = 960.0 / w.max(h).max(1) as f64;
    let par = |x: f64| ((x / 2.0).round() as u32 * 2).max(2);
    (par(w as f64 * k), par(h as f64 * k))
}

impl TrabajoProxies {
    /// `clips`: (video a decodificar, ancho, alto, proxy destino). Uno a la vez y con prioridad baja.
    pub fn lanzar(clips: Vec<(PathBuf, u32, u32, PathBuf)>, tx: Sender<Mensaje>, despertar: impl Fn() + Send + Sync + 'static) -> Self {
        let parar = Arc::new(AtomicBool::new(false));
        let hijo: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let vivo = Arc::new(AtomicBool::new(true));
        let (p2, h2, v2) = (parar.clone(), hijo.clone(), vivo.clone());
        std::thread::spawn(move || {
            // los ffmpeg de los proxies van a un trabajo de Windows, como el análisis: si la ventana muere
            // de golpe, Windows los mata (antes seguían usando NVDEC y dejaban un .part.mp4 huérfano)
            let trabajo = Trabajo::nuevo();
            let total = clips.len();
            for (n, (fuente, w, h, destino)) in clips.into_iter().enumerate() {
                if p2.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tx.send(Mensaje::Proxy(format!("{}/{}", n + 1, total)));
                despertar();
                let parte = destino.with_extension("part.mp4");
                if let Some(d) = destino.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let (pw, ph) = medida_analisis(w, h);
                // mismos ajustes que el proxy del análisis: GOP corto para buscar al instante
                let mut c = Command::new("ffmpeg");
                c.args(["-v", "error", "-y", "-nostdin", "-hwaccel", "auto", "-i"]).arg(&fuente);
                c.args(["-vf", &format!("scale={pw}:{ph}:flags=area")]);
                c.args(camcuts_core::sampler::PROXY_ARGS).arg(&parte);
                c.stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
                sin_consola_baja(&mut c);
                // Se lanza ffmpeg con el candado tomado y tras volver a mirar `parar`: así `detener` o bien
                // llega antes (y no se lanza nada) o bien encuentra el proceso ya guardado y lo mata. Sin
                // esto, un ffmpeg lanzado justo durante `detener` quedaba vivo tras cerrar la app.
                {
                    let Ok(mut g) = h2.lock() else { break };
                    if p2.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(proc_) = c.spawn() else { continue };
                    // al trabajo nada más nacer (con el candado tomado: `detener` no lo ve antes)
                    if let Some(t) = &trabajo {
                        t.asignar(&proc_);
                    }
                    *g = Some(proc_);
                }
                let ok = loop {
                    let estado = h2.lock().ok().and_then(|mut g| g.as_mut().map(|c| c.try_wait()));
                    match estado {
                        Some(Ok(Some(st))) => break st.success(),
                        Some(Ok(None)) => std::thread::sleep(std::time::Duration::from_millis(200)),
                        _ => break false,
                    }
                };
                if ok && parte.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                    let _ = std::fs::rename(&parte, &destino);
                } else {
                    let _ = std::fs::remove_file(&parte);
                }
            }
            v2.store(false, Ordering::Relaxed);
            let _ = tx.send(Mensaje::Proxy("done".into()));
            despertar();
        });
        Self { parar, hijo, vivo }
    }

    /// ¿Sigue trabajando?
    pub fn vivo(&self) -> bool {
        self.vivo.load(Ordering::Relaxed)
    }

    /// Para en cuanto se pueda (el análisis necesita la GPU y hace sus propios proxies).
    pub fn detener(&self) {
        self.parar.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.hijo.lock() {
            if let Some(c) = g.as_mut() {
                let _ = c.kill();
            }
        }
    }
}

#[cfg(test)]
mod pruebas {
    #[test]
    fn progreso_de_linea() {
        assert_eq!(super::progreso("[3/10] C9172.MP4: 40 muestras"), Some((3, 10, "C9172.MP4: 40 muestras".into())));
        assert_eq!(super::progreso("equipo: GPU"), None);
        assert_eq!(super::medida_analisis(3840, 2160), (960, 540));
    }

    #[test]
    fn avance_de_linea() {
        assert_eq!(super::avance("progreso\t37\tD:\\a b\\C9243.MP4"), Some((Some(37), "D:\\a b\\C9243.MP4")));
        assert_eq!(super::avance("progreso\terror\tD:\\x.mov"), Some((None, "D:\\x.mov")));
        assert_eq!(super::avance("progreso\t100\tC:\\c.mp4"), Some((Some(100), "C:\\c.mp4")));
        assert_eq!(super::avance("progreso\tx\tC:\\c.mp4"), None);
        assert_eq!(super::avance("progreso\t5\t"), None);
        assert_eq!(super::avance("[3/10] C9172.MP4: 40 muestras"), None);
        // una línea de avance no es una de clip terminado (no hace recargar la lista)
        assert_eq!(super::progreso("progreso\t37\tD:\\C9243.MP4"), None);
    }

    /// Cerrar el trabajo (como cuando la ventana muere) mata al hijo y a sus nietos: aquí un `cmd` sin
    /// consola que lanza un `ping` de 30 s, como el análisis lanza sus ffmpeg.
    #[cfg(windows)]
    #[test]
    fn trabajo_mata_al_hijo_al_cerrarse() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let mut c = Command::new("cmd");
        c.args(["/c", "ping -n 30 127.0.0.1 >nul"]).stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
        super::sin_consola(&mut c);
        let mut hijo = c.spawn().expect("lanzar cmd");
        let t = super::Trabajo::nuevo().expect("crear el Job Object");
        assert!(t.asignar(&hijo), "meter el hijo en el trabajo");
        std::thread::sleep(Duration::from_millis(300));
        assert!(hijo.try_wait().unwrap().is_none(), "el hijo sigue vivo mientras el trabajo existe");
        drop(t);
        let inicio = Instant::now();
        while hijo.try_wait().unwrap().is_none() {
            assert!(inicio.elapsed() < Duration::from_secs(10), "el hijo sigue vivo tras cerrar el trabajo");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
