//! Verificador CLIP (ViT-B/32): segunda opiniÃ³n sobre detecciones dÃ©biles, como `JewelVerifier` de
//! `classify.py`.
//!
//! Preprocesado de `clip.load` (torchvision sobre PIL): BGRâ†’RGB, `Resize(224, BICUBIC)` del lado corto
//! (el largo = int(224Â·largo/corto)), `CenterCrop(224)` con `round((ladoâˆ’224)/2)`, /255 y normalizaciÃ³n
//! con media (0.48145466, 0.4578275, 0.40821073) y desviaciÃ³n (0.26862954, 0.26130258, 0.27577711).
//!
//! Probabilidad: embedding normalizado Â· textos normalizados Ã— 100 â†’ softmax sobre los 15 textos â†’
//! suma de los 7 primeros (los de joya). Los embeddings de texto vienen de `models/clip_text.json`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::imgops::{bgr_to_chw, crop, py_round, resize_bicubic_pil};
use crate::runtime::{run_f32, tensor_f32, Device, ShapedSession};

/// Lado de la entrada de CLIP ViT-B/32.
const N_PX: usize = 224;
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];
/// Relleno alrededor de la caja antes de recortar (verify_weak en __main__.py).
pub const VERIFY_PAD: f32 = 0.15;

#[derive(Deserialize)]
struct TextFile {
    logit_scale: f32,
    verifier: VerifierText,
}

#[derive(Deserialize)]
struct VerifierText {
    n_jewel: usize,
    embeddings: Vec<Vec<f32>>,
}

/// Codificador de imagen de CLIP mÃ¡s los textos del verificador.
pub struct ClipVerifier {
    /// forma fija para un recorte por llamada (lo que hace `verify_weak`); los lotes, en la dinámica
    session: ShapedSession,
    text: Vec<Vec<f32>>,
    n_jewel: usize,
    logit_scale: f32,
}

impl ClipVerifier {
    pub fn open(env: &ort::environment::Environment, model: &Path, text_json: &Path, device: Device) -> Result<Self> {
        let raw = std::fs::read_to_string(text_json).with_context(|| format!("leyendo {}", text_json.display()))?;
        let t: TextFile = serde_json::from_str(&raw).context("clip_text.json mal formado")?;
        Ok(Self {
            session: ShapedSession::open(env, model, device)?,
            text: t.verifier.embeddings,
            n_jewel: t.verifier.n_jewel,
            logit_scale: t.logit_scale,
        })
    }

    /// Cuántas sesiones de forma fija se guardan (ver `ShapedSession::set_max_fijas`).
    pub fn set_max_fijas(&mut self, n: usize) {
        self.session.set_max_fijas(n);
    }

    /// Prepara un recorte BGR (hÃ—w) como lo hace el `preprocess` de CLIP y lo aÃ±ade a `dst`.
    fn preprocess(bgr: &[u8], h: usize, w: usize, dst: &mut Vec<f32>) {
        // Resize(224): el lado corto pasa a 224 y el largo a int(224Â·largo/corto) (torchvision trunca)
        let (nh, nw) = if w <= h {
            (((N_PX * h) as f64 / w as f64) as usize, N_PX)
        } else {
            (N_PX, ((N_PX * w) as f64 / h as f64) as usize)
        };
        // torchvision no redimensiona si el lado corto ya mide 224
        let resized = if (w <= h && w == N_PX) || (h < w && h == N_PX) {
            bgr.to_vec()
        } else {
            resize_bicubic_pil(bgr, w, h, nw, nh)
        };
        let (rh, rw) = if (w <= h && w == N_PX) || (h < w && h == N_PX) { (h, w) } else { (nh, nw) };
        // CenterCrop(224)
        let top = py_round((rh as f64 - N_PX as f64) / 2.0).max(0) as usize;
        let left = py_round((rw as f64 - N_PX as f64) / 2.0).max(0) as usize;
        let c = crop(&resized, rw, left, top, left + N_PX, top + N_PX);
        bgr_to_chw(&c, N_PX, N_PX, 255.0, MEAN, STD, dst);
    }

    /// Embedding normalizado de cada imagen BGR `(datos, alto, ancho)`, con el preprocesado de CLIP
    /// (lo usa el clasificador de momentos de la ediciÃ³n Bodas).
    pub fn embeddings(&mut self, crops: &[(Vec<u8>, usize, usize)]) -> Result<Vec<Vec<f32>>> {
        if crops.is_empty() {
            return Ok(vec![]);
        }
        let mut data = Vec::with_capacity(crops.len() * 3 * N_PX * N_PX);
        for (c, h, w) in crops {
            Self::preprocess(c, *h, *w, &mut data);
        }
        let out = run_f32(self.session.get(&[("batch", crops.len() as i64)], crops.len() == 1)?, vec![("pixels", tensor_f32(vec![crops.len() as i64, 3, N_PX as i64, N_PX as i64], data)?)])?;
        let emb = &out[0].1;
        let dim = emb.len() / crops.len();
        Ok((0..crops.len())
            .map(|i| {
                let e = &emb[i * dim..(i + 1) * dim];
                let norm = e.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                e.iter().map(|v| v / norm).collect()
            })
            .collect())
    }

    /// Probabilidad de joya de cada recorte BGR `(datos, alto, ancho)`, en un solo lote.
    pub fn probabilities(&mut self, crops: &[(Vec<u8>, usize, usize)]) -> Result<Vec<f32>> {
        if crops.is_empty() {
            return Ok(vec![]);
        }
        let mut data = Vec::with_capacity(crops.len() * 3 * N_PX * N_PX);
        for (c, h, w) in crops {
            Self::preprocess(c, *h, *w, &mut data);
        }
        let out = run_f32(self.session.get(&[("batch", crops.len() as i64)], crops.len() == 1)?, vec![("pixels", tensor_f32(vec![crops.len() as i64, 3, N_PX as i64, N_PX as i64], data)?)])?;
        let emb = &out[0].1;
        let dim = emb.len() / crops.len();
        Ok((0..crops.len())
            .map(|i| {
                let e = &emb[i * dim..(i + 1) * dim];
                let norm = e.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                let logits: Vec<f32> = self
                    .text
                    .iter()
                    .map(|t| self.logit_scale * t.iter().zip(e).map(|(a, b)| a * b / norm).sum::<f32>())
                    .collect();
                let m = logits.iter().cloned().fold(f32::MIN, f32::max);
                let ex: Vec<f32> = logits.iter().map(|l| (l - m).exp()).collect();
                let s: f32 = ex.iter().sum();
                ex[..self.n_jewel].iter().sum::<f32>() / s
            })
            .collect())
    }
}

/// Recorte que mira el verificador para una caja de detecciÃ³n: la caja (en enteros, como en Python)
/// agrandada un 15 % por lado con `int()`, y recortada al cuadro. Devuelve [x0, y0, x1, y1].
pub fn verify_crop_rect(bbox: [f32; 4], w: usize, h: usize) -> [usize; 4] {
    let [x0, y0, x1, y1] = bbox.map(|v| v.trunc() as i64);
    // int((x1 - x0) * 0.15) en doble precisiÃ³n, como Python
    let px = ((x1 - x0) as f64 * VERIFY_PAD as f64) as i64;
    let py = ((y1 - y0) as f64 * VERIFY_PAD as f64) as i64;
    [
        (x0 - px).max(0) as usize,
        (y0 - py).max(0) as usize,
        ((x1 + px).min(w as i64)).max(0) as usize,
        ((y1 + py).min(h as i64)).max(0) as usize,
    ]
}
