//! Puntuación por muestra y tramos al estilo Cutdown (port de `scoring.py`). Trabaja solo sobre las
//! medidas guardadas en la caché, así que re-puntuar no vuelve a analizar nada.

use std::collections::HashMap;

use serde_json::Value;

use crate::jewel::Sample;
// las sumas de floats imitan sum() de Python 3.12 (compensada), para dar los mismos bits
use crate::pyfmt::pysum;

fn clip01(v: f64) -> f64 {
    if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
}

/// Nombres de los criterios, en el orden de los pesos de Python.
pub const CRITERIA: [&str; 6] = ["sharp", "ratio", "still", "aniso", "exposure", "size"];

/// Criterio de selección (dataclass `Preset` de Python, mismos valores por defecto).
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    // qué cuenta como joya visible
    pub min_conf: f64,
    pub min_area_frac: f64,
    // referencias de normalización (calibrar con el material propio)
    /// Densidad de bordes de caja que cuenta como totalmente nítida (pruebas Sony 4K: p90 ~0.056).
    pub jewel_edge_ref: f64,
    /// Lo mismo medido sobre la silueta de SAM (mucho más densa: sin piel ni fondo).
    pub masked_edge_ref: f64,
    /// Nitidez principal: |Laplaciano| p99 de la joya a 960 px (corte ~124 sobre 25 muestras etiquetadas).
    pub lap_soft: f64,
    pub lap_sharp: f64,
    /// Una joya perfectamente nítida conserva >= 60 % de su puntuación, sea como sea el resto.
    pub sharp_floor: f64,
    /// Joya tan nítida como el fondo → 0.
    pub ratio_lo: f64,
    /// Joya 2.5 veces más nítida que el fondo → 1.
    pub ratio_hi: f64,
    /// Diagonales de caja por segundo que anulan la quietud.
    pub speed_max: f64,
    pub aniso_ref: f64,
    pub size_ref: f64,
    /// Pesos por criterio (orden de [`CRITERIA`]). La nitidez domina.
    pub weights: Vec<(String, f64)>,
    // Desactivado por defecto: en 121 votos del editor cada uno quitaba ~1 corte malo por 2-3 buenos.
    /// Joya que cruza más de 2 de sus propias diagonales por segundo.
    pub blur_speed: f64,
    /// Gradientes borrados en una dirección.
    pub blur_aniso: f64,
    /// Multiplicador cuando está barrida (1.0 = desactivado).
    pub blur_factor: f64,
    /// La media de un corte debe llegar a esta parte del umbral de muestra (0 = desactivado).
    pub min_mean_factor: f64,
    // reglas duras
    /// Joya claramente más blanda que el fondo.
    pub exclude_background_focus: bool,
    /// Joya que toca el borde del cuadro.
    pub cut_jewel_factor: f64,
    // tramos (mismo significado que en Cutdown)
    pub score_threshold: f64,
    pub min_segment_seconds: f64,
    pub max_segment_seconds: f64,
    pub max_segments_per_clip: usize,
    pub gap_tolerance_seconds: f64,
    pub handle_samples: usize,
    /// Ventana de la mediana móvil de consistencia (0 = desactivada).
    pub smooth_seconds: f64,
    /// Solo estos tipos de plano (producto, modelo, hablando); vacío = todos.
    pub shot_types: Vec<String>,
    /// Solo cortes con alguna de estas joyas (dije, anillo…); vacío = todas.
    pub jewel_types: Vec<String>,
    // cortes casi duplicados dentro de un clip
    pub dedupe: bool,
    /// El centro de la joya se movió menos del 15 % de su tamaño.
    pub dedupe_box_move: f64,
    /// El tamaño de la joya cambió menos del 20 %.
    pub dedupe_resize: f64,
    /// La cámara recorrió menos de 40 px (a 960 px) entre los dos.
    pub dedupe_cam_px: f64,
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            name: "joyas".into(),
            min_conf: 0.15,
            min_area_frac: 0.002,
            jewel_edge_ref: 0.055,
            masked_edge_ref: 0.30,
            lap_soft: 50.0,
            lap_sharp: 125.0,
            sharp_floor: 0.6,
            ratio_lo: 1.0,
            ratio_hi: 2.5,
            speed_max: 1.0,
            aniso_ref: 0.30,
            size_ref: 0.02,
            weights: [("sharp", 0.70), ("ratio", 0.05), ("still", 0.05), ("aniso", 0.05), ("exposure", 0.05), ("size", 0.10)]
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            blur_speed: 2.0,
            blur_aniso: 0.30,
            blur_factor: 1.0,
            min_mean_factor: 0.0,
            exclude_background_focus: true,
            cut_jewel_factor: 0.8,
            score_threshold: 0.55,
            min_segment_seconds: 1.0,
            max_segment_seconds: 6.0,
            max_segments_per_clip: 3,
            gap_tolerance_seconds: 0.5,
            handle_samples: 1,
            smooth_seconds: 1.0,
            shot_types: vec![],
            jewel_types: vec![],
            dedupe: true,
            dedupe_box_move: 0.15,
            dedupe_resize: 0.20,
            dedupe_cam_px: 40.0,
        }
    }
}

