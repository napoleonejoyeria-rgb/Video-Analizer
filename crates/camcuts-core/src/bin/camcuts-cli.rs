//! `camcuts-cli`: la línea de comandos de CAMCUTS sin modelos.
//!
//! `--rescore` funciona completo (re-puntúa la caché y exporta OTIO/CSV). Analizar clips nuevos necesita
//! los modelos de `camcuts-infer`; ese crate (o la app) llama a `camcuts_core::cli::run` con su detector.

use clap::Parser;

fn main() {
    let args = camcuts_core::cli::Args::parse();
    if let Err(e) = camcuts_core::cli::run(args, None) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
