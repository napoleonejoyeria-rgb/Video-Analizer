//! Capturas sin pantalla de la ventana principal (egui_kittest + wgpu fuera de pantalla).
//!
//! Nunca abre una ventana: la interfaz se dibuja en una textura de wgpu y se guarda como PNG en
//! `camcuts_rs/design/snapshots/`. Necesita una carpeta de resultados hecha por Python (por ejemplo
//! una copia de `camcuts_next/bench_oct27`, mejor con ≥ 25 clips) en la variable `CAMCUTS_BENCH`;
//! sin ella la prueba se salta.
//!
//!   set CAMCUTS_BENCH=C:\ruta\a\copia\bench_oct27
//!   cargo test -p camcuts-gui --test capturas -- --nocapture

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use camcuts_gui::App;
use egui_kittest::Harness;

/// Carpeta donde se guardan las capturas.
fn carpeta_capturas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("design").join("snapshots")
}

/// Crea el arnés de `ancho`×`alto` con la app abierta sobre la carpeta de resultados.
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

/// Avanza cuadros hasta que llegue el cuadro del visor y las miniaturas (o se agote el tiempo).
fn esperar(h: &mut Harness<'static, Option<App>>, miniaturas: usize) {
    let inicio = Instant::now();
    loop {
        h.step();
        std::thread::sleep(Duration::from_millis(30));
        let listo = h.state().as_ref().map(|a| a.tex_visor.is_some() && a.texturas_mini.len() >= miniaturas).unwrap_or(false);
        if listo || inicio.elapsed() > Duration::from_secs(40) {
            break;
        }
    }
    // unas vueltas más para que terminen las animaciones de entrada
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

/// Elige el clip cuyo nombre empieza por `nombre` y va a la mitad de su primer tramo.
fn ir_a(app: &mut App, nombre: &str) {
    if let Some(i) = app.clips.iter().position(|c| c.nombre().starts_with(nombre)) {
        app.clic_fila(i, false, false);
        let t = app.clips[i].tramos.first().map(|s| (s.inicio + s.fin) / 2.0).unwrap_or(1.0);
        app.buscar(t);
    }
}

#[test]
fn capturas_pantalla_principal() {
    let Some(bench) = std::env::var_os("CAMCUTS_BENCH").map(PathBuf::from).filter(|p| p.exists()) else {
        eprintln!("CAMCUTS_BENCH no está definida: se salta la prueba de capturas");
        return;
    };

    // 1) pantalla principal a 1440×900: lista densa con todos los clips a la vista
    let mut h = arnes(bench.clone(), 1440.0, 900.0, |a| ir_a(a, "C9172"));
    esperar(&mut h, 12);
    {
        let app = h.state().as_ref().unwrap();
        assert!(app.clips.len() >= 19, "deben cargarse los clips de Python");
        assert!(app.clips.iter().all(|c| c.entrada.is_some()), "todos analizados");
        assert!(app.clips.iter().any(|c| !c.tramos.is_empty()), "hay tramos aprobados");
        assert!(app.titulo().contains("Octubre 27"), "el título es la carpeta del material: {}", app.titulo());
        assert!(app.filas_visibles >= 25, "a 1440×900 caben {} filas (se piden ≥ 25)", app.filas_visibles);
    }
    guardar(&mut h, "marfil_principal.png");

    // 2) línea de tiempo muy alta con el panel de color abierto y las tarjetas desplegadas:
    //    nada debe cortarse (las tarjetas se pliegan o desplazan su contenido)
    let mut h = arnes(bench.clone(), 1440.0, 900.0, |a| {
        a.paneles.color = true;
        a.titulo_plegado = false;
        a.recibo_plegado = false;
        a.alto_tiempo = 560.0;
        ir_a(a, "C9175");
        a.enfocar_clip();
    });
    esperar(&mut h, 3);
    guardar(&mut h, "marfil_linea_alta.png");

    // 3) tarjetas desplegadas (título con estado y recibo completo)
    let mut h = arnes(bench.clone(), 1440.0, 900.0, |a| {
        a.titulo_plegado = false;
        a.recibo_plegado = false;
        ir_a(a, "C9169");
    });
    esperar(&mut h, 3);
    guardar(&mut h, "marfil_recibo.png");

    // 4) pantalla pequeña: 1280×720
    let mut h = arnes(bench.clone(), 1280.0, 720.0, |a| ir_a(a, "C9177"));
    esperar(&mut h, 3);
    guardar(&mut h, "marfil_1280x720.png");

    // 5) pantalla grande: 1920×1080
    let mut h = arnes(bench.clone(), 1920.0, 1080.0, |a| {
        a.paneles.color = true;
        ir_a(a, "C9181");
    });
    esperar(&mut h, 3);
    guardar(&mut h, "marfil_1920x1080.png");

    // 6) modo foco: columnas ocultas, solo visor y línea de tiempo
    let mut h = arnes(bench.clone(), 1440.0, 900.0, |a| {
        a.paneles.material = false;
        a.paneles.pesos = false;
        ir_a(a, "C9177");
    });
    esperar(&mut h, 0);
    guardar(&mut h, "marfil_foco.png");

    // 7) menú de material abierto con el selector de detector en "Propio (beta)": las cachés del bench
    //    son de YOLO-World, así que todos los clips quedan "Reanalizar · detector" (punto coral)
    let mut h = arnes(bench.clone(), 1440.0, 900.0, |a| {
        ir_a(a, "C9172");
        a.cambiar_detector(camcuts_core::detector::DetectorKind::Propio);
        egui::Popup::open_id(&a.ctx, egui::Id::unique("popup_material"));
    });
    esperar(&mut h, 12);
    {
        let app = h.state().as_ref().unwrap();
        assert!(app.clips.iter().all(|c| app.estado_de(c) == "Reanalizar · detector"), "las cachés de YOLO-World piden reanalizar con el propio");
    }
    guardar(&mut h, "marfil_detector.png");

    // 8) análisis en curso: tres clips "sin analizar", uno al 37 %, otro en cola (0 %) y otro con error;
    //    y el mapa con la línea de tiempo del alto de siempre (200) y zoom ×4 (la franja marca lo visible)
    let mut h = arnes(bench, 1440.0, 900.0, |a| {
        ir_a(a, "C9172");
        a.alto_tiempo = 200.0;
        a.paneles.mapa = true;
        a.zoom_mapa = 4.0;
        for (nombre, avance) in [("C9173", Some(Some(37u8))), ("C9174", None), ("C9175", Some(None))] {
            if let Some(c) = a.clips.iter_mut().find(|c| c.nombre().starts_with(nombre)) {
                c.entrada = None;
                if let Some(v) = avance {
                    a.avance_clips.insert(c.clave.clone(), v);
                }
            }
        }
    });
    esperar(&mut h, 12);
    {
        let app = h.state().as_ref().unwrap();
        let de = |n: &str| app.clips.iter().find(|c| c.nombre().starts_with(n)).and_then(|c| app.avance_de(c));
        assert_eq!(de("C9173"), Some(Some(37)));
        assert_eq!(de("C9174"), Some(Some(0)), "en cola");
        assert_eq!(de("C9175"), Some(None), "con error");
        assert_eq!(de("C9172"), None, "los ya analizados enseñan su puntuación");
    }
    guardar(&mut h, "marfil_avance_y_mapa.png");
}