impl Preset {
    /// Peso de un criterio (0 si no está).
    pub fn weight(&self, k: &str) -> f64 {
        self.weights.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(0.0)
    }

    /// Cambia un peso conservando el orden.
    pub fn set_weight(&mut self, k: &str, v: f64) {
        match self.weights.iter_mut().find(|(n, _)| n == k) {
            Some(e) => e.1 = v,
            None => self.weights.push((k.to_string(), v)),
        }
    }

    /// Aplica la calibración del editor (`calibration.json`, campos con el mismo nombre) sobre el preset.
    pub fn apply_calibration(&mut self, data: &serde_json::Map<String, Value>) {
        for (k, v) in data {
            let f = v.as_f64();
            match (k.as_str(), f) {
                ("min_conf", Some(x)) => self.min_conf = x,
                ("min_area_frac", Some(x)) => self.min_area_frac = x,
                ("jewel_edge_ref", Some(x)) => self.jewel_edge_ref = x,
                ("masked_edge_ref", Some(x)) => self.masked_edge_ref = x,
                ("lap_soft", Some(x)) => self.lap_soft = x,
                ("lap_sharp", Some(x)) => self.lap_sharp = x,
                ("sharp_floor", Some(x)) => self.sharp_floor = x,
                ("ratio_lo", Some(x)) => self.ratio_lo = x,
                ("ratio_hi", Some(x)) => self.ratio_hi = x,
                ("speed_max", Some(x)) => self.speed_max = x,
                ("aniso_ref", Some(x)) => self.aniso_ref = x,
                ("size_ref", Some(x)) => self.size_ref = x,
                ("blur_speed", Some(x)) => self.blur_speed = x,
                ("blur_aniso", Some(x)) => self.blur_aniso = x,
                ("blur_factor", Some(x)) => self.blur_factor = x,
                ("min_mean_factor", Some(x)) => self.min_mean_factor = x,
                ("cut_jewel_factor", Some(x)) => self.cut_jewel_factor = x,
                ("score_threshold", Some(x)) => self.score_threshold = x,
                ("min_segment_seconds", Some(x)) => self.min_segment_seconds = x,
                ("max_segment_seconds", Some(x)) => self.max_segment_seconds = x,
                ("gap_tolerance_seconds", Some(x)) => self.gap_tolerance_seconds = x,
                ("smooth_seconds", Some(x)) => self.smooth_seconds = x,
                ("dedupe_box_move", Some(x)) => self.dedupe_box_move = x,
                ("dedupe_resize", Some(x)) => self.dedupe_resize = x,
                ("dedupe_cam_px", Some(x)) => self.dedupe_cam_px = x,
                ("max_segments_per_clip", Some(x)) => self.max_segments_per_clip = x as usize,
                ("handle_samples", Some(x)) => self.handle_samples = x as usize,
                ("exclude_background_focus", _) => self.exclude_background_focus = v.as_bool().unwrap_or(self.exclude_background_focus),
                ("dedupe", _) => self.dedupe = v.as_bool().unwrap_or(self.dedupe),
                ("weights", _) => {
                    if let Some(m) = v.as_object() {
                        self.weights = m.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect();
                    }
                }
                _ => {}
            }
        }
    }
}

