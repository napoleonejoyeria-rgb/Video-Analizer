//! Silero VAD (modelo ONNX que trae el paquete `silero-vad`), con el mismo envoltorio que
//! `silero_vad.utils_vad.OnnxWrapper` y el mismo recorrido que `get_speech_timestamps`:
//!
//! - audio mono a 16 kHz en f32 (int16 / 32768);
//! - ventanas de 512 muestras (32 ms); la última se rellena con ceros;
//! - a cada ventana se le anteponen las 64 últimas muestras de la anterior (contexto; ceros al empezar)
//!   y se pasa el estado recurrente (2×1×128) de la llamada anterior;
//! - salida: probabilidad de voz por ventana. Los intervalos (umbral 0.5, histéresis, duración mínima)
//!   los calcula `camcuts-core` a partir de estas probabilidades.
//!
//! Corre siempre en CPU con un hilo: el modelo es diminuto y DirectML solo añadiría latencia por llamada.

use std::path::Path;

use anyhow::Result;
use ort::session::Session;

use crate::runtime::{run_f32, session, tensor_f32, tensor_i64, Device};

/// Muestras por ventana a 16 kHz.
pub const WINDOW: usize = 512;
/// Muestras de contexto que se anteponen a cada ventana.
const CONTEXT: usize = 64;
/// Frecuencia de muestreo admitida.
pub const SR: i64 = 16000;

pub struct SileroVad {
    session: Session,
}

impl SileroVad {
    pub fn open(env: &ort::environment::Environment, model: &Path) -> Result<Self> {
        Ok(Self { session: session(env, model, Device::Cpu, Some(1))? })
    }

    /// Probabilidad de voz por ventana de 512 muestras de `audio` (16 kHz mono), empezando con el estado
    /// a cero (como `reset_states()` al principio de `get_speech_timestamps`).
    pub fn probabilities(&mut self, audio: &[f32]) -> Result<Vec<f32>> {
        let mut state = vec![0f32; 2 * 128];
        let mut context = vec![0f32; CONTEXT];
        let mut out = Vec::with_capacity(audio.len().div_ceil(WINDOW));
        let mut i = 0;
        while i < audio.len() {
            let end = (i + WINDOW).min(audio.len());
            let mut x = Vec::with_capacity(CONTEXT + WINDOW);
            x.extend_from_slice(&context);
            x.extend_from_slice(&audio[i..end]);
            x.resize(CONTEXT + WINDOW, 0.0); // relleno de la última ventana
            let res = run_f32(
                &mut self.session,
                vec![
                    ("input", tensor_f32(vec![1, (CONTEXT + WINDOW) as i64], x.clone())?),
                    ("state", tensor_f32(vec![2, 1, 128], state)?),
                    ("sr", tensor_i64(vec![], vec![SR])?),
                ],
            )?;
            out.push(res[0].1[0]);
            state = res[1].1.clone();
            context.copy_from_slice(&x[x.len() - CONTEXT..]);
            i += WINDOW;
        }
        Ok(out)
    }
}
