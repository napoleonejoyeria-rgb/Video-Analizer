//! Exportaciones: línea de tiempo OTIO para DaVinci Resolve y CSV (port de `export.py`).
//!
//! El OTIO se escribe a mano con el mismo esquema, orden de claves y formato que la librería
//! `opentimelineio` de Python (sangría de 4, saltos `\r\n`, metadatos con claves ordenadas), así
//! Resolve lo importa igual y los archivos se pueden comparar con los de Python.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::pyfmt;
use crate::sampler::{start_frames, ClipInfo};
use crate::scoring::Segment;

/// Formas de la línea de tiempo exportada.
pub const MODES: &[(&str, &str)] = &[
    ("aprobados", "Solo los tramos aprobados, uno tras otro"),
    ("v1v2", "V2 tramos aprobados, V1 el resto de cada clip (sin duplicar)"),
    ("marcadores", "V1 clips completos, tramos aprobados como marcadores"),
];

/// Resolve no tiene marcador naranja (un ORANGE llega verde), así que los cortes más flojos van en rojo.
const MARKER_COLORS: &[(f64, &str)] = &[(0.8, "GREEN"), (0.65, "YELLOW"), (0.0, "RED")];

/// `RationalTime` de OTIO (valor y frecuencia), con su misma aritmética.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RationalTime {
    pub value: f64,
    pub rate: f64,
}

impl RationalTime {
    pub fn new(value: f64, rate: f64) -> Self {
        RationalTime { value, rate }
    }
    /// Valor a otra frecuencia (igual si es la misma).
    pub fn value_rescaled_to(&self, rate: f64) -> f64 {
        if rate == self.rate { self.value } else { self.value * rate / self.rate }
    }
    pub fn rescaled_to(&self, rate: f64) -> Self {
        RationalTime::new(self.value_rescaled_to(rate), rate)
    }
    fn secs(&self) -> f64 {
        self.value / self.rate
    }
    fn to_json(self) -> Value {
        json!({"OTIO_SCHEMA": "RationalTime.1", "rate": self.rate, "value": self.value})
    }
}

impl std::ops::Add for RationalTime {
    type Output = RationalTime;
    fn add(self, rhs: RationalTime) -> RationalTime {
        if self.rate < rhs.rate {
            RationalTime::new(self.value_rescaled_to(rhs.rate) + rhs.value, rhs.rate)
        } else {
            RationalTime::new(rhs.value_rescaled_to(self.rate) + self.value, self.rate)
        }
    }
}

impl std::ops::Sub for RationalTime {
    type Output = RationalTime;
    fn sub(self, rhs: RationalTime) -> RationalTime {
        if self.rate < rhs.rate {
            RationalTime::new(self.value_rescaled_to(rhs.rate) - rhs.value, rhs.rate)
        } else {
            RationalTime::new(self.value - rhs.value_rescaled_to(self.rate), self.rate)
        }
    }
}

impl PartialOrd for RationalTime {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        self.secs().partial_cmp(&o.secs())
    }
}

/// Segundos → tiempo a `rate`, redondeado al cuadro (empates al par, como `round()` de Python).
fn rt(seconds: f64, rate: f64) -> RationalTime {
    RationalTime::new(pyfmt::round_half_even(seconds * rate), rate)
}

fn time_range(start: RationalTime, dur: RationalTime) -> Value {
    json!({"OTIO_SCHEMA": "TimeRange.1", "duration": dur.to_json(), "start_time": start.to_json()})
}

/// Un elemento de pista: trozo de clip o hueco.
#[derive(Clone)]
enum Item {
    Clip { name: String, url: String, available: (RationalTime, RationalTime), range: (RationalTime, RationalTime), metadata: BTreeMap<String, Value> },
    Gap(RationalTime),
}

impl Item {
    fn to_json(&self) -> Value {
        match self {
            Item::Clip { name, url, available, range, metadata } => {
                let mut meta = Map::new();
                for (k, v) in metadata {
                    meta.insert(k.clone(), v.clone()); // AnyDictionary de OTIO: claves ordenadas
                }
                json!({
                    "OTIO_SCHEMA": "Clip.2",
                    "metadata": meta,
                    "name": name,
                    "source_range": time_range(range.0, range.1),
                    "effects": [],
                    "markers": [],
                    "enabled": true,
                    "color": null,
                    "media_references": {
                        "DEFAULT_MEDIA": {
                            "OTIO_SCHEMA": "ExternalReference.1",
                            "metadata": {},
                            "name": "",
                            "available_range": time_range(available.0, available.1),
                            "available_image_bounds": null,
                            "target_url": url
                        }
                    },
                    "active_media_reference_key": "DEFAULT_MEDIA"
                })
            }
            Item::Gap(d) => json!({
                "OTIO_SCHEMA": "Gap.1",
                "metadata": {},
                "name": "",
                "source_range": time_range(RationalTime::new(0.0, d.rate), *d),
                "effects": [],
                "markers": [],
                "enabled": true,
                "color": null
            }),
        }
    }
}

