//! Controles propios del diseño Marfil: tarjetas con sombra, botones del riel, píldoras,
//! deslizadores punteados de peso, barra de avance del transporte, fichas y menús.
//!
//! Todos animan el hover y la pulsación con `animate_bool_with_time` (transiciones de ~150 ms),
//! y se dibujan con el `Painter` para respetar radios y colores de la maqueta.

use egui::{pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

use crate::iconos::{self, Icono};
use crate::tema::*;

/// Texto de una sola línea que se corta con "…" si no cabe en `ancho` (nunca se monta sobre otra etiqueta).
pub fn galeria_corta(p: &Painter, texto: &str, fuente: egui::FontId, color: Color32, ancho: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(texto.to_owned(), fuente, color);
    job.wrap = egui::text::TextWrapping { max_width: ancho.max(4.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

/// Pinta texto de una línea con elipsis, anclado como `Painter::text`. Devuelve su rectángulo.
pub fn texto_corto(p: &Painter, pos: Pos2, ancla: Align2, texto: &str, fuente: egui::FontId, color: Color32, ancho: f32) -> Rect {
    let g = galeria_corta(p, texto, fuente, color, ancho);
    let r = ancla.anchor_size(pos, g.size());
    p.galley(r.min, g, color);
    r
}

/// Desvanece los bordes de una zona con desplazamiento (indica que hay más contenido arriba o abajo).
/// `fondo` es el color de la tarjeta; `arriba`/`abajo` dicen qué bordes tienen contenido oculto.
pub fn desvanecer_bordes(p: &Painter, zona: Rect, fondo: Color32, arriba: bool, abajo: bool) {
    let alto = 22.0f32.min(zona.height() / 3.0);
    let franja = |y0: f32, y1: f32, c0: Color32, c1: Color32| {
        let mut m = egui::Mesh::default();
        let r = Rect::from_min_max(pos2(zona.left(), y0), pos2(zona.right(), y1));
        m.colored_vertex(r.left_top(), c0);
        m.colored_vertex(r.right_top(), c0);
        m.colored_vertex(r.left_bottom(), c1);
        m.colored_vertex(r.right_bottom(), c1);
        m.add_triangle(0, 1, 2);
        m.add_triangle(2, 1, 3);
        p.add(Shape::mesh(m));
    };
    if arriba {
        franja(zona.top(), zona.top() + alto, fondo, Color32::TRANSPARENT);
    }
    if abajo {
        franja(zona.bottom() - alto, zona.bottom(), Color32::TRANSPARENT, fondo);
    }
}

/// Fades a partir de la salida de un `ScrollArea` vertical.
pub fn desvanecer_scroll<R>(p: &Painter, salida: &egui::scroll_area::ScrollAreaOutput<R>, fondo: Color32) {
    let y = salida.state.clamped_offset().y;
    let visible = salida.inner_rect.height();
    desvanecer_bordes(p, salida.inner_rect, fondo, y > 1.0, y + visible < salida.content_size.y - 1.0);
}

/// Progreso 0..1 de una animación booleana (hover, pulsado, activo).
pub fn anim(ui: &Ui, id: Id, valor: bool, segundos: f32) -> f32 {
    ui.ctx().animate_bool_with_time(id, valor, segundos)
}

// ---------------------------------------------------------------------------------------------
// Tarjetas
// ---------------------------------------------------------------------------------------------

/// Tipos de tarjeta de la maqueta.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tarjeta {
    /// `.dk`: fondo #16140f, radio 32, brillo interior arriba.
    Oscura,
    /// `.iv`: marfil, radio 30, sombra fuerte y borde inferior más oscuro.
    Marfil,
}

/// Pinta una tarjeta con su sombra. `opacidad` permite desvanecerla al ocultar el panel.
pub fn tarjeta(p: &Painter, rect: Rect, tipo: Tarjeta, opacidad: f32) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let mut p = p.clone();
    p.multiply_opacity(opacidad);
    match tipo {
        Tarjeta::Oscura => {
            let r = CornerRadius::same(RADIO_OSCURO.min((rect.height() / 2.0) as u8));
            p.add(sombra_suave().as_shape(rect, r));
            p.rect_filled(rect, r, OSCURO);
            // `inset 0 1px 0 rgba(255,255,255,.05)`: un hilo de luz en el borde superior
            p.rect_stroke(rect.shrink(0.5), r, Stroke::new(1.0, Color32::from_white_alpha(10)), StrokeKind::Inside);
        }
        Tarjeta::Marfil => {
            let r = CornerRadius::same(RADIO_MARFIL.min((rect.height() / 2.0) as u8));
            p.add(sombra_marfil().as_shape(rect, r));
            // `inset 0 -3px 0 rgba(0,0,0,.08)`: base algo más oscura para dar volumen
            p.rect_filled(rect, r, mezclar(MARFIL, TINTA, 0.08));
            p.rect_filled(Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - 3.0)), r, MARFIL);
        }
    }
}

