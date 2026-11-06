//! Qué detector de joyas usa el análisis: el propio (por defecto desde el 2026-11-05), un YOLO11
//! entrenado con el material de la marca (`camcuts-infer/src/trained.rs`), o YOLO-World (el clásico, el
//! de Python).
//!
//! El tipo vive en el núcleo (y `camcuts-infer` lo reexporta) porque la elección cambia cosas del
//! núcleo, no solo de los modelos:
//! - la caché de cada clip guarda con qué detector se analizó (clave [`CLAVE_CACHE`]); una caché hecha
//!   con el otro detector cuenta como vieja y se vuelve a analizar, igual que cuando cambia el color.
//!   Las cachés sin la clave (las de Python y las anteriores a esta opción) son de YOLO-World;
//! - los clips en paralelo: YOLO-World x con 4 clips a la vez reinició el driver de NVIDIA (TDR) en la
//!   RTX 5060 de 8 GB, así que con él hay un tope (ver `hardware::parallel_clips`);
//! - el respaldo automático: con el propio, los clips en los que casi nunca da la joya nítida
//!   ([`necesita_respaldo`]) se vuelven a analizar con YOLO-World, y su caché lleva la marca
//!   [`CLAVE_RESPALDO`] para que valga también para el propio (no se repite la próxima vez).
//!
//! La edición Bodas tiene su propio YOLO-World de boda: allí la elección no aplica y sus cachés no
//! llevan la clave.

use serde_json::Value;

use crate::project::ClipEntry;
use crate::scoring;

/// Clave de la caché de un clip con el detector que lo analizó (`"world"` o `"propio"`).
pub const CLAVE_CACHE: &str = "detector";

/// Clave de la caché que marca un análisis de respaldo: YOLO-World volvió a analizar un clip en el que el
/// detector propio casi nunca daba la joya nítida. Su valor es el detector que se pidió (`"propio"`).
pub const CLAVE_RESPALDO: &str = "respaldo";

/// Clave de la caché que marca un clip analizado con el detector propio porque el pedido no cabía en la
/// GPU: en modo ligero (DaVinci abierto) se pidió YOLO-World, pero su versión mediana confunde las joyas
/// de la marca (en "Videos Yo Manos Detalle", 33 clips de anillos: casi todo "pulsera", 1318 s aprobados
/// frente a 2083 s del grande y 2134 s del propio). Su valor es el detector que se pidió (`"world"`), y la
/// caché vale para él: no se repite al volver a tener memoria.
pub const CLAVE_EN_LUGAR: &str = "en_lugar_de";

/// Respaldo: un clip pasa a YOLO-World si el propio aprueba menos de esta fracción del tiempo en que ve
/// joya. En las pruebas (37 clips de Octubre 27, Candongas y Anillo) todos daban 0.5 o más salvo C9181,
/// una pulsera macro con poca profundidad de campo (0.09: el propio aprobaba 1.8 s y YOLO-World 61 s), así
/// que 0.25 deja margen a los dos lados.
pub const RESPALDO_FRACCION: f64 = 0.25;

/// Respaldo: segundos con joya que hacen falta para mirar la regla (con menos, el clip apenas tiene joya
/// y volver a analizarlo no compensa).
pub const RESPALDO_MIN_JOYA_S: f64 = 2.0;

/// Variable de entorno que elige el detector cuando no se pasa `--detector`.
pub const VARIABLE: &str = "CAMCUTS_DETECTOR";

/// Qué detector usa el análisis (`--detector propio|world`, variable `CAMCUTS_DETECTOR`).
///
/// Por defecto el propio desde el 2026-11-05: en 33 clips de anillos aprobó 2134 s frente a 2083 s de
/// YOLO-World x, con las etiquetas bien (anillo/topo), y es más rápido; los clips macro que no ve nítidos
/// pasan solos por el clásico (respaldo automático). Las cachés SIN clave siguen siendo de YOLO-World
/// ([`DetectorKind::de_cache`]), que es con lo que se hicieron.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetectorKind {
    /// YOLO-World x/m con prompts (el de siempre, igual que Python).
    World,
    /// El YOLO11 entrenado con el material de la empresa. Por defecto.
    #[default]
    Propio,
}

