//! Edición Bodas: medidas, puntuación y momentos para material de bodas.
//!
//! Es el port de la edición Bodas de Python (`camcuts_bodas/camcuts`). Solo se usa cuando la edición
//! del proceso es [`crate::edicion::Edicion::Bodas`]: las funciones de `scoring` y `analyse` le pasan el
//! trabajo con una línea al principio, y el comportamiento de la edición de joyas no cambia.
//!
//! Vocabulario: la caché y los tipos del núcleo siguen llamando "jewel" a lo que aquí es el **sujeto
//! principal** (una cara, una persona o un objeto de boda: anillos, ramo, pastel, copas…). Los campos
//! nuevos de esta edición van en el mapa `extra` de cada muestra y de cada sujeto, así la estructura
//! [`Sample`] no cambia y las cachés de joyas se leen y escriben igual que siempre:
//!
//! - muestra: `frame_lap_p99` (nitidez del cuadro entero), `jewel_luma` / `jewel_clip` (exposición del
//!   sujeto), `faced` (el sujeto muestra la cara), `face_frac` (cara más grande / cuadro), `voice` (alguien
//!   habla en el audio), `moment_raw` / `moment_conf_raw` / `moment_top` (opinión de CLIP en ese cuadro),
//!   `moment` / `moment_conf` (momento estable) y `shake` (temblor de cámara, se calcula al cargar);
//! - sujeto: `luma`, `clip`, `faced`.

use serde_json::{json, Map, Value};

use crate::jewel::{self, box_iou, Det, FrameMaps, Jewel, MeasureOpts, MeasureState, Sample, SilCache, KP_MIN, L_EAR, L_EYE, L_SH, L_WRIST, NOSE, R_EAR, R_EYE, R_SH, R_WRIST};
use crate::metrics::{self, Region};
use crate::pyfmt::pysum;
use crate::scoring::{self, Preset, Segment};
use crate::types::{Detector, Frame, Pose};

// =================================================================================================
// Sujetos
// =================================================================================================

/// Etiquetas de los sujetos (las del detector de bodas, en el orden de sus clases).
pub const SUJETOS: [&str; 12] = ["cara", "persona", "novia", "novio", "vestido", "anillos", "ramo", "pastel", "copas", "micrófono", "zapatos", "bengalas"];
/// Caras.
pub const CARAS: [&str; 1] = ["cara"];
/// Cuerpos (una persona, la novia, el novio).
pub const CUERPOS: [&str; 3] = ["persona", "novia", "novio"];

/// ¿Es un sujeto de boda?
pub fn es_sujeto(label: &str) -> bool {
    SUJETOS.contains(&label)
}
/// ¿Es una cara?
pub fn es_cara(label: &str) -> bool {
    CARAS.contains(&label)
}
/// ¿Es un cuerpo?
pub fn es_cuerpo(label: &str) -> bool {
    CUERPOS.contains(&label)
}

/// Cuando dos detecciones cubren lo mismo (una novia también es una persona), gana la más específica.
fn prioridad(label: &str) -> i32 {
    match label {
        "cara" => 0,
        "anillos" | "ramo" | "pastel" | "copas" | "micrófono" | "zapatos" | "bengalas" => 1,
        "novia" | "novio" => 2,
        "vestido" => 3,
        "persona" => 4,
        _ => 9,
    }
}

/// Sujetos medidos por cuadro (una multitud de invitados serían docenas).
pub const MAX_SUJETOS: usize = 8;
/// Ajuste de la corrección de nitidez de la ruta de GPU para esta edición (ver [`measure`]).
pub const GPU_AJUSTE: f64 = 1.05;
/// |Laplaciano| p99 desde el que un sujeto cuenta como nítido (las caras tienen detalle más suave).
pub const SHARP_LAP: f64 = 70.0;
/// Dos cajas de lo mismo (persona + novia sobre un cuerpo).
pub const GROUP_IOU: f64 = 0.5;
/// Confianza mínima con la que se usa una detección (la de Python sin verificador CLIP).
pub const DETECT_CONF: f64 = 0.05;

/// Cuánto "sostiene" cada tipo de sujeto un plano de boda al elegir el principal (objetos: 0.8).
fn peso_tipo(label: &str) -> f64 {
    match label {
        "cara" => 1.0,
        "novia" | "novio" => 0.75,
        "vestido" => 0.7,
        "persona" => 0.6,
        _ => 0.8,
    }
}
/// |Laplaciano| p99 de blando a nítido al elegir el sujeto principal.
const HERO_LAP_LO: f64 = 30.0;
const HERO_LAP_HI: f64 = 100.0;
/// Una cara de ~45 px a 960 px ya cuenta como del todo visible.
const HERO_SIZE_REF: f64 = 0.004;

fn clip01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

// --- campos extra de muestras y sujetos ----------------------------------------------------------

/// Número del mapa `extra` (o `None`).
pub fn xf(m: &Map<String, Value>, k: &str) -> Option<f64> {
    m.get(k).and_then(Value::as_f64)
}
/// Booleano del mapa `extra`.
pub fn xb(m: &Map<String, Value>, k: &str) -> Option<bool> {
    m.get(k).and_then(Value::as_bool)
}
/// Texto del mapa `extra`.
pub fn xs<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}
/// Número finito como valor JSON (NaN o infinito → 0, que JSON no admite).
fn num(v: f64) -> Value {
    json!(if v.is_finite() { v } else { 0.0 })
}

/// Momento estable de una muestra (vacío si no tiene).
pub fn momento(s: &Sample) -> &str {
    xs(&s.extra, "moment").unwrap_or("")
}

/// ¿Muestra la cara el sujeto? (las cachés sin el campo cuentan como sí)
fn faced(j: &Jewel) -> bool {
    xb(&j.extra, "faced").unwrap_or(true)
}

