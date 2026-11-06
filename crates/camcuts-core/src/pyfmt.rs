//! Compatibilidad con los formatos de Python.
//!
//! La app de Python y esta comparten archivos (caché, proyecto, colores, OTIO, CSV). Para que los
//! archivos salgan iguales byte a byte (o casi), aquí se imitan:
//! - `round()` de Python (redondeo al par) y `round(x, n)`;
//! - `repr(float)` (la representación más corta, con `e-05` en vez de `e-5`);
//! - el formato `{:.0%}` y `{:.2f}`;
//! - `json.dumps` con sus separadores, `indent` y `ensure_ascii`.

use std::io;

/// `round(x)` de Python para floats: redondeo al entero más cercano, empates al par.
#[inline]
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// `sum()` de Python 3.12 sobre floats: suma compensada de Neumaier (Kahan-Babuška mejorado).
///
/// Python 3.12 cambió `sum()` a esta suma; con la suma ingenua salen diferencias en el último bit.
pub fn pysum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut f = 0.0f64;
    let mut c = 0.0f64;
    for x in values {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// `round(x, n)` de Python: redondea el valor decimal exacto del float (empates al par).
///
/// Python usa el valor binario exacto; el formateo de Rust con precisión fija también lo hace,
/// así que formatear y volver a leer da el mismo resultado.
pub fn round_ndigits(x: f64, n: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{:.*}", n, x).parse().unwrap_or(x)
}

/// `f"{x:.{n}f}"` de Python (mismo redondeo del valor exacto, empates al par).
pub fn fixed(x: f64, n: usize) -> String {
    let s = format!("{:.*}", n, x);
    // Python escribe "-0.00" para valores negativos que redondean a cero; Rust también: no hay nada que tocar
    s
}

/// `f"{x:.0%}"` de Python: multiplica por 100 (en doble precisión) y formatea sin decimales.
pub fn percent0(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// `repr(float)` de Python: los dígitos más cortos que vuelven al mismo float, en notación fija si el
/// exponente decimal está entre -4 y 15, y científica (`1e-05`, `1.5e+16`) fuera de ese rango.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // `{:e}` de Rust da los dígitos más cortos: "1.2345e-5", "5e0"
    let e = format!("{:e}", x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if neg { "-" } else { "" };
    if (-4..16).contains(&exp) {
        // notación fija
        let n = digits.len() as i32;
        let s = if exp >= 0 {
            let int_len = exp + 1;
            if n <= int_len {
                format!("{}{}.0", digits, "0".repeat((int_len - n) as usize))
            } else {
                format!("{}.{}", &digits[..int_len as usize], &digits[int_len as usize..])
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{s}")
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        let es = if exp < 0 { '-' } else { '+' };
        format!("{sign}{m}e{es}{:02}", exp.abs())
    }
}

/// Formateador de `serde_json` que escribe como `json.dumps` de Python.
///
/// - `indent: None` → separadores `", "` y `": "` (lo que usa `json.dumps(obj)` por defecto).
/// - `indent: Some(n)` → una línea por elemento con `n` espacios por nivel (`json.dumps(obj, indent=n)`).
/// - `ensure_ascii` → los caracteres no ASCII salen como `\uXXXX` (minúsculas, pares sustitutos).
/// - `crlf` → saltos de línea `\r\n` (el OTIO que escribe la librería de OTIO en Windows).
pub struct PyFormatter {
    pub indent: Option<usize>,
    pub ensure_ascii: bool,
    pub crlf: bool,
    level: usize,
    has_value: bool,
}

impl PyFormatter {
    pub fn new(indent: Option<usize>, ensure_ascii: bool) -> Self {
        PyFormatter { indent, ensure_ascii, crlf: false, level: 0, has_value: false }
    }

    fn newline<W: ?Sized + io::Write>(&self, w: &mut W) -> io::Result<()> {
        if let Some(n) = self.indent {
            w.write_all(if self.crlf { b"\r\n" } else { b"\n" })?;
            for _ in 0..self.level * n {
                w.write_all(b" ")?;
            }
        }
        Ok(())
    }
}

impl serde_json::ser::Formatter for PyFormatter {
    fn write_f64<W: ?Sized + io::Write>(&mut self, w: &mut W, value: f64) -> io::Result<()> {
        w.write_all(float_repr(value).as_bytes())
    }
    fn write_f32<W: ?Sized + io::Write>(&mut self, w: &mut W, value: f32) -> io::Result<()> {
        w.write_all(float_repr(value as f64).as_bytes())
    }
    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level += 1;
        self.has_value = false;
        w.write_all(b"[")
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level -= 1;
        if self.has_value {
            self.newline(w)?;
        }
        w.write_all(b"]")
    }
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(if self.indent.is_some() { b"," } else { b", " })?;
        }
        self.newline(w)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level += 1;
        self.has_value = false;
        w.write_all(b"{")
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level -= 1;
        if self.has_value {
            self.newline(w)?;
        }
        w.write_all(b"}")
    }
    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(if self.indent.is_some() { b"," } else { b", " })?;
        }
        self.newline(w)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn write_string_fragment<W: ?Sized + io::Write>(&mut self, w: &mut W, fragment: &str) -> io::Result<()> {
        // serde_json ya escapa comillas, barras y controles; aquí solo falta lo que Python escapa de más
        let mut start = 0;
        for (i, ch) in fragment.char_indices() {
            let escape = if self.ensure_ascii { !(' '..='~').contains(&ch) } else { false };
            if escape {
                w.write_all(fragment[start..i].as_bytes())?;
                let mut buf = [0u16; 2];
                for unit in ch.encode_utf16(&mut buf) {
                    write!(w, "\\u{:04x}", unit)?;
                }
                start = i + ch.len_utf8();
            }
        }
        w.write_all(fragment[start..].as_bytes())
    }
}

