//! Línea de comandos de CAMCUTS (port de `main()` de `__main__.py`), con las mismas opciones que Python:
//!
//! ```text
//! camcuts-cli <clips o carpetas...> --out <dir> [--preset joyas|social|todo] [--fps 4]
//! camcuts-cli --out <dir> --rescore [--preset ...]     # re-puntuar sin volver a analizar
//! camcuts-cli <clips...> --out <dir> --detector world   # YOLO-World en vez del detector propio
//! ```
//!
//! `--rescore` usa todas las cachés de esta edición, sean del detector que sean (no vuelve a analizar).
//!
//! Con el detector propio (el de por defecto) hay un respaldo automático ([`run_con_respaldo`]): los clips en los que el propio
//! casi nunca da la joya nítida se analizan otra vez con YOLO-World, y ese análisis es el que queda.
//!
//! `camcuts-core` no puede depender de `camcuts-infer` (que depende de él), así que el análisis recibe
//! una fábrica de detectores: [`run`] la acepta para que otro binario (la app o `camcuts-infer`) la
//! conecte. Sin fábrica, `--rescore` funciona completo y analizar clips nuevos avisa de que faltan modelos.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{anyhow, Result};
use clap::Parser;
use serde_json::Value;

use crate::analyse::{self, AnalyseOpts};
use crate::detector::DetectorKind;
use crate::export;
use crate::hardware;
use crate::project::{self, ClipEntry};
use crate::sampler::{self, ClipInfo};
use crate::scoring::{self, Segment};
use crate::types::Detector;

/// Crea un detector por hilo de trabajo (cada uno con sus modelos cargados).
pub type DetectorFactory = dyn Fn() -> Result<Box<dyn Detector>> + Sync;

/// Variable de entorno con la que la ventana pide el avance de cada clip: con ella, el análisis escribe
/// líneas `progreso<TAB>0..100<TAB>ruta` (`error` en vez del número si el clip falla), que la ventana
/// muestra junto a cada clip. Sin ella no salen (la salida de la terminal queda como siempre).
pub const VARIABLE_PROGRESO: &str = "CAMCUTS_PROGRESO";

/// Modo ligero con YOLO-World pedido: lo que dice la línea "equipo:" en lugar de
/// [`hardware::TEXTO_LIGERO`] (el propio sustituye al mediano, ver `detector::CLAVE_EN_LUGAR`).
pub const TEXTO_LIGERO_PROPIO: &str = "detector propio (el clásico grande no cabe y el mediano confunde las joyas)";

/// Línea de avance de un clip (ver [`VARIABLE_PROGRESO`]).
fn linea_progreso(path: &Path, valor: &str) {
    println!("progreso\t{valor}\t{}", path.display());
}

/// Opciones de la línea de comandos (mismos nombres que Python).
#[derive(Parser, Debug, Clone)]
#[command(name = "camcuts", about = "Selects al estilo Cutdown que exigen una joya nítida, quieta y visible")]
pub struct Args {
    /// Clips o carpetas de clips
    pub inputs: Vec<String>,
    /// Carpeta de resultados
    #[arg(long, required = true)]
    pub out: String,
    /// joyas (equilibrado), social (estricto, cortes ≤ 4 s) o todo (permisivo); en la edición Bodas,
    /// "bodas" es el equilibrado (y "joyas" significa lo mismo)
    #[arg(long, default_value = "joyas", value_parser = ["joyas", "bodas", "social", "todo"])]
    pub preset: String,
    /// muestras por segundo
    #[arg(long, default_value_t = 4.0)]
    pub fps: f64,
    /// solo re-puntuar desde la caché
    #[arg(long)]
    pub rescore: bool,
    /// sin hoja de contacto (en Rust no se genera hoja de contacto: se acepta por compatibilidad)
    #[arg(long)]
    pub no_sheet: bool,
    /// medir sobre la caja en vez de la silueta (más rápido)
    #[arg(long)]
    pub no_sam: bool,
    /// prioridad baja: seguir editando mientras analiza
    #[arg(long)]
    pub background: bool,
    /// archivo de texto con una ruta de clip por línea
    #[arg(long)]
    pub lista: Option<String>,
    /// no generar el video de vista previa para la app
    #[arg(long)]
    pub no_proxy: bool,
    /// JSON {ruta del clip en minúsculas: {"id": conversión, "cube": ruta .cube}}; sin entrada = "auto"
    #[arg(long)]
    pub colores: Option<String>,
    /// igual que --modo aprobados
    #[arg(long)]
    pub solo_aprobados: bool,
    /// aprobados: solo tramos · v1v2: clips completos en V1 y aprobados en V2 · marcadores: clips completos con los tramos como marcadores
    #[arg(long, default_value = "v1v2", value_parser = ["aprobados", "v1v2", "marcadores"])]
    pub modo: String,
    /// segunda mirada a 1920 px en orejas, cuello y muñecas (tomas lejanas; más lento)
    #[arg(long)]
    pub zoom: bool,
    /// solo estos tipos de plano: producto,modelo,hablando (por defecto todos)
    #[arg(long, default_value = "")]
    pub tipo: String,
    /// solo estas joyas principales: dije,anillo,arete,topo,pulsera,collar,cadena
    #[arg(long, default_value = "")]
    pub joya: String,
    /// edición Bodas: solo tramos de estos momentos (preparativos,ceremonia,votos,beso,primer baile…)
    #[arg(long, default_value = "")]
    pub momento: String,
    /// máximo de clips en paralelo (por defecto, según el equipo)
    #[arg(long)]
    pub workers: Option<usize>,
    /// fps de la línea de tiempo de DaVinci (por defecto, la del clip más común)
    #[arg(long)]
    pub timeline_fps: Option<f64>,
    /// carpeta con proxies para RAW (BRAW, R3D, NEV, ARRI...); se puede repetir
    #[arg(long = "proxy-dir")]
    pub proxy_dir: Vec<String>,
    /// auto: modo ligero si quedan menos de 4.5 GB de VRAM libre · normal: YOLO-World grande siempre ·
    /// ligero: YOLO-World mediano (en joyas lo sustituye el propio) y 1 clip a la vez; con el detector
    /// propio no cambia nada
    #[arg(long, default_value = "auto", value_parser = ["auto", "normal", "ligero"])]
    pub gpu: String,
    /// detector de joyas: propio (entrenado con el material de la marca) o world (YOLO-World, el clásico).
    /// Sin la opción manda la variable CAMCUTS_DETECTOR, y si no, el propio; las cachés sin clave (Python)
    /// son del clásico. Las cachés hechas con el otro detector se vuelven a analizar. La edición Bodas usa
    /// siempre su YOLO-World de boda.
    #[arg(long, value_parser = ["propio", "world"])]
    pub detector: Option<String>,
    /// con el detector propio, no volver a analizar con el clásico los clips en los que casi nunca da la
    /// joya nítida (el respaldo automático, ver `detector::necesita_respaldo`)
    #[arg(long)]
    pub sin_respaldo: bool,
    /// motor de la GPU: directml (el de siempre y el más rápido), cuda (pruebas: necesita `runtime_cuda`) o
    /// auto (CUDA si `runtime_cuda` está y la GPU responde, si no DirectML). Sin la opción: `CAMCUTS_MOTOR`,
    /// si no DirectML. Lo aplican los binarios con modelos.
    #[arg(long, value_parser = ["auto", "cuda", "directml"])]
    pub motor: Option<String>,
}