fn track_json(name: &str, kind: &str, items: &[Item]) -> Value {
    json!({
        "OTIO_SCHEMA": "Track.1",
        "metadata": {},
        "name": name,
        "source_range": null,
        "effects": [],
        "markers": [],
        "enabled": true,
        "color": null,
        "children": items.iter().map(Item::to_json).collect::<Vec<_>>(),
        "kind": kind
    })
}

/// Ordena las claves de los diccionarios anidados de metadatos (como el AnyDictionary de OTIO).
fn sorted_value(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let b: BTreeMap<String, Value> = m.into_iter().map(|(k, v)| (k, sorted_value(v))).collect();
            Value::Object(b.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted_value).collect()),
        o => o,
    }
}

/// Construye la línea de tiempo OTIO (como valor JSON) y cuenta los tramos colocados.
///
/// Resolve lee todos los tiempos del OTIO a la frecuencia de la línea de tiempo, diga lo que diga el
/// archivo, así que todo se escribe a una frecuencia que debe coincidir con la de Resolve.
/// `mode`: "aprobados" | "v1v2" | "marcadores". Cada tramo aprobado lleva además un marcador de
/// duración en la regla, coloreado por su puntuación, con lo que contiene en la nota. `audio`: cada
/// pista de video lleva su pista de audio enlazada, como una edición normal.
pub fn build_otio(results: &[(ClipInfo, Vec<Segment>)], name: &str, timeline_fps: Option<f64>, mode: &str, audio: bool) -> (Value, usize) {
    let rate = timeline_fps.unwrap_or_else(|| results.first().map(|r| r.0.fps.as_f64()).unwrap_or(30.0));
    // v1v2 parte cada clip: tramos aprobados en V2, el resto en V1, nunca los mismos cuadros dos veces
    let mut rest: Vec<Item> = Vec::new();
    let mut cuts: Vec<Item> = Vec::new();
    let mut markers: Vec<(RationalTime, RationalTime, &Segment)> = Vec::new();
    let mut count = 0usize;
    let mut link = 0i64;

    // un trozo de clip; Resolve enlaza los trozos de video y audio que comparten "Link Group ID"
    let mut piece = |info: &ClipInfo, tc0: RationalTime, whole: RationalTime, start: RationalTime, dur: RationalTime, seg: Option<&Segment>| {
        link += 1;
        let mut metadata = BTreeMap::new();
        metadata.insert("Resolve_OTIO".to_string(), json!({"Link Group ID": link}));
        if let Some(seg) = seg {
            let datos = if crate::edicion::es_bodas() {
                // edición Bodas: el momento del tramo y sus sujetos
                json!({"mean_score": seg.mean_score, "peak_score": seg.peak_score, "label": seg.label, "subjects": seg.present,
                       "moment": seg.moment, "moments": seg.moments, "shot": seg.shot})
            } else {
                json!({"mean_score": seg.mean_score, "peak_score": seg.peak_score, "label": seg.label, "jewels": seg.present})
            };
            metadata.insert("camcuts".to_string(), sorted_value(datos));
        }
        Item::Clip {
            name: info.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            url: info.path.to_string_lossy().to_string(),
            available: (tc0, whole),
            range: (tc0 + start, dur),
            metadata,
        }
    };

    let mut clock = rt(0.0, rate); // posición en la línea de tiempo
    for (info, segs) in results {
        // El tiempo de medio es el timecode de cámara: Resolve casa los rangos con el timecode del clip.
        let tc0 = RationalTime::new(start_frames(info), info.fps.as_f64()).rescaled_to(rate);
        let whole = rt(info.duration, rate);
        let zero = rt(0.0, rate);
        if mode == "marcadores" {
            rest.push(piece(info, tc0, whole, zero, whole, None));
        }
        let mut cursor = zero; // posición dentro de este clip
        for seg in segs {
            let (mut start, mut dur) = (rt(seg.start, rate), rt(seg.end - seg.start, rate));
            if start < cursor {
                // asas solapadas: nunca colocar cuadros dos veces
                dur = dur - (cursor - start);
                start = cursor;
            }
            if dur.value <= 0.0 {
                continue;
            }
            if mode == "aprobados" {
                markers.push((clock, dur, seg));
                clock = clock + dur;
                cuts.push(piece(info, tc0, whole, start, dur, Some(seg)));
            } else {
                if start > cursor && mode == "v1v2" {
                    rest.push(piece(info, tc0, whole, cursor, start - cursor, None));
                    cuts.push(Item::Gap(start - cursor));
                }
                markers.push((clock + start, dur, seg));
                if mode == "v1v2" {
                    rest.push(Item::Gap(dur));
                    cuts.push(piece(info, tc0, whole, start, dur, Some(seg)));
                }
                cursor = start + dur;
            }
            count += 1;
        }
        if mode != "aprobados" {
            if whole > cursor && mode == "v1v2" {
                // la cola del clip tras su último tramo aprobado
                rest.push(piece(info, tc0, whole, cursor, whole - cursor, None));
                cuts.push(Item::Gap(whole - cursor));
            }
            clock = clock + whole;
        }
    }
    let video: Vec<(&str, &Vec<Item>)> = match mode {
        "aprobados" => vec![("V1 · aprobados", &cuts)],
        "marcadores" => vec![("V1 · clips completos", &rest)],
        _ => vec![("V1 · resto del clip", &rest), ("V2 · aprobados", &cuts)],
    };
    let mut tracks: Vec<Value> = video.iter().map(|(n, items)| track_json(n, "Video", items)).collect();
    if audio {
        // audio bajo cada pista de video, enlazado con su trozo de video
        for (i, (_, items)) in video.iter().enumerate() {
            tracks.push(track_json(&format!("A{}", i + 1), "Audio", items));
        }
    }
    let marker_json: Vec<Value> = markers
        .iter()
        .map(|(pos, dur, seg)| {
            let color = MARKER_COLORS.iter().find(|(lim, _)| seg.mean_score >= *lim).map(|(_, c)| *c).unwrap_or("RED");
            let what = if crate::edicion::es_bodas() {
                // en bodas el marcador se llama como el momento del tramo
                if seg.moment.is_empty() { "tramo".to_string() } else { seg.moment.clone() }
            } else if !seg.present.is_empty() {
                seg.present.join(", ")
            } else if !seg.label.is_empty() {
                seg.label.clone()
            } else {
                "joya".into()
            };
            json!({
                "OTIO_SCHEMA": "Marker.2",
                "metadata": {},
                "name": format!("{what} · {}", pyfmt::percent0(seg.mean_score)),
                "color": color,
                "marked_range": time_range(*pos, *dur),
                "comment": seg.describe()
            })
        })
        .collect();
    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "metadata": {},
        "name": name,
        "global_start_time": rt(0.0, rate).to_json(),
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "metadata": {},
            "name": "tracks",
            "source_range": null,
            "effects": [],
            "markers": marker_json,
            "enabled": true,
            "color": null,
            "children": tracks
        }
    });
    (timeline, count)
}