/// Qué tan bien sostiene el plano un sujeto: nítido, centrado, seguro, visible, y una cara si la hay.
/// `w`, `h`: tamaño del cuadro en los píxeles de las cajas (960 px el lado largo).
pub fn hero_priority(j: &Jewel, w: f64, h: f64) -> f64 {
    let b = j.bbox;
    let (cx, cy) = ((b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0);
    let half_diag = 0.5 * (w * w + h * h).sqrt();
    let centre = 1.0 - (((cx - w / 2.0).powi(2) + (cy - h / 2.0).powi(2)).sqrt() / half_diag).min(1.0);
    let sharp = clip01((j.lap_p99.unwrap_or(0.0) - HERO_LAP_LO) / (HERO_LAP_HI - HERO_LAP_LO));
    let size = (j.area_frac.unwrap_or(0.0) / HERO_SIZE_REF).min(1.0);
    let mut kind = peso_tipo(&j.label);
    if es_cuerpo(&j.label) && !faced(j) {
        kind *= 0.8; // alguien de espaldas sostiene menos el plano
    }
    (0.15 + sharp) * (0.25 + centre) * (0.5 + j.conf) * (0.35 + 0.65 * size) * kind
}

/// Grupos de sujetos, el más seguro primero: (detección principal, etiquetas fusionadas con ella).
///
/// Solo se juntan cajas de lo mismo (IoU ≥ 0.5: un cuerpo visto como "persona" y como "novia"); una cara
/// dentro de un cuerpo o unos anillos en una mano siguen siendo sujetos aparte.
pub fn groups(dets: &[Det], min_conf: f64) -> Vec<(Det, Vec<String>)> {
    let mut left: Vec<&Det> = dets.iter().filter(|d| d.conf >= min_conf && es_sujeto(&d.label)).collect();
    left.sort_by(|a, b| b.conf.total_cmp(&a.conf));
    let mut out = Vec::new();
    while !left.is_empty() && out.len() < MAX_SUJETOS {
        let seed = left.remove(0);
        let mut group = vec![seed];
        group.extend(left.iter().copied().filter(|d| box_iou(&d.bbox, &seed.bbox) >= GROUP_IOU && es_cara(&d.label) == es_cara(&seed.label)));
        left.retain(|d| !group.iter().any(|g| *g == *d));
        let mut main = group[0];
        for d in &group[1..] {
            if (prioridad(&d.label), -d.conf) < (prioridad(&main.label), -main.conf) {
                main = d;
            }
        }
        let mut extras: Vec<String> = group.iter().filter(|d| d.label != main.label).map(|d| d.label.clone()).collect();
        extras.sort();
        extras.dedup();
        out.push((main.clone(), extras));
    }
    out
}

fn cara_visible(p: &Pose) -> bool {
    let k = &p.keypoints;
    k[NOSE][2] > KP_MIN && (k[L_EYE][2] > KP_MIN || k[R_EYE][2] > KP_MIN)
}

/// Nariz de cada persona que mira a cámara (nariz y un ojo visibles).
pub fn face_points(people: &[Pose]) -> Vec<(f64, f64)> {
    people.iter().filter(|p| cara_visible(p)).map(|p| (p.keypoints[NOSE][0] as f64, p.keypoints[NOSE][1] as f64)).collect()
}

/// Centro de los hombros de cada persona que se ve de espaldas: hombros visibles y ni nariz ni ojos.
pub fn back_points(people: &[Pose]) -> Vec<(f64, f64)> {
    people
        .iter()
        .filter(|p| {
            let k = &p.keypoints;
            k[L_SH][2] > KP_MIN && k[R_SH][2] > KP_MIN && k[NOSE][2] <= KP_MIN && k[L_EYE][2] <= KP_MIN && k[R_EYE][2] <= KP_MIN
        })
        .map(|p| {
            let k = &p.keypoints;
            (((k[L_SH][0] + k[R_SH][0]) / 2.0) as f64, ((k[L_SH][1] + k[R_SH][1]) / 2.0) as f64)
        })
        .collect()
}

/// Cajas de cara a partir de la pose (nariz, ojos, orejas) para las caras que el detector no vio.
///
/// El prompt "face" de YOLO-World casi no encuentra caras en material real mientras la pose las ve
/// claras; sin esto el sujeto principal sería el cuerpo entero. Llevan `zoom = false` y una confianza
/// algo menor que la de la pose.
pub fn pose_faces(people: &[Pose], dets: &[Det]) -> Vec<Det> {
    let have: Vec<[i64; 4]> = dets.iter().filter(|d| es_cara(&d.label) && d.conf >= 0.15).map(|d| d.bbox).collect();
    let mut out = Vec::new();
    for p in people {
        if !cara_visible(p) {
            continue;
        }
        let k = &p.keypoints;
        let pts: Vec<(f64, f64)> = [NOSE, L_EYE, R_EYE, L_EAR, R_EAR].iter().filter(|&&i| k[i][2] > KP_MIN).map(|&i| (k[i][0] as f64, k[i][1] as f64)).collect();
        let (xmin, xmax) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let ymean = pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64;
        let ojos = k[L_EYE][2] > KP_MIN && k[R_EYE][2] > KP_MIN;
        let entre_ojos = if ojos { ((k[L_EYE][0] - k[R_EYE][0]) as f64).hypot((k[L_EYE][1] - k[R_EYE][1]) as f64) * 2.5 } else { 0.0 };
        let span = (xmax - xmin).max(entre_ojos);
        if span < 6.0 {
            continue;
        }
        let side = span * 1.25;
        let (cx, cy) = ((xmax + xmin) / 2.0, ymean + 0.1 * side);
        let bbox = [(cx - side / 2.0) as i64, (cy - side * 0.65) as i64, (cx + side / 2.0) as i64, (cy + side * 0.65) as i64];
        if have.iter().any(|b| box_iou(&bbox, b) > 0.3) {
            continue;
        }
        let conf = k[NOSE][2].min(k[L_EYE][2].max(k[R_EYE][2])) as f64;
        out.push(Det { bbox, conf: (conf * 0.9 * 1000.0).round() / 1000.0, label: "cara".into(), zoom: false, verified: false });
    }
    out
}

/// Zonas cuadradas alrededor de cabezas y manos de personas lejanas, para la segunda mirada con zoom
/// (caras de 15-25 px en un plano general de la ceremonia, anillos en las manos).
pub fn zoom_regions(people: &[Pose], w: u32, h: u32, max_regions: usize) -> Vec<[i64; 4]> {
    let mut regions = Vec::new();
    for p in people {
        let k = &p.keypoints;
        let ok = |i: usize| k[i][2] > KP_MIN;
        let dist = |a: usize, b: usize| if ok(a) && ok(b) { ((k[a][0] - k[b][0]) as f64).hypot((k[a][1] - k[b][1]) as f64) } else { 0.0 };
        let or = |a: f64, b: f64| if a != 0.0 { a } else { b };
        let shoulders = dist(L_SH, R_SH);
        let head = or(or(or(dist(L_EAR, R_EAR), dist(NOSE, L_EAR).max(dist(NOSE, R_EAR)) * 1.2), dist(L_EYE, R_EYE) * 2.5), shoulders * 0.4);
        let body = or(shoulders, head * 2.5);
        if head <= 0.0 {
            continue;
        }
        let mut spots: Vec<(f64, f64, f64)> = Vec::new();
        if ok(NOSE) {
            spots.push((k[NOSE][0] as f64, k[NOSE][1] as f64 - 0.2 * head, 1.8 * head));
        }
        for wr in [L_WRIST, R_WRIST] {
            if ok(wr) {
                spots.push((k[wr][0] as f64, k[wr][1] as f64, 0.55 * body));
            }
        }
        for (cx, cy, side) in spots {
            let r = [((cx - side / 2.0) as i64).max(0), ((cy - side / 2.0) as i64).max(0), ((cx + side / 2.0) as i64).min(w as i64), ((cy + side / 2.0) as i64).min(h as i64)];
            let area = ((r[2] - r[0]) * (r[3] - r[1])) as f64 / (w as f64 * h as f64);
            if r[2] - r[0] >= 8 && r[3] - r[1] >= 8 && area <= jewel::ZOOM_MAX_AREA {
                regions.push(r);
            }
        }
    }
    regions.truncate(max_regions);
    regions
}

/// ¿Muestra cara este sujeto? Un cuerpo cuenta como "de espaldas" solo si la pose ve en él unos hombros
/// sin nariz ni ojos y no hay ninguna cara en su parte alta. Una mano con anillos (que el detector también
/// llama "persona") o un cuerpo cortado no son "de espaldas": no hay pruebas.
fn tiene_cara(label: &str, b: &[i64; 4], faces: &[[i64; 4]], points: &[(f64, f64)], backs: &[(f64, f64)]) -> bool {
    if !es_cuerpo(label) {
        return true;
    }
    let (x0, y0, x1, y1) = (b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64);
    let top = y0 + 0.6 * (y1 - y0);
    let arriba = |x: f64, y: f64| x0 <= x && x <= x1 && y0 <= y && y <= top;
    if faces.iter().any(|f| arriba((f[0] + f[2]) as f64 / 2.0, (f[1] + f[3]) as f64 / 2.0)) || points.iter().any(|&(x, y)| arriba(x, y)) {
        return true;
    }
    !backs.iter().any(|&(x, y)| x0 <= x && x <= x1 && y0 <= y && y <= y1)
}

/// Luma media (0..1) y fracción de píxeles negros (≤ 2) o quemados (≥ 253) dentro de una región.
fn exposicion_region(gray: &[u8], w: usize, region: Region) -> (f64, f64) {
    let (mut n, mut suma, mut recorte) = (0u64, 0u64, 0u64);
    region.for_each(w, |i| {
        let v = gray[i];
        n += 1;
        suma += v as u64;
        if v <= 2 || v >= 253 {
            recorte += 1;
        }
    });
    if n == 0 {
        return (0.0, 0.0);
    }
    (suma as f64 / n as f64 / 255.0, recorte as f64 / n as f64)
}

/// Mide una muestra de boda (como `jewel::measure_with`, con los sujetos de boda).
///
/// `face_pts`: narices de las personas que miran a cámara (pose); `back_pts`: hombros de las que están de
/// espaldas; `face_seen`: la pose vio una cara.
#[allow(clippy::too_many_arguments)]
pub fn measure(
    t: f64,
    frame: &Frame,
    pre: FrameMaps,
    dets: &[Det],
    state: &mut MeasureState,
    opts: &MeasureOpts,
    segmenter: Option<&mut dyn Detector>,
    face_seen: bool,
    face_pts: &[(f64, f64)],
    back_pts: &[(f64, f64)],
) -> Sample {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let FrameMaps { gray, maps, exposure: ex, dx, dy } = pre;
    // `lap_scale` < 1 es la ruta de GPU (NVDEC + scale_cuda lanczos), corregida con `GPU_LAP_SCALE` = 1/1.103,
    // medido en joyería. En personas (clips verticales Sony 4K e iPhone HEVC) el lanczos solo sale ~5 % más
    // nítido que el "area" de CPU (mediana GPU/CPU de la nitidez del sujeto = 0.95 tras la corrección, 669
    // muestras): sin este ajuste la GPU aprobaba ~12 % menos tiempo que la CPU con el mismo material.
    let ls = if opts.lap_scale < 1.0 { opts.lap_scale * GPU_AJUSTE } else { opts.lap_scale };
    let mut s = Sample {
        t,
        edge_density: metrics::density_full(&maps),
        lap_var: metrics::laplacian_variance(&maps.lap, None) * ls * ls,
        mean_luma: ex.mean_luma,
        clip_black: ex.clip_black,
        clip_white: ex.clip_white,
        cam_dx: dx,
        cam_dy: dy,
        anisotropy: 1.0,
        ..Default::default()
    };
    // nitidez del cuadro entero (planos sin sujeto: lugar, decoración), en una rejilla de 1 de cada 2 px
    let mut vals: Vec<f32> = Vec::with_capacity(w * h / 4);
    for y in (1..h.saturating_sub(1)).step_by(2) {
        for x in (1..w.saturating_sub(1)).step_by(2) {
            vals.push(maps.lap.data[y * w + x].abs());
        }
    }
    s.extra.insert("frame_lap_p99".into(), num(metrics::percentile(&mut vals, 99.0) * ls));

    let people: Vec<&Det> = dets.iter().filter(|d| es_cuerpo(&d.label) && d.conf >= 0.25).collect();
    let faces: Vec<[i64; 4]> = dets.iter().filter(|d| es_cara(&d.label) && d.conf >= 0.25).map(|d| d.bbox).collect();
    s.person = !people.is_empty();
    s.face = face_seen || !faces.is_empty();
    let area = |b: &[i64; 4]| ((b[2] - b[0]) * (b[3] - b[1])) as f64 / (w * h) as f64;
    if let Some(m) = people.iter().map(|d| area(&d.bbox)).reduce(f64::max) {
        s.person_frac = m;
    }
    s.extra.insert("face_frac".into(), num(faces.iter().map(area).fold(0.0, f64::max)));

    let mut gs: Vec<(Det, Vec<String>, [i64; 4])> = Vec::new();
    for (main, extras) in groups(dets, opts.min_conf) {
        let b = main.bbox;
        let r = [b[0].max(0), b[1].max(0), b[2].min(w as i64), b[3].min(h as i64)];
        if r[2] - r[0] >= 4 && r[3] - r[1] >= 4 {
            gs.push((main, extras, r));
        }
    }
    s.n_jewels = gs.len() as i64;
    let prev_boxes = std::mem::take(&mut state.prev_boxes);
    let had_prev = state.has_prev;
    state.has_prev = true;
    if gs.is_empty() {
        return s;
    }
    let ur = |r: &[i64; 4]| [r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize];

    // fondo = todo lo que queda fuera de cada caja agrandada un 25 %
    let mut outside = vec![1u8; w * h];
    for (_, _, r) in &gs {
        let (gx, gy) = (((r[2] - r[0]) as f64 * 0.25) as i64, ((r[3] - r[1]) as f64 * 0.25) as i64);
        let (ya, yb) = ((r[1] - gy).max(0) as usize, (r[3] + gy).min(h as i64) as usize);
        let (xa, xb) = ((r[0] - gx).max(0) as usize, (r[2] + gx).min(w as i64) as usize);
        for y in ya..yb {
            outside[y * w + xa..y * w + xb].fill(0);
        }
    }
    let bg = if outside.iter().any(|&v| v != 0) { metrics::density(&maps, Region::Mask(&outside)) } else { 0.0 };

    // siluetas solo donde pueden cambiar el veredicto (los más seguros primero, como mucho SAM_MAX)
    let boxes: Vec<[i64; 4]> = gs.iter().map(|g| g.2).collect();
    let mut sils: Vec<Option<Vec<u8>>> = vec![None; gs.len()];
    let reused = if segmenter.is_some() { jewel::reuse_silhouettes(&state.sil, t, &boxes, w, h) } else { None };
    if let Some(r) = reused {
        sils = r;
    } else if let Some(seg) = segmenter {
        let mut want = Vec::new();
        for (i, (_, _, r)) in gs.iter().enumerate() {
            if want.len() < jewel::SAM_MAX && metrics::laplacian_peak(&maps.lap, Region::Rect(ur(r)), 99.0) * ls >= SHARP_LAP / 2.0 {
                want.push(i);
            }
        }
        for i in want {
            let r = gs[i].2;
            // un fallo de SAM se cuenta (la caché lo apunta) y la caja se mide entera, como en joyas
            let m = match seg.silhouette(frame, [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]) {
                Ok(m) => Some(m).filter(|m| m.len() == w * h),
                Err(_) => {
                    state.sam_fallos += 1;
                    None
                }
            };
            sils[i] = m.and_then(|m| {
                let [x0, y0, x1, y1] = ur(&r);
                let mut cut = vec![0u8; w * h];
                let mut n = 0usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        if m[y * w + x] != 0 {
                            cut[y * w + x] = 1;
                            n += 1;
                        }
                    }
                }
                (n >= 30).then_some(cut)
            });
        }
        state.sil = SilCache::new(t, boxes.clone(), sils.clone());
    }

    for ((main, extras, r), sil) in gs.iter().zip(&sils) {
        let rr = ur(r);
        let [x0, y0, x1, y1] = rr;
        let grown;
        let (region, masked, mask_frac) = match sil {
            Some(sil) => {
                grown = jewel::grow_in_rect(sil, w, h, rr);
                let n = grown.iter().filter(|&&v| v != 0).count();
                (Region::RectMask(rr, &grown), true, n as f64 / ((x1 - x0) * (y1 - y0)) as f64)
            }
            None => (Region::Rect(rr), false, 0.0),
        };
        let edge = metrics::density(&maps, region);
        let (cx, cy) = ((r[0] + r[2]) as f64 / 2.0, (r[1] + r[3]) as f64 / 2.0);
        let diag = ((r[2] - r[0]) as f64).hypot((r[3] - r[1]) as f64);
        let mut speed = 0.0;
        if had_prev && !prev_boxes.is_empty() && opts.dt > 0.0 {
            let mut best: Option<(f64, f64, f64)> = None;
            for b in &prev_boxes {
                let c = ((b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0);
                let d = (c.0 - cx).powi(2) + (c.1 - cy).powi(2);
                if best.map(|x| d < x.2).unwrap_or(true) {
                    best = Some((c.0, c.1, d));
                }
            }
            let (px, py, _) = best.unwrap();
            speed = (cx - px).hypot(cy - py) / diag.max(1.0) / opts.dt;
        }
        let (luma, recorte) = exposicion_region(&gray, w, region);
        let worn = !es_cuerpo(&main.label) && people.iter().any(|p| p.bbox[0] as f64 <= cx && cx <= p.bbox[2] as f64 && p.bbox[1] as f64 <= cy && cy <= p.bbox[3] as f64);
        let mut extra = Map::new();
        extra.insert("luma".into(), num(luma));
        extra.insert("clip".into(), num(recorte));
        extra.insert("faced".into(), json!(tiene_cara(&main.label, r, &faces, face_pts, back_pts)));
        s.jewels.push(Jewel {
            label: main.label.clone(),
            with_labels: Some(extras.clone()),
            conf: main.conf,
            bbox: *r,
            zoom: Some(main.zoom),
            area_frac: Some(area(r)),
            edge: Some(edge),
            ratio: Some(edge / bg.max(1e-3)),
            masked: Some(masked),
            mask_frac: Some(mask_frac),
            lap_var: Some(metrics::laplacian_variance(&maps.lap, Some(region)) * ls * ls),
            lap_p99: Some(metrics::laplacian_peak(&maps.lap, region, 99.0) * ls),
            anisotropy: Some(metrics::blur_anisotropy(&gray, w, rr)),
            speed: Some(speed),
            touches_edge: Some(r[0] <= 2 || r[1] <= 2 || r[2] >= w as i64 - 2 || r[3] >= h as i64 - 2),
            worn: Some(worn),
            extra,
            ..Default::default()
        });
    }
    state.prev_boxes = s.jewels.iter().map(|j| j.bbox).collect();
    s.n_sharp = s.jewels.iter().filter(|j| j.lap_p99.unwrap_or(0.0) >= SHARP_LAP).count() as i64;
    let mut hero = 0;
    for i in 1..s.jewels.len() {
        if hero_priority(&s.jewels[i], w as f64, h as f64) > hero_priority(&s.jewels[hero], w as f64, h as f64) {
            hero = i;
        }
    }
    aplicar_principal(&mut s, hero);
    s.bg_edge_density = bg;
    s
}

