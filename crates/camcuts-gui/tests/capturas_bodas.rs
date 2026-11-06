//! Capturas sin pantalla de la edición Bodas (diseño "Salvia"), con egui_kittest + wgpu fuera de pantalla.
//!
//! Nunca abre una ventana: la interfaz se dibuja en una textura y se guarda como PNG en
//! `camcuts_rs/design/snapshots/bodas_*.png`. Necesita una carpeta de resultados analizada con
//! `camcuts-bodas.exe` (con momentos) en `CAMCUTS_BODAS_BENCH`; sin ella la prueba se salta.
//!
//!   set CAMCUTS_BODAS_BENCH=C:\Users\Usuario\Tools\camcuts_rs\bench_bodas
//!   cargo test -p camcuts-gui --test capturas_bodas -- --nocapture

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use camcuts_gui::App;
use egui_kittest::Harness;

fn carpeta_capturas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("design").join("snapshots")
}

/// Arnés de `ancho`×`alto` con la app de bodas abierta sobre la carpeta de resultados.
fn arnes(bench: PathBuf, ancho: f32, alto: f32, preparar: impl Fn(&mut App) + 'static) -> Harness<'static, Option<App>> {
    Harness::builder()
        .with_size(egui::vec2(ancho, alto))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .with_step_dt(1.0 / 30.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(
            move |ui, estado: &mut Option<App>| {
                let app = estado.get_or_insert_with(|| {
                    let mut a = App::nuevo(ui.ctx());
                    a.persistir = false;
                    a.abrir_resultados(&bench);
                    preparar(&mut a);
                    a
                });
                app.dibujar(ui);
            },
            None,
        )
}

/// Avanza cuadros hasta que lleguen el cuadro del visor y la miniatura de CADA clip analizado (o pasen
/// 90 s). `_minimo` se conserva por compatibilidad: ahora se esperan siempre todas.
fn esperar(h: &mut Harness<'static, Option<App>>, _minimo: usize) {
    let inicio = Instant::now();
    loop {
        h.step();
        std::thread::sleep(Duration::from_millis(30));
        let listo = h
            .state()
            .as_ref()
            .map(|a| a.tex_visor.is_some() && a.clips.iter().filter(|c| c.entrada.is_some()).all(|c| a.texturas_mini.contains_key(&c.clave)))
            .unwrap_or(false);
        if listo || inicio.elapsed() > Duration::from_secs(90) {
            break;
        }
    }
    for _ in 0..20 {
        h.step();
    }
}

fn guardar(h: &mut Harness<'static, Option<App>>, nombre: &str) {
    let img = h.render().expect("render fuera de pantalla");
    let dir = carpeta_capturas();
    std::fs::create_dir_all(&dir).unwrap();
    let ruta = dir.join(nombre);
    img.save(&ruta).expect("guardar PNG");
    println!("captura: {}", ruta.display());
}

/// Elige el clip cuyo nombre empieza por `nombre` (o el primero con tramos) y va a la mitad de su
/// primer tramo.
fn ir_a(app: &mut App, nombre: &str) {
    let i = app.clips.iter().position(|c| c.nombre().starts_with(nombre)).or_else(|| app.clips.iter().position(|c| !c.tramos.is_empty()));
    if let Some(i) = i {
        app.clic_fila(i, false, false);
        let t = app.clips[i].tramos.first().map(|s| (s.inicio + s.fin) / 2.0).unwrap_or(1.0);
        app.buscar(t);
    }
}

#[test]
fn capturas_bodas() {
    camcuts_core::edicion::fijar(camcuts_core::edicion::Edicion::Bodas);
    let Some(bench) = std::env::var_os("CAMCUTS_BODAS_BENCH").map(PathBuf::from).filter(|p| p.exists()) else {
        eprintln!("CAMCUTS_BODAS_BENCH no está definida: se salta la prueba de capturas de Bodas");
        return;
    };
    let vertical = std::env::var("CAMCUTS_BODAS_VERTICAL").unwrap_or_else(|_| "IMG_414".into());
    let principal = std::env::var("CAMCUTS_BODAS_CLIP").unwrap_or_else(|_| "C9058".into());

    // 1) pantalla principal a 1440×900, modo claro
    let mut h = arnes(bench.clone(), 1440.0, 900.0, move |a| ir_a(a, &principal));
    esperar(&mut h, 6);
    {
        let app = h.state().as_ref().unwrap();
        assert!(!app.clips.is_empty(), "deben cargarse los clips");
        assert!(app.clips.iter().any(|c| c.entrada.is_some()), "hay clips analizados");
        assert!(app.clips.iter().any(|c| !c.tramos.is_empty()), "hay tramos aprobados");
        assert!(app.criterio.nombre == "bodas", "criterio por defecto de bodas: {}", app.criterio.nombre);
        assert!(!app.cache_bodas.carriles.is_empty(), "la línea de tiempo tiene carriles de momentos");
        assert!(app.clips.iter().flat_map(|c| &c.tramos).all(|t| !t.momento.is_empty()), "cada tramo lleva su momento");
        // cada clip analizado tiene su miniatura (verticales Sony y HEVC de iPhone incluidos)
        let faltan: Vec<String> = app.clips.iter().filter(|c| c.entrada.is_some() && !app.texturas_mini.contains_key(&c.clave)).map(|c| c.nombre()).collect();
        assert!(faltan.is_empty(), "miniaturas que no llegaron: {faltan:?}");
    }
    guardar(&mut h, "bodas_principal.png");

    // 2) modo oscuro
    let principal2 = std::env::var("CAMCUTS_BODAS_CLIP").unwrap_or_else(|_| "C9058".into());
    let mut h = arnes(bench.clone(), 1440.0, 900.0, move |a| {
        a.alternar_modo();
        ir_a(a, &principal2);
    });
    esperar(&mut h, 6);
    guardar(&mut h, "bodas_oscuro.png");

    // 3) clip vertical: los controles quedan bajo el cuadro, nunca encima
    let mut h = arnes(bench.clone(), 1440.0, 900.0, move |a| ir_a(a, &vertical));
    esperar(&mut h, 3);
    guardar(&mut h, "bodas_vertical.png");

    // 4) pantalla grande con el panel de color y la línea de tiempo alta
    let mut h = arnes(bench.clone(), 1920.0, 1080.0, |a| {
        a.paneles.color = true;
        a.alto_tiempo = 420.0;
        ir_a(a, "C9061");
    });
    esperar(&mut h, 3);
    guardar(&mut h, "bodas_1920x1080.png");

    // 5) pantalla pequeña
    let mut h = arnes(bench.clone(), 1280.0, 720.0, |a| ir_a(a, "C9062"));
    esperar(&mut h, 3);
    guardar(&mut h, "bodas_1280x720.png");

    // 6) solo el visor y los momentos (paneles ocultos)
    let mut h = arnes(bench, 1440.0, 900.0, |a| {
        a.paneles.material = false;
        a.paneles.pesos = false;
        ir_a(a, "C9057");
    });
    esperar(&mut h, 0);
    guardar(&mut h, "bodas_foco.png");
}