impl Args {
    /// Lee las opciones de la línea de comandos del proceso (sale con la ayuda o el error de clap), para
    /// que otros binarios no tengan que depender de clap.
    pub fn from_env() -> Args {
        Args::parse()
    }

    /// Detector de joyas de este análisis: `--detector`, si no `CAMCUTS_DETECTOR`, si no el propio.
    /// En la edición Bodas siempre YOLO-World (el de boda): la elección es solo de la de joyas.
    /// Quien crea los detectores (`camcuts.exe --analizar`) pone esto en `InferConfig::detector`.
    pub fn detector_kind(&self) -> DetectorKind {
        if crate::edicion::es_bodas() {
            return DetectorKind::World;
        }
        DetectorKind::resolver(self.detector.as_deref(), std::env::var(crate::detector::VARIABLE).ok().as_deref())
    }
}

/// Calibración del editor (`models/calibration.json` junto al ejecutable o `CAMCUTS_CALIBRATION`).
pub fn load_calibration() -> Option<serde_json::Map<String, Value>> {
    let path = std::env::var_os("CAMCUTS_CALIBRATION").map(PathBuf::from).or_else(|| {
        std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("models").join("calibration.json")))
    })?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?.as_object().cloned()
}

/// Lista separada por comas, sin vacíos.
fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// Preset con los filtros de la línea de comandos.
pub fn preset_for(args: &Args) -> Result<scoring::Preset> {
    // cada edición tiene su criterio equilibrado ("joyas" o "bodas"): el nombre de la otra se traduce
    let bodas = crate::edicion::es_bodas();
    let name = match args.preset.as_str() {
        "joyas" if bodas => "bodas",
        "bodas" if !bodas => "joyas",
        n => n,
    };
    let mut p = scoring::preset(name, load_calibration().as_ref()).ok_or_else(|| anyhow!("preset desconocido: {}", args.preset))?;
    if bodas && !args.momento.is_empty() {
        p.jewel_types = split_list(&args.momento); // en bodas el filtro de tipos es el momento
    }
    if !args.tipo.is_empty() {
        p.shot_types = split_list(&args.tipo);
    }
    if !args.joya.is_empty() {
        p.jewel_types = split_list(&args.joya);
    }
    Ok(p)
}

/// Clips de las entradas: los videos de cada carpeta (sin subcarpetas, ordenados) y los archivos de video.
pub fn find_clips(inputs: &[String]) -> Vec<PathBuf> {
    let mut clips = Vec::new();
    for raw in inputs {
        let p = PathBuf::from(raw);
        // "._C0001.MP4": copias de metadatos que deja macOS en discos exFAT, no son video (tampoco cuando
        // llegan sueltas: la ventana pasa los archivos uno a uno en `clips.txt`)
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&p).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|q| sampler::is_video(q) && !sampler::es_basura_mac(q)).collect()).unwrap_or_default();
            v.sort_by_key(|q| q.to_string_lossy().to_lowercase());
            clips.extend(v);
        } else if sampler::is_video(&p) && !sampler::es_basura_mac(&p) {
            clips.push(p);
        }
    }
    clips
}