/// Tarjeta "recibo": marfil con muescas perforadas a los lados y línea discontinua a la altura `y_corte`.
pub fn recibo(p: &Painter, rect: Rect, y_corte: f32, opacidad: f32) {
    tarjeta(p, rect, Tarjeta::Marfil, opacidad);
    let mut p = p.clone();
    p.multiply_opacity(opacidad);
    // muescas: círculos del color del fondo que "muerden" la tarjeta
    for x in [rect.left(), rect.right()] {
        p.circle_filled(pos2(x, y_corte), 12.0, FONDO);
    }
    // línea discontinua `border-top: 2px dashed rgba(26,22,16,.25)`
    let (x0, x1) = (rect.left() + 22.0, rect.right() - 22.0);
    let mut x = x0;
    while x < x1 {
        p.line_segment([pos2(x, y_corte), pos2((x + 6.0).min(x1), y_corte)], Stroke::new(2.0, tinta_a(0.25)));
        x += 11.0;
    }
}

// ---------------------------------------------------------------------------------------------
// Riel
// ---------------------------------------------------------------------------------------------

/// Botón cuadrado del riel (52×52, radio 18). Activo: fondo marfil e icono tinta.
pub fn boton_riel(ui: &mut Ui, icono: Icono, activo: bool, ayuda: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::click());
    let resp = resp.on_hover_text(ayuda).on_hover_cursor(CursorIcon::PointingHand);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
    let a = anim(ui, resp.id.with("a"), activo, 0.2);
    let pul = anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
    let r = rect.shrink(pul * 2.0);
    let p = ui.painter();
    let fondo = mezclar(mezclar(Color32::TRANSPARENT, OSCURO_HOVER, h), MARFIL, a);
    p.rect_filled(r, 18, fondo);
    let color = mezclar(mezclar(APAGADO, MARFIL, h), TINTA, a);
    iconos::dibujar(p, icono, Rect::from_center_size(r.center(), vec2(22.0, 22.0)), color, 1.9);
    resp
}

// ---------------------------------------------------------------------------------------------
// Píldoras (botones redondeados)
// ---------------------------------------------------------------------------------------------

/// Estilos de píldora.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estilo {
    /// Fondo tinta, texto marfil (sobre superficies marfil).
    Tinta,
    /// Fondo marfil, texto tinta (sobre superficies oscuras).
    Marfil,
    /// Fondo oro, texto tinta (acción principal).
    Oro,
    /// Fondo oscuro suave, texto marfil (acciones secundarias sobre oscuro).
    Suave,
    /// Transparente con borde fino (sobre marfil).
    Contorno,
}

impl Estilo {
    fn colores(self) -> (Color32, Color32, Color32) {
        // (fondo, fondo en hover, texto)
        match self {
            Estilo::Tinta => (TINTA, mezclar(TINTA, ORO, 0.18), MARFIL),
            Estilo::Marfil => (MARFIL, mezclar(MARFIL, Color32::WHITE, 0.35), TINTA),
            Estilo::Oro => (ORO, mezclar(ORO, Color32::WHITE, 0.25), TINTA),
            Estilo::Suave => (OSCURO_3, OSCURO_HOVER, MARFIL),
            Estilo::Contorno => (Color32::TRANSPARENT, tinta_a(0.08), TINTA),
        }
    }
}

