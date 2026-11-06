//! YOLO-World (detector de joyas, personas y caras) y yolo11n-pose, con el pre- y posprocesado de
//! Ultralytics 8.4 tal como lo usa `jewel.py`:
//!
//! - letterbox de rectángulo mínimo (`auto=True`, stride 32) a `imgsz`, relleno 114, BGR→RGB, /255;
//! - salida (lote, 4+clases[+extra], anclas) con cajas xywh en píxeles de la entrada;
//! - NMS por clase de `ultralytics.utils.nms.non_max_suppression` (conf estricta `>`, IoU 0.7,
//!   desplazamiento 7680 por clase, máximo 30000 candidatos y 300 detecciones);
//! - `scale_boxes`/`scale_coords` para volver a píxeles del cuadro, recortando al borde.

use std::path::Path;

use anyhow::{bail, Result};
use crate::imgops::{bgr_to_chw, letterbox, letterbox_params, py_round, LetterboxInfo};
use crate::runtime::{run_f32, tensor_f32, Device, ShapedSession};

/// Umbral de IoU del NMS (valor por defecto de `predict` en Ultralytics).
pub const NMS_IOU: f32 = 0.7;
/// Desplazamiento por clase para que el NMS no mezcle clases (`max_wh`).
const MAX_WH: f32 = 7680.0;
/// Candidatos máximos que entran al NMS (`max_nms`).
const MAX_NMS: usize = 30000;
/// Detecciones máximas por imagen (`max_det`).
const MAX_DET: usize = 300;

/// Una detección cruda tras el NMS, en píxeles del cuadro original.
#[derive(Debug, Clone)]
pub struct RawDet {
    pub xyxy: [f32; 4],
    pub conf: f32,
    pub class: usize,
    /// canales extra (en pose: 17×(x, y, conf) ya en píxeles del cuadro)
    pub extra: Vec<f32>,
}

/// Un modelo YOLO de Ultralytics exportado a ONNX con entrada dinámica (`batch`, `height`, `width`).
/// Con DirectML se ejecuta con una sesión de forma fija por tamaño de entrada ([`ShapedSession`]).
pub struct Yolo {
    session: ShapedSession,
    imgsz: usize,
    nc: usize,
}

impl Yolo {
    /// `imgsz`: lado del cuadrado de `predict` (960 en YOLO-World, 640 en pose); `nc`: número de clases.
    pub fn open(env: &ort::environment::Environment, path: &Path, device: Device, imgsz: usize, nc: usize) -> Result<Self> {
        Ok(Self { session: ShapedSession::open(env, path, device)?, imgsz, nc })
    }

    /// Cuántas sesiones de forma fija se guardan (ver [`ShapedSession::set_max_fijas`]).
    pub fn set_max_fijas(&mut self, n: usize) {
        self.session.set_max_fijas(n);
    }

    /// Infiere un lote de cuadros BGR del MISMO tamaño (como `predict` con una lista: un solo letterbox).
    pub fn predict(&mut self, frames: &[(&[u8], usize, usize)], conf: f32) -> Result<Vec<Vec<RawDet>>> {
        if frames.is_empty() {
            return Ok(vec![]);
        }
        let (h, w) = (frames[0].1, frames[0].2);
        if frames.iter().any(|f| f.1 != h || f.2 != w) {
            bail!("un lote de YOLO debe tener cuadros del mismo tamaño");
        }
        let lb = letterbox_params(h, w, (self.imgsz, self.imgsz), true, 32, true);
        let (ih, iw) = lb.input;
        let mut data = Vec::with_capacity(frames.len() * 3 * ih * iw);
        for (bgr, _, _) in frames {
            let img = letterbox(bgr, &lb, 114);
            bgr_to_chw(&img, ih, iw, 255.0, [0.0; 3], [1.0; 3], &mut data);
        }
        let input = tensor_f32(vec![frames.len() as i64, 3, ih as i64, iw as i64], data)?;
        let dims = [("batch", frames.len() as i64), ("height", ih as i64), ("width", iw as i64)];
        // forma fija solo para los cuadros de análisis (un cuadro de 960 px de lado largo); los recortes
        // de `--zoom` y los lotes van a la sesión dinámica
        let fixed = frames.len() == 1 && h.max(w) == camcuts_core::sampler::LONG_EDGE as usize;
        let out = run_f32(self.session.get(&dims, fixed)?, vec![("images", input)])?;
        let (shape, pred) = &out[0];
        let (c, a) = (shape[1] as usize, shape[2] as usize);
        let per = c * a;
        Ok((0..frames.len()).map(|b| nms(&pred[b * per..(b + 1) * per], c, a, self.nc, conf, &lb)).collect())
    }
}