/// Edición Bodas: los tres momentos más frecuentes de un clip con su parte, p. ej. "votos 60%, beso 20%".
fn resumen_momentos(ss: &[crate::jewel::Sample]) -> String {
    let mut c: Vec<(String, usize)> = Vec::new();
    for s in ss {
        let m = crate::bodas::momento(s);
        if m.is_empty() {
            continue;
        }
        match c.iter_mut().find(|x| x.0 == m) {
            Some(e) => e.1 += 1,
            None => c.push((m.to_string(), 1)),
        }
    }
    c.sort_by(|a, b| b.1.cmp(&a.1));
    let n = ss.len().max(1) as f64;
    let t: Vec<String> = c.iter().take(3).map(|(k, v)| format!("{k} {:.0}%", 100.0 * *v as f64 / n)).collect();
    if t.is_empty() { "-".into() } else { t.join(", ") }
}

/// Frecuencia más común entre los clips (empates: la primera que aparece).
pub fn most_common_rate(results: &[(ClipInfo, Vec<Segment>)]) -> Option<f64> {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for (i, _) in results {
        let r = i.fps.as_f64();
        match counts.iter_mut().find(|(x, _)| *x == r) {
            Some(e) => e.1 += 1,
            None => counts.push((r, 1)),
        }
    }
    let mut best: Option<(f64, usize)> = None;
    for c in counts {
        if best.map(|b| c.1 > b.1).unwrap_or(true) {
            best = Some(c);
        }
    }
    best.map(|b| b.0)
}

/// Re-puntúa entradas de caché: etiquetas estables, tipos forzados por el editor y tramos.
pub fn rescore_entries(entries: &mut [ClipEntry], preset: &scoring::Preset, clip_types: &serde_json::Map<String, Value>) -> Vec<(ClipInfo, Vec<Segment>)> {
    let mut results = Vec::new();
    for e in entries.iter_mut() {
        let info = e.clip_info();
        scoring::refine_jewels(&mut e.samples, e.width, e.height);
        scoring::stabilize_labels(&mut e.samples, e.per_second, 1.0);
        if let Some(forced) = clip_types.get(&e.path.to_lowercase()).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            scoring::force_type(&mut e.samples, forced);
        }
        let segs = scoring::build_segments(&e.samples, preset, e.per_second, e.duration);
        results.push((info, segs));
    }
    results
}

/// Ejecuta la línea de comandos. `factory` crea detectores para analizar clips nuevos.
pub fn run(args: Args, factory: Option<&DetectorFactory>) -> Result<()> {
    run_con_respaldo(args, factory, None)
}

