//! Hilos de trabajo, prioridad del proceso y modo de GPU (port de `hardware.py`).
//!
//! Modo ligero (como `hardware.detect` de Python): si otras apps (DaVinci, Lightroom…) dejan menos de
//! [`LOW_VRAM_GB`] de VRAM libre, los modelos no caben, Windows los pasa a la RAM del sistema y el
//! análisis va decenas de veces más lento. Entonces, con YOLO-World, se usa el mediano (yolov8m-worldv2;
//! en joyas lo sustituye el detector propio) y un solo clip a la vez; con el propio no cambia nada. La
//! línea de comandos decide el modo de cada pase con [`decide_gpu_mode`] y lo publica con
//! [`set_gpu_mode`]; `camcuts-infer` lo lee con [`gpu_mode`] al crear cada detector.
//!
//! Cuántos clips se analizan a la vez lo decide [`parallel_clips`] (modo, `--workers`, `--background`
//! y el tope de YOLO-World, que con 4 clips reinició el driver de la GPU).

use std::sync::atomic::{AtomicU8, Ordering};

/// Con el editor trabajando (DaVinci en la misma GPU): un solo clip a la vez. Con 2 la GPU llegaba al
/// ~92 %; el editor pidió ~60 % mientras edita. Medido el 2026-11-03 en Octubre 27 (19 clips): 1 clip va
/// ya al 56 % de media (p95 73 %) y tarda 226 s (antes 300 s); 2 clips pasarían del 80 %.
pub const BACKGROUND_WORKERS: usize = 1;

/// Por debajo de esta VRAM libre (GB): detector mediano y un clip a la vez. Python usaba 3.0, pero un clip
/// con YOLO-World x necesita ~3.3 GB (2.2 de modelos + 1.1 del ffmpeg con NVDEC) y Windows empieza a pasar
/// memoria a la RAM antes de llenar la VRAM. Medido con DaVinci abierto (RTX 5060, 8 GB): con 3.2 GB
/// libres, 1.4 GB fueron a la RAM y un clip de 35 s tardó 109 s; con 3.8 GB, DirectML no pudo ni compilar
/// el modelo y todos los clips salían con error. En modo ligero, el mismo clip: 8.5 s.
pub const LOW_VRAM_GB: f64 = 4.5;

/// Modo de GPU del análisis. Solo cambia algo con YOLO-World; con el detector propio (el de por defecto)
/// el modo ligero no cambia ni el detector ni los clips en paralelo (los limita [`vram_workers`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    /// YOLO-World grande (yolov8x-worldv2), clips en paralelo según el equipo.
    Normal,
    /// YOLO-World mediano (yolov8m-worldv2; en joyas lo sustituye el propio, que no confunde las joyas)
    /// y un solo clip a la vez.
    Ligero,
}

/// Modo publicado para este proceso: 0 = sin fijar (manda `InferConfig`), 1 = normal, 2 = ligero. Un
/// `AtomicU8` y no un `OnceLock`: el respaldo automático es un segundo pase en el mismo proceso que
/// decide su propio modo, y con `OnceLock` contaba solo el del primero (la línea "equipo:" del respaldo
/// decía un modo y sus detectores abrían el otro).
static GPU_MODE: AtomicU8 = AtomicU8::new(0);

/// Señal de que la GPU se quedó sin memoria a mitad de un análisis: un clip con YOLO-World x que va a más
/// de estos segundos por muestra. Normal: ~0.05 s (1 clip) a ~0.1 s (2 clips, `--zoom`); con 1.4 GB de
/// los modelos pasados a la RAM por Windows (DaVinci abierto), 0.8 s.
pub const LENTO_S_POR_MUESTRA: f64 = 0.4;

/// Muestras mínimas de un clip para juzgar su velocidad (en clips cortos pesan el arranque y la
/// compilación de una orientación nueva).
pub const LENTO_MIN_MUESTRAS: usize = 80;

/// El detector grande no cupo en la GPU durante este análisis ([`bajar_a_ligero`]).
static BAJADO_A_LIGERO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Publica el modo de GPU del proceso. Cada pase del análisis (el normal y el del respaldo) publica el
/// suyo y la última llamada manda; [`bajar_a_ligero`] sigue mandando sobre los dos.
pub fn set_gpu_mode(mode: GpuMode) {
    GPU_MODE.store(
        match mode {
            GpuMode::Normal => 1,
            GpuMode::Ligero => 2,
        },
        Ordering::Relaxed,
    );
}