/// Píldora con texto (y opcionalmente un icono a la izquierda). `alto` en puntos.
pub fn pildora(ui: &mut Ui, texto: &str, icono: Option<Icono>, estilo: Estilo, alto: f32, habilitada: bool) -> Response {
    let fuente = f_medio(if alto >= 40.0 { 15.0 } else { 13.0 });
    let galeria = ui.painter().layout_no_wrap(texto.to_owned(), fuente, Color32::PLACEHOLDER);
    let ancho_icono = if icono.is_some() { alto * 0.42 + 6.0 } else { 0.0 };
    let pad = (alto * 0.45).max(12.0);
    let tam = vec2(galeria.size().x + ancho_icono + pad * 2.0, alto);
    let sentido = if habilitada { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(tam, sentido);
    let resp = if habilitada { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    pintar_pildora(ui, rect, &resp, galeria, icono, estilo, habilitada);
    resp
}

/// Dibuja una píldora ya ubicada (lo comparten `pildora` y los botones de ancho fijo).
pub fn pintar_pildora(ui: &Ui, rect: Rect, resp: &Response, galeria: std::sync::Arc<egui::Galley>, icono: Option<Icono>, estilo: Estilo, habilitada: bool) {
    let h = anim(ui, resp.id.with("h"), resp.hovered() && habilitada, 0.15);
    let pul = anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
    let (fondo, fondo_h, texto) = estilo.colores();
    let r = rect.shrink2(vec2(pul * 1.5, pul * 1.0));
    let p = ui.painter();
    let mut fill = mezclar(fondo, fondo_h, h);
    let mut tinta = texto;
    if !habilitada {
        fill = fill.gamma_multiply(0.45);
        tinta = tinta.gamma_multiply(0.5);
    }
    if matches!(estilo, Estilo::Oro | Estilo::Marfil) && h > 0.0 {
        // brillo dorado suave al pasar por encima
        p.add(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: oro_a(0.25 * h) }.as_shape(r, CornerRadius::same(255)));
    }
    p.rect_filled(r, CornerRadius::same(255), fill);
    if estilo == Estilo::Contorno {
        p.rect_stroke(r, CornerRadius::same(255), Stroke::new(1.2, tinta_a(0.3)), StrokeKind::Inside);
    }
    let alto = r.height();
    let ancho_icono = if icono.is_some() { alto * 0.42 + 6.0 } else { 0.0 };
    let total = galeria.size().x + ancho_icono;
    let x0 = r.center().x - total / 2.0;
    if let Some(i) = icono {
        let s = alto * 0.42;
        iconos::dibujar(p, i, Rect::from_min_size(pos2(x0, r.center().y - s / 2.0), vec2(s, s)), tinta, 2.0);
    }
    p.galley(pos2(x0 + ancho_icono, r.center().y - galeria.size().y / 2.0), galeria, tinta);
}

/// Botón redondo con icono (play, ±2 s…). `fondo` None = transparente.
pub fn boton_icono(ui: &mut Ui, icono: Icono, diametro: f32, fondo: Option<Color32>, color: Color32, ayuda: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(diametro, diametro), Sense::click());
    let resp = resp.on_hover_text(ayuda).on_hover_cursor(CursorIcon::PointingHand);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
    let pul = anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
    let p = ui.painter();
    let c = rect.center();
    let rad = diametro / 2.0 * (1.0 + 0.04 * h - 0.06 * pul);
    match fondo {
        Some(f) => {
            p.circle_filled(c, rad, mezclar(f, ORO, 0.12 * h));
        }
        None => {
            p.circle_filled(c, rad, tinta_a(0.08 * h));
        }
    }
    let s = diametro * 0.38;
    iconos::dibujar(p, icono, Rect::from_center_size(c, vec2(s, s)), color, 2.0);
    resp
}

// ---------------------------------------------------------------------------------------------
// Deslizador punteado de los pesos
// ---------------------------------------------------------------------------------------------

