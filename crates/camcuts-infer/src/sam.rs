//! MobileSAM: silueta de una joya a partir de su caja, como `JewelSegmenter` (Ultralytics `SAM.predict`
//! con `bboxes=`).
//!
//! Preprocesado (`models/sam/predict.py`): letterbox a 1024 sin centrar (relleno 114 abajo/derecha),
//! BGR→RGB, `(x − [123.675, 116.28, 103.53]) / [58.395, 57.12, 57.375]`. Las cajas se multiplican por
//! la misma escala `r = min(1024/alto, 1024/ancho)`.
//!
//! Posprocesado (`postprocess` + `ops.scale_masks`): se descartan las máscaras con puntuación ≤ 0.25
//! (`conf` por defecto de predict), la máscara de 256×256 se lleva por bilineal (sin `align_corners`) desde
//! la zona útil (sin relleno) al tamaño del cuadro y se umbraliza en > 0.
//!
//! El codificador (lo caro) se ejecuta una vez por cuadro: el embedding se guarda y se reutiliza mientras
//! llegan cajas del mismo cuadro.

use std::path::Path;

use anyhow::Result;
use ort::session::Session;

use crate::imgops::{bgr_to_chw, bilinear_threshold, letterbox, letterbox_params, py_round};
use crate::runtime::{run_f32, session, tensor_f32, Device, ShapedSession};

/// Lado de la entrada del codificador.
const SIDE: usize = 1024;
/// Lado de la máscara de baja resolución que da el decodificador.
const LOWRES: usize = 256;
/// Puntuación mínima (estricta) para quedarse con una máscara (`conf` por defecto de predict).
pub const SAM_CONF: f32 = 0.25;

/// Embeddings guardados (los de los últimos cuadros). Con un detector compartido por varios clips en
/// paralelo (`Compartido`) las cajas de cuadros de clips distintos llegan intercaladas: con un solo hueco,
/// cada clip borraba el embedding del otro y el codificador (lo caro, ~30 ms; ~120 ms con DaVinci
/// ocupando la GPU) se repetía por cada caja. Medido el 2026-11-05: C9280 pasó de 37 s a 114 s.
const CACHE_CUADROS: usize = 4;

/// Las dos mitades de MobileSAM y el embedding de los últimos cuadros.
pub struct MobileSam {
    encoder: Session,
    /// forma fija para una caja por llamada (lo que pide `measure`); varias cajas, en la dinámica
    decoder: ShapedSession,
    /// (huella del cuadro, alto, ancho, embedding 1×256×64×64), el más reciente al final
    cache: Vec<(u64, usize, usize, Vec<f32>)>,
}

/// Huella rápida de un cuadro (FNV-1a de 64 bits) para saber si el embedding guardado vale.
fn fingerprint(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    // de 8 en 8 bytes: rápido y suficiente para distinguir cuadros reales
    for c in data.chunks(8) {
        let mut v = 0u64;
        for (i, &b) in c.iter().enumerate() {
            v |= (b as u64) << (8 * i);
        }
        h ^= v;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^ data.len() as u64
}

impl MobileSam {
    pub fn open(env: &ort::environment::Environment, encoder: &Path, decoder: &Path, device: Device) -> Result<Self> {
        Ok(Self {
            encoder: session(env, encoder, device, None)?,
            // el decodificador es pequeño; en la misma GPU para no copiar el embedding
            decoder: ShapedSession::open(env, decoder, device)?,
            cache: Vec::with_capacity(CACHE_CUADROS),
        })
    }

    /// Cuántas sesiones de forma fija guarda el decodificador (ver `ShapedSession::set_max_fijas`).
    pub fn set_max_fijas(&mut self, n: usize) {
        self.decoder.set_max_fijas(n);
    }

    /// Embedding del cuadro (lo calcula solo si no es de uno de los últimos [`CACHE_CUADROS`] cuadros).
    fn embedding(&mut self, bgr: &[u8], h: usize, w: usize) -> Result<Vec<f32>> {
        let fp = fingerprint(bgr);
        if let Some(i) = self.cache.iter().position(|(f, ch, cw, _)| *f == fp && *ch == h && *cw == w) {
            let e = self.cache.remove(i);
            let emb = e.3.clone();
            self.cache.push(e);
            return Ok(emb);
        }
        let lb = letterbox_params(h, w, (SIDE, SIDE), false, 32, false);
        let img = letterbox(bgr, &lb, 114);
        let mut data = Vec::with_capacity(3 * SIDE * SIDE);
        bgr_to_chw(&img, SIDE, SIDE, 1.0, [123.675, 116.28, 103.53], [58.395, 57.12, 57.375], &mut data);
        let out = run_f32(&mut self.encoder, vec![("image", tensor_f32(vec![1, 3, SIDE as i64, SIDE as i64], data)?)])?;
        let emb = out.into_iter().next().map(|o| o.1).unwrap_or_default();
        if self.cache.len() >= CACHE_CUADROS {
            self.cache.remove(0);
        }
        self.cache.push((fp, h, w, emb.clone()));
        Ok(emb)
    }

    /// Máscaras (0/255, tamaño del cuadro) para varias cajas en una sola llamada, con la MISMA semántica que
    /// `JewelSegmenter.masks` de Python: Ultralytics quita las máscaras con puntuación ≤ 0.25 y Python las
    /// reparte por posición, así que si se cae una, las siguientes se corren un puesto y las últimas quedan en
    /// `None`. Con una sola caja no hay corrimiento posible.
    pub fn masks(&mut self, bgr: &[u8], h: usize, w: usize, boxes: &[[f32; 4]]) -> Result<Vec<Option<Vec<u8>>>> {
        if boxes.is_empty() {
            return Ok(vec![]);
        }
        let emb = self.embedding(bgr, h, w)?;
        let r = (SIDE as f32 / h as f32).min(SIDE as f32 / w as f32);
        let scaled: Vec<f32> = boxes.iter().flat_map(|b| b.iter().map(move |v| v * r)).collect();
        let out = run_f32(
            self.decoder.get(&[("n", boxes.len() as i64)], boxes.len() == 1)?,
            vec![
                ("image_embeddings", tensor_f32(vec![1, 256, 64, 64], emb)?),
                ("boxes", tensor_f32(vec![boxes.len() as i64, 4], scaled)?),
            ],
        )?;
        let (masks, scores) = (&out[0].1, &out[1].1);
        // zona útil de la máscara de baja resolución: round(o*gain)*256/1024 por eje (ratio de postprocess)
        let gain = (SIDE as f64 / h as f64).min(SIDE as f64 / w as f64);
        let span_h = py_round(h as f64 * gain) as f64 * LOWRES as f64 / SIDE as f64;
        let span_w = py_round(w as f64 * gain) as f64 * LOWRES as f64 / SIDE as f64;
        let kept: Vec<Vec<u8>> = (0..boxes.len())
            .filter(|&i| scores[i] > SAM_CONF)
            .map(|i| {
                let m = &masks[i * LOWRES * LOWRES..(i + 1) * LOWRES * LOWRES];
                bilinear_threshold(m, LOWRES, LOWRES, span_h, span_w, h, w, 0.0)
            })
            .collect();
        let mut res: Vec<Option<Vec<u8>>> = kept.into_iter().map(Some).collect();
        res.resize(boxes.len(), None);
        Ok(res)
    }
}
