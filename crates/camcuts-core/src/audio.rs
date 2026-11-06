//! Voz en el audio de un clip (port de `audio.py`): distingue planos hablados de música o ambiente.
//!
//! El modelo (Silero VAD) corre en `camcuts-infer` y devuelve una probabilidad por ventana de 32 ms;
//! aquí se convierten en intervalos con el mismo algoritmo que `get_speech_timestamps` de `silero_vad`
//! (umbral 0.5, silencio mínimo 100 ms, relleno 30 ms, voz mínima 400 ms).

use std::path::Path;

use crate::pyfmt::round_ndigits;
use crate::types::Detector;

/// Frecuencia de muestreo del VAD.
pub const SR: usize = 16000;
/// Muestras por ventana a 16 kHz.
pub const WINDOW: usize = 512;

/// Parámetros de `get_speech_timestamps` (valores por defecto de silero_vad).
#[derive(Debug, Clone, Copy)]
pub struct VadParams {
    pub threshold: f64,
    pub min_speech_duration_ms: f64,
    pub max_speech_duration_s: f64,
    pub min_silence_duration_ms: f64,
    pub speech_pad_ms: f64,
    pub min_silence_at_max_speech: f64,
    pub time_resolution: usize,
}

impl Default for VadParams {
    fn default() -> Self {
        VadParams {
            threshold: 0.5,
            min_speech_duration_ms: 250.0,
            max_speech_duration_s: f64::INFINITY,
            min_silence_duration_ms: 100.0,
            speech_pad_ms: 30.0,
            min_silence_at_max_speech: 98.0,
            time_resolution: 1,
        }
    }
}

/// `get_speech_timestamps_from_probs` de silero_vad (con `use_max_poss_sil_at_max_speech=True`) en segundos.
pub fn speech_timestamps(probs: &[f32], audio_len: usize, p: &VadParams) -> Vec<[f64; 2]> {
    let sr = SR as f64;
    let ws = WINDOW as f64;
    let min_speech = sr * p.min_speech_duration_ms / 1000.0;
    let pad = sr * p.speech_pad_ms / 1000.0;
    let max_speech = sr * p.max_speech_duration_s - ws - 2.0 * pad;
    let min_silence = sr * p.min_silence_duration_ms / 1000.0;
    let min_silence_max = sr * p.min_silence_at_max_speech / 1000.0;
    let neg = (p.threshold - 0.15).max(0.01);
    let mut triggered = false;
    // (inicio, fin) en muestras; fin < 0 = abierto
    let mut speeches: Vec<(f64, f64)> = Vec::new();
    let mut cur: Option<(f64, Option<f64>)> = None;
    let (mut temp_end, mut prev_end, mut next_start) = (0.0f64, 0.0f64, 0.0f64);
    let mut possible_ends: Vec<(f64, f64)> = Vec::new();
    for (i, &pr) in probs.iter().enumerate() {
        let prob = pr as f64;
        let cs = ws * i as f64;
        if prob >= p.threshold && temp_end != 0.0 {
            let sil = cs - temp_end;
            if sil > min_silence_max {
                possible_ends.push((temp_end, sil));
            }
            temp_end = 0.0;
            if next_start < prev_end {
                next_start = cs;
            }
        }
        if prob >= p.threshold && !triggered {
            triggered = true;
            cur = Some((cs, None));
            continue;
        }
        if triggered && cur.map(|c| cs - c.0 > max_speech).unwrap_or(false) {
            if !possible_ends.is_empty() {
                // el silencio más largo dentro del tramo actual
                let mut best = possible_ends[0];
                for e in &possible_ends[1..] {
                    if e.1 > best.1 {
                        best = *e;
                    }
                }
                prev_end = best.0;
                let start = cur.unwrap().0;
                speeches.push((start, prev_end));
                cur = None;
                next_start = prev_end + best.1;
                if next_start < prev_end + cs {
                    cur = Some((next_start, None));
                } else {
                    triggered = false;
                }
                prev_end = 0.0;
                next_start = 0.0;
                temp_end = 0.0;
                possible_ends.clear();
            } else {
                speeches.push((cur.unwrap().0, cs));
                cur = None;
                prev_end = 0.0;
                next_start = 0.0;
                temp_end = 0.0;
                triggered = false;
                possible_ends.clear();
                continue;
            }
        }
        if prob < neg && triggered {
            if temp_end == 0.0 {
                temp_end = cs;
            }
            let sil_now = cs - temp_end;
            if sil_now < min_silence {
                continue;
            }
            if let Some((start, _)) = cur {
                if temp_end - start > min_speech {
                    speeches.push((start, temp_end));
                }
            }
            cur = None;
            prev_end = 0.0;
            next_start = 0.0;
            temp_end = 0.0;
            triggered = false;
            possible_ends.clear();
            continue;
        }
    }
    let total = audio_len as f64;
    if let Some((start, _)) = cur {
        if total - start > min_speech {
            speeches.push((start, total));
        }
    }
    let n = speeches.len();
    for i in 0..n {
        if i == 0 {
            speeches[0].0 = (speeches[0].0 - pad).max(0.0).trunc();
        }
        if i != n - 1 {
            let silence = speeches[i + 1].0 - speeches[i].1;
            if silence < 2.0 * pad {
                speeches[i].1 += (silence / 2.0).floor().trunc();
                speeches[i + 1].0 = (speeches[i + 1].0 - (silence / 2.0).floor()).max(0.0).trunc();
            } else {
                speeches[i].1 = total.min(speeches[i].1 + pad).trunc();
                speeches[i + 1].0 = (speeches[i + 1].0 - pad).max(0.0).trunc();
            }
        } else {
            speeches[i].1 = total.min(speeches[i].1 + pad).trunc();
        }
    }
    let len_s = total / sr;
    speeches
        .into_iter()
        .map(|(a, b)| [round_ndigits(a / sr, p.time_resolution).max(0.0), round_ndigits(b / sr, p.time_resolution).min(len_s)])
        .collect()
}