/// Barra gruesa punteada con relleno de color y perilla redonda dorada (`.bar` de la maqueta).
/// Devuelve la respuesta; `valor` cambia al hacer clic o arrastrar. `paso` redondea el valor.
pub fn barra_punteada(ui: &mut Ui, valor: &mut f64, min: f64, max: f64, paso: f64, color: Color32, perilla_marfil: bool) -> Response {
    let ancho = ui.available_width();
    let (rect, mut resp) = ui.allocate_exact_size(vec2(ancho, 34.0), Sense::click_and_drag());
    resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let pista = Rect::from_center_size(rect.center(), vec2(rect.width(), 26.0));
    // la perilla (34 px) viaja dentro de la pista para no salirse de la tarjeta
    let (x0, x1) = (pista.left() + 13.0, pista.right() - 13.0);
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on() {
            let f = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64;
            let mut v = min + f * (max - min);
            v = (v / paso).round() * paso;
            v = (v * 1e6).round() / 1e6;
            if (v - *valor).abs() > 1e-9 {
                *valor = v;
                resp.mark_changed();
            }
        }
    }
    let f = (((*valor - min) / (max - min)) as f32).clamp(0.0, 1.0);
    // animación suave del relleno al cambiar de criterio
    let fa = ui.ctx().animate_value_with_time(resp.id.with("v"), f, 0.12);
    let xk = x0 + fa * (x1 - x0);
    let p = ui.painter();
    p.rect_filled(pista, 13, OSCURO_2);
    // relleno hasta la perilla
    let lleno = Rect::from_min_max(pista.min, pos2(xk + 13.0, pista.max.y));
    p.rect_filled(lleno, 13, color);
    // seis puntos repartidos (`justify-content: space-around`)
    let n = 6;
    let paso_x = (pista.width() - 32.0) / n as f32;
    for i in 0..n {
        let x = pista.left() + 16.0 + paso_x * (i as f32 + 0.5);
        let sobre = x < xk;
        p.circle_filled(pos2(x, pista.center().y), 2.5, if sobre { tinta_a(0.28) } else { marfil_a(0.35) });
    }
    // perilla: 34 px, borde de 4 px del color del riel; crece un poco al pasar y al arrastrar
    let h = anim(ui, resp.id.with("h"), resp.hovered() || resp.dragged(), 0.15);
    let d = anim(ui, resp.id.with("d"), resp.dragged(), 0.1);
    let rad = 17.0 * (1.0 + 0.06 * h - 0.04 * d);
    let c = pos2(xk, pista.center().y);
    if h > 0.0 {
        p.circle_filled(c, rad + 5.0 * h, oro_a(0.18 * h));
    }
    p.circle_filled(c, rad, OSCURO_2);
    p.circle_filled(c, rad - 4.0, if perilla_marfil { MARFIL } else { ORO });
    resp
}

/// Fila completa de un peso: ficha de color con icono, nombre, valor en negrita y la barra.
pub fn fila_peso(ui: &mut Ui, icono: Icono, color: Color32, nombre: &str, valor: &mut f64, max: f64, formato: &dyn Fn(f64) -> String) -> bool {
    let mut cambio = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            ui.painter().rect_filled(r, 12, color);
            iconos::dibujar(ui.painter(), icono, Rect::from_center_size(r.center(), vec2(18.0, 18.0)), TINTA, 2.0);
            ui.add_space(4.0);
            let resto = ui.available_width();
            let valor_txt = formato(*valor);
            let g_val = ui.painter().layout_no_wrap(valor_txt, f_negrita(15.0), MARFIL);
            let w_nombre = (resto - g_val.size().x - 8.0).max(20.0);
            let (rn, _) = ui.allocate_exact_size(vec2(w_nombre, 36.0), Sense::hover());
            // una línea con elipsis: el nombre nunca se monta sobre el valor
            let g_nom = galeria_corta(ui.painter(), nombre, f_texto(14.0), MARFIL, w_nombre);
            ui.painter().galley(pos2(rn.left(), rn.center().y - g_nom.size().y / 2.0), g_nom, MARFIL);
            let (rv, _) = ui.allocate_exact_size(vec2(g_val.size().x, 36.0), Sense::hover());
            ui.painter().galley(pos2(rv.left(), rv.center().y - g_val.size().y / 2.0), g_val, MARFIL);
        });
        let r = barra_punteada(ui, valor, 0.0, max, 0.05, color, false);
        cambio = r.changed();
    });
    cambio
}

