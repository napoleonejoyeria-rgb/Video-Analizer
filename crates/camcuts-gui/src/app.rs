//! Estado y lógica de la ventana (el equivalente de la clase `App` de `gui.py`, sin el dibujo).
//!
//! Aquí viven el material, la lista de clips, el criterio, el proyecto, el visor y los trabajos de
//! fondo. El dibujo de cada panel está en `paneles/`. Todo lo que se calcula tras el análisis
//! (puntuación, tramos, filtros, pesos) se recalcula al instante sobre la caché, como en Python.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use camcuts_core::detector::DetectorKind;
use egui::{Color32, TextureHandle, TextureOptions};
use serde_json::{json, Map, Value};

use crate::analisis::{self, Analisis, Mensaje, TrabajoProxies};
use crate::datos::{self, clave, fmt_tiempo, Meta};
use crate::motor::{self, ColorSpec, Criterio, Entrada, Tramo};
use crate::proyecto;
use crate::tema::{APAGADO, ORO, ROJO};
use crate::video::{self, Grado, Miniaturas, Pedido, PedidoMini, Visor};

/// Cadencias que se ofrecen para la línea de tiempo de DaVinci.
pub const FPS: [&str; 9] = ["Auto", "23.976", "24", "25", "29.97", "30", "50", "59.94", "60"];

/// Un clip de la lista: su ruta, lo que dice la caché (si ya se analizó) y lo calculado con el criterio.
pub struct Clip {
    pub ruta: PathBuf,
    pub clave: String,
    pub entrada: Option<Entrada>,
    pub meta: Meta,
    /// Curva (t, puntuación) tras el suavizado.
    pub curva: Vec<(f64, f64)>,
    pub tramos: Vec<Tramo>,
    pub aprobado: f64,
}

impl Clip {
    /// Duración: la de la caché si está analizado, si no la de ffprobe.
    pub fn dur(&self) -> f64 {
        self.entrada.as_ref().map(|e| e.duracion).unwrap_or_else(|| self.meta.duration.unwrap_or(0.0))
    }
    /// Nombre del archivo.
    pub fn nombre(&self) -> String {
        self.ruta.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    }
    /// Nombre sin extensión (para el mapa y el recibo).
    pub fn base(&self) -> String {
        self.ruta.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    }
    /// Puntuación media del clip (0..1), si está analizado.
    pub fn media(&self) -> Option<f64> {
        (!self.curva.is_empty()).then(|| self.curva.iter().map(|c| c.1).sum::<f64>() / self.curva.len() as f64)
    }
    /// Parte del clip con joya visible.
    pub fn parte_joya(&self) -> f64 {
        self.entrada.as_ref().map(|e| e.parte_joya()).unwrap_or(0.0)
    }
}

/// Columnas por las que se puede ordenar (las de la tabla de Python).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Columna {
    Ruta,
    Clip,
    Estado,
    Puntuacion,
    Duracion,
    Joya,
    Aprobado,
    Tramos,
}

impl Columna {
    pub const TODAS: [(Columna, &'static str); 8] = [
        (Columna::Ruta, "# (carpeta)"),
        (Columna::Clip, "Clip"),
        (Columna::Estado, "Estado"),
        (Columna::Puntuacion, "Puntuación"),
        (Columna::Duracion, "Duración"),
        (Columna::Joya, "Con joya"),
        (Columna::Aprobado, "Aprobado"),
        (Columna::Tramos, "Tramos"),
    ];
}

/// Qué paneles se ven (se ocultan con los botones del riel o con Ctrl+1…5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paneles {
    pub material: bool,
    pub pesos: bool,
    pub color: bool,
    pub tiempo: bool,
    pub mapa: bool,
}

/// Un aviso modal (resultado de exportar, errores, confirmaciones).
pub struct Aviso {
    pub titulo: String,
    pub texto: String,
    /// Si es una confirmación de salida con análisis en curso.
    pub confirmar_salida: bool,
}

/// Clave del estado guardado con la versión del detector elegido (ver `aplicar_estado`).
pub const CLAVE_DETECTOR_V: &str = "detector_v";
/// Versión 2: el propio es el predeterminado (2026-11-05); un "detector" guardado desde entonces se respeta.
pub const DETECTOR_V: u64 = 2;

/// Toda la ventana.
pub struct App {
    // --- material y resultados ---
    pub fuentes: Vec<String>,
    pub recursivo: bool,
    pub out: String,
    pub out_auto: bool,
    pub segundo_plano: bool,
    pub zoom_joyas: bool,
    /// Edición de joyas: detector del análisis ("Propio", por defecto, o "Clásico" = YOLO-World).
    /// Se guarda en el proyecto (`detector`) y va al análisis como `--detector`. En Bodas no se usa: esa
    /// edición tiene su propio YOLO-World de boda.
    pub detector: DetectorKind,
    // --- clips ---
    pub clips: Vec<Clip>,
    pub actual: Option<usize>,
    /// Claves de los clips seleccionados (Ctrl/Shift+clic), para dar color a varios a la vez.
    pub seleccion: BTreeSet<String>,
    /// Clip del último clic sin Mayús (su clave, no su fila: la lista se reordena y se rehace).
    pub ancla: Option<String>,
    pub orden: Option<(Columna, bool)>,
    pub tipos: BTreeMap<String, String>,
    tipos_proyecto: Option<BTreeMap<String, String>>,
    pub colores: BTreeMap<String, ColorSpec>,
    // --- criterio y exportación ---
    pub criterio: Criterio,
    pub fps: String,
    pub modo: String,
    // --- visor ---
    pub t: f64,
    pub reproduciendo: bool,
    pub ver_joyas: bool,
    pub ver_color: bool,
    pub tex_visor: Option<TextureHandle>,
    pub tam_cuadro: [u32; 2],
    /// La última búsqueda no dio cuadro (clip sin conexión y sin proxy, error de ffmpeg, más allá del
    /// final): el visor no muestra imagen y avisa "No se puede mostrar este cuadro".
    pub sin_cuadro: bool,
    serie_cuadro: u64,
    t_pedido: Option<(f64, Instant)>,
    // --- línea de tiempo / mapa ---
    pub alto_tiempo: f32,
    pub zoom_mapa: f64,
    pub t0_mapa: f64,
    pub paneles: Paneles,
    /// Tarjetas de título y de recibo plegadas (dejan más sitio a la lista de clips).
    pub titulo_plegado: bool,
    pub recibo_plegado: bool,
    /// Filas de la lista de clips que caben a la vista (lo usan las pruebas sin pantalla).
    pub filas_visibles: usize,
    // --- estado visible ---
    pub estado: String,
    pub estado_color: Color32,
    pub progreso: Option<(usize, usize)>,
    /// Avance del análisis en curso por clip (clave -> 0..100, `None` = falló), de las líneas `progreso`
    /// de la CLI. La lista lo muestra junto a cada clip; se vacía al terminar el análisis.
    pub avance_clips: HashMap<String, Option<u8>>,
    pub resumen: String,
    pub totales: String,
    pub etiqueta_proyecto: String,
    pub ruta_proyecto: Option<PathBuf>,
    /// El proyecto abierto es de la otra edición (joyas / bodas): se ve, pero el autoguardado no lo
    /// reescribe; "Guardar como" guarda una copia de esta edición.
    pub solo_lectura: bool,
    pub aviso: Option<Aviso>,
    // --- trabajos de fondo ---
    pub ctx: egui::Context,
    visor: Visor,
    miniaturas: Miniaturas,
    pub texturas_mini: HashMap<String, TextureHandle>,
    minis_pedidas: HashSet<String>,
    tx: Sender<Mensaje>,
    rx: Receiver<Mensaje>,
    pub analisis: Option<Analisis>,
    proxies: Option<TrabajoProxies>,
    sondeando: bool,
    /// Metadatos sondeados en esta sesión (clave -> meta). `cargar_clips` los mezcla con `clips_meta.json`:
    /// si la carpeta de resultados no deja escribir, sin esto cada recarga veía los clips sin metadatos y
    /// lanzaba otro sondeo, sin fin.
    metas_sesion: BTreeMap<String, datos::Meta>,
    /// Ya se avisó de que no se puede escribir `clips_meta.json` (se avisa una sola vez).
    aviso_metas: bool,
    /// Conversión que aplica el visor por clip (clave -> (color elegido, conversión)), para la píldora del
    /// transporte: leerla (XML de Sony, ffprobe) en cada cuadro sería caro. Se rehace si cambia el color.
    color_visor: HashMap<String, (String, String)>,
    // --- guardado y recálculo diferidos ---
    cargando: bool,
    sucio: Option<Instant>,
    puntuar_pendiente: bool,
    /// Si es `false` no se escriben preferencias (pruebas sin pantalla).
    pub persistir: bool,
    /// Edición Bodas: modo claro (por defecto) u oscuro de la interfaz.
    pub modo_claro: bool,
    /// Edición Bodas: carriles de momentos y momentos de cada clip (se rehace al cambiar los clips).
    pub cache_bodas: crate::bodas_ui::CacheBodas,
    /// Sube cada vez que se releen los clips y sus cachés (`cargar_clips`): lo que se calcula a partir
    /// de ellos (los carriles de momentos) se rehace aunque solo cambie una muestra.
    pub revision_clips: u64,
}

impl App {
    /// Crea la ventana vacía (sin abrir preferencias ni proyectos).
    pub fn nuevo(ctx: &egui::Context) -> Self {
        if motor::bodas() {
            crate::bodas_tema::instalar(ctx, true);
        } else {
            crate::tema::instalar(ctx);
        }
        let (tx, rx) = channel();
        let c1 = ctx.clone();
        let c2 = ctx.clone();
        Self {
            fuentes: Vec::new(),
            recursivo: true,
            out: String::new(),
            out_auto: true,
            segundo_plano: true,
            zoom_joyas: false,
            detector: DetectorKind::default(),
            clips: Vec::new(),
            actual: None,
            seleccion: BTreeSet::new(),
            ancla: None,
            orden: None,
            tipos: BTreeMap::new(),
            tipos_proyecto: None,
            colores: BTreeMap::new(),
            criterio: Criterio::base(motor::criterio_defecto()),
            fps: "Auto".into(),
            modo: "v1v2".into(),
            t: 0.0,
            reproduciendo: false,
            ver_joyas: true,
            ver_color: true,
            tex_visor: None,
            tam_cuadro: [0, 0],
            sin_cuadro: false,
            serie_cuadro: 0,
            t_pedido: None,
            // en Bodas la línea de tiempo lleva un carril por momento: empieza más alta
            alto_tiempo: if motor::bodas() { 250.0 } else { 200.0 },
            zoom_mapa: 1.0,
            t0_mapa: 0.0,
            paneles: Paneles { material: true, pesos: true, color: false, tiempo: true, mapa: true },
            // plegadas por defecto: la lista de clips manda (~25 filas a 1440×900)
            titulo_plegado: true,
            recibo_plegado: true,
            filas_visibles: 0,
            estado: "Agrega material y pulsa Analizar.".into(),
            estado_color: APAGADO,
            progreso: None,
            avance_clips: HashMap::new(),
            resumen: String::new(),
            totales: String::new(),
            etiqueta_proyecto: String::new(),
            ruta_proyecto: None,
            solo_lectura: false,
            aviso: None,
            ctx: ctx.clone(),
            visor: Visor::nuevo(move || c1.request_repaint()),
            miniaturas: Miniaturas::nuevo(move || c2.request_repaint()),
            texturas_mini: HashMap::new(),
            minis_pedidas: HashSet::new(),
            tx,
            rx,
            analisis: None,
            proxies: None,
            sondeando: false,
            metas_sesion: BTreeMap::new(),
            aviso_metas: false,
            color_visor: HashMap::new(),
            cargando: false,
            sucio: None,
            puntuar_pendiente: false,
            persistir: true,
            modo_claro: true,
            cache_bodas: Default::default(),
            revision_clips: 0,
        }
    }

