//! Prueba de re-puntuación contra Python.
//!
//! Copia las cachés de Python de `camcuts_next/bench_oct27/cache` (19 clips Sony 4K a 29.97 DF) a una
//! carpeta temporal, ejecuta la CLI de Rust con `--rescore` y compara el OTIO y el CSV con los que
//! escribió Python con las mismas opciones (`tests/data/rescore/`, generados por `gen_rescore_ref.py`).
//! Si las cachés no están en este equipo, la prueba se salta.

use std::path::{Path, PathBuf};

use camcuts_core::cli::{self, Args};
use clap::Parser;

const BENCH: &str = r"C:\Users\Usuario\Tools\camcuts_next\bench_oct27\cache";

/// Mismas combinaciones que `gen_rescore_ref.py`: (nombre, argumentos extra, preset).
const VARIANTS: &[(&str, &[&str], &str)] = &[
    ("joyas_v1v2", &[], "joyas"),
    ("social_v1v2", &["--preset", "social"], "social"),
    ("todo_aprobados", &["--preset", "todo", "--modo", "aprobados"], "todo"),
    ("joyas_marcadores_filtros", &["--modo", "marcadores", "--joya", "dije,anillo", "--tipo", "producto,modelo"], "joyas"),
    ("joyas_25fps", &["--timeline-fps", "25"], "joyas"),
];

fn data() -> PathBuf {
    // en el workspace, junto al Cargo.toml del crate; en una compilación aislada, junto a este archivo
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data");
    let d = if d.is_dir() { d } else { Path::new(file!()).parent().unwrap().join("data") };
    d.join("rescore")
}

/// Carpeta temporal con una copia de las cachés (nunca se escribe en la carpeta de Python).
fn scratch_copy() -> Option<PathBuf> {
    let src = Path::new(BENCH);
    if !src.is_dir() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("camcuts_rescore_{}", std::process::id()));
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "json").unwrap_or(false) {
            std::fs::copy(&p, cache.join(p.file_name().unwrap())).unwrap();
        }
    }
    Some(dir)
}

// --- Orden de los empates en "joyas presentes" ---------------------------------------------------
// Python ordena las joyas del corte por frecuencia, pero los empates salen en el orden de iteración de
// un `set` de textos, que cambia en cada proceso (hash aleatorio). Rust usa el orden de aparición. Por
// eso esas listas se comparan sin orden; todo lo demás (tramos, tiempos, puntuaciones…) debe ser igual.

fn sort_list(s: &str) -> String {
    let mut v: Vec<&str> = s.split(", ").collect();
    v.sort();
    v.join(", ")
}

/// Ordena la lista de joyas dentro de "joyas: a, b · …" y en el nombre de marcador "a, b · 87%".
fn norm_text(s: &str) -> String {
    let mut s = s.to_string();
    if let Some(i) = s.find("joyas: ") {
        let a = i + "joyas: ".len();
        if let Some(j) = s[a..].find(" · ") {
            s = format!("{}{}{}", &s[..a], sort_list(&s[a..a + j]), &s[a + j..]);
        }
    }
    if s.ends_with('%') {
        if let Some(i) = s.rfind(" · ") {
            s = format!("{}{}", sort_list(&s[..i]), &s[i..]);
        }
    }
    s
}

fn norm_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "jewels" {
                    if let serde_json::Value::Array(a) = x {
                        a.sort_by(|p, q| p.as_str().cmp(&q.as_str()));
                    }
                } else {
                    norm_json(x);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(norm_json),
        serde_json::Value::String(s) => *s = norm_text(s),
        _ => {}
    }
}

/// Divide una línea CSV (comillas mínimas de Python).
fn csv_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn norm_csv_line(line: &str) -> Vec<String> {
    let mut f = csv_split(line);
    if f.len() == 11 {
        f[8] = sort_list(&f[8]);
        f[10] = norm_text(&f[10]);
    }
    f
}

