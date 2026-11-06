//! Ventana de la edición Bodas (diseño "Salvia").
//!
//! Comparte todo el estado y la lógica con la edición de joyas (`app.rs`: material, análisis,
//! proyecto, color por clip, visor, exportar); aquí solo cambia el dibujo:
//!
//! - **Barra superior**: los anillos de la marca, el título del material, el estado del análisis y las
//!   acciones (Analizar, Exportar, Proyecto, modo claro/oscuro y los paneles que se ocultan).
//! - **Clips** (Ctrl+1): filas con miniatura, duración, tramos y los momentos del clip como puntos de
//!   color; encima, la tarjeta **Color** (Ctrl+3) con la conversión por clip, grupo o todos.
//! - **Visor**: el cuadro sin nada encima salvo las esquinas de las caras; los controles van debajo,
//!   nunca sobre la imagen. A su derecha, **Foco** (Ctrl+5): la cara principal ampliada, su momento y
//!   tres medidores (nitidez, exposición, cámara estable).
//! - **Criterio** (Ctrl+2): criterio, pesos, tramos, filtro de momentos (chips de color) y de planos,
//!   y la exportación a DaVinci.
//! - **Momentos** (Ctrl+4): la línea de tiempo de todo el material con un carril de color por momento
//!   y los tramos aprobados; se sube o se baja arrastrando su borde, rueda = zoom, Mayús+rueda = mover.
//!
//! Todo se dibuja con rectángulos calculados (como Marfil), así cada panel se oculta con una animación.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, Id, Key, Layout, Modifiers, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder};

use camcuts_core as nucleo;

use crate::app::{App, Columna};
use crate::bodas_tema::{self as bt, color_momento, mezclar, paleta, Paleta, TINTA_MOMENTO};
use crate::datos::{codigo_tiempo, fmt_corto, fmt_tiempo};
use crate::iconos::{self, Icono};
use crate::motor;
use crate::tema::{f_medio, f_negrita, f_texto, f_titulo};
use crate::widgets::{anim, desvanecer_scroll, texto_corto};

// =================================================================================================
// Medidas
// =================================================================================================

const MARGEN: f32 = 18.0;
const HUECO: f32 = 14.0;
const ALTO_BARRA: f32 = 64.0;
const ANCHO_CLIPS: f32 = 300.0;
const ANCHO_CRITERIO: f32 = 330.0;
const ANCHO_FOCO: f32 = 220.0;
const RADIO: u8 = 22;
const ALTO_FILA: f32 = 58.0;
const ALTO_COLOR: f32 = 150.0;
/// Columna de nombres de la línea de tiempo.
const ANCHO_ETIQUETAS: f32 = 116.0;

/// Colores de los pesos (en el orden de `motor::PESOS_BODAS`).
fn color_peso(k: &str) -> Color32 {
    match k {
        "sharp" => paleta().verde,
        "exposure" => color_momento("anillos"),
        "steady" => color_momento("discursos"),
        "size" => color_momento("preparativos"),
        "aniso" => color_momento("ceremonia"),
        "still" => color_momento("retratos"),
        _ => color_momento("votos"),
    }
}

// =================================================================================================
// Caché de la línea de tiempo (carriles por momento) y de los momentos de cada clip
// =================================================================================================

/// Lo que la línea de tiempo y la lista necesitan de los momentos, recalculado solo cuando cambian los
/// clips (cargar, analizar, fijar el momento de un clip).
#[derive(Default)]
pub struct CacheBodas {
    clave: u64,
    /// (momento, tramos globales [inicio, fin) en segundos), en el orden del día.
    pub carriles: Vec<(String, Vec<(f64, f64)>)>,
    /// Momentos más frecuentes de cada clip (clave del clip → hasta 3).
    pub por_clip: HashMap<String, Vec<String>>,
}

impl App {
    /// Rehace la caché de momentos si cambiaron los clips.
    ///
    /// Mirar unas pocas muestras no basta (una caché reanalizada o un momento cambiado entre ellas dejaba
    /// carriles viejos): la clave lleva la revisión de los clips, que sube cada vez que se releen las
    /// cachés (`cargar_clips`: cargar, analizar, fijar el momento de un clip), más el orden y la
    /// duración de los clips (ordenar la lista mueve los carriles).
    fn actualizar_cache_bodas(&mut self) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.revision_clips.hash(&mut h);
        for c in &self.clips {
            c.clave.hash(&mut h);
            c.dur().to_bits().hash(&mut h);
            c.entrada.as_ref().map(|e| e.muestras.len()).hash(&mut h);
        }
        let clave = h.finish();
        if clave == self.cache_bodas.clave && !self.cache_bodas.carriles.is_empty() {
            return;
        }
        let (offs, _) = self.desplazamientos();
        let mut carriles: Vec<(String, Vec<(f64, f64)>)> = nucleo::bodas::MOMENTOS.iter().map(|m| (m.to_string(), Vec::new())).collect();
        let mut por_clip = HashMap::new();
        for (i, c) in self.clips.iter().enumerate() {
            let Some(e) = &c.entrada else { continue };
            let paso = 1.0 / e.por_segundo.max(0.1);
            let mut cuenta: Vec<(String, usize)> = Vec::new();
            let mut k = 0;
            let ms = &e.muestras;
            while k < ms.len() {
                let m = &ms[k].momento;
                let mut f = k;
                while f + 1 < ms.len() && ms[f + 1].momento == *m {
                    f += 1;
                }
                if !m.is_empty() {
                    let (a, b) = (offs[i] + ms[k].t, offs[i] + (ms[f].t + paso).min(e.duracion));
                    if let Some(l) = carriles.iter_mut().find(|l| l.0 == *m) {
                        l.1.push((a, b));
                    } else {
                        carriles.push((m.clone(), vec![(a, b)]));
                    }
                    match cuenta.iter_mut().find(|x| x.0 == *m) {
                        Some(x) => x.1 += f - k + 1,
                        None => cuenta.push((m.clone(), f - k + 1)),
                    }
                }
                k = f + 1;
            }
            cuenta.sort_by(|a, b| b.1.cmp(&a.1));
            por_clip.insert(c.clave.clone(), cuenta.into_iter().take(3).map(|x| x.0).collect::<Vec<_>>());
        }
        carriles.retain(|l| !l.1.is_empty());
        self.cache_bodas = CacheBodas { clave, carriles, por_clip };
    }
}

// =================================================================================================
// Controles propios (con la paleta del modo actual)
// =================================================================================================

/// Un hijo de `ui` limitado a `rect`, con opacidad y recorte.
fn hijo(ui: &mut Ui, rect: Rect, id: &str, opacidad: f32, recorte: Rect) -> Ui {
    let mut h = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::top_down(Align::Min)));
    h.set_clip_rect(recorte.intersect(ui.clip_rect()));
    h.multiply_opacity(opacidad);
    h
}

/// Tarjeta blanca (o oscura) con sombra suave y borde fino.
fn tarjeta(p: &Painter, rect: Rect, pal: &Paleta, opacidad: f32) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let mut p = p.clone();
    p.multiply_opacity(opacidad);
    let r = CornerRadius::same(RADIO.min((rect.height() / 2.0) as u8));
    p.add(bt::sombra(pal).as_shape(rect, r));
    p.rect_filled(rect, r, pal.tarjeta);
    p.rect_stroke(rect, r, Stroke::new(1.0, pal.borde), StrokeKind::Inside);
}

/// Estilos de botón.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tipo {
    /// Relleno verde clarito (la acción principal).
    Principal,
    /// Relleno suave.
    Suave,
    /// Solo borde.
    Contorno,
}

/// Botón en píldora con icono opcional.
fn boton(ui: &mut Ui, texto: &str, icono: Option<Icono>, tipo: Tipo, alto: f32, activo: bool) -> Response {
    let pal = paleta();
    let tinta = if tipo == Tipo::Principal { pal.sobre_verde } else { pal.texto };
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_medio(13.5), tinta);
    let ancho = g.size().x + 30.0 + if icono.is_some() { 22.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, alto), if activo { Sense::click() } else { Sense::hover() });
    let hv = anim(ui, resp.id.with("h"), resp.hovered() && activo, 0.12);
    let pul = anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
    let r = rect.shrink(pul);
    let mut p = ui.painter().clone();
    if !activo {
        p.multiply_opacity(0.45);
    }
    match tipo {
        Tipo::Principal => {
            p.rect_filled(r, 255, mezclar(pal.verde, pal.verde_fuerte, 0.22 * hv));
        }
        Tipo::Suave => {
            p.rect_filled(r, 255, mezclar(pal.hundido, pal.verde_suave, hv));
        }
        Tipo::Contorno => {
            p.rect_filled(r, 255, mezclar(pal.tarjeta, pal.verde_suave, hv));
            p.rect_stroke(r, 255, Stroke::new(1.0, mezclar(pal.borde, pal.verde, hv)), StrokeKind::Inside);
        }
    }
    let mut x = r.left() + 15.0;
    if let Some(ic) = icono {
        iconos::dibujar(&p, ic, Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(17.0, 17.0)), tinta, 2.0);
        x += 22.0;
    }
    p.galley(pos2(x, r.center().y - g.size().y / 2.0), g, tinta);
    resp.on_hover_cursor(if activo { CursorIcon::PointingHand } else { CursorIcon::Default })
}

/// Botón redondo con icono; `activo` lo rellena de verde (paneles visibles, modos encendidos).
fn boton_icono(ui: &mut Ui, icono: Icono, d: f32, activo: bool, ayuda: &str) -> Response {
    let pal = paleta();
    let (rect, resp) = ui.allocate_exact_size(vec2(d, d), Sense::click());
    let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let on = anim(ui, resp.id.with("on"), activo, 0.18);
    let p = ui.painter();
    let fondo = mezclar(mezclar(pal.hundido, pal.verde_suave, hv), pal.verde, on);
    p.circle_filled(rect.center(), d / 2.0, fondo);
    iconos::dibujar(p, icono, rect.shrink(d * 0.28), mezclar(pal.suave, pal.sobre_verde, on.max(0.4 * hv)), 2.0);
    resp.on_hover_text(ayuda).on_hover_cursor(CursorIcon::PointingHand)
}

/// Chip: un momento (con su punto de color) o una opción; activa = rellena.
fn chip(ui: &mut Ui, texto: &str, color: Option<Color32>, activa: bool) -> Response {
    let pal = paleta();
    let tinta = match (activa, color) {
        (true, Some(_)) => TINTA_MOMENTO,
        (true, None) => pal.sobre_verde,
        _ => pal.texto,
    };
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_medio(12.5), tinta);
    let ancho = g.size().x + 22.0 + if color.is_some() { 13.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 27.0), Sense::click());
    let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let on = anim(ui, resp.id.with("on"), activa, 0.18);
    let p = ui.painter();
    let base = mezclar(pal.hundido, pal.verde_suave, hv);
    let lleno = color.unwrap_or(pal.verde);
    p.rect_filled(rect, 255, mezclar(base, lleno, on));
    let mut x = rect.left() + 11.0;
    if let Some(c) = color {
        p.circle_filled(pos2(x + 3.5, rect.center().y), 4.0, mezclar(c, TINTA_MOMENTO.gamma_multiply(0.55), on));
        x += 13.0;
    }
    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, tinta);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Control segmentado (criterio). Devuelve la opción pulsada si cambió.
