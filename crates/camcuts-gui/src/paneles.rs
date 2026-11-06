//! Dibujo de la ventana con el diseño Marfil: riel de iconos, columna izquierda (título, recibo del
//! clip, color y lista de clips), visor con transporte, columna "Pesos" y línea de tiempo.
//!
//! La distribución es absoluta (rectángulos calculados), como la maqueta: así cada panel puede
//! ocultarse con una animación que lo desliza y desvanece, y la línea de tiempo cambia de alto
//! arrastrando su borde superior.

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, Id, Key, Layout, Modifiers, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder};

use crate::app::{App, Columna};
use crate::datos::{codigo_tiempo, fmt_corto, fmt_tiempo};
use crate::iconos::Icono;
use crate::motor;
use crate::tema::*;
use crate::widgets::{self, Estilo, Tarjeta};

/// Altos de las tarjetas de la columna izquierda (desplegadas y plegadas).
const ALTO_COLOR: f32 = 166.0;
const ALTO_TITULO: f32 = 86.0;
const ALTO_TITULO_PLEGADO: f32 = 52.0;
const ALTO_RECIBO: f32 = 160.0;
const ALTO_RECIBO_PLEGADO: f32 = 62.0;
/// Alto de cada fila de la lista de clips.
const ALTO_FILA: f32 = 25.0;

/// Iconos y colores de cada peso (en el orden de `motor::PESOS`).
const ESTILO_PESOS: [(Icono, Color32); 6] = [
    (Icono::Diamante, ORO),
    (Icono::Cuadro, CORAL),
    (Icono::Sol, LAVANDA),
    (Icono::Cruz, MENTA),
    (Icono::Circulos, CIELO),
    (Icono::Onda, ROSA),
];

/// Un hijo de `ui` limitado a `rect`, con opacidad y recorte.
fn hijo(ui: &mut Ui, rect: Rect, id: &str, opacidad: f32, recorte: Rect) -> Ui {
    let mut h = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::top_down(Align::Min)));
    h.set_clip_rect(recorte.intersect(ui.clip_rect()));
    h.multiply_opacity(opacidad);
    h
}

impl App {
    /// Dibuja toda la ventana dentro de `ui`.
    pub fn dibujar(&mut self, ui: &mut Ui) {
        // la edición Bodas tiene su propia ventana (diseño Salvia) sobre el mismo estado
        if motor::bodas() {
            return self.dibujar_bodas(ui);
        }
        self.antes_de_dibujar();
        self.atender_cierre(&ui.ctx().clone());
        self.atajos(ui);
        // toda la ventana (sin los márgenes que pueda poner el contenedor)
        let pantalla = ui.ctx().content_rect();
        ui.painter().with_clip_rect(pantalla).rect_filled(pantalla, 0, FONDO);

        // --- animaciones de mostrar/ocultar (0 = oculto, 1 = visible) ---
        let ctx = ui.ctx().clone();
        let a_mat = ctx.animate_bool_with_time(Id::unique("a_mat"), self.paneles.material, 0.25);
        let a_pes = ctx.animate_bool_with_time(Id::unique("a_pes"), self.paneles.pesos, 0.25);
        let a_tie = ctx.animate_bool_with_time(Id::unique("a_tie"), self.paneles.tiempo, 0.25);
        let a_col = ctx.animate_bool_with_time(Id::unique("a_col"), self.paneles.color, 0.25);
        let suave = |x: f32| x * x * (3.0 - 2.0 * x);
        let (a_mat, a_pes, a_tie, a_col) = (suave(a_mat), suave(a_pes), suave(a_tie), suave(a_col));

        // --- distribución ---
        let r = pantalla.shrink(MARGEN);
        let riel = Rect::from_min_size(r.min, vec2(ANCHO_RIEL, r.height()));
        let x0 = riel.right() + HUECO;
        let max_tiempo = (r.height() * 0.62).max(140.0);
        self.alto_tiempo = self.alto_tiempo.clamp(120.0, max_tiempo);
        let alto_t = self.alto_tiempo * a_tie;
        // la columna izquierda ocupa todo el alto (la lista de clips necesita sitio para ~25 filas);
        // la línea de tiempo va debajo del visor y de los pesos
        let w_izq = ANCHO_COLUMNA * a_mat;
        let w_der = ANCHO_COLUMNA * a_pes;
        let izq = Rect::from_min_size(pos2(x0, r.top()), vec2(w_izq, r.height()));
        let x_c = izq.right() + HUECO * a_mat;
        let tiempo = Rect::from_min_max(pos2(x_c, r.bottom() - alto_t), pos2(r.right(), r.bottom()));
        let sup = Rect::from_min_max(pos2(x_c, r.top()), pos2(r.right(), r.bottom() - alto_t - HUECO * a_tie));
        let der = Rect::from_min_max(pos2(sup.right() - w_der, sup.top()), sup.max);
        let centro = Rect::from_min_max(sup.min, pos2(der.left() - HUECO * a_pes, sup.bottom()));

        self.riel(ui, riel);
        if a_mat > 0.01 {
            // la columna se dibuja a su ancho completo y se desliza bajo el borde al ocultarse
            let col = Rect::from_min_size(pos2(izq.right() - ANCHO_COLUMNA, izq.top()), vec2(ANCHO_COLUMNA, izq.height()));
            // recortada a su zona: si la línea de tiempo sube, la columna no se dibuja debajo de ella
            let recorte = Rect::from_min_max(pos2(riel.right() + 2.0, pantalla.top()), pos2(izq.right() + HUECO, pantalla.bottom()));
            let mut h = hijo(ui, col, "izq", a_mat, recorte);
            self.columna_izquierda(&mut h, col, a_col);
        }
        if a_pes > 0.01 {
            let col = Rect::from_min_size(der.min, vec2(ANCHO_COLUMNA, der.height()));
            let recorte = Rect::from_min_max(pos2(der.left() - HUECO, pantalla.top()), pantalla.max);
            let mut h = hijo(ui, col, "der", a_pes, recorte);
            self.columna_pesos(&mut h, col);
        }
        {
            let mut h = hijo(ui, centro, "centro", 1.0, pantalla);
            self.centro(&mut h, centro);
        }
        if a_tie > 0.01 {
            let caja = Rect::from_min_size(tiempo.min, vec2(tiempo.width(), self.alto_tiempo));
            let mut h = hijo(ui, caja, "tiempo", a_tie, Rect::from_min_max(pos2(x_c - HUECO + 2.0, tiempo.top() - 40.0), pantalla.max));
            self.linea_de_tiempo(&mut h, caja, max_tiempo);
        }
        self.dibujar_aviso(ui);
    }

    // =============================================================================================
    // Atajos de teclado
    // =============================================================================================

    pub(crate) fn atajos(&mut self, ui: &mut Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let ctrl = Modifiers::COMMAND;
        let mut paneles = self.paneles;
        let (mut play, mut atras, mut adelante, mut siguiente, mut inicio, mut arriba, mut abajo) = (false, false, false, false, false, false, false);
        let (mut abrir, mut guardar_como, mut nuevo, mut exportar) = (false, false, false, false);
        ui.input_mut(|i| {
            if i.consume_key(ctrl, Key::Num1) { paneles.material = !paneles.material; }
            if i.consume_key(ctrl, Key::Num2) { paneles.pesos = !paneles.pesos; }
            if i.consume_key(ctrl, Key::Num3) { paneles.color = !paneles.color; }
            if i.consume_key(ctrl, Key::Num4) { paneles.tiempo = !paneles.tiempo; }
            if i.consume_key(ctrl, Key::Num5) { paneles.mapa = !paneles.mapa; }
            abrir = i.consume_key(ctrl, Key::O);
            guardar_como = i.consume_key(ctrl | Modifiers::SHIFT, Key::S);
            nuevo = i.consume_key(ctrl, Key::N);
            exportar = i.consume_key(ctrl, Key::E);
            play = i.consume_key(Modifiers::NONE, Key::Space);
            atras = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
            adelante = i.consume_key(Modifiers::NONE, Key::ArrowRight);
            siguiente = i.consume_key(Modifiers::NONE, Key::N);
            inicio = i.consume_key(Modifiers::NONE, Key::Home);
            arriba = i.consume_key(Modifiers::NONE, Key::ArrowUp);
            abajo = i.consume_key(Modifiers::NONE, Key::ArrowDown);
        });
        if paneles != self.paneles {
            let antes = self.paneles;
            self.paneles = paneles;
            self.al_cambiar_paneles(antes);
            self.marcar_sucio();
        }
        if play { self.alternar_reproduccion(); }
        if atras { self.buscar(self.t - 2.0); }
        if adelante { self.buscar(self.t + 2.0); }
        if siguiente { self.siguiente_tramo(); }
        if inicio { self.buscar(0.0); }
        if arriba || abajo {
            let n = self.clips.len();
            if n > 0 {
                let i = match self.actual { Some(i) if arriba => i.saturating_sub(1), Some(i) => (i + 1).min(n - 1), None => 0 };
                self.clic_fila(i, false, false);
            }
        }
        if abrir { self.abrir_proyecto_dialogo(); }
        if guardar_como { self.guardar_como(); }
        if nuevo { self.nuevo_proyecto(); }
        if exportar { self.exportar(); }
    }

    /// Al encender el mapa, que se vea: va dentro de la línea de tiempo, así que la muestra si estaba
    /// oculta y le da el alto que necesita la franja. Solo en joyas: en Bodas ese botón es otro panel.
    pub(crate) fn al_cambiar_paneles(&mut self, antes: crate::app::Paneles) {
        if self.paneles.mapa && !antes.mapa && !motor::bodas() {
            self.paneles.tiempo = true;
            self.alto_tiempo = self.alto_tiempo.max(200.0);
        }
    }

    // =============================================================================================
    // Riel
    // =============================================================================================