/// Los presets de Python: "joyas" (por defecto), "social" y "todo".
///
/// "joyas": aprueba cada tramo estable en el que la joya se lee bien, como los selects de Cutdown en V2;
/// el editor quiere los segundos consistentes, no "el mejor cuadro" (2026-10-29).
pub fn presets() -> Vec<Preset> {
    if crate::edicion::es_bodas() {
        return crate::bodas::presets(); // edición Bodas: sus propios criterios
    }
    let d = Preset::default();
    vec![
        Preset { score_threshold: 0.5, max_segment_seconds: 60.0, max_segments_per_clip: 99, gap_tolerance_seconds: 1.0, dedupe: false, ..d.clone() },
        Preset { name: "social".into(), score_threshold: 0.6, max_segment_seconds: 4.0, max_segments_per_clip: 3, ..d.clone() },
        Preset { name: "todo".into(), score_threshold: 0.4, max_segment_seconds: 60.0, max_segments_per_clip: 99, gap_tolerance_seconds: 1.5, dedupe: false, ..d },
    ]
}

/// Preset por nombre, con la calibración del editor aplicada si existe.
pub fn preset(name: &str, calibration: Option<&serde_json::Map<String, Value>>) -> Option<Preset> {
    let mut p = presets().into_iter().find(|p| p.name == name)?;
    if let Some(c) = calibration {
        p.apply_calibration(c);
    }
    Some(p)
}

/// Cada criterio en 0..1, en el orden de [`CRITERIA`].
pub fn components(s: &Sample, p: &Preset) -> [f64; 6] {
    let sharp = if s.jewel_lap_p99 != 0.0 {
        clip01((s.jewel_lap_p99 - p.lap_soft) / (p.lap_sharp - p.lap_soft))
    } else {
        clip01(s.jewel_edge_density / if s.masked { p.masked_edge_ref } else { p.jewel_edge_ref })
    };
    [
        sharp,
        clip01((s.focus_ratio - p.ratio_lo) / (p.ratio_hi - p.ratio_lo)),
        1.0 - clip01(s.jewel_speed / p.speed_max),
        clip01(s.anisotropy / p.aniso_ref),
        1.0 - clip01((s.clip_black + s.clip_white) * 5.0),
        clip01(s.area_frac / p.size_ref),
    ]
}

/// Nitidez 0..1 de una joya a partir de su |Laplaciano| p99 (el criterio "sharp" de [`components`]).
pub fn sharpness_of(lap_p99: f64, p: &Preset) -> f64 {
    clip01((lap_p99 - p.lap_soft) / (p.lap_sharp - p.lap_soft))
}

fn crit_index(k: &str) -> Option<usize> {
    CRITERIA.iter().position(|c| *c == k)
}

/// Puntuación de una muestra y sus avisos (`sin_joya`, `fondo_enfocado`, `barrido_por_movimiento`, `joya_cortada`).
pub fn score_sample(s: &Sample, p: &Preset) -> (f64, Vec<&'static str>) {
    if crate::edicion::es_bodas() {
        return crate::bodas::score_sample(s, p);
    }
    let mut flags = Vec::new();
    if !s.jewel || s.conf < p.min_conf || s.area_frac < p.min_area_frac {
        return (0.0, vec!["sin_joya"]);
    }
    let c = components(s, p);
    let others: Vec<(usize, f64)> = p.weights.iter().filter(|(k, w)| k != "sharp" && *w > 0.0).filter_map(|(k, w)| crit_index(k).map(|i| (i, *w))).collect();
    let rest = if others.is_empty() { 1.0 } else { pysum(others.iter().map(|(i, w)| w * c[*i])) / pysum(others.iter().map(|(_, w)| *w)) };
    // La nitidez es la puerta: el resto solo la modula (hasta sharp_floor), nunca levanta una joya borrosa.
    // El peso de "sharp" fija lo dura que es: 0 la ignora, 0.7 es lineal, más alto es más estricto.
    let ws = p.weight("sharp");
    let gate = if ws > 0.0 { c[0].powf(ws / 0.7) } else { 1.0 };
    let mut score = if others.is_empty() { gate } else { gate * (p.sharp_floor + (1.0 - p.sharp_floor) * rest) };
    if p.exclude_background_focus && s.focus_ratio < 0.8 && s.bg_edge_density > 0.01 {
        flags.push("fondo_enfocado");
        score = 0.0;
    }
    if p.blur_factor < 1.0 && (s.jewel_speed > p.blur_speed || s.anisotropy < p.blur_aniso) {
        flags.push("barrido_por_movimiento");
        score *= p.blur_factor;
    }
    if s.touches_edge {
        flags.push("joya_cortada");
        score *= p.cut_jewel_factor;
    }
    (score, flags)
}

