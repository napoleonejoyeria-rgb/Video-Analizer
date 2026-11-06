//! Detector propio de joyas: un YOLO11 de vocabulario cerrado entrenado con el material de Napoleone
//! (`training/` del workspace), alternativa más rápida a YOLO-World x.
//!
//! - Modelo: `models/jewel_det.onnx` (o `jewel_det_fp16.onnx`), exportado con Ultralytics como
//!   YOLO-World: entrada `images` f32 `[lote, 3, alto, ancho]` dinámica, salida `[lote, 4 + clases, anclas]`
//!   con las puntuaciones ya con sigmoide. Mismo letterbox (960, stride 32, relleno 114), mismo NMS y
//!   mismo reescalado que [`crate::yolo`], así que se reutiliza tal cual.
//! - Clases y umbrales: `models/jewel_det.classes.json`
//!   (`{"names": [...], "imgsz": 960, "thresholds": {"anillo": 0.3, ...}}`). Los nombres ya son las
//!   etiquetas en español que usa el núcleo (`anillo`, `collar`, `cadena`, `arete`, `topo`, `pulsera`,
//!   `dije`, `persona`, `cara`), así que [`camcuts_core::Detection`] sale igual que con YOLO-World.
//! - Confianza: cada clase de joya tiene su umbral de trabajo `u` (0.4, elegido en la validación revisada
//!   a mano). Por encima se reescala para que el umbral caiga en 0.15, el `min_conf` de los presets:
//!   `c' = 0.15 + (c − u)·0.85/(1 − u)`. Personas y caras salen sin tocar.
//! - Detecciones débiles: con `weak_threshold` (`lo`) en el JSON, las joyas con `lo ≤ c < u` no se tiran:
//!   se llevan a la banda débil del núcleo, `c' = 0.021 + (c − lo)·0.128/(u − lo)` (entre 0.021 y 0.149),
//!   y el núcleo las pasa por el verificador CLIP como hace con YOLO-World (`verify_weak`: hasta 4 por
//!   cuadro que no solapen una fuerte, aceptadas con p ≥ 0.6). Así se recuperan piezas que el detector ve
//!   con poca confianza (planos macro de una pieza que llena el cuadro) sin bajar el umbral de las demás.
//!   CLIP solo se carga si llega alguna débil. Por debajo de `lo` no se devuelve nada.

use std::path::Path;

use anyhow::{Context, Result};
use camcuts_core::Detection;

use crate::runtime::Device;
use crate::yolo::Yolo;

/// `min_conf` de los presets del núcleo: el umbral propio de cada clase se lleva a este valor.
pub const CORE_MIN_CONF: f32 = 0.15;
/// Confianza mínima de personas y caras (el núcleo usa ≥ 0.25 y ≥ 0.4).
pub const PEOPLE_CONF: f32 = 0.10;
/// Nombre base del modelo en `models/`.
pub const STEM: &str = "jewel_det";
/// IoU por encima del cual dos joyas de distinta clase se consideran la misma pieza.
pub const CROSS_IOU: f32 = 0.6;
/// Banda débil del núcleo a la que se llevan las detecciones entre `weak_threshold` y el umbral de
/// trabajo: el núcleo verifica con CLIP las joyas con 0.02 ≤ conf < 0.15. Se empieza en 0.021 y no en
/// 0.02 porque el núcleo compara en f64 y 0.02 en f32 vale 0.0199999…, que quedaría fuera; y se acaba en
/// 0.149 para que ninguna débil llegue a 0.15 por redondeo y cuente como fuerte sin pasar por CLIP.
pub const WEAK_BAND: (f32, f32) = (0.021, 0.149);

/// Qué detector usa el análisis (`--detector propio|world`, variable `CAMCUTS_DETECTOR`). Vive en el
/// núcleo (`camcuts_core::detector`) porque la caché de cada clip guarda con cuál se analizó y el núcleo
/// limita los clips en paralelo con YOLO-World; aquí se reexporta con el mismo nombre de siempre.
pub use camcuts_core::detector::DetectorKind;

