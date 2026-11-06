//! # camcuts-gui
//!
//! La ventana de escritorio de CAMCUTS con el diseño "Marfil" (eframe/egui sobre wgpu).
//! Elige el material, muestra cada clip con sus joyas y su curva de nitidez, deja ajustar el
//! criterio en vivo y exporta a DaVinci Resolve. Abre las mismas carpetas de resultados y los
//! mismos proyectos `.camcuts` que la versión de Python.
//!
//! Módulos:
//! - [`tema`]: colores, fuentes y estilo de Marfil.
//! - [`iconos`]: iconos vectoriales.
//! - [`widgets`]: controles propios (tarjetas, píldoras, deslizadores punteados…).
//! - [`datos`]: buscar clips, ffprobe, tiempos y códigos de tiempo.
//! - [`motor`]: puente con `camcuts-core` (puntuar, tramos, exportar, color).
//! - [`proyecto`]: archivos `.camcuts` y preferencias.
//! - [`video`]: decodificación del visor y de las miniaturas con ffmpeg en hilos aparte.
//! - [`analisis`]: análisis (proceso hijo), proxies, metadatos y perfiles de color en segundo plano.
//! - [`app`]: estado y lógica de la ventana.
//! - [`paneles`]: dibujo de cada panel (diseño Marfil, edición de joyas).
//! - [`bodas_tema`] y [`bodas_ui`]: tema "Salvia" y ventana de la edición Bodas (`camcuts-bodas.exe`).
//!
//! La edición la fija el binario al arrancar (`camcuts_core::edicion`); el estado, el análisis, el
//! proyecto y la exportación son los mismos en las dos.

pub mod analisis;
pub mod app;
pub mod bodas_tema;
pub mod bodas_ui;
pub mod datos;
pub mod iconos;
pub mod motor;
pub mod paneles;
pub mod proyecto;
pub mod tema;
pub mod video;
pub mod widgets;

pub use app::App;

/// Adaptador para eframe: la app completa como `eframe::App`.
pub struct Ventana(pub App);

impl eframe::App for Ventana {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.dibujar(ui);
    }

    fn on_exit(&mut self) {
        self.0.al_cerrar();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if motor::bodas() { bodas_tema::paleta().fondo.to_normalized_gamma_f32() } else { tema::FONDO.to_normalized_gamma_f32() }
    }
}
