//! Tema "Salvia" de la edición Bodas: verde clarito (salvia/menta) sobre fondos limpios, con modo
//! claro (por defecto) y oscuro.
//!
//! A diferencia de Marfil (colores fijos en constantes), aquí la paleta se elige al vuelo: [`paleta`]
//! devuelve la del modo actual y [`instalar`] cambia de modo y ajusta el estilo de egui (menús,
//! ventanas emergentes, selección de texto). Las fuentes son las de Marfil (DM Sans y DM Serif Display,
//! incrustadas). Cada momento de la boda tiene su color pastel ([`color_momento`]), el mismo en los dos
//! modos, para que las chips, los carriles de la línea de tiempo y los tramos se reconozcan de un vistazo.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, Shadow, Stroke};

/// Colores de la interfaz en un modo (claro u oscuro).
#[derive(Clone, Copy, Debug)]
pub struct Paleta {
    /// Fondo de la ventana.
    pub fondo: Color32,
    /// Tarjetas.
    pub tarjeta: Color32,
    /// Zonas hundidas dentro de una tarjeta (rieles, campos, filas).
    pub hundido: Color32,
    /// Bordes finos.
    pub borde: Color32,
    /// Texto principal.
    pub texto: Color32,
    /// Texto secundario.
    pub suave: Color32,
    /// Texto terciario (ayudas, ejes).
    pub tenue: Color32,
    /// El verde clarito de la marca (rellenos, botones principales).
    pub verde: Color32,
    /// Verde más intenso para trazos y texto sobre fondo claro.
    pub verde_fuerte: Color32,
    /// Verde muy suave (filas elegidas, fondos de chips).
    pub verde_suave: Color32,
    /// Tinta sobre el verde.
    pub sobre_verde: Color32,
    /// Fondo del visor (alrededor del cuadro).
    pub visor: Color32,
    /// Avisos y errores.
    pub rojo: Color32,
    /// ¿Es el modo claro?
    pub claro: bool,
}

/// Modo claro (por defecto en Bodas).
pub const CLARO: Paleta = Paleta {
    fondo: Color32::from_rgb(0xf3, 0xf6, 0xf1),
    tarjeta: Color32::from_rgb(0xff, 0xff, 0xff),
    hundido: Color32::from_rgb(0xee, 0xf3, 0xec),
    borde: Color32::from_rgb(0xdd, 0xe6, 0xda),
    texto: Color32::from_rgb(0x1e, 0x2b, 0x24),
    suave: Color32::from_rgb(0x5a, 0x6b, 0x61),
    tenue: Color32::from_rgb(0x93, 0xa1, 0x98),
    verde: Color32::from_rgb(0xa8, 0xd8, 0xb9),
    verde_fuerte: Color32::from_rgb(0x3c, 0x7a, 0x59),
    verde_suave: Color32::from_rgb(0xe2, 0xf2, 0xe7),
    sobre_verde: Color32::from_rgb(0x15, 0x33, 0x24),
    visor: Color32::from_rgb(0x1b, 0x22, 0x1e),
    rojo: Color32::from_rgb(0xd0, 0x5e, 0x55),
    claro: true,
};

/// Modo oscuro.
pub const OSCURO: Paleta = Paleta {
    fondo: Color32::from_rgb(0x0d, 0x12, 0x10),
    tarjeta: Color32::from_rgb(0x16, 0x1d, 0x19),
    hundido: Color32::from_rgb(0x1d, 0x26, 0x21),
    borde: Color32::from_rgb(0x27, 0x33, 0x2c),
    texto: Color32::from_rgb(0xe6, 0xef, 0xe8),
    suave: Color32::from_rgb(0xa0, 0xb3, 0xa7),
    tenue: Color32::from_rgb(0x6c, 0x7d, 0x72),
    verde: Color32::from_rgb(0xa8, 0xd8, 0xb9),
    verde_fuerte: Color32::from_rgb(0x95, 0xd1, 0xab),
    verde_suave: Color32::from_rgb(0x1f, 0x33, 0x28),
    sobre_verde: Color32::from_rgb(0x10, 0x24, 0x1a),
    visor: Color32::from_rgb(0x09, 0x0c, 0x0b),
    rojo: Color32::from_rgb(0xe0, 0x7a, 0x6e),
    claro: false,
};

static MODO_CLARO: AtomicBool = AtomicBool::new(true);

/// Paleta del modo actual.
pub fn paleta() -> Paleta {
    if MODO_CLARO.load(Ordering::Relaxed) { CLARO } else { OSCURO }
}