    fn riel(&mut self, ui: &mut Ui, rect: Rect) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Oscura, 1.0);
        let mut h = ui.new_child(UiBuilder::new().id_salt("riel").max_rect(rect.shrink2(vec2(0.0, 20.0))).layout(Layout::top_down(Align::Center)));
        h.spacing_mut().item_spacing.y = 12.0;
        h.label(egui::RichText::new("C").font(f_titulo(26.0)).color(ORO));
        h.add_space(16.0);
        let botones: [(Icono, &str, fn(&mut crate::app::Paneles) -> &mut bool); 5] = [
            (Icono::Material, "Material · Ctrl+1", |p| &mut p.material),
            (Icono::Pesos, "Pesos y criterio · Ctrl+2", |p| &mut p.pesos),
            (Icono::Color, "Color y joya del clip · Ctrl+3", |p| &mut p.color),
            (Icono::Tiempo, "Línea de tiempo · Ctrl+4", |p| &mut p.tiempo),
            (Icono::Mapa, "Mapa: todo el material en una franja sobre la línea de tiempo; con zoom (rueda) marca lo que ves y se arrastra para moverte · Ctrl+5", |p| &mut p.mapa),
        ];
        for (icono, ayuda, campo) in botones {
            let activo = *campo(&mut self.paneles);
            if widgets::boton_riel(&mut h, icono, activo, ayuda).clicked() {
                let antes = self.paneles;
                let v = campo(&mut self.paneles);
                *v = !*v;
                self.al_cambiar_paneles(antes);
                self.marcar_sucio();
            }
        }
        // menú del proyecto: el círculo dorado de abajo
        let abajo = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 44.0), vec2(48.0, 48.0));
        let resp = ui.interact(abajo, Id::unique("menu_proyecto"), Sense::click()).on_hover_text("Proyecto").on_hover_cursor(CursorIcon::PointingHand);
        let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
        let p = ui.painter();
        if hv > 0.0 {
            p.circle_filled(abajo.center(), 24.0 + 5.0 * hv, oro_a(0.2 * hv));
        }
        p.circle_filled(abajo.center(), 24.0, ORO);
        p.text(abajo.center(), Align2::CENTER_CENTER, "N", f_negrita(17.0), TINTA);
        let mut accion = 0;
        egui::Popup::menu(&resp).id(Id::unique("popup_proyecto")).width(280.0).show(|ui| {
            ui.set_min_width(280.0);
            ui.label(egui::RichText::new("Proyecto").font(f_titulo(20.0)).color(MARFIL));
            if !self.etiqueta_proyecto.is_empty() {
                ui.label(egui::RichText::new(&self.etiqueta_proyecto).font(f_texto(12.0)).color(APAGADO));
            }
            ui.add_space(4.0);
            if widgets::item_menu(ui, "Nuevo proyecto        Ctrl+N", false).clicked() { accion = 1; }
            if widgets::item_menu(ui, "Abrir proyecto…       Ctrl+O", false).clicked() { accion = 2; }
            if widgets::item_menu(ui, "Guardar como…   Ctrl+Mayús+S", false).clicked() { accion = 3; }
            ui.separator();
            ui.label(egui::RichText::new(format!("Resultados en: {}", if self.out.is_empty() { "—" } else { &self.out })).font(f_texto(12.0)).color(APAGADO));
            if widgets::item_menu(ui, "Cambiar carpeta de resultados…", false).clicked() { accion = 4; }
            if widgets::item_menu(ui, "Abrir carpeta de resultados", false).clicked() { accion = 5; }
            if widgets::item_menu(ui, "Abrir resultados existentes (sin material)…", false).clicked() { accion = 6; }
            ui.separator();
            ui.label(egui::RichText::new("Espacio reproducir · ←/→ ±2 s · N siguiente tramo · ↑/↓ clip · Ctrl+E exportar").font(f_texto(11.5)).color(APAGADO));
        });
        match accion {
            1 => self.nuevo_proyecto(),
            2 => self.abrir_proyecto_dialogo(),
            3 => self.guardar_como(),
            4 => self.elegir_out(),
            5 => self.abrir_carpeta_resultados(),
            // no con un análisis en curso, y guardando antes lo pendiente
            6 => self.abrir_resultados_dialogo("Carpeta de resultados de CAMCUTS"),
            _ => {}
        }
    }

    // =============================================================================================
    // Columna izquierda
    // =============================================================================================

    /// Columna izquierda. La lista de clips se queda con todo el alto libre; si no le llega para unas
    /// cuantas filas, el título y el recibo se pliegan solos y la tarjeta de color se encoge (con
    /// desplazamiento por dentro), así nada se corta a ningún alto de la línea de tiempo.
    fn columna_izquierda(&mut self, ui: &mut Ui, col: Rect, a_col: f32) {
        const LISTA_MIN: f32 = 200.0;
        let alto = col.height();
        let mut tit_pleg = self.titulo_plegado;
        let mut rec_pleg = self.recibo_plegado;
        let alto_tit = |p: bool| if p { ALTO_TITULO_PLEGADO } else { ALTO_TITULO };
        let alto_rec = |p: bool| if p { ALTO_RECIBO_PLEGADO } else { ALTO_RECIBO };
        let fijo = |t: bool, r: bool| alto_tit(t) + alto_rec(r) + 2.0 * HUECO + (ALTO_COLOR + HUECO) * a_col;
        // plegado automático (solo visual: la preferencia del editor no cambia)
        if alto - fijo(tit_pleg, rec_pleg) < LISTA_MIN {
            tit_pleg = true;
        }
        if alto - fijo(tit_pleg, rec_pleg) < LISTA_MIN {
            rec_pleg = true;
        }
        let h_tit = alto_tit(tit_pleg);
        let h_rec = alto_rec(rec_pleg);
        // la tarjeta de color cede alto antes que la lista baje de su mínimo
        let libre_color = (alto - h_tit - h_rec - 2.0 * HUECO - LISTA_MIN - HUECO).max(0.0);
        let h_color = ALTO_COLOR.min(libre_color.max(96.0)) * a_col;
        let titulo = Rect::from_min_size(col.min, vec2(col.width(), h_tit));
        let recibo = Rect::from_min_size(pos2(col.left(), titulo.bottom() + HUECO), vec2(col.width(), h_rec));
        let color = Rect::from_min_size(pos2(col.left(), recibo.bottom() + HUECO * a_col), vec2(col.width(), h_color));
        let lista = Rect::from_min_max(pos2(col.left(), color.bottom() + HUECO), col.max);
        self.tarjeta_titulo(ui, titulo, tit_pleg);
        self.tarjeta_recibo(ui, recibo, rec_pleg);
        if a_col > 0.01 && color.height() > 8.0 {
            let mut h = hijo(ui, color, "color", a_col, color.expand(30.0));
            self.tarjeta_color(&mut h, color);
        }
        if lista.height() > 60.0 {
            self.tarjeta_clips(ui, lista);
        }
    }

    /// Botón redondo pequeño con chevrón (plegar/desplegar). `abajo`: el chevrón apunta abajo.
    fn chevron(ui: &mut Ui, centro: Pos2, id: &str, abajo: bool, sobre_marfil: bool) -> egui::Response {
        let r = Rect::from_center_size(centro, vec2(24.0, 24.0));
        let resp = ui.interact(r, Id::unique(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
        let p = ui.painter();
        let (fondo, trazo) = if sobre_marfil { (tinta_a(0.06 + 0.08 * hv), TINTA) } else { (mezclar(OSCURO_2, OSCURO_HOVER, hv), MARFIL) };
        p.circle_filled(centro, 12.0, fondo);
        let d = if abajo { 1.0 } else { -1.0 };
        p.line(vec![centro + vec2(-4.0, -2.0 * d), centro + vec2(0.0, 2.0 * d), centro + vec2(4.0, -2.0 * d)], Stroke::new(1.6, trazo));
        resp
    }

    fn tarjeta_titulo(&mut self, ui: &mut Ui, rect: Rect, plegado: bool) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Oscura, 1.0);
        let p = ui.painter().clone();
        let interior = rect.shrink2(vec2(22.0, 12.0));
        let ch = pos2(interior.right() - 10.0, if plegado { rect.center().y } else { interior.top() + 9.0 });
        let resp = Self::chevron(ui, ch, "plegar_titulo", plegado, false).on_hover_text(if plegado { "Mostrar título y estado" } else { "Plegar el título" });
        if resp.clicked() {
            self.titulo_plegado = !self.titulo_plegado;
            self.marcar_sucio();
        }
        let ancho = interior.width() - 30.0;
        if plegado {
            widgets::texto_corto(&p, pos2(interior.left(), rect.center().y), Align2::LEFT_CENTER, &self.titulo(), f_titulo(21.0), MARFIL, ancho);
        } else {
            p.text(interior.left_top(), Align2::LEFT_TOP, "CAMCUTS", f_texto(12.0), APAGADO);
            if self.etiqueta_proyecto.contains("guardado") {
                let hora = self.etiqueta_proyecto.rsplit("guardado ").next().unwrap_or("");
                p.text(pos2(ch.x - 18.0, interior.top()), Align2::RIGHT_TOP, format!("guardado {hora}"), f_texto(11.5), APAGADO);
            }
            widgets::texto_corto(&p, interior.left_top() + vec2(0.0, 16.0), Align2::LEFT_TOP, &self.titulo(), f_titulo(24.0), MARFIL, interior.width());
            widgets::texto_corto(&p, pos2(interior.left(), interior.bottom()), Align2::LEFT_BOTTOM, &self.estado, f_texto(11.5), self.estado_color, interior.width());
        }
        // progreso del análisis: hilo dorado en el borde inferior de la tarjeta
        if let Some((a, b)) = self.progreso {
            let barra = Rect::from_min_size(pos2(rect.left() + 22.0, rect.bottom() - 6.0), vec2(rect.width() - 44.0, 3.0));
            p.rect_filled(barra, 2, OSCURO_2);
            let f = if b > 0 { a as f32 / b as f32 } else { 0.0 };
            let fa = ui.ctx().animate_value_with_time(Id::unique("prog"), f, 0.4);
            p.rect_filled(Rect::from_min_size(barra.min, vec2(barra.width() * fa.max(0.02), 3.0)), 2, ORO);
        }
        ui.interact(rect.shrink(30.0), Id::unique("titulo_hover"), Sense::hover()).on_hover_text(format!("{}\n{}", self.titulo(), self.estado));
    }

    /// Joyas del clip (las más frecuentes en sus tramos, o en sus muestras) y si el tipo está fijado.
    fn joyas_del_clip(&self, i: usize) -> (String, bool) {
        let c = &self.clips[i];
        if let Some(f) = self.tipos.get(&c.clave) {
            return (f.clone(), true);
        }
        let mut cuenta: Vec<(String, usize)> = Vec::new();
        let sumar = |cuenta: &mut Vec<(String, usize)>, j: &str| match cuenta.iter_mut().find(|x| x.0 == j) {
            Some(x) => x.1 += 1,
            None => cuenta.push((j.to_owned(), 1)),
        };
        for s in &c.tramos {
            for j in &s.presentes {
                sumar(&mut cuenta, j);
            }
        }
        if cuenta.is_empty() {
            if let Some(e) = &c.entrada {
                for m in &e.muestras {
                    for j in &m.joyas {
                        sumar(&mut cuenta, &j.etiqueta);
                    }
                }
            }
        }
        cuenta.sort_by(|a, b| b.1.cmp(&a.1));
        let nombres: Vec<String> = cuenta.into_iter().take(2).map(|x| x.0).collect();
        let t = if !nombres.is_empty() {
            nombres.join(" · ")
        } else if c.entrada.is_some() {
            "sin joya".into()
        } else {
            "pendiente".into()
        };
        (t, false)
    }

    fn tarjeta_recibo(&mut self, ui: &mut Ui, rect: Rect, plegado: bool) {
        let y_corte = rect.top() + 104.0;
        if plegado {
            widgets::tarjeta(ui.painter(), rect, Tarjeta::Marfil, 1.0);
        } else {
            widgets::recibo(ui.painter(), rect, y_corte, 1.0);
        }
        let p = ui.painter().clone();
        let x0 = rect.left() + 22.0;
        let x1 = rect.right() - 22.0;
        let actual = self.actual.filter(|&i| i < self.clips.len());
        let (nombre, aprob, meta) = match actual.map(|i| &self.clips[i]) {
            Some(c) => {
                let m = &c.meta;
                let tc = c.entrada.as_ref().map(|e| e.tc_inicio.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
                    let (offs, _) = self.desplazamientos();
                    codigo_tiempo(offs.get(self.actual.unwrap_or(0)).copied().unwrap_or(0.0), self.cadencia())
                });
                let fps = m.fps.or(c.entrada.as_ref().map(|e| e.cadencia())).map(|f| format!("{f:.2}")).unwrap_or_default();
                let partes: Vec<String> = [m.codec.clone().unwrap_or_default(), m.res.clone().unwrap_or_default(), fps, format!("TC {tc}")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                (c.base(), format!("{} de {}", fmt_corto(c.aprobado), fmt_corto(c.dur())), partes.join(" · "))
            }
            None => ("—".into(), "—".into(), "Elige un clip de la lista".into()),
        };
        let (joyas, forzado) = actual.map(|i| self.joyas_del_clip(i)).unwrap_or_else(|| ("joya de este clip".into(), false));
        let n_tramos = actual.map(|i| self.clips[i].tramos.len()).unwrap_or(0);

        if plegado {
            // una fila: clip y aprobado; debajo, las joyas. Un clic despliega el recibo.
            let resp = ui.interact(rect, Id::unique("recibo_desplegar"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Mostrar el recibo del clip");
            let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
            if hv > 0.0 {
                p.rect_filled(rect, RADIO_MARFIL, tinta_a(0.04 * hv));
            }
            let g_ap = p.layout_no_wrap(aprob, f_negrita(15.0), TINTA);
            let x_ap = x1 - 30.0 - g_ap.size().x;
            p.galley(pos2(x_ap, rect.top() + 14.0), g_ap, TINTA);
            widgets::texto_corto(&p, pos2(x0, rect.top() + 14.0), Align2::LEFT_TOP, &nombre, f_negrita(15.0), TINTA, x_ap - x0 - 10.0);
            let sub = format!("{joyas} · {n_tramos} tramo{}", if n_tramos == 1 { "" } else { "s" });
            widgets::texto_corto(&p, pos2(x0, rect.top() + 36.0), Align2::LEFT_TOP, &sub, f_texto(12.0), if forzado { Color32::from_rgb(0x9a, 0x6e, 0x1c) } else { TINTA_APAGADA }, x1 - x0 - 30.0);
            let ch = pos2(x1 - 6.0, rect.center().y);
            let pc = ui.painter();
            pc.circle_filled(ch, 12.0, tinta_a(0.06 + 0.08 * hv));
            pc.line(vec![ch + vec2(-4.0, -2.0), ch + vec2(0.0, 2.0), ch + vec2(4.0, -2.0)], Stroke::new(1.6, TINTA));
            if resp.clicked() {
                self.recibo_plegado = false;
                self.marcar_sucio();
            }
            return;
        }

        // píldora "Recibo del clip": también pliega el recibo
        let g = p.layout_no_wrap("Recibo del clip".into(), f_texto(12.5), MARFIL);
        let pill = Rect::from_center_size(pos2(rect.center().x, rect.top() + 12.0 + 12.0), vec2(g.size().x + 44.0, 24.0));
        let rp = ui.interact(pill, Id::unique("recibo_plegar"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Plegar el recibo");
        let hp = widgets::anim(ui, rp.id.with("h"), rp.hovered(), 0.15);
        p.rect_filled(pill, 12, mezclar(TINTA, Color32::from_rgb(0x3a, 0x31, 0x22), hp));
        p.galley(pos2(pill.left() + 16.0, pill.center().y - g.size().y / 2.0), g, MARFIL);
        let ch = pos2(pill.right() - 14.0, pill.center().y);
        p.line(vec![ch + vec2(-3.5, 1.8), ch + vec2(0.0, -1.8), ch + vec2(3.5, 1.8)], Stroke::new(1.4, ARENA));
        if rp.clicked() {
            self.recibo_plegado = true;
            self.marcar_sucio();
        }
        let y = pill.bottom() + 9.0;
        p.text(pos2(x0, y), Align2::LEFT_TOP, "Clip", f_texto(11.5), TINTA_APAGADA);
        p.text(pos2(x1, y), Align2::RIGHT_TOP, "Aprobado", f_texto(11.5), TINTA_APAGADA);
        let g_ap = p.layout_no_wrap(aprob, f_negrita(18.0), TINTA);
        let x_ap = x1 - g_ap.size().x;
        p.galley(pos2(x_ap, y + 14.0), g_ap, TINTA);
        widgets::texto_corto(&p, pos2(x0, y + 14.0), Align2::LEFT_TOP, &nombre, f_negrita(18.0), TINTA, x_ap - x0 - 12.0);
        widgets::texto_corto(&p, pos2(x0, y + 38.0), Align2::LEFT_TOP, &meta, f_texto(11.0), TINTA_APAGADA, x1 - x0);

        // recuadro oscuro: joyas del clip; es también el selector "Joya de este clip"
        let caja = Rect::from_min_max(pos2(x0, y_corte + 10.0), pos2(x1, rect.bottom() - 12.0));
        let resp = ui
            .interact(caja, Id::unique("recibo_joya"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Joya de este clip: corrige el nombre en todo el clip y queda como ejemplo para entrenar");
        let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
        p.rect_filled(caja, 14, mezclar(TINTA, Color32::from_rgb(0x2a, 0x24, 0x1a), hv));
        // dos "gemas" solapadas (degradado radial como en la maqueta)
        let rad = (caja.height() / 2.0 - 5.0).clamp(8.0, 14.0);
        let c1 = pos2(caja.left() + 10.0 + rad, caja.center().y);
        let c2 = c1 + vec2(rad * 1.4, 0.0);
        for (c, a, b) in [(c1, Color32::from_rgb(0xf3, 0xdf, 0xb4), Color32::from_rgb(0x8c, 0x6b, 0x3a)), (c2, Color32::from_rgb(0xe6, 0xd3, 0xc2), Color32::from_rgb(0x6e, 0x55, 0x45))] {
            p.circle_filled(c, rad + 1.0, TINTA);
            let r = Rect::from_center_size(c, vec2(rad * 2.0, rad * 2.0));
            widgets::gradiente_radial(&p.with_clip_rect(r), r, c, vec2(rad, rad), &[(0.0, a), (1.0, b)]);
            p.circle_stroke(c, rad, Stroke::new(2.0, TINTA));
        }
        let nit = actual
            .and_then(|i| self.clips[i].entrada.as_ref())
            .and_then(|e| e.muestra_en(self.t))
            .and_then(|m| m.joyas.iter().find(|j| j.principal).or(m.joyas.first()))
            .map(|j| format!("{:.2}", j.nitidez))
            .unwrap_or_else(|| "—".into());
        let sub = format!("{n_tramos} tramo{} · nitidez {nit}", if n_tramos == 1 { "" } else { "s" });
        let tx = c2.x + rad + 10.0;
        let ancho_txt = caja.right() - 30.0 - tx;
        widgets::texto_corto(&p, pos2(tx, caja.center().y + 1.0), Align2::LEFT_BOTTOM, &joyas, f_negrita(13.5), if forzado { ORO } else { MARFIL }, ancho_txt);
        widgets::texto_corto(&p, pos2(tx, caja.center().y + 1.0), Align2::LEFT_TOP, &sub, f_texto(11.0), ARENA, ancho_txt);
        let ch = pos2(caja.right() - 15.0, caja.center().y);
        p.line(vec![ch + vec2(-4.0, -2.0), ch + vec2(0.0, 2.5), ch + vec2(4.0, -2.0)], Stroke::new(1.6, ARENA));
        let mut elegido: Option<Option<String>> = None;
        egui::Popup::menu(&resp).id(Id::unique("popup_joya")).width(220.0).show(|ui| {
            ui.set_min_width(220.0);
            ui.label(egui::RichText::new("Joya de este clip").font(f_titulo(18.0)).color(MARFIL));
            let actual_tipo = actual.and_then(|i| self.tipos.get(&self.clips[i].clave).cloned());
            if widgets::item_menu(ui, "Automático", actual_tipo.is_none()).clicked() {
                elegido = Some(None);
            }
            for j in motor::JOYAS {
                if widgets::item_menu(ui, j, actual_tipo.as_deref() == Some(j)).clicked() {
                    elegido = Some(Some(j.to_owned()));
                }
            }
        });
        if let Some(v) = elegido {
            self.poner_tipo(v);
        }
    }

    fn tarjeta_color(&mut self, ui: &mut Ui, rect: Rect) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Oscura, 1.0);
        let interior = rect.shrink2(vec2(20.0, 14.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("color_in").max_rect(interior).layout(Layout::top_down(Align::Min)));
        h.set_clip_rect(interior.intersect(ui.clip_rect()));
        // si la tarjeta se encoge, su contenido se desplaza por dentro en vez de cortarse
        let salida = egui::ScrollArea::vertical().id_salt("color_scroll").auto_shrink([false, false]).show(&mut h, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Color").font(f_titulo(22.0)).color(MARFIL));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut v = self.ver_color;
                    if widgets::interruptor(ui, &mut v, "Ver con color", false).changed() {
                        self.ver_color = v;
                        self.minis_y_cuadro();
                    }
                });
            });
            let opciones = motor::transformaciones();
            let actual = self.actual.and_then(|i| self.clips.get(i)).map(|c| self.color_de(c)).unwrap_or_else(motor::ColorSpec::auto);
            let etiquetas: Vec<String> = opciones.iter().map(|o| o.1.clone()).collect();
            let actual_txt = motor::etiqueta_color(&actual.id);
            let ancho = ui.available_width();
            if let Some(i) = widgets::desplegable(ui, Id::unique("dd_color"), &actual_txt, &etiquetas, ancho, false) {
                let id = opciones[i].0.clone();
                self.aplicar_color(&id, "este");
            }
            let n_sel = self.seleccion.len();
            let mut accion = None;
            // una sola fila: dar el color a los seleccionados, a todos, o leer el perfil de la cámara
            let mut detectar = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if widgets::pildora(ui, &format!("Selección ({n_sel})"), None, Estilo::Suave, 30.0, n_sel > 0)
                    .on_hover_text("Da este color a los clips elegidos con Ctrl/Mayús+clic")
                    .clicked()
                {
                    accion = Some("seleccionados");
                }
                if widgets::pildora(ui, "Todos", None, Estilo::Suave, 30.0, !self.clips.is_empty()).clicked() {
                    accion = Some("todos");
                }
                if widgets::pildora(ui, "Detectar", Some(Icono::Lupa), Estilo::Marfil, 30.0, !self.clips.is_empty())
                    .on_hover_text("Lee el perfil de cámara (XML de Sony, metadatos) de los seleccionados, o de todos")
                    .clicked()
                {
                    detectar = true;
                }
            });
            if let Some(a) = accion {
                self.aplicar_color(&actual.id, a);
            }
            if detectar {
                self.detectar_colores();
            }
        });
        widgets::desvanecer_scroll(ui.painter(), &salida, OSCURO);
    }

    /// Cambió "ver con color": rehacer cuadro y miniaturas.
    fn minis_y_cuadro(&mut self) {
        self.texturas_mini.clear();
        self.redibujar_cuadro();
        self.marcar_sucio();
        self.limpiar_minis();
    }

    fn tarjeta_clips(&mut self, ui: &mut Ui, rect: Rect) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Oscura, 1.0);
        let interior = rect.shrink2(vec2(10.0, 10.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("clips_in").max_rect(interior).layout(Layout::top_down(Align::Min)));
        h.set_clip_rect(interior.expand(2.0).intersect(ui.clip_rect()));
        h.spacing_mut().item_spacing = vec2(6.0, 6.0);
        // cabecera: "Clips 21", ordenar, añadir material, analizar
        let mut accion: Option<&str> = None;
        // detector elegido en el menú (se aplica después, fuera de los cierres del menú)
        let mut nuevo_detector: Option<camcuts_core::detector::DetectorKind> = None;
        h.horizontal(|ui| {
            ui.set_height(28.0);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Clips").font(f_medio(15.0)).color(MARFIL));
            ui.label(egui::RichText::new(self.clips.len().to_string()).font(f_texto(12.5)).color(APAGADO));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if self.analisis.is_some() {
                    if widgets::pildora(ui, "Detener", Some(Icono::Detener), Estilo::Suave, 28.0, true).clicked() {
                        accion = Some("detener");
                    }
                } else if widgets::pildora(ui, "Analizar", Some(Icono::Rayo), Estilo::Oro, 28.0, true)
                    .on_hover_text(format!("Analiza el material que falta (los ya analizados se saltan)\nDetector: {}", motor::nombre_detector(self.detector)))
                    .clicked()
                {
                    accion = Some("analizar");
                }
                // menú de material
                let r_mas = widgets::boton_icono(ui, Icono::Mas, 28.0, Some(OSCURO_2), MARFIL, "Material");
                egui::Popup::menu(&r_mas).id(Id::unique("popup_material")).width(300.0).show(|ui| {
                    ui.set_min_width(300.0);
                    ui.label(egui::RichText::new("Material").font(f_titulo(20.0)).color(MARFIL));
                    ui.label(egui::RichText::new(self.texto_fuentes()).font(f_texto(12.0)).color(APAGADO));
                    if widgets::item_menu(ui, "+ Carpeta…", false).clicked() { accion = Some("carpeta"); }
                    if widgets::item_menu(ui, "+ Archivos…", false).clicked() { accion = Some("archivos"); }
                    if widgets::item_menu(ui, "Quitar todo", false).clicked() { accion = Some("quitar"); }
                    ui.separator();
                    let mut r = self.recursivo;
                    if widgets::interruptor(ui, &mut r, "Subcarpetas", false).changed() { self.recursivo = r; accion = Some("recargar"); }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Análisis").font(f_medio(13.0)).color(ARENA));
                    let mut b = self.segundo_plano;
                    if widgets::interruptor(ui, &mut b, "Segundo plano (seguir editando)", false).changed() { self.segundo_plano = b; accion = Some("sucio"); }
                    let mut z = self.zoom_joyas;
                    if widgets::interruptor(ui, &mut z, "Zoom joyas diminutas", false).changed() { self.zoom_joyas = z; accion = Some("sucio"); }
                    // detector de joyas: "Propio" (predeterminado) o "Clásico" (YOLO-World); cambiarlo marca para
                    // reanalizar los clips hechos con el otro
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Detector").font(f_texto(12.5)).color(APAGADO));
                    let nombres: Vec<&str> = motor::DETECTORES.iter().map(|d| d.1).collect();
                    let sel = motor::DETECTORES.iter().position(|d| d.0 == self.detector).unwrap_or(0);
                    if let Some(i) = widgets::segmentado_con_ayuda(ui, Id::unique("detector"), &nombres, sel, Some(motor::AYUDA_DETECTOR)) {
                        nuevo_detector = Some(motor::DETECTORES[i].0);
                    }
                });
                // menú de orden
                let r_ord = widgets::boton_icono(ui, Icono::Orden, 28.0, Some(OSCURO_2), MARFIL, "Ordenar");
                egui::Popup::menu(&r_ord).id(Id::unique("popup_orden")).width(220.0).show(|ui| {
                    ui.set_min_width(220.0);
                    ui.label(egui::RichText::new("Ordenar por").font(f_titulo(18.0)).color(MARFIL));
                    for (col, nombre) in Columna::TODAS {
                        let marcado = self.orden.map(|o| o.0) == Some(col);
                        let flecha = match self.orden { Some((c, rev)) if c == col => if rev { "  ↓" } else { "  ↑" }, _ => "" };
                        if widgets::item_menu(ui, &format!("{nombre}{flecha}"), marcado).clicked() {
                            self.ordenar_por(col);
                        }
                    }
                });
            });
        });
        match accion {
            Some("analizar") => self.analizar(),
            Some("detener") => self.detener(),
            Some("carpeta") => self.agregar_carpeta(),
            Some("archivos") => self.agregar_archivos(),
            Some("quitar") => self.quitar_fuentes(),
            Some("recargar") => {
                self.cargar_clips();
                self.marcar_sucio();
            }
            Some("sucio") => self.marcar_sucio(),
            _ => {}
        }
        if let Some(k) = nuevo_detector {
            self.cambiar_detector(k);
        }
        // filas compactas: todo el alto libre de la tarjeta, con desplazamiento. Sin separación entre
        // filas: `show_rows` calcula las filas visibles con el alto de fila + item_spacing.y
        h.add_space(2.0);
        h.spacing_mut().item_spacing.y = 0.0;
        let alto_pie = 18.0;
        let alto_lista = (h.available_height() - alto_pie - 2.0).max(30.0);
        self.filas_visibles = (alto_lista / ALTO_FILA).floor() as usize;
        let mut clic: Option<(usize, bool, bool)> = None;
        let salida = egui::ScrollArea::vertical().id_salt("lista_clips").max_height(alto_lista).min_scrolled_height(alto_lista).auto_shrink([false, false]).show_rows(
            &mut h,
            ALTO_FILA,
            self.clips.len().max(1),
            |ui, filas| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if self.clips.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Sin clips todavía. Pulsa + para agregar una carpeta de material, o abre un proyecto desde el círculo N.").font(f_texto(12.5)).color(APAGADO));
                    return;
                }
                for i in filas {
                    if i < self.clips.len() {
                        if let Some(c) = self.fila_clip(ui, i) {
                            clic = Some(c);
                        }
                    }
                }
            },
        );
        widgets::desvanecer_scroll(ui.painter(), &salida, OSCURO);
        if let Some((i, ctrl, shift)) = clic {
            self.clic_fila(i, ctrl, shift);
        }
        // pie: el estado (análisis, color, proxies) en una línea; los totales en la ayuda
        let pie = Rect::from_min_max(pos2(interior.left() + 8.0, interior.bottom() - alto_pie), interior.max);
        widgets::texto_corto(ui.painter(), pos2(pie.left(), pie.center().y), Align2::LEFT_CENTER, &self.estado, f_texto(11.0), self.estado_color, pie.width() - 8.0);
        ui.interact(pie, Id::unique("pie_clips"), Sense::hover()).on_hover_text(format!("{}\n{}", self.estado, self.totales));
    }

    /// Una fila densa de la lista (una sola línea): punto de estado, miniatura pequeña, nombre,
    /// "duración · tramos" y puntuación. Devuelve el clic (índice, Ctrl, Shift).
    fn fila_clip(&mut self, ui: &mut Ui, i: usize) -> Option<(usize, bool, bool)> {
        let ancho = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(ancho, ALTO_FILA), Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let c = &self.clips[i];
        let es_actual = self.actual == Some(i);
        let seleccionado = self.seleccion.contains(&c.clave);
        let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
        let sel = widgets::anim(ui, resp.id.with("s"), es_actual || seleccionado, 0.18);
        let p = ui.painter();
        let fila = rect.shrink2(vec2(0.0, 1.0));
        let fondo = mezclar(mezclar(Color32::TRANSPARENT, OSCURO_HOVER, hv), oro_a(if es_actual { 0.16 } else { 0.08 }), sel);
        p.rect_filled(fila, 8, fondo);
        let cy = fila.center().y;
        // punto de estado: oro = con tramos, marfil apagado = analizado sin tramos,
        // coral = hay que reanalizar (cambió el color), gris = pendiente
        let estado = self.estado_de(c);
        let color_punto = match &c.entrada {
            Some(_) if estado.starts_with("Reanalizar") => CORAL,
            Some(_) if !c.tramos.is_empty() => ORO,
            Some(_) => marfil_a(0.35),
            None => marfil_a(0.15),
        };
        p.circle_filled(pos2(fila.left() + 8.0, cy), 3.0, color_punto);
        // miniatura 30×18, radio 5 (degradado mientras llega)
        let mini = Rect::from_min_size(pos2(fila.left() + 16.0, cy - 9.0), vec2(30.0, 18.0));
        match self.texturas_mini.get(&c.clave) {
            Some(t) => {
                p.add(egui::epaint::RectShape::filled(mini, 5, Color32::WHITE).with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
            }
            None => {
                let pc = p.with_clip_rect(mini);
                widgets::gradiente_radial(&pc, mini, mini.center(), vec2(18.0, 12.0), &[(0.0, Color32::from_rgb(0xe3, 0xcf, 0xae)), (0.45, Color32::from_rgb(0x6e, 0x56, 0x38)), (1.0, Color32::from_rgb(0x1c, 0x17, 0x12))]);
            }
        }
        if c.entrada.is_none() {
            p.rect_filled(mini, 5, Color32::from_black_alpha(130));
        }
        // de derecha a izquierda: puntuación (ancho fijo, columnas alineadas), duración · tramos, nombre
        let punt = c.media().map(|m| format!("{:.2}", m).trim_start_matches('0').to_owned()).unwrap_or_else(|| "—".into());
        let color_p = if es_actual { ORO } else if c.tramos.is_empty() { APAGADO } else { MARFIL };
        // mientras se analiza, el porcentaje del clip en lugar de la puntuación (0 % = en cola) y una
        // barra fina al pie de la fila
        let avance = self.avance_de(c);
        let (punt, color_p) = match avance {
            Some(Some(v)) => (format!("{v}%"), if v > 0 { ORO } else { APAGADO }),
            Some(None) => ("error".to_owned(), CORAL),
            None => (punt, color_p),
        };
        if let Some(Some(v)) = avance.filter(|a| matches!(a, Some(1..=99))) {
            let pista = Rect::from_min_size(pos2(fila.left() + 16.0, fila.bottom() - 3.0), vec2(fila.width() - 24.0, 2.0));
            p.rect_filled(pista, 1, marfil_a(0.08));
            p.rect_filled(Rect::from_min_size(pista.min, vec2(pista.width() * v as f32 / 100.0, 2.0)), 1, ORO);
        }
        let x_punt = fila.right() - 8.0;
        p.text(pos2(x_punt, cy), Align2::RIGHT_CENTER, punt, f_negrita(12.5), color_p);
        let detalle = match &c.entrada {
            Some(_) => format!("{} · {}", fmt_corto(c.dur()), c.tramos.len()),
            None if c.dur() > 0.0 => fmt_corto(c.dur()),
            None => "—".into(),
        };
        let x_det = x_punt - 34.0;
        let r_det = p.text(pos2(x_det, cy), Align2::RIGHT_CENTER, detalle, f_texto(11.0), APAGADO);
        let x = mini.right() + 8.0;
        let color_nombre = if c.entrada.is_some() { MARFIL } else { APAGADO };
        widgets::texto_corto(p, pos2(x, cy), Align2::LEFT_CENTER, &c.base(), if es_actual { f_negrita(12.5) } else { f_medio(12.5) }, color_nombre, r_det.left() - 8.0 - x);
        // marca de selección múltiple: borde dorado fino
        if seleccionado && !es_actual {
            p.rect_stroke(fila, 8, Stroke::new(1.0, oro_a(0.45)), StrokeKind::Inside);
        }
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_ui(|ui| {
            let c = &self.clips[i];
            let m = &c.meta;
            ui.label(egui::RichText::new(c.ruta.to_string_lossy()).font(f_texto(12.0)));
            let linea = [
                m.codec.clone().unwrap_or_default(),
                m.container.clone().unwrap_or_default(),
                m.res.clone().unwrap_or_default(),
                m.fps.map(|f| format!("{f:.2} fps")).unwrap_or_default(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            ui.label(egui::RichText::new(linea).font(f_texto(12.0)).color(ARENA));
            ui.label(egui::RichText::new(self.estado_de(c)).font(f_texto(12.0)).color(ARENA));
            if c.entrada.is_some() {
                ui.label(
                    egui::RichText::new(format!(
                        "{} tramos · puntuación {:.0}% · con joya {:.0}% · aprobado {}",
                        c.tramos.len(),
                        100.0 * c.media().unwrap_or(0.0),
                        100.0 * c.parte_joya(),
                        fmt_tiempo(c.aprobado)
                    ))
                    .font(f_texto(12.0))
                    .color(ARENA),
                );
            }
            ui.label(egui::RichText::new("Ctrl+clic o Mayús+clic para elegir varios").font(f_texto(11.0)).color(APAGADO));
        });
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            return Some((i, m.command, m.shift));
        }
        None
    }

    // =============================================================================================
    // Centro: visor y transporte
    // =============================================================================================

    fn centro(&mut self, ui: &mut Ui, rect: Rect) {
        // barra de transporte delgada: el espacio es para el video
        let alto_transporte = 44.0;
        let visor = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - alto_transporte - HUECO));
        let transporte = Rect::from_min_max(pos2(rect.left(), rect.bottom() - alto_transporte), rect.max);
        self.visor(ui, visor);
        self.transporte(ui, transporte);
    }

    fn visor(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        // marco: borde de 6 px del color de las tarjetas, radio 34
        p.add(sombra_suave().as_shape(rect, CornerRadius::same(34)));
        p.rect_filled(rect, 34, OSCURO);
        let dentro = rect.shrink(6.0);
        let pc = p.with_clip_rect(dentro);
        let actual = self.actual;
        let mut cuadro_rect = None;
        match (&self.tex_visor, actual) {
            (Some(tex), Some(_)) => {
                // fondo negro y el cuadro ajustado (letterbox), con esquinas redondeadas
                pc.rect_filled(dentro, 28, Color32::from_rgb(0x0e, 0x0c, 0x0a));
                let [w, h] = self.tam_cuadro;
                let k = (dentro.width() / w.max(1) as f32).min(dentro.height() / h.max(1) as f32);
                let r = Rect::from_center_size(dentro.center(), vec2(w as f32 * k, h as f32 * k));
                let radio = if (r.width() - dentro.width()).abs() < 2.0 || (r.height() - dentro.height()).abs() < 2.0 { 28 } else { 8 };
                pc.add(egui::epaint::RectShape::filled(r, radio, Color32::WHITE).with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
                cuadro_rect = Some(r);
            }
            _ => {
                // marcador del diseño: degradado cálido radial
                widgets::gradiente_radial(
                    &pc,
                    dentro,
                    pos2(dentro.center().x, dentro.top() + dentro.height() * 0.46),
                    vec2(dentro.width().min(880.0) * 0.5, dentro.height().min(600.0) * 0.5),
                    &[(0.0, Color32::from_rgb(0xf0, 0xdc, 0xb6)), (0.38, Color32::from_rgb(0x93, 0x72, 0x3f)), (0.7, Color32::from_rgb(0x2a, 0x21, 0x16)), (1.0, Color32::from_rgb(0x0e, 0x0c, 0x0a))],
                );
                // la búsqueda no dio cuadro (sin conexión y sin proxy, error de ffmpeg…): se dice, en vez
                // de dejar la imagen de otro clip
                let texto = if self.clips.is_empty() {
                    "Agrega material para empezar"
                } else if actual.is_some() && self.sin_cuadro {
                    "No se puede mostrar este cuadro"
                } else {
                    "Selecciona un clip"
                };
                let g = p.layout_no_wrap(texto.into(), f_titulo(26.0), TINTA);
                let caja = Rect::from_center_size(dentro.center(), g.size() + vec2(40.0, 20.0));
                pc.rect_filled(caja, 255, marfil_a(0.85));
                pc.galley(caja.center() - g.size() / 2.0, g, TINTA);
            }
        }
        // cajas de joya
        if let (Some(r), Some(i), true) = (cuadro_rect, actual, self.ver_joyas) {
            if let Some(e) = &self.clips[i].entrada {
                if let Some(m) = e.muestra_en(self.t) {
                    let k = r.width() / e.ancho_analisis().max(1.0);
                    for j in &m.joyas {
                        let b = Rect::from_min_max(r.min + vec2(j.caja[0] * k, j.caja[1] * k), r.min + vec2(j.caja[2] * k, j.caja[3] * k));
                        let radio = (b.width().min(b.height()) * 0.18).clamp(4.0, 22.0);
                        // cada caja con SU etiqueta y SU confianza (las de esta detección en este cuadro);
                        // el tipo del clip va solo en la píldora de información
                        if j.principal {
                            pc.rect_stroke(b, radio, Stroke::new(3.0, MARFIL), StrokeKind::Middle);
                            let etiqueta = format!("{} {:.2}", j.propia, j.conf);
                            let g = p.layout_no_wrap(etiqueta, f_negrita(13.0), TINTA);
                            let pill = Rect::from_min_size(pos2(b.left(), (b.top() - 34.0).max(dentro.top() + 6.0)), g.size() + vec2(28.0, 12.0));
                            pc.rect_filled(pill, 255, MARFIL);
                            pc.galley(pill.center() - g.size() / 2.0, g, TINTA);
                        } else {
                            pc.rect_stroke(b, radio, Stroke::new(1.5, marfil_a(0.45)), StrokeKind::Middle);
                            pc.text(b.left_top() + vec2(4.0, -4.0), Align2::LEFT_BOTTOM, format!("{} {:.2}", j.propia, j.conf), f_medio(11.5), marfil_a(0.7));
                        }
                    }
                }
            }
        }
        // píldoras superiores: aprobado y datos de la muestra
        if let Some(i) = actual {
            let c = &self.clips[i];
            let dentro_tramo = c.tramos.iter().any(|s| s.inicio <= self.t && self.t < s.fin);
            let a = widgets::anim(ui, Id::unique("aprobado_pill"), dentro_tramo, 0.2);
            if a > 0.01 {
                let g = p.layout_no_wrap("APROBADO".into(), f_negrita(12.0), TINTA);
                let pill = Rect::from_min_size(dentro.min + vec2(18.0, 18.0), g.size() + vec2(40.0, 14.0));
                let mut pa = pc.clone();
                pa.multiply_opacity(a);
                pa.rect_filled(pill, 255, ORO);
                pa.circle_filled(pos2(pill.left() + 15.0, pill.center().y), 4.0, TINTA);
                pa.galley(pos2(pill.left() + 26.0, pill.center().y - g.size().y / 2.0), g, TINTA);
            }
            let info = match c.entrada.as_ref().and_then(|e| e.muestra_en(self.t)) {
                Some(m) => {
                    let mut nombres: Vec<&str> = m.joyas.iter().map(|j| j.etiqueta.as_str()).collect();
                    nombres.sort();
                    nombres.dedup();
                    let joyas = if nombres.is_empty() { "sin joya".to_owned() } else { nombres.join(", ") };
                    format!("{} · {} · nitidez {:.0}{}", c.nombre(), joyas, m.lap_p99, if c.entrada.as_ref().map(|e| e.es_log()).unwrap_or(false) { " · LOG" } else { "" })
                }
                None => format!("{} · pendiente de análisis", c.nombre()),
            };
            // ancho máximo: lo que deja la píldora "APROBADO" a la izquierda
            let g = widgets::galeria_corta(&p, &info, f_medio(12.0), MARFIL, (dentro.width() - 190.0).max(60.0));
            let pill = Rect::from_min_size(pos2(dentro.right() - 18.0 - g.size().x - 28.0, dentro.top() + 18.0), g.size() + vec2(28.0, 14.0));
            pc.rect_filled(pill, 255, Color32::from_black_alpha(150));
            pc.galley(pill.center() - g.size() / 2.0, g, MARFIL);
        }
        // controles: ⏮ −2s +2s Tramo ▶ · joyas · color. Si queda franja libre bajo el cuadro (clips
        // horizontales) van ahí, siempre visibles; si no (clips verticales), solo aparecen al pasar el
        // ratón por el visor, sobre un fondo oscuro para que se lean sobre cualquier imagen.
        let hover_visor = ui.rect_contains_pointer(dentro);
        let av = widgets::anim(ui, Id::unique("visor_hover"), hover_visor, 0.25);
        let libre_abajo = cuadro_rect.map(|r| dentro.bottom() - r.bottom()).unwrap_or(0.0);
        let debajo = libre_abajo >= 56.0;
        let cy = match cuadro_rect {
            Some(r) if debajo => (r.bottom() + dentro.bottom()) / 2.0,
            _ => dentro.bottom() - 34.0,
        };
        let barra = Rect::from_center_size(pos2(dentro.center().x, cy), vec2(350.0, 46.0));
        let opacidad = if debajo { 1.0 } else { av };
        // con el visor muy bajo o estrecho los controles taparían el cuadro: se ocultan (siguen el
        // teclado y el transporte)
        if actual.is_some() && opacidad > 0.01 && dentro.height() >= 260.0 && dentro.width() >= 380.0 {
            let mut h = ui.new_child(UiBuilder::new().id_salt("ctl_visor").max_rect(barra.shrink2(vec2(10.0, 4.0))).layout(Layout::left_to_right(Align::Center)));
            h.multiply_opacity(opacidad);
            h.painter().rect_filled(barra, 255, Color32::from_black_alpha(215));
            h.spacing_mut().item_spacing.x = 4.0;
            let blanco = MARFIL;
            if widgets::boton_icono(&mut h, Icono::Inicio, 34.0, None, blanco, "Al inicio (Inicio)").clicked() { self.buscar(0.0); }
            if widgets::boton_icono(&mut h, Icono::Atras, 34.0, None, blanco, "−2 s (←)").clicked() { self.buscar(self.t - 2.0); }
            if widgets::boton_icono(&mut h, Icono::Adelante, 34.0, None, blanco, "+2 s (→)").clicked() { self.buscar(self.t + 2.0); }
            if widgets::boton_icono(&mut h, Icono::SiguienteTramo, 34.0, None, blanco, "Siguiente tramo aprobado (N)").clicked() { self.siguiente_tramo(); }
            h.add_space(6.0);
            let mut vj = self.ver_joyas;
            if widgets::interruptor(&mut h, &mut vj, "Joyas", false).changed() {
                self.ver_joyas = vj;
            }
            let mut vc = self.ver_color;
            if widgets::interruptor(&mut h, &mut vc, "Color", false).changed() {
                self.ver_color = vc;
                self.minis_y_cuadro();
            }
        }
        // clic en el visor: reproducir / pausar
        let resp = ui.interact(dentro.shrink(60.0), Id::unique("visor_clic"), Sense::click());
        if resp.clicked() && !barra.contains(resp.interact_pointer_pos().unwrap_or(Pos2::ZERO)) {
            self.alternar_reproduccion();
        }
    }

    fn transporte(&mut self, ui: &mut Ui, rect: Rect) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Marfil, 1.0);
        let interior = rect.shrink2(vec2(6.0, 5.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("transporte").max_rect(interior).layout(Layout::left_to_right(Align::Center)));
        h.spacing_mut().item_spacing.x = 10.0;
        let icono = if self.reproduciendo { Icono::Pausa } else { Icono::Reproducir };
        if widgets::boton_icono(&mut h, icono, 34.0, Some(TINTA), ORO, "Reproducir / pausa (Espacio)").clicked() {
            self.alternar_reproduccion();
        }
        let (dur, tramos) = match self.actual.and_then(|i| self.clips.get(i)) {
            Some(c) => (c.dur(), c.tramos.iter().map(|s| (s.inicio, s.fin)).collect::<Vec<_>>()),
            None => (0.0, Vec::new()),
        };
        let cuadro_s = self.duracion_cuadro();
        h.label(egui::RichText::new(fmt_corto(self.t)).font(f_negrita(13.0)).color(TINTA));
        // a la derecha: duración y la píldora de color
        // la conversión que de verdad aplica el visor (con "auto", el perfil de la cámara si se conoce)
        let color_txt = self.texto_color_actual("Automático");
        // en pantallas estrechas la píldora de color se abrevia para que la barra conserve su ancho
        let color_txt = if interior.width() < 520.0 { color_txt.split_whitespace().next().unwrap_or("").to_owned() } else { color_txt };
        let g_col = h.painter().layout_no_wrap(color_txt.clone(), f_texto(12.0), MARFIL);
        let g_dur = h.painter().layout_no_wrap(fmt_corto(dur), f_texto(13.0), TINTA_APAGADA);
        let ancho_barra = (h.available_width() - g_col.size().x - 28.0 - g_dur.size().x - 2.0 * 10.0 - 4.0).max(40.0);
        if let Some(t) = widgets::barra_avance(&mut h, ancho_barra, self.t, dur, &tramos, cuadro_s) {
            self.buscar(t);
        }
        h.label(egui::RichText::new(fmt_corto(dur)).font(f_texto(13.0)).color(TINTA_APAGADA));
        let r = widgets::pildora(&mut h, &color_txt, None, Estilo::Tinta, 26.0, true).on_hover_text("Ver con color (abre el panel Color con Ctrl+3)");
        if r.clicked() {
            self.paneles.color = !self.paneles.color;
            self.marcar_sucio();
        }
    }

    // =============================================================================================
    // Columna derecha: Pesos
    // =============================================================================================

    fn columna_pesos(&mut self, ui: &mut Ui, rect: Rect) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Oscura, 1.0);
        let interior = rect.shrink(22.0);
        // con poco alto, el bloque de exportar se compacta (sin la línea de resumen)
        let compacto = rect.height() < 400.0;
        let alto_exportar = if compacto { 70.0 } else { 116.0 };
        let arriba = Rect::from_min_max(interior.min, pos2(interior.right(), interior.bottom() - alto_exportar));
        let mut h = ui.new_child(UiBuilder::new().id_salt("pesos_in").max_rect(arriba).layout(Layout::top_down(Align::Min)));
        h.spacing_mut().item_spacing.y = 14.0;
        let nombre_crit = motor::CRITERIOS.iter().find(|c| c.0 == self.criterio.nombre).map(|c| c.1).unwrap_or("");
        h.horizontal(|ui| {
            ui.label(egui::RichText::new("Pesos").font(f_titulo(26.0)).color(MARFIL));
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("Criterio: {}", nombre_crit.split(' ').next().unwrap_or(""))).font(f_texto(13.0)).color(APAGADO));
            });
        });
        let opciones = ["Equilibrado", "Redes", "Permisivo"];
        let sel = motor::CRITERIOS.iter().position(|c| c.0 == self.criterio.nombre).unwrap_or(0);
        if let Some(i) = widgets::segmentado(&mut h, Id::unique("criterio"), &opciones, sel) {
            self.aplicar_criterio(motor::CRITERIOS[i].0);
        }
        let mut cambio = false;
        // todo lo de debajo del criterio se desplaza dentro de la tarjeta (nada se corta)
        let salida = egui::ScrollArea::vertical().id_salt("pesos_scroll").auto_shrink([false, false]).show(&mut h, |ui| {
            ui.spacing_mut().item_spacing.y = 18.0;
            ui.set_width(ui.available_width() - 4.0);
            ui.label(egui::RichText::new("Cuánto importa cada cosa. La nitidez manda: en 0 se ignora.").font(f_texto(12.5)).color(APAGADO));
            for (n, (k, nombre)) in motor::PESOS.iter().enumerate() {
                let (icono, color) = ESTILO_PESOS[n];
                let mut v = self.criterio.pesos.get(*k).copied().unwrap_or(0.0);
                let max = if *k == "sharp" { 1.5 } else { 1.0 };
                if widgets::fila_peso(ui, icono, color, nombre, &mut v, max, &|x| format!("{x:.2}")) {
                    self.criterio.pesos.insert(k.to_string(), v);
                    cambio = true;
                }
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Tramos").font(f_titulo(21.0)).color(MARFIL));
            let c = &mut self.criterio;
            let dos = |x: f64| format!("{x:.2}");
            let seg2 = |x: f64| format!("{x:.2} s");
            let seg0 = |x: f64| format!("{x:.0} s");
            let entero = |x: f64| format!("{x:.0}");
            cambio |= widgets::fila_ajuste(ui, "Puntuación mínima", &mut c.umbral, 0.1, 0.95, 0.01, &dos);
            cambio |= widgets::fila_ajuste(ui, "Duración mínima del tramo", &mut c.min_s, 0.25, 10.0, 0.25, &seg2);
            cambio |= widgets::fila_ajuste(ui, "Duración máxima del tramo", &mut c.max_s, 1.0, 120.0, 1.0, &seg0);
            let mut por = c.por_clip as f64;
            if widgets::fila_ajuste(ui, "Tramos máximos por clip", &mut por, 1.0, 99.0, 1.0, &entero) {
                c.por_clip = por.round() as u32;
                cambio = true;
            }
            cambio |= widgets::fila_ajuste(ui, "Tolerancia a caídas", &mut c.tolerancia, 0.0, 4.0, 0.25, &seg2);
            cambio |= widgets::fila_ajuste(ui, "Consistencia (suavizado)", &mut c.suavizado, 0.0, 4.0, 0.25, &seg2);
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Joyas").font(f_titulo(21.0)).color(MARFIL));
            ui.label(egui::RichText::new("Ninguna marcada = todas").font(f_texto(12.0)).color(APAGADO));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for j in motor::JOYAS {
                    let activa = self.criterio.joyas.iter().any(|x| x == j);
                    if widgets::ficha(ui, j, activa).clicked() {
                        if activa { self.criterio.joyas.retain(|x| x != j); } else { self.criterio.joyas.push(j.to_owned()); }
                        cambio = true;
                    }
                }
            });
            ui.label(egui::RichText::new("Planos").font(f_titulo(21.0)).color(MARFIL));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (k, nombre) in motor::PLANOS {
                    let activa = self.criterio.planos.iter().any(|x| x == k);
                    if widgets::ficha(ui, nombre, activa).clicked() {
                        if activa { self.criterio.planos.retain(|x| x != k); } else { self.criterio.planos.push(k.to_owned()); }
                        cambio = true;
                    }
                }
            });
            ui.label(egui::RichText::new("Exportar a DaVinci").font(f_titulo(21.0)).color(MARFIL));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("fps de tu línea de tiempo").font(f_texto(13.0)).color(ARENA));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let opciones: Vec<String> = crate::app::FPS.iter().map(|s| s.to_string()).collect();
                    if let Some(i) = widgets::desplegable(ui, Id::unique("dd_fps"), &self.fps.clone(), &opciones, 96.0, false) {
                        self.fps = opciones[i].clone();
                        self.marcar_sucio();
                    }
                });
            });
            for (k, nombre) in motor::MODOS_EXPORTAR {
                let activo = self.modo == k;
                if widgets::item_menu(ui, nombre, activo).clicked() {
                    self.modo = k.into();
                    self.marcar_sucio();
                }
            }
            ui.horizontal(|ui| {
                if widgets::pildora(ui, "Restablecer criterio", None, Estilo::Suave, 30.0, true).clicked() {
                    let n = self.criterio.nombre.clone();
                    self.aplicar_criterio(&n);
                }
                if widgets::pildora(ui, "Abrir resultados", Some(Icono::Carpeta), Estilo::Suave, 30.0, !self.out.is_empty()).clicked() {
                    self.abrir_carpeta_resultados();
                }
            });
            ui.add_space(8.0);
        });
        widgets::desvanecer_scroll(ui.painter(), &salida, OSCURO);
        if cambio {
            self.programar();
        }
        // abajo: resumen y la píldora "Exportar ›››››"
        let abajo = Rect::from_min_max(pos2(interior.left(), interior.bottom() - alto_exportar + 12.0), interior.max);
        let p = ui.painter();
        if !compacto {
            widgets::texto_corto(p, pos2(abajo.left() + 4.0, abajo.top()), Align2::LEFT_TOP, &self.resumen, f_texto(12.5), ARENA, abajo.width() - 8.0);
        }
        let pill = Rect::from_min_max(pos2(abajo.left(), abajo.bottom() - if compacto { 54.0 } else { 66.0 }), abajo.max);
        let resp = ui.interact(pill, Id::unique("exportar"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("Exportar la línea de tiempo (OTIO + CSV) · Ctrl+E\n{}", self.resumen));
        let hv = widgets::anim(ui, resp.id.with("h"), resp.hovered(), 0.18);
        let pul = widgets::anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
        let pill_r = pill.shrink(pul * 1.5);
        p.rect_filled(pill_r, 255, mezclar(OSCURO_3, OSCURO_HOVER, hv));
        let g = p.layout_no_wrap("Exportar".into(), f_negrita(15.0), TINTA);
        let boton = Rect::from_min_size(pill_r.min + vec2(8.0, 8.0), vec2(g.size().x + 44.0, pill_r.height() - 16.0));
        p.rect_filled(boton, 255, mezclar(MARFIL, Color32::WHITE, 0.3 * hv));
        p.galley(boton.center() - g.size() / 2.0, g, TINTA);
        // flechas que avanzan un poco con el hover
        let n = 5;
        for k in 0..n {
            let x = pill_r.right() - 26.0 - (n - 1 - k) as f32 * 14.0 + 6.0 * hv;
            let c = pos2(x, pill_r.center().y);
            let alpha = 0.45 + 0.55 * (k as f32 / (n - 1) as f32) * (0.5 + 0.5 * hv) + 0.25 * hv;
            p.line(vec![c + vec2(-3.5, -6.0), c + vec2(2.5, 0.0), c + vec2(-3.5, 6.0)], Stroke::new(2.0, oro_a(alpha.min(1.0))));
        }
        if resp.clicked() {
            self.exportar();
        }
    }

    // =============================================================================================
    // Línea de tiempo (mapa de todo el material)
    // =============================================================================================

    fn linea_de_tiempo(&mut self, ui: &mut Ui, rect: Rect, max_alto: f32) {
        widgets::tarjeta(ui.painter(), rect, Tarjeta::Marfil, 1.0);
        let p = ui.painter().clone();
        // asa de arriba: arrastrar cambia el alto
        let asa = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 16.0));
        let r_asa = ui.interact(asa, Id::unique("asa_tiempo"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        if r_asa.dragged() {
            self.alto_tiempo = (self.alto_tiempo - r_asa.drag_delta().y).clamp(120.0, max_alto);
        }
        if r_asa.drag_stopped() {
            self.marcar_sucio();
        }
        let ha = widgets::anim(ui, r_asa.id.with("h"), r_asa.hovered() || r_asa.dragged(), 0.15);
        let pastilla = Rect::from_center_size(pos2(rect.center().x, rect.top() + 11.0), vec2(72.0 + 16.0 * ha, 6.0));
        p.rect_filled(pastilla, 3, tinta_a(0.35 + 0.3 * ha));
        // cabecera
        let y_cab = rect.top() + 22.0;
        let x0 = rect.left() + 22.0;
        let x1 = rect.right() - 22.0;
        let r_titulo = p.text(pos2(x0, y_cab + 14.0), Align2::LEFT_CENTER, "Línea de tiempo", f_titulo(22.0), TINTA);
        let modo_txt = match self.modo.as_str() { "v1v2" => "V2 aprobados · V1 resto", "aprobados" => "Solo aprobados", _ => "V1 con marcadores" };
        let mut h = ui.new_child(UiBuilder::new().id_salt("cab_tiempo").max_rect(Rect::from_min_max(pos2(x0, y_cab), pos2(x1, y_cab + 30.0))).layout(Layout::right_to_left(Align::Center)));
        h.spacing_mut().item_spacing.x = 8.0;
        // la píldora del modo cambia de modo de exportación con cada clic
        if widgets::pildora(&mut h, modo_txt, None, Estilo::Tinta, 28.0, true).on_hover_text("Modo de exportación (clic para cambiar)").clicked() {
            let i = motor::MODOS_EXPORTAR.iter().position(|m| m.0 == self.modo).unwrap_or(0);
            self.modo = motor::MODOS_EXPORTAR[(i + 1) % motor::MODOS_EXPORTAR.len()].0.to_owned();
            self.marcar_sucio();
        }
        if widgets::pildora(&mut h, "Clip", Some(Icono::Lupa), Estilo::Contorno, 28.0, self.actual.is_some()).on_hover_text("Acercar al clip actual").clicked() {
            self.enfocar_clip();
        }
        if widgets::pildora(&mut h, "Ajustar", Some(Icono::Ajustar), Estilo::Contorno, 28.0, true).on_hover_text("Ver todo el material").clicked() {
            self.ajustar_mapa();
        }
        if self.zoom_mapa > 1.01 {
            h.label(egui::RichText::new(format!("×{:.1}", self.zoom_mapa)).font(f_medio(13.0)).color(TINTA_APAGADA));
        }
        // la ayuda ocupa lo que queda entre el título y los botones (con elipsis; nunca debajo de ellos)
        let hueco_ayuda = h.min_rect().left() - 14.0 - (r_titulo.right() + 12.0);
        if hueco_ayuda > 60.0 {
            widgets::texto_corto(
                &p,
                pos2(r_titulo.right() + 12.0, y_cab + 15.0),
                Align2::LEFT_CENTER,
                "arrastra el borde para subirla o bajarla · rueda = zoom · Mayús+rueda = desplazar",
                f_texto(13.0),
                TINTA_APAGADA,
                hueco_ayuda,
            );
        }
        // área oscura con las pistas
        let area = Rect::from_min_max(pos2(x0, y_cab + 36.0), pos2(x1, rect.bottom() - 16.0));
        if area.height() < 30.0 {
            return;
        }
        p.rect_filled(area, 20, TINTA);
        self.pistas(ui, area);
    }

    /// Dibuja las pistas del material dentro del área oscura e interpreta clic, rueda y Mayús+rueda.
    fn pistas(&mut self, ui: &mut Ui, area: Rect) {
        let p = ui.painter().with_clip_rect(area.shrink(1.0));
        let interior = area.shrink2(vec2(14.0, 8.0));
        let ancho = interior.width().max(1.0);
        if self.clips.is_empty() {
            p.text(area.center(), Align2::CENTER_CENTER, "La línea de tiempo aparece al cargar material", f_texto(13.0), marfil_a(0.5));
            return;
        }
        // interacción (antes de dibujar, para que el zoom se vea en este mismo cuadro)
        let resp = ui.interact(area, Id::unique("pistas"), Sense::click_and_drag());
        let (offs, total, mut t0, mut t1) = self.ventana_mapa();
        if resp.hovered() {
            // eventos de rueda en bruto: Mayús+rueda llega en x o en y según el sistema
            let (delta, shift, puntero) = ui.input(|i| {
                let mut d = 0.0f32;
                for e in &i.raw.events {
                    if let egui::Event::MouseWheel { delta, .. } = e {
                        d += if delta.y != 0.0 { delta.y } else { delta.x };
                    }
                }
                (d, i.modifiers.shift, i.pointer.hover_pos())
            });
            if delta != 0.0 {
                if shift {
                    // Mayús+rueda: desplazar
                    self.t0_mapa -= (t1 - t0) * 0.1 * delta.signum() as f64;
                } else if let Some(pp) = puntero {
                    // rueda: zoom alrededor del puntero
                    let fx = ((pp.x - interior.left()) / ancho).clamp(0.0, 1.0) as f64;
                    let ancla = t0 + fx * (t1 - t0);
                    self.zoom_mapa = (self.zoom_mapa * if delta > 0.0 { 1.25 } else { 0.8 }).clamp(1.0, 200.0);
                    let span = total / self.zoom_mapa;
                    self.t0_mapa = ancla - fx * span;
                }
                self.marcar_sucio();
                let v = self.ventana_mapa();
                t0 = v.2;
                t1 = v.3;
            }
        }
        let span = (t1 - t0).max(1e-6);
        let x_de = |gt: f64| interior.left() + ((gt - t0) / span) as f32 * ancho;
        let t_de = |x: f32| t0 + ((x - interior.left()) / ancho).clamp(0.0, 1.0) as f64 * span;

        // ---- carriles (de arriba abajo) ----
        let mut y = interior.top();
        // el minimapa solo cuando hay alto de sobra (si no, las pistas quedarían finas). Con el alto de
        // siempre (200) el interior mide 110: antes pedía más de 120 y el botón del mapa no hacía nada
        let mapa = if self.paneles.mapa && interior.height() > 90.0 {
            let r = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, 10.0));
            y += 16.0;
            Some(r)
        } else {
            None
        };
        let regla = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, 14.0));
        y += 16.0;
        let alto_disp = interior.bottom() - y;
        let lanes_joya = if alto_disp > 170.0 { 3 } else { 0 };
        let alto_clips = 18.0;
        let alto_calor = 4.0;
        let alto_joya = 3.0;
        let alto_tipos = lanes_joya as f32 * 11.0;
        let principal_h = (alto_disp - alto_clips - alto_calor - alto_joya - alto_tipos - 3.0 * 4.0).max(16.0);
        let carril_clips = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, alto_clips));
        y += alto_clips + 4.0;
        let principal = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, principal_h));
        y += principal_h + 4.0;
        let calor = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, alto_calor));
        y += alto_calor + 4.0;
        let carril_joya = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, alto_joya));
        y += alto_joya + 4.0;
        let tipos_r = Rect::from_min_size(pos2(interior.left(), y), vec2(ancho, alto_tipos));

        // ---- minimapa de todo el material, con la ventana visible ----
        if let Some(m) = mapa {
            p.rect_filled(m, 5, marfil_a(0.12));
            let xm = |gt: f64| m.left() + (gt / total) as f32 * m.width();
            for (i, c) in self.clips.iter().enumerate() {
                for s in &c.tramos {
                    let r = Rect::from_min_max(pos2(xm(offs[i] + s.inicio), m.top() + 2.0), pos2(xm(offs[i] + s.fin).max(xm(offs[i] + s.inicio) + 1.0), m.bottom() - 2.0));
                    p.rect_filled(r, 2, oro_a(0.8));
                }
            }
            let win = Rect::from_min_max(pos2(xm(t0), m.top() - 1.0), pos2(xm(t1).max(xm(t0) + 4.0), m.bottom() + 1.0));
            p.rect_stroke(win, 4, Stroke::new(1.5, MARFIL), StrokeKind::Middle);
            // arrastrar sobre el minimapa mueve la ventana
            let rm = ui.interact(m.expand2(vec2(0.0, 3.0)), Id::unique("minimapa"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::Grab);
            if let Some(pp) = rm.interact_pointer_pos() {
                if rm.dragged() || rm.clicked() {
                    let centro = ((pp.x - m.left()) / m.width()).clamp(0.0, 1.0) as f64 * total;
                    self.t0_mapa = centro - span / 2.0;
                    self.marcar_sucio();
                }
            }
        }

        // ---- regla con código de tiempo, una marca cada ~120 px en pasos redondos ----
        let pasos = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0];
        let paso = pasos.iter().copied().find(|s| s * ancho as f64 / span >= 120.0).unwrap_or(3600.0);
        let cad = self.cadencia();
        let mut t = (t0 / paso).floor() * paso;
        while t <= t1 {
            let x = x_de(t);
            if x >= interior.left() - 1.0 {
                p.line_segment([pos2(x, regla.bottom() - 4.0), pos2(x, regla.bottom())], Stroke::new(1.0, marfil_a(0.35)));
                p.text(pos2(x + 3.0, regla.top()), Align2::LEFT_TOP, codigo_tiempo(t, cad), f_texto(10.5), marfil_a(0.45));
            }
            t += paso;
        }

        // ---- clips, tramos, curva, calor y presencia de joya ----
        let umbral = self.criterio.umbral;
        let y_de = |v: f64| principal.bottom() - 6.0 - (principal.height() - 12.0) * v.clamp(0.0, 1.0) as f32;
        for (i, c) in self.clips.iter().enumerate() {
            let (a, b) = (offs[i], offs[i] + c.dur());
            if b < t0 || a > t1 {
                continue;
            }
            let (xa, xb) = (x_de(a), x_de(b));
            let es_actual = self.actual == Some(i);
            let bloque = Rect::from_min_max(pos2(xa + 1.0, carril_clips.top()), pos2((xb - 1.0).max(xa + 2.0), carril_clips.bottom()));
            let fondo = if es_actual { oro_a(0.32) } else if i % 2 == 0 { marfil_a(0.10) } else { marfil_a(0.07) };
            p.rect_filled(bloque, 6, fondo);
            if es_actual {
                p.rect_stroke(bloque, 6, Stroke::new(1.5, ORO), StrokeKind::Inside);
            }
            if xb - xa > 46.0 {
                let pc = p.with_clip_rect(bloque.intersect(area));
                pc.text(pos2(xa.max(interior.left()) + 6.0, bloque.center().y), Align2::LEFT_CENTER, c.base(), f_medio(11.0), if c.entrada.is_some() { MARFIL } else { marfil_a(0.45) });
            }
            // tramos aprobados: píldoras doradas (V2)
            for s in &c.tramos {
                let (sa, sb) = (x_de(a + s.inicio), x_de(a + s.fin));
                let r = Rect::from_min_max(pos2(sa, principal.top() + 6.0), pos2(sb.max(sa + 3.0), principal.bottom() - 6.0));
                let radio = (r.width() / 2.0).min(14.0);
                p.rect_filled(r, radio, if es_actual { ORO } else { oro_a(0.82) });
            }
            // separación entre clips
            p.line_segment([pos2(xa, principal.top()), pos2(xa, principal.bottom())], Stroke::new(1.0, marfil_a(0.08)));
        }
        // umbral (línea discontinua)
        let yu = y_de(umbral);
        let mut x = interior.left();
        while x < interior.right() {
            p.line_segment([pos2(x, yu), pos2((x + 5.0).min(interior.right()), yu)], Stroke::new(1.0, marfil_a(0.28)));
            x += 9.0;
        }
        p.text(pos2(interior.right() - 4.0, yu - 3.0), Align2::RIGHT_BOTTOM, format!("umbral {umbral:.2}"), f_texto(10.0), marfil_a(0.45));
        // curva de puntuación, calor y joya: muestreadas por columna de píxeles (como el mapa de Python)
        let paso_px = 2.0;
        let n_col = (ancho / paso_px).ceil() as usize;
        let mut tramo_curva: Vec<Pos2> = Vec::new();
        let mut clip_prev = usize::MAX;
        let trazo = Stroke::new(1.6, MARFIL);
        for k in 0..n_col {
            let xp = interior.left() + k as f32 * paso_px;
            let gt = t_de(xp + paso_px / 2.0);
            let i = offs.partition_point(|&o| o <= gt).saturating_sub(1).min(self.clips.len() - 1);
            let c = &self.clips[i];
            let local = gt - offs[i];
            let valor = c.entrada.as_ref().and_then(|e| {
                if c.curva.is_empty() || local > e.duracion {
                    return None;
                }
                let f = local * e.por_segundo;
                let j = (f.floor() as usize).min(c.curva.len() - 1);
                let j2 = (j + 1).min(c.curva.len() - 1);
                let w = (f - j as f64).clamp(0.0, 1.0);
                let m = e.muestra_en(local).map(|m| m.joya).unwrap_or(false);
                Some((c.curva[j].1 * (1.0 - w) + c.curva[j2].1 * w, m))
            });
            if i != clip_prev || valor.is_none() {
                if tramo_curva.len() > 1 {
                    p.add(Shape::line(std::mem::take(&mut tramo_curva), trazo));
                }
                tramo_curva.clear();
                clip_prev = i;
            }
            let col = Rect::from_min_size(pos2(xp, calor.top()), vec2(paso_px + 0.5, calor.height()));
            match valor {
                Some((v, joya)) => {
                    tramo_curva.push(pos2(xp + paso_px / 2.0, y_de(v)));
                    p.rect_filled(col, 0, color_calor(v));
                    if joya {
                        p.rect_filled(Rect::from_min_size(pos2(xp, carril_joya.top()), vec2(paso_px + 0.5, carril_joya.height())), 0, marfil_a(0.4));
                    }
                }
                None => {
                    p.rect_filled(col, 0, marfil_a(0.06));
                }
            }
        }
        if tramo_curva.len() > 1 {
            p.add(Shape::line(tramo_curva, trazo));
        }
        // carriles por tipo de joya del clip actual (cuando la línea es alta)
        if lanes_joya > 0 {
            if let Some(i) = self.actual {
                if let Some(e) = &self.clips[i].entrada {
                    let mut por_tipo: Vec<(String, Vec<f64>)> = Vec::new();
                    for m in &e.muestras {
                        for j in &m.joyas {
                            match por_tipo.iter_mut().find(|x| x.0 == j.etiqueta) {
                                Some(x) => x.1.push(m.t),
                                None => por_tipo.push((j.etiqueta.clone(), vec![m.t])),
                            }
                        }
                    }
                    por_tipo.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
                    for (n, (nombre, ts)) in por_tipo.iter().take(lanes_joya).enumerate() {
                        let yl = tipos_r.top() + n as f32 * 11.0;
                        for &t in ts {
                            let xa = x_de(offs[i] + t);
                            let xb = x_de(offs[i] + t + 1.0 / e.por_segundo);
                            if xb >= interior.left() && xa <= interior.right() {
                                p.rect_filled(Rect::from_min_max(pos2(xa, yl), pos2(xb.max(xa + 1.0), yl + 7.0)), 1, marfil_a(0.32));
                            }
                        }
                        p.text(pos2(interior.left() + 2.0, yl + 3.5), Align2::LEFT_CENTER, nombre, f_texto(9.5), ORO);
                    }
                }
            }
        }
        // cabezal
        if let Some(i) = self.actual {
            let gt = offs[i] + self.t;
            if gt >= t0 && gt <= t1 {
                let x = x_de(gt);
                p.line_segment([pos2(x, regla.bottom()), pos2(x, interior.bottom())], Stroke::new(3.0, MARFIL));
                p.circle_filled(pos2(x, regla.bottom()), 5.0, MARFIL);
            }
        }
        // clic o arrastre: ir a ese momento (elige el clip que cae ahí)
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if let Some(pp) = resp.interact_pointer_pos() {
            let sobre_mapa = mapa.map(|m| m.expand2(vec2(0.0, 3.0)).contains(pp)).unwrap_or(false);
            if (resp.clicked() || resp.dragged()) && !sobre_mapa {
                self.ir_a_global(t_de(pp.x));
            }
        }
    }

    // =============================================================================================
    // Avisos
    // =============================================================================================

    fn dibujar_aviso(&mut self, ui: &mut Ui) {
        let Some(av) = &self.aviso else { return };
        let (titulo, texto, salir) = (av.titulo.clone(), av.texto.clone(), av.confirmar_salida);
        let mut cerrar = false;
        let mut salir_ya = false;
        let marco = egui::Frame::new().fill(MARFIL).corner_radius(26).inner_margin(26).shadow(sombra_marfil());
        egui::Modal::new(Id::unique("aviso")).frame(marco).backdrop_color(Color32::from_black_alpha(140)).show(ui.ctx(), |ui| {
            ui.set_max_width(460.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(egui::RichText::new(&titulo).font(f_titulo(26.0)).color(TINTA));
            ui.label(egui::RichText::new(&texto).font(f_texto(14.0)).color(TINTA));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if salir {
                    if widgets::pildora(ui, "Detener y salir", None, Estilo::Tinta, 38.0, true).clicked() {
                        salir_ya = true;
                    }
                    if widgets::pildora(ui, "Seguir", None, Estilo::Contorno, 38.0, true).clicked() {
                        cerrar = true;
                    }
                } else if widgets::pildora(ui, "Entendido", None, Estilo::Tinta, 38.0, true).clicked() {
                    cerrar = true;
                }
            });
        });
        if cerrar {
            self.aviso = None;
        }
        if salir_ya {
            self.aviso = None;
            self.al_cerrar();
            self.analisis = None;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Color de la franja de calor: rojo oscuro → ámbar → oro según la puntuación (como el mapa de Python).
fn color_calor(v: f64) -> Color32 {
    let h = v.clamp(0.0, 1.0) as f32;
    let (lo, mid, hi) = ([90.0, 30.0, 30.0], [200.0, 120.0, 30.0], [226.0, 184.0, 99.0]);
    let mezcla = |a: [f32; 3], b: [f32; 3], t: f32| Color32::from_rgb((a[0] + (b[0] - a[0]) * t) as u8, (a[1] + (b[1] - a[1]) * t) as u8, (a[2] + (b[2] - a[2]) * t) as u8);
    if h < 0.5 { mezcla(lo, mid, h / 0.5) } else { mezcla(mid, hi, (h - 0.5) / 0.5) }
}