/// Un tramo aprobado.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Segundos, incluido.
    pub start: f64,
    /// Segundos, excluido.
    pub end: f64,
    pub mean_score: f64,
    pub peak_score: f64,
    pub peak_t: f64,
    pub label: String,
    /// Media de cada criterio en el tramo (vacío si no hubo joya), en el orden de [`CRITERIA`].
    pub parts: Vec<(String, f64)>,
    /// Aviso → número de muestras, en orden de aparición.
    pub flags: Vec<(String, usize)>,
    /// producto | modelo | hablando
    pub shot: String,
    /// Tipos de joya vistos de forma estable en el corte, el más frecuente primero.
    pub present: Vec<String>,
    /// Solo edición Bodas: momento dominante del tramo (vacío en joyas).
    pub moment: String,
    /// Solo edición Bodas: momentos con peso en el tramo, el dominante primero.
    pub moments: Vec<String>,
}

impl Segment {
    fn part(&self, k: &str) -> f64 {
        self.parts.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(0.0)
    }

    /// Motivo corto, p. ej. "joya nítida, quieta, más nítida que el fondo".
    pub fn describe(&self) -> String {
        if crate::edicion::es_bodas() {
            return crate::bodas::describe(self);
        }
        let mut words: Vec<String> = Vec::new();
        if !self.parts.is_empty() {
            let sh = self.part("sharp");
            words.push(if sh >= 0.8 { "joya nítida" } else if sh >= 0.5 { "joya algo blanda" } else { "joya blanda" }.into());
            let st = self.part("still");
            words.push(if st >= 0.8 { "quieta" } else if st >= 0.5 { "algo de movimiento" } else { "movida" }.into());
            if self.part("ratio") >= 0.8 {
                words.push("más nítida que el fondo".into());
            }
            if self.part("size") < 0.5 {
                words.push("joya pequeña en cuadro".into());
            }
        }
        words.extend(self.flags.iter().filter(|(k, _)| k != "sin_joya").map(|(k, v)| format!("{k} x{v}")));
        let jewels = if self.present.is_empty() { String::new() } else { format!("joyas: {} · ", self.present.join(", ")) };
        format!("[{}] {}{}", self.shot, jewels, words.join(", "))
    }
}

/// Tipos de joya vistos en al menos el 15 % de las muestras del corte (las detecciones sueltas son ruido).
///
/// En Python, el orden de los empates depende del orden de iteración de un `set` (aleatorio por
/// proceso); aquí los empates siguen el orden de aparición, que es determinista.
pub fn jewels_present(window: &[Sample], min_conf: f64, min_share: f64) -> Vec<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for s in window {
        let mut seen: Vec<&str> = Vec::new();
        for j in &s.jewels {
            if j.conf >= min_conf && !seen.contains(&j.label.as_str()) {
                seen.push(&j.label);
            }
        }
        if seen.is_empty() && s.jewel && !s.label.is_empty() {
            seen.push(&s.label); // cachés anteriores a varias joyas por cuadro
        }
        for l in seen {
            match counts.iter_mut().find(|(k, _)| k == l) {
                Some(e) => e.1 += 1,
                None => counts.push((l.to_string(), 1)),
            }
        }
    }
    let need = (2.0f64).max(min_share * window.len() as f64);
    counts.sort_by(|a, b| b.1.cmp(&a.1)); // estable: empates en orden de aparición
    counts.into_iter().filter(|(_, v)| *v as f64 >= need).map(|(k, _)| k).collect()
}

/// "hablando" si una cara visible habla más de un tercio del corte, "modelo" si alguien la lleva o sostiene.
pub fn shot_type(window: &[Sample]) -> String {
    let n = window.len().max(1) as f64;
    if window.iter().filter(|s| s.speaking.unwrap_or(false)).count() as f64 / n > 0.33 {
        return "hablando".into();
    }
    // una mano con un anillo también se detecta como "persona": hace falta cara o una persona de cuerpo
    if window.iter().filter(|s| s.face || s.person_frac > 0.45).count() as f64 / n > 0.5 {
        return "modelo".into();
    }
    "producto".into()
}