/// Las medidas de la muestra pasan a ser las del sujeto `i` (`HERO_FIELDS` de Python).
fn aplicar_principal(s: &mut Sample, i: usize) {
    let j = s.jewels[i].clone();
    s.jewel = true;
    s.label = j.label.clone();
    s.with_labels = j.with_labels.clone().unwrap_or_default();
    s.conf = j.conf;
    s.bbox = j.bbox.to_vec();
    s.area_frac = j.area_frac.unwrap_or(0.0);
    s.touches_edge = j.touches_edge.unwrap_or(false);
    s.worn = j.worn.unwrap_or(false);
    s.jewel_edge_density = j.edge.unwrap_or(0.0);
    s.focus_ratio = j.ratio.unwrap_or(0.0);
    s.jewel_lap_var = j.lap_var.unwrap_or(0.0);
    s.anisotropy = j.anisotropy.unwrap_or(1.0);
    s.jewel_speed = j.speed.unwrap_or(0.0);
    s.jewel_lap_p99 = j.lap_p99.unwrap_or(0.0);
    s.masked = j.masked.unwrap_or(false);
    s.mask_frac = j.mask_frac.unwrap_or(0.0);
    for (de, a) in [("luma", "jewel_luma"), ("clip", "jewel_clip"), ("faced", "faced")] {
        if let Some(v) = j.extra.get(de) {
            s.extra.insert(a.into(), v.clone());
        }
    }
}

