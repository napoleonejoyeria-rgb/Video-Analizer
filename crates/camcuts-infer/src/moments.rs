//! Edición Bodas: momento de la boda de un cuadro con CLIP zero-shot (`MomentClassifier` de Python).
//!
//! El cuadro entero se rellena a un cuadrado (negro arriba/abajo o a los lados) para que el recorte
//! central de CLIP no corte nada, se codifica con el mismo `clip_visual.onnx` del verificador, y se
//! compara con un embedding de texto por momento (media renormalizada de 3-6 frases en inglés,
//! precalculada en `models/bodas/clip_moments.json` por `models/export_bodas.py`). Softmax con escala 100.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::clip::ClipVerifier;

#[derive(Deserialize)]
struct MomentsFile {
    logit_scale: f32,
    labels: Vec<String>,
    embeddings: Vec<Vec<f32>>,
}

/// Textos de los momentos (ya normalizados).
pub struct Moments {
    labels: Vec<String>,
    text: Vec<Vec<f32>>,
    logit_scale: f32,
}

impl Moments {
    /// Lee `clip_moments.json`.
    pub fn open(json: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(json).with_context(|| format!("leyendo {}", json.display()))?;
        let f: MomentsFile = serde_json::from_str(&raw).context("clip_moments.json mal formado")?;
        anyhow::ensure!(f.labels.len() == f.embeddings.len() && !f.labels.is_empty(), "clip_moments.json sin momentos");
        Ok(Self { labels: f.labels, text: f.embeddings, logit_scale: f.logit_scale })
    }

    /// Etiquetas de los momentos, en el orden del archivo.
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Probabilidad de cada momento para un cuadro BGR (`h`×`w`), usando la sesión de CLIP de `clip`.
    pub fn classify(&self, clip: &mut ClipVerifier, bgr: &[u8], h: usize, w: usize) -> Result<Vec<(String, f32)>> {
        let (sq, side) = letterbox(bgr, h, w);
        let emb = clip.embeddings(&[(sq, side, side)])?;
        let e = &emb[0];
        let logits: Vec<f32> = self.text.iter().map(|t| self.logit_scale * t.iter().zip(e).map(|(a, b)| a * b).sum::<f32>()).collect();
        let m = logits.iter().cloned().fold(f32::MIN, f32::max);
        let ex: Vec<f32> = logits.iter().map(|l| (l - m).exp()).collect();
        let s: f32 = ex.iter().sum();
        Ok(self.labels.iter().cloned().zip(ex.iter().map(|v| v / s)).collect())
    }
}

/// Rellena con negro un cuadro BGR hasta un cuadrado, centrado (como `classify.letterbox` de Python).
pub fn letterbox(bgr: &[u8], h: usize, w: usize) -> (Vec<u8>, usize) {
    let side = h.max(w);
    let mut out = vec![0u8; side * side * 3];
    let (oy, ox) = ((side - h) / 2, (side - w) / 2);
    for y in 0..h {
        let d = ((oy + y) * side + ox) * 3;
        out[d..d + w * 3].copy_from_slice(&bgr[y * w * 3..(y + 1) * w * 3]);
    }
    (out, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relleno_centrado() {
        // 2 alto × 4 ancho → cuadrado de 4 con una fila negra arriba y otra abajo
        let bgr: Vec<u8> = (0..2 * 4 * 3).map(|v| v as u8 + 1).collect();
        let (sq, side) = letterbox(&bgr, 2, 4);
        assert_eq!(side, 4);
        assert!(sq[..12].iter().all(|&v| v == 0));
        assert_eq!(&sq[12..24], &bgr[..12]);
        assert!(sq[36..].iter().all(|&v| v == 0));
    }
}