/// `[[inicio, fin], ...]` en segundos donde alguien habla; vacío si el clip no tiene audio.
pub fn speech_intervals(path: &Path, detector: &mut dyn Detector, min_speech_s: f64) -> Vec<[f64; 2]> {
    speech_intervals_from(&crate::sampler::audio_16k(path), detector, min_speech_s)
}

/// [`speech_intervals`] con el audio ya decodificado (mono, 16 kHz), p. ej. extraído en paralelo con el video.
pub fn speech_intervals_from(audio: &[f32], detector: &mut dyn Detector, min_speech_s: f64) -> Vec<[f64; 2]> {
    if audio.is_empty() {
        return Vec::new();
    }
    let Ok(probs) = detector.speech(audio) else { return Vec::new() };
    let p = VadParams { min_speech_duration_ms: (min_speech_s * 1000.0).trunc(), ..Default::default() };
    speech_timestamps(&probs, audio.len(), &p).into_iter().map(|[a, b]| [round_ndigits(a, 2), round_ndigits(b, 2)]).collect()
}

/// ¿El instante `t` cae dentro de algún intervalo de voz?
pub fn is_speaking(t: f64, intervals: &[[f64; 2]]) -> bool {
    intervals.iter().any(|[a, b]| *a <= t && t <= *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voz_simple() {
        // 1 s de silencio, 1 s de voz, 1 s de silencio (31.25 ventanas por segundo)
        let mut probs = vec![0.0f32; 31];
        probs.extend(vec![0.9f32; 32]);
        probs.extend(vec![0.0f32; 31]);
        let v = speech_timestamps(&probs, probs.len() * 512, &VadParams { min_speech_duration_ms: 400.0, ..Default::default() });
        assert_eq!(v.len(), 1);
        // inicio 31*512 - 480 = 15392 → 0.96 s → 1.0; fin 63*512 + 480 = 32736 → 2.05 s → 2.0
        assert_eq!(v[0], [1.0, 2.0]);
    }
}