/// `json.dumps(value)` de Python (separadores `", "`/`": "`, ASCII escapado).
pub fn dumps(value: &impl serde::Serialize) -> anyhow::Result<String> {
    dumps_with(value, None, true)
}

/// `json.dumps(value, indent=..., ensure_ascii=...)` de Python.
pub fn dumps_with(value: &impl serde::Serialize, indent: Option<usize>, ensure_ascii: bool) -> anyhow::Result<String> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, PyFormatter::new(indent, ensure_ascii));
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(out)?)
}

/// JSON con sangría y saltos `\r\n`, como lo escribe la librería de OTIO en Windows.
pub fn dumps_otio(value: &impl serde::Serialize) -> anyhow::Result<String> {
    let mut out = Vec::new();
    let mut f = PyFormatter::new(Some(4), false);
    f.crlf = true;
    let mut ser = serde_json::Serializer::with_formatter(&mut out, f);
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_como_python() {
        assert_eq!(float_repr(0.0), "0.0");
        assert_eq!(float_repr(4.0), "4.0");
        assert_eq!(float_repr(1e-5), "1e-05");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1.5e16), "1.5e+16");
        assert_eq!(float_repr(123456789012345.6), "123456789012345.6");
        assert_eq!(float_repr(29.97002997002997), "29.97002997002997");
        assert_eq!(float_repr(-0.25), "-0.25");
        assert_eq!(float_repr(1914720.0), "1914720.0");
    }

    #[test]
    fn formatos_como_python() {
        // valores calculados con Python 3.12: f"{0.125:.2f} {0.375:.2f} {2.5:.0f} {0.995:.0%} {0.805:.0%} {0.785:.0%}"
        assert_eq!(fixed(0.125, 2), "0.12");
        assert_eq!(fixed(0.375, 2), "0.38");
        assert_eq!(fixed(2.5, 0), "2");
        assert_eq!(percent0(0.995), "100%");
        assert_eq!(percent0(0.805), "80%");
        assert_eq!(percent0(0.785), "78%");
        assert_eq!(round_ndigits(0.25, 1), 0.2);
    }

    #[test]
    fn dumps_como_python() {
        let v = serde_json::json!({"a": [1, 2.0, "ñ"], "b": {}, "c": []});
        // Python: json.dumps(...) escapa la ñ como ñ (en minúsculas)
        let esperado = format!("{{\"a\": [1, 2.0, \"{}u00f1\"], \"b\": {{}}, \"c\": []}}", '\\');
        assert_eq!(dumps(&v).unwrap(), esperado);
        assert_eq!(dumps_with(&v, Some(1), false).unwrap(), "{\n \"a\": [\n  1,\n  2.0,\n  \"ñ\"\n ],\n \"b\": {},\n \"c\": []\n}");
    }
}