/// Detección de una muestra que cambió: sujetos, momento (CLIP sobre el cuadro entero), pose si hay
/// alguien y, con `zoom`, la segunda mirada a cabezas y manos. Devuelve detecciones, poses y el momento
/// en bruto (`moment_raw`, `moment_conf_raw`, `moment_top`).
pub fn detect_sample(frame: &Frame, detail: Option<&Frame>, zoom: bool, detector: &mut dyn Detector) -> anyhow::Result<(Vec<Det>, Vec<Pose>, Map<String, Value>)> {
    let mut dets: Vec<Det> = detector.detect(frame)?.iter().map(Det::from_detection).filter(|d| d.conf >= DETECT_CONF).collect();
    let momento = momento_en_bruto(&detector.moments(frame)?);
    let kps: Vec<Pose> = if dets.iter().any(|d| es_cuerpo(&d.label) && d.conf >= 0.4) {
        detector.pose(frame)?.into_iter().filter(|p| p.conf >= crate::analyse::POSE_CONF).collect()
    } else {
        Vec::new()
    };
    if zoom {
        let det_frame = detail.unwrap_or(frame);
        let k = det_frame.width as f64 / frame.width as f64;
        for r in zoom_regions(&kps, frame.width, frame.height, 5) {
            let c = recortar(det_frame, (r[0] as f64 * k) as usize, (r[1] as f64 * k) as usize, (r[2] as f64 * k) as usize, (r[3] as f64 * k) as usize);
            if c.width == 0 || c.height == 0 {
                continue;
            }
            for d in detector.detect(&c)? {
                let mut d = Det::from_detection(&d);
                if d.conf < DETECT_CONF || !es_sujeto(&d.label) {
                    continue;
                }
                let b = d.bbox;
                d.bbox = [(r[0] as f64 + b[0] as f64 / k) as i64, (r[1] as f64 + b[1] as f64 / k) as i64, (r[0] as f64 + b[2] as f64 / k) as i64, (r[1] as f64 + b[3] as f64 / k) as i64];
                d.zoom = true;
                dets.push(d);
            }
        }
    }
    Ok((dets, kps, momento))
}

/// Recorta una región de un cuadro BGR.
fn recortar(f: &Frame, x0: usize, y0: usize, x1: usize, y1: usize) -> Frame {
    let (x1, y1) = (x1.min(f.width as usize), y1.min(f.height as usize));
    let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let mut data = Vec::with_capacity(cw * ch * 3);
    for y in y0..y0 + ch {
        let row = y * f.width as usize * 3;
        data.extend_from_slice(&f.data[row + x0 * 3..row + (x0 + cw) * 3]);
    }
    Frame { width: cw as u32, height: ch as u32, data }
}

/// Opinión de CLIP sobre un cuadro (todas las probabilidades) → los 5 momentos más probables.
pub fn momento_en_bruto(probs: &[(String, f32)]) -> Map<String, Value> {
    let mut m = Map::new();
    if probs.is_empty() {
        return m;
    }
    let mut orden: Vec<&(String, f32)> = probs.iter().collect();
    orden.sort_by(|a, b| b.1.total_cmp(&a.1));
    let r4 = |v: f32| ((v as f64) * 10000.0).round() / 10000.0;
    m.insert("moment_raw".into(), json!(orden[0].0));
    m.insert("moment_conf_raw".into(), json!(r4(orden[0].1)));
    let mut top = Map::new();
    for (k, v) in orden.iter().take(5) {
        top.insert(k.clone(), json!(r4(*v)));
    }
    m.insert("moment_top".into(), Value::Object(top));
    m
}

// =================================================================================================
// Momentos
// =================================================================================================

/// Momentos de la boda en el orden del día (las etiquetas de `clip_moments.json`), más "otro".
pub const MOMENTOS: [&str; 17] = [
    "preparativos", "detalles", "lugar", "ceremonia", "votos", "anillos", "beso", "cóctel", "retratos", "entrada", "primer baile", "discursos",
    "banquete", "pastel", "fiesta", "salida", "otro",
];
/// Ventana de mayoría: un momento no cambia de una muestra a la siguiente.
pub const MOMENT_WINDOW_S: f64 = 3.0;
/// Probabilidad media del ganador en la ventana por debajo de la cual queda "otro".
pub const MOMENT_MIN: f64 = 0.18;
/// Alguien habla (audio): votos y discursos más probables…
pub const VOICE_BOOST: f64 = 1.6;
/// …y menos en silencio.
pub const NO_VOICE: f64 = 0.7;
const VOICE_MOMENTS: [&str; 2] = ["votos", "discursos"];
/// Momento cuando ninguno está claro.
pub const OTRO: &str = "otro";

/// Votos de una muestra: su top 5 de CLIP con el empuje de la voz.
fn votos(s: &Sample) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = match s.extra.get("moment_top").and_then(Value::as_object) {
        Some(top) => top.iter().filter_map(|(k, p)| p.as_f64().map(|p| (k.clone(), p))).collect(),
        None => match xs(&s.extra, "moment_raw") {
            Some(m) if !m.is_empty() => vec![(m.to_string(), xf(&s.extra, "moment_conf_raw").unwrap_or(0.5))],
            _ => Vec::new(),
        },
    };
    if let Some(voz) = xb(&s.extra, "voice") {
        for (k, p) in v.iter_mut() {
            if VOICE_MOMENTS.contains(&k.as_str()) {
                *p *= if voz { VOICE_BOOST } else { NO_VOICE };
            }
        }
    }
    v
}