/// Modo de GPU publicado por la línea de comandos, si lo hay; ligero desde que el detector grande no
/// cupo ([`bajar_a_ligero`]).
pub fn gpu_mode() -> Option<GpuMode> {
    if BAJADO_A_LIGERO.load(Ordering::Relaxed) {
        return Some(GpuMode::Ligero);
    }
    match GPU_MODE.load(Ordering::Relaxed) {
        1 => Some(GpuMode::Normal),
        2 => Some(GpuMode::Ligero),
        _ => None,
    }
}

/// Variable con la que todo el análisis va a la CPU (la misma que lee `camcuts_infer::InferConfig`).
pub const VARIABLE_DISPOSITIVO: &str = "CAMCUTS_DISPOSITIVO";

/// ¿`CAMCUTS_DISPOSITIVO` pide la CPU? (separada de [`en_cpu`] para probarla sin tocar el entorno)
fn pide_cpu(valor: Option<&str>) -> bool {
    valor.map(|v| v.trim().eq_ignore_ascii_case("cpu")).unwrap_or(false)
}

/// ¿Analizan los modelos en la CPU (`CAMCUTS_DISPOSITIVO=cpu`)? Entonces la VRAM libre no cuenta (no se
/// usa) y ~2 s por muestra es lo normal, no una GPU sin memoria: ni modo ligero ni guarda de lentitud.
pub fn en_cpu() -> bool {
    pide_cpu(std::env::var(VARIABLE_DISPOSITIVO).ok().as_deref())
}

/// El detector grande no se pudo abrir en la GPU (otra app la llenó después de decidir el modo): desde
/// ahora, los detectores nuevos de este proceso usan el mediano. Devuelve `true` la primera vez (para
/// avisar una sola vez).
pub fn bajar_a_ligero() -> bool {
    !BAJADO_A_LIGERO.swap(true, Ordering::Relaxed)
}

/// La GPU principal: nombre, VRAM total y libre en GB.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub total_gb: f64,
    pub free_gb: f64,
}

/// VRAM de la GPU: primero `nvidia-smi` (sin ventana, con tiempo máximo), que da la memoria libre de TODO
/// el equipo (DaVinci, Lightroom… incluidos); DXGI solo si no hay `nvidia-smi` (GPU de otra marca).
///
/// DXGI (`QueryVideoMemoryInfo`) da el presupuesto de ESTE proceso menos su uso, que no cuenta lo que
/// ocupan las otras apps: en este equipo marcaba 6.87 GB libres con nvidia-smi en 2.78 GB, así que con
/// DaVinci abierto el modo ligero no saltaba nunca.
pub fn gpu_info() -> Option<GpuInfo> {
    pick_gpu_info(nvidia_smi, || {
        #[cfg(windows)]
        {
            dxgi::query()
        }
        #[cfg(not(windows))]
        {
            None
        }
    })
}

/// Orden de las fuentes de [`gpu_info`]: la de todo el equipo primero.
fn pick_gpu_info(system_wide: impl FnOnce() -> Option<GpuInfo>, this_process: impl FnOnce() -> Option<GpuInfo>) -> Option<GpuInfo> {
    system_wide().or_else(this_process)
}

