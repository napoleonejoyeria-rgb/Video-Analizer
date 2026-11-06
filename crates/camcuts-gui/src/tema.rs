//! Tema "Marfil": colores, tipografías y el estilo general de egui.
//!
//! Los valores salen de `design/Marfil.html` (la maqueta que eligió el usuario):
//! fondo casi negro, tarjetas oscuras, tarjetas marfil con tinta oscura y acento dorado.
//! Las fuentes DM Serif Display (títulos) y DM Sans (texto) van incrustadas en el binario
//! (licencia OFL, descargadas de Google Fonts; ver `assets/fonts/OFL-*.txt`).

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Shadow};

// ---------------------------------------------------------------------------------------------
// Colores (los mismos hex de la maqueta)
// ---------------------------------------------------------------------------------------------

/// Fondo de la ventana, casi negro cálido.
pub const FONDO: Color32 = Color32::from_rgb(0x0a, 0x09, 0x07);
/// Tarjetas oscuras (riel, título, lista de clips, pesos).
pub const OSCURO: Color32 = Color32::from_rgb(0x16, 0x14, 0x0f);
/// Riel de las barras de peso (un poco más claro que la tarjeta).
pub const OSCURO_2: Color32 = Color32::from_rgb(0x1d, 0x1a, 0x14);
/// Fondo de la píldora "Exportar".
pub const OSCURO_3: Color32 = Color32::from_rgb(0x1f, 0x1c, 0x16);
/// Hover sobre superficies oscuras.
pub const OSCURO_HOVER: Color32 = Color32::from_rgb(0x26, 0x22, 0x1a);
/// Tarjetas marfil (recibo, transporte, línea de tiempo).
pub const MARFIL: Color32 = Color32::from_rgb(0xef, 0xe4, 0xcc);
/// Tinta oscura sobre marfil.
pub const TINTA: Color32 = Color32::from_rgb(0x1a, 0x16, 0x10);
/// Acento dorado.
pub const ORO: Color32 = Color32::from_rgb(0xe2, 0xb8, 0x63);
/// Texto apagado sobre oscuro (subtítulos, contadores).
pub const APAGADO: Color32 = Color32::from_rgb(0x8d, 0x82, 0x6b);
/// Texto apagado sobre marfil.
pub const TINTA_APAGADA: Color32 = Color32::from_rgb(0x6b, 0x60, 0x4a);
/// Texto secundario dentro del recuadro oscuro del recibo.
pub const ARENA: Color32 = Color32::from_rgb(0xb8, 0xab, 0x8d);
/// Rojo suave para errores y avisos.
pub const ROJO: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x5a);

/// Colores de las fichas de cada peso (los cuatro de la maqueta y dos más del mismo tono).
pub const LAVANDA: Color32 = Color32::from_rgb(0xa9, 0x9a, 0xd6);
pub const CORAL: Color32 = Color32::from_rgb(0xe8, 0x9f, 0x8c);
pub const MENTA: Color32 = Color32::from_rgb(0x8f, 0xc3, 0xb0);
pub const CIELO: Color32 = Color32::from_rgb(0x8f, 0xb3, 0xd9);
pub const ROSA: Color32 = Color32::from_rgb(0xd6, 0xa3, 0xc4);

/// Marfil con transparencia (puntos de las barras, líneas finas sobre oscuro).
pub fn marfil_a(alpha: f32) -> Color32 {
    MARFIL.gamma_multiply(alpha)
}

/// Tinta con transparencia (rieles y separadores sobre marfil).
pub fn tinta_a(alpha: f32) -> Color32 {
    TINTA.gamma_multiply(alpha)
}

/// Oro con transparencia (filas seleccionadas, tramos fuera del filtro).
pub fn oro_a(alpha: f32) -> Color32 {
    ORO.gamma_multiply(alpha)
}

/// Mezcla lineal entre dos colores (para animaciones de hover).
pub fn mezclar(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ---------------------------------------------------------------------------------------------
// Medidas (radios y separaciones de la maqueta)
// ---------------------------------------------------------------------------------------------

/// Margen exterior de la ventana.
pub const MARGEN: f32 = 22.0;
/// Separación entre tarjetas.
pub const HUECO: f32 = 18.0;
/// Ancho del riel de iconos.
pub const ANCHO_RIEL: f32 = 84.0;
/// Ancho de las columnas izquierda y derecha.
pub const ANCHO_COLUMNA: f32 = 330.0;
/// Radio de las tarjetas oscuras.
pub const RADIO_OSCURO: u8 = 32;
/// Radio de las tarjetas marfil.
pub const RADIO_MARFIL: u8 = 30;

/// Sombra suave de las tarjetas marfil (`0 20px 40px rgba(0,0,0,.45)` en la maqueta).
pub fn sombra_marfil() -> Shadow {
    Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha(110) }
}

