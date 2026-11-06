//! Iconos vectoriales dibujados con el `Painter` de egui (equivalentes a los SVG de la maqueta).
//!
//! Cada icono se dibuja dentro de un cuadrado `rect` con trazo redondeado, así escalan a cualquier
//! tamaño y densidad de píxeles sin archivos de imagen.

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

/// Todos los iconos que usa la ventana.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icono {
    Material,
    Pesos,
    Color,
    Tiempo,
    Mapa,
    Reproducir,
    Pausa,
    Inicio,
    Atras,
    Adelante,
    SiguienteTramo,
    Diamante,
    Sol,
    Cuadro,
    Cruz,
    Circulos,
    Onda,
    Carpeta,
    Archivo,
    Ojo,
    Paleta,
    Orden,
    Mas,
    Detener,
    Rayo,
    Lupa,
    Ajustar,
}

/// Convierte coordenadas de un lienzo 24×24 (como los SVG de la maqueta) al rectángulo destino.
fn mapa(rect: Rect) -> impl Fn(f32, f32) -> Pos2 {
    let k = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * k, 12.0 * k);
    move |x, y| pos2(o.x + x * k, o.y + y * k)
}

/// Dibuja un icono con el color y grosor dados (grosor en unidades del lienzo 24×24).
pub fn dibujar(p: &Painter, icono: Icono, rect: Rect, color: Color32, grosor: f32) {
    let m = mapa(rect);
    let k = rect.width().min(rect.height()) / 24.0;
    let st = Stroke::new(grosor * k, color);
    let linea = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| m(x, y)).collect(), st));
    };
    let caja = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        p.rect_stroke(Rect::from_min_max(m(x0, y0), m(x1, y1)), r * k, st, egui::StrokeKind::Middle);
    };
    let circ = |x: f32, y: f32, r: f32| {
        p.circle_stroke(m(x, y), r * k, st);
    };
    let disco = |x: f32, y: f32, r: f32, c: Color32| {
        p.circle_filled(m(x, y), r * k, c);
    };
    let tri = |pts: [(f32, f32); 3]| {
        p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| m(x, y)).collect(), color, Stroke::NONE));
    };
    match icono {
        // rectángulo con franja: "material de cámara"
        Icono::Material => {
            caja(3.0, 5.0, 21.0, 19.0, 3.0);
            linea(&[(3.0, 10.0), (21.0, 10.0)]);
        }
        // tres deslizadores con perilla
        Icono::Pesos => {
            for (y, x) in [(7.0, 9.0), (12.0, 15.0), (17.0, 7.0)] {
                linea(&[(4.0, y), (20.0, y)]);
                disco(x, y, 2.6, color);
            }
        }
        // círculo partido en cuatro
        Icono::Color => {
            circ(12.0, 12.0, 9.0);
            linea(&[(12.0, 3.0), (12.0, 21.0)]);
            linea(&[(3.0, 12.0), (21.0, 12.0)]);
        }
        // pistas de la línea de tiempo
        Icono::Tiempo => {
            caja(3.0, 5.0, 21.0, 19.0, 3.0);
            linea(&[(6.0, 10.0), (12.0, 10.0)]);
            linea(&[(9.0, 14.0), (18.0, 14.0)]);
            linea(&[(15.0, 3.0), (15.0, 21.0)]);
        }
        // mapa plegado
        Icono::Mapa => {
            linea(&[(3.0, 6.0), (9.0, 4.0), (15.0, 6.0), (21.0, 4.0), (21.0, 18.0), (15.0, 20.0), (9.0, 18.0), (3.0, 20.0), (3.0, 6.0)]);
            linea(&[(9.0, 4.0), (9.0, 18.0)]);
            linea(&[(15.0, 6.0), (15.0, 20.0)]);
        }
        Icono::Reproducir => tri([(7.0, 4.0), (20.0, 12.0), (7.0, 20.0)]),
        Icono::Pausa => {
            p.rect_filled(Rect::from_min_max(m(6.0, 4.0), m(10.0, 20.0)), 1.5 * k, color);
            p.rect_filled(Rect::from_min_max(m(14.0, 4.0), m(18.0, 20.0)), 1.5 * k, color);
        }
        Icono::Inicio => {
            p.rect_filled(Rect::from_min_max(m(5.0, 5.0), m(7.5, 19.0)), 1.0 * k, color);
            tri([(19.0, 5.0), (19.0, 19.0), (8.5, 12.0)]);
        }
        Icono::Atras => {
            tri([(12.0, 6.0), (12.0, 18.0), (3.0, 12.0)]);
            tri([(21.0, 6.0), (21.0, 18.0), (12.0, 12.0)]);
        }
        Icono::Adelante => {
            tri([(3.0, 6.0), (3.0, 18.0), (12.0, 12.0)]);
            tri([(12.0, 6.0), (12.0, 18.0), (21.0, 12.0)]);
        }
        Icono::SiguienteTramo => {
            tri([(5.0, 5.0), (5.0, 19.0), (15.5, 12.0)]);
            p.rect_filled(Rect::from_min_max(m(16.5, 5.0), m(19.0, 19.0)), 1.0 * k, color);
        }
        // talla de gema
        Icono::Diamante => linea(&[(6.0, 3.0), (18.0, 3.0), (21.0, 9.0), (12.0, 21.0), (3.0, 9.0), (6.0, 3.0)]),
        Icono::Sol => {
            circ(12.0, 12.0, 4.0);
            for (a, b) in [((12.0, 2.0), (12.0, 5.0)), ((12.0, 19.0), (12.0, 22.0)), ((2.0, 12.0), (5.0, 12.0)), ((19.0, 12.0), (22.0, 12.0))] {
                linea(&[a, b]);
            }
        }
        Icono::Cuadro => caja(5.0, 5.0, 19.0, 19.0, 2.0),
        Icono::Cruz => {
            linea(&[(4.0, 12.0), (20.0, 12.0)]);
            linea(&[(12.0, 4.0), (12.0, 20.0)]);
        }
        // joya nítida delante del fondo
        Icono::Circulos => {
            circ(9.0, 12.0, 5.0);
            p.circle_stroke(m(15.5, 12.0), 5.0 * k, Stroke::new(st.width, color.gamma_multiply(0.55)));
        }
        // barrido de movimiento
        Icono::Onda => {
            for y in [8.0, 12.0, 16.0] {
                linea(&[(3.0, y), (6.0, y - 2.0), (10.0, y + 1.0), (14.0, y - 2.0), (18.0, y + 1.0), (21.0, y - 1.0)]);
            }
        }
        Icono::Carpeta => linea(&[(3.0, 7.0), (3.0, 19.0), (21.0, 19.0), (21.0, 9.0), (11.0, 9.0), (9.0, 6.0), (3.0, 6.0), (3.0, 7.0)]),
        Icono::Archivo => {
            linea(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0), (6.0, 3.0)]);
            linea(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]);
        }
        Icono::Ojo => {
            linea(&[(2.0, 12.0), (6.0, 7.0), (12.0, 5.0), (18.0, 7.0), (22.0, 12.0), (18.0, 17.0), (12.0, 19.0), (6.0, 17.0), (2.0, 12.0)]);
            circ(12.0, 12.0, 3.0);
        }
        Icono::Paleta => {
            circ(12.0, 12.0, 9.0);
            disco(8.0, 10.0, 1.6, color);
            disco(12.0, 7.5, 1.6, color);
            disco(16.0, 10.0, 1.6, color);
        }
        Icono::Orden => {
            linea(&[(7.0, 4.0), (7.0, 20.0)]);
            linea(&[(3.5, 16.5), (7.0, 20.0), (10.5, 16.5)]);
            linea(&[(17.0, 20.0), (17.0, 4.0)]);
            linea(&[(13.5, 7.5), (17.0, 4.0), (20.5, 7.5)]);
        }
        Icono::Mas => {
            linea(&[(12.0, 5.0), (12.0, 19.0)]);
            linea(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Icono::Detener => {
            p.rect_filled(Rect::from_min_max(m(6.0, 6.0), m(18.0, 18.0)), 2.5 * k, color);
        }
        Icono::Rayo => {
            // dos triángulos que se solapan forman el rayo
            tri([(14.0, 2.0), (5.0, 14.0), (13.0, 14.0)]);
            tri([(11.0, 10.0), (19.0, 10.0), (10.0, 22.0)]);
        }
        Icono::Lupa => {
            circ(10.5, 10.5, 6.5);
            linea(&[(15.5, 15.5), (21.0, 21.0)]);
        }
        Icono::Ajustar => {
            linea(&[(3.0, 8.0), (3.0, 3.0), (8.0, 3.0)]);
            linea(&[(16.0, 3.0), (21.0, 3.0), (21.0, 8.0)]);
            linea(&[(21.0, 16.0), (21.0, 21.0), (16.0, 21.0)]);
            linea(&[(8.0, 21.0), (3.0, 21.0), (3.0, 16.0)]);
        }
    }
}