impl DetectorKind {
    /// `propio` / `world` (sin distinguir mayúsculas; también `own`, `trained`, `yolo-world`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "propio" | "own" | "trained" => Some(Self::Propio),
            "world" | "yolo-world" | "yoloworld" => Some(Self::World),
            _ => None,
        }
    }

    /// Nombre corto, el de la línea de comandos, la caché y el proyecto: `world` o `propio`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Propio => "propio",
        }
    }

    /// El elegido en la variable de entorno `CAMCUTS_DETECTOR` (por defecto el propio).
    pub fn from_env() -> Self {
        Self::resolver(None, std::env::var(VARIABLE).ok().as_deref())
    }

    /// Elección final: la opción `--detector` si se dio, si no la variable de entorno, si no el
    /// propio. Un valor que no se entiende cuenta como no dado.
    pub fn resolver(opcion: Option<&str>, entorno: Option<&str>) -> Self {
        opcion.and_then(Self::parse).or_else(|| entorno.and_then(Self::parse)).unwrap_or_default()
    }

    /// Quita `--detector X` (o `--detector=X`) de una lista de argumentos y lo publica en
    /// `CAMCUTS_DETECTOR` para que lo lean los detectores de este proceso. Pensado para binarios cuyo
    /// parser no conoce la opción (la CLI del núcleo ya la conoce). Devuelve lo elegido.
    pub fn take_arg(args: &mut Vec<String>) -> Option<Self> {
        let mut found = None;
        let mut i = 0;
        while i < args.len() {
            if let Some(v) = args[i].strip_prefix("--detector=") {
                found = Self::parse(v);
                args.remove(i);
                continue;
            }
            if args[i] == "--detector" && i + 1 < args.len() {
                found = Self::parse(&args[i + 1]);
                args.drain(i..i + 2);
                continue;
            }
            i += 1;
        }
        if let Some(k) = found {
            std::env::set_var(VARIABLE, k.as_str());
        }
        found
    }

    /// Detector con el que se analizó una caché: su clave `detector`; sin clave (Python, cachés
    /// anteriores a esta opción) o con un valor desconocido, YOLO-World.
    pub fn de_cache(entry: &ClipEntry) -> Self {
        // sin clave = YOLO-World (no el detector por defecto: esas cachés se hicieron con YOLO-World)
        entry.extra.get(CLAVE_CACHE).and_then(Value::as_str).and_then(Self::parse).unwrap_or(Self::World)
    }
}

/// ¿Sirve una caché para un análisis con el detector `quiero`? En la edición Bodas siempre (usa su
/// propio detector y la elección no aplica); en la de joyas, solo si se hizo con el mismo detector, si
/// es un respaldo de YOLO-World y se pide el propio (ese clip ya se vio que necesita el clásico), o si el
/// propio lo analizó en lugar del pedido porque este no cabía en la GPU ([`CLAVE_EN_LUGAR`]).
pub fn cache_vigente(entry: &ClipEntry, quiero: DetectorKind) -> bool {
    crate::edicion::es_bodas()
        || DetectorKind::de_cache(entry) == quiero
        || (quiero == DetectorKind::Propio && es_respaldo(entry))
        || en_lugar_de(entry) == Some(quiero)
}

/// Detector que se pidió para una caché que el propio hizo en su lugar (modo ligero), si es el caso.
pub fn en_lugar_de(entry: &ClipEntry) -> Option<DetectorKind> {
    entry.extra.get(CLAVE_EN_LUGAR).and_then(Value::as_str).and_then(DetectorKind::parse)
}

/// ¿Es la caché un análisis de respaldo (YOLO-World en lugar del propio)?
pub fn es_respaldo(entry: &ClipEntry) -> bool {
    entry.extra.get(CLAVE_RESPALDO).and_then(Value::as_str) == Some(DetectorKind::Propio.as_str())
}