/// Clases y umbrales del modelo (`jewel_det.classes.json`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClassesFile {
    pub names: Vec<String>,
    #[serde(default = "default_imgsz")]
    pub imgsz: usize,
    /// umbral de trabajo por clase de joya (las que falten usan `default_threshold`)
    #[serde(default)]
    pub thresholds: std::collections::HashMap<String, f32>,
    #[serde(default = "default_threshold")]
    pub default_threshold: f32,
    /// Umbral de las detecciones débiles (`lo`): las joyas con `lo ≤ conf < umbral de trabajo` se
    /// devuelven en la banda débil del núcleo para que CLIP decida (ver la cabecera del módulo). Elegido
    /// en la validación revisada a mano con CLIP en el bucle (F1 de "joya"). Sin él (JSON antiguos) no hay
    /// banda débil y todo lo que no llega al umbral se descarta, como antes.
    #[serde(default)]
    pub weak_threshold: Option<f32>,
}

fn default_imgsz() -> usize {
    960
}
fn default_threshold() -> f32 {
    0.25
}

/// El detector propio abierto.
pub struct TrainedDetector {
    yolo: Yolo,
    names: Vec<String>,
    /// umbral por índice de clase (`None` = persona/cara, sin reescalar)
    thresholds: Vec<Option<f32>>,
    /// umbral de las débiles que van a CLIP (`weak_threshold` del JSON)
    weak: Option<f32>,
}

/// ¿Es una clase de persona (sin umbral propio ni reescalado)?
fn is_people(label: &str) -> bool {
    label == "persona" || label == "cara"
}

/// Confianza que ve el núcleo para una joya de confianza `c` con umbral de trabajo `u` y umbral débil
/// `weak`: ≥ `u` → banda fuerte (`u` pasa a 0.15); entre `weak` y `u` → banda débil (CLIP decide);
/// por debajo → `None` (no se devuelve).
pub fn core_conf(c: f32, u: f32, weak: Option<f32>) -> Option<f32> {
    if c >= u {
        return Some((CORE_MIN_CONF + (c - u) * (1.0 - CORE_MIN_CONF) / (1.0 - u).max(1e-6)).min(1.0));
    }
    let lo = weak.filter(|lo| *lo < u)?;
    if c < lo {
        return None;
    }
    let (a, b) = WEAK_BAND;
    Some(a + (c - lo) * (b - a) / (u - lo))
}

/// ¿Está instalado el detector propio (clases y al menos una versión del modelo)?
pub fn disponible(models_dir: &Path) -> bool {
    models_dir.join(format!("{STEM}.classes.json")).exists()
        && (models_dir.join(format!("{STEM}_fp16.onnx")).exists() || models_dir.join(format!("{STEM}.onnx")).exists())
}

impl TrainedDetector {
    /// Abre `models/jewel_det[_fp16].onnx` y su `jewel_det.classes.json`.
    pub fn open(env: &ort::environment::Environment, models_dir: &Path, device: Device, fp16: bool) -> Result<Self> {
        let cpath = models_dir.join(format!("{STEM}.classes.json"));
        let text = std::fs::read_to_string(&cpath).with_context(|| format!("leyendo {}", cpath.display()))?;
        let cls: ClassesFile = serde_json::from_str(&text).with_context(|| format!("interpretando {}", cpath.display()))?;
        let model = models_dir.join(if fp16 { format!("{STEM}_fp16.onnx") } else { format!("{STEM}.onnx") });
        let yolo = Yolo::open(env, &model, device, cls.imgsz, cls.names.len())?;
        let thresholds = cls
            .names
            .iter()
            .map(|n| if is_people(n) { None } else { Some(*cls.thresholds.get(n).unwrap_or(&cls.default_threshold)) })
            .collect();
        Ok(Self { yolo, names: cls.names, thresholds, weak: cls.weak_threshold })
    }

    /// Cuántas sesiones de forma fija se guardan (2 en un detector compartido por varios clips).
    pub fn set_max_fijas(&mut self, n: usize) {
        self.yolo.set_max_fijas(n);
    }