/// Deslizador compacto para los ajustes de tramos (mismo lenguaje que los pesos, más fino).
pub fn fila_ajuste(ui: &mut Ui, nombre: &str, valor: &mut f64, min: f64, max: f64, paso: f64, formato: &dyn Fn(f64) -> String) -> bool {
    let mut cambio = false;
    let texto = formato(*valor);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(nombre).font(f_texto(13.0)).color(ARENA));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(texto).font(f_negrita(13.0)).color(MARFIL));
            });
        });
        let ancho = ui.available_width();
        let (rect, mut resp) = ui.allocate_exact_size(vec2(ancho, 20.0), Sense::click_and_drag());
        resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        let (x0, x1) = (rect.left() + 9.0, rect.right() - 9.0);
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.is_pointer_button_down_on() || resp.dragged() {
                let f = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64;
                let v = ((min + f * (max - min)) / paso).round() * paso;
                let v = (v * 1e6).round() / 1e6;
                if (v - *valor).abs() > 1e-9 {
                    *valor = v;
                    cambio = true;
                }
            }
        }
        let f = (((*valor - min) / (max - min)) as f32).clamp(0.0, 1.0);
        let fa = ui.ctx().animate_value_with_time(resp.id.with("v"), f, 0.12);
        let xk = x0 + fa * (x1 - x0);
        let p = ui.painter();
        let pista = Rect::from_center_size(rect.center(), vec2(rect.width(), 10.0));
        p.rect_filled(pista, 5, OSCURO_2);
        p.rect_filled(Rect::from_min_max(pista.min, pos2(xk, pista.max.y)), 5, oro_a(0.75));
        let h = anim(ui, resp.id.with("h"), resp.hovered() || resp.dragged(), 0.15);
        p.circle_filled(pos2(xk, rect.center().y), 9.0 + 1.5 * h, OSCURO_2);
        p.circle_filled(pos2(xk, rect.center().y), 6.0 + 1.0 * h, ORO);
    });
    cambio
}

// ---------------------------------------------------------------------------------------------
// Barra de avance del transporte
// ---------------------------------------------------------------------------------------------

/// Lo que tiene que moverse el puntero (en px) para pedir otra búsqueda con el botón aún pulsado.
const UMBRAL_BUSQUEDA_PX: f32 = 0.5;

/// ¿Pedir otro cuadro? Al pulsar, siempre. Con el botón aún pulsado, solo si el puntero se movió de
/// verdad (más de 0.5 px) y el instante cambió al menos un cuadro (`cuadro_s`) desde la última
/// búsqueda: mantener el ratón quieto sobre la barra lanzaba un ffmpeg en cada repintado (10-30 por
/// segundo).
pub fn busqueda_nueva(anterior: Option<(f32, f64)>, x: f32, t: f64, cuadro_s: f64) -> bool {
    match anterior {
        None => true,
        Some((ax, at)) => (x - ax).abs() > UMBRAL_BUSQUEDA_PX && (t - at).abs() >= cuadro_s,
    }
}

/// Filtro de búsquedas de una barra de avance. `pulsando` es (x del puntero, instante bajo él) mientras
/// el botón está pulsado sobre la barra, o `None`. Recuerda en la memoria de egui la última búsqueda de
/// esta barra (se olvida al soltar) y devuelve el instante solo cuando hay que buscar de verdad.
pub fn filtrar_busqueda(ui: &Ui, id: Id, pulsando: Option<(f32, f64)>, cuadro_s: f64) -> Option<f64> {
    let clave = id.with("ultima_busqueda");
    let Some((x, t)) = pulsando else {
        ui.data_mut(|d| d.remove::<(f32, f64)>(clave));
        return None;
    };
    let anterior = ui.data(|d| d.get_temp::<(f32, f64)>(clave));
    if !busqueda_nueva(anterior, x, t, cuadro_s) {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(clave, (x, t)));
    Some(t)
}