fn segmentado(ui: &mut Ui, id: Id, opciones: &[&str], sel: usize) -> Option<usize> {
    let pal = paleta();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 255, pal.hundido);
    let n = opciones.len().max(1);
    let w = (rect.width() - 8.0) / n as f32;
    let xs = ui.ctx().animate_value_with_time(id.with("x"), sel as f32, 0.2);
    let pastilla = Rect::from_min_size(pos2(rect.left() + 4.0 + xs * w, rect.top() + 4.0), vec2(w, rect.height() - 8.0));
    p.add(bt::sombra(&pal).as_shape(pastilla, CornerRadius::same(255)));
    p.rect_filled(pastilla, 255, pal.tarjeta);
    let mut out = None;
    for (i, o) in opciones.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 4.0 + i as f32 * w, rect.top() + 4.0), vec2(w, rect.height() - 8.0));
        let resp = ui.interact(r, id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() && i != sel {
            out = Some(i);
        }
        p.text(r.center(), Align2::CENTER_CENTER, *o, f_medio(13.0), if i == sel { pal.texto } else { pal.suave });
    }
    out
}

/// Deslizador con nombre, valor y riel de color. Devuelve si cambió.
#[allow(clippy::too_many_arguments)]
fn deslizador(ui: &mut Ui, nombre: &str, color: Color32, valor: &mut f64, min: f64, max: f64, paso: f64, formato: &dyn Fn(f64) -> String) -> bool {
    let pal = paleta();
    let ancho = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, 42.0), Sense::hover());
    let riel = Rect::from_min_max(pos2(rect.left() + 2.0, rect.top() + 26.0), pos2(rect.right() - 2.0, rect.top() + 32.0));
    let resp = ui.interact(riel.expand2(vec2(4.0, 9.0)), Id::unique(("desl", nombre)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
    let mut cambio = false;
    if resp.dragged() || resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let f = ((pp.x - riel.left()) / riel.width()).clamp(0.0, 1.0) as f64;
            let v = ((min + f * (max - min)) / paso).round() * paso;
            let v = v.clamp(min, max);
            if (v - *valor).abs() > 1e-9 {
                *valor = v;
                cambio = true;
            }
        }
    }
    let p = ui.painter();
    p.circle_filled(pos2(rect.left() + 5.0, rect.top() + 9.0), 4.0, color);
    texto_corto(p, pos2(rect.left() + 16.0, rect.top() + 9.0), Align2::LEFT_CENTER, nombre, f_texto(13.0), pal.texto, ancho - 70.0);
    p.text(pos2(rect.right(), rect.top() + 9.0), Align2::RIGHT_CENTER, formato(*valor), f_negrita(13.0), pal.texto);
    let f = ((*valor - min) / (max - min)).clamp(0.0, 1.0) as f32;
    p.rect_filled(riel, 255, pal.hundido);
    p.rect_filled(Rect::from_min_max(riel.min, pos2(riel.left() + riel.width() * f, riel.bottom())), 255, color);
    let hv = anim(ui, resp.id.with("h"), resp.hovered() || resp.dragged(), 0.12);
    let perilla = pos2(riel.left() + riel.width() * f, riel.center().y);
    let rr = 7.5 + 2.0 * hv;
    p.circle_filled(perilla, rr, pal.tarjeta);
    p.circle_stroke(perilla, rr, Stroke::new(2.0, mezclar(color, pal.verde_fuerte, 0.45)));
    cambio
}

/// Interruptor con texto. Devuelve la respuesta (con `changed()` si se pulsó).
fn interruptor(ui: &mut Ui, valor: &mut bool, texto: &str) -> Response {
    let pal = paleta();
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_texto(13.0), pal.texto);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(38.0 + 8.0 + g.size().x, 26.0), Sense::click());
    if resp.clicked() {
        *valor = !*valor;
        resp.mark_changed();
    }
    let a = anim(ui, resp.id.with("on"), *valor, 0.15);
    let p = ui.painter();
    let pista = Rect::from_min_size(pos2(rect.left(), rect.center().y - 10.0), vec2(36.0, 20.0));
    p.rect_filled(pista, 255, mezclar(pal.borde, pal.verde, a));
    p.circle_filled(pos2(pista.left() + 10.0 + 16.0 * a, pista.center().y), 7.5, Color32::WHITE);
    p.galley(pos2(pista.right() + 8.0, rect.center().y - g.size().y / 2.0), g, pal.texto);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Opción de menú (con punto de color opcional, p. ej. el de un momento).
fn item_menu(ui: &mut Ui, texto: &str, marcado: bool, punto: Option<Color32>) -> Response {
    let pal = paleta();
    let ancho = ui.available_width().max(170.0);
    let g = ui.painter().layout(texto.to_owned(), f_texto(13.5), pal.texto, ancho - 36.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, g.size().y + 12.0), Sense::click());
    let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    if marcado || hv > 0.0 {
        p.rect_filled(rect, 10, if marcado { pal.verde_suave } else { pal.verde_suave.gamma_multiply(hv) });
    }
    match punto {
        Some(c) => {
            p.circle_filled(pos2(rect.left() + 13.0, rect.center().y), 4.5, c);
        }
        None if marcado => {
            p.circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.5, pal.verde_fuerte);
        }
        None => {}
    }
    p.galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, pal.texto);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Píldora pequeña de 26 px (la conversión de color en el transporte).
fn boton_fino(ui: &mut Ui, texto: &str) -> Response {
    let pal = paleta();
    let g = ui.painter().layout_no_wrap(texto.to_owned(), f_medio(13.0), pal.texto);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 26.0), Sense::click());
    let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, 255, mezclar(pal.hundido, pal.verde_suave, hv));
    p.galley(pos2(rect.left() + 13.0, rect.center().y - g.size().y / 2.0), g, pal.texto);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Desplegable: muestra el valor actual y abre un menú. Devuelve la opción elegida.
fn desplegable(ui: &mut Ui, id: Id, actual: &str, punto: Option<Color32>, opciones: &[(String, Option<Color32>)], ancho: f32) -> Option<usize> {
    let pal = paleta();
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 32.0), Sense::click());
    let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, 12, mezclar(pal.hundido, pal.verde_suave, hv));
    let mut x = rect.left() + 12.0;
    if let Some(c) = punto {
        p.circle_filled(pos2(x + 4.0, rect.center().y), 4.5, c);
        x += 15.0;
    }
    texto_corto(p, pos2(x, rect.center().y), Align2::LEFT_CENTER, actual, f_medio(13.0), pal.texto, rect.right() - 30.0 - x);
    let ch = pos2(rect.right() - 15.0, rect.center().y);
    p.line(vec![ch + vec2(-4.0, -2.0), ch + vec2(0.0, 2.0), ch + vec2(4.0, -2.0)], Stroke::new(1.6, pal.suave));
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let mut out = None;
    egui::Popup::menu(&resp).id(id).width(ancho.max(210.0)).show(|ui| {
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            for (i, (o, c)) in opciones.iter().enumerate() {
                if item_menu(ui, o, o == actual, *c).clicked() {
                    out = Some(i);
                }
            }
        });
    });
    out
}

/// Esquinas de una caja (como el visor de una cámara): se lee la cara sin taparla.
fn esquinas(p: &Painter, b: Rect, color: Color32, grosor: f32) {
    let l = (b.width().min(b.height()) * 0.24).clamp(6.0, 28.0);
    let s = Stroke::new(grosor, color);
    for (c, dx, dy) in [(b.left_top(), 1.0, 1.0), (b.right_top(), -1.0, 1.0), (b.left_bottom(), 1.0, -1.0), (b.right_bottom(), -1.0, -1.0)] {
        p.line_segment([c, c + vec2(l * dx, 0.0)], s);
        p.line_segment([c, c + vec2(0.0, l * dy)], s);
    }
}

/// Medidor 0..1 con nombre y valor (verde si va bien, coral si no).
fn medidor(p: &Painter, rect: Rect, nombre: &str, v: f64, pal: &Paleta) {
    let v = v.clamp(0.0, 1.0) as f32;
    p.text(pos2(rect.left(), rect.top() + 7.0), Align2::LEFT_CENTER, nombre, f_texto(12.5), pal.suave);
    p.text(pos2(rect.right(), rect.top() + 7.0), Align2::RIGHT_CENTER, format!("{:.0}%", 100.0 * v), f_negrita(12.5), pal.texto);
    let riel = Rect::from_min_max(pos2(rect.left(), rect.top() + 18.0), pos2(rect.right(), rect.top() + 24.0));
    p.rect_filled(riel, 255, pal.hundido);
    let c = if v >= 0.6 { pal.verde } else { mezclar(pal.rojo, pal.verde, (v / 0.6).powf(2.0)) };
    p.rect_filled(Rect::from_min_max(riel.min, pos2(riel.left() + riel.width() * v.max(0.02), riel.bottom())), 255, c);
}

/// Color del texto de estado: el rojo de los avisos se respeta; lo demás, gris de la paleta.
fn color_estado(c: Color32, pal: &Paleta) -> Color32 {
    if c == crate::tema::ROJO { pal.rojo } else { pal.suave }
}

// =================================================================================================
// La ventana
// =================================================================================================

impl App {
    /// Dibuja toda la ventana de la edición Bodas dentro de `ui`.
    pub fn dibujar_bodas(&mut self, ui: &mut Ui) {
        self.antes_de_dibujar();
        self.atender_cierre(&ui.ctx().clone());
        self.atajos(ui);
        self.atajos_bodas(ui);
        self.actualizar_cache_bodas();
        let pal = paleta();
        let pantalla = ui.ctx().content_rect();
        let p = ui.painter().with_clip_rect(pantalla);
        p.rect_filled(pantalla, 0, pal.fondo);
        // un velo verde muy suave arriba a la izquierda: da calidez sin distraer
        crate::widgets::gradiente_radial(
            &p,
            pantalla,
            pantalla.left_top() + vec2(pantalla.width() * 0.18, 0.0),
            vec2(pantalla.width() * 0.55, pantalla.height() * 0.55),
            &[(0.0, mezclar(pal.fondo, pal.verde, if pal.claro { 0.22 } else { 0.10 })), (1.0, pal.fondo)],
        );

        let ctx = ui.ctx().clone();
        let suave = |x: f32| x * x * (3.0 - 2.0 * x);
        let a_cli = suave(ctx.animate_bool_with_time(Id::unique("b_cli"), self.paneles.material, 0.25));
        let a_cri = suave(ctx.animate_bool_with_time(Id::unique("b_cri"), self.paneles.pesos, 0.25));
        let a_col = suave(ctx.animate_bool_with_time(Id::unique("b_col"), self.paneles.color, 0.25));
        let a_tie = suave(ctx.animate_bool_with_time(Id::unique("b_tie"), self.paneles.tiempo, 0.25));
        let a_foc = suave(ctx.animate_bool_with_time(Id::unique("b_foc"), self.paneles.mapa, 0.25));

        let r = pantalla.shrink(MARGEN);
        let barra = Rect::from_min_size(r.min, vec2(r.width(), ALTO_BARRA));
        let cuerpo = Rect::from_min_max(pos2(r.left(), barra.bottom() + HUECO), r.max);
        let max_tiempo = (cuerpo.height() * 0.55).max(150.0);
        self.alto_tiempo = self.alto_tiempo.clamp(130.0, max_tiempo);
        let alto_t = self.alto_tiempo * a_tie;
        let tiempo = Rect::from_min_max(pos2(cuerpo.left(), cuerpo.bottom() - alto_t), cuerpo.max);
        let sup = Rect::from_min_max(cuerpo.min, pos2(cuerpo.right(), cuerpo.bottom() - alto_t - HUECO * a_tie));
        let izq = Rect::from_min_size(sup.min, vec2(ANCHO_CLIPS * a_cli, sup.height()));
        let der = Rect::from_min_max(pos2(sup.right() - ANCHO_CRITERIO * a_cri, sup.top()), sup.max);
        let centro = Rect::from_min_max(pos2(izq.right() + HUECO * a_cli, sup.top()), pos2(der.left() - HUECO * a_cri, sup.bottom()));

        self.barra_superior(ui, barra);
        if a_cli > 0.01 {
            let col = Rect::from_min_size(pos2(izq.right() - ANCHO_CLIPS, izq.top()), vec2(ANCHO_CLIPS, izq.height()));
            let recorte = Rect::from_min_max(pos2(pantalla.left(), barra.bottom() + 2.0), pos2(izq.right() + HUECO, sup.bottom() + 12.0));
            let mut h = hijo(ui, col, "b_izq", a_cli, recorte);
            self.columna_clips(&mut h, col, a_col);
        }
        if a_cri > 0.01 {
            let col = Rect::from_min_size(der.min, vec2(ANCHO_CRITERIO, der.height()));
            let recorte = Rect::from_min_max(pos2(der.left() - HUECO, barra.bottom() + 2.0), pos2(pantalla.right(), sup.bottom() + 12.0));
            let mut h = hijo(ui, col, "b_der", a_cri, recorte);
            self.columna_criterio(&mut h, col);
        }
        {
            let mut h = hijo(ui, centro, "b_centro", 1.0, pantalla);
            self.visor_bodas(&mut h, centro, a_foc);
        }
        if a_tie > 0.01 {
            let caja = Rect::from_min_size(tiempo.min, vec2(tiempo.width(), self.alto_tiempo));
            let mut h = hijo(ui, caja, "b_tiempo", a_tie, Rect::from_min_max(pos2(pantalla.left(), tiempo.top() - 30.0), pantalla.max));
            self.carriles(&mut h, caja, max_tiempo);
        }
        self.aviso_bodas(ui);
    }