/// Como [`run`]; además, con el detector propio, `respaldo` crea los YOLO-World del respaldo automático
/// (sin él, o con `--sin-respaldo`, no hay respaldo).
pub fn run_con_respaldo(args: Args, factory: Option<&DetectorFactory>, respaldo: Option<&DetectorFactory>) -> Result<()> {
    // pocos hilos de rayon: con todos los del equipo las medidas por cuadro iban ~3 veces más lentas
    hardware::init_cpu_pool();
    let out = PathBuf::from(&args.out);
    let cache = out.join("cache");
    std::fs::create_dir_all(&cache)?;
    let preset = preset_for(&args)?;

    let mut entries: Vec<ClipEntry> = Vec::new();
    if args.rescore {
        // una caché dañada se salta con un aviso: no debe tumbar la re-puntuación de todo el lote; una de la
        // otra edición también (sus medidas no sirven para los criterios de esta)
        entries.extend(project::read_entries_lenient(&cache).into_iter().filter(|e| {
            let vale = crate::edicion::cache_vigente(e);
            if !vale {
                eprintln!("aviso: {} se analizó con la otra edición de CAMCUTS; se omite", e.path);
            }
            vale
        }));
    } else {
        if args.background {
            hardware::run_in_background();
        }
        // detector de joyas de este análisis (en Bodas, siempre su YOLO-World)
        let detector = args.detector_kind();
        let color_file = args.colores.as_ref().map(PathBuf::from).unwrap_or_else(|| out.join("colores.json"));
        let colors = project::read_colors(&color_file);
        let proxy_dirs: Vec<PathBuf> = args.proxy_dir.iter().map(PathBuf::from).collect();
        let mut inputs = args.inputs.clone();
        if let Some(l) = &args.lista {
            // el Bloc de notas y PowerShell 5 guardan UTF-8 con BOM: sin quitarlo, la primera ruta no existe
            let text = std::fs::read_to_string(l)?;
            inputs.extend(text.lines().map(|x| x.trim_start_matches('\u{feff}').trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()));
        }
        let mut todo: Vec<PathBuf> = Vec::new();
        for path in find_clips(&inputs) {
            let cpath = project::cache_file(&cache, &path);
            // una caché ilegible (cortada) cuenta como clip sin analizar: se vuelve a analizar y se reescribe
            let cached = if cpath.exists() {
                project::read_entry(&cpath).map_err(|e| eprintln!("aviso: {e:#}; se vuelve a analizar")).ok()
            } else {
                None
            };
            let want = colors.get(&path.to_string_lossy().to_lowercase());
            match cached {
                // solo se reutiliza una caché de ESTA edición (las de joyas sin marca valen para joyas) y,
                // en joyas, del mismo detector (las que no dicen detector son de YOLO-World)
                // Una caché de un archivo cortado vale mientras el archivo sea el mismo (si se volvió a
                // copiar entero, cambia su tamaño o su fecha y se analiza de nuevo), y se avisa cada vez
                Some(c)
                    if project::same_color(c.color.as_ref(), want)
                        && crate::edicion::cache_vigente(&c)
                        && crate::detector::cache_vigente(&c, detector)
                        && analyse::cortado_vigente(&c) =>
                {
                    println!("[cache] {}", path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    if let Some(t) = c.extra.get(analyse::CLAVE_CORTADO).and_then(|v| v.get("hasta")).and_then(Value::as_f64) {
                        eprintln!("aviso: {} está cortado o dañado: solo se analizó hasta {t:.1} s de {:.1} s", path.display(), c.duration);
                    }
                    entries.push(c);
                }
                // clip nuevo, su conversión de color cambió desde que se analizó, la caché es de la otra
                // edición, se hizo con el otro detector o su archivo cortado cambió
                _ => todo.push(path),
            }
        }
        let lotes = Lotes { args: &args, preset: &preset, colors: &colors, proxy_dirs: &proxy_dirs, cache: &cache };
        if !todo.is_empty() {
            let factory = factory.ok_or_else(|| anyhow!("{} clip(s) sin analizar y este binario no tiene modelos: usa la app de CAMCUTS o --rescore", todo.len()))?;
            let t_all = Instant::now();
            let lote = analizar_lote(&lotes, &todo, factory, detector, false)?;
            entries.extend(lote.analysed);
            println!("análisis: {} clips en {:.0}s", todo.len(), t_all.elapsed().as_secs_f64());
            if !lote.need_proxy.is_empty() {
                let f = out.join("necesitan_proxy.txt");
                std::fs::write(&f, lote.need_proxy.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"))?;
                println!("{} clip(s) RAW sin proxy: ver {}", lote.need_proxy.len(), f.display());
            }
            let ferr = out.join("errores.txt");
            if lote.failed.is_empty() {
                let _ = std::fs::remove_file(&ferr);
            } else {
                std::fs::write(&ferr, lote.failed.iter().map(|(p, m)| format!("{}\t{m}", p.display())).collect::<Vec<_>>().join("\n"))?;
                println!("{} clip(s) no se pudieron analizar: ver {}", lote.failed.len(), ferr.display());
            }
        }
        // respaldo automático del detector propio: los clips (nuevos o de caché) en los que casi nunca da la
        // joya nítida se vuelven a analizar con YOLO-World. También cuando se pidió YOLO-World y el propio
        // lo sustituyó por falta de memoria (modo ligero). `--sin-respaldo` lo apaga
        let hubo_propio = detector == DetectorKind::Propio || entries.iter().any(|e| crate::detector::en_lugar_de(e).is_some());
        if let (true, false, Some(f)) = (hubo_propio, args.sin_respaldo, respaldo) {
            respaldar(&lotes, &mut entries, f)?;
        }
    }

    // tipos de joya que el editor fijó por clip en la app
    let clip_types = project::read_dict(&out.join("clip_types.json"));
    let results = rescore_entries(&mut entries, &preset, &clip_types);
    let timeline_fps = args.timeline_fps.or_else(|| most_common_rate(&results));
    let mode = if args.solo_aprobados { "aprobados" } else { args.modo.as_str() };
    // joyas: selects_<criterio> como Python; bodas lleva su marca para no pisar los de joyas en la misma carpeta
    let edicion = crate::edicion::actual();
    let base = edicion.nombre_selects(&preset.name);
    let otio = out.join(format!("{base}.otio"));
    let app = edicion.nombre_app();
    // Para aquí todas las cachés ya están escritas: si no se puede reemplazar la línea de tiempo o el CSV
    // (Excel tiene abierto el CSV y lo bloquea), es un aviso, no un análisis fallido. Antes salía con
    // código 1 y la ventana decía "El análisis se detuvo" con todo analizado.
    match export::write_otio(&results, &otio, &format!("{app} {}", preset.name), timeline_fps, mode, true) {
        Ok(n) => {
            println!("línea de tiempo a {:.3} fps (debe coincidir con el proyecto de DaVinci)", timeline_fps.unwrap_or(30.0));
            println!("{n} selects -> {}", otio.display());
        }
        Err(e) => eprintln!("aviso: no se pudo escribir la línea de tiempo: {e:#}"),
    }
    if let Err(e) = export::write_csv(&results, &out.join(format!("{base}.csv"))) {
        eprintln!("aviso: no se pudo escribir el CSV (¿abierto en Excel?): {e:#}");
    }
    Ok(())
}

/// Lo común a los lotes de clips de un análisis: opciones, criterio, colores y carpetas.
struct Lotes<'a> {
    args: &'a Args,
    preset: &'a scoring::Preset,
    colors: &'a serde_json::Map<String, Value>,
    proxy_dirs: &'a [PathBuf],
    cache: &'a Path,
}

/// Lo que deja un lote de clips analizados.
struct Lote {
    /// Clips analizados, en el orden de la lista pedida.
    analysed: Vec<ClipEntry>,
    /// Clips RAW sin proxy.
    need_proxy: Vec<PathBuf>,
    /// Clips que no se pudieron analizar, con el motivo.
    failed: Vec<(PathBuf, String)>,
}

/// Analiza `todo` con detectores `detector` hechos por `factory`, varios clips en paralelo según la GPU.
/// `respaldo`: es el segundo pase del respaldo automático (lo dicen la línea del equipo y la de cada clip,
/// que la ventana muestra en la barra de estado).
fn analizar_lote(l: &Lotes, todo: &[PathBuf], factory: &DetectorFactory, detector: DetectorKind, respaldo: bool) -> Result<Lote> {
    let (args, preset, colors, cache) = (l.args, l.preset, l.colors, l.cache.to_path_buf());
    let proxy_dirs = l.proxy_dirs.to_vec();
    // en segundo plano, un clip a la vez (GPU ~60 % mientras se edita) salvo --workers explícito; con
    // YOLO-World como mucho 2 (con 4 el driver de la GPU se reinició). El modo ligero se aplica abajo,
    // cuando se conoce, con la misma función
    let mut workers = hardware::parallel_clips(args.workers, args.background, hardware::GpuMode::Normal, detector);
    // modo de GPU antes de cargar ningún modelo (la VRAM libre es la que dejan las otras apps);
    // los detectores que crea `factory` lo leen con `hardware::gpu_mode()`. En la CPU
    // (`CAMCUTS_DISPOSITIVO=cpu`) la VRAM no se usa: no se mira ni pone tope
    let gpu = if hardware::en_cpu() { None } else { hardware::gpu_info() };
    // los clips que caben en la VRAM que dejan las otras apps (cada uno carga sus modelos)
    // (`--workers` explícito manda: es para pruebas)
    let cabe = if args.workers.is_some() { usize::MAX } else { hardware::vram_workers(gpu.as_ref().map(|g| g.free_gb), detector) };
    let (mode, linea) = hardware::decide_gpu_mode(&args.gpu, gpu.as_ref(), workers.min(cabe));
    // cada pase publica su modo (el del respaldo sustituye al del primero)
    hardware::set_gpu_mode(mode);
    // El detector que de verdad analiza lo dice el primer detector, ya creado con el modo publicado: con el
    // clásico pedido y poca memoria es el propio (ver `detector::CLAVE_EN_LUGAR`), pero solo si el modelo
    // propio está instalado; si no, el mediano, que va de 1 en 1. Ese detector es el del primer hilo
    let primero = factory()?;
    let efectivo = primero.kind();
    let cabe = if args.workers.is_some() { usize::MAX } else { hardware::vram_workers(gpu.as_ref().map(|g| g.free_gb), efectivo) };
    // la decisión final, con el modo ya conocido (ligero = 1 clip con el clásico) y la VRAM libre
    workers = hardware::parallel_clips(args.workers, args.background, mode, efectivo).min(cabe);
    let linea = hardware::linea_con_clips(&linea, workers);
    // la ventana muestra esta línea en la barra de estado: con el detector propio o en el respaldo, que se vea
    let linea = if respaldo {
        format!("{linea} · respaldo con el detector clásico")
    } else if detector == DetectorKind::Propio && mode == hardware::GpuMode::Ligero {
        // el modo ligero no cambia el propio: la línea no debe decir "detector mediano"
        linea.replace(hardware::TEXTO_LIGERO, "detector propio")
    } else if detector == DetectorKind::Propio {
        format!("{linea} · detector propio")
    } else if mode == hardware::GpuMode::Ligero && efectivo == DetectorKind::Propio {
        // `camcuts-infer` cambia el YOLO-World mediano por el propio (ver `detector::CLAVE_EN_LUGAR`)
        linea.replace(hardware::TEXTO_LIGERO, TEXTO_LIGERO_PROPIO)
    } else {
        linea
    };
    let primero = Mutex::new(Some(primero));
    println!("{linea}");
    // con varios clips en paralelo, los más pesados primero (por tamaño de archivo, que sale gratis): así
    // el lote no acaba con un clip largo analizándose solo mientras los demás hilos esperan. Con un solo
    // clip a la vez (modo ligero, segundo plano) eso no gana nada y retrasa el primer resultado: en una
    // carpeta de 509 clips el primero era uno de 9 minutos y la ventana pasaba minutos sin mostrar nada.
    // Los resultados se devuelven luego en el orden de siempre (el de las entradas)
    let mut by_size: Vec<(usize, PathBuf)> = todo.iter().cloned().enumerate().collect();
    if workers > 1 {
        by_size.sort_by_key(|(i, p)| (std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)), *i));
    }
    let queue = Mutex::new(by_size.into_iter());
    // avance de cada clip para la ventana (ver VARIABLE_PROGRESO)
    let avance = std::env::var_os(VARIABLE_PROGRESO).is_some();
    let done = Mutex::new((0usize, Vec::<ClipEntry>::new(), Vec::<PathBuf>::new()));
    // clips que no se pudieron analizar (archivo dañado, vacío, sin video…): no paran a los demás
    let failed = Mutex::new(Vec::<(PathBuf, String)>::new());
    let total = todo.len();
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for _ in 0..workers.min(total) {
            handles.push(scope.spawn(|| -> Result<()> {
                // el primer hilo usa el detector ya creado al decidir los clips; los demás piden el suyo
                let ya = primero.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                let mut det = match ya {
                    Some(d) => d,
                    None => factory()?,
                };
                // si este detector se creó ya en modo ligero (con el YOLO-World mediano)
                let mut det_ligero = hardware::gpu_mode() == Some(hardware::GpuMode::Ligero);
                let opts_for = |path: &Path| AnalyseOpts {
                    per_second: args.fps,
                    min_conf: preset.min_conf,
                    use_sam: !args.no_sam,
                    zoom: args.zoom,
                    proxy: None,
                    color: colors.get(&path.to_string_lossy().to_lowercase()).cloned(),
                };
                // El clip siguiente de este hilo se abre y su ffmpeg arranca en cuanto el actual
                // termina de decodificar: el arranque (ffprobe, CUDA, NVDEC, primer GOP; ~1-2 s por
                // clip) se solapa con las últimas muestras del actual. Los resultados no cambian.
                // CAMCUTS_PREFETCH=0 lo desactiva.
                let prefetch = std::env::var("CAMCUTS_PREFETCH").map(|v| v != "0").unwrap_or(true);
                type Pending = (PathBuf, std::thread::JoinHandle<Result<analyse::PreparedClip>>);
                let start_prepare = |np: &Path| {
                    let (np, cache, dirs, o, nop) = (np.to_path_buf(), cache.clone(), proxy_dirs.clone(), opts_for(np), args.no_proxy);
                    std::thread::spawn(move || analyse::prepare_clip(&np, &cache, &dirs, &o, nop, true))
                };
                let mut pending: Option<Pending> = None;
                // primer clip del hilo: su ffmpeg arranca ya y, mientras, se abren y compilan los
                // modelos (todos a la vez); antes se abrían uno a uno en las primeras muestras y la GPU
                // pasaba los primeros ~8 s casi parada. Con el tamaño de cuadro de ese clip (apaisado o
                // vertical: cada orientación compila su propia forma)
                let first = queue.lock().unwrap().next();
                if let Some((_, p)) = first {
                    pending = Some((p.clone(), start_prepare(&p)));
                    let (w, h) = sampler::open_clip(&p, &proxy_dirs).ok().flatten().map(|i| sampler::scaled_size(&i)).unwrap_or((sampler::LONG_EDGE, sampler::LONG_EDGE * 9 / 16));
                    // un fallo aquí no para nada: cada modelo se vuelve a abrir en su primer uso
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| det.prepare(w, h)));
                }
                loop {
                    let (path, ready) = match pending.take() {
                        Some((p, h)) => {
                            let r = h.join().unwrap_or_else(|_| Err(anyhow!("fallo interno al abrir el clip")));
                            (p, Some(r))
                        }
                        None => match queue.lock().unwrap().next() {
                            Some((_, p)) => (p, None),
                            None => return Ok(()),
                        },
                    };
                    // solo si quedan clips para los demás hilos (con pocos clips, mejor en paralelo)
                    let next: Option<PathBuf> = if prefetch {
                        let mut q = queue.lock().unwrap();
                        if q.len() >= workers { q.next().map(|(_, p)| p) } else { None }
                    } else {
                        None
                    };
                    let mut spawned = None;
                    let t0 = Instant::now();
                    let opts = opts_for(&path);
                    if avance {
                        linea_progreso(&path, "0");
                    }
                    // como mucho 99 mientras se decodifica (la voz y la caché aún faltan); el 100 sale al
                    // terminar el clip. Una línea por punto, no por muestra
                    let mut ultimo = 0u32;
                    let al_avanzar = |f: f64| {
                        let v = ((f * 100.0) as u32).min(99);
                        if avance && v > ultimo {
                            ultimo = v;
                            linea_progreso(&path, &v.to_string());
                        }
                    };
                    // un clip que falla (o que hace fallar a un modelo) se anota y se sigue con el
                    // siguiente; tras un pánico el detector se vuelve a crear por si quedó a medias
                    let r = {
                        let mut on_eof = || {
                            if spawned.is_none() {
                                spawned = next.as_deref().map(start_prepare);
                            }
                        };
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
                            let p = match ready {
                                Some(r) => r?,
                                None => analyse::prepare_clip(&path, &cache, &proxy_dirs, &opts, args.no_proxy, false)?,
                            };
                            analyse::analyse_prepared(p, &cache, det.as_mut(), &opts, None, al_avanzar, &mut on_eof)
                        }))
                    };
                    // el siguiente: si no se lanzó al terminar de decodificar (error, RAW…), ahora
                    if let Some(np) = next {
                        let h = spawned.take().unwrap_or_else(|| start_prepare(&np));
                        pending = Some((np, h));
                    }
                    let r = match r {
                        Ok(r) => r,
                        Err(p) => {
                            det = factory()?;
                            det_ligero = hardware::gpu_mode() == Some(hardware::GpuMode::Ligero);
                            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                            Err(anyhow!("fallo interno: {msg}"))
                        }
                    };
                    // La GPU se quedó corta a mitad del lote (DaVinci u otra app ocupó más VRAM): Windows pasa
                    // memoria de los modelos a la RAM y todo va decenas de veces más lento. Desde el clip
                    // siguiente, el detector mediano (modo ligero), también en los demás hilos. Los
                    // resultados del clip lento valen (solo tardó más)
                    // en la CPU (`CAMCUTS_DISPOSITIVO=cpu`) ~2 s por muestra es lo normal: no es la GPU sin
                    // memoria y bajar al mediano solo empeoraría el análisis (Bodas lo documenta así)
                    if !det_ligero && det.kind() == DetectorKind::World && !hardware::en_cpu() {
                        // `CAMCUTS_LENTO_S`: otro umbral (pruebas: un umbral mínimo fuerza el cambio)
                        let umbral = std::env::var("CAMCUTS_LENTO_S").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(hardware::LENTO_S_POR_MUESTRA);
                        let lento = match &r {
                            Ok(Some((e, _))) => {
                                let por_muestra = t0.elapsed().as_secs_f64() / e.samples.len().max(1) as f64;
                                (e.samples.len() >= hardware::LENTO_MIN_MUESTRAS && por_muestra > umbral).then_some(por_muestra)
                            }
                            _ => None,
                        };
                        if let Some(s) = lento {
                            if hardware::bajar_a_ligero() {
                                // en el respaldo sigue el clásico mediano; si no, el propio lo sustituye
                                let sigue = if respaldo || crate::edicion::es_bodas() { "el detector mediano" } else { "el detector propio" };
                                println!(
                                    "equipo: GPU OCUPADA — {} fue a {s:.2} s por muestra: la memoria de la GPU no alcanza (¿DaVinci?); se sigue con {sigue} (modo ligero)",
                                    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                                );
                            }
                        }
                        // este hilo o otro (o el detector, al no poder abrir el grande) pasó a modo ligero
                        if hardware::gpu_mode() == Some(hardware::GpuMode::Ligero) {
                            det = factory()?;
                            det_ligero = true;
                        }
                    }
                    let mut d = done.lock().unwrap();
                    d.0 += 1;
                    let i = d.0;
                    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    // en el respaldo, que se vea en la ventana que es el segundo pase de ese clip
                    let name = if respaldo { format!("{name} (respaldo con el clásico)") } else { name };
                    // la ventana deja de mostrar el porcentaje del clip: terminado (100) o con error
                    if avance {
                        linea_progreso(&path, if matches!(r, Ok(Some(_))) { "100" } else { "error" });
                    }
                    let r = match r {
                        Ok(r) => r,
                        Err(e) => {
                            let msg = format!("{e:#}").replace(['\n', '\r'], " ");
                            println!("[{i}/{total}] [error] {name}: {msg}");
                            failed.lock().unwrap().push((path.clone(), msg));
                            continue;
                        }
                    };
                    match r {
                        None => {
                            println!("[{i}/{total}] [proxy?] {name}: formato RAW sin proxy encontrado");
                            d.2.push(path);
                        }
                        Some((e, st)) => {
                            let ss = &e.samples;
                            let n = ss.len().max(1) as f64;
                            let pct = |f: &dyn Fn(&crate::jewel::Sample) -> bool| 100.0 * ss.iter().filter(|s| f(s)).count() as f64 / n;
                            let mut via = e.decode_path.as_ref().map(|p| format!(" (vía proxy {})", Path::new(p).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())).unwrap_or_default();
                            if let Some(t) = st.cut_at {
                                // archivo cortado: se analizó lo que se pudo leer, y se dice
                                via.push_str(&format!(" [archivo cortado: solo hasta {t:.1} s de {:.1} s]", e.duration));
                                failed.lock().unwrap().push((path.clone(), format!("archivo cortado o dañado: solo se pudo leer hasta {t:.1} s de {:.1} s", e.duration)));
                            }
                            if crate::edicion::es_bodas() {
                                println!(
                                    "[{i}/{total}] {name}{via}: {} muestras, sujeto {:.0}%, cara {:.0}%, hablando {:.0}%, {:.1}s ({}, {:.0}% quietas) · momentos: {}",
                                    ss.len(),
                                    pct(&|s| s.jewel),
                                    pct(&|s| s.face),
                                    pct(&|s| s.speaking.unwrap_or(false)),
                                    t0.elapsed().as_secs_f64(),
                                    e.decode.to_uppercase(),
                                    100.0 * e.reused as f64 / n,
                                    resumen_momentos(ss)
                                );
                                d.1.push(e);
                                continue;
                            }
                            println!(
                                "[{i}/{total}] {name}{via}: {} muestras, joya {:.0}%, persona {:.0}%, hablando {:.0}%, {:.1}s ({}, {:.0}% quietas)",
                                ss.len(),
                                pct(&|s| s.jewel),
                                pct(&|s| s.person),
                                pct(&|s| s.speaking.unwrap_or(false)),
                                t0.elapsed().as_secs_f64(),
                                e.decode.to_uppercase(),
                                100.0 * e.reused as f64 / n
                            );
                            d.1.push(e);
                        }
                    }
                }
            }));
        }
        for h in handles {
            h.join().map_err(|_| anyhow!("un hilo de análisis falló"))??;
        }
        Ok(())
    })?;
    let (_, mut analysed, need_proxy) = done.into_inner().unwrap();
    // en el orden de las entradas, no en el que fueron terminando los hilos
    let pos = |e: &ClipEntry| todo.iter().position(|p| p.to_string_lossy() == e.path.as_str()).unwrap_or(usize::MAX);
    analysed.sort_by_key(pos);
    Ok(Lote { analysed, need_proxy, failed: failed.into_inner().unwrap() })
}