/// Muestras a cada lado en la ventana de mayoría de los momentos: `int(round(3 s × fps / 2))` de Python,
/// que redondea los empates al par (4.5 → 4), no hacia arriba como `f64::round`.
fn media_ventana_momentos(per_second: f64) -> usize {
    (MOMENT_WINDOW_S * per_second / 2.0).round_ties_even().max(0.0) as usize
}

/// Momento de cada muestra = mayoría ponderada de las opiniones de CLIP en ~3 s alrededor, con el empuje
/// de la voz (en sitio, idempotente: las opiniones en bruto se quedan en `moment_top` / `moment_raw`).
/// Las cachés sin momentos no se tocan.
pub fn stabilize_moments(samples: &mut [Sample], per_second: f64) {
    if !samples.iter().any(|s| s.extra.contains_key("moment_top") || s.extra.contains_key("moment_raw")) {
        return;
    }
    let votes: Vec<Vec<(String, f64)>> = samples.iter().map(votos).collect();
    let half = media_ventana_momentos(per_second);
    let n = samples.len();
    for i in 0..n {
        let (a, b) = (i.saturating_sub(half), (i + half + 1).min(n));
        let mut acc: Vec<(String, f64)> = Vec::new();
        for v in &votes[a..b] {
            for (k, p) in v {
                match acc.iter_mut().find(|x| x.0 == *k) {
                    Some(e) => e.1 += p,
                    None => acc.push((k.clone(), *p)),
                }
            }
        }
        let s = &mut samples[i];
        if acc.is_empty() {
            s.extra.insert("moment".into(), json!(OTRO));
            s.extra.insert("moment_conf".into(), json!(0.0));
            continue;
        }
        let mut best = &acc[0];
        for e in &acc[1..] {
            if e.1 > best.1 {
                best = e;
            }
        }
        let conf = best.1 / (b - a) as f64;
        let m = if conf >= MOMENT_MIN { best.0.clone() } else { OTRO.to_string() };
        s.extra.insert("moment".into(), json!(m));
        s.extra.insert("moment_conf".into(), json!((conf * 1000.0).round() / 1000.0));
    }
}

/// El editor dijo qué momento muestra el clip: todas sus muestras lo toman (en sitio).
pub fn force_moment(samples: &mut [Sample], label: &str) {
    for s in samples.iter_mut() {
        s.extra.insert("moment".into(), json!(label));
        s.extra.insert("moment_conf".into(), json!(1.0));
    }
}

/// Momentos de un tramo ponderados por confianza, el dominante primero (los que tienen ≥ 20 % del peso).
pub fn moments_present(window: &[Sample], min_share: f64) -> Vec<String> {
    let mut peso: Vec<(String, f64)> = Vec::new();
    for s in window {
        let m = momento(s);
        if m.is_empty() {
            continue;
        }
        let w = xf(&s.extra, "moment_conf").unwrap_or(0.0).max(0.05);
        match peso.iter_mut().find(|x| x.0 == m) {
            Some(e) => e.1 += w,
            None => peso.push((m.to_string(), w)),
        }
    }
    let total: f64 = peso.iter().map(|x| x.1).sum();
    peso.sort_by(|a, b| b.1.total_cmp(&a.1));
    if total <= 0.0 {
        return Vec::new();
    }
    peso.into_iter().filter(|x| x.1 >= min_share * total).map(|x| x.0).collect()
}

// =================================================================================================
// Etiquetas estables, temblor de cámara y sujeto principal
// =================================================================================================

fn familia(label: &str) -> u8 {
    if es_cara(label) {
        0
    } else if es_cuerpo(label) {
        1
    } else {
        2
    }
}

/// Sujeto principal de cada muestra por nitidez, centro, tipo y tamaño (en sitio, idempotente).
pub fn refine_subjects(samples: &mut [Sample], width: u32, height: u32) {
    let k = 960.0 / width.max(height).max(1) as f64;
    let (w, h) = (width as f64 * k, height as f64 * k);
    for s in samples.iter_mut() {
        if s.jewels.is_empty() {
            continue;
        }
        let mut hero = 0;
        for i in 1..s.jewels.len() {
            if hero_priority(&s.jewels[i], w, h) > hero_priority(&s.jewels[hero], w, h) {
                hero = i;
            }
        }
        let bg = s.bg_edge_density;
        aplicar_principal(s, hero);
        s.bg_edge_density = bg;
    }
}

/// Etiquetas estables en el tiempo (en sitio, idempotente): sujetos, temblor de cámara y momentos.
///
/// Cada sujeto físico conserva una etiqueta mientras esté en pantalla (persona/novia/novio parpadean): se
/// sigue por solape de caja dentro de su familia (caras, cuerpos, objetos) y toma la etiqueta mayoritaria
/// ponderada por confianza. La etiqueta cruda queda en `raw_label`.
pub fn stabilize_all(samples: &mut [Sample], per_second: f64, max_gap_s: f64) {
    struct Track {
        bbox: [i64; 4],
        t: f64,
        family: u8,
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
            let raw = j.raw_label.clone().unwrap();
            let fam = familia(&raw);
            let bx = j.bbox;
            let (cx, cy) = ((bx[0] + bx[2]) as f64 / 2.0, (bx[1] + bx[3]) as f64 / 2.0);
            let diag = (((bx[2] - bx[0]).pow(2) + (bx[3] - bx[1]).pow(2)) as f64).sqrt().max(1.0);
            let mut best: Option<usize> = None;
            let mut best_score = 0.0;
            for (ti, tr) in tracks.iter().enumerate() {
                if taken.contains(&ti) || tr.family != fam || t - tr.t > max_gap_s {
                    continue;
                }
                let ov = box_iou(&bx, &tr.bbox);
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
                    tracks.push(Track { bbox: bx, t, family: fam, votes: Vec::new(), members: Vec::new() });
                    tracks.len() - 1
                }
            };
            let tr = &mut tracks[best];
            taken.push(best);
            tr.bbox = bx;
            tr.t = t;
            match tr.votes.iter_mut().find(|(k, _)| *k == raw) {
                Some(e) => e.1 += j.conf,
                None => tr.votes.push((raw, j.conf)),
            }
            tr.members.push((si, ji));
        }
    }
    for tr in tracks {
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
    camera_shake(samples, per_second);
    stabilize_moments(samples, per_second);
}