/// Sombra más discreta para las tarjetas oscuras y las píldoras flotantes.
pub fn sombra_suave() -> Shadow {
    Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(90) }
}

// ---------------------------------------------------------------------------------------------
// Tipografías
// ---------------------------------------------------------------------------------------------

/// Familia para títulos (DM Serif Display).
pub fn serif() -> FontFamily {
    FontFamily::Name("serif".into())
}
/// DM Sans medium (500).
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}
/// DM Sans bold (700).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Atajos para construir `FontId` con las familias del tema.
pub fn f_texto(tam: f32) -> FontId {
    FontId::new(tam, FontFamily::Proportional)
}
pub fn f_medio(tam: f32) -> FontId {
    FontId::new(tam, medium())
}
pub fn f_negrita(tam: f32) -> FontId {
    FontId::new(tam, bold())
}
pub fn f_titulo(tam: f32) -> FontId {
    FontId::new(tam, serif())
}

/// Registra las fuentes incrustadas. Las de egui quedan detrás como respaldo para símbolos
/// que DM Sans no trae (flechas, ▶, ❚❚…).
pub fn fuentes() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let datos: [(&str, &'static [u8]); 4] = [
        ("dm-sans", include_bytes!("../assets/fonts/DMSans-Regular.ttf")),
        ("dm-sans-medium", include_bytes!("../assets/fonts/DMSans-Medium.ttf")),
        ("dm-sans-bold", include_bytes!("../assets/fonts/DMSans-Bold.ttf")),
        ("dm-serif", include_bytes!("../assets/fonts/DMSerifDisplay-Regular.ttf")),
    ];
    for (nombre, bytes) in datos {
        f.font_data.insert(nombre.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    // respaldo: las fuentes de egui de siempre (incluyen emoji y símbolos)
    let respaldo: Vec<String> = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let con = |primera: &str| {
        let mut v = vec![primera.to_owned()];
        v.extend(respaldo.iter().cloned());
        v
    };
    f.families.insert(FontFamily::Proportional, con("dm-sans"));
    f.families.insert(medium(), con("dm-sans-medium"));
    f.families.insert(bold(), con("dm-sans-bold"));
    f.families.insert(serif(), con("dm-serif"));
    f
}

/// Aplica fuentes, colores y espaciados de Marfil al contexto de egui.
pub fn instalar(ctx: &egui::Context) {
    ctx.set_fonts(fuentes());
    ctx.all_styles_mut(|s| {
        use egui::{Stroke, TextStyle};
        s.text_styles.insert(TextStyle::Body, f_texto(14.0));
        s.text_styles.insert(TextStyle::Button, f_medio(14.0));
        s.text_styles.insert(TextStyle::Small, f_texto(12.0));
        s.text_styles.insert(TextStyle::Heading, f_titulo(26.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        s.animation_time = 0.18;
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.scroll.bar_width = 6.0;
        // barra fija y fina: se ve que la tarjeta tiene más contenido
        s.spacing.scroll.floating = false;
        s.spacing.scroll.bar_inner_margin = 2.0;
        s.spacing.scroll.bar_outer_margin = 0.0;
        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = FONDO;
        v.window_fill = OSCURO;
        v.extreme_bg_color = OSCURO_2;
        v.faint_bg_color = OSCURO_2;
        v.override_text_color = None;
        v.hyperlink_color = ORO;
        v.selection.bg_fill = oro_a(0.35);
        v.selection.stroke = Stroke::new(1.0, ORO);
        v.window_corner_radius = egui::CornerRadius::same(20);
        v.menu_corner_radius = egui::CornerRadius::same(16);
        v.window_shadow = sombra_suave();
        v.popup_shadow = sombra_suave();
        v.window_stroke = Stroke::new(1.0, marfil_a(0.08));
        let w = &mut v.widgets;
        for (st, fondo, texto) in [
            (&mut w.noninteractive, OSCURO, MARFIL),
            (&mut w.inactive, OSCURO_2, MARFIL),
            (&mut w.hovered, OSCURO_HOVER, MARFIL),
            (&mut w.active, OSCURO_3, ORO),
            (&mut w.open, OSCURO_2, MARFIL),
        ] {
            st.bg_fill = fondo;
            st.weak_bg_fill = fondo;
            st.fg_stroke = Stroke::new(1.2, texto);
            st.bg_stroke = Stroke::NONE;
            st.corner_radius = egui::CornerRadius::same(12);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, marfil_a(0.08));
    });
}