fn iou(a: &[i64; 4], b: &[i64; 4]) -> f64 {
    crate::jewel::box_iou(a, b)
}

fn inside(small: &[i64; 4], big: &[i64; 4]) -> f64 {
    let ix = (small[2].min(big[2]) - small[0].max(big[0])).max(0);
    let iy = (small[3].min(big[3]) - small[1].max(big[1])).max(0);
    let a = (small[2] - small[0]) * (small[3] - small[1]);
    if a > 0 { (ix * iy) as f64 / a as f64 } else { 0.0 }
}

/// Quita las cajas contenedoras y elige la joya principal por centro y foco (en sitio, idempotente).
///
/// - Una caja que contiene joyas al menos 4 veces más pequeñas es el estuche o la mano, no una joya.
/// - Joya principal = la más nítida y centrada (normalmente de la que trata el plano), con algo de peso
///   a la confianza del detector; las medidas de la muestra pasan a ser las suyas.
pub fn refine_jewels(samples: &mut [Sample], width: u32, height: u32) {
    if crate::edicion::es_bodas() {
        return crate::bodas::refine_subjects(samples, width, height);
    }
    let k = 960.0 / width.max(height) as f64;
    let (w, h) = (width as f64 * k, height as f64 * k);
    let half_diag = 0.5 * (w * w + h * h).sqrt();
    for s in samples.iter_mut() {
        if s.jewels.is_empty() {
            continue;
        }
        let area = |b: &[i64; 4]| ((b[2] - b[0]) * (b[3] - b[1])).max(1);
        let js = &s.jewels;
        let kept: Vec<usize> = (0..js.len())
            .filter(|&a| !(0..js.len()).any(|o| o != a && inside(&js[o].bbox, &js[a].bbox) > 0.85 && area(&js[a].bbox) >= 4 * area(&js[o].bbox)))
            .collect();
        if !kept.is_empty() && kept.len() != js.len() {
            s.jewels = kept.into_iter().map(|i| s.jewels[i].clone()).collect();
        }
        let priority = |j: &crate::jewel::Jewel| {
            let b = j.bbox;
            let (cx, cy) = ((b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0);
            let centre = 1.0 - (((cx - w / 2.0).powi(2) + (cy - h / 2.0).powi(2)).sqrt() / half_diag).min(1.0);
            let sharp = ((j.lap_p99.unwrap_or(0.0) - 50.0) / 75.0).clamp(0.0, 1.0);
            (0.15 + sharp) * (0.25 + centre) * (0.5 + j.conf)
        };
        let mut hero = 0;
        for i in 1..s.jewels.len() {
            if priority(&s.jewels[i]) > priority(&s.jewels[hero]) {
                hero = i;
            }
        }
        let j = s.jewels[hero].clone();
        // HERO_FIELDS: solo lo que tenga la joya
        s.label = j.label.clone();
        if let Some(v) = &j.with_labels {
            s.with_labels = v.clone();
        }
        s.conf = j.conf;
        s.bbox = j.bbox.to_vec();
        if let Some(v) = j.area_frac {
            s.area_frac = v;
        }
        if let Some(v) = j.touches_edge {
            s.touches_edge = v;
        }
        if let Some(v) = j.worn {
            s.worn = v;
        }
        if let Some(v) = j.edge {
            s.jewel_edge_density = v;
        }
        if let Some(v) = j.ratio {
            s.focus_ratio = v;
        }
        if let Some(v) = j.lap_var {
            s.jewel_lap_var = v;
        }
        if let Some(v) = j.lap_p99 {
            s.jewel_lap_p99 = v;
        }
        if let Some(v) = j.anisotropy {
            s.anisotropy = v;
        }
        if let Some(v) = j.speed {
            s.jewel_speed = v;
        }
        if let Some(v) = j.masked {
            s.masked = v;
        }
        if let Some(v) = j.mask_frac {
            s.mask_frac = v;
        }
    }
}

/// Da a cada joya física un solo tipo mientras siga en pantalla (en sitio).
///
/// El tipo por cuadro parpadea (anillo → pulsera → cadena sobre el mismo arete). Las joyas se siguen de
/// muestra en muestra por solape de caja (o centro cercano si la cámara se mueve) y cada detección de
/// una pista toma el tipo mayoritario ponderado por confianza. El tipo crudo queda en `raw_label`.
pub fn stabilize_labels(samples: &mut [Sample], _per_second: f64, max_gap_s: f64) {
    if crate::edicion::es_bodas() {
        return crate::bodas::stabilize_all(samples, _per_second, max_gap_s);
    }
    struct Track {
        bbox: [i64; 4],
        t: f64,
        votes: Vec<(String, f64)>,
        members: Vec<(usize, usize)>,
    }
    let mut tracks: Vec<Track> = Vec::new();
    for si in 0..samples.len() {
        let t = samples[si].t;
        let mut taken: Vec<usize> = Vec::new();
        for ji in 0..samples[si].jewels.len() {
            let j = &mut samples[si].jewels[ji];
            if j.raw_label.is_none() {
                j.raw_label = Some(j.label.clone());
            }
            let bx = j.bbox;
            let (cx, cy) = ((bx[0] + bx[2]) as f64 / 2.0, (bx[1] + bx[3]) as f64 / 2.0);
            let diag = (((bx[2] - bx[0]).pow(2) + (bx[3] - bx[1]).pow(2)) as f64).sqrt().max(1.0);
            let mut best: Option<usize> = None;
            let mut best_score = 0.0;
            for (ti, tr) in tracks.iter().enumerate() {
                if taken.contains(&ti) || t - tr.t > max_gap_s {
                    continue;
                }
                let ov = iou(&bx, &tr.bbox);
                let (tx, ty) = ((tr.bbox[0] + tr.bbox[2]) as f64 / 2.0, (tr.bbox[1] + tr.bbox[3]) as f64 / 2.0);
                let near = 1.0 - (((cx - tx).powi(2) + (cy - ty).powi(2)).sqrt() / (0.5 * diag)).min(1.0);
                let score = ov.max(0.5 * near);
                if score > best_score && (ov >= 0.3 || near > 0.4) {
                    best = Some(ti);
                    best_score = score;
                }
            }
            let best = match best {
                Some(b) => b,
                None => {
                    tracks.push(Track { bbox: bx, t, votes: Vec::new(), members: Vec::new() });
                    tracks.len() - 1
                }
            };
            let tr = &mut tracks[best];
            taken.push(best);
            tr.bbox = bx;
            tr.t = t;
            let raw = j.raw_label.clone().unwrap();
            match tr.votes.iter_mut().find(|(k, _)| *k == raw) {
                Some(e) => e.1 += j.conf,
                None => tr.votes.push((raw, j.conf)),
            }
            tr.members.push((si, ji));
        }
    }
    for tr in tracks {
        // max() de Python: el primero con el máximo de votos
        let mut label = &tr.votes[0];
        for v in &tr.votes[1..] {
            if v.1 > label.1 {
                label = v;
            }
        }
        let label = label.0.clone();
        for (si, ji) in tr.members {
            let s = &mut samples[si];
            s.jewels[ji].label = label.clone();
            if s.jewels[ji].bbox[..] == s.bbox[..] {
                s.label = label.clone();
            }
        }
    }
}

/// El editor dijo qué muestra el clip: todas sus joyas toman ese tipo (en sitio).
pub fn force_type(samples: &mut [Sample], label: &str) {
    if crate::edicion::es_bodas() {
        return crate::bodas::force_moment(samples, label); // en bodas el tipo del clip es su momento
    }
    for s in samples.iter_mut() {
        for j in s.jewels.iter_mut() {
            j.label = label.to_string();
        }
        if s.jewel {
            s.label = label.to_string();
        }
    }
}

fn full_label(s: &Sample) -> String {
    let extra: Vec<&str> = s.with_labels.iter().filter(|x| **x != s.label).map(String::as_str).collect();
    if !extra.is_empty() && !s.label.is_empty() { format!("{} con {}", s.label, extra.join(" y ")) } else { s.label.clone() }
}

/// Muestra puntuada: (t, puntuación, avisos, tipo).
pub type Scored = (f64, f64, Vec<&'static str>, String);

/// (t, puntuación, avisos, tipo) por muestra tras la mediana de consistencia: la curva de la que salen los cortes.
pub fn scored_samples(samples: &[Sample], p: &Preset, per_second: f64) -> Vec<Scored> {
    let mut scored: Vec<Scored> = samples.iter().map(|s| {
        let (sc, fl) = score_sample(s, p);
        (s.t, sc, fl, full_label(s))
    }).collect();
    // consistencia: cada momento se juzga por la mediana de ~1 s alrededor, así un cuadro nítido en un
    // tramo blando no pasa y un cuadro blando en un tramo bueno no lo rompe
    let half = (p.smooth_seconds * per_second / 2.0).round_ties_even() as i64;
    if half > 0 {
        let raw: Vec<f64> = scored.iter().map(|r| r.1).collect();
        let n = raw.len() as i64;
        for i in 0..n {
            let mut win: Vec<f64> = raw[(i - half).max(0) as usize..((i + half + 1).min(n)) as usize].to_vec();
            win.sort_by(|a, b| a.total_cmp(b));
            scored[i as usize].1 = win[win.len() / 2];
        }
    }
    scored
}

/// Tramos aprobados de un clip.
pub fn build_segments(samples: &[Sample], p: &Preset, per_second: f64, duration: f64) -> Vec<Segment> {
    if crate::edicion::es_bodas() {
        return crate::bodas::build_segments(samples, p, per_second, duration);
    }
    let step = 1.0 / per_second;
    let scored = scored_samples(samples, p, per_second);
    let ok: Vec<bool> = scored.iter().map(|r| r.1 >= p.score_threshold).collect();
    let n = scored.len();

    // tramos de muestras que pasan; los huecos que no pasan hasta gap_tolerance se puentean
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let max_gap = (p.gap_tolerance_seconds * per_second).round_ties_even() as i64;
    let mut i = 0;
    while i < n {
        if !ok[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut j = i + 1;
        while j < n {
            if ok[j] {
                end = j;
                j += 1;
                continue;
            }
            let mut k = j;
            while k < n && !ok[k] {
                k += 1;
            }
            if k < n && (k - j) as i64 <= max_gap {
                end = k;
                j = k + 1;
            } else {
                break;
            }
        }
        runs.push((start, end));
        i = end + 1;
    }

    let max_len = ((p.max_segment_seconds * per_second).round_ties_even() as i64).max(1) as usize;
    let min_len = ((p.min_segment_seconds * per_second).round_ties_even() as i64).max(1) as usize;
    // Los cortes se juzgan por su núcleo; las asas se añaden después y solo en los bordes exteriores.
    let mut windows: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (a0, b) in runs {
        let mut a = a0;
        let mut pieces = Vec::new();
        // una toma buena larga se parte en cortes de longitud máxima (la cola queda solo si llega al mínimo)
        while b - a + 1 > max_len {
            pieces.push((a, a + max_len - 1));
            a += max_len;
        }
        if b - a + 1 >= min_len || pieces.is_empty() {
            pieces.push((a, b));
        }
        let np = pieces.len();
        for (pi, (ca, cb)) in pieces.into_iter().enumerate() {
            windows.push((ca, cb, if pi == 0 { p.handle_samples } else { 0 }, if pi == np - 1 { p.handle_samples } else { 0 }));
        }
    }
    let mut segs: Vec<Segment> = Vec::new();
    for (ca, cb, h0, h1) in windows {
        let window = &scored[ca..=cb];
        if (cb - ca + 1) as f64 * step < p.min_segment_seconds {
            continue;
        }
        let a = ca.saturating_sub(h0);
        let b = (cb + h1).min(n - 1);
        let mean = pysum(window.iter().map(|r| r.1)) / window.len() as f64;
        // el corte entero debe pasar, no solo sus mejores muestras
        if p.min_mean_factor != 0.0 && mean < p.score_threshold * p.min_mean_factor {
            continue;
        }
        let mut peak = &window[0];
        for r in &window[1..] {
            if r.1 > peak.1 {
                peak = r;
            }
        }
        let labels: Vec<&String> = window.iter().map(|r| &r.3).filter(|l| !l.is_empty()).collect();
        let with_jewel: Vec<&Sample> = samples[ca..=cb].iter().filter(|s| s.jewel).collect();
        let mut parts = Vec::new();
        if !with_jewel.is_empty() {
            let cs: Vec<[f64; 6]> = with_jewel.iter().map(|s| components(s, p)).collect();
            for (k, name) in CRITERIA.iter().enumerate() {
                parts.push((name.to_string(), pysum(cs.iter().map(|c| c[k])) / cs.len() as f64));
            }
        }
        let mut flags: Vec<(String, usize)> = Vec::new();
        for r in window {
            for f in &r.2 {
                match flags.iter_mut().find(|(k, _)| k == f) {
                    Some(e) => e.1 += 1,
                    None => flags.push((f.to_string(), 1)),
                }
            }
        }
        // tipo de la joya en el mejor cuadro, para que coincida con la caja que se ve allí
        let label = if !peak.3.is_empty() {
            peak.3.clone()
        } else {
            let mut best: Option<(&String, usize)> = None;
            for l in &labels {
                let c = labels.iter().filter(|x| **x == *l).count();
                if best.map(|b| c > b.1).unwrap_or(true) {
                    best = Some((l, c));
                }
            }
            best.map(|b| b.0.clone()).unwrap_or_default()
        };
        segs.push(Segment {
            start: scored[a].0,
            end: duration.min(scored[b].0 + step),
            mean_score: mean,
            peak_score: peak.1,
            peak_t: peak.0,
            label,
            parts,
            flags,
            shot: shot_type(&samples[ca..=cb]),
            present: jewels_present(&samples[ca..=cb], 0.15, 0.15),
            moment: String::new(),
            moments: Vec::new(),
        });
    }
    if !p.shot_types.is_empty() {
        segs.retain(|s| p.shot_types.contains(&s.shot));
    }
    if !p.jewel_types.is_empty() {
        // cualquier joya del corte cuenta, no solo la principal
        segs.retain(|s| s.present.iter().any(|j| p.jewel_types.contains(j)));
    }
    // los mejores primero, saltando casi duplicados de un corte ya elegido
    let by_t: HashMap<u64, &Sample> = samples.iter().map(|s| (s.t.to_bits(), s)).collect();
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| segs[b].mean_score.total_cmp(&segs[a].mean_score)); // estable, como sorted(reverse=True)
    let mut chosen: Vec<usize> = Vec::new();
    for i in order {
        if chosen.len() >= p.max_segments_per_clip {
            break;
        }
        let seg = &segs[i];
        if chosen.iter().any(|&c| same_shot(by_t.get(&seg.peak_t.to_bits()).copied(), by_t.get(&segs[c].peak_t.to_bits()).copied(), samples, p)) {
            continue;
        }
        chosen.push(i);
    }
    let mut out: Vec<Segment> = chosen.into_iter().map(|i| segs[i].clone()).collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Dos cortes se parecen: joya en el mismo sitio y tamaño, y la cámara apenas se movió entre ellos.
fn same_shot(a: Option<&Sample>, b: Option<&Sample>, samples: &[Sample], p: &Preset) -> bool {
    let (Some(a), Some(b)) = (a, b) else { return false };
    if !p.dedupe || a.bbox.len() < 4 || b.bbox.len() < 4 {
        return false;
    }
    let (ax0, ay0, ax1, ay1) = (a.bbox[0] as f64, a.bbox[1] as f64, a.bbox[2] as f64, a.bbox[3] as f64);
    let (bx0, by0, bx1, by1) = (b.bbox[0] as f64, b.bbox[1] as f64, b.bbox[2] as f64, b.bbox[3] as f64);
    let diag = ((ax1 - ax0).powi(2) + (ay1 - ay0).powi(2)).sqrt().max(1.0);
    let moved = (((ax0 + ax1 - bx0 - bx1) / 2.0).powi(2) + ((ay0 + ay1 - by0 - by1) / 2.0).powi(2)).sqrt() / diag;
    let (area_a, area_b) = ((ax1 - ax0) * (ay1 - ay0), (bx1 - bx0) * (by1 - by0));
    let resized = (area_a - area_b).abs() / area_a.max(area_b).max(1.0);
    let (t0, t1) = if a.t <= b.t { (a.t, b.t) } else { (b.t, a.t) };
    let travel = pysum(samples.iter().filter(|s| t0 < s.t && s.t <= t1).map(|s| s.cam_dx.abs() + s.cam_dy.abs()));
    moved < p.dedupe_box_move && resized < p.dedupe_resize && travel < p.dedupe_cam_px
}