fn mediana(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Temblor de cámara en cada muestra (en sitio): cuánto se aparta su desplazamiento de la mediana local.
/// Una panorámica suave o un gimbal dan un desplazamiento estable (≈ 0); un plano a mano tembloroso no.
/// Se expresa por cada 1/4 s, así se lee igual con cualquier `--fps`.
pub fn camera_shake(samples: &mut [Sample], per_second: f64) {
    let dx: Vec<f64> = samples.iter().map(|s| s.cam_dx).collect();
    let dy: Vec<f64> = samples.iter().map(|s| s.cam_dy).collect();
    let n = samples.len();
    for i in 0..n {
        let (a, b) = (i.saturating_sub(2), (i + 3).min(n));
        let (mx, my) = (mediana(&dx[a..b]), mediana(&dy[a..b]));
        let v = ((dx[i] - mx).powi(2) + (dy[i] - my).powi(2)).sqrt() * per_second / 4.0;
        samples[i].extra.insert("shake".into(), json!((v * 100.0).round() / 100.0));
    }
}

// =================================================================================================
// Criterios y puntuación
// =================================================================================================

/// Temblor de cámara (px a 960 por 1/4 s) que anula "cámara estable".
pub const SHAKE_REF: f64 = 15.0;
/// Luma media del sujeto: ≤ LUMA_LO subexpuesto (0), ≥ LUMA_OK bien; por encima de LUMA_HI, quemado.
pub const LUMA_LO: f64 = 0.10;
pub const LUMA_OK: f64 = 0.22;
pub const LUMA_HI: f64 = 0.93;
/// El sujeto principal es una persona de espaldas.
pub const BACK_FACTOR: f64 = 0.75;
/// Momentos que también se juzgan por el cuadro entero (lugar, decoración).
pub const SCENE_MOMENTS: [&str; 2] = ["lugar", "detalles"];
/// Un plano de ambiente puntúa algo por debajo de uno con cara.
pub const SCENE_FACTOR: f64 = 0.85;
/// Nitidez del cuadro entero de blando a nítido.
pub const FRAME_LAP_SOFT: f64 = 60.0;
pub const FRAME_LAP_SHARP: f64 = 150.0;

/// Pesos de la edición, en el orden de la ventana (la nitidez manda; temblor y exposición importan).
pub const PESOS: [(&str, f64); 7] = [("sharp", 0.70), ("exposure", 0.15), ("steady", 0.10), ("size", 0.10), ("aniso", 0.05), ("still", 0.0), ("ratio", 0.0)];

/// Criterios de la edición: "bodas" (todos los tramos buenos, por momento), "social" (highlight: cortes
/// de hasta 4 s, los 3 mejores por clip) y "todo" (permisivo).
pub fn presets() -> Vec<Preset> {
    let d = Preset {
        name: "bodas".into(),
        min_area_frac: 0.0008,
        lap_soft: 35.0,
        lap_sharp: 90.0,
        size_ref: 0.01,
        // la piel lisa tiene pocos bordes: una cara parecería "más blanda" que cualquier fondo con textura;
        // la puerta de nitidez ya atrapa un foco perdido
        exclude_background_focus: false,
        weights: PESOS.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        ..Preset::default()
    };
    vec![
        Preset { score_threshold: 0.5, max_segment_seconds: 60.0, max_segments_per_clip: 99, gap_tolerance_seconds: 1.0, dedupe: false, ..d.clone() },
        Preset { name: "social".into(), score_threshold: 0.6, max_segment_seconds: 4.0, max_segments_per_clip: 3, ..d.clone() },
        Preset { name: "todo".into(), score_threshold: 0.4, max_segment_seconds: 60.0, max_segments_per_clip: 99, gap_tolerance_seconds: 1.5, dedupe: false, ..d },
    ]
}

/// Exposición del sujeto: luma en un rango sano y pocos píxeles negros o quemados en su silueta.
fn exposicion_sujeto(s: &Sample) -> f64 {
    let Some(lum) = xf(&s.extra, "jewel_luma") else { return 1.0 - clip01((s.clip_black + s.clip_white) * 5.0) };
    let nivel = if lum < LUMA_OK { clip01((lum - LUMA_LO) / (LUMA_OK - LUMA_LO)) } else { 1.0 - clip01((lum - LUMA_HI) / 0.05) };
    nivel * (1.0 - clip01(xf(&s.extra, "jewel_clip").unwrap_or(0.0) * 4.0))
}

/// "Cámara estable" de una muestra (0..1).
fn estable(s: &Sample) -> f64 {
    1.0 - clip01(xf(&s.extra, "shake").unwrap_or(0.0) / SHAKE_REF)
}

/// Cada criterio en 0..1 (las claves de los pesos).
pub fn components(s: &Sample, p: &Preset) -> Vec<(&'static str, f64)> {
    let sharp = if s.jewel_lap_p99 != 0.0 {
        clip01((s.jewel_lap_p99 - p.lap_soft) / (p.lap_sharp - p.lap_soft))
    } else {
        clip01(s.jewel_edge_density / if s.masked { p.masked_edge_ref } else { p.jewel_edge_ref })
    };
    vec![
        ("sharp", sharp),
        ("ratio", clip01((s.focus_ratio - p.ratio_lo) / (p.ratio_hi - p.ratio_lo))),
        ("still", 1.0 - clip01(s.jewel_speed / p.speed_max)),
        ("aniso", clip01(s.anisotropy / p.aniso_ref)),
        ("exposure", exposicion_sujeto(s)),
        ("size", clip01(s.area_frac / p.size_ref)),
        ("steady", estable(s)),
    ]
}

/// Criterios de un cuadro juzgado entero (lugar, decoración): nitidez, exposición y cámara estable.
fn scene_components(s: &Sample) -> Vec<(&'static str, f64)> {
    vec![
        ("sharp", clip01((xf(&s.extra, "frame_lap_p99").unwrap_or(0.0) - FRAME_LAP_SOFT) / (FRAME_LAP_SHARP - FRAME_LAP_SOFT))),
        ("exposure", 1.0 - clip01((s.clip_black + s.clip_white) * 3.0)),
        ("steady", estable(s)),
    ]
}

/// La nitidez es la puerta; el resto solo la modula (hasta `sharp_floor`). El peso de "sharp" fija lo
/// dura que es: 0 la ignora, 0.7 es lineal, más alto es más estricto.
fn gated(c: &[(&str, f64)], p: &Preset) -> f64 {
    let val = |k: &str| c.iter().find(|x| x.0 == k).map(|x| x.1);
    let others: Vec<(f64, f64)> = p.weights.iter().filter(|(k, w)| k != "sharp" && *w > 0.0).filter_map(|(k, w)| val(k).map(|v| (*w, v))).collect();
    let rest = if others.is_empty() { 1.0 } else { pysum(others.iter().map(|(w, v)| w * v)) / pysum(others.iter().map(|(w, _)| *w)) };
    let ws = p.weight("sharp");
    let gate = if ws > 0.0 { val("sharp").unwrap_or(0.0).powf(ws / 0.7) } else { 1.0 };
    if others.is_empty() { gate } else { gate * (p.sharp_floor + (1.0 - p.sharp_floor) * rest) }
}

fn scene_score(s: &Sample, p: &Preset) -> f64 {
    if !SCENE_MOMENTS.contains(&momento(s)) || xf(&s.extra, "frame_lap_p99").unwrap_or(0.0) == 0.0 {
        return 0.0;
    }
    gated(&scene_components(s), p) * SCENE_FACTOR
}

/// Puntuación de una muestra y sus avisos (`sin_sujeto`, `plano_de_ambiente`, `sujeto_cortado`,
/// `de_espaldas`, `fondo_enfocado`, `barrido_por_movimiento`).
pub fn score_sample(s: &Sample, p: &Preset) -> (f64, Vec<&'static str>) {
    let scene = scene_score(s, p);
    if !s.jewel || s.conf < p.min_conf || s.area_frac < p.min_area_frac {
        return if scene > 0.0 { (scene, vec!["plano_de_ambiente"]) } else { (0.0, vec!["sin_sujeto"]) };
    }
    let mut flags = Vec::new();
    let mut score = gated(&components(s, p), p);
    if p.exclude_background_focus && s.focus_ratio < 0.8 && s.bg_edge_density > 0.01 {
        flags.push("fondo_enfocado");
        score = 0.0;
    }
    if p.blur_factor < 1.0 && (s.jewel_speed > p.blur_speed || s.anisotropy < p.blur_aniso) {
        flags.push("barrido_por_movimiento");
        score *= p.blur_factor;
    }
    if s.touches_edge && !es_cuerpo(&s.label) {
        flags.push("sujeto_cortado");
        score *= p.cut_jewel_factor;
    }
    if es_cuerpo(&s.label) && !xb(&s.extra, "faced").unwrap_or(true) {
        flags.push("de_espaldas");
        score *= BACK_FACTOR;
    }
    if scene > score {
        return (scene, vec!["plano_de_ambiente"]);
    }
    (score, flags)
}

/// Tamaño del plano por caras y personas: hablando > primer_plano > plano_medio > plano_general / detalle.
pub fn shot_type(window: &[Sample]) -> String {
    let n = window.len().max(1) as f64;
    if window.iter().filter(|s| s.speaking.unwrap_or(false)).count() as f64 / n > 0.33 {
        return "hablando".into();
    }
    let cara = |s: &Sample| xf(&s.extra, "face_frac").filter(|v| *v > 0.0).unwrap_or_else(|| s.jewels.iter().filter(|j| es_cara(&j.label)).map(|j| j.area_frac.unwrap_or(0.0)).fold(0.0, f64::max));
    let face = mediana(&window.iter().map(cara).collect::<Vec<_>>());
    if face >= 0.025 {
        return "primer_plano".into();
    }
    if face >= 0.004 || mediana(&window.iter().map(|s| s.person_frac).collect::<Vec<_>>()) >= 0.15 {
        return "plano_medio".into();
    }
    if window.iter().filter(|s| s.person).count() as f64 / n >= 0.5 {
        return "plano_general".into();
    }
    let objetos = window.iter().filter(|s| s.jewel && !es_cara(&s.label) && !es_cuerpo(&s.label)).count() as f64;
    if objetos / n >= 0.5 { "detalle".into() } else { "plano_general".into() }
}

/// Nombre de cada plano en la interfaz.
pub fn nombre_plano(k: &str) -> &str {
    match k {
        "hablando" => "Hablando",
        "primer_plano" => "Primer plano",
        "plano_medio" => "Plano medio",
        "plano_general" => "Plano general",
        "detalle" => "Detalle",
        o => o,
    }
}

/// Parte la racha a..b donde cambia el momento; los trozos más cortos que `min_len` se unen al vecino.
fn moment_pieces(a: usize, b: usize, samples: &[Sample], min_len: usize) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut start = a;
    for k in a + 1..=b {
        if momento(&samples[k]) != momento(&samples[k - 1]) {
            pieces.push((start, k - 1));
            start = k;
        }
    }
    pieces.push((start, b));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (pa, pb) in pieces {
        match merged.last_mut() {
            Some(last) if pb - pa + 1 < min_len => last.1 = pb,
            _ => merged.push((pa, pb)),
        }
    }
    if merged.len() > 1 && merged[0].1 - merged[0].0 + 1 < min_len {
        merged[1].0 = merged[0].0;
        merged.remove(0);
    }
    merged
}

