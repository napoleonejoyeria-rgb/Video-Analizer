//! # camcuts-core
//!
//! Núcleo de CAMCUTS portado de la app de Python (`camcuts_next/camcuts`). Elige los tramos de video
//! en los que una joya se ve, está nítida y bien expuesta, y los exporta como línea de tiempo de
//! DaVinci Resolve. Este crate no carga modelos: recibe un [`types::Detector`] desde `camcuts-infer`.
//!
//! Módulos (uno por módulo de Python):
//! - [`types`]: contrato con los otros crates (`Frame`, `Detection`, `Pose`, `Detector`).
//! - [`metrics`]: medidas de imagen (bordes nítidos, Laplaciano, exposición, movimiento de cámara).
//! - [`image`]: aritmética de imagen de 8 bits idéntica a OpenCV 5 (gris, YCrCb, Sobel, INTER_AREA).
//! - [`color`]: conversiones LOG/HDR → Rec.709, LUT `.cube`, detección de perfil.
//! - [`sampler`]: decodificación con ffmpeg (GPU con CPU de respaldo), proxy y timecode.
//! - [`jewel`]: medida de cada muestra centrada en la joya, reutilización de siluetas.
//! - [`scoring`]: criterios, puntuación, tramos, etiquetas estables.
//! - [`export`]: OTIO para Resolve y CSV.
//! - [`project`]: proyecto `.camcuts`, caché por clip y archivos JSON de la carpeta de resultados.
//! - [`analyse`]: bucle de análisis con filtro de movimiento.
//! - [`audio`]: voz (intervalos a partir de las probabilidades de Silero VAD).
//! - [`hardware`]: hilos, prioridad baja.
//! - [`pyfmt`]: formatos de Python (`round`, `repr(float)`, `json.dumps`).
//! - [`cli`]: la línea de comandos (reutilizable desde otros binarios con un detector real).
//! - [`edicion`]: edición del proceso (joyas por defecto, o bodas para `camcuts-bodas.exe`).
//! - [`detector`]: detector de joyas del análisis (YOLO-World por defecto o el propio entrenado), su
//!   marca en la caché de cada clip y la regla de caché vieja.
//! - [`bodas`]: la edición Bodas (caras, momentos de la boda, cámara estable); `scoring` y `analyse` le
//!   pasan el trabajo cuando la edición es Bodas, sin cambiar nada de la de joyas.

pub mod analyse;
pub mod audio;
pub mod bodas;
pub mod cli;
pub mod color;
pub mod detector;
pub mod edicion;
pub mod export;
pub mod hardware;
pub mod image;
pub mod jewel;
pub mod metrics;
pub mod project;
pub mod pyfmt;
pub mod sampler;
pub mod scoring;
pub mod types;

pub use types::{Detection, Detector, Frame, Pose};