/// IoU de dos cajas xyxy (como torchvision: sin +1).
pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let ua = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if ua > 0.0 { inter / ua } else { 0.0 }
}

/// NMS de Ultralytics para una imagen: `pred` es (canales, anclas) por filas.
fn nms(pred: &[f32], c: usize, a: usize, nc: usize, conf_thres: f32, lb: &LetterboxInfo) -> Vec<RawDet> {
    let extra = c - 4 - nc;
    // candidatos: mejor clase con puntuación > umbral (solo la mejor clase: multi_label=False)
    let mut cand: Vec<(f32, usize, usize)> = Vec::new(); // (conf, clase, ancla)
    for i in 0..a {
        let mut best = (f32::MIN, 0usize);
        for k in 0..nc {
            let s = pred[(4 + k) * a + i];
            if s > best.0 {
                best = (s, k);
            }
        }
        if best.0 > conf_thres {
            cand.push((best.0, best.1, i));
        }
    }
    // orden por confianza descendente (torchvision.ops.nms ordena igual); estable ante empates
    cand.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    cand.truncate(MAX_NMS);
    let boxes: Vec<[f32; 4]> = cand
        .iter()
        .map(|&(_, _, i)| {
            let (cx, cy, bw, bh) = (pred[i], pred[a + i], pred[2 * a + i], pred[3 * a + i]);
            [cx - bw / 2.0, cy - bh / 2.0, cx + bw / 2.0, cy + bh / 2.0]
        })
        .collect();
    // NMS voraz con las cajas desplazadas por clase
    let off: Vec<[f32; 4]> = boxes
        .iter()
        .zip(&cand)
        .map(|(b, &(_, k, _))| {
            let o = k as f32 * MAX_WH;
            [b[0] + o, b[1] + o, b[2] + o, b[3] + o]
        })
        .collect();
    let mut suppressed = vec![false; cand.len()];
    let mut keep = Vec::new();
    for i in 0..cand.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(i);
        if keep.len() >= MAX_DET {
            break;
        }
        for j in i + 1..cand.len() {
            if !suppressed[j] && iou(&off[i], &off[j]) > NMS_IOU {
                suppressed[j] = true;
            }
        }
    }
    // de la entrada del modelo a píxeles del cuadro (scale_boxes / scale_coords con padding)
    let (h0, w0) = lb.orig;
    let (h1, w1) = lb.input;
    let gain = (h1 as f64 / h0 as f64).min(w1 as f64 / w0 as f64);
    let nh = py_round(h0 as f64 * gain).max(1) as f64;
    let nw = py_round(w0 as f64 * gain).max(1) as f64;
    let (gy, gx) = ((nh / h0 as f64) as f32, (nw / w0 as f64) as f32);
    let px = py_round((w1 as f64 - nw) / 2.0 - 0.1) as f32;
    let py = py_round((h1 as f64 - nh) / 2.0 - 0.1) as f32;
    let (fw, fh) = (w0 as f32, h0 as f32);
    keep.into_iter()
        .map(|i| {
            let b = boxes[i];
            let xyxy = [
                ((b[0] - px) / gx).clamp(0.0, fw),
                ((b[1] - py) / gy).clamp(0.0, fh),
                ((b[2] - px) / gx).clamp(0.0, fw),
                ((b[3] - py) / gy).clamp(0.0, fh),
            ];
            let anchor = cand[i].2;
            let mut ex: Vec<f32> = (0..extra).map(|e| pred[(4 + nc + e) * a + anchor]).collect();
            // puntos de pose: (x, y, conf) por punto; x e y a píxeles del cuadro y recortados
            for p in ex.chunks_mut(3) {
                if p.len() == 3 {
                    p[0] = ((p[0] - px) / gx).clamp(0.0, fw);
                    p[1] = ((p[1] - py) / gy).clamp(0.0, fh);
                }
            }
            RawDet { xyxy, conf: cand[i].0, class: cand[i].1, extra: ex }
        })
        .collect()
}