/// "sujeto con otro y otro" (`_full_label` de Python): la etiqueta de la curva de una muestra.
fn etiqueta_completa(s: &Sample) -> String {
    let extra: Vec<&str> = s.with_labels.iter().filter(|x| **x != s.label).map(String::as_str).collect();
    if !extra.is_empty() && !s.label.is_empty() { format!("{} con {}", s.label, extra.join(" y ")) } else { s.label.clone() }
}

/// (t, puntuación, avisos, etiqueta) por muestra con la puntuación de BODAS ([`score_sample`]) y la
/// misma mediana de consistencia que `scoring::scored_samples`. No depende de la edición del proceso:
/// los tramos de boda se puntúan siempre como boda (también en las pruebas, que corren en joyas).
pub fn scored_samples(samples: &[Sample], p: &Preset, per_second: f64) -> Vec<scoring::Scored> {
    let mut scored: Vec<scoring::Scored> = samples
        .iter()
        .map(|s| {
            let (sc, fl) = score_sample(s, p);
            (s.t, sc, fl, etiqueta_completa(s))
        })
        .collect();
    // consistencia: cada instante se juzga por la mediana de ~1 s alrededor (igual que en joyas)
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

/// Etiqueta más frecuente de un tramo (en un empate, la primera que aparece), como hace
/// `scoring::build_segments` en joyas; vacía si ninguna muestra tiene etiqueta.
fn etiqueta_mas_comun(window: &[scoring::Scored]) -> String {
    let labels: Vec<&String> = window.iter().map(|r| &r.3).filter(|l| !l.is_empty()).collect();
    let mut best: Option<(&String, usize)> = None;
    for l in &labels {
        let c = labels.iter().filter(|x| **x == *l).count();
        if best.map(|b| c > b.1).unwrap_or(true) {
            best = Some((l, c));
        }
    }
    best.map(|b| b.0.clone()).unwrap_or_default()
}

/// Tramos aprobados de un clip de boda: como en joyas, pero un tramo nunca cruza dos momentos y cada
/// uno lleva su momento dominante; el filtro de tipos (`jewel_types`) filtra por momento.
pub fn build_segments(samples: &[Sample], p: &Preset, per_second: f64, duration: f64) -> Vec<Segment> {
    let step = 1.0 / per_second;
    // puntuación de bodas siempre, sea cual sea la edición del proceso
    let scored = scored_samples(samples, p, per_second);
    let ok: Vec<bool> = scored.iter().map(|r| r.1 >= p.score_threshold).collect();
    let n = scored.len();
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
    // asas solo en los bordes exteriores de cada racha: nunca pisan el tramo vecino de otro momento
    let mut windows: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (a, b) in runs {
        let mut pieces = Vec::new();
        for (mut ma, mb) in moment_pieces(a, b, samples, min_len) {
            let mut sub = Vec::new();
            while mb - ma + 1 > max_len {
                sub.push((ma, ma + max_len - 1));
                ma += max_len;
            }
            if mb - ma + 1 >= min_len || sub.is_empty() {
                sub.push((ma, mb));
            }
            pieces.extend(sub);
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
        if p.min_mean_factor != 0.0 && mean < p.score_threshold * p.min_mean_factor {
            continue;
        }
        let mut peak = &window[0];
        for r in &window[1..] {
            if r.1 > peak.1 {
                peak = r;
            }
        }
        let core = &samples[ca..=cb];
        let with_subject: Vec<&Sample> = core.iter().filter(|s| s.jewel).collect();
        let mut parts = Vec::new();
        if !with_subject.is_empty() {
            let cs: Vec<Vec<(&str, f64)>> = with_subject.iter().map(|s| components(s, p)).collect();
            for (k, (name, _)) in cs[0].iter().enumerate() {
                parts.push((name.to_string(), pysum(cs.iter().map(|c| c[k].1)) / cs.len() as f64));
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
        // el sujeto del mejor cuadro; si no tiene, el más frecuente del tramo (Python: max(set, key=count))
        let label = if !peak.3.is_empty() { peak.3.clone() } else { etiqueta_mas_comun(window) };
        let moments = moments_present(core, 0.2);
        segs.push(Segment {
            start: scored[a].0,
            end: duration.min(scored[b].0 + step),
            mean_score: mean,
            peak_score: peak.1,
            peak_t: peak.0,
            label,
            parts,
            flags,
            shot: shot_type(core),
            present: scoring::jewels_present(core, 0.15, 0.15),
            moment: moments.first().cloned().unwrap_or_default(),
            moments,
        });
    }
    if !p.shot_types.is_empty() {
        segs.retain(|s| p.shot_types.contains(&s.shot));
    }
    if !p.jewel_types.is_empty() {
        // en esta edición los "tipos" son momentos: cuenta cualquier momento con peso en el corte
        segs.retain(|s| if s.moments.is_empty() { p.jewel_types.contains(&s.moment) } else { s.moments.iter().any(|m| p.jewel_types.contains(m)) });
    }
    let by_t: std::collections::HashMap<u64, &Sample> = samples.iter().map(|s| (s.t.to_bits(), s)).collect();
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| segs[b].mean_score.total_cmp(&segs[a].mean_score));
    let mut chosen: Vec<usize> = Vec::new();
    for i in order {
        if chosen.len() >= p.max_segments_per_clip {
            break;
        }
        let seg = &segs[i];
        if chosen.iter().any(|&c| mismo_plano(by_t.get(&seg.peak_t.to_bits()).copied(), by_t.get(&segs[c].peak_t.to_bits()).copied(), samples, p)) {
            continue;
        }
        chosen.push(i);
    }
    let mut out: Vec<Segment> = chosen.into_iter().map(|i| segs[i].clone()).collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Dos cortes se parecen: mismo momento, sujeto en el mismo sitio y tamaño, y la cámara apenas se movió.
fn mismo_plano(a: Option<&Sample>, b: Option<&Sample>, samples: &[Sample], p: &Preset) -> bool {
    let (Some(a), Some(b)) = (a, b) else { return false };
    if !p.dedupe || a.bbox.len() < 4 || b.bbox.len() < 4 || momento(a) != momento(b) {
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

/// Motivo corto de un tramo de boda, p. ej. "[votos · Primer plano] sujetos: cara, novia · sujeto nítido, cámara estable".
pub fn describe(seg: &Segment) -> String {
    let part = |k: &str| seg.parts.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
    let mut words: Vec<String> = Vec::new();
    if seg.parts.is_empty() {
        words.push("plano de ambiente".into());
    } else {
        let sh = part("sharp").unwrap_or(0.0);
        words.push(if sh >= 0.8 { "sujeto nítido" } else if sh >= 0.5 { "sujeto algo blando" } else { "sujeto blando" }.into());
        if let Some(st) = part("steady") {
            words.push(if st >= 0.8 { "cámara estable" } else if st >= 0.5 { "algo de temblor" } else { "cámara temblorosa" }.into());
        }
        if part("exposure").unwrap_or(1.0) < 0.6 {
            words.push("exposición dudosa".into());
        }
        if part("size").unwrap_or(1.0) < 0.5 {
            words.push("sujeto pequeño en cuadro".into());
        }
    }
    words.extend(seg.flags.iter().filter(|(k, _)| k != "sin_sujeto" && k != "plano_de_ambiente").map(|(k, v)| format!("{k} x{v}")));
    let sujetos = if seg.present.is_empty() { String::new() } else { format!("sujetos: {} · ", seg.present.join(", ")) };
    let m = if seg.moment.is_empty() { OTRO } else { &seg.moment };
    format!("[{} · {}] {}{}", m, nombre_plano(&seg.shot), sujetos, words.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(b: [i64; 4], conf: f64, label: &str) -> Det {
        Det { bbox: b, conf, label: label.into(), zoom: false, verified: false }
    }

    #[test]
    fn grupos_separan_cara_y_cuerpo() {
        // novia y persona sobre el mismo cuerpo se juntan (gana novia); la cara queda aparte
        let d = vec![det([100, 50, 300, 500], 0.9, "persona"), det([105, 55, 300, 495], 0.6, "novia"), det([170, 70, 230, 140], 0.5, "cara")];
        let g = groups(&d, 0.15);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].0.label, "novia");
        assert_eq!(g[0].1, vec!["persona".to_string()]);
        assert_eq!(g[1].0.label, "cara");
    }

    fn muestra(t: f64, m: &str, voz: bool) -> Sample {
        let mut s = Sample { t, ..Default::default() };
        s.extra.insert("moment_top".into(), json!({m: 0.5, "retratos": 0.3, "discursos": 0.2}));
        s.extra.insert("voice".into(), json!(voz));
        s
    }

    #[test]
    fn momentos_estables_y_voz() {
        let mut v: Vec<Sample> = (0..40).map(|i| muestra(i as f64 / 4.0, if i < 20 { "preparativos" } else { "votos" }, i >= 20)).collect();
        // un cuadro suelto de otro momento no cambia el momento de la ventana
        v[8].extra.insert("moment_top".into(), json!({"fiesta": 0.9}));
        stabilize_moments(&mut v, 4.0);
        assert_eq!(momento(&v[8]), "preparativos");
        assert_eq!(momento(&v[39]), "votos");
        force_moment(&mut v, "beso");
        assert!(v.iter().all(|s| momento(s) == "beso"));
    }

    /// Los tramos se parten por momento y se puntúan SIEMPRE como boda: las pruebas del núcleo corren en
    /// la edición de joyas, y el tramo de "lugar" (nadie a cuadro, el sitio nítido) solo sale con la
    /// puntuación de escena de bodas; con la de joyas, sin joya, valdría 0.
    #[test]
    fn tramos_se_parten_por_momento() {
        let p = presets().remove(0);
        let mut v: Vec<Sample> = (0..80)
            .map(|i| {
                let m = if i < 32 { "preparativos" } else if i < 56 { "votos" } else { "lugar" };
                let mut s = muestra(i as f64 / 4.0, m, (32..56).contains(&i));
                if i < 56 {
                    s.jewel = true;
                    s.conf = 0.8;
                    s.area_frac = 0.03;
                    s.label = "cara".into();
                    s.jewel_lap_p99 = 85.0;
                    s.anisotropy = 0.6;
                    s.bbox = vec![400, 100, 520, 230];
                    s.extra.insert("jewel_luma".into(), json!(0.5));
                } else {
                    // plano de ambiente: sin sujeto, el lugar nítido
                    s.extra.insert("frame_lap_p99".into(), json!(140.0));
                }
                s
            })
            .collect();
        stabilize_moments(&mut v, 4.0);
        camera_shake(&mut v, 4.0);
        // la curva es la de bodas (puntuación de escena incluida), no la de la edición del proceso
        let curva = scored_samples(&v, &p, 4.0);
        assert!((curva[70].1 - score_sample(&v[70], &p).0).abs() < 1e-12 && curva[70].1 > 0.5, "{:?}", curva[70]);
        assert_eq!(curva[70].2, vec!["plano_de_ambiente"]);
        let segs = build_segments(&v, &p, 4.0, 20.0);
        let momentos: Vec<&str> = segs.iter().map(|s| s.moment.as_str()).collect();
        assert_eq!(momentos, ["preparativos", "votos", "lugar"], "{segs:?}");
        assert!(describe(&segs[0]).starts_with("[preparativos · "));
        let lugar = segs.last().unwrap();
        assert!(lugar.parts.is_empty() && lugar.flags.iter().any(|f| f.0 == "plano_de_ambiente"), "{lugar:?}");
    }

    /// La ventana de mayoría de los momentos redondea como Python (empates al par): a 3 muestras/s,
    /// 3 × 3 / 2 = 4.5 → 4 (con `f64::round` salían 5).
    #[test]
    fn ventana_de_momentos_redondea_como_python() {
        assert_eq!(media_ventana_momentos(3.0), 4);
        assert_eq!(media_ventana_momentos(4.0), 6);
        assert_eq!(media_ventana_momentos(1.0), 2); // 1.5 → 2
        assert_eq!(media_ventana_momentos(5.0), 8); // 7.5 → 8
    }

    /// Si el mejor cuadro del tramo no tiene sujeto, el tramo toma la etiqueta más frecuente (como
    /// Python), no la primera que aparece.
    #[test]
    fn etiqueta_del_tramo_es_la_mas_frecuente() {
        let p = presets().remove(0);
        let v: Vec<Sample> = (0..16)
            .map(|i| {
                let mut s = Sample { t: i as f64 / 4.0, ..Default::default() };
                s.extra.insert("moment".into(), json!("lugar"));
                s.extra.insert("frame_lap_p99".into(), json!(140.0));
                // todas puntúan igual: el pico es la primera muestra, que no tiene etiqueta
                s.label = match i {
                    0 => "",
                    1 => "novia",
                    _ => "ramo",
                }
                .into();
                s
            })
            .collect();
        let segs = build_segments(&v, &p, 4.0, 4.0);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!(segs[0].peak_t, 0.0);
        assert_eq!(segs[0].label, "ramo");
    }

    #[test]
    fn de_espaldas_solo_con_pruebas() {
        let cuerpo = [100, 50, 300, 500];
        // una mano con anillos (detectada como "persona") sin pose: no es "de espaldas"
        assert!(tiene_cara("persona", &cuerpo, &[], &[], &[]));
        // hombros sin cara dentro del cuerpo: de espaldas
        assert!(!tiene_cara("persona", &cuerpo, &[], &[], &[(200.0, 150.0)]));
        // una cara arriba manda aunque la pose dude
        assert!(tiene_cara("novia", &cuerpo, &[[170, 60, 230, 120]], &[], &[(200.0, 150.0)]));
        // los objetos siempre "muestran cara"
        assert!(tiene_cara("anillos", &cuerpo, &[], &[], &[(200.0, 150.0)]));
    }

    #[test]
    fn escena_sin_sujeto_y_de_espaldas() {
        let p = presets().remove(0);
        let mut s = Sample { t: 0.0, ..Default::default() };
        s.extra.insert("moment".into(), json!("lugar"));
        s.extra.insert("frame_lap_p99".into(), json!(140.0));
        let (sc, fl) = score_sample(&s, &p);
        assert!(sc > 0.5 && fl == vec!["plano_de_ambiente"], "{sc} {fl:?}");
        let mut b = Sample { t: 0.0, jewel: true, conf: 0.8, area_frac: 0.2, label: "persona".into(), jewel_lap_p99: 90.0, anisotropy: 0.6, ..Default::default() };
        b.extra.insert("faced".into(), json!(false));
        let (_, fl) = score_sample(&b, &p);
        assert!(fl.contains(&"de_espaldas"));
    }
}