/// Respaldo automático del detector propio: los clips de `entries` en los que casi nunca da la joya nítida
/// ([`crate::detector::necesita_respaldo`]) se vuelven a analizar con YOLO-World (detectores de `factory`).
/// Su análisis sustituye al del propio y su caché queda marcada como respaldo, que vale para el propio: la
/// próxima vez se reutiliza y no se repite nada. Si el respaldo de un clip falla, se queda el del propio.
fn respaldar(l: &Lotes, entries: &mut [ClipEntry], factory: &DetectorFactory) -> Result<()> {
    let rutas: Vec<PathBuf> = entries.iter().filter(|e| crate::detector::necesita_respaldo(e)).map(|e| PathBuf::from(&e.path)).filter(|p| p.exists()).collect();
    if rutas.is_empty() {
        return Ok(());
    }
    let nombres: Vec<String> = rutas.iter().map(|p| p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).collect();
    println!("respaldo: con el detector propio la joya casi nunca sale nítida en {}; se analiza(n) también con el clásico", nombres.join(", "));
    let t = Instant::now();
    let lote = analizar_lote(l, &rutas, factory, DetectorKind::World, true)?;
    // un archivo cortado sale analizado (lo que se pudo leer) y también en `failed` (para errores.txt):
    // su respaldo sí se hizo y es el que queda
    let hechos: Vec<String> = lote.analysed.iter().map(|e| e.path.clone()).collect();
    for mut e in lote.analysed {
        e.extra.insert(crate::detector::CLAVE_RESPALDO.into(), Value::String(DetectorKind::Propio.as_str().into()));
        // sin la marca, el clip se volvería a analizar con el propio la próxima vez (y otra vez el respaldo)
        if let Err(err) = project::write_entry(&project::cache_file(l.cache, Path::new(&e.path)), &e) {
            eprintln!("aviso: no se pudo marcar el respaldo de {}: {err:#}", e.path);
        }
        if let Some(x) = entries.iter_mut().find(|x| x.path == e.path) {
            *x = e;
        }
    }
    for (p, m) in &lote.failed {
        if hechos.iter().any(|h| h.as_str() == p.to_string_lossy()) {
            eprintln!("aviso: respaldo de {} hecho con lo que se pudo leer ({m})", p.display());
        } else {
            eprintln!("aviso: el respaldo de {} falló ({m}); se queda el análisis del detector propio", p.display());
        }
    }
    println!("respaldo: {} clip(s) en {:.0}s", rutas.len(), t.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Con el CSV de selects abierto en Excel (bloqueado), el análisis termina bien: el CSV nuevo queda
    /// en `.csv.tmp` y sale un aviso, no el código 1 de "El análisis se detuvo".
    #[cfg(windows)]
    #[test]
    fn csv_bloqueado_no_tumba_el_analisis() {
        use std::os::windows::fs::OpenOptionsExt;
        let out = std::env::temp_dir().join(format!("camcuts_csv_bloqueado_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(out.join("cache")).unwrap();
        let args = Args::try_parse_from(["camcuts", "--out", out.to_str().unwrap(), "--rescore"]).unwrap();
        let base = crate::edicion::actual().nombre_selects(&preset_for(&args).unwrap().name);
        let csv = out.join(format!("{base}.csv"));
        std::fs::write(&csv, "viejo").unwrap();
        // como Excel: abierto sin compartir (ni leer, ni escribir, ni borrar/renombrar)
        let bloqueo = std::fs::OpenOptions::new().read(true).share_mode(0).open(&csv).unwrap();
        run(args, None).expect("un CSV bloqueado es un aviso, no un error");
        assert!(out.join(format!("{base}.csv.tmp")).exists(), "el CSV nuevo queda al lado");
        assert!(out.join(format!("{base}.otio")).exists(), "la línea de tiempo sí se escribe");
        drop(bloqueo);
        assert_eq!(std::fs::read_to_string(&csv).unwrap(), "viejo");
        let _ = std::fs::remove_dir_all(&out);
    }
}