    /// Abre lo último: el proyecto de la sesión anterior o las preferencias guardadas.
    pub fn cargar_preferencias(&mut self) {
        let d = proyecto::leer_preferencias();
        if let Some(ultimo) = d.get("last_project").and_then(Value::as_str) {
            if Path::new(ultimo).exists() {
                self.abrir_proyecto_al_arrancar(Path::new(ultimo));
                return;
            }
        }
        self.aplicar_estado(&d);
    }

    fn despertar(&self) -> impl Fn() + Send + Sync + 'static {
        let c = self.ctx.clone();
        move || c.request_repaint()
    }

    // =============================================================================================
    // Criterio y puntuación
    // =============================================================================================

    /// Restablece los valores del criterio elegido (igual que `_apply_criterion`).
    pub fn aplicar_criterio(&mut self, nombre: &str) {
        let joyas = std::mem::take(&mut self.criterio.joyas);
        let planos = std::mem::take(&mut self.criterio.planos);
        self.criterio = Criterio::base(nombre);
        self.criterio.joyas = joyas;
        self.criterio.planos = planos;
        self.programar();
    }

    /// Algo del criterio cambió: recalcular en el próximo cuadro y autoguardar.
    pub fn programar(&mut self) {
        if self.cargando {
            return;
        }
        self.marcar_sucio();
        self.puntuar_pendiente = true;
    }

    /// Recalcula curvas y tramos de todos los clips (igual que `_rescore`).
    pub fn puntuar(&mut self) {
        self.puntuar_pendiente = false;
        let p = self.criterio.preset();
        let (mut total, mut aprobado, mut joya_s, mut an_total) = (0.0, 0.0, 0.0, 0.0);
        let (mut n, mut analizados) = (0usize, 0usize);
        for c in &mut self.clips {
            if let Some(e) = &c.entrada {
                let (curva, tramos) = motor::puntuar(e, &p);
                c.aprobado = tramos.iter().map(|s| s.fin - s.inicio).sum();
                c.curva = curva;
                c.tramos = tramos;
                aprobado += c.aprobado;
                joya_s += e.parte_joya() * e.duracion;
                an_total += e.duracion;
                n += c.tramos.len();
                analizados += 1;
            }
            total += c.dur();
        }
        let pct = if total > 0.0 { 100.0 * aprobado / total } else { 0.0 };
        self.resumen = format!("{n} tramos · {} aprobados ({pct:.0}% del material)", fmt_tiempo(aprobado));
        self.totales = format!(
            "{} clips ({analizados} analizados) · {} · {} en {:.0}% · {n} tramos · {} aprobados ({pct:.0}%)",
            self.clips.len(),
            fmt_tiempo(total),
            if motor::bodas() { "sujeto" } else { "joya" },
            if an_total > 0.0 { 100.0 * joya_s / an_total } else { 0.0 },
            fmt_tiempo(aprobado)
        );
    }

    /// Cadencia de la línea de tiempo: la elegida o la más común entre los clips analizados.
    pub fn cadencia(&self) -> f64 {
        match self.fps.as_str() {
            "Auto" => {
                let mut cuenta: Vec<(f64, usize)> = Vec::new();
                for c in self.clips.iter().filter_map(|c| c.entrada.as_ref()) {
                    let r = c.cadencia();
                    match cuenta.iter_mut().find(|(x, _)| (*x - r).abs() < 1e-6) {
                        Some(e) => e.1 += 1,
                        None => cuenta.push((r, 1)),
                    }
                }
                cuenta.into_iter().max_by_key(|e| e.1).map(|e| e.0).unwrap_or(30.0)
            }
            "23.976" => 24000.0 / 1001.0,
            "29.97" => 30000.0 / 1001.0,
            "59.94" => 60000.0 / 1001.0,
            o => o.parse().unwrap_or(30.0),
        }
    }

    // =============================================================================================
    // Clips
    // =============================================================================================

    /// Conversión de color asignada a un clip (por defecto, automática).
    pub fn color_de(&self, c: &Clip) -> ColorSpec {
        self.colores.get(&c.clave).cloned().unwrap_or_else(ColorSpec::auto)
    }

    /// Texto de la píldora de color del transporte para el clip actual. Dice la conversión que de verdad
    /// aplica el visor (`motor::conversion_visor`): con "auto" y un perfil de cámara conocido, p. ej.
    /// "S-Log3 → 709 (auto)"; antes decía "LOG → contraste" o "Automático" mientras el visor aplicaba el
    /// perfil. `automatico` es el texto de cada edición para "auto" sin perfil.
    pub fn texto_color_actual(&mut self, automatico: &str) -> String {
        let Some(c) = self.actual.and_then(|i| self.clips.get(i)) else { return "Color".to_owned() };
        let s = self.color_de(c);
        let es_log = c.entrada.as_ref().map(|e| e.es_log()).unwrap_or(false);
        let (clave, ruta) = (c.clave.clone(), c.ruta.clone());
        let conv = match self.color_visor.get(&clave) {
            Some((id, conv)) if *id == s.id => conv.clone(),
            _ => {
                let conv = motor::conversion_visor(&s, &ruta);
                self.color_visor.insert(clave, (s.id.clone(), conv.clone()));
                conv
            }
        };
        motor::texto_pildora_color(&s.id, &conv, es_log, automatico)
    }

    /// Qué enseña la columna de la puntuación de un clip mientras hay un análisis: `Some(Some(v))` = su
    /// porcentaje (0 = en cola), `Some(None)` = falló, `None` = lo de siempre (no hay análisis, o el clip ya
    /// terminó y su caché está leída). Un mapa de avance con datos también cuenta como análisis en curso
    /// (se vacía al terminar), así las pruebas sin pantalla lo pueden simular.
    pub fn avance_de(&self, c: &Clip) -> Option<Option<u8>> {
        if self.analisis.is_none() && self.avance_clips.is_empty() {
            return None;
        }
        match self.avance_clips.get(&c.clave) {
            Some(Some(100)) if c.entrada.is_some() => None,
            Some(v) => Some(*v),
            None if c.entrada.is_none() => Some(Some(0)),
            None => None,
        }
    }

    /// Texto de la columna "Estado" (igual que `_row` de Python).
    pub fn estado_de(&self, c: &Clip) -> String {
        match &c.entrada {
            Some(e) => {
                let elegido = self.color_de(c);
                if !ColorSpec::igual(e.color.as_ref(), Some(&elegido)) {
                    "Reanalizar · color".into()
                } else if !motor::bodas() && !camcuts_core::detector::cache_vigente(&e.nucleo, self.detector) {
                    // analizado con el otro detector de joyas: Analizar lo rehace con el elegido (la misma
                    // regla que la CLI: un respaldo del clásico vale para el propio y no se rehace)
                    "Reanalizar · detector".into()
                } else {
                    let cid = e.color.as_ref().map(|s| s.id.clone()).unwrap_or_else(|| "auto".into());
                    if cid == "auto" {
                        if e.es_log() { "LOG · analizado".into() } else { "Analizado".into() }
                    } else {
                        format!("{} · analizado", motor::color_corto(&cid))
                    }
                }
            }
            None if self.analisis.is_some() => "Analizando…".into(),
            None if c.meta.error == Some(true) => "Sin conexión".into(),
            None => "Pendiente".into(),
        }
    }

    /// Relee la carpeta de resultados y arma la lista (igual que `_load_clips`).
    pub fn cargar_clips(&mut self) {
        let out = (!self.out.is_empty()).then(|| PathBuf::from(&self.out));
        let mantener = self.actual.and_then(|i| self.clips.get(i)).map(|c| c.clave.clone());
        let mut metas = out.as_deref().map(datos::leer_metas).unwrap_or_default();
        // lo sondeado en esta sesión vale aunque no se haya podido guardar en el disco
        metas.extend(self.metas_sesion.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.tipos = out
            .as_deref()
            .and_then(|o| std::fs::read_to_string(o.join("clip_types.json")).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if let Some(t) = self.tipos_proyecto.take() {
            self.tipos.extend(t);
        }
        let mut entradas = out.as_deref().map(|o| motor::cargar_cache(o, &self.tipos)).unwrap_or_default();
        // la lista muestra el material (analizado o no); sin material, lo que tenga la carpeta
        let sin_conexion: Vec<String> = self.fuentes.iter().filter(|s| !Path::new(s.as_str()).exists()).cloned().collect();
        let rutas: Vec<PathBuf> = if !self.fuentes.is_empty() {
            let en_linea: Vec<String> = self.fuentes.iter().filter(|s| Path::new(s.as_str()).exists()).cloned().collect();
            let mut v = datos::reunir_clips(&en_linea, self.recursivo);
            // material sin conexión: sus clips ya analizados siguen en la lista (desde la caché). Se
            // compara por componentes enteros: `D:\Boda` no recoge los clips de `D:\Boda 2\…`
            for s in &sin_conexion {
                let base = Path::new(s.as_str());
                let mut extra: Vec<PathBuf> = entradas.values().map(|e| e.ruta.clone()).filter(|r| datos::dentro_de(r, base, self.recursivo)).collect();
                extra.sort();
                v.extend(extra);
            }
            // fuentes anidadas (`D:\Boda` y `D:\Boda\Cam1`) darían el mismo clip dos veces: una fila por clip
            let mut vistos = HashSet::new();
            v.retain(|r| vistos.insert(clave(r)));
            v
        } else {
            let mut v: Vec<PathBuf> = entradas.values().map(|e| e.ruta.clone()).collect();
            v.sort();
            v
        };
        // las cachés se acaban de releer: lo calculado a partir de ellas se rehace
        self.revision_clips = self.revision_clips.wrapping_add(1);
        self.clips = rutas
            .into_iter()
            .map(|ruta| {
                let k = clave(&ruta);
                Clip {
                    entrada: entradas.remove(&k),
                    meta: metas.get(&k).cloned().unwrap_or_default(),
                    clave: k,
                    ruta,
                    curva: Vec::new(),
                    tramos: Vec::new(),
                    aprobado: 0.0,
                }
            })
            .collect();
        if let Some((col, rev)) = self.orden {
            self.ordenar_clips(col, rev);
        }
        self.puntuar();
        if self.analisis.is_none() {
            let hechos = self.clips.iter().filter(|c| c.entrada.is_some()).count();
            let faltan = self.clips.len() - hechos;
            self.estado = format!("{hechos} de {} clips analizados.", self.clips.len())
                + &if faltan > 0 { format!(" Pulsa Analizar para los {faltan} que faltan.") } else { " Ajusta el criterio a la derecha y exporta.".into() };
            self.estado_color = APAGADO;
            if !sin_conexion.is_empty() {
                let nombres: Vec<String> = sin_conexion.iter().map(|s| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.clone())).collect();
                self.estado = format!("Material sin conexión: {}. Conecta la unidad; el proyecto lo conserva.", nombres.join(", "));
                self.estado_color = ROJO;
            }
        }
        self.actual = mantener.and_then(|k| self.clips.iter().position(|c| c.clave == k));
        let claves: HashSet<String> = self.clips.iter().map(|c| c.clave.clone()).collect();
        self.seleccion.retain(|k| claves.contains(k));
        // metadatos que faltan, en segundo plano
        // (solo los que están: el error de una unidad desconectada no se guarda, se sondea al volver)
        let faltan: Vec<PathBuf> = self.clips.iter().filter(|c| c.meta.codec.is_none() && c.meta.error.is_none() && c.ruta.exists()).map(|c| c.ruta.clone()).collect();
        if let (false, Some(o), false) = (faltan.is_empty(), out.clone(), self.sondeando) {
            self.sondeando = true;
            analisis::sondear(faltan, o, self.tx.clone(), self.despertar());
        }
        self.lanzar_proxies();
    }

    /// Ordena por una columna (`rev` = descendente).
    fn ordenar_clips(&mut self, col: Columna, rev: bool) {
        let clave_f = |c: &Clip| -> (f64, String) {
            match col {
                Columna::Ruta => (0.0, c.clave.clone()),
                Columna::Clip => (0.0, c.nombre().to_lowercase()),
                Columna::Duracion => (c.dur(), String::new()),
                Columna::Joya => (if c.entrada.is_some() { c.parte_joya() } else { -1.0 }, String::new()),
                Columna::Aprobado => (if c.entrada.is_some() { c.aprobado } else { -1.0 }, String::new()),
                Columna::Tramos => (c.tramos.len() as f64, String::new()),
                Columna::Estado => (if c.entrada.is_some() { 1.0 } else { 0.0 }, String::new()),
                Columna::Puntuacion => (c.media().unwrap_or(-1.0), String::new()),
            }
        };
        let actual = self.actual.and_then(|i| self.clips.get(i)).map(|c| c.clave.clone());
        // orden estable, como `list.sort` de Python
        self.clips.sort_by(|a, b| {
            let (x, y) = (clave_f(a), clave_f(b));
            let o = x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| x.1.cmp(&y.1));
            if rev { o.reverse() } else { o }
        });
        self.actual = actual.and_then(|k| self.clips.iter().position(|c| c.clave == k));
    }

    /// Clic en una columna: ascendente, y descendente si se repite (igual que `_sort_by`).
    pub fn ordenar_por(&mut self, col: Columna) {
        let rev = self.orden == Some((col, false));
        self.orden = Some((col, rev));
        self.ordenar_clips(col, rev);
    }

    /// Elige un clip de la lista (igual que `_on_select`): para la reproducción y va a su primer tramo.
    pub fn seleccionar(&mut self, i: usize) {
        if i >= self.clips.len() || Some(i) == self.actual {
            return;
        }
        self.actual = Some(i);
        self.reproduciendo = false;
        let t = self.clips[i].tramos.first().map(|s| s.inicio).unwrap_or(0.0);
        self.buscar(t);
        self.marcar_sucio();
    }

    /// Clic en una fila con modificadores (Ctrl alterna, Shift añade un rango).
    pub fn clic_fila(&mut self, i: usize, ctrl: bool, shift: bool) {
        if i >= self.clips.len() {
            return;
        }
        let k = self.clips[i].clave.clone();
        if shift {
            // la fila del ancla se busca por su clave; si ya no está en la lista, el rango empieza aquí
            let a = self.ancla.as_ref().and_then(|ka| self.clips.iter().position(|c| &c.clave == ka)).unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            for j in lo..=hi {
                self.seleccion.insert(self.clips[j].clave.clone());
            }
        } else if ctrl {
            if !self.seleccion.remove(&k) {
                self.seleccion.insert(k.clone());
            }
            self.ancla = Some(k);
        } else {
            self.seleccion.clear();
            self.seleccion.insert(k.clone());
            self.ancla = Some(k);
        }
        self.seleccionar(i);
    }

    /// "Joya de este clip": corrige el nombre en todo el clip y queda como ejemplo para entrenar.
    pub fn poner_tipo(&mut self, valor: Option<String>) {
        let Some(i) = self.actual else { return };
        if self.out.is_empty() {
            return;
        }
        let k = self.clips[i].clave.clone();
        match valor {
            None => {
                self.tipos.remove(&k);
            }
            Some(v) => {
                self.tipos.insert(k, v);
            }
        }
        let out = PathBuf::from(&self.out);
        let _ = std::fs::create_dir_all(&out);
        let _ = std::fs::write(out.join("clip_types.json"), proyecto::a_json_sangrado(&json!(self.tipos)));
        let t = self.t;
        self.cargar_clips();
        self.buscar(t);
        self.marcar_sucio();
    }

    // =============================================================================================
    // Color por clip / seleccionados / todos
    // =============================================================================================

    /// Da la conversión `id` a este clip, a los seleccionados o a todos (igual que `_apply_color`).
    pub fn aplicar_color(&mut self, id: &str, alcance: &str) {
        let mut spec = ColorSpec { id: id.to_owned(), cube: None };
        if id == "cube" {
            let Some(f) = rfd::FileDialog::new().set_title("LUT .cube").add_filter("LUT 3D", &["cube"]).pick_file() else { return };
            spec.cube = Some(f.to_string_lossy().to_string());
        }
        let objetivos: Vec<String> = match alcance {
            "todos" => self.clips.iter().map(|c| c.clave.clone()).collect(),
            "seleccionados" => self.seleccion.iter().cloned().collect(),
            _ => self.actual.map(|i| vec![self.clips[i].clave.clone()]).unwrap_or_default(),
        };
        for k in &objetivos {
            if id == "auto" {
                self.colores.remove(k);
            } else {
                self.colores.insert(k.clone(), spec.clone());
            }
        }
        self.escribir_colores();
        self.tras_cambio_color(objetivos.len());
    }

    fn escribir_colores(&self) {
        if self.out.is_empty() {
            return;
        }
        let out = PathBuf::from(&self.out);
        let _ = std::fs::create_dir_all(&out);
        let _ = std::fs::write(out.join("colores.json"), proyecto::a_json_sangrado(&json!(self.colores)));
    }

    fn tras_cambio_color(&mut self, n: usize) {
        let rehacer = self
            .clips
            .iter()
            .filter(|c| c.entrada.as_ref().map(|e| !ColorSpec::igual(e.color.as_ref(), Some(&self.color_de(c)))).unwrap_or(false))
            .count();
        self.estado = format!("Color aplicado a {n} clip(s).")
            + &if rehacer > 0 { format!(" {rehacer} necesitan reanalizarse con el nuevo color: pulsa Analizar.") } else { String::new() };
        self.estado_color = if rehacer > 0 { ORO } else { APAGADO };
        // las miniaturas se rehacen con el color nuevo
        self.minis_pedidas.clear();
        self.texturas_mini.clear();
        self.marcar_sucio();
        self.redibujar_cuadro();
    }

    // =============================================================================================
    // Detector de joyas (edición de joyas)
    // =============================================================================================

    /// Elige el detector del próximo análisis. Los clips ya analizados con el otro pasan a
    /// "Reanalizar · detector" (como al cambiar el color) y Analizar los rehace; nada se borra.
    pub fn cambiar_detector(&mut self, k: DetectorKind) {
        if self.detector == k || motor::bodas() {
            return;
        }
        self.detector = k;
        // la misma regla que la lista y la CLI: un respaldo del clásico o una caché "en lugar de" valen
        let rehacer = self.clips.iter().filter(|c| c.entrada.as_ref().map(|e| !camcuts_core::detector::cache_vigente(&e.nucleo, k)).unwrap_or(false)).count();
        self.estado = format!("Detector: {}.", motor::nombre_detector(k))
            + &if rehacer > 0 { format!(" {rehacer} clip(s) se analizaron con el otro detector: pulsa Analizar para rehacerlos.") } else { String::new() };
        self.estado_color = if rehacer > 0 { ORO } else { APAGADO };
        self.marcar_sucio();
    }

    /// Opción `--detector` del análisis: solo en la edición de joyas y solo para el `--analizar` de esta
    /// app (`previos`); una CLI externa (`CAMCUTS_CLI`, p. ej. la de Python) no la conoce. Se pasa siempre,
    /// también "world": así una variable `CAMCUTS_DETECTOR` heredada no cambia lo elegido en la ventana.
    pub fn args_detector(&self, previos: &[String]) -> Vec<String> {
        if motor::bodas() || previos.first().map(String::as_str) != Some(analisis::ARG_ANALIZAR) {
            return Vec::new();
        }
        vec!["--detector".into(), self.detector.as_str().into()]
    }

    /// "Detectar perfil": lee el perfil de cámara de los seleccionados (o de todos) en segundo plano.
    pub fn detectar_colores(&mut self) {
        let objetivos: Vec<PathBuf> = if self.seleccion.len() > 1 {
            self.clips.iter().filter(|c| self.seleccion.contains(&c.clave)).map(|c| c.ruta.clone()).collect()
        } else {
            self.clips.iter().map(|c| c.ruta.clone()).collect()
        };
        self.estado = format!("Detectando perfil de color en {} clips…", objetivos.len());
        self.estado_color = APAGADO;
        analisis::detectar_colores(objetivos, self.tx.clone(), self.despertar());
    }

    // =============================================================================================
    // Visor
    // =============================================================================================

    /// Video que se decodifica: el proxy si existe (búsquedas instantáneas), si no el original.
    fn fuente(&self, c: &Clip) -> PathBuf {
        match &c.entrada {
            Some(e) if e.proxy.exists() => e.proxy.clone(),
            Some(e) => e.ruta_decodificar.clone().unwrap_or_else(|| e.ruta.clone()),
            None => c.ruta.clone(),
        }
    }

    /// Medida a la que se decodifica (la de análisis: lado largo 960, como los proxies).
    fn medida(&self, c: &Clip) -> (u32, u32) {
        if let Some(e) = &c.entrada {
            return analisis::medida_analisis(e.ancho, e.alto);
        }
        let res = c.meta.res.clone().unwrap_or_default();
        let mut it = res.split('×').filter_map(|x| x.trim().parse::<u32>().ok());
        match (it.next(), it.next()) {
            (Some(w), Some(h)) if w > 0 && h > 0 => analisis::medida_analisis(w, h),
            _ => (960, 540),
        }
    }

    fn grado(&self, c: &Clip) -> Option<Grado> {
        self.ver_color.then(|| Grado {
            spec: self.color_de(c),
            log: c.entrada.as_ref().map(|e| e.es_log()).unwrap_or(false),
            ruta_clip: c.ruta.clone(),
        })
    }

    /// Mueve el cabezal ya; el cuadro llega cuando el decodificador lo tiene (igual que `_seek`).
    pub fn buscar(&mut self, t: f64) {
        let Some(i) = self.actual else { return };
        let c = &self.clips[i];
        self.t = t.clamp(0.0, (c.dur() - 0.05).max(0.0));
        self.reproduciendo = false;
        let (w, h) = self.medida(c);
        self.visor.pedir(Pedido::Buscar { fuente: self.fuente(c), t: self.t, ancho: w, alto: h, grado: self.grado(c) });
        self.t_pedido = Some((self.t, Instant::now()));
    }

    /// Duración de un cuadro del clip actual (1/30 s si no se conoce): la barra de avance no vuelve a
    /// buscar por cambios menores.
    pub fn duracion_cuadro(&self) -> f64 {
        let fps = self.actual.and_then(|i| self.clips.get(i)).and_then(|c| c.entrada.as_ref().map(|e| e.cadencia()).or(c.meta.fps));
        1.0 / fps.filter(|f| *f > 0.0).unwrap_or(30.0)
    }

    /// Reproducir / pausar.
    pub fn alternar_reproduccion(&mut self) {
        let Some(i) = self.actual else { return };
        if self.reproduciendo {
            self.reproduciendo = false;
            self.visor.pedir(Pedido::Parar);
            return;
        }
        let c = &self.clips[i];
        if self.t >= c.dur() - 0.1 {
            self.t = 0.0;
        }
        let fps = c.entrada.as_ref().map(|e| e.cadencia()).or(c.meta.fps).unwrap_or(30.0);
        let (w, h) = self.medida(c);
        self.visor.pedir(Pedido::Reproducir { fuente: self.fuente(c), t: self.t, fps, ancho: w, alto: h, grado: self.grado(c) });
        self.reproduciendo = true;
    }

    /// Salta al siguiente tramo aprobado (o al primero).
    pub fn siguiente_tramo(&mut self) {
        let Some(i) = self.actual else { return };
        let segs = &self.clips[i].tramos;
        let t = segs.iter().map(|s| s.inicio).find(|&s| s > self.t + 0.1).or_else(|| segs.first().map(|s| s.inicio)).unwrap_or(0.0);
        self.buscar(t);
    }

    /// Vuelve a pedir el cuadro actual (cambió el color o se activaron/desactivaron las cajas).
    pub fn redibujar_cuadro(&mut self) {
        let t = self.t;
        let seguia = self.reproduciendo;
        self.buscar(t);
        if seguia {
            self.alternar_reproduccion();
        }
    }

    /// Recoge el último cuadro decodificado y lo sube como textura. Si la última búsqueda no dio cuadro,
    /// quita la textura: la imagen anterior es de otro clip u otro instante, y sobre ella se dibujarían
    /// las cajas, las píldoras y el recorte de Foco del clip nuevo.
    fn recoger_cuadro(&mut self) {
        let mut nuevo = None;
        let mut fallo = None;
        if let Ok(mut s) = self.visor.salida.lock() {
            if let Some(c) = s.cuadro.as_ref() {
                if c.serie != self.serie_cuadro {
                    nuevo = s.cuadro.take();
                }
            }
            if let Some(n) = s.sin_cuadro.filter(|&n| n != self.serie_cuadro) {
                fallo = Some(n);
            }
            if self.reproduciendo && !s.reproduciendo && s.cuadro.is_none() {
                // la reproducción llegó al final
                self.reproduciendo = false;
            }
            if let Some(e) = s.error.take() {
                self.estado = e;
                self.estado_color = ROJO;
            }
        }
        // el decodificador publica o un cuadro o un fallo (nunca los dos a la vez)
        if let Some(n) = fallo {
            self.serie_cuadro = n;
            self.tex_visor = None;
            self.sin_cuadro = true;
            self.t_pedido = None;
        }
        if let Some(c) = nuevo {
            self.sin_cuadro = false;
            self.serie_cuadro = c.serie;
            self.tam_cuadro = [c.ancho, c.alto];
            let img = egui::ColorImage::from_rgb([c.ancho as usize, c.alto as usize], &c.rgb);
            match self.tex_visor.as_mut() {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => self.tex_visor = Some(self.ctx.load_texture("visor", img, TextureOptions::LINEAR)),
            }
            if self.reproduciendo {
                self.t = c.t;
            }
            self.t_pedido = None;
        }
        if self.reproduciendo {
            if let Ok(s) = self.visor.salida.lock() {
                if !s.reproduciendo {
                    self.reproduciendo = false;
                }
            }
        }
    }

    /// Pide las miniaturas que faltan y sube las que llegaron.
    fn recoger_miniaturas(&mut self) {
        let listas: Vec<(String, Vec<u8>)> = self.miniaturas.listas.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default();
        for (k, rgb) in listas {
            let img = egui::ColorImage::from_rgb([video::MINI_W as usize, video::MINI_H as usize], &rgb);
            let tex = self.ctx.load_texture(format!("mini-{k}"), img, TextureOptions::LINEAR);
            self.texturas_mini.insert(k, tex);
        }
        for c in &self.clips {
            if self.minis_pedidas.contains(&c.clave) {
                continue;
            }
            // solo clips analizados con proxy, o clips sin analizar ya sondeados: un vistazo rápido
            let fuente = self.fuente(c);
            if c.entrada.is_none() && c.meta.codec.is_none() {
                continue;
            }
            let t = c.tramos.first().map(|s| (s.inicio + s.fin) / 2.0).unwrap_or(c.dur() / 3.0);
            self.minis_pedidas.insert(c.clave.clone());
            self.miniaturas.pedir(PedidoMini { clave: c.clave.clone(), fuente, t, grado: self.grado(c) });
        }
    }

    // =============================================================================================
    // Material
    // =============================================================================================

    pub fn agregar_carpeta(&mut self) {
        if let Some(d) = rfd::FileDialog::new().set_title("Carpeta con material de cámara").pick_folder() {
            self.agregar_fuentes(vec![d.to_string_lossy().to_string()]);
        }
    }

    pub fn agregar_archivos(&mut self) {
        let ext = datos::extensiones_video();
        let ext: Vec<&str> = ext.iter().map(String::as_str).collect();
        if let Some(f) = rfd::FileDialog::new().set_title("Clips").add_filter("Video", &ext).add_filter("Todos", &["*"]).pick_files() {
            self.agregar_fuentes(f.into_iter().map(|p| p.to_string_lossy().to_string()).collect());
        }
    }

    /// Añade carpetas o archivos (igual que `_add_sources`).
    pub fn agregar_fuentes(&mut self, nuevas: Vec<String>) {
        for s in nuevas {
            if !self.fuentes.contains(&s) {
                self.fuentes.push(s);
            }
        }
        if self.out_auto {
            self.out = self.out_automatica();
        }
        self.cargar_clips();
        self.marcar_sucio();
    }

    /// Resultados junto al material, en la carpeta de esta edición: `<material>\CAMCUTS` (joyas) o
    /// `<material>\CAMCUTS Bodas`. La carpeta del material sale de la misma prueba que
    /// `carpeta_material` (por extensión), así una carpeta sin conexión no cae en su carpeta madre.
    fn out_automatica(&self) -> String {
        let Some(primera) = self.fuentes.first() else { return String::new() };
        let base = datos::carpeta_de_fuente(Path::new(primera));
        camcuts_core::edicion::carpeta_resultados(&base).to_string_lossy().to_string()
    }

    pub fn quitar_fuentes(&mut self) {
        self.fuentes.clear();
        self.out_auto = true;
        self.out.clear();
        self.actual = None;
        self.seleccion.clear();
        self.tex_visor = None;
        self.sin_cuadro = false;
        self.cargar_clips();
    }

    /// Texto "21 clips · Octubre 27" de la barra de material.
    pub fn texto_fuentes(&self) -> String {
        if self.fuentes.is_empty() {
            return "Sin material".into();
        }
        let n = datos::reunir_clips(&self.fuentes, self.recursivo).len();
        let donde: Vec<String> = self.fuentes.iter().take(3).map(|s| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.clone())).collect();
        format!("{n} clips · {}{}", donde.join("; "), if self.fuentes.len() > 3 { "…" } else { "" })
    }

    /// Elegir carpeta de resultados a mano.
    pub fn elegir_out(&mut self) {
        if let Some(d) = rfd::FileDialog::new().set_title("Carpeta de resultados").pick_folder() {
            self.out = d.to_string_lossy().to_string();
            self.out_auto = false;
            self.cargar_clips();
            self.marcar_sucio();
        }
    }

    /// Abre una carpeta de resultados existente (por ejemplo una hecha por Python) sin material.
    pub fn abrir_resultados(&mut self, dir: &Path) {
        // ya no se mira un proyecto de la otra edición: lo que se haga ahora sí se autoguarda
        self.solo_lectura = false;
        self.fuentes.clear();
        self.out = dir.to_string_lossy().to_string();
        self.out_auto = false;
        self.colores = std::fs::read_to_string(dir.join("colores.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        self.cargar_clips();
    }

    pub fn abrir_carpeta_resultados(&self) {
        let d = Path::new(&self.out);
        if d.exists() {
            let mut c = std::process::Command::new("explorer");
            c.arg(d);
            let _ = c.spawn();
        }
    }

    // =============================================================================================
    // Análisis (proceso hijo)
    // =============================================================================================

    pub fn analizar(&mut self) {
        let clips = datos::reunir_clips(&self.fuentes, self.recursivo);
        if clips.is_empty() {
            self.avisar("CAMCUTS", "Agrega una carpeta o clips primero.");
            return;
        }
        let Some((cli, previos)) = analisis::ruta_cli() else {
            self.avisar("CAMCUTS", "No encuentro el ejecutable de CAMCUTS para analizar.");
            return;
        };
        let out = PathBuf::from(&self.out);
        if std::fs::create_dir_all(&out).is_err() {
            self.avisar("CAMCUTS", &format!("No se pudo crear la carpeta de resultados:\n{}", out.display()));
            return;
        }
        let lista = out.join("clips.txt");
        let texto: Vec<String> = clips.iter().map(|c| c.to_string_lossy().to_string()).collect();
        let _ = std::fs::write(&lista, texto.join("\n"));
        self.escribir_colores();
        // camcuts.exe --analizar + las mismas opciones que la ventana de Python pasa a `python -m camcuts`
        // (+ `--detector` en la edición de joyas)
        let detector = self.args_detector(&previos);
        let mut args: Vec<String> = previos;
        args.extend::<Vec<String>>(vec![
            "--lista".into(),
            lista.to_string_lossy().into(),
            "--out".into(),
            out.to_string_lossy().into(),
            "--no-sheet".into(),
            "--preset".into(),
            self.criterio.nombre.clone(),
            "--colores".into(),
            out.join("colores.json").to_string_lossy().into(),
        ]);
        if self.segundo_plano {
            args.push("--background".into());
        }
        if self.zoom_joyas {
            args.push("--zoom".into());
        }
        args.extend(detector);
        self.guardar_preferencias();
        self.parar_proxies();
        match Analisis::lanzar(&cli, &args, self.tx.clone(), self.despertar()) {
            Ok(a) => {
                self.analisis = Some(a);
                self.progreso = Some((0, clips.len()));
                self.avance_clips.clear();
                self.estado = format!("Analizando {} clips… (los ya analizados se saltan)", clips.len());
                self.estado_color = APAGADO;
            }
            Err(e) => self.avisar("CAMCUTS", &format!("No se pudo lanzar el análisis:\n{e}")),
        }
    }

    pub fn detener(&mut self) {
        if let Some(a) = &self.analisis {
            a.detener();
        }
    }

    fn lanzar_proxies(&mut self) {
        if self.analisis.is_some() || self.proxies.as_ref().map(|p| p.vivo()).unwrap_or(false) {
            return;
        }
        let faltan: Vec<(PathBuf, u32, u32, PathBuf)> = self
            .clips
            .iter()
            .filter_map(|c| c.entrada.as_ref())
            .filter(|e| !e.proxy.exists())
            .map(|e| (e.ruta_decodificar.clone().unwrap_or_else(|| e.ruta.clone()), e.ancho, e.alto, e.proxy.clone()))
            .filter(|(f, ..)| f.exists())
            .collect();
        if faltan.is_empty() {
            return;
        }
        self.proxies = Some(TrabajoProxies::lanzar(faltan, self.tx.clone(), self.despertar()));
    }

    fn parar_proxies(&mut self) {
        if let Some(p) = &self.proxies {
            p.detener();
        }
    }

    /// Atiende los avisos de los hilos de fondo (igual que `_poll`).
    fn atender_mensajes(&mut self) {
        let mut recargar = false;
        let mut aviso_metas = false;
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Mensaje::Metas(metas, escrito) => {
                    self.recibir_metas(metas);
                    aviso_metas |= !escrito && !self.aviso_metas;
                    recargar = true;
                }
                Mensaje::Colores(encontrados) => {
                    let n = encontrados.len();
                    self.colores.extend(encontrados);
                    self.escribir_colores();
                    self.tras_cambio_color(n);
                    if n == 0 {
                        self.estado = "No se encontró perfil de color en los metadatos de esos clips.".into();
                    }
                }
                Mensaje::Proxy(msg) => {
                    if self.analisis.is_none() {
                        self.estado = if msg == "done" { "Vistas previas listas: el visor ya va fluido.".into() } else { format!("Creando vistas previas para un visor fluido… {msg}") };
                        self.estado_color = APAGADO;
                    }
                }
                Mensaje::Fin(codigo) => {
                    self.analisis = None;
                    self.progreso = None;
                    self.avance_clips.clear();
                    self.estado = if codigo == 0 { "Análisis terminado. Ajusta el criterio a la derecha y exporta.".into() } else { format!("El análisis se detuvo (código {codigo}).") };
                    self.estado_color = if codigo == 0 { APAGADO } else { ROJO };
                    recargar = true;
                }
                Mensaje::Linea(l) => {
                    if let Some((valor, ruta)) = analisis::avance(&l) {
                        // avance de un clip: el porcentaje de su fila y la barra de estado (sin releer la lista)
                        let ruta = Path::new(ruta);
                        self.avance_clips.insert(datos::clave(ruta), valor);
                        if let Some(v) = valor.filter(|v| *v < 100) {
                            let nombre = ruta.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            let (hechos, total) = self.progreso.unwrap_or((0, 0));
                            self.estado = format!("Analizando… {hechos} de {total} clips listos · {nombre} {v}%");
                            self.estado_color = APAGADO;
                        }
                    } else if let Some((a, b, resto)) = analisis::progreso(&l) {
                        self.progreso = Some((a, b));
                        let corto: String = resto.chars().take(90).collect();
                        self.estado = format!("Analizando… {a} de {b} clips · {corto}");
                        self.estado_color = APAGADO;
                        recargar = true;
                    } else if l.starts_with("equipo:") {
                        self.estado_color = if l.contains("OCUPADA") { ROJO } else { APAGADO };
                        self.estado = l;
                    }
                }
            }
        }
        if recargar {
            let t = self.t;
            let habia = self.actual;
            self.cargar_clips();
            if habia.is_some() && self.actual.is_some() && !self.reproduciendo {
                self.t = t;
            }
        }
        // después de recargar (que reescribe la barra de estado), y una sola vez
        if aviso_metas {
            self.aviso_metas = true;
            self.estado = format!("No se puede escribir en la carpeta de resultados ({}): los datos de los clips no se guardan.", self.out);
            self.estado_color = ROJO;
        }
    }

    /// Guarda en la sesión los metadatos sondeados (aunque no se hayan podido escribir en el disco).
    fn recibir_metas(&mut self, metas: BTreeMap<String, datos::Meta>) {
        self.sondeando = false;
        self.metas_sesion.extend(metas);
    }

    // =============================================================================================
    // Exportar
    // =============================================================================================

    /// Escribe el OTIO y el CSV con el modo elegido (igual que `_export`).
    pub fn exportar(&mut self) {
        let analizados: Vec<&Clip> = self.clips.iter().filter(|c| c.entrada.is_some()).collect();
        if analizados.is_empty() {
            self.avisar("CAMCUTS", "No hay material analizado en esta carpeta de resultados.");
            return;
        }
        let cadencia = self.cadencia();
        let nombre_crit = motor::criterios().iter().find(|c| c.0 == self.criterio.nombre).map(|c| c.1).unwrap_or("Equilibrado");
        let modo_txt = motor::MODOS_EXPORTAR.iter().find(|m| m.0 == self.modo).map(|m| m.1).unwrap_or("");
        // CAMCUTS_<criterio>_<modo>.otio en joyas (como Python), CAMCUTS_Bodas_… en bodas: nunca se pisan
        let prefijo = camcuts_core::edicion::actual().prefijo_export();
        let ruta = Path::new(&self.out).join(format!("{prefijo}_{}_{}.otio", nombre_crit.split(' ').next().unwrap_or(""), self.modo));
        let datos: Vec<(&Entrada, &[Tramo])> = analizados.iter().map(|c| (c.entrada.as_ref().expect("analizado"), c.tramos.as_slice())).collect();
        let pendientes = self.clips.len() - analizados.len();
        match motor::exportar(&datos, &ruta, &format!("{} · {nombre_crit} · {modo_txt}", motor::nombre_app()), cadencia, &self.modo) {
            Ok((n, fallo_csv)) => {
                self.ctx.copy_text(ruta.to_string_lossy().to_string());
                let mut texto = format!(
                    "{n} tramos de {} clips exportados a {cadencia:.3} fps{}.\n\n{}\n\nLa ruta quedó copiada. En DaVinci: File ▸ Import ▸ Timeline… y pégala.\nLos fps deben coincidir con los de tu línea de tiempo.",
                    analizados.len(),
                    if pendientes > 0 { format!(" ({pendientes} clips sin analizar quedaron fuera)") } else { String::new() },
                    ruta.display()
                );
                // la línea de tiempo está; el CSV no se pudo reemplazar (¿abierto en Excel?)
                if let Some(e) = fallo_csv {
                    texto.push_str(&format!("\n\nEl CSV no se pudo reemplazar (¿está abierto en Excel?): {e}"));
                }
                self.avisar("Exportado", &texto);
            }
            Err(e) => self.avisar("No se pudo exportar", &e.to_string()),
        }
    }

    pub fn avisar(&mut self, titulo: &str, texto: &str) {
        self.aviso = Some(Aviso { titulo: titulo.into(), texto: texto.into(), confirmar_salida: false });
    }

    // =============================================================================================
    // Proyecto y preferencias
    // =============================================================================================

    /// Todo lo que recuerda un proyecto (mismas claves que `_state()` de Python).
    pub fn estado_json(&self) -> Map<String, Value> {
        let c = &self.criterio;
        let mut m = Map::new();
        m.insert("sources".into(), json!(self.fuentes));
        m.insert("out".into(), json!(self.out));
        m.insert("results_dir".into(), json!(self.out));
        m.insert("out_auto".into(), json!(self.out_auto));
        m.insert("criterion".into(), json!(c.nombre));
        m.insert("fps".into(), json!(self.fps));
        m.insert("recursive".into(), json!(self.recursivo));
        m.insert("background".into(), json!(self.segundo_plano));
        m.insert("zoom".into(), json!(self.zoom_joyas));
        if !motor::bodas() {
            // detector de joyas ("world" / "propio"); Python ignora la clave. Bodas no la escribe.
            m.insert("detector".into(), json!(self.detector.as_str()));
            // versión de la clave: sin ella, el "world" guardado era solo el antiguo predeterminado
            m.insert(CLAVE_DETECTOR_V.into(), json!(DETECTOR_V));
        }
        m.insert("export_mode".into(), json!(self.modo));
        m.insert("thr".into(), json!(c.umbral));
        m.insert("min".into(), json!(c.min_s));
        m.insert("max".into(), json!(c.max_s));
        m.insert("per".into(), json!(c.por_clip));
        m.insert("gap".into(), json!(c.tolerancia));
        m.insert("smooth".into(), json!(c.suavizado));
        let mut pesos = Map::new();
        for (k, _) in motor::pesos() {
            pesos.insert((*k).into(), json!(c.pesos.get(*k).copied().unwrap_or(0.0)));
        }
        m.insert("weights".into(), Value::Object(pesos));
        m.insert(motor::clave_tipos().into(), json!(c.joyas));
        m.insert("tipos".into(), json!(c.planos));
        m.insert("clip_types".into(), json!(self.tipos));
        m.insert("colors".into(), json!(self.colores));
        let actual = self.actual.and_then(|i| self.clips.get(i)).map(|c| c.ruta.to_string_lossy().to_string());
        m.insert("ui".into(), json!({"current": actual, "t": self.t, "map_zoom": self.zoom_mapa, "map_t0": self.t0_mapa,
            // extras de esta ventana (Python los ignora)
            "paneles": {"material": self.paneles.material, "pesos": self.paneles.pesos, "color": self.paneles.color,
                        "tiempo": self.paneles.tiempo, "mapa": self.paneles.mapa}, "alto_tiempo": self.alto_tiempo,
            "titulo_plegado": self.titulo_plegado, "recibo_plegado": self.recibo_plegado, "modo_claro": self.modo_claro}));
        let pendientes: Vec<String> = self.clips.iter().filter(|c| c.entrada.is_none()).take(2000).map(|c| c.ruta.to_string_lossy().to_string()).collect();
        m.insert("analysis".into(), json!({"clips": self.clips.len(), "analysed": self.clips.iter().filter(|c| c.entrada.is_some()).count(), "pending": pendientes}));
        m
    }

    /// Restaura un estado guardado (igual que `_apply_state`).
    pub fn aplicar_estado(&mut self, d: &Map<String, Value>) {
        self.cargando = true;
        let crit = d.get("criterion").and_then(Value::as_str).filter(|c| motor::criterios().iter().any(|x| x.0 == *c)).unwrap_or(motor::criterio_defecto());
        self.criterio = Criterio::base(crit);
        let f = |k: &str| d.get(k).and_then(Value::as_f64);
        if let Some(v) = f("thr") { self.criterio.umbral = v; }
        if let Some(v) = f("min") { self.criterio.min_s = v; }
        if let Some(v) = f("max") { self.criterio.max_s = v; }
        if let Some(v) = f("per") { self.criterio.por_clip = v as u32; }
        if let Some(v) = f("gap") { self.criterio.tolerancia = v; }
        if let Some(v) = f("smooth") { self.criterio.suavizado = v; }
        if let Some(p) = d.get("weights").and_then(Value::as_object) {
            for (k, v) in p {
                if let (Some(x), true) = (v.as_f64(), self.criterio.pesos.contains_key(k)) {
                    self.criterio.pesos.insert(k.clone(), x);
                }
            }
        }
        let lista = |k: &str| -> Vec<String> { d.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
        self.criterio.joyas = lista(motor::clave_tipos()).into_iter().filter(|j| motor::tipos().contains(&j.as_str())).collect();
        self.criterio.planos = lista("tipos").into_iter().filter(|j| motor::planos().iter().any(|p| p.0 == j)).collect();
        if let Some(b) = d.get("recursive").and_then(Value::as_bool) { self.recursivo = b; }
        if let Some(b) = d.get("background").and_then(Value::as_bool) { self.segundo_plano = b; }
        if let Some(b) = d.get("zoom").and_then(Value::as_bool) { self.zoom_joyas = b; }
        // El detector guardado vale solo desde que el propio es el predeterminado (clave `detector_v`).
        // Antes casi todos guardaban "world" sin haberlo elegido (era el predeterminado): esos estados, y
        // los de Python sin clave, pasan al propio. Sus clips hechos con el clásico salen "Reanalizar ·
        // detector" y Analizar los rehace.
        if !motor::bodas() {
            let vale = d.get(CLAVE_DETECTOR_V).and_then(Value::as_u64).unwrap_or(0) >= DETECTOR_V;
            self.detector = if vale { d.get("detector").and_then(Value::as_str).and_then(DetectorKind::parse).unwrap_or_default() } else { DetectorKind::default() };
        }
        match d.get("export_mode").and_then(Value::as_str) {
            Some(m) if motor::MODOS_EXPORTAR.iter().any(|x| x.0 == m) => self.modo = m.into(),
            _ if d.get("all_clips").and_then(Value::as_bool) == Some(false) => self.modo = "aprobados".into(),
            _ => {}
        }
        if let Some(fps) = d.get("fps").and_then(Value::as_str).filter(|f| FPS.contains(f)) {
            self.fps = fps.into();
        }
        // se conservan también las rutas que no están (unidad desconectada): se muestran "sin conexión" y
        // el proyecto no las pierde al guardarse
        self.fuentes = lista("sources");
        self.out_auto = d.get("out_auto").and_then(Value::as_bool).unwrap_or(true);
        self.out = d.get("out").and_then(Value::as_str).unwrap_or("").to_owned();
        if self.out_auto {
            // nunca mostrar resultados de otro material: la carpeta sale del material
            self.out = self.out_automatica();
        }
        self.colores = d.get("colors").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let tipos: BTreeMap<String, String> = d.get("clip_types").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        // Un proyecto de la otra edición (solo lectura, ya fijado por `abrir_proyecto`) trae tipos de la
        // otra ("votos" en Bodas, "anillo" en joyas): no valen aquí y escribirlos pisaba el
        // `clip_types.json` de esta edición, que es su carpeta de resultados
        if self.solo_lectura {
            self.tipos_proyecto = None;
        } else {
            if !tipos.is_empty() && !self.out.is_empty() {
                // la línea de comandos lee los tipos por clip de la carpeta de resultados
                let out = PathBuf::from(&self.out);
                if std::fs::create_dir_all(&out).is_ok() {
                    let _ = std::fs::write(out.join("clip_types.json"), proyecto::a_json_sangrado(&json!(tipos)));
                }
            }
            self.tipos_proyecto = Some(tipos);
        }
        let ui = d.get("ui").and_then(Value::as_object).cloned().unwrap_or_default();
        self.zoom_mapa = ui.get("map_zoom").and_then(Value::as_f64).unwrap_or(1.0);
        self.t0_mapa = ui.get("map_t0").and_then(Value::as_f64).unwrap_or(0.0);
        if let Some(p) = ui.get("paneles").and_then(Value::as_object) {
            let b = |k: &str, def: bool| p.get(k).and_then(Value::as_bool).unwrap_or(def);
            self.paneles = Paneles { material: b("material", true), pesos: b("pesos", true), color: b("color", false), tiempo: b("tiempo", true), mapa: b("mapa", true) };
        }
        self.titulo_plegado = ui.get("titulo_plegado").and_then(Value::as_bool).unwrap_or(true);
        self.recibo_plegado = ui.get("recibo_plegado").and_then(Value::as_bool).unwrap_or(true);
        if let Some(b) = ui.get("modo_claro").and_then(Value::as_bool) {
            if b != self.modo_claro && motor::bodas() {
                self.modo_claro = b;
                crate::bodas_tema::instalar(&self.ctx, b);
            }
        }
        if let Some(a) = ui.get("alto_tiempo").and_then(Value::as_f64) {
            self.alto_tiempo = (a as f32).clamp(120.0, 520.0);
        }
        self.cargando = false;
        self.actual = None;
        self.cargar_clips();
        // de vuelta donde estaba el editor
        if let Some(quiero) = ui.get("current").and_then(Value::as_str) {
            if let Some(i) = self.clips.iter().position(|c| c.ruta.to_string_lossy() == quiero) {
                self.seleccion = [self.clips[i].clave.clone()].into();
                self.seleccionar(i);
                self.buscar(ui.get("t").and_then(Value::as_f64).unwrap_or(0.0));
            }
        }
        self.sucio = None;
    }

    pub fn marcar_sucio(&mut self) {
        if !self.cargando {
            self.sucio = Some(Instant::now());
        }
    }

    /// Guarda el proyecto (en su ruta, o en la de un trabajo nuevo junto a los resultados).
    ///
    /// `ruta` es la que eligió el editor ("Guardar como": el diálogo ya preguntó si reemplazar). Sin ella
    /// es el autoguardado, que nunca reescribe un proyecto de la otra edición: ni el abierto en solo
    /// lectura ni uno que ya esté en la ruta por defecto.
    pub fn guardar_proyecto(&mut self, ruta: Option<PathBuf>) {
        let elegida = ruta.is_some();
        let destino = match ruta.or_else(|| self.ruta_proyecto.clone()) {
            Some(r) => r,
            None => {
                if self.fuentes.is_empty() {
                    return;
                }
                match proyecto::ruta_por_defecto(&self.out, &self.fuentes) {
                    Some(r) => r,
                    None => return,
                }
            }
        };
        let nombre = destino.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !elegida {
            let ajena = proyecto::app_ajena(&destino).or_else(|| self.solo_lectura.then(String::new));
            if let Some(app) = ajena {
                let de = if app.is_empty() { "otra edición".to_owned() } else { app };
                self.etiqueta_proyecto = format!("Proyecto: {nombre} · de {de}, solo lectura (los cambios no se guardan; usa Guardar como)");
                return;
            }
        }
        match proyecto::guardar(&destino, &self.estado_json()) {
            Ok(()) => {
                self.etiqueta_proyecto = format!("Proyecto: {nombre} · guardado {}", proyecto::hora_texto());
                self.ruta_proyecto = Some(destino);
                // guardado como proyecto de esta edición: ya no es de solo lectura
                self.solo_lectura = false;
            }
            Err(e) => {
                self.etiqueta_proyecto = format!("No se pudo guardar el proyecto: {e}");
            }
        }
    }

    /// Preferencias de la app (último proyecto) + autoguardado del proyecto abierto.
    pub fn guardar_preferencias(&mut self) {
        self.sucio = None;
        if !self.persistir {
            return;
        }
        self.guardar_proyecto(None);
        let mut d = self.estado_json();
        d.insert("last_project".into(), json!(self.ruta_proyecto.as_ref().map(|p| p.to_string_lossy().to_string())));
        proyecto::guardar_preferencias(&d);
    }

    /// ¿Se puede dejar el proyecto de ahora? No con un análisis en curso (escribe en la carpeta de
    /// resultados de este proyecto; mismo aviso que "Nuevo proyecto"). Si se puede, antes se guarda lo
    /// pendiente: `aplicar_estado` descarta el autoguardado diferido (`sucio`) del proyecto que se deja.
    pub fn antes_de_cambiar_de_proyecto(&mut self) -> bool {
        // un mapa de avance con datos también es un análisis en curso (ver `avance_de`)
        if self.analisis.is_some() || !self.avance_clips.is_empty() {
            self.avisar("CAMCUTS", "Espera a que termine el análisis.");
            return false;
        }
        self.guardar_preferencias();
        true
    }

    pub fn abrir_proyecto_dialogo(&mut self) {
        if !self.antes_de_cambiar_de_proyecto() {
            return;
        }
        if let Some(f) = rfd::FileDialog::new().set_title("Abrir proyecto").add_filter("Proyecto CAMCUTS", &["camcuts"]).pick_file() {
            self.abrir_proyecto_al_arrancar(&f);
        }
    }

    /// "Abrir resultados existentes (sin material)…" del menú (las dos ediciones).
    pub fn abrir_resultados_dialogo(&mut self, titulo: &str) {
        if !self.antes_de_cambiar_de_proyecto() {
            return;
        }
        if let Some(d) = rfd::FileDialog::new().set_title(titulo).pick_folder() {
            self.ruta_proyecto = None;
            self.abrir_resultados(&d);
        }
    }

    pub fn guardar_como(&mut self) {
        let inicial = self
            .ruta_proyecto
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| format!("{}{}", self.fuentes.first().and_then(|s| Path::new(s).file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "proyecto".into()), proyecto::EXT));
        if let Some(f) = rfd::FileDialog::new().set_title("Guardar proyecto como").add_filter("Proyecto CAMCUTS", &["camcuts"]).set_file_name(inicial).save_file() {
            self.guardar_proyecto(Some(f));
            self.guardar_preferencias();
        }
    }

    pub fn nuevo_proyecto(&mut self) {
        if self.analisis.is_some() {
            self.avisar("CAMCUTS", "Espera a que termine el análisis.");
            return;
        }
        self.guardar_preferencias();
        self.ruta_proyecto = None;
        self.solo_lectura = false;
        self.tipos.clear();
        self.colores.clear();
        self.quitar_fuentes();
        self.etiqueta_proyecto = "Proyecto nuevo: se guardará solo al agregar material.".into();
    }

    /// Abre un proyecto desde la sesión en marcha: no con un análisis en curso, y guardando antes lo
    /// pendiente del de ahora ([`App::antes_de_cambiar_de_proyecto`]).
    pub fn abrir_proyecto(&mut self, ruta: &Path) {
        if self.antes_de_cambiar_de_proyecto() {
            self.abrir_proyecto_al_arrancar(ruta);
        }
    }

    /// Abre un proyecto sin mirar el de ahora: al arrancar (no hay nada que guardar, y guardar las
    /// preferencias de una ventana vacía borraría las de la sesión anterior) o tras
    /// [`App::antes_de_cambiar_de_proyecto`].
    pub fn abrir_proyecto_al_arrancar(&mut self, ruta: &Path) {
        match proyecto::abrir(ruta) {
            Ok(mut d) => {
                self.ruta_proyecto = Some(ruta.to_path_buf());
                // un proyecto de la otra edición (joyas / bodas) se abre solo para mirar: el autoguardado
                // de esta sesión lo reemplazaría en silencio
                let suya = proyecto::edicion_de(&d);
                self.solo_lectura = suya != camcuts_core::edicion::actual();
                let rd = d.get("results_dir").cloned().unwrap_or(json!(""));
                d.insert("out".into(), rd);
                let con_material = d.get("sources").and_then(Value::as_array).map(|a| !a.is_empty()).unwrap_or(false);
                if self.solo_lectura && con_material {
                    // sus resultados son de la otra edición: esta trabaja en su propia carpeta junto al
                    // material (así Analizar nunca reescribe las cachés de la otra)
                    d.insert("out_auto".into(), json!(true));
                }
                self.aplicar_estado(&d);
                // no se guarda al abrir: el archivo solo se reescribe cuando el editor cambia algo
                let nombre = ruta.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.etiqueta_proyecto = format!("Proyecto: {nombre}");
                if self.solo_lectura {
                    self.etiqueta_proyecto = format!("Proyecto: {nombre} · de {}, solo lectura", suya.nombre_app());
                    self.avisar(
                        "Proyecto de otra edición",
                        &format!(
                            "{nombre} es un proyecto de {}.\n\nSe abrió solo para mirar: {} no lo va a reescribir. Para trabajar con él aquí, usa «Guardar como» y guarda una copia de {}.",
                            suya.nombre_app(),
                            motor::nombre_app(),
                            motor::nombre_app()
                        ),
                    );
                }
            }
            Err(e) => self.avisar("CAMCUTS", &format!("No se pudo abrir el proyecto:\n{e}")),
        }
    }

    /// Título grande de la tarjeta de la izquierda: el nombre de la carpeta del material
    /// (p. ej. "Selects de Octubre 27"); si no se conoce, el nombre del proyecto.
    pub fn titulo(&self) -> String {
        if let Some(n) = self.nombre_material() {
            return format!("Selects de {n}");
        }
        if let Some(p) = &self.ruta_proyecto {
            return p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        }
        "Selects nuevos".into()
    }

    /// Carpeta del material: la primera fuente (o su carpeta si es un archivo); si se abrió una carpeta
    /// de resultados sin material, la carpeta común de los clips de la caché.
    pub fn carpeta_material(&self) -> Option<PathBuf> {
        if let Some(f) = self.fuentes.first() {
            return Some(datos::carpeta_de_fuente(Path::new(f)));
        }
        let mut comun = self.clips.first()?.ruta.parent()?.to_path_buf();
        for c in &self.clips {
            while !c.ruta.starts_with(&comun) {
                comun = comun.parent()?.to_path_buf();
            }
        }
        Some(comun)
    }

    /// Nombre legible de la carpeta del material. Las carpetas genéricas de cámara ("videos", "M4ROOT",
    /// "CLIP"…) se saltan hacia arriba, y un nombre solo numérico lleva delante el de su carpeta madre:
    /// `…\Octubre\27\videos` -> "Octubre 27".
    pub fn nombre_material(&self) -> Option<String> {
        const GENERICAS: &[&str] = &["videos", "video", "clips", "clip", "m4root", "private", "dcim", "footage", "material", "camara", "cámara", "media", "100apple", "xdroot", "sub"];
        let carpeta = self.carpeta_material()?;
        let mut actual: &Path = &carpeta;
        loop {
            let nombre = actual.file_name()?.to_string_lossy().to_string();
            if GENERICAS.contains(&nombre.to_lowercase().as_str()) {
                actual = actual.parent()?;
                continue;
            }
            if nombre.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_') {
                if let Some(madre) = actual.parent().and_then(|m| m.file_name()) {
                    return Some(format!("{} {nombre}", madre.to_string_lossy()));
                }
            }
            return Some(nombre);
        }
    }

    // =============================================================================================
    // Cada cuadro
    // =============================================================================================

    /// Trabajo previo al dibujo: mensajes, cuadros, recálculo y autoguardado.
    pub fn antes_de_dibujar(&mut self) {
        self.atender_mensajes();
        self.recoger_cuadro();
        self.recoger_miniaturas();
        if self.puntuar_pendiente {
            self.puntuar();
        }
        if let Some(s) = self.sucio {
            if s.elapsed() > Duration::from_millis(1500) {
                self.guardar_preferencias();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(300));
            }
        }
        if self.reproduciendo || self.analisis.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
        // si el decodificador tarda, volver a mirar
        if let Some((_, cuando)) = self.t_pedido {
            if cuando.elapsed() < Duration::from_secs(3) {
                self.ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
    }

    /// Olvida las miniaturas para que se rehagan (cambió el color).
    pub fn limpiar_minis(&mut self) {
        self.texturas_mini.clear();
        self.minis_pedidas.clear();
    }

    /// El usuario cerró la ventana: si hay un análisis en curso se pregunta antes (como Python).
    pub fn atender_cierre(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.analisis.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.aviso = Some(Aviso {
                titulo: "Análisis en curso".into(),
                texto: "Hay un análisis en curso. ¿Detenerlo y salir?".into(),
                confirmar_salida: true,
            });
        } else {
            self.al_cerrar();
        }
    }

    /// Al cerrar: parar trabajos y guardar (igual que `_on_close`).
    pub fn al_cerrar(&mut self) {
        self.detener();
        self.parar_proxies();
        self.reproduciendo = false;
        self.visor.pedir(Pedido::Parar);
        self.guardar_preferencias();
    }

    /// Posición del clip en el tiempo global (suma de duraciones previas) y total.
    pub fn desplazamientos(&self) -> (Vec<f64>, f64) {
        let mut offs = Vec::with_capacity(self.clips.len());
        let mut t = 0.0;
        for c in &self.clips {
            offs.push(t);
            t += c.dur();
        }
        (offs, t.max(0.1))
    }

    /// Ventana visible del mapa (zoom y desplazamiento acotados).
    pub fn ventana_mapa(&mut self) -> (Vec<f64>, f64, f64, f64) {
        let (offs, total) = self.desplazamientos();
        let span = total / self.zoom_mapa;
        self.t0_mapa = self.t0_mapa.clamp(0.0, (total - span).max(0.0));
        (offs, total, self.t0_mapa, self.t0_mapa + span)
    }

    /// Ajusta el mapa para que se vea todo.
    pub fn ajustar_mapa(&mut self) {
        self.zoom_mapa = 1.0;
        self.t0_mapa = 0.0;
        self.marcar_sucio();
    }

    /// Acerca el mapa al clip actual.
    pub fn enfocar_clip(&mut self) {
        let Some(i) = self.actual else { return };
        let (offs, total) = self.desplazamientos();
        let d = self.clips[i].dur().max(0.5);
        self.zoom_mapa = (total / (d * 1.1)).clamp(1.0, 200.0);
        self.t0_mapa = offs[i] - d * 0.05;
        self.marcar_sucio();
    }

    /// Clic en el tiempo global: elige el clip que cae ahí y mueve el cabezal (igual que `_map_click`).
    pub fn ir_a_global(&mut self, gt: f64) {
        if self.clips.is_empty() {
            return;
        }
        let (offs, _) = self.desplazamientos();
        let i = offs.iter().filter(|&&o| o <= gt).count().saturating_sub(1).min(self.clips.len() - 1);
        if self.actual != Some(i) {
            self.seleccion = [self.clips[i].clave.clone()].into();
            self.ancla = Some(self.clips[i].clave.clone());
            self.actual = Some(i);
        }
        self.reproduciendo = false;
        self.buscar(gt - offs[i]);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn clip(nombre: &str) -> Clip {
        Clip {
            ruta: PathBuf::from(format!("C:\\x\\{nombre}.MP4")),
            clave: format!("c:\\x\\{}.mp4", nombre.to_lowercase()),
            entrada: None,
            meta: Meta::default(),
            curva: Vec::new(),
            tramos: Vec::new(),
            aprobado: 0.0,
        }
    }

    /// Mayús+clic tras abrir un proyecto con menos clips (antes: pánico) o tras reordenar (antes: elegía
    /// los clips equivocados): el ancla es el clip, no su fila.
    #[test]
    fn ancla_sobrevive_a_menos_clips_y_al_orden() {
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.clips = (0..5).map(|i| clip(&format!("C{i}"))).collect();
        a.clic_fila(4, false, false);
        a.clips.truncate(2);
        a.actual = None;
        a.clic_fila(1, false, true);
        assert!(a.seleccion.contains(&a.clips[1].clave));
        a.clips = (0..5).map(|i| clip(&format!("C{i}"))).collect();
        a.actual = None;
        a.clic_fila(1, false, false);
        a.clips.reverse();
        a.actual = None;
        a.clic_fila(4, false, true);
        assert_eq!(a.seleccion.len(), 2, "{:?}", a.seleccion);
        assert!(a.seleccion.contains(&clip("C1").clave) && a.seleccion.contains(&clip("C0").clave));
    }

    /// Con la unidad del material desconectada al abrir, el proyecto conserva sus fuentes y no se
    /// reescribe hasta que el editor cambie algo (antes se guardaba al instante con `sources: []`).
    #[test]
    fn material_sin_conexion_se_conserva() {
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        let mut d = Map::new();
        d.insert("sources".into(), json!(["Z:\\boda que no esta"]));
        d.insert("out_auto".into(), json!(true));
        a.aplicar_estado(&d);
        assert_eq!(a.fuentes, vec!["Z:\\boda que no esta".to_string()]);
        assert_eq!(a.estado_json()["sources"], json!(["Z:\\boda que no esta"]));
        assert!(a.estado.contains("sin conexión"), "{}", a.estado);
        assert!(a.sucio.is_none(), "abrir no marca cambios");
    }

    /// Carpeta temporal vacía para una prueba.
    fn carpeta_prueba(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("camcuts_gui_prueba_{nombre}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Escribe una caché mínima del clip `ruta` en `out\cache` (con la marca `edition` si se da) y lo
    /// marca en `clips_meta.json` como ya sondeado, para que la prueba no lance ffprobe.
    fn escribir_cache(out: &Path, ruta: &Path, edicion: Option<&str>, muestras: Value) {
        let cache = out.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let mut e = json!({"path": ruta.to_string_lossy(), "fps": [30, 1], "width": 1920, "height": 1080,
                           "duration": 10.0, "per_second": 4.0, "samples": muestras});
        if let Some(ed) = edicion {
            e["edition"] = json!(ed);
        }
        let n = std::fs::read_dir(&cache).map(|r| r.count()).unwrap_or(0);
        let nombre = format!("{}_{n}.json", ruta.file_stem().unwrap().to_string_lossy());
        std::fs::write(cache.join(nombre), e.to_string()).unwrap();
        let mut metas = datos::leer_metas(out);
        metas.insert(clave(ruta), Meta { error: Some(true), ..Default::default() });
        datos::escribir_metas(out, &metas);
    }

    /// Marca `edition` de las cachés de esta edición y de la otra.
    fn marcas_edicion() -> (Option<&'static str>, Option<&'static str>) {
        if motor::bodas() { (Some("bodas"), None) } else { (None, Some("bodas")) }
    }

    /// Una fuente sin conexión recoge sus clips por componentes enteros (`Boda` no recoge los de
    /// `Boda 2`) y las fuentes anidadas no duplican filas.
    #[test]
    fn fuentes_sin_conexion_por_componentes() {
        let base = carpeta_prueba("sin_conexion");
        let out = base.join("resultados");
        let (propia, _) = marcas_edicion();
        let boda = base.join("Boda");
        let c1 = boda.join("C0001.MP4");
        let c2 = boda.join("Cam1").join("C0002.MP4");
        let c3 = base.join("Boda 2").join("C0003.MP4");
        for c in [&c1, &c2, &c3] {
            escribir_cache(&out, c, propia, json!([]));
        }
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        // las dos fuentes faltan en el disco (unidad desconectada) y una está dentro de la otra
        a.fuentes = vec![boda.to_string_lossy().to_string(), boda.join("Cam1").to_string_lossy().to_string()];
        a.out = out.to_string_lossy().to_string();
        a.out_auto = false;
        a.cargar_clips();
        let nombres: Vec<String> = a.clips.iter().map(|c| c.nombre()).collect();
        assert_eq!(nombres, ["C0001.MP4", "C0002.MP4"], "sin C0003 de «Boda 2» y sin filas repetidas");
        assert!(a.clips.iter().all(|c| c.entrada.is_some()));
        // sin subcarpetas, «Boda» solo recoge lo de su nivel (C0002 llega por la fuente «Cam1»)
        a.recursivo = false;
        a.fuentes.truncate(1);
        a.cargar_clips();
        let nombres: Vec<String> = a.clips.iter().map(|c| c.nombre()).collect();
        assert_eq!(nombres, ["C0001.MP4"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// La carpeta de resultados automática de una carpeta sin conexión es la suya, no la de su madre,
    /// y cada edición tiene la suya.
    #[test]
    fn carpeta_automatica_sin_conexion_y_por_edicion() {
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        let boda = std::env::temp_dir().join("camcuts_gui_prueba_no_existe").join("Boda");
        let esperada = camcuts_core::edicion::carpeta_resultados(&boda).to_string_lossy().to_string();
        a.fuentes = vec![boda.to_string_lossy().to_string()];
        assert_eq!(a.out_automatica(), esperada);
        a.fuentes = vec![boda.join("C0001.MP4").to_string_lossy().to_string()];
        assert_eq!(a.out_automatica(), esperada);
        assert_eq!(a.carpeta_material(), Some(boda.clone()));
        let nombre = Path::new(&esperada).file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(nombre, if motor::bodas() { "CAMCUTS Bodas" } else { "CAMCUTS" });
    }

    /// Un proyecto de la otra edición se abre en solo lectura: el autoguardado no lo pisa (ni a uno que
    /// esté en la ruta por defecto); "Guardar como" guarda una copia de esta edición.
    #[test]
    fn proyecto_de_otra_edicion_no_se_pisa() {
        use camcuts_core::edicion::{actual, Edicion};
        let otra = if actual() == Edicion::Bodas { Edicion::Joyas } else { Edicion::Bodas };
        let base = carpeta_prueba("app_proyecto_ajeno");
        let p = base.join("boda.camcuts");
        let texto = format!(r#"{{"format": "camcuts-project", "version": 1, "app": "{}", "sources": [], "criterion": "todo"}}"#, otra.nombre_app());
        std::fs::write(&p, &texto).unwrap();
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.abrir_proyecto(&p);
        assert!(a.solo_lectura, "proyecto de {} abierto en {}", otra.nombre_app(), motor::nombre_app());
        assert!(a.aviso.is_some(), "se avisa al abrirlo");
        a.marcar_sucio();
        a.guardar_proyecto(None);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), texto, "el autoguardado no reescribe el proyecto ajeno");
        // "Guardar como" (ruta elegida): una copia de esta edición, que ya se autoguarda
        let copia = base.join("copia.camcuts");
        a.guardar_proyecto(Some(copia.clone()));
        assert!(!a.solo_lectura);
        let d = proyecto::abrir(&copia).unwrap();
        assert_eq!(d["app"], motor::nombre_app());

        // trabajo nuevo cuya ruta por defecto ya tiene un proyecto de la otra edición: tampoco se pisa
        let material = base.join("Boda");
        std::fs::create_dir_all(&material).unwrap();
        let mut b = App::nuevo(&ctx);
        b.persistir = false;
        b.fuentes = vec![material.to_string_lossy().to_string()];
        b.out = b.out_automatica();
        let defecto = proyecto::ruta_por_defecto(&b.out, &b.fuentes).unwrap();
        std::fs::create_dir_all(defecto.parent().unwrap()).unwrap();
        std::fs::write(&defecto, &texto).unwrap();
        b.guardar_proyecto(None);
        assert_eq!(std::fs::read_to_string(&defecto).unwrap(), texto);
        assert!(b.ruta_proyecto.is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Las cachés de la otra edición no se cargan: el clip queda pendiente para volver a analizarse.
    #[test]
    fn cache_de_otra_edicion_queda_pendiente() {
        let base = carpeta_prueba("cache_ajena");
        let out = base.join("resultados");
        let (propia, ajena) = marcas_edicion();
        escribir_cache(&out, &base.join("Boda").join("C0001.MP4"), propia, json!([]));
        escribir_cache(&out, &base.join("Boda").join("C0002.MP4"), ajena, json!([]));
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.abrir_resultados(&out);
        let nombres: Vec<String> = a.clips.iter().map(|c| c.nombre()).collect();
        assert_eq!(nombres, ["C0001.MP4"], "la caché de la otra edición no se muestra como analizada");
        // con el material conectado, el clip de la otra edición sale pendiente (Analizar lo rehace)
        let boda = base.join("Boda");
        std::fs::create_dir_all(&boda).unwrap();
        for n in ["C0001.MP4", "C0002.MP4"] {
            std::fs::write(boda.join(n), b"").unwrap();
        }
        a.fuentes = vec![boda.to_string_lossy().to_string()];
        a.cargar_clips();
        let estado: Vec<(String, bool)> = a.clips.iter().map(|c| (c.nombre(), c.entrada.is_some())).collect();
        assert_eq!(estado, [("C0001.MP4".to_string(), true), ("C0002.MP4".to_string(), false)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Detector de joyas: el propio por defecto; se guarda en el proyecto con su versión (un "world" de
    /// antes pasa al propio), va al análisis como `--detector`, y los clips analizados con el otro salen
    /// "Reanalizar · detector".
    #[test]
    fn detector_en_proyecto_estado_y_analisis() {
        if motor::bodas() {
            return; // Bodas usa siempre su YOLO-World de boda: no hay selector
        }
        let base = carpeta_prueba("detector");
        let out = base.join("resultados");
        let material = base.join("Material");
        std::fs::create_dir_all(&material).unwrap();
        let (c_viejo, c_propio) = (material.join("C0001.MP4"), material.join("C0002.MP4"));
        for c in [&c_viejo, &c_propio] {
            std::fs::write(c, b"").unwrap();
        }
        // una caché sin clave (YOLO-World, como las de Python) y una del propio
        escribir_cache(&out, &c_viejo, None, json!([]));
        escribir_cache(&out, &c_propio, None, json!([]));
        let f = std::fs::read_dir(out.join("cache")).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("C0002")).unwrap();
        let mut v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        v["detector"] = json!("propio");
        std::fs::write(&f, v.to_string()).unwrap();

        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.detector = DetectorKind::default();
        assert_eq!(a.detector, DetectorKind::Propio, "por defecto, el propio");
        a.fuentes = vec![material.to_string_lossy().to_string()];
        a.out = out.to_string_lossy().to_string();
        a.out_auto = false;
        a.cargar_clips();
        let estados = |a: &App| -> Vec<String> { a.clips.iter().map(|c| a.estado_de(c)).collect() };
        // la caché sin clave es del clásico: con el propio elegido, a reanalizar
        assert_eq!(estados(&a), ["Reanalizar · detector", "Analizado"]);
        a.cambiar_detector(DetectorKind::World);
        assert_eq!(estados(&a), ["Analizado", "Reanalizar · detector"]);
        assert!(a.estado.contains("1 clip(s)"), "{}", a.estado);

        // proyecto: se guarda y se restaura con su versión
        let d = a.estado_json();
        assert_eq!(d["detector"], "world");
        assert_eq!(d[CLAVE_DETECTOR_V], DETECTOR_V);
        let mut b = App::nuevo(&ctx);
        b.persistir = false;
        b.aplicar_estado(&d);
        assert_eq!(b.detector, DetectorKind::World, "elegido a mano desde la versión 2: se respeta");
        // un estado de antes (el "world" era solo el predeterminado de entonces) o sin clave: el propio
        let mut viejo = d.clone();
        viejo.remove(CLAVE_DETECTOR_V);
        b.aplicar_estado(&viejo);
        assert_eq!(b.detector, DetectorKind::Propio);
        let mut sin = viejo.clone();
        sin.remove("detector");
        b.detector = DetectorKind::World;
        b.aplicar_estado(&sin);
        assert_eq!(b.detector, DetectorKind::Propio);

        // análisis: `--detector` solo para el `--analizar` de esta app (no para una CLI externa)
        let propia = vec![analisis::ARG_ANALIZAR.to_string()];
        assert_eq!(a.args_detector(&propia), ["--detector", "world"]);
        assert_eq!(b.args_detector(&propia), ["--detector", "propio"]);
        assert!(a.args_detector(&[]).is_empty());

        // un respaldo del clásico vale para el propio: cambiar al propio no anuncia reanálisis
        let f = std::fs::read_dir(out.join("cache")).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("C0001")).unwrap();
        let mut v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        v["detector"] = json!("world");
        v["respaldo"] = json!("propio");
        std::fs::write(&f, v.to_string()).unwrap();
        a.cargar_clips();
        a.cambiar_detector(DetectorKind::Propio);
        assert!(!a.estado.contains("otro detector"), "el respaldo no se rehace: {}", a.estado);
        assert_eq!(estados(&a), ["Analizado", "Analizado"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Si `clips_meta.json` no se puede escribir, lo sondeado se queda en la sesión y no se vuelve a
    /// sondear sin fin.
    #[test]
    fn metas_sin_poder_escribir_no_se_resondean() {
        let base = carpeta_prueba("metas_sin_escribir");
        let material = base.join("Material");
        std::fs::create_dir_all(&material).unwrap();
        let c = material.join("C0001.MP4");
        std::fs::write(&c, b"").unwrap();
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.fuentes = vec![material.to_string_lossy().to_string()];
        // una carpeta de resultados imposible (unidad que no existe)
        a.out = "Q:\\no_existe_camcuts".into();
        a.out_auto = false;
        a.cargar_clips();
        assert!(a.sondeando, "el clip sin metadatos se sondea");
        // llega el sondeo (que no se pudo guardar en el disco)
        a.recibir_metas([(clave(&c), datos::sondear_meta(&c))].into());
        a.cargar_clips();
        assert!(!a.sondeando, "no se lanza otro sondeo");
        assert!(a.clips[0].meta.codec.is_some() || a.clips[0].meta.error.is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Proyecto de la otra edición: no pisa el `clip_types.json` de esta; "Abrir resultados" quita el
    /// solo lectura; y con un análisis en curso no se cambia de proyecto.
    #[test]
    fn proyecto_ajeno_tipos_resultados_y_analisis_en_curso() {
        use camcuts_core::edicion::{actual, Edicion};
        let otra = if actual() == Edicion::Bodas { Edicion::Joyas } else { Edicion::Bodas };
        let base = carpeta_prueba("ajeno_tipos");
        let material = base.join("Material");
        std::fs::create_dir_all(&material).unwrap();
        // tipos de esta edición ya guardados en su carpeta de resultados
        let out = camcuts_core::edicion::carpeta_resultados(&material);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("clip_types.json"), r#"{"x": "anillo"}"#).unwrap();
        let p = base.join("ajeno.camcuts");
        let texto = format!(
            r#"{{"format": "camcuts-project", "version": 1, "app": "{}", "sources": [{}], "clip_types": {{"x": "votos"}}}}"#,
            otra.nombre_app(),
            json!(material.to_string_lossy())
        );
        std::fs::write(&p, &texto).unwrap();
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.abrir_proyecto(&p);
        assert!(a.solo_lectura);
        assert_eq!(PathBuf::from(&a.out), out);
        let tipos = std::fs::read_to_string(out.join("clip_types.json")).unwrap();
        assert!(tipos.contains("anillo") && !tipos.contains("votos"), "{tipos}");

        // con un análisis en curso no se abre otro proyecto (ni se pierde lo pendiente)
        a.avance_clips.insert("x".into(), Some(10));
        a.aviso = None;
        let antes = a.ruta_proyecto.clone();
        let otro = base.join("otro.camcuts");
        std::fs::write(&otro, &texto).unwrap();
        a.abrir_proyecto(&otro);
        assert_eq!(a.ruta_proyecto, antes, "no cambia de proyecto");
        assert!(a.aviso.as_ref().map(|v| v.texto.contains("análisis")).unwrap_or(false));
        a.avance_clips.clear();

        // "Abrir resultados existentes": ya no es el proyecto ajeno, se autoguarda
        a.ruta_proyecto = None;
        a.abrir_resultados(&out);
        assert!(!a.solo_lectura);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// La píldora de color dice la conversión que aplica el visor: con "auto" y un S-Log3 de Sony,
    /// "S-Log3 → 709 (auto)".
    #[test]
    fn pildora_con_la_conversion_del_visor() {
        assert_eq!(motor::texto_pildora_color("auto", "auto", true, "Automático"), "LOG → contraste");
        assert_eq!(motor::texto_pildora_color("auto", "auto", false, "Automático"), "Automático");
        assert_eq!(motor::texto_pildora_color("none", "none", true, "Automático"), "Sin conversión");
        assert_eq!(motor::texto_pildora_color("cube", "cube", false, "Automático"), "LUT .cube");
        let dir = carpeta_prueba("pildora");
        let c = dir.join("C0001.MP4");
        std::fs::write(&c, b"").unwrap();
        std::fs::write(dir.join("C0001M01.XML"), r#"<Item name="CaptureGammaEquation" value="s-log3-cine"/><Item name="CaptureColorPrimaries" value="s-gamut3-cine"/>"#).unwrap();
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        let mut k = clip("C0001");
        k.ruta = c.clone();
        k.clave = clave(&c);
        a.clips = vec![k];
        a.actual = Some(0);
        let t = a.texto_color_actual("Automático");
        assert!(t.contains("S-Log3") && t.ends_with("(auto)"), "{t}");
        // elegido a mano: lo elegido
        a.colores.insert(clave(&c), ColorSpec { id: "hlg".into(), cube: None });
        assert!(!a.texto_color_actual("Automático").contains("(auto)"));
        // material real (solo lectura) si está en este equipo
        let real = Path::new("D:\\CONTENIDO NAPO\\2026\\Noviembre\\3\\Videos Yo Manos Detalle\\C9262.MP4");
        if real.exists() {
            a.colores.clear();
            let mut k = clip("C9262");
            k.ruta = real.to_path_buf();
            k.clave = clave(real);
            a.clips = vec![k];
            assert!(a.texto_color_actual("Automático").contains("S-Log3"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Si la búsqueda del clip nuevo no da cuadro, el visor no se queda con la imagen del anterior.
    #[test]
    fn cuadro_fallido_quita_la_imagen_anterior() {
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.clips = vec![clip("C0")];
        a.actual = Some(0);
        a.tex_visor = Some(ctx.load_texture("visor", egui::ColorImage::from_rgb([2, 2], &[200u8; 12]), TextureOptions::LINEAR));
        a.visor.salida.lock().unwrap().sin_cuadro = Some(7);
        a.recoger_cuadro();
        assert!(a.tex_visor.is_none(), "la imagen anterior se quita");
        assert!(a.sin_cuadro);
        // y un cuadro nuevo la vuelve a poner
        a.visor.salida.lock().unwrap().cuadro = Some(video::Cuadro { t: 0.0, ancho: 2, alto: 2, rgb: vec![10; 12], serie: 8 });
        a.visor.salida.lock().unwrap().sin_cuadro = None;
        a.recoger_cuadro();
        assert!(a.tex_visor.is_some() && !a.sin_cuadro);
    }

    /// Las líneas `progreso` del análisis dan el porcentaje de cada clip: el que avanza, 0 % los que
    /// esperan, "error" el que falló; al terminar el análisis todo vuelve a lo de siempre.
    #[test]
    fn porcentaje_por_clip() {
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.clips = (0..4).map(|i| clip(&format!("C{i}"))).collect();
        assert!(a.clips.iter().all(|c| a.avance_de(c).is_none()), "sin análisis no hay porcentajes");
        a.progreso = Some((0, 4));
        for l in ["progreso\t0\tC:\\x\\C0.MP4", "progreso\t42\tC:\\x\\C0.MP4", "progreso\terror\tC:\\x\\C1.MP4", "progreso\t100\tC:\\x\\C2.MP4"] {
            a.tx.send(Mensaje::Linea(l.into())).unwrap();
        }
        a.atender_mensajes();
        assert_eq!(a.avance_de(&a.clips[0]), Some(Some(42)), "el que avanza");
        assert!(a.estado.contains("C0.MP4 42%"), "la barra de estado lo dice: {}", a.estado);
        assert_eq!(a.avance_de(&a.clips[1]), Some(None), "el que falló");
        assert_eq!(a.avance_de(&a.clips[2]), Some(Some(100)), "terminado, con su caché aún sin leer");
        assert_eq!(a.avance_de(&a.clips[3]), Some(Some(0)), "en cola");
        assert_eq!(a.clips.len(), 4, "una línea de avance no relee la lista");
        a.tx.send(Mensaje::Fin(0)).unwrap();
        a.atender_mensajes();
        assert!(a.avance_clips.is_empty(), "al terminar se quitan los porcentajes");
    }

    /// Encender el mapa lo hace visible: muestra la línea de tiempo si estaba oculta y le da alto para la
    /// franja (antes, con el alto de siempre, el botón no hacía nada).
    #[test]
    fn encender_el_mapa_se_ve() {
        if motor::bodas() {
            return; // en Bodas ese botón es otro panel
        }
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.paneles.tiempo = false;
        a.paneles.mapa = false;
        a.alto_tiempo = 120.0;
        let antes = a.paneles;
        a.paneles.mapa = true;
        a.al_cambiar_paneles(antes);
        assert!(a.paneles.tiempo, "la línea de tiempo se muestra");
        assert!(a.alto_tiempo >= 200.0, "con alto para el mapa: {}", a.alto_tiempo);
        // apagarlo no toca nada más
        let antes = a.paneles;
        a.paneles.mapa = false;
        a.al_cambiar_paneles(antes);
        assert!(a.paneles.tiempo && a.alto_tiempo >= 200.0);
    }
}