/// Barra fina sobre marfil con los tramos aprobados, relleno tinta hasta la perilla dorada.
/// Devuelve el nuevo instante si el editor hizo clic o arrastró a otro sitio (ver [`filtrar_busqueda`]);
/// `cuadro_s` es la duración de un cuadro del clip.
pub fn barra_avance(ui: &mut Ui, ancho: f32, t: f64, dur: f64, tramos: &[(f64, f64)], cuadro_s: f64) -> Option<f64> {
    // zona de clic cómoda (20 px) pero línea fina (4 px): se ve delicada y se agarra fácil
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 20.0), Sense::click_and_drag());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let pista = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let x_de = |t: f64| pista.left() + (pista.width() as f64 * if dur > 0.0 { (t / dur).clamp(0.0, 1.0) } else { 0.0 }) as f32;
    let pulsando = resp
        .interact_pointer_pos()
        .filter(|_| (resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked()) && dur > 0.0)
        .map(|pos| (pos.x, ((pos.x - pista.left()) / pista.width()).clamp(0.0, 1.0) as f64 * dur));
    let nuevo = filtrar_busqueda(ui, resp.id, pulsando, cuadro_s);
    // la perilla sigue al puntero aunque no se pida otro cuadro
    let tt = pulsando.map(|p| p.1).unwrap_or(t);
    let p = ui.painter();
    // al pasar el ratón la línea engorda un poco (4 → 6 px) para indicar que se puede arrastrar
    let h = anim(ui, resp.id.with("h"), resp.hovered() || resp.dragged(), 0.15);
    let pista = Rect::from_center_size(pista.center(), vec2(pista.width(), 4.0 + 2.0 * h));
    p.rect_filled(pista, 3, tinta_a(0.15));
    for &(a, b) in tramos {
        let r = Rect::from_min_max(pos2(x_de(a), pista.top()), pos2(x_de(b).max(x_de(a) + 2.0), pista.bottom()));
        p.rect_filled(r, 3, oro_a(0.85));
    }
    let xk = x_de(tt);
    p.rect_filled(Rect::from_min_max(pista.min, pos2(xk, pista.max.y)), 3, tinta_a(0.85));
    let c = pos2(xk, pista.center().y);
    if h > 0.0 {
        p.circle_filled(c, 7.0 + 4.0 * h, oro_a(0.22 * h));
    }
    // perilla pequeña: anillo oscuro de 7 px con centro dorado
    p.circle_filled(c, 7.0 + h, TINTA);
    p.circle_filled(c, 4.5 + h, ORO);
    nuevo
}

// ---------------------------------------------------------------------------------------------
// Fichas, control segmentado, casillas y menús
// ---------------------------------------------------------------------------------------------

/// Ficha seleccionable (filtros de joya y plano). Sobre oscuro.
pub fn ficha(ui: &mut Ui, texto: &str, activa: bool) -> Response {
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_medio(13.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 30.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
    let a = anim(ui, resp.id.with("a"), activa, 0.18);
    let p = ui.painter();
    let fondo = mezclar(mezclar(OSCURO_2, OSCURO_HOVER, h), ORO, a);
    p.rect_filled(rect, 15, fondo);
    if a < 1.0 {
        p.rect_stroke(rect, 15, Stroke::new(1.0, marfil_a(0.08 * (1.0 - a))), StrokeKind::Inside);
    }
    let color = mezclar(MARFIL, TINTA, a);
    p.galley(rect.center() - g.size() / 2.0, g, color);
    resp
}

/// Control segmentado (criterio). Sobre oscuro; la selección se desliza animada.
pub fn segmentado(ui: &mut Ui, id: Id, opciones: &[&str], sel: usize) -> Option<usize> {
    segmentado_con_ayuda(ui, id, opciones, sel, None)
}

/// [`segmentado`] con un texto de ayuda al pasar por encima de cualquiera de sus opciones.
pub fn segmentado_con_ayuda(ui: &mut Ui, id: Id, opciones: &[&str], sel: usize, ayuda: Option<&str>) -> Option<usize> {
    let ancho = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, 38.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 19, OSCURO_2);
    let n = opciones.len().max(1) as f32;
    let w = (rect.width() - 6.0) / n;
    let xa = ui.ctx().animate_value_with_time(id.with("sel"), sel as f32, 0.2);
    let sel_rect = Rect::from_min_size(pos2(rect.left() + 3.0 + xa * w, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
    p.add(sombra_suave().as_shape(sel_rect, CornerRadius::same(16)));
    p.rect_filled(sel_rect, 16, MARFIL);
    let mut elegido = None;
    for (i, o) in opciones.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 3.0 + i as f32 * w, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
        let mut resp = ui.interact(r, id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if let Some(t) = ayuda {
            resp = resp.on_hover_text(t);
        }
        if resp.clicked() {
            elegido = Some(i);
        }
        let activo = (xa - i as f32).abs() < 0.5;
        let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
        let color = if activo { TINTA } else { mezclar(APAGADO, MARFIL, h) };
        p.text(r.center(), Align2::CENTER_CENTER, *o, f_medio(13.0), color);
    }
    elegido
}

/// Casilla redonda tipo interruptor pequeño (sobre oscuro o marfil según `sobre_marfil`).
pub fn interruptor(ui: &mut Ui, valor: &mut bool, texto: &str, sobre_marfil: bool) -> Response {
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_texto(13.0), Color32::PLACEHOLDER);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(34.0 + 8.0 + g.size().x, 24.0), Sense::click());
    resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        *valor = !*valor;
        resp.mark_changed();
    }
    let a = anim(ui, resp.id.with("a"), *valor, 0.18);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
    let p = ui.painter();
    let pista = Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), vec2(34.0, 18.0));
    let apagado = if sobre_marfil { tinta_a(0.18) } else { OSCURO_HOVER };
    p.rect_filled(pista, 9, mezclar(apagado, ORO, a));
    let x = pista.left() + 9.0 + a * 16.0;
    p.circle_filled(pos2(x, pista.center().y), 6.5 + h * 0.8, if sobre_marfil { MARFIL } else { MARFIL });
    if sobre_marfil {
        p.circle_stroke(pos2(x, pista.center().y), 6.5 + h * 0.8, Stroke::new(1.0, tinta_a(0.2)));
    }
    let color = if sobre_marfil { TINTA } else { MARFIL };
    p.galley(pos2(pista.right() + 8.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

/// Desplegable con forma de píldora: muestra el valor y abre un menú con las opciones.
/// Devuelve el índice elegido.
pub fn desplegable(ui: &mut Ui, id: Id, actual: &str, opciones: &[String], ancho: f32, sobre_marfil: bool) -> Option<usize> {
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 34.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
    let p = ui.painter();
    let (fondo, texto) = if sobre_marfil { (mezclar(tinta_a(0.07), tinta_a(0.13), h), TINTA) } else { (mezclar(OSCURO_2, OSCURO_HOVER, h), MARFIL) };
    p.rect_filled(rect, 17, fondo);
    let clip = p.with_clip_rect(rect.shrink2(vec2(14.0, 0.0)).with_max_x(rect.right() - 28.0));
    clip.text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, actual, f_medio(13.0), texto);
    // chevrón
    let c = pos2(rect.right() - 16.0, rect.center().y);
    p.line(vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.5), c + vec2(4.0, -2.0)], Stroke::new(1.6, texto));
    let mut elegido = None;
    egui::Popup::menu(&resp).id(id).width(ancho.max(180.0)).show(|ui| {
        ui.set_min_width(ancho.max(180.0));
        for (i, o) in opciones.iter().enumerate() {
            if item_menu(ui, o, o == actual).clicked() {
                elegido = Some(i);
            }
        }
    });
    elegido
}

