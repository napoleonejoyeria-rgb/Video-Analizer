//! Análisis por línea de comandos con los modelos de verdad, sin depender de la ventana (`camcuts-gui`).
//!
//! Hace lo mismo que `camcuts.exe --analizar ...`: las opciones de `camcuts_core::cli::Args` y un
//! `OnnxDetector` por hilo. Sirve para las pruebas largas de robustez y velocidad del núcleo aunque la
//! ventana esté a medio editar.
//!
//! `cargo run -p camcuts-infer --release --example analizar -- <clips o carpetas> --out <dir> [...]`

fn main() {
    let args = camcuts_core::cli::Args::from_env();
    // `--detector propio|world` (o `CAMCUTS_DETECTOR`): el mismo que usa el núcleo para la caché
    let cfg = camcuts_infer::InferConfig { detector: args.detector_kind(), ..Default::default() };
    // `--motor cuda|directml` (o `CAMCUTS_MOTOR`); en automático, CUDA si está `runtime_cuda`
    if let Some(codigo) = camcuts_infer::motor_listo(args.motor.as_deref(), &cfg) {
        std::process::exit(codigo);
    }
    // con el detector propio, el respaldo automático usa YOLO-World de esta otra fábrica (como la app)
    let cfg_respaldo = camcuts_infer::InferConfig { detector: camcuts_infer::DetectorKind::World, propio_si_no_cabe: false, ..cfg.clone() };
    let respaldo = move || -> anyhow::Result<Box<dyn camcuts_core::Detector>> { Ok(Box::new(camcuts_infer::OnnxDetector::new(cfg_respaldo.clone())?)) };
    // cada hilo de análisis pide su detector a esta fábrica (como la app)
    let fabrica = camcuts_infer::fabrica(cfg);
    if let Err(e) = camcuts_core::cli::run_con_respaldo(args, Some(&fabrica), Some(&respaldo)) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