/// Tiempo máximo de `nvidia-smi` (normalmente tarda ~0.2 s; un driver colgado no debe parar el análisis).
const NVIDIA_SMI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// `nvidia-smi`: memoria total y libre de la primera GPU, en todo el equipo.
fn nvidia_smi() -> Option<GpuInfo> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut c = Command::new("nvidia-smi");
    c.args(["--query-gpu=name,memory.total,memory.free", "--format=csv,noheader,nounits"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::sampler::no_window(&mut c);
    let mut child = c.spawn().ok()?;
    let t0 = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => break,
            Ok(None) if t0.elapsed() < NVIDIA_SMI_TIMEOUT => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_nvidia_smi(&out)
}

/// Lee la primera línea de `nvidia-smi --query-gpu=name,memory.total,memory.free` (MiB, sin unidades).
fn parse_nvidia_smi(text: &str) -> Option<GpuInfo> {
    let line = text.lines().next()?;
    let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
    let (total, free): (f64, f64) = (parts.get(1)?.parse().ok()?, parts.get(2)?.parse().ok()?);
    Some(GpuInfo { name: parts[0].to_string(), total_gb: total / 1024.0, free_gb: free / 1024.0 })
}

/// Detector del modo ligero en la línea "equipo:" (la línea de comandos lo cambia cuando el propio
/// sustituye al YOLO-World mediano).
pub const TEXTO_LIGERO: &str = "detector mediano (yolov8m-worldv2)";

/// Decide el modo: `forced` = "normal" / "ligero" desde `--gpu`, cualquier otra cosa = automático según
/// la VRAM libre. Devuelve el modo y la línea "equipo: ..." para el registro (la ventana la muestra en
/// la barra de estado, en rojo si contiene "OCUPADA").
pub fn decide_gpu_mode(forced: &str, gpu: Option<&GpuInfo>, workers: usize) -> (GpuMode, String) {
    decide_gpu_mode_en(forced, gpu, workers, en_cpu())
}

/// [`decide_gpu_mode`] sabiendo si los modelos van en la CPU (`cpu`). En la CPU no se mira la VRAM: el
/// modo normal (el detector grande, que en la CPU solo cuesta tiempo) y una línea que lo dice. Antes se
/// miraba nvidia-smi igual y, con DaVinci abierto, la edición Bodas pasaba al detector mediano sin motivo.
pub fn decide_gpu_mode_en(forced: &str, gpu: Option<&GpuInfo>, workers: usize, cpu: bool) -> (GpuMode, String) {
    let hilos = cpu_threads();
    if cpu {
        return (GpuMode::Normal, format!("equipo: CPU ({VARIABLE_DISPOSITIVO}=cpu), {hilos} hilos de CPU -> {workers} clips en paralelo"));
    }
    let donde = gpu.map(|g| format!("{} ({:.0} GB, {:.1} GB libres)", g.name, g.total_gb, g.free_gb)).unwrap_or_else(|| "GPU sin datos de VRAM".into());
    let ligero = |motivo: &str| format!("equipo: {donde} — {motivo}; modo ligero: {TEXTO_LIGERO}, 1 clip a la vez, {hilos} hilos de CPU");
    match forced {
        "normal" => (GpuMode::Normal, format!("equipo: {donde} — modo normal forzado (--gpu normal), {hilos} hilos de CPU -> {workers} clips en paralelo")),
        "ligero" => (GpuMode::Ligero, ligero("forzado con --gpu ligero")),
        _ => match gpu {
            Some(g) if g.free_gb < LOW_VRAM_GB => (
                GpuMode::Ligero,
                ligero(&format!("GPU OCUPADA por otras apps: {:.1} GB libres (cierra DaVinci/Lightroom/Cutdown para ir más rápido)", g.free_gb)),
            ),
            _ => (GpuMode::Normal, format!("equipo: {donde}, {hilos} hilos de CPU -> {workers} clips en paralelo")),
        },
    }
}

/// Consulta de VRAM con DXGI sin crates extra: `dxgi.dll` se carga en tiempo de ejecución y los métodos
/// COM se llaman por su posición en la vtable (IDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3).
#[cfg(windows)]
mod dxgi {
    use std::ffi::c_void;

    use super::GpuInfo;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const IID_IDXGI_FACTORY1: Guid = Guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
    const IID_IDXGI_ADAPTER3: Guid = Guid(0x645967a4, 0x1392, 0x4310, [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd]);

    /// `DXGI_ADAPTER_DESC1`.
    #[repr(C)]
    struct AdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
        dedicated_video_memory: usize,
        dedicated_system_memory: usize,
        shared_system_memory: usize,
        luid_low: u32,
        luid_high: i32,
        flags: u32,
    }

    /// `DXGI_QUERY_VIDEO_MEMORY_INFO`.
    #[repr(C)]
    #[derive(Default)]
    struct MemInfo {
        budget: u64,
        current_usage: u64,
        available_for_reservation: u64,
        current_reservation: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    /// Puntero a la función `idx` de la vtable de un objeto COM.
    unsafe fn method(obj: *mut c_void, idx: usize) -> *const c_void {
        let vtbl = *(obj as *const *const *const c_void);
        *vtbl.add(idx)
    }

    /// `IUnknown::Release`.
    unsafe fn release(obj: *mut c_void) {
        if !obj.is_null() {
            let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(method(obj, 2));
            f(obj);
        }
    }

    pub fn query() -> Option<GpuInfo> {
        // SAFETY: firmas y posiciones de vtable de los encabezados de DXGI (dxgi.h, dxgi1_4.h); cada
        // objeto obtenido se libera con Release antes de salir.
        unsafe {
            let name: Vec<u16> = "dxgi.dll\0".encode_utf16().collect();
            let lib = LoadLibraryW(name.as_ptr());
            if lib.is_null() {
                return None;
            }
            let create = GetProcAddress(lib, c"CreateDXGIFactory1".as_ptr() as *const u8);
            if create.is_null() {
                return None;
            }
            let create: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(create);
            let mut factory: *mut c_void = std::ptr::null_mut();
            if create(&IID_IDXGI_FACTORY1, &mut factory) < 0 || factory.is_null() {
                return None;
            }
            // IDXGIFactory1::EnumAdapters1 (12): adaptador 0, el que usa DirectML con device_id 0
            let enum1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 = std::mem::transmute(method(factory, 12));
            let mut adapter: *mut c_void = std::ptr::null_mut();
            let hr = enum1(factory, 0, &mut adapter);
            release(factory);
            if hr < 0 || adapter.is_null() {
                return None;
            }
            // IDXGIAdapter1::GetDesc1 (10): nombre y VRAM dedicada
            let get_desc1: unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32 = std::mem::transmute(method(adapter, 10));
            let mut desc: AdapterDesc1 = std::mem::zeroed();
            let ok_desc = get_desc1(adapter, &mut desc) >= 0;
            // IUnknown::QueryInterface (0) → IDXGIAdapter3
            let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(method(adapter, 0));
            let mut a3: *mut c_void = std::ptr::null_mut();
            let hr = qi(adapter, &IID_IDXGI_ADAPTER3, &mut a3);
            release(adapter);
            if hr < 0 || a3.is_null() || !ok_desc {
                release(a3);
                return None;
            }
            // IDXGIAdapter3::QueryVideoMemoryInfo (14), nodo 0, segmento LOCAL (VRAM dedicada)
            let qvmi: unsafe extern "system" fn(*mut c_void, u32, u32, *mut MemInfo) -> i32 = std::mem::transmute(method(a3, 14));
            let mut info = MemInfo::default();
            let hr = qvmi(a3, 0, 0, &mut info);
            release(a3);
            if hr < 0 || info.budget == 0 {
                return None;
            }
            let end = desc.description.iter().position(|&c| c == 0).unwrap_or(128);
            let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
            Some(GpuInfo {
                name: String::from_utf16_lossy(&desc.description[..end]),
                total_gb: gb(desc.dedicated_video_memory as u64),
                free_gb: gb(info.budget.saturating_sub(info.current_usage)),
            })
        }
    }
}

/// Hilos de CPU del equipo.
pub fn cpu_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// Clips en paralelo: la GPU es el cuello de botella, pasados ~4 clips los hilos solo hacen cola.
pub fn clip_workers(max_workers: Option<usize>) -> usize {
    let w = (cpu_threads() / 4).clamp(1, 4);
    match max_workers {
        Some(m) if m > 0 => w.min(m).max(1),
        _ => w,
    }
}

/// Clips en paralelo de un análisis: `--workers` manda si se da; si no, con `--background` ("Segundo
/// plano" en la ventana) [`BACKGROUND_WORKERS`], y en primer plano según el equipo ([`clip_workers`]).
pub fn analysis_workers(explicit: Option<usize>, background: bool) -> usize {
    clip_workers(explicit.or(if background { Some(BACKGROUND_WORKERS) } else { None }))
}

/// Tope de clips en paralelo con YOLO-World. Con YOLO-World x y 4 clips a la vez el driver de NVIDIA se
/// reinició 3 veces (TDR) en la RTX 5060 de 8 GB con DirectML; con 2 no pasó. El detector propio
/// (YOLO11s, mucho más ligero) aguantó 4 sin problema y no tiene este tope.
pub const WORLD_MAX_WORKERS: usize = 2;

/// Segundo plano con el detector propio: 2 clips a la vez. Con él el análisis lo limita el decodificador
/// de video (NVDEC), no los modelos: 1 clip lo tiene al ~79 % y la GPU al 44 % de media; 2 clips llegan al
/// 90 % del decodificador con la GPU al 49 % de media (p95 75 %), un 10 % más rápido en el primer análisis
/// y casi el doble al reanalizar (cuando solo se decodifican los cuadros I/P, ver
/// `sampler::saltar_b`). Medido el 2026-11-05 en 6 clips de "Videos Yo Manos Detalle".
pub const BACKGROUND_WORKERS_PROPIO: usize = 2;

/// Clips en paralelo de un análisis, decidido en un solo sitio. En orden:
/// 1. con YOLO-World (también el de boda): modo ligero (poca VRAM libre o `--gpu ligero`) = 1 clip; si
///    no, lo que piden `--workers` / `--background` con el tope del equipo ([`analysis_workers`]) y como
///    mucho [`WORLD_MAX_WORKERS`];
/// 2. con el propio, el modo ligero no cambia nada (sus modelos caben; el tope de VRAM lo pone
///    [`vram_workers`]) y en segundo plano son [`BACKGROUND_WORKERS_PROPIO`] salvo `--workers` explícito.
pub fn parallel_clips(explicit: Option<usize>, background: bool, mode: GpuMode, detector: crate::detector::DetectorKind) -> usize {
    match detector {
        crate::detector::DetectorKind::World if mode == GpuMode::Ligero => 1,
        crate::detector::DetectorKind::World => analysis_workers(explicit, background).min(WORLD_MAX_WORKERS),
        crate::detector::DetectorKind::Propio => clip_workers(explicit.or(if background { Some(BACKGROUND_WORKERS_PROPIO) } else { None })),
    }
}

/// La línea "equipo:" con el número final de clips en paralelo (`decide_gpu_mode` la escribe antes de
/// conocerlo del todo: el tope de VRAM y el detector que de verdad analiza se aplican después).
pub fn linea_con_clips(linea: &str, n: usize) -> String {
    let texto = if n == 1 { "1 clip a la vez".to_string() } else { format!("{n} clips en paralelo") };
    if linea.contains("1 clip a la vez") {
        return linea.replacen("1 clip a la vez", &texto, 1);
    }
    const FIN: &str = " clips en paralelo";
    if let (Some(i), Some(j)) = (linea.find("-> "), linea.find(FIN)) {
        if j > i {
            return format!("{}-> {texto}{}", &linea[..i], &linea[j + FIN.len()..]);
        }
    }
    linea.to_string()
}

/// VRAM que ocupa cada clip en paralelo con YOLO-World x (sus cinco modelos en DirectML: detector FP32,
/// SAM, CLIP, pose y sus memorias de trabajo). Medido el 2026-11-03 en la RTX 5060: 1 clip sube la VRAM
/// en uso de 1.3 a 3.9 GB, 2 clips a 6.5 GB.
pub const WORLD_GB_PER_WORKER: f64 = 2.6;
/// Lo mismo con el detector propio (YOLO11 pequeño en vez de YOLO-World x). Medido el 2026-11-05: el
/// ffmpeg con NVDEC de un 4K 4:2:2 ocupa ~0.96 GB (no baja con `-extra_hw_frames 0` ni saltando cuadros
/// B) y los modelos ~0.75 GB; el segundo clip sube la VRAM en uso 1.7 GB. Con él el análisis lo limita
/// el decodificador de video (NVDEC), no los modelos: en Octubre 27, 2 clips tardaron lo mismo que 4
/// (111.6 frente a 111.0 s) con 5.1 GB de VRAM en vez de 7.5.
pub const OWN_GB_PER_WORKER: f64 = 1.8;
/// VRAM que se deja libre para el sistema y las demás apps.
pub const VRAM_MARGIN_GB: f64 = 0.5;

/// Clips en paralelo que caben en la VRAM libre (sin dato de VRAM: sin tope). Si no cabe ni uno, el
/// modo ligero (menos de [`LOW_VRAM_GB`]) ya se encarga. Sin este tope, con DaVinci ocupando 2-3 GB, dos
/// clips de YOLO-World x no cabían: Windows pasaba memoria de la GPU a la RAM, todo iba más lento y
/// es la situación en la que el driver se reinicia (TDR).
pub fn vram_workers(free_gb: Option<f64>, detector: crate::detector::DetectorKind) -> usize {
    let Some(f) = free_gb else { return usize::MAX };
    // + 1e-9: que 3.6 / 1.8 cuente como 2 y no como 1.999…
    let n = match detector {
        crate::detector::DetectorKind::World => (f - VRAM_MARGIN_GB) / WORLD_GB_PER_WORKER + 1e-9,
        // con el propio los hilos comparten los modelos (`camcuts_infer::Compartido`): el primero lleva
        // todo, cada uno más solo su ffmpeg
        crate::detector::DetectorKind::Propio => 1.0 + (f - VRAM_MARGIN_GB - OWN_GB_PER_WORKER) / OWN_GB_EXTRA_WORKER + 1e-9,
    };
    (n.floor().max(1.0)) as usize
}

/// VRAM de cada clip más en paralelo con el detector propio: solo su ffmpeg con NVDEC (~0.96 GB medidos en
/// un 4K 4:2:2 de Sony), porque los modelos se comparten.
pub const OWN_GB_EXTRA_WORKER: f64 = 1.0;

/// Hilos de rayon para las medidas de imagen. Los cuadros de análisis son pequeños (960×540): con los
/// 28 hilos del i7-14700 el reparto y el despertar de hilos costaba más que el cálculo (una muestra
/// medía 18 ms con 28 hilos, 5 ms con 4, 8 ms con 1). Pasados 4 hilos no se gana nada.
pub const MEASURE_THREADS: usize = 4;

/// Fija el tamaño del grupo global de rayon (una vez por proceso; si ya existe no hace nada).
/// `RAYON_NUM_THREADS` sigue mandando si está definida.
pub fn init_cpu_pool() {
    if std::env::var_os("RAYON_NUM_THREADS").is_some() {
        return;
    }
    let n = MEASURE_THREADS.min(cpu_threads()).max(1);
    let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
}

/// Baja la prioridad del proceso (los hijos como ffmpeg la heredan), para que editar siga fluido.
pub fn run_in_background() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn SetPriorityClass(process: isize, class: u32) -> i32;
        }
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        // SAFETY: llamadas de Win32 sin punteros; el pseudo-handle del proceso actual no se cierra
        unsafe {
            SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`decide_gpu_mode`] con la GPU (sin mirar `CAMCUTS_DISPOSITIVO`, que podría estar puesta).
    fn decide(forced: &str, gpu: Option<&GpuInfo>, workers: usize) -> (GpuMode, String) {
        decide_gpu_mode_en(forced, gpu, workers, false)
    }

    /// En la CPU no se mira la VRAM: nada de modo ligero ni "GPU OCUPADA", aunque haya poca libre.
    #[test]
    fn en_cpu_no_hay_modo_ligero() {
        let poca = GpuInfo { name: "GPU".into(), total_gb: 8.0, free_gb: 1.0 };
        let (m, linea) = decide_gpu_mode_en("auto", Some(&poca), 2, true);
        assert_eq!(m, GpuMode::Normal);
        assert!(linea.starts_with("equipo: CPU (CAMCUTS_DISPOSITIVO=cpu)") && !linea.contains("OCUPADA"), "{linea}");
        assert!(linea_con_clips(&linea, 1).contains("-> 1 clip a la vez"));
        assert!(pide_cpu(Some("cpu")) && pide_cpu(Some(" CPU ")));
        assert!(!pide_cpu(None) && !pide_cpu(Some("gpu")) && !pide_cpu(Some("")));
    }

    /// El modo publicado lo puede cambiar el segundo pase (el respaldo): manda la última llamada.
    #[test]
    fn el_respaldo_puede_cambiar_el_modo() {
        // ningún otro test de este binario lee el modo publicado
        set_gpu_mode(GpuMode::Normal);
        assert_eq!(gpu_mode(), Some(GpuMode::Normal));
        set_gpu_mode(GpuMode::Ligero);
        assert_eq!(gpu_mode(), Some(GpuMode::Ligero), "el modo del respaldo sustituye al del primer pase");
        set_gpu_mode(GpuMode::Normal);
        assert_eq!(gpu_mode(), Some(GpuMode::Normal));
    }

    /// En segundo plano, un solo clip a la vez salvo que se pida `--workers`; en primer plano no cambia.
    #[test]
    fn clips_en_paralelo_segun_plano() {
        assert_eq!(analysis_workers(None, true), 1, "segundo plano sin --workers: un clip");
        assert_eq!(analysis_workers(None, false), clip_workers(None), "primer plano: según el equipo");
        // --workers explícito manda también en segundo plano (con el tope del equipo de siempre)
        assert_eq!(analysis_workers(Some(3), true), clip_workers(Some(3)));
        assert_eq!(analysis_workers(Some(3), false), clip_workers(Some(3)));
        assert_eq!(analysis_workers(Some(1), false), 1);
    }

    /// YOLO-World nunca pasa de 2 clips a la vez (TDR con 4); con él, segundo plano y modo ligero siguen
    /// en 1. El propio: 2 en segundo plano y el modo ligero no lo limita (lo hace la VRAM, aparte).
    #[test]
    fn tope_de_clips_con_yolo_world() {
        use crate::detector::DetectorKind::{Propio, World};
        let (n, l) = (GpuMode::Normal, GpuMode::Ligero);
        // YOLO-World: tope 2 aunque --workers o el equipo pidan más
        for w in [Some(4), Some(8), Some(3), None] {
            assert!(parallel_clips(w, false, n, World) <= WORLD_MAX_WORKERS, "--workers {w:?}");
            assert!(parallel_clips(w, true, n, World) <= WORLD_MAX_WORKERS, "--workers {w:?} en segundo plano");
        }
        assert_eq!(parallel_clips(Some(4), false, n, World), clip_workers(Some(4)).min(2));
        assert_eq!(parallel_clips(Some(2), false, n, World), clip_workers(Some(2)));
        assert_eq!(parallel_clips(None, false, n, World), clip_workers(None).min(2));
        assert_eq!(parallel_clips(None, true, n, World), 1);
        assert_eq!(parallel_clips(Some(4), false, l, World), 1);
        assert_eq!(parallel_clips(None, false, l, World), 1);
        for d in [World, Propio] {
            assert_eq!(parallel_clips(Some(1), false, n, d), 1);
        }
        // el propio: 2 en segundo plano, el modo ligero no lo baja, --workers manda
        assert_eq!(parallel_clips(None, true, n, Propio), clip_workers(Some(BACKGROUND_WORKERS_PROPIO)));
        assert_eq!(parallel_clips(None, true, l, Propio), clip_workers(Some(BACKGROUND_WORKERS_PROPIO)));
        assert_eq!(parallel_clips(Some(4), false, n, Propio), clip_workers(Some(4)));
        assert_eq!(parallel_clips(Some(4), false, l, Propio), clip_workers(Some(4)));
        assert_eq!(parallel_clips(None, false, n, Propio), analysis_workers(None, false));
        assert_eq!(parallel_clips(Some(3), true, n, Propio), clip_workers(Some(3)));
    }

    /// La línea "equipo:" lleva el número final de clips.
    #[test]
    fn linea_con_el_numero_final_de_clips() {
        let l = "equipo: GPU (8 GB, 3.9 GB libres) — GPU OCUPADA; modo ligero: detector propio, 1 clip a la vez, 28 hilos de CPU";
        assert!(linea_con_clips(l, 2).contains("detector propio, 2 clips en paralelo, 28 hilos"));
        assert_eq!(linea_con_clips(l, 1), l);
        let n = "equipo: GPU (8 GB, 6.6 GB libres), 28 hilos de CPU -> 3 clips en paralelo · detector propio";
        assert_eq!(linea_con_clips(n, 2), "equipo: GPU (8 GB, 6.6 GB libres), 28 hilos de CPU -> 2 clips en paralelo · detector propio");
        assert!(linea_con_clips(n, 1).contains("-> 1 clip a la vez · detector propio"));
    }

    /// Clips que caben en la VRAM libre: con el equipo libre (6.5 GB) 2 de YOLO-World; con DaVinci
    /// ocupando ~2-3 GB, uno; sin dato, sin tope.
    #[test]
    fn clips_segun_vram_libre() {
        use crate::detector::DetectorKind::{Propio, World};
        assert_eq!(vram_workers(Some(6.5), World), 2);
        assert_eq!(vram_workers(Some(4.5), World), 1);
        assert_eq!(vram_workers(Some(3.0), World), 1);
        assert_eq!(vram_workers(Some(1.0), World), 1, "nunca 0: el modo ligero decide aparte");
        // el propio comparte modelos: 1.8 GB el primero y 1.0 cada uno más
        assert_eq!(vram_workers(Some(6.5), Propio), 5);
        // con DaVinci abierto (3.3–4 GB libres) caben 2
        assert_eq!(vram_workers(Some(3.9), Propio), 2);
        assert_eq!(vram_workers(Some(3.3), Propio), 2);
        assert_eq!(vram_workers(Some(3.0), Propio), 1);
        assert_eq!(vram_workers(Some(1.0), Propio), 1);
        assert_eq!(vram_workers(None, World), usize::MAX);
    }

    #[test]
    fn modo_segun_vram() {
        let g = |free| GpuInfo { name: "GPU".into(), total_gb: 8.0, free_gb: free };
        assert_eq!(decide("auto", Some(&g(5.0)), 2).0, GpuMode::Normal);
        let (m, linea) = decide("auto", Some(&g(1.2)), 2);
        assert_eq!(m, GpuMode::Ligero);
        // la ventana pone la línea en rojo cuando contiene "OCUPADA"
        assert!(linea.starts_with("equipo:") && linea.contains("OCUPADA"));
        assert_eq!(decide("ligero", Some(&g(7.0)), 2).0, GpuMode::Ligero);
        assert_eq!(decide("normal", Some(&g(0.5)), 2).0, GpuMode::Normal);
        // sin datos de VRAM: modo normal, como Python cuando no puede medir
        assert_eq!(decide("auto", None, 2).0, GpuMode::Normal);
    }

    #[test]
    fn auto_usa_la_vram_libre_de_todo_el_equipo() {
        // lo que vio la revisión con DaVinci abierto: nvidia-smi 2.78 GB libres, DXGI (este proceso) 6.87
        let smi = parse_nvidia_smi("NVIDIA GeForce RTX 5060, 8151, 2847\n").unwrap();
        assert!((smi.free_gb - 2.78).abs() < 0.01 && (smi.total_gb - 7.96).abs() < 0.01);
        let dxgi = GpuInfo { name: "NVIDIA GeForce RTX 5060".into(), total_gb: 7.96, free_gb: 6.87 };
        let elegida = pick_gpu_info(|| Some(smi.clone()), || Some(dxgi.clone())).unwrap();
        assert_eq!(elegida, smi, "manda la cifra de todo el equipo");
        let (m, linea) = decide("auto", Some(&elegida), 2);
        assert_eq!(m, GpuMode::Ligero);
        assert!(linea.contains("OCUPADA"));
        // sin nvidia-smi (otra marca de GPU): DXGI como respaldo
        assert_eq!(pick_gpu_info(|| None, || Some(dxgi.clone())), Some(dxgi));
        assert_eq!(parse_nvidia_smi(""), None);
        assert_eq!(parse_nvidia_smi("No devices were found"), None);
    }

    #[test]
    fn gpu_info_coincide_con_nvidia_smi() {
        // en un equipo con NVIDIA, la VRAM libre que decide el modo es la de nvidia-smi (todo el equipo)
        let (Some(g), Some(s)) = (gpu_info(), nvidia_smi()) else { return };
        assert!((g.free_gb - s.free_gb).abs() < 0.5, "gpu_info {:.2} GB, nvidia-smi {:.2} GB", g.free_gb, s.free_gb);
    }

    #[test]
    fn vram_se_puede_leer() {
        // solo informativo: en un equipo sin GPU puede no haber datos
        println!("DXGI: {:?}", if cfg!(windows) { dxgi::query() } else { None });
        println!("nvidia-smi: {:?}", nvidia_smi());
    }
}