/// Opción de un menú emergente, con marca dorada si está elegida.
pub fn item_menu(ui: &mut Ui, texto: &str, marcado: bool) -> Response {
    let ancho = ui.available_width().max(160.0);
    let g = ui.painter().layout(texto.to_owned(), f_texto(13.5), MARFIL, ancho - 34.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, g.size().y + 12.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, 10, oro_a(0.16 * h));
    if marcado {
        p.circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, ORO);
    }
    p.galley(pos2(rect.left() + 24.0, rect.center().y - g.size().y / 2.0), g, if marcado { ORO } else { MARFIL });
    resp
}

/// Gradiente radial (como los `radial-gradient` de la maqueta) con un abanico de triángulos.
/// `paradas`: (radio relativo 0..1, color). Se recorta al rectángulo con esquinas redondeadas
/// pintando primero y dejando que el llamador recorte con `clip`.
pub fn gradiente_radial(p: &Painter, rect: Rect, centro: Pos2, radio: Vec2, paradas: &[(f32, Color32)]) {
    let mut m = egui::Mesh::default();
    let seg = 48;
    // relleno de fondo con el color final, para cubrir las esquinas
    if let Some(&(_, ultimo)) = paradas.last() {
        p.rect_filled(rect, 0, ultimo);
    }
    let anillo = |r: f32| -> Vec<Pos2> {
        (0..seg).map(|i| {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            centro + vec2(a.cos() * radio.x * r, a.sin() * radio.y * r)
        }).collect()
    };
    // centro
    let c0 = paradas[0].1;
    let primero = anillo(paradas[0].0.max(0.001));
    let ic = m.vertices.len() as u32;
    m.colored_vertex(centro, c0);
    for q in &primero {
        m.colored_vertex(*q, c0);
    }
    for i in 0..seg as u32 {
        m.add_triangle(ic, ic + 1 + i, ic + 1 + (i + 1) % seg as u32);
    }
    // anillos entre paradas
    for w in paradas.windows(2) {
        let (r0, col0) = w[0];
        let (r1, col1) = w[1];
        let a = anillo(r0.max(0.001));
        let b = anillo(r1);
        let base = m.vertices.len() as u32;
        for i in 0..seg {
            m.colored_vertex(a[i], col0);
            m.colored_vertex(b[i], col1);
        }
        for i in 0..seg as u32 {
            let j = (i + 1) % seg as u32;
            m.add_triangle(base + 2 * i, base + 2 * i + 1, base + 2 * j);
            m.add_triangle(base + 2 * j, base + 2 * i + 1, base + 2 * j + 1);
        }
    }
    p.with_clip_rect(rect).add(Shape::mesh(m));
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use egui_kittest::Harness;

    /// Estado de la prueba: búsquedas pedidas y esquina de la barra.
    type Estado = (Vec<f64>, Option<Pos2>);

    fn pulsar(h: &Harness<'_, Estado>, pos: Pos2, abajo: bool) {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: abajo, modifiers: egui::Modifiers::NONE });
    }

    /// Mantener pulsada la barra de avance sin mover el ratón pide UNA búsqueda, no una por repintado
    /// (antes, 10-30 ffmpeg por segundo); medio píxel de temblor tampoco busca; moverla de verdad sí.
    #[test]
    fn barra_quieta_no_repite_busquedas() {
        let mut h = Harness::builder().with_size(egui::vec2(400.0, 80.0)).with_accessibility_check(false).build_ui_state(
            |ui, e: &mut Estado| {
                e.1 = Some(ui.cursor().min);
                // 60 s en 300 px: 0.2 s por píxel
                if let Some(t) = barra_avance(ui, 300.0, 0.0, 60.0, &[], 1.0 / 30.0) {
                    e.0.push(t);
                }
            },
            (Vec::new(), None),
        );
        h.step();
        let p = h.state().1.expect("barra dibujada") + vec2(150.0, 10.0);
        h.event(egui::Event::PointerMoved(p));
        pulsar(&h, p, true);
        h.step();
        assert_eq!(h.state().0.len(), 1, "al pulsar se busca una vez");
        // el botón sigue pulsado y el ratón quieto durante muchos repintados
        h.run_steps(30);
        assert_eq!(h.state().0.len(), 1, "con el ratón quieto no se vuelve a buscar: {:?}", h.state().0);
        // temblor de menos de medio píxel
        h.event(egui::Event::PointerMoved(p + vec2(0.3, 0.0)));
        h.run_steps(3);
        assert_eq!(h.state().0.len(), 1);
        // arrastre de verdad: otra búsqueda, más adelante
        h.event(egui::Event::PointerMoved(p + vec2(30.0, 0.0)));
        h.run_steps(5);
        assert_eq!(h.state().0.len(), 2, "{:?}", h.state().0);
        assert!(h.state().0[1] > h.state().0[0]);
        pulsar(&h, p + vec2(30.0, 0.0), false);
        h.run_steps(3);
        assert_eq!(h.state().0.len(), 2, "soltar no busca");
        // un clic nuevo en el mismo sitio vuelve a buscar
        pulsar(&h, p + vec2(30.0, 0.0), true);
        h.step();
        pulsar(&h, p + vec2(30.0, 0.0), false);
        h.run_steps(3);
        assert_eq!(h.state().0.len(), 3, "{:?}", h.state().0);
    }

    /// La regla de la barra: al pulsar siempre; luego, más de 0.5 px y al menos un cuadro.
    #[test]
    fn regla_de_busqueda_nueva() {
        assert!(busqueda_nueva(None, 10.0, 1.0, 1.0 / 30.0));
        assert!(!busqueda_nueva(Some((10.0, 1.0)), 10.0, 1.0, 1.0 / 30.0));
        assert!(!busqueda_nueva(Some((10.0, 1.0)), 10.4, 1.08, 1.0 / 30.0), "menos de medio píxel");
        assert!(!busqueda_nueva(Some((10.0, 1.0)), 12.0, 1.01, 1.0 / 30.0), "menos de un cuadro");
        assert!(busqueda_nueva(Some((10.0, 1.0)), 12.0, 1.2, 1.0 / 30.0));
    }
}