    /// Atajos propios de esta edición: Ctrl+L alterna el modo claro y el oscuro.
    fn atajos_bodas(&mut self, ui: &mut Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::L)) {
            self.alternar_modo();
        }
    }

    /// Modo claro ↔ oscuro (se recuerda en el proyecto).
    pub fn alternar_modo(&mut self) {
        self.modo_claro = !self.modo_claro;
        bt::instalar(&self.ctx, self.modo_claro);
        self.marcar_sucio();
    }

    // =============================================================================================
    // Barra superior
    // =============================================================================================

    fn barra_superior(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let p = ui.painter().clone();
        // la marca: dos anillos entrelazados
        let c = pos2(rect.left() + 38.0, rect.center().y);
        p.circle_stroke(c - vec2(7.0, 0.0), 12.0, Stroke::new(3.2, pal.verde));
        p.circle_stroke(c + vec2(7.0, 0.0), 12.0, Stroke::new(3.2, pal.verde_fuerte));
        let x = rect.left() + 66.0;
        let rt = p.text(pos2(x, rect.center().y - 9.0), Align2::LEFT_CENTER, "CAMCUTS Bodas", f_titulo(21.0), pal.texto);
        texto_corto(&p, pos2(x, rect.center().y + 12.0), Align2::LEFT_CENTER, &self.titulo(), f_texto(12.5), pal.suave, 300.0);

        // acciones, de derecha a izquierda
        let zona = Rect::from_min_max(pos2(rt.right() + 20.0, rect.top() + 12.0), pos2(rect.right() - 14.0, rect.bottom() - 12.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_acciones").max_rect(zona).layout(Layout::right_to_left(Align::Center)));
        h.spacing_mut().item_spacing.x = 8.0;
        let mut accion = "";
        if boton(&mut h, "Exportar", Some(Icono::Adelante), Tipo::Principal, 40.0, true).on_hover_text(format!("Línea de tiempo para DaVinci (OTIO + CSV) · Ctrl+E\n{}", self.resumen)).clicked() {
            accion = "exportar";
        }
        if self.analisis.is_some() {
            if boton(&mut h, "Detener", Some(Icono::Detener), Tipo::Suave, 40.0, true).clicked() {
                accion = "detener";
            }
        } else if boton(&mut h, "Analizar", Some(Icono::Rayo), Tipo::Contorno, 40.0, !self.fuentes.is_empty())
            .on_hover_text("Analiza el material que falta (los clips ya analizados se saltan)")
            .clicked()
        {
            accion = "analizar";
        }
        let rp = boton(&mut h, "Proyecto", Some(Icono::Carpeta), Tipo::Contorno, 40.0, true);
        egui::Popup::menu(&rp).id(Id::unique("b_menu_proyecto")).width(300.0).show(|ui| {
            ui.set_min_width(300.0);
            ui.label(egui::RichText::new("Proyecto").font(f_titulo(20.0)).color(pal.texto));
            if !self.etiqueta_proyecto.is_empty() {
                ui.label(egui::RichText::new(&self.etiqueta_proyecto).font(f_texto(12.0)).color(pal.suave));
            }
            if item_menu(ui, "Nuevo proyecto        Ctrl+N", false, None).clicked() { accion = "nuevo"; }
            if item_menu(ui, "Abrir proyecto…       Ctrl+O", false, None).clicked() { accion = "abrir"; }
            if item_menu(ui, "Guardar como…   Ctrl+Mayús+S", false, None).clicked() { accion = "guardar_como"; }
            ui.separator();
            ui.label(egui::RichText::new("Material").font(f_medio(13.0)).color(pal.suave));
            ui.label(egui::RichText::new(self.texto_fuentes()).font(f_texto(12.0)).color(pal.tenue));
            if item_menu(ui, "+ Carpeta…", false, None).clicked() { accion = "carpeta"; }
            if item_menu(ui, "+ Archivos…", false, None).clicked() { accion = "archivos"; }
            if item_menu(ui, "Quitar todo", false, None).clicked() { accion = "quitar"; }
            let mut r = self.recursivo;
            if interruptor(ui, &mut r, "Subcarpetas").changed() { self.recursivo = r; accion = "recargar"; }
            let mut b = self.segundo_plano;
            if interruptor(ui, &mut b, "Analizar en segundo plano").changed() { self.segundo_plano = b; accion = "sucio"; }
            let mut z = self.zoom_joyas;
            if interruptor(ui, &mut z, "Zoom caras lejanas (más lento)").changed() { self.zoom_joyas = z; accion = "sucio"; }
            ui.separator();
            ui.label(egui::RichText::new(format!("Resultados en: {}", if self.out.is_empty() { "—" } else { &self.out })).font(f_texto(12.0)).color(pal.suave));
            if item_menu(ui, "Cambiar carpeta de resultados…", false, None).clicked() { accion = "out"; }
            if item_menu(ui, "Abrir carpeta de resultados", false, None).clicked() { accion = "ver_out"; }
            if item_menu(ui, "Abrir resultados existentes (sin material)…", false, None).clicked() { accion = "resultados"; }
            ui.separator();
            ui.label(egui::RichText::new("Espacio reproducir · ←/→ ±2 s · N siguiente tramo · ↑/↓ clip · Ctrl+E exportar · Ctrl+L claro/oscuro").font(f_texto(11.5)).color(pal.tenue));
        });
        h.add_space(6.0);
        if boton_icono(&mut h, Icono::Sol, 36.0, !self.modo_claro, if self.modo_claro { "Modo oscuro · Ctrl+L" } else { "Modo claro · Ctrl+L" }).clicked() {
            accion = "modo";
        }
        h.add_space(6.0);
        let paneles: [(Icono, &str, fn(&mut crate::app::Paneles) -> &mut bool); 5] = [
            (Icono::Ojo, "Foco en la cara · Ctrl+5", |p| &mut p.mapa),
            (Icono::Tiempo, "Momentos (línea de tiempo) · Ctrl+4", |p| &mut p.tiempo),
            (Icono::Color, "Color por clip · Ctrl+3", |p| &mut p.color),
            (Icono::Pesos, "Criterio · Ctrl+2", |p| &mut p.pesos),
            (Icono::Material, "Clips · Ctrl+1", |p| &mut p.material),
        ];
        for (icono, ayuda, campo) in paneles {
            let activo = *campo(&mut self.paneles);
            if boton_icono(&mut h, icono, 36.0, activo, ayuda).clicked() {
                let v = campo(&mut self.paneles);
                *v = !*v;
                self.marcar_sucio();
            }
        }
        // en medio: el estado (y el avance del análisis)
        let x_est = rt.right() + 24.0;
        let ancho_est = h.min_rect().left() - 18.0 - x_est;
        if ancho_est > 60.0 {
            texto_corto(&p, pos2(x_est, rect.center().y - if self.progreso.is_some() { 5.0 } else { 0.0 }), Align2::LEFT_CENTER, &self.estado, f_texto(12.5), color_estado(self.estado_color, &pal), ancho_est);
            if let Some((a, b)) = self.progreso {
                let riel = Rect::from_min_size(pos2(x_est, rect.center().y + 9.0), vec2(ancho_est.min(320.0), 4.0));
                p.rect_filled(riel, 2, pal.hundido);
                let f = if b > 0 { a as f32 / b as f32 } else { 0.0 };
                let fa = ui.ctx().animate_value_with_time(Id::unique("b_prog"), f, 0.4);
                p.rect_filled(Rect::from_min_size(riel.min, vec2(riel.width() * fa.max(0.02), 4.0)), 2, pal.verde_fuerte);
            }
            ui.interact(Rect::from_min_size(pos2(x_est, rect.top()), vec2(ancho_est, rect.height())), Id::unique("b_estado"), Sense::hover())
                .on_hover_text(format!("{}\n{}", self.estado, self.totales));
        }
        match accion {
            "exportar" => self.exportar(),
            "analizar" => self.analizar(),
            "detener" => self.detener(),
            "nuevo" => self.nuevo_proyecto(),
            "abrir" => self.abrir_proyecto_dialogo(),
            "guardar_como" => self.guardar_como(),
            "carpeta" => self.agregar_carpeta(),
            "archivos" => self.agregar_archivos(),
            "quitar" => self.quitar_fuentes(),
            "recargar" => {
                self.cargar_clips();
                self.marcar_sucio();
            }
            "sucio" => self.marcar_sucio(),
            "out" => self.elegir_out(),
            "ver_out" => self.abrir_carpeta_resultados(),
            // no con un análisis en curso, y guardando antes lo pendiente
            "resultados" => self.abrir_resultados_dialogo("Carpeta de resultados de CAMCUTS Bodas"),
            "modo" => self.alternar_modo(),
            _ => {}
        }
    }

    // =============================================================================================
    // Columna de clips (con la tarjeta de color encima)
    // =============================================================================================

    fn columna_clips(&mut self, ui: &mut Ui, col: Rect, a_col: f32) {
        let alto_color = ALTO_COLOR * a_col;
        let color = Rect::from_min_size(col.min, vec2(col.width(), alto_color));
        let lista = Rect::from_min_max(pos2(col.left(), color.bottom() + HUECO * a_col), col.max);
        if a_col > 0.01 && color.height() > 8.0 {
            let mut h = hijo(ui, color, "b_color", a_col, color.expand(30.0));
            self.tarjeta_color_bodas(&mut h, color);
        }
        if lista.height() > 80.0 {
            self.lista_clips(ui, lista);
        }
    }

    fn tarjeta_color_bodas(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let interior = rect.shrink2(vec2(18.0, 14.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_color_in").max_rect(interior).layout(Layout::top_down(Align::Min)));
        h.set_clip_rect(interior.expand(4.0).intersect(ui.clip_rect()));
        h.spacing_mut().item_spacing = vec2(6.0, 9.0);
        h.horizontal(|ui| {
            ui.label(egui::RichText::new("Color").font(f_titulo(19.0)).color(pal.texto));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut v = self.ver_color;
                if interruptor(ui, &mut v, "Ver con color").changed() {
                    self.ver_color = v;
                    self.texturas_mini.clear();
                    self.redibujar_cuadro();
                    self.marcar_sucio();
                    self.limpiar_minis();
                }
            });
        });
        let opciones = motor::transformaciones();
        let actual = self.actual.and_then(|i| self.clips.get(i)).map(|c| self.color_de(c)).unwrap_or_else(motor::ColorSpec::auto);
        let etiquetas: Vec<(String, Option<Color32>)> = opciones.iter().map(|o| (o.1.clone(), None)).collect();
        let ancho = h.available_width();
        if let Some(i) = desplegable(&mut h, Id::unique("b_dd_color"), &motor::etiqueta_color(&actual.id), None, &etiquetas, ancho) {
            let id = opciones[i].0.clone();
            self.aplicar_color(&id, "este");
        }
        let n_sel = self.seleccion.len();
        let (mut accion, mut detectar) = (None, false);
        h.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if boton(ui, &format!("Elegidos ({n_sel})"), None, Tipo::Suave, 30.0, n_sel > 0).on_hover_text("Este color a los clips elegidos con Ctrl/Mayús+clic").clicked() {
                accion = Some("seleccionados");
            }
            if boton(ui, "Todos", None, Tipo::Suave, 30.0, !self.clips.is_empty()).clicked() {
                accion = Some("todos");
            }
            if boton_icono(ui, Icono::Lupa, 30.0, false, "Detectar perfil: lee el perfil de cámara de los seleccionados, o de todos").clicked() && !self.clips.is_empty() {
                detectar = true;
            }
        });
        if let Some(a) = accion {
            self.aplicar_color(&actual.id, a);
        }
        if detectar {
            self.detectar_colores();
        }
    }

    fn lista_clips(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let interior = rect.shrink2(vec2(10.0, 12.0));
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_clips_in").max_rect(interior).layout(Layout::top_down(Align::Min)));
        h.set_clip_rect(interior.expand(2.0).intersect(ui.clip_rect()));
        let mut accion: Option<&str> = None;
        h.horizontal(|ui| {
            ui.set_height(30.0);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Clips").font(f_titulo(19.0)).color(pal.texto));
            ui.label(egui::RichText::new(self.clips.len().to_string()).font(f_texto(12.5)).color(pal.tenue));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let r_mas = boton_icono(ui, Icono::Mas, 30.0, false, "Agregar material");
                egui::Popup::menu(&r_mas).id(Id::unique("b_popup_material")).width(240.0).show(|ui| {
                    if item_menu(ui, "+ Carpeta…", false, None).clicked() { accion = Some("carpeta"); }
                    if item_menu(ui, "+ Archivos…", false, None).clicked() { accion = Some("archivos"); }
                });
                let r_ord = boton_icono(ui, Icono::Orden, 30.0, self.orden.is_some(), "Ordenar");
                egui::Popup::menu(&r_ord).id(Id::unique("b_popup_orden")).width(220.0).show(|ui| {
                    ui.label(egui::RichText::new("Ordenar por").font(f_titulo(17.0)).color(pal.texto));
                    for (col, nombre) in Columna::TODAS {
                        let nombre = if col == Columna::Joya { "Con sujeto" } else { nombre };
                        let marcado = self.orden.map(|o| o.0) == Some(col);
                        let flecha = match self.orden { Some((c, rev)) if c == col => if rev { "  ↓" } else { "  ↑" }, _ => "" };
                        if item_menu(ui, &format!("{nombre}{flecha}"), marcado, None).clicked() {
                            self.ordenar_por(col);
                        }
                    }
                });
            });
        });
        match accion {
            Some("carpeta") => self.agregar_carpeta(),
            Some("archivos") => self.agregar_archivos(),
            _ => {}
        }
        h.add_space(4.0);
        h.spacing_mut().item_spacing.y = 0.0;
        let alto_pie = 20.0;
        let alto_lista = (h.available_height() - alto_pie - 4.0).max(40.0);
        self.filas_visibles = (alto_lista / ALTO_FILA).floor() as usize;
        let mut clic: Option<(usize, bool, bool)> = None;
        let salida = egui::ScrollArea::vertical().id_salt("b_lista").max_height(alto_lista).min_scrolled_height(alto_lista).auto_shrink([false, false]).show_rows(
            &mut h,
            ALTO_FILA,
            self.clips.len().max(1),
            |ui, filas| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if self.clips.is_empty() {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Sin clips todavía. Pulsa + para agregar la carpeta de la boda, o abre un proyecto desde Proyecto.").font(f_texto(12.5)).color(pal.suave));
                    return;
                }
                for i in filas {
                    if i < self.clips.len() {
                        if let Some(c) = self.fila_clip_bodas(ui, i) {
                            clic = Some(c);
                        }
                    }
                }
            },
        );
        desvanecer_scroll(ui.painter(), &salida, pal.tarjeta);
        if let Some((i, ctrl, shift)) = clic {
            self.clic_fila(i, ctrl, shift);
        }
        let pie = Rect::from_min_max(pos2(interior.left() + 8.0, interior.bottom() - alto_pie), interior.max);
        texto_corto(ui.painter(), pos2(pie.left(), pie.center().y), Align2::LEFT_CENTER, &self.resumen, f_texto(11.5), pal.tenue, pie.width() - 8.0);
        ui.interact(pie, Id::unique("b_pie_clips"), Sense::hover()).on_hover_text(self.totales.clone());
    }

    /// Una fila de la lista: miniatura, nombre, duración y tramos, los momentos del clip y su puntuación.
    fn fila_clip_bodas(&mut self, ui: &mut Ui, i: usize) -> Option<(usize, bool, bool)> {
        let pal = paleta();
        let ancho = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(ancho, ALTO_FILA), Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let c = &self.clips[i];
        let es_actual = self.actual == Some(i);
        let seleccionado = self.seleccion.contains(&c.clave);
        let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
        let sel = anim(ui, resp.id.with("s"), es_actual, 0.18);
        let p = ui.painter();
        let fila = rect.shrink2(vec2(2.0, 3.0));
        let fondo = mezclar(mezclar(pal.tarjeta, pal.hundido, hv), pal.verde_suave, sel.max(if seleccionado { 0.55 } else { 0.0 }));
        p.rect_filled(fila, 14, fondo);
        if sel > 0.01 {
            p.rect_filled(Rect::from_min_size(pos2(fila.left() + 1.0, fila.top() + 12.0), vec2(4.0, fila.height() - 24.0)), 2, pal.verde_fuerte.gamma_multiply(sel));
        }
        // miniatura redondeada
        let mini = Rect::from_min_size(pos2(fila.left() + 12.0, fila.center().y - 20.0), vec2(64.0, 40.0));
        match self.texturas_mini.get(&c.clave) {
            Some(t) => {
                p.add(egui::epaint::RectShape::filled(mini, 10, Color32::WHITE).with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
            }
            None => {
                let pc = p.with_clip_rect(mini);
                crate::widgets::gradiente_radial(&pc, mini, mini.center(), vec2(40.0, 26.0), &[(0.0, pal.verde_suave), (1.0, pal.hundido)]);
            }
        }
        if c.entrada.is_none() {
            p.rect_filled(mini, 10, Color32::from_white_alpha(if pal.claro { 120 } else { 0 }).gamma_multiply(1.0));
        }
        // derecha: puntuación y punto de estado
        let punt = c.media().map(|m| format!("{:.2}", m).trim_start_matches('0').to_owned()).unwrap_or_else(|| "—".into());
        let color_p = if es_actual { pal.verde_fuerte } else { pal.texto };
        // mientras se analiza, el porcentaje del clip en lugar de la puntuación (0 % = en cola) y una
        // barra fina al pie de la fila
        let avance = self.avance_de(c);
        let (punt, color_p) = match avance {
            Some(Some(v)) => (format!("{v}%"), if v > 0 { pal.verde_fuerte } else { pal.tenue }),
            Some(None) => ("error".to_owned(), pal.rojo),
            None => (punt, color_p),
        };
        if let Some(Some(v)) = avance.filter(|a| matches!(a, Some(1..=99))) {
            let pista = Rect::from_min_size(pos2(fila.left() + 12.0, fila.bottom() - 5.0), vec2(fila.width() - 24.0, 3.0));
            p.rect_filled(pista, 2, pal.hundido);
            p.rect_filled(Rect::from_min_size(pista.min, vec2(pista.width() * v as f32 / 100.0, 3.0)), 2, pal.verde_fuerte);
        }
        let x_der = fila.right() - 12.0;
        p.text(pos2(x_der, fila.center().y - 8.0), Align2::RIGHT_CENTER, punt, f_negrita(13.5), color_p);
        let estado = self.estado_de(c);
        let color_punto = match &c.entrada {
            Some(_) if estado.starts_with("Reanalizar") => pal.rojo,
            Some(_) if !c.tramos.is_empty() => pal.verde_fuerte,
            Some(_) => pal.tenue,
            None => pal.borde,
        };
        p.circle_filled(pos2(x_der - 4.0, fila.center().y + 10.0), 3.5, color_punto);
        // nombre y detalle
        let x = mini.right() + 12.0;
        let ancho_txt = x_der - 44.0 - x;
        texto_corto(p, pos2(x, fila.center().y - 9.0), Align2::LEFT_CENTER, &c.base(), if es_actual { f_negrita(13.5) } else { f_medio(13.5) }, if c.entrada.is_some() { pal.texto } else { pal.suave }, ancho_txt);
        let detalle = match &c.entrada {
            Some(_) => format!("{} · {} tramo{}", fmt_corto(c.dur()), c.tramos.len(), if c.tramos.len() == 1 { "" } else { "s" }),
            None if c.dur() > 0.0 => format!("{} · pendiente", fmt_corto(c.dur())),
            None => "pendiente".into(),
        };
        let rd = texto_corto(p, pos2(x, fila.center().y + 10.0), Align2::LEFT_CENTER, &detalle, f_texto(11.5), pal.tenue, ancho_txt);
        // los momentos del clip: puntos de color (el fijado por el editor, con borde)
        let fijado = self.tipos.get(&c.clave);
        let momentos: Vec<String> = match fijado {
            Some(m) => vec![m.clone()],
            None => self.cache_bodas.por_clip.get(&c.clave).cloned().unwrap_or_default(),
        };
        let mut xp = rd.right() + 8.0;
        for m in momentos.iter().take(3) {
            if xp > x + ancho_txt - 6.0 {
                break;
            }
            p.circle_filled(pos2(xp + 4.0, fila.center().y + 10.0), 4.5, color_momento(m));
            if fijado.is_some() {
                p.circle_stroke(pos2(xp + 4.0, fila.center().y + 10.0), 5.5, Stroke::new(1.2, pal.verde_fuerte));
            }
            xp += 12.0;
        }
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_ui(|ui| {
            let c = &self.clips[i];
            ui.label(egui::RichText::new(c.ruta.to_string_lossy()).font(f_texto(12.0)));
            ui.label(egui::RichText::new(self.estado_de(c)).font(f_texto(12.0)).color(pal.suave));
            if !momentos.is_empty() {
                ui.label(egui::RichText::new(format!("Momentos: {}", momentos.join(", "))).font(f_texto(12.0)).color(pal.suave));
            }
            if c.entrada.is_some() {
                ui.label(
                    egui::RichText::new(format!(
                        "{} tramos · puntuación {:.0}% · con sujeto {:.0}% · aprobado {}",
                        c.tramos.len(),
                        100.0 * c.media().unwrap_or(0.0),
                        100.0 * c.parte_joya(),
                        fmt_tiempo(c.aprobado)
                    ))
                    .font(f_texto(12.0))
                    .color(pal.suave),
                );
            }
            ui.label(egui::RichText::new("Ctrl+clic o Mayús+clic para elegir varios").font(f_texto(11.0)).color(pal.tenue));
        });
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            return Some((i, m.command, m.shift));
        }
        None
    }

    // =============================================================================================
    // Visor (centrado en las caras) y foco
    // =============================================================================================

    fn visor_bodas(&mut self, ui: &mut Ui, rect: Rect, a_foco: f32) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let interior = rect.shrink(16.0);
        let cab = Rect::from_min_size(interior.min, vec2(interior.width(), 36.0));
        let pie = Rect::from_min_max(pos2(interior.left(), interior.bottom() - 34.0), interior.max);
        let medio = Rect::from_min_max(pos2(interior.left(), cab.bottom() + 10.0), pos2(interior.right(), pie.top() - 10.0));
        // el foco necesita sitio: con el centro estrecho se pliega solo
        let w_foco = if medio.width() > 700.0 { ANCHO_FOCO * a_foco } else { 0.0 };
        let marco = Rect::from_min_max(medio.min, pos2(medio.right() - w_foco - 14.0 * (w_foco / ANCHO_FOCO), medio.bottom()));
        let foco = Rect::from_min_max(pos2(medio.right() - ANCHO_FOCO, medio.top()), medio.max);
        self.cabecera_visor(ui, cab);
        let cuadro = self.marco_visor(ui, marco);
        if w_foco > 20.0 {
            let mut h = hijo(ui, foco, "b_foco", a_foco, Rect::from_min_max(pos2(marco.right() + 4.0, medio.top() - 4.0), pos2(rect.right(), medio.bottom() + 4.0)));
            self.panel_foco(&mut h, foco, cuadro);
        }
        self.transporte_bodas(ui, pie);
    }

    /// Nombre del clip y sus datos; a la derecha, el momento de este clip y lo que se ve sobre el cuadro.
    fn cabecera_visor(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        let p = ui.painter().clone();
        let actual = self.actual.filter(|&i| i < self.clips.len());
        let (nombre, meta) = match actual.map(|i| &self.clips[i]) {
            Some(c) => {
                let m = &c.meta;
                let tc = c.entrada.as_ref().map(|e| e.tc_inicio.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
                    let (offs, _) = self.desplazamientos();
                    codigo_tiempo(offs.get(self.actual.unwrap_or(0)).copied().unwrap_or(0.0), self.cadencia())
                });
                let fps = m.fps.or(c.entrada.as_ref().map(|e| e.cadencia())).map(|f| format!("{f:.2} fps")).unwrap_or_default();
                let partes: Vec<String> = [m.res.clone().unwrap_or_default(), fps, format!("TC {tc}"), format!("aprobado {} de {}", fmt_corto(c.aprobado), fmt_corto(c.dur()))]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                (c.base(), partes.join(" · "))
            }
            None => ("Elige un clip".into(), "La lista de la izquierda tiene todo el material de la boda".into()),
        };
        // derecha: momento de este clip y los dos interruptores
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_cab_visor").max_rect(rect).layout(Layout::right_to_left(Align::Center)));
        h.spacing_mut().item_spacing.x = 10.0;
        let mut vc = self.ver_color;
        if interruptor(&mut h, &mut vc, "Color").changed() {
            self.ver_color = vc;
            self.texturas_mini.clear();
            self.redibujar_cuadro();
            self.limpiar_minis();
            self.marcar_sucio();
        }
        let mut vj = self.ver_joyas;
        if interruptor(&mut h, &mut vj, "Caras").changed() {
            self.ver_joyas = vj;
        }
        if let Some(i) = actual {
            let fijado = self.tipos.get(&self.clips[i].clave).cloned();
            let texto = match &fijado {
                Some(m) => format!("Momento: {m}"),
                None => "Momento: automático".into(),
            };
            let mut opciones: Vec<(String, Option<Color32>)> = vec![("Automático".into(), None)];
            opciones.extend(nucleo::bodas::MOMENTOS.iter().map(|m| (m.to_string(), Some(color_momento(m)))));
            if let Some(k) = desplegable(&mut h, Id::unique("b_momento_clip"), &texto, fijado.as_deref().map(color_momento), &opciones, 200.0) {
                self.poner_tipo(if k == 0 { None } else { Some(opciones[k].0.clone()) });
            }
        }
        let x_fin = h.min_rect().left() - 12.0;
        texto_corto(&p, pos2(rect.left(), rect.top() + 9.0), Align2::LEFT_CENTER, &nombre, f_titulo(19.0), pal.texto, x_fin - rect.left());
        texto_corto(&p, pos2(rect.left(), rect.top() + 28.0), Align2::LEFT_CENTER, &meta, f_texto(11.5), pal.tenue, x_fin - rect.left());
    }

    /// El cuadro, con las esquinas de los sujetos y las marcas de aprobado y momento. Devuelve el
    /// rectángulo del cuadro en pantalla (si hay).
    fn marco_visor(&mut self, ui: &mut Ui, rect: Rect) -> Option<Rect> {
        let pal = paleta();
        let p = ui.painter().clone();
        p.rect_filled(rect, 18, pal.visor);
        let pc = p.with_clip_rect(rect.shrink(1.0));
        let actual = self.actual;
        let mut cuadro = None;
        match (&self.tex_visor, actual) {
            (Some(tex), Some(_)) => {
                let [w, h] = self.tam_cuadro;
                let k = (rect.width() / w.max(1) as f32).min(rect.height() / h.max(1) as f32);
                let r = Rect::from_center_size(rect.center(), vec2(w as f32 * k, h as f32 * k));
                pc.add(egui::epaint::RectShape::filled(r, 14, Color32::WHITE).with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
                cuadro = Some(r);
            }
            _ => {
                crate::widgets::gradiente_radial(&pc, rect, rect.center(), vec2(rect.width() * 0.55, rect.height() * 0.6), &[(0.0, mezclar(pal.visor, pal.verde, 0.35)), (1.0, pal.visor)]);
                // la búsqueda no dio cuadro (sin conexión y sin proxy, error de ffmpeg…): se dice, en vez
                // de dejar la imagen de otro clip
                let texto = if self.clips.is_empty() {
                    "Agrega el material de la boda para empezar"
                } else if actual.is_some() && self.sin_cuadro {
                    "No se puede mostrar este cuadro"
                } else {
                    "Elige un clip de la lista"
                };
                let g = p.layout_no_wrap(texto.into(), f_titulo(22.0), pal.texto);
                let caja = Rect::from_center_size(rect.center(), g.size() + vec2(44.0, 22.0));
                pc.rect_filled(caja, 255, pal.tarjeta.gamma_multiply(0.92));
                pc.galley(caja.center() - g.size() / 2.0, g, pal.texto);
            }
        }
        if let (Some(r), Some(i)) = (cuadro, actual) {
            let c = &self.clips[i];
            if let Some(e) = &c.entrada {
                if let Some(m) = e.muestra_en(self.t) {
                    if self.ver_joyas {
                        let k = r.width() / e.ancho_analisis().max(1.0);
                        let caja = |j: &motor::JoyaVista| Rect::from_min_max(r.min + vec2(j.caja[0] * k, j.caja[1] * k), r.min + vec2(j.caja[2] * k, j.caja[3] * k));
                        // Zonas ya ocupadas por las píldoras de "APROBADO" y del momento: ninguna etiqueta las
                        // pisa, ni se pisan entre sí (si no hay sitio, la etiqueta menos importante no se dibuja).
                        let mut ocupado: Vec<Rect> = Vec::new();
                        if c.tramos.iter().any(|s| s.inicio <= self.t && self.t < s.fin) {
                            ocupado.push(Rect::from_min_size(r.min + vec2(14.0, 14.0), vec2(124.0, 28.0)));
                        }
                        if !m.momento.is_empty() {
                            let g = p.layout_no_wrap(m.momento.clone(), f_negrita(12.5), TINTA_MOMENTO);
                            let tam = g.size() + vec2(30.0, 12.0);
                            let esquina = if r.width() >= 320.0 { pos2(r.right() - 16.0 - tam.x, r.top() + 14.0) } else { pos2(r.left() + 14.0, r.top() + 50.0) };
                            ocupado.push(Rect::from_min_size(esquina, tam));
                        }
                        // primer hueco libre entre las posiciones candidatas (encima, debajo, dentro arriba)
                        let colocar = |b: Rect, tam: egui::Vec2, ocupado: &[Rect]| -> Option<Rect> {
                            let x = b.left().clamp(r.left() + 4.0, (r.right() - 4.0 - tam.x).max(r.left() + 4.0));
                            [b.top() - tam.y - 6.0, b.bottom() + 6.0, b.top() + 6.0]
                                .into_iter()
                                .map(|y| Rect::from_min_size(pos2(x, y), tam))
                                .find(|q| r.contains_rect(*q) && !ocupado.iter().any(|o| o.expand(3.0).intersects(*q)))
                        };
                        let puntero = ui.ctx().pointer_hover_pos().filter(|pp| r.contains(*pp));
                        // 1) los demás: caras con esquinas suaves, el resto con un contorno tenue; sin texto
                        let mut bajo_puntero: Option<&motor::JoyaVista> = None;
                        for j in m.joyas.iter().filter(|j| !j.principal && j.conf >= 0.25) {
                            let b = caja(j);
                            if j.propia == "cara" {
                                esquinas(&pc, b, Color32::from_white_alpha(110), 1.2);
                            } else {
                                pc.rect_stroke(b, 6, Stroke::new(1.0, Color32::from_white_alpha(55)), StrokeKind::Middle);
                            }
                            if puntero.map(|pp| b.contains(pp)).unwrap_or(false) && bajo_puntero.map(|o| o.conf < j.conf).unwrap_or(true) {
                                bajo_puntero = Some(j);
                            }
                        }
                        // 2) el sujeto principal: esquinas verdes y su etiqueta (SU nombre y SU confianza)
                        if let Some(j) = m.joyas.iter().find(|j| j.principal) {
                            let b = caja(j);
                            esquinas(&pc, b, pal.verde, 3.0);
                            let g = p.layout_no_wrap(format!("{} {:.2}", j.propia, j.conf), f_negrita(12.5), pal.sobre_verde);
                            if let Some(pill) = colocar(b, g.size() + vec2(22.0, 10.0), &ocupado) {
                                pc.rect_filled(pill, 255, pal.verde);
                                pc.galley(pill.center() - g.size() / 2.0, g, pal.sobre_verde);
                                ocupado.push(pill);
                            }
                        }
                        // 3) al pasar el ratón por otra caja: su etiqueta, si cabe sin pisar nada
                        if let Some(j) = bajo_puntero {
                            let b = caja(j);
                            pc.rect_stroke(b, 6, Stroke::new(1.5, Color32::from_white_alpha(170)), StrokeKind::Middle);
                            let g = p.layout_no_wrap(format!("{} {:.2}", j.propia, j.conf), f_medio(11.5), Color32::WHITE);
                            if let Some(pill) = colocar(b, g.size() + vec2(16.0, 8.0), &ocupado) {
                                pc.rect_filled(pill, 255, Color32::from_black_alpha(185));
                                pc.galley(pill.center() - g.size() / 2.0, g, Color32::WHITE);
                            }
                        }
                    }
                    // el momento de este instante: arriba a la derecha (o bajo "APROBADO" si el cuadro es estrecho)
                    if !m.momento.is_empty() {
                        let g = p.layout_no_wrap(m.momento.clone(), f_negrita(12.5), TINTA_MOMENTO);
                        let tam = g.size() + vec2(30.0, 12.0);
                        let esquina = if r.width() >= 320.0 { pos2(r.right() - 16.0 - tam.x, r.top() + 14.0) } else { pos2(r.left() + 14.0, r.top() + 50.0) };
                        let pill = Rect::from_min_size(esquina, tam);
                        pc.rect_filled(pill, 255, color_momento(&m.momento));
                        pc.circle_filled(pos2(pill.left() + 12.0, pill.center().y), 3.5, TINTA_MOMENTO.gamma_multiply(0.6));
                        pc.galley(pos2(pill.left() + 21.0, pill.center().y - g.size().y / 2.0), g, TINTA_MOMENTO);
                    }
                }
            }
            // arriba a la izquierda: "APROBADO" dentro de un tramo
            let dentro = c.tramos.iter().any(|s| s.inicio <= self.t && self.t < s.fin);
            let a = anim(ui, Id::unique("b_aprobado"), dentro, 0.2);
            if a > 0.01 {
                let g = p.layout_no_wrap("APROBADO".into(), f_negrita(12.0), pal.sobre_verde);
                let pill = Rect::from_min_size(r.min + vec2(14.0, 14.0), g.size() + vec2(36.0, 12.0));
                let mut pa = pc.clone();
                pa.multiply_opacity(a);
                pa.rect_filled(pill, 255, pal.verde);
                pa.circle_filled(pos2(pill.left() + 13.0, pill.center().y), 4.0, pal.sobre_verde);
                pa.galley(pos2(pill.left() + 23.0, pill.center().y - g.size().y / 2.0), g, pal.sobre_verde);
            }
        }
        // clic en el cuadro: reproducir / pausar
        if ui.interact(rect, Id::unique("b_visor_clic"), Sense::click()).clicked() {
            self.alternar_reproduccion();
        }
        cuadro
    }

    /// Foco: la cara (o el sujeto) principal ampliada, el momento y los medidores de este instante.
    fn panel_foco(&mut self, ui: &mut Ui, rect: Rect, cuadro: Option<Rect>) {
        let pal = paleta();
        let p = ui.painter().clone();
        p.rect_filled(rect, 18, pal.hundido);
        let x0 = rect.left() + 14.0;
        let ancho = rect.width() - 28.0;
        p.text(pos2(x0, rect.top() + 20.0), Align2::LEFT_CENTER, "Foco", f_titulo(19.0), pal.texto);
        p.text(pos2(rect.right() - 14.0, rect.top() + 21.0), Align2::RIGHT_CENTER, "sujeto principal", f_texto(11.5), pal.tenue);
        let lado = ancho.min(rect.height() - 230.0).max(60.0);
        let rec = Rect::from_min_size(pos2(x0 + (ancho - lado) / 2.0, rect.top() + 40.0), vec2(lado, lado));
        let actual = self.actual.filter(|&i| i < self.clips.len());
        let e = actual.and_then(|i| self.clips[i].entrada.as_ref());
        let idx = e.map(|e| ((self.t * e.por_segundo).round().max(0.0) as usize).min(e.muestras.len().saturating_sub(1)));
        let m = e.and_then(|e| e.muestra_en(self.t));
        let principal = m.and_then(|m| m.joyas.iter().find(|j| j.principal));
        // recorte ampliado de la textura del visor (sin decodificar nada más)
        match (&self.tex_visor, e, principal, cuadro) {
            (Some(tex), Some(e), Some(j), Some(_)) => {
                let aw = e.ancho_analisis().max(1.0);
                let ah = (e.alto as f32 * aw / e.ancho.max(1) as f32).max(1.0);
                let (cx, cy) = ((j.caja[0] + j.caja[2]) / 2.0, (j.caja[1] + j.caja[3]) / 2.0);
                let s = ((j.caja[2] - j.caja[0]).max(j.caja[3] - j.caja[1]) * 1.35).max(24.0).min(aw.min(ah));
                let x = (cx - s / 2.0).clamp(0.0, aw - s);
                let y = (cy - s / 2.0).clamp(0.0, ah - s);
                let uv = Rect::from_min_max(pos2(x / aw, y / ah), pos2((x + s) / aw, (y + s) / ah));
                p.add(egui::epaint::RectShape::filled(rec, 16, Color32::WHITE).with_texture(tex.id(), uv));
                esquinas(&p, rec.shrink(10.0), pal.verde, 2.5);
            }
            _ => {
                p.rect_filled(rec, 16, pal.tarjeta);
                let texto = match actual {
                    Some(_) if self.sin_cuadro => "Sin cuadro",
                    Some(_) => "Sin sujeto en este cuadro",
                    None => "—",
                };
                p.text(rec.center(), Align2::CENTER_CENTER, texto, f_texto(12.0), pal.tenue);
            }
        }
        let mut y = rec.bottom() + 14.0;
        if let Some(j) = principal {
            texto_corto(&p, pos2(x0, y + 6.0), Align2::LEFT_CENTER, &format!("{} · confianza {:.2}", j.propia, j.conf), f_medio(12.5), pal.texto, ancho);
            y += 20.0;
        }
        if let Some(m) = m.filter(|m| !m.momento.is_empty()) {
            let g = p.layout_no_wrap(m.momento.clone(), f_negrita(12.5), TINTA_MOMENTO);
            let pill = Rect::from_min_size(pos2(x0, y), g.size() + vec2(30.0, 12.0));
            p.rect_filled(pill, 255, color_momento(&m.momento));
            p.galley(pos2(pill.left() + 15.0, pill.center().y - g.size().y / 2.0), g, TINTA_MOMENTO);
            y += pill.height() + 10.0;
        }
        // medidores con los criterios de la edición en este instante
        if let (Some(e), Some(k)) = (e, idx) {
            if let Some(s) = e.nucleo.samples.get(k) {
                let preset = preset_bodas();
                let c = nucleo::bodas::components(s, &preset);
                let val = |n: &str| c.iter().find(|x| x.0 == n).map(|x| x.1).unwrap_or(0.0);
                let punt = actual.and_then(|i| self.clips[i].curva.get(k)).map(|x| x.1).unwrap_or(0.0);
                for (nombre, v) in [("Nitidez", if s.jewel { val("sharp") } else { 0.0 }), ("Exposición", if s.jewel { val("exposure") } else { 0.0 }), ("Cámara estable", val("steady")), ("Puntuación", punt)] {
                    if y + 26.0 > rect.bottom() - 8.0 {
                        break;
                    }
                    medidor(&p, Rect::from_min_size(pos2(x0, y), vec2(ancho, 26.0)), nombre, v, &pal);
                    y += 32.0;
                }
            }
        }
    }

    /// Transporte bajo el cuadro: reproducir, tiempo, saltos, barra con los tramos (del color de su
    /// momento) y la conversión de color del clip.
    fn transporte_bodas(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_transporte").max_rect(rect).layout(Layout::left_to_right(Align::Center)));
        // proporciones finas (como el transporte de joyas): botón de 34 px, saltos de 26, textos de 13
        h.spacing_mut().item_spacing.x = 6.0;
        let icono = if self.reproduciendo { Icono::Pausa } else { Icono::Reproducir };
        if boton_icono(&mut h, icono, 34.0, true, "Reproducir / pausa (Espacio)").clicked() {
            self.alternar_reproduccion();
        }
        h.add_space(2.0);
        h.label(egui::RichText::new(fmt_corto(self.t)).font(f_negrita(13.0)).color(pal.texto));
        if boton_icono(&mut h, Icono::Inicio, 26.0, false, "Al inicio (Inicio)").clicked() {
            self.buscar(0.0);
        }
        if boton_icono(&mut h, Icono::Atras, 26.0, false, "−2 s (←)").clicked() {
            self.buscar(self.t - 2.0);
        }
        if boton_icono(&mut h, Icono::Adelante, 26.0, false, "+2 s (→)").clicked() {
            self.buscar(self.t + 2.0);
        }
        if boton_icono(&mut h, Icono::SiguienteTramo, 26.0, false, "Siguiente tramo aprobado (N)").clicked() {
            self.siguiente_tramo();
        }
        let (dur, tramos) = match self.actual.and_then(|i| self.clips.get(i)) {
            Some(c) => (c.dur(), c.tramos.iter().map(|s| (s.inicio, s.fin, color_momento(&s.momento))).collect::<Vec<_>>()),
            None => (0.0, Vec::new()),
        };
        // la conversión que de verdad aplica el visor (con "auto", el perfil de la cámara si se conoce)
        let color_txt = self.texto_color_actual("Color automático");
        let g_col = h.painter().layout_no_wrap(color_txt.clone(), f_medio(13.0), pal.texto);
        let g_dur = h.painter().layout_no_wrap(fmt_corto(dur), f_texto(13.0), pal.suave);
        let ancho_barra = (h.available_width() - g_col.size().x - 26.0 - g_dur.size().x - 3.0 * 6.0 - 6.0).max(40.0);
        let cuadro_s = self.duracion_cuadro();
        if let Some(t) = barra_avance(&mut h, ancho_barra, self.t, dur, &tramos, cuadro_s) {
            self.buscar(t);
        }
        h.label(egui::RichText::new(fmt_corto(dur)).font(f_texto(13.0)).color(pal.suave));
        if boton_fino(&mut h, &color_txt).on_hover_text("Conversión de color del clip (panel Color, Ctrl+3)").clicked() {
            self.paneles.color = !self.paneles.color;
            self.marcar_sucio();
        }
    }

    // =============================================================================================
    // Columna del criterio
    // =============================================================================================

    fn columna_criterio(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let interior = rect.shrink(18.0);
        let compacto = rect.height() < 420.0;
        let alto_exportar = if compacto { 58.0 } else { 92.0 };
        let arriba = Rect::from_min_max(interior.min, pos2(interior.right(), interior.bottom() - alto_exportar));
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_criterio_in").max_rect(arriba).layout(Layout::top_down(Align::Min)));
        h.set_clip_rect(arriba.expand(4.0).intersect(ui.clip_rect()));
        h.spacing_mut().item_spacing.y = 12.0;
        h.horizontal(|ui| {
            ui.label(egui::RichText::new("Criterio").font(f_titulo(24.0)).color(pal.texto));
        });
        let opciones = ["Equilibrado", "Highlight", "Permisivo"];
        let crit = motor::criterios();
        let sel = crit.iter().position(|c| c.0 == self.criterio.nombre).unwrap_or(0);
        if let Some(i) = segmentado(&mut h, Id::unique("b_criterio"), &opciones, sel) {
            self.aplicar_criterio(crit[i].0);
        }
        let mut cambio = false;
        let salida = egui::ScrollArea::vertical().id_salt("b_criterio_scroll").auto_shrink([false, false]).show(&mut h, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.set_width(ui.available_width() - 6.0);
            let titulo = |ui: &mut Ui, t: &str, sub: &str| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(t).font(f_titulo(19.0)).color(pal.texto));
                if !sub.is_empty() {
                    ui.label(egui::RichText::new(sub).font(f_texto(12.0)).color(pal.tenue));
                }
            };
            titulo(ui, "Pesos", "La nitidez de las caras manda: en 0 se ignora.");
            for (k, nombre) in motor::pesos() {
                let mut v = self.criterio.pesos.get(*k).copied().unwrap_or(0.0);
                let max = if *k == "sharp" { 1.5 } else { 1.0 };
                if deslizador(ui, nombre, color_peso(k), &mut v, 0.0, max, 0.05, &|x| format!("{x:.2}")) {
                    self.criterio.pesos.insert(k.to_string(), v);
                    cambio = true;
                }
            }
            titulo(ui, "Tramos", "");
            let c = &mut self.criterio;
            let dos = |x: f64| format!("{x:.2}");
            let seg2 = |x: f64| format!("{x:.2} s");
            let seg0 = |x: f64| format!("{x:.0} s");
            let entero = |x: f64| format!("{x:.0}");
            let v = pal.verde;
            cambio |= deslizador(ui, "Puntuación mínima", v, &mut c.umbral, 0.1, 0.95, 0.01, &dos);
            cambio |= deslizador(ui, "Duración mínima del tramo", v, &mut c.min_s, 0.25, 10.0, 0.25, &seg2);
            cambio |= deslizador(ui, "Duración máxima del tramo", v, &mut c.max_s, 1.0, 120.0, 1.0, &seg0);
            let mut por = c.por_clip as f64;
            if deslizador(ui, "Tramos máximos por clip", v, &mut por, 1.0, 99.0, 1.0, &entero) {
                c.por_clip = por.round() as u32;
                cambio = true;
            }
            cambio |= deslizador(ui, "Tolerancia a caídas", v, &mut c.tolerancia, 0.0, 4.0, 0.25, &seg2);
            cambio |= deslizador(ui, "Consistencia (suavizado)", v, &mut c.suavizado, 0.0, 4.0, 0.25, &seg2);
            titulo(ui, "Momentos", "Ninguno marcado = todos");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for m in nucleo::bodas::MOMENTOS {
                    let activa = self.criterio.joyas.iter().any(|x| x == m);
                    if chip(ui, m, Some(color_momento(m)), activa).clicked() {
                        if activa { self.criterio.joyas.retain(|x| x != m); } else { self.criterio.joyas.push(m.to_owned()); }
                        cambio = true;
                    }
                }
            });
            titulo(ui, "Planos", "Ninguno marcado = todos");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (k, nombre) in motor::planos() {
                    let activa = self.criterio.planos.iter().any(|x| x == k);
                    if chip(ui, nombre, None, activa).clicked() {
                        if activa { self.criterio.planos.retain(|x| x != k); } else { self.criterio.planos.push(k.to_string()); }
                        cambio = true;
                    }
                }
            });
            titulo(ui, "Exportar a DaVinci", "");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("fps de tu línea de tiempo").font(f_texto(13.0)).color(pal.suave));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let opciones: Vec<(String, Option<Color32>)> = crate::app::FPS.iter().map(|s| (s.to_string(), None)).collect();
                    if let Some(i) = desplegable(ui, Id::unique("b_dd_fps"), &self.fps.clone(), None, &opciones, 100.0) {
                        self.fps = opciones[i].0.clone();
                        self.marcar_sucio();
                    }
                });
            });
            for (k, nombre) in motor::MODOS_EXPORTAR {
                if item_menu(ui, nombre, self.modo == k, None).clicked() {
                    self.modo = k.into();
                    self.marcar_sucio();
                }
            }
            ui.horizontal(|ui| {
                if boton(ui, "Restablecer", None, Tipo::Suave, 30.0, true).on_hover_text("Vuelve a los valores del criterio").clicked() {
                    let n = self.criterio.nombre.clone();
                    self.aplicar_criterio(&n);
                }
                if boton(ui, "Resultados", Some(Icono::Carpeta), Tipo::Suave, 30.0, !self.out.is_empty()).clicked() {
                    self.abrir_carpeta_resultados();
                }
            });
            ui.add_space(8.0);
        });
        desvanecer_scroll(ui.painter(), &salida, pal.tarjeta);
        if cambio {
            self.programar();
        }
        // abajo: resumen y el gran botón de exportar
        let abajo = Rect::from_min_max(pos2(interior.left(), interior.bottom() - alto_exportar + 8.0), interior.max);
        let p = ui.painter();
        if !compacto {
            texto_corto(p, pos2(abajo.left() + 4.0, abajo.top() + 6.0), Align2::LEFT_CENTER, &self.resumen, f_texto(12.5), pal.suave, abajo.width() - 8.0);
        }
        let pill = Rect::from_min_max(pos2(abajo.left(), abajo.bottom() - 50.0), abajo.max);
        let resp = ui.interact(pill, Id::unique("b_exportar"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Exportar la línea de tiempo (OTIO + CSV) · Ctrl+E");
        let hv = anim(ui, resp.id.with("h"), resp.hovered(), 0.15);
        let pul = anim(ui, resp.id.with("p"), resp.is_pointer_button_down_on(), 0.08);
        let r = pill.shrink(pul * 1.5);
        p.rect_filled(r, 255, mezclar(pal.verde, pal.verde_fuerte, 0.2 * hv));
        let g = p.layout_no_wrap("Exportar a DaVinci".into(), f_negrita(15.0), pal.sobre_verde);
        p.galley(pos2(r.left() + 22.0, r.center().y - g.size().y / 2.0), g, pal.sobre_verde);
        let c = pos2(r.right() - 25.0 + 4.0 * hv, r.center().y);
        p.circle_filled(c, 15.0, pal.sobre_verde.gamma_multiply(0.12));
        iconos::dibujar(p, Icono::Adelante, Rect::from_center_size(c, vec2(16.0, 16.0)), pal.sobre_verde, 2.2);
        if resp.clicked() {
            self.exportar();
        }
    }

    // =============================================================================================
    // Momentos: línea de tiempo del material con un carril por momento
    // =============================================================================================

    fn carriles(&mut self, ui: &mut Ui, rect: Rect, max_alto: f32) {
        let pal = paleta();
        tarjeta(ui.painter(), rect, &pal, 1.0);
        let p = ui.painter().clone();
        // asa de arriba: arrastrar cambia el alto
        let asa = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 14.0));
        let r_asa = ui.interact(asa, Id::unique("b_asa_tiempo"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        if r_asa.dragged() {
            self.alto_tiempo = (self.alto_tiempo - r_asa.drag_delta().y).clamp(130.0, max_alto);
        }
        if r_asa.drag_stopped() {
            self.marcar_sucio();
        }
        let ha = anim(ui, r_asa.id.with("h"), r_asa.hovered() || r_asa.dragged(), 0.15);
        p.rect_filled(Rect::from_center_size(pos2(rect.center().x, rect.top() + 8.0), vec2(64.0 + 16.0 * ha, 5.0)), 3, mezclar(pal.borde, pal.verde_fuerte, ha));
        // cabecera
        let y_cab = rect.top() + 16.0;
        let (x0, x1) = (rect.left() + 18.0, rect.right() - 18.0);
        let r_titulo = p.text(pos2(x0, y_cab + 14.0), Align2::LEFT_CENTER, "Momentos", f_titulo(21.0), pal.texto);
        let mut h = ui.new_child(UiBuilder::new().id_salt("b_cab_tiempo").max_rect(Rect::from_min_max(pos2(x0, y_cab), pos2(x1, y_cab + 30.0))).layout(Layout::right_to_left(Align::Center)));
        h.spacing_mut().item_spacing.x = 8.0;
        let modo_txt = match self.modo.as_str() { "v1v2" => "V2 aprobados · V1 resto", "aprobados" => "Solo aprobados", _ => "V1 con marcadores" };
        if boton(&mut h, modo_txt, None, Tipo::Suave, 30.0, true).on_hover_text("Modo de exportación (clic para cambiar)").clicked() {
            let i = motor::MODOS_EXPORTAR.iter().position(|m| m.0 == self.modo).unwrap_or(0);
            self.modo = motor::MODOS_EXPORTAR[(i + 1) % motor::MODOS_EXPORTAR.len()].0.to_owned();
            self.marcar_sucio();
        }
        if boton(&mut h, "Clip", Some(Icono::Lupa), Tipo::Contorno, 30.0, self.actual.is_some()).on_hover_text("Acercar al clip actual").clicked() {
            self.enfocar_clip();
        }
        if boton(&mut h, "Ajustar", Some(Icono::Ajustar), Tipo::Contorno, 30.0, true).on_hover_text("Ver todo el material").clicked() {
            self.ajustar_mapa();
        }
        if self.zoom_mapa > 1.01 {
            h.label(egui::RichText::new(format!("×{:.1}", self.zoom_mapa)).font(f_medio(13.0)).color(pal.suave));
        }
        let hueco = h.min_rect().left() - 14.0 - (r_titulo.right() + 12.0);
        if hueco > 60.0 {
            texto_corto(&p, pos2(r_titulo.right() + 12.0, y_cab + 15.0), Align2::LEFT_CENTER, "un carril por momento · rueda = zoom · Mayús+rueda = mover · arrastra el borde para subirla", f_texto(12.5), pal.tenue, hueco);
        }
        let area = Rect::from_min_max(pos2(x0, y_cab + 38.0), pos2(x1, rect.bottom() - 14.0));
        if area.height() < 30.0 {
            return;
        }
        p.rect_filled(area, 16, pal.hundido);
        self.pistas_momentos(ui, area);
    }

    fn pistas_momentos(&mut self, ui: &mut Ui, area: Rect) {
        let pal = paleta();
        let p = ui.painter().with_clip_rect(area.shrink(1.0));
        if self.clips.is_empty() {
            p.text(area.center(), Align2::CENTER_CENTER, "Los momentos de la boda aparecen al analizar el material", f_texto(13.0), pal.tenue);
            return;
        }
        let pistas = Rect::from_min_max(pos2(area.left() + ANCHO_ETIQUETAS, area.top() + 6.0), pos2(area.right() - 12.0, area.bottom() - 6.0));
        let ancho = pistas.width().max(1.0);
        let resp = ui.interact(pistas, Id::unique("b_pistas"), Sense::click_and_drag());
        let (_, total, mut t0, mut t1) = self.ventana_mapa();
        if resp.hovered() {
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
                    self.t0_mapa -= (t1 - t0) * 0.1 * delta.signum() as f64;
                } else if let Some(pp) = puntero {
                    let fx = ((pp.x - pistas.left()) / ancho).clamp(0.0, 1.0) as f64;
                    let ancla = t0 + fx * (t1 - t0);
                    self.zoom_mapa = (self.zoom_mapa * if delta > 0.0 { 1.25 } else { 0.8 }).clamp(1.0, 200.0);
                    self.t0_mapa = ancla - fx * total / self.zoom_mapa;
                }
                self.marcar_sucio();
                let v = self.ventana_mapa();
                t0 = v.2;
                t1 = v.3;
            }
        }
        let span = (t1 - t0).max(1e-6);
        let x_de = |gt: f64| pistas.left() + ((gt - t0) / span) as f32 * ancho;
        let t_de = |x: f32| t0 + ((x - pistas.left()) / ancho).clamp(0.0, 1.0) as f64 * span;
        if (resp.clicked() || resp.dragged()) && resp.interact_pointer_pos().is_some() {
            let x = resp.interact_pointer_pos().unwrap().x;
            self.ir_a_global(t_de(x));
        }

        // filas: regla, clips, un carril por momento, los aprobados y la puntuación (lo que sobre)
        let n_carriles = self.cache_bodas.carriles.len();
        let disponible = (pistas.height() - 18.0 - 22.0 - 4.0).max(10.0);
        let filas = n_carriles as f32 + 1.0;
        let reserva = if disponible > filas * 12.0 + 30.0 { 30.0 } else { 0.0 };
        let alto_carril = ((disponible - reserva) / filas).clamp(8.0, 30.0);
        let mut y = pistas.top();
        // regla
        let paso = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0].into_iter().find(|s| s * ancho as f64 / span >= 110.0).unwrap_or(3600.0);
        let mut tt = (t0 / paso).ceil() * paso;
        while tt <= t1 {
            let x = x_de(tt);
            p.line_segment([pos2(x, y + 12.0), pos2(x, pistas.bottom())], Stroke::new(1.0, pal.borde.gamma_multiply(0.6)));
            p.text(pos2(x + 3.0, y + 5.0), Align2::LEFT_CENTER, codigo_tiempo(tt, self.cadencia()), f_texto(10.5), pal.tenue);
            tt += paso;
        }
        y += 18.0;
        // clips: bloques alternos con el nombre si cabe
        let (offs, _) = self.desplazamientos();
        let fila_clips = Rect::from_min_size(pos2(pistas.left(), y), vec2(ancho, 16.0));
        p.text(pos2(area.left() + 14.0, fila_clips.center().y), Align2::LEFT_CENTER, "Clips", f_medio(11.5), pal.suave);
        for (i, c) in self.clips.iter().enumerate() {
            let (a, b) = (offs[i], offs[i] + c.dur());
            if b < t0 || a > t1 {
                continue;
            }
            let r = Rect::from_min_max(pos2(x_de(a).max(pistas.left()), fila_clips.top()), pos2(x_de(b).min(pistas.right()), fila_clips.bottom()));
            let actual = self.actual == Some(i);
            let col = if actual { pal.verde } else if i % 2 == 0 { pal.tarjeta } else { mezclar(pal.tarjeta, pal.borde, 0.5) };
            p.rect_filled(r.shrink2(vec2(0.5, 0.0)), 5, col);
            if r.width() > 44.0 {
                texto_corto(&p, pos2(r.left() + 5.0, r.center().y), Align2::LEFT_CENTER, &c.base(), f_texto(10.5), if actual { pal.sobre_verde } else { pal.suave }, r.width() - 8.0);
            }
        }
        y += 22.0;
        // carriles por momento
        for (m, tramos) in &self.cache_bodas.carriles {
            let fila = Rect::from_min_size(pos2(pistas.left(), y), vec2(ancho, alto_carril - 3.0));
            let color = color_momento(m);
            p.rect_filled(Rect::from_min_size(pos2(area.left() + 8.0, fila.top()), vec2(area.width() - 16.0, fila.height())), 4, color.gamma_multiply(0.10));
            p.circle_filled(pos2(area.left() + 18.0, fila.center().y), (alto_carril * 0.22).clamp(2.5, 4.5), color);
            texto_corto(&p, pos2(area.left() + 28.0, fila.center().y), Align2::LEFT_CENTER, m, f_texto((alto_carril * 0.62).clamp(8.5, 12.5)), pal.texto, ANCHO_ETIQUETAS - 34.0);
            // tramos fusionados por píxel (miles de muestras se dibujan como pocos rectángulos)
            let mut abierto: Option<(f32, f32)> = None;
            for &(a, b) in tramos {
                if b < t0 || a > t1 {
                    continue;
                }
                let (xa, xb) = (x_de(a).max(pistas.left()), x_de(b).min(pistas.right()));
                match abierto {
                    Some((sa, sb)) if xa <= sb + 1.0 => abierto = Some((sa, sb.max(xb))),
                    Some((sa, sb)) => {
                        p.rect_filled(Rect::from_min_max(pos2(sa, fila.top()), pos2(sb.max(sa + 1.5), fila.bottom())), 3, color);
                        abierto = Some((xa, xb));
                    }
                    None => abierto = Some((xa, xb)),
                }
            }
            if let Some((sa, sb)) = abierto {
                p.rect_filled(Rect::from_min_max(pos2(sa, fila.top()), pos2(sb.max(sa + 1.5), fila.bottom())), 3, color);
            }
            y += alto_carril;
        }
        // aprobados: cada tramo con el color de su momento y un borde más intenso
        let fila = Rect::from_min_size(pos2(pistas.left(), y + 2.0), vec2(ancho, (alto_carril + 2.0).max(12.0)));
        p.text(pos2(area.left() + 14.0, fila.center().y), Align2::LEFT_CENTER, "Aprobados", f_medio(11.5), pal.verde_fuerte);
        for (i, c) in self.clips.iter().enumerate() {
            for s in &c.tramos {
                let (a, b) = (offs[i] + s.inicio, offs[i] + s.fin);
                if b < t0 || a > t1 {
                    continue;
                }
                let r = Rect::from_min_max(pos2(x_de(a).max(pistas.left()), fila.top()), pos2(x_de(b).min(pistas.right()).max(x_de(a) + 2.0), fila.bottom()));
                p.rect_filled(r, 4, color_momento(&s.momento));
                p.rect_stroke(r, 4, Stroke::new(1.0, pal.verde_fuerte.gamma_multiply(0.7)), StrokeKind::Inside);
            }
        }
        // puntuación de todo el material (lo que sobra de alto): área verde con la línea del umbral
        let mut y_fin = fila.bottom();
        let alto_punt = pistas.bottom() - fila.bottom() - 8.0;
        if alto_punt >= 18.0 {
            let fp = Rect::from_min_max(pos2(pistas.left(), fila.bottom() + 8.0), pos2(pistas.right(), pistas.bottom()));
            p.text(pos2(area.left() + 14.0, fp.center().y), Align2::LEFT_CENTER, "Puntuación", f_medio(11.5), pal.suave);
            let n = fp.width().max(1.0) as usize;
            let mut col = vec![-1.0f64; n];
            for (i, c) in self.clips.iter().enumerate() {
                for &(t, s) in &c.curva {
                    let gt = offs[i] + t;
                    if gt < t0 || gt > t1 {
                        continue;
                    }
                    let k = ((x_de(gt) - fp.left()) as usize).min(n - 1);
                    col[k] = col[k].max(s);
                }
            }
            let umbral = self.criterio.umbral;
            for (k, &s) in col.iter().enumerate() {
                if s < 0.0 {
                    continue;
                }
                let alto = (fp.height() * s as f32).max(1.0);
                let c = if s >= umbral { pal.verde } else { mezclar(pal.borde, pal.rojo, (1.0 - s as f32 / umbral.max(0.01) as f32).clamp(0.0, 1.0) * 0.5) };
                p.rect_filled(Rect::from_min_max(pos2(fp.left() + k as f32, fp.bottom() - alto), pos2(fp.left() + k as f32 + 1.0, fp.bottom())), 0, c);
            }
            let yu = fp.bottom() - fp.height() * umbral as f32;
            let mut x = fp.left();
            while x < fp.right() {
                p.line_segment([pos2(x, yu), pos2((x + 5.0).min(fp.right()), yu)], Stroke::new(1.0, pal.verde_fuerte.gamma_multiply(0.7)));
                x += 9.0;
            }
            y_fin = fp.bottom();
        }
        // cabezal
        if let Some(i) = self.actual {
            let gt = offs.get(i).copied().unwrap_or(0.0) + self.t;
            if gt >= t0 && gt <= t1 {
                let x = x_de(gt);
                p.line_segment([pos2(x, pistas.top()), pos2(x, y_fin)], Stroke::new(2.0, pal.verde_fuerte));
                p.circle_filled(pos2(x, pistas.top() + 3.0), 4.5, pal.verde_fuerte);
            }
        }
        if let Some(pp) = resp.hover_pos() {
            let gt = t_de(pp.x);
            resp.on_hover_text_at_pointer(codigo_tiempo(gt, self.cadencia()));
        }
    }

    // =============================================================================================
    // Avisos
    // =============================================================================================

    fn aviso_bodas(&mut self, ui: &mut Ui) {
        let Some(av) = &self.aviso else { return };
        let pal = paleta();
        let (titulo, texto, salir) = (av.titulo.clone(), av.texto.clone(), av.confirmar_salida);
        let (mut cerrar, mut salir_ya) = (false, false);
        let marco = egui::Frame::new().fill(pal.tarjeta).corner_radius(24).inner_margin(26).shadow(bt::sombra(&pal)).stroke(Stroke::new(1.0, pal.borde));
        egui::Modal::new(Id::unique("b_aviso")).frame(marco).backdrop_color(Color32::from_black_alpha(if pal.claro { 70 } else { 140 })).show(ui.ctx(), |ui| {
            ui.set_max_width(460.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(egui::RichText::new(&titulo).font(f_titulo(24.0)).color(pal.texto));
            ui.label(egui::RichText::new(&texto).font(f_texto(14.0)).color(pal.texto));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if salir {
                    if boton(ui, "Detener y salir", None, Tipo::Principal, 38.0, true).clicked() {
                        salir_ya = true;
                    }
                    if boton(ui, "Seguir", None, Tipo::Contorno, 38.0, true).clicked() {
                        cerrar = true;
                    }
                } else if boton(ui, "Entendido", None, Tipo::Principal, 38.0, true).clicked() {
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

/// Barra de avance del transporte: los tramos aprobados con el color de su momento y la perilla.
/// Devuelve el instante solo cuando hay que buscar de verdad (al pulsar, o al arrastrar a otro sitio:
/// `widgets::filtrar_busqueda`); `cuadro_s` es la duración de un cuadro del clip.
fn barra_avance(ui: &mut Ui, ancho: f32, t: f64, dur: f64, tramos: &[(f64, f64, Color32)], cuadro_s: f64) -> Option<f64> {
    let pal = paleta();
    // zona de clic de 20 px y línea de 4 px (6 al pasar el ratón): fina pero fácil de agarrar
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 20.0), Sense::click_and_drag());
    let hv = anim(ui, resp.id.with("h"), resp.hovered() || resp.dragged(), 0.12);
    let riel = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0 + 2.0 * hv));
    // mantener el botón quieto sobre la barra no vuelve a buscar en cada repintado
    let pulsando = resp
        .interact_pointer_pos()
        .filter(|_| (resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked()) && dur > 0.0)
        .map(|pp| (pp.x, ((pp.x - riel.left()) / riel.width()).clamp(0.0, 1.0) as f64 * dur));
    let nuevo = crate::widgets::filtrar_busqueda(ui, resp.id, pulsando, cuadro_s);
    let p = ui.painter();
    p.rect_filled(riel, 3, pal.hundido);
    let x_de = |s: f64| riel.left() + if dur > 0.0 { ((s / dur).clamp(0.0, 1.0)) as f32 * riel.width() } else { 0.0 };
    for &(a, b, c) in tramos {
        let r = Rect::from_min_max(pos2(x_de(a), riel.top()), pos2(x_de(b).max(x_de(a) + 2.0), riel.bottom()));
        p.rect_filled(r, 3, c);
    }
    // la perilla sigue al puntero aunque no se pida otro cuadro
    let t = pulsando.map(|q| q.1).unwrap_or(t);
    let perilla = pos2(x_de(t.min(dur)), riel.center().y);
    if hv > 0.0 {
        p.circle_filled(perilla, 7.0 + 4.0 * hv, pal.verde.gamma_multiply(0.3 * hv));
    }
    // perilla pequeña: anillo verde de 7 px con centro claro
    p.circle_filled(perilla, 7.0 + hv, pal.verde_fuerte);
    p.circle_filled(perilla, 4.5 + hv, pal.tarjeta);
    let _ = resp.on_hover_cursor(CursorIcon::PointingHand);
    nuevo
}

/// Criterio "bodas" del núcleo (para los medidores del foco; se calcula una vez).
fn preset_bodas() -> nucleo::scoring::Preset {
    static P: std::sync::OnceLock<nucleo::scoring::Preset> = std::sync::OnceLock::new();
    P.get_or_init(|| nucleo::bodas::presets().remove(0)).clone()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    /// Al releer las cachés, los carriles de momentos se rehacen aunque solo cambie UNA muestra (antes se
    /// miraban ~9 muestras sueltas y un cambio entre ellas dejaba los carriles viejos).
    #[test]
    fn carriles_se_rehacen_al_releer_las_caches() {
        let dir = std::env::temp_dir().join("camcuts_gui_prueba_carriles");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let ruta = dir.join("Boda").join("C0001.MP4");
        // la muestra 3 no está entre las que miraba la clave vieja (0, 5, 10…)
        let escribir = |m3: &str| {
            let muestras: Vec<serde_json::Value> = (0..40).map(|i| json!({"t": i as f64 / 4.0, "moment": if i == 3 { m3 } else { "votos" }})).collect();
            let mut e = json!({"path": ruta.to_string_lossy(), "fps": [30, 1], "width": 1920, "height": 1080,
                               "duration": 10.0, "per_second": 4.0, "samples": muestras});
            if motor::bodas() {
                e["edition"] = json!("bodas");
            }
            std::fs::write(cache.join("C0001_prueba.json"), e.to_string()).unwrap();
        };
        // ya sondeado: la prueba no lanza ffprobe
        let mut metas = std::collections::BTreeMap::new();
        metas.insert(crate::datos::clave(&ruta), crate::datos::Meta { error: Some(true), ..Default::default() });
        crate::datos::escribir_metas(&dir, &metas);
        escribir("votos");
        let ctx = egui::Context::default();
        let mut a = App::nuevo(&ctx);
        a.persistir = false;
        a.abrir_resultados(&dir);
        a.actualizar_cache_bodas();
        let nombres = |a: &App| a.cache_bodas.carriles.iter().map(|l| l.0.clone()).collect::<Vec<_>>();
        assert_eq!(nombres(&a), ["votos"]);
        // la caché cambia (reanálisis) y la ventana la relee
        escribir("beso");
        a.cargar_clips();
        a.actualizar_cache_bodas();
        assert!(nombres(&a).contains(&"beso".to_string()), "carriles viejos: {:?}", nombres(&a));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