/// Escribe la línea de tiempo OTIO; devuelve cuántos tramos colocó.
pub fn write_otio(results: &[(ClipInfo, Vec<Segment>)], path: &Path, name: &str, timeline_fps: Option<f64>, mode: &str, audio: bool) -> Result<usize> {
    let (tl, n) = build_otio(results, name, timeline_fps, mode, audio);
    crate::project::write_atomic(path, pyfmt::dumps_otio(&tl)?)?;
    Ok(n)
}

/// Campo CSV con las comillas mínimas del módulo `csv` de Python.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// CSV de los tramos (mismas columnas y formato que Python, líneas `\r\n`).
pub fn write_csv(results: &[(ClipInfo, Vec<Segment>)], path: &Path) -> Result<()> {
    let mut out = String::new();
    let bodas = crate::edicion::es_bodas();
    let header: &[&str] = if bodas {
        &["clip", "inicio_s", "fin_s", "duracion_s", "score_medio", "score_pico", "pico_s", "momento", "sujeto", "sujetos_en_corte", "plano", "motivo"]
    } else {
        &["clip", "inicio_s", "fin_s", "duracion_s", "score_medio", "score_pico", "pico_s", "joya", "joyas_en_corte", "tipo", "motivo"]
    };
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for (info, segs) in results {
        let name = info.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        for s in segs {
            if bodas {
                let row = [
                    name.clone(),
                    pyfmt::fixed(s.start, 2),
                    pyfmt::fixed(s.end, 2),
                    pyfmt::fixed(s.end - s.start, 2),
                    pyfmt::fixed(s.mean_score, 3),
                    pyfmt::fixed(s.peak_score, 3),
                    pyfmt::fixed(s.peak_t, 2),
                    s.moment.clone(),
                    s.label.clone(),
                    s.present.join(", "),
                    s.shot.clone(),
                    s.describe(),
                ];
                out.push_str(&row.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
                out.push_str("\r\n");
                continue;
            }
            let row = [
                name.clone(),
                pyfmt::fixed(s.start, 2),
                pyfmt::fixed(s.end, 2),
                pyfmt::fixed(s.end - s.start, 2),
                pyfmt::fixed(s.mean_score, 3),
                pyfmt::fixed(s.peak_score, 3),
                pyfmt::fixed(s.peak_t, 2),
                s.label.clone(),
                s.present.join(", "),
                s.shot.clone(),
                s.describe(),
            ];
            out.push_str(&row.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
            out.push_str("\r\n");
        }
    }
    crate::project::write_atomic(path, out)?;
    Ok(())
}