/// Color pastel de cada momento de la boda (el mismo en los dos modos).
pub fn color_momento(m: &str) -> Color32 {
    let (r, g, b) = match m {
        "preparativos" => (0xf4, 0xc2, 0xb1),
        "detalles" => (0xe8, 0xd5, 0xa0),
        "lugar" => (0xb3, 0xd4, 0xe8),
        "ceremonia" => (0xc9, 0xb8, 0xec),
        "votos" => (0xee, 0xab, 0xc6),
        "anillos" => (0xf0, 0xd4, 0x86),
        "beso" => (0xef, 0x9c, 0x9c),
        "cóctel" => (0xa3, 0xdc, 0xc6),
        "retratos" => (0xb8, 0xd3, 0x9b),
        "entrada" => (0xf6, 0xb5, 0x8a),
        "primer baile" => (0xd6, 0xa6, 0xdc),
        "discursos" => (0x9f, 0xc5, 0xec),
        "banquete" => (0xdc, 0xc3, 0xa0),
        "pastel" => (0xf5, 0xc5, 0xd8),
        "fiesta" => (0xff, 0xb7, 0x8c),
        "salida" => (0xab, 0xbd, 0xf2),
        _ => (0xc6, 0xcd, 0xc9),
    };
    Color32::from_rgb(r, g, b)
}

/// Tinta legible sobre un color de momento (siempre oscura: los pasteles son claros).
pub const TINTA_MOMENTO: Color32 = Color32::from_rgb(0x22, 0x26, 0x24);

/// Sombra suave de las tarjetas (más leve en claro, más profunda en oscuro).
pub fn sombra(pal: &Paleta) -> Shadow {
    if pal.claro {
        Shadow { offset: [0, 8], blur: 26, spread: 0, color: Color32::from_rgba_unmultiplied(40, 70, 52, 30) }
    } else {
        Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(110) }
    }
}

/// Cambia de modo y aplica fuentes, colores y espaciados al estilo de egui.
pub fn instalar(ctx: &egui::Context, claro: bool) {
    MODO_CLARO.store(claro, Ordering::Relaxed);
    let pal = paleta();
    ctx.set_fonts(crate::tema::fuentes());
    ctx.all_styles_mut(|s| {
        use egui::TextStyle;
        use crate::tema::{f_medio, f_texto, f_titulo};
        s.text_styles.insert(TextStyle::Body, f_texto(14.0));
        s.text_styles.insert(TextStyle::Button, f_medio(14.0));
        s.text_styles.insert(TextStyle::Small, f_texto(12.0));
        s.text_styles.insert(TextStyle::Heading, f_titulo(26.0));
        s.text_styles.insert(TextStyle::Monospace, egui::FontId::monospace(13.0));
        s.animation_time = 0.18;
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.scroll.bar_width = 6.0;
        s.spacing.scroll.floating = false;
        s.spacing.scroll.bar_inner_margin = 2.0;
        s.spacing.scroll.bar_outer_margin = 0.0;
        let v = &mut s.visuals;
        v.dark_mode = !pal.claro;
        v.panel_fill = pal.fondo;
        v.window_fill = pal.tarjeta;
        v.extreme_bg_color = pal.hundido;
        v.faint_bg_color = pal.hundido;
        v.override_text_color = Some(pal.texto);
        v.hyperlink_color = pal.verde_fuerte;
        v.selection.bg_fill = pal.verde.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, pal.verde_fuerte);
        v.window_corner_radius = egui::CornerRadius::same(18);
        v.menu_corner_radius = egui::CornerRadius::same(16);
        v.window_shadow = sombra(&pal);
        v.popup_shadow = sombra(&pal);
        v.window_stroke = Stroke::new(1.0, pal.borde);
        let w = &mut v.widgets;
        for (st, fondo) in [(&mut w.noninteractive, pal.tarjeta), (&mut w.inactive, pal.hundido), (&mut w.hovered, pal.verde_suave), (&mut w.active, pal.verde), (&mut w.open, pal.hundido)] {
            st.bg_fill = fondo;
            st.weak_bg_fill = fondo;
            st.fg_stroke = Stroke::new(1.2, pal.texto);
            st.bg_stroke = Stroke::NONE;
            st.corner_radius = egui::CornerRadius::same(12);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, pal.borde);
    });
}

/// Mezcla lineal entre dos colores (animaciones de hover).
pub fn mezclar(a: Color32, b: Color32, t: f32) -> Color32 {
    crate::tema::mezclar(a, b, t)
}