/// ¿Hay que volver a analizar este clip con YOLO-World? Solo cachés del detector propio (las de respaldo
/// ya son de YOLO-World) con al menos [`RESPALDO_MIN_JOYA_S`] s de joya, de las que el criterio "joyas" sin
/// calibrar aprueba menos de [`RESPALDO_FRACCION`]: la joya está, pero el propio casi nunca la da nítida
/// (tomas macro con poca profundidad de campo, donde su caja cubre la parte desenfocada de la pieza).
/// Se usa siempre el mismo criterio, no el elegido: con "social" o un filtro de joyas la fracción baja en
/// todos los clips y la decisión no debe cambiar con eso.
pub fn necesita_respaldo(entry: &ClipEntry) -> bool {
    if crate::edicion::es_bodas() || DetectorKind::de_cache(entry) != DetectorKind::Propio || entry.samples.is_empty() {
        return false;
    }
    let Some(preset) = scoring::preset("joyas", None) else { return false };
    // las mismas etiquetas estables que al puntuar (sobre una copia: la caché no cambia)
    let mut ss = entry.samples.clone();
    scoring::refine_jewels(&mut ss, entry.width, entry.height);
    scoring::stabilize_labels(&mut ss, entry.per_second, 1.0);
    // tiempo con joya contado por muestras (cada una vale 1/per_second), y la duración que de verdad se
    // muestreó: en un archivo cortado las muestras acaban mucho antes de la duración nominal, y repartir
    // esa duración entre las muestras inflaba el tiempo con joya (y el respaldo saltaba sin motivo)
    let ps = entry.per_second.max(1e-9);
    let joya_s = ss.iter().filter(|s| s.jewel).count() as f64 / ps;
    if joya_s < RESPALDO_MIN_JOYA_S {
        return false;
    }
    let muestreado = ss.last().map(|s| s.t + 1.0 / ps).unwrap_or(0.0).min(entry.duration);
    let aprobado: f64 = scoring::build_segments(&ss, &preset, entry.per_second, muestreado).iter().map(|s| s.end - s.start).sum();
    aprobado < RESPALDO_FRACCION * joya_s
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Caché mínima con (o sin) la clave `detector`.
    fn entrada(marca: Option<&str>) -> ClipEntry {
        let mut v = serde_json::json!({"path": "C0001.MP4", "fps": [30, 1], "width": 1920, "height": 1080,
                                       "duration": 2.0, "per_second": 4.0, "samples": []});
        if let Some(m) = marca {
            v[CLAVE_CACHE] = serde_json::json!(m);
        }
        serde_json::from_value(v).unwrap()
    }

    /// Nombres y alias de la opción.
    #[test]
    fn nombres() {
        assert_eq!(DetectorKind::parse("Propio"), Some(DetectorKind::Propio));
        assert_eq!(DetectorKind::parse(" world "), Some(DetectorKind::World));
        assert_eq!(DetectorKind::parse("yolo-world"), Some(DetectorKind::World));
        assert_eq!(DetectorKind::parse("otro"), None);
        for k in [DetectorKind::World, DetectorKind::Propio] {
            assert_eq!(DetectorKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(DetectorKind::default(), DetectorKind::Propio);
    }

    /// La opción manda sobre la variable; sin ninguna (o con basura), el propio.
    #[test]
    fn opcion_variable_y_defecto() {
        assert_eq!(DetectorKind::resolver(None, None), DetectorKind::Propio);
        assert_eq!(DetectorKind::resolver(None, Some("world")), DetectorKind::World);
        assert_eq!(DetectorKind::resolver(Some("world"), Some("propio")), DetectorKind::World);
        assert_eq!(DetectorKind::resolver(Some("propio"), None), DetectorKind::Propio);
        assert_eq!(DetectorKind::resolver(None, Some("x")), DetectorKind::Propio);
    }

    /// Cachés sin clave = YOLO-World; una caché del otro detector no vale (en joyas).
    #[test]
    fn cache_de_otro_detector_es_vieja() {
        assert_eq!(DetectorKind::de_cache(&entrada(None)), DetectorKind::World);
        assert_eq!(DetectorKind::de_cache(&entrada(Some("world"))), DetectorKind::World);
        assert_eq!(DetectorKind::de_cache(&entrada(Some("propio"))), DetectorKind::Propio);
        if !crate::edicion::es_bodas() {
            assert!(cache_vigente(&entrada(None), DetectorKind::World));
            assert!(!cache_vigente(&entrada(None), DetectorKind::Propio));
            assert!(cache_vigente(&entrada(Some("propio")), DetectorKind::Propio));
            assert!(!cache_vigente(&entrada(Some("propio")), DetectorKind::World));
        }
    }

    /// Caché de `segundos` s a 4 muestras/s con joya en todas, siempre borrosa (nada se aprueba).
    fn borrosa(marca: &str, segundos: f64) -> ClipEntry {
        let mut e = entrada(Some(marca));
        e.duration = segundos;
        e.samples = (0..(segundos * 4.0) as usize)
            .map(|i| crate::jewel::Sample {
                t: i as f64 / 4.0,
                jewel: true,
                label: "pulsera".into(),
                conf: 0.8,
                bbox: vec![400, 200, 560, 340],
                area_frac: 0.05,
                jewel_lap_p99: 4.0,
                ..Default::default()
            })
            .collect();
        e
    }

    /// Una caché de respaldo (YOLO-World marcado) vale para el propio y para YOLO-World.
    #[test]
    fn cache_de_respaldo_vale_para_los_dos() {
        if crate::edicion::es_bodas() {
            return;
        }
        let mut e = entrada(Some("world"));
        e.extra.insert(CLAVE_RESPALDO.into(), serde_json::json!("propio"));
        assert!(es_respaldo(&e));
        assert!(cache_vigente(&e, DetectorKind::Propio));
        assert!(cache_vigente(&e, DetectorKind::World));
        assert!(!es_respaldo(&entrada(Some("world"))));
    }

    /// El propio en lugar de YOLO-World (modo ligero): vale para los dos y puede necesitar respaldo.
    #[test]
    fn cache_en_lugar_del_clasico() {
        if crate::edicion::es_bodas() {
            return;
        }
        let mut e = entrada(Some("propio"));
        e.extra.insert(CLAVE_EN_LUGAR.into(), serde_json::json!("world"));
        assert_eq!(en_lugar_de(&e), Some(DetectorKind::World));
        assert!(cache_vigente(&e, DetectorKind::World));
        assert!(cache_vigente(&e, DetectorKind::Propio));
        assert_eq!(en_lugar_de(&entrada(Some("propio"))), None);
        let mut b = borrosa("propio", 20.0);
        b.extra.insert(CLAVE_EN_LUGAR.into(), serde_json::json!("world"));
        assert!(necesita_respaldo(&b));
    }

    /// Joya presente y nunca nítida con el propio: respaldo. Con YOLO-World, un respaldo ya hecho o casi
    /// sin joya, no.
    #[test]
    fn regla_del_respaldo() {
        if crate::edicion::es_bodas() {
            return;
        }
        assert!(necesita_respaldo(&borrosa("propio", 20.0)));
        assert!(!necesita_respaldo(&borrosa("world", 20.0)));
        let mut hecho = borrosa("world", 20.0);
        hecho.extra.insert(CLAVE_RESPALDO.into(), serde_json::json!("propio"));
        assert!(!necesita_respaldo(&hecho));
        assert!(!necesita_respaldo(&borrosa("propio", 1.5)), "con menos de 2 s de joya no compensa");
        assert!(!necesita_respaldo(&entrada(Some("propio"))), "sin muestras");
    }

    /// Archivo cortado: 60 s nominales pero solo 10 s muestreados (40 muestras), todo con joya y ~3 s
    /// aprobados. La fracción es de lo muestreado (3 de 10, no 3 de 60): no necesita respaldo.
    #[test]
    fn cortado_cuenta_lo_muestreado() {
        if crate::edicion::es_bodas() {
            return;
        }
        let mut e = borrosa("propio", 10.0);
        e.duration = 60.0;
        for s in e.samples.iter_mut().take(12) {
            s.jewel_lap_p99 = 300.0;
            s.focus_ratio = 3.0;
            s.anisotropy = 1.0;
        }
        // comprobación de la prueba: lo aprobado cae entre el 25 % de lo muestreado y el de lo nominal
        let preset = scoring::preset("joyas", None).unwrap();
        let mut ss = e.samples.clone();
        scoring::refine_jewels(&mut ss, e.width, e.height);
        scoring::stabilize_labels(&mut ss, e.per_second, 1.0);
        let aprobado: f64 = scoring::build_segments(&ss, &preset, e.per_second, 10.0).iter().map(|s| s.end - s.start).sum();
        assert!(aprobado >= RESPALDO_FRACCION * 10.0 && aprobado < RESPALDO_FRACCION * 60.0, "aprobado {aprobado}");
        assert!(!necesita_respaldo(&e), "con la duración nominal salía respaldo");
    }
}