/// Compara dos JSON de OTIO: estructura y textos idénticos, números idénticos bit a bit.
fn same_json(a: &serde_json::Value, b: &serde_json::Value, path: &str, diffs: &mut Vec<String>) {
    use serde_json::Value::*;
    match (a, b) {
        (Object(x), Object(y)) => {
            let kx: Vec<&std::string::String> = x.keys().collect();
            let ky: Vec<&std::string::String> = y.keys().collect();
            if kx != ky {
                diffs.push(format!("{path}: claves {kx:?} != {ky:?}"));
                return;
            }
            for (k, v) in x {
                same_json(v, &y[k], &format!("{path}.{k}"), diffs);
            }
        }
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                diffs.push(format!("{path}: {} elementos != {}", x.len(), y.len()));
                return;
            }
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                same_json(u, v, &format!("{path}[{i}]"), diffs);
            }
        }
        (Number(x), Number(y)) => {
            let (u, v) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if u != v {
                diffs.push(format!("{path}: {u} != {v}"));
            }
        }
        _ => {
            if a != b {
                diffs.push(format!("{path}: {a} != {b}"));
            }
        }
    }
}

#[test]
fn rescore_igual_que_python() {
    let Some(dir) = scratch_copy() else {
        eprintln!("sin cachés de bench_oct27: prueba saltada");
        return;
    };
    let mut report = Vec::new();
    for (name, extra, preset) in VARIANTS {
        let mut argv = vec!["camcuts", "--out", dir.to_str().unwrap(), "--rescore", "--no-sheet"];
        argv.extend_from_slice(extra);
        cli::run(Args::parse_from(&argv), None).unwrap();
        let otio_rs = std::fs::read(dir.join(format!("selects_{preset}.otio"))).unwrap();
        let otio_py = std::fs::read(data().join(format!("{name}.otio"))).unwrap();
        let csv_rs = std::fs::read_to_string(dir.join(format!("selects_{preset}.csv"))).unwrap();
        let csv_py = std::fs::read_to_string(data().join(format!("{name}.csv"))).unwrap();
        // CSV: mismo texto (salvo el orden de los empates de joyas presentes) y mismas líneas \r\n
        let csv_rows = csv_py.lines().count() - 1;
        let mut tie_lines = 0;
        for (i, (a, b)) in csv_rs.lines().zip(csv_py.lines()).enumerate() {
            if a != b {
                if norm_csv_line(a) == norm_csv_line(b) {
                    tie_lines += 1;
                } else {
                    report.push(format!("{name}.csv línea {}:\n  rust:   {a}\n  python: {b}", i + 1));
                }
            }
        }
        if csv_rs.lines().count() != csv_py.lines().count() || csv_rs.matches("\r\n").count() != csv_py.matches("\r\n").count() {
            report.push(format!("{name}.csv: {} líneas != {}", csv_rs.lines().count(), csv_py.lines().count()));
        }
        // OTIO: mismo contenido; byte a byte igual si no hubo empates
        let identical = otio_rs == otio_py;
        if !identical {
            let mut a: serde_json::Value = serde_json::from_slice(&otio_rs).unwrap();
            let mut b: serde_json::Value = serde_json::from_slice(&otio_py).unwrap();
            norm_json(&mut a);
            norm_json(&mut b);
            let mut diffs = Vec::new();
            same_json(&a, &b, name, &mut diffs);
            report.extend(diffs.into_iter().take(20));
            // mismo formato de texto: mismas líneas \r\n (las diferencias solo pueden ser de orden)
            let (ta, tb) = (String::from_utf8_lossy(&otio_rs), String::from_utf8_lossy(&otio_py));
            let (la, lb) = (ta.split("\r\n").count(), tb.split("\r\n").count());
            if la != lb || ta.len() != tb.len() {
                report.push(format!("{name}.otio: {la} líneas / {} bytes != {lb} líneas / {} bytes", ta.len(), tb.len()));
            }
        }
        println!(
            "{name}: {csv_rows} tramos; CSV {} ({tie_lines} líneas con empates de orden); OTIO {}",
            if csv_rs == csv_py { "idéntico byte a byte" } else { "igual salvo empates" },
            if identical { "idéntico byte a byte" } else { "igual salvo empates" }
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(report.is_empty(), "diferencias con Python:\n{}", report.join("\n"));
}