    /// Supresión entre clases de joya: el NMS de Ultralytics es por clase, así que la misma pieza puede
    /// salir dos veces con etiquetas distintas (un aro como `arete` y como `anillo`). Se queda la más
    /// segura cuando dos cajas de joya de distinta clase se solapan con IoU > [`CROSS_IOU`]. Una caja
    /// pequeña dentro de otra grande (el dije sobre su cadena) tiene IoU bajo y se conserva.
    /// Personas y caras no entran. Las detecciones llegan ordenadas por confianza.
    fn cross_class(&self, mut dets: Vec<crate::yolo::RawDet>) -> Vec<crate::yolo::RawDet> {
        dets.sort_by(|a, b| b.conf.total_cmp(&a.conf));
        let jewel = |d: &crate::yolo::RawDet| self.thresholds.get(d.class).copied().flatten().is_some();
        let mut keep: Vec<crate::yolo::RawDet> = Vec::with_capacity(dets.len());
        for d in dets {
            let dup = jewel(&d) && keep.iter().any(|k| jewel(k) && k.class != d.class && crate::yolo::iou(&k.xyxy, &d.xyxy) > CROSS_IOU);
            if !dup {
                keep.push(d);
            }
        }
        keep
    }

    /// Umbral más bajo que hay que pedirle al NMS (el menor de todas las clases, de las débiles y de
    /// personas).
    fn nms_conf(&self) -> f32 {
        let base = self.thresholds.iter().map(|t| t.unwrap_or(PEOPLE_CONF)).fold(1.0, f32::min);
        self.weak.map_or(base, |w| base.min(w))
    }

    /// Detecta en un lote de cuadros BGR del mismo tamaño. Cajas truncadas a enteros como en Python.
    pub fn detect(&mut self, frames: &[(&[u8], usize, usize)]) -> Result<Vec<Vec<Detection>>> {
        let conf = self.nms_conf();
        let raw = self.yolo.predict(frames, conf)?;
        Ok(raw
            .into_iter()
            .map(|dets| {
                let dets = self.cross_class(dets);
                dets.into_iter()
                    .filter_map(|d| {
                        let label = self.names.get(d.class)?.clone();
                        let c = match self.thresholds.get(d.class).copied().flatten() {
                            // joya: fuerte (umbral -> 0.15), débil para CLIP (0.021–0.149) o descartada
                            Some(u) => core_conf(d.conf, u, self.weak)?,
                            None => {
                                if d.conf < PEOPLE_CONF {
                                    return None;
                                }
                                d.conf
                            }
                        };
                        Some(Detection {
                            x1: d.xyxy[0].trunc(),
                            y1: d.xyxy[1].trunc(),
                            x2: d.xyxy[2].trunc(),
                            y2: d.xyxy[3].trunc(),
                            conf: c.min(1.0),
                            label,
                        })
                    })
                    .collect()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bandas de confianza: fuerte desde el umbral, débil (dentro de la banda que verifica el núcleo,
    /// comparada en f64 como hace él) entre `lo` y el umbral, nada por debajo; sin `lo`, como antes.
    #[test]
    fn bandas_de_confianza() {
        let (u, lo) = (0.4, Some(0.25));
        assert!((core_conf(0.4, u, lo).unwrap() - 0.15).abs() < 1e-6);
        assert!((core_conf(1.0, u, lo).unwrap() - 1.0).abs() < 1e-6);
        for c in [0.25f32, 0.3, 0.35, 0.399_99] {
            let w = core_conf(c, u, lo).unwrap() as f64;
            assert!((0.02..0.15).contains(&w), "{c} -> {w}");
        }
        assert!(core_conf(0.30, u, lo).unwrap() < core_conf(0.35, u, lo).unwrap());
        assert_eq!(core_conf(0.249, u, lo), None);
        assert_eq!(core_conf(0.3, u, None), None);
        assert!(core_conf(0.4, u, None).unwrap() as f64 >= 0.15);
    }

    /// `--detector` se quita de los argumentos en sus dos formas.
    #[test]
    fn take_arg_quita_la_opcion() {
        let mut a: Vec<String> = ["clip.mp4", "--detector", "propio", "--out", "x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(DetectorKind::take_arg(&mut a), Some(DetectorKind::Propio));
        assert_eq!(a, vec!["clip.mp4", "--out", "x"]);
        let mut b: Vec<String> = ["--detector=world", "--out", "x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(DetectorKind::take_arg(&mut b), Some(DetectorKind::World));
        assert_eq!(b, vec!["--out", "x"]);
    }
}
