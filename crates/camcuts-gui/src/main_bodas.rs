//! `camcuts-bodas.exe`: la edición Bodas de CAMCUTS (diseño "Salvia", verde clarito).
//!
//! Es el mismo programa que `camcuts.exe` con la edición fijada en Bodas antes de nada: sujetos de boda
//! (caras primero), momentos de la boda, cámara estable, sus criterios y su ventana. El análisis corre
//! en un proceso hijo del propio `camcuts-bodas.exe --analizar ...`, que también arranca en Bodas.
//!
//! Uso:
//!   camcuts-bodas.exe                       abre el último proyecto de bodas o las últimas preferencias
//!   camcuts-bodas.exe boda.camcuts          abre ese proyecto
//!   camcuts-bodas.exe <carpeta CAMCUTS>     abre una carpeta de resultados existente

// sin consola en Windows al abrir la ventana con doble clic
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

use camcuts_gui::{App, Ventana};

fn main() -> eframe::Result {
    // la edición, antes que nada: el análisis, los criterios, los modelos y la ventana la leen
    camcuts_core::edicion::fijar(camcuts_core::edicion::Edicion::Bodas);
    // modo análisis: la ventana se lanza a sí misma como proceso hijo para analizar (sin ventana)
    if std::env::args().nth(1).as_deref() == Some(camcuts_gui::analisis::ARG_ANALIZAR) {
        let codigo = camcuts_gui::analisis::ejecutar_analisis(std::env::args().skip(2).collect());
        std::process::exit(codigo);
    }
    let argumento = std::env::args().nth(1).map(PathBuf::from);
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CAMCUTS Bodas")
            .with_app_id("CAMCUTS Bodas")
            .with_inner_size([1500.0, 920.0])
            .with_min_inner_size([1200.0, 760.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "CAMCUTS Bodas",
        opciones,
        Box::new(move |cc| {
            let mut app = App::nuevo(&cc.egui_ctx);
            match argumento {
                Some(p) if p.extension().map(|e| e.eq_ignore_ascii_case("camcuts")).unwrap_or(false) && p.exists() => app.abrir_proyecto_al_arrancar(&p),
                Some(p) if p.is_dir() => app.abrir_resultados(&p),
                _ => app.cargar_preferencias(),
            }
            Ok(Box::new(Ventana(app)))
        }),
    )
}
