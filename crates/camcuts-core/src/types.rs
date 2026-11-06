//! Tipos compartidos entre crates (contrato de INTERFACES.md).
//!
//! `camcuts-infer` implementa [`Detector`] con los modelos (YOLO-World, MobileSAM, pose, CLIP, Silero VAD)
//! y `camcuts-core` lo usa en el bucle de análisis sin saber nada de ONNX ni de la GPU.
//! Los nombres y campos son exactamente los de INTERFACES.md: cualquier cambio se anuncia allí.

/// Un cuadro tal como lo entrega el muestreador: BGR, 8 bits, por filas, `width*height*3` bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Una detección en píxeles del cuadro.
///
/// `label` es el tipo en español que usa Python (`anillo`, `collar`, `cadena`, `arete`, `topo`,
/// `pulsera`, `dije`, `joya`, `persona`, `cara`). Si el detector devuelve el texto en inglés del
/// prompt (`ring`, `person`…), el núcleo lo traduce con [`crate::jewel::spanish_label`].
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub conf: f32,
    pub label: String,
}

/// Puntos de pose de una persona (17 puntos COCO: x, y, confianza).
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub keypoints: [[f32; 3]; 17],
    pub bbox: [f32; 4],
    pub conf: f32,
}

/// Todo lo que los modelos le dan al núcleo para un cuadro.
pub trait Detector: Send {
    /// Detecciones de joyas / personas / caras (YOLO-World con los mismos prompts que Python).
    fn detect(&mut self, frame: &Frame) -> anyhow::Result<Vec<Detection>>;
    /// Silueta de una caja (MobileSAM), del mismo tamaño que el cuadro, 0/255.
    fn silhouette(&mut self, frame: &Frame, bbox: [f32; 4]) -> anyhow::Result<Vec<u8>>;
    /// Puntos de pose (yolo11n-pose).
    fn pose(&mut self, frame: &Frame) -> anyhow::Result<Vec<Pose>>;
    /// Verificador CLIP: probabilidad de que el recorte sea una joya
    /// (detecciones débiles 0.02–0.15, aceptadas con p ≥ 0.6).
    fn verify(&mut self, frame: &Frame, bbox: [f32; 4]) -> anyhow::Result<f32>;
    /// Probabilidad de voz por ventana de 32 ms de audio mono a 16 kHz (Silero VAD).
    fn speech(&mut self, audio_16k: &[f32]) -> anyhow::Result<Vec<f32>>;
    /// Solo edición Bodas: probabilidad de cada momento de la boda para el cuadro entero (CLIP zero-shot,
    /// `models/clip_moments.json`), como (momento, probabilidad). Por defecto no hay momentos.
    fn moments(&mut self, _frame: &Frame) -> anyhow::Result<Vec<(String, f32)>> {
        Ok(Vec::new())
    }
    /// Qué detector de joyas hay detrás de [`Detector::detect`]: se guarda en la caché de cada clip
    /// (clave `detector`) para saber si hay que reanalizarlo al cambiar de detector. Por defecto
    /// YOLO-World (el de siempre); `camcuts-infer` dice el propio cuando lo usa.
    fn kind(&self) -> crate::detector::DetectorKind {
        crate::detector::DetectorKind::World
    }
    /// Si [`Detector::kind`] no es el detector que se pidió (en modo ligero, el propio en lugar de
    /// YOLO-World), el que se pidió: se guarda en la caché (`detector::CLAVE_EN_LUGAR`).
    fn en_lugar_de(&self) -> Option<crate::detector::DetectorKind> {
        None
    }
    /// Abre y deja listos los modelos para cuadros de `width`×`height` antes del primer clip (el núcleo
    /// lo llama mientras arranca el ffmpeg del primero). No cambia ningún resultado: solo adelanta la
    /// carga que si no se haría en la primera muestra. Por defecto no hace nada.
    fn prepare(&mut self, _width: u32, _height: u32) -> anyhow::Result<()> {
        Ok(())
    }
}
