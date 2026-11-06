//! Operaciones de imagen que replican las de Python (OpenCV, PIL, PyTorch) para que la entrada de cada
//! modelo sea la misma que en la app original.
//!
//! - [`resize_linear`]: `cv2.resize(..., INTER_LINEAR)` en 8 bits (aritmética de punto fijo de OpenCV).
//! - [`letterbox`]: `ultralytics.data.augment.LetterBox` (redimensionar + rellenar con 114).
//! - [`resize_bicubic_pil`]: `PIL.Image.resize(..., BICUBIC)` (lo que usa el preprocesado de CLIP).
//! - [`py_round`]: `round()` de Python (mitades al par), usado en las cuentas del letterbox.

/// `round()` de Python 3: redondeo bancario (las mitades van al número par).
pub fn py_round(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 {
        // mitad exacta: al par más cercano
        let f = x.floor();
        if (f as i64) % 2 == 0 { f as i64 } else { f as i64 + 1 }
    } else {
        r as i64
    }
}

/// Redimensiona una imagen de 8 bits con `canales` intercalados como `cv2.resize(INTER_LINEAR)`.
///
/// OpenCV usa centros de píxel (x+0.5)·escala−0.5, pesos de 11 bits redondeados por separado,
/// una pasada horizontal a enteros y una vertical con desplazamientos (>>4, >>16, +2 >>2).
/// Así el resultado coincide con OpenCV salvo, como mucho, ±1 nivel en casos raros.
pub fn resize_linear(src: &[u8], sw: usize, sh: usize, ch: usize, dw: usize, dh: usize) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    const SCALE: f64 = 2048.0; // INTER_RESIZE_COEF_SCALE
    // tabla de coordenadas y pesos de un eje (igual para x e y)
    fn axis(s: usize, d: usize) -> (Vec<usize>, Vec<usize>, Vec<i32>, Vec<i32>) {
        let scale = s as f64 / d as f64;
        let (mut i0, mut i1, mut a0, mut a1) = (vec![0; d], vec![0; d], vec![0; d], vec![0; d]);
        for x in 0..d {
            let f = (x as f64 + 0.5) * scale - 0.5;
            let mut sx = f.floor() as i64;
            let mut fx = (f - sx as f64) as f32 as f64; // OpenCV calcula en float
            if sx < 0 {
                fx = 0.0;
                sx = 0;
            }
            if sx >= s as i64 - 1 {
                fx = 0.0;
                sx = s as i64 - 1;
            }
            i0[x] = sx as usize;
            i1[x] = (sx as usize + 1).min(s - 1);
            // saturate_cast<short>(v * 2048): redondeo al más cercano
            a0[x] = ((1.0 - fx) * SCALE).round() as i32;
            a1[x] = (fx * SCALE).round() as i32;
        }
        (i0, i1, a0, a1)
    }
    let (xs0, xs1, ax0, ax1) = axis(sw, dw);
    let (ys0, ys1, by0, by1) = axis(sh, dh);
    // pasada horizontal de una fila de origen -> enteros escalados por 2048
    let hrow = |y: usize, out: &mut Vec<i32>| {
        let row = &src[y * sw * ch..(y + 1) * sw * ch];
        for x in 0..dw {
            let (p0, p1) = (xs0[x] * ch, xs1[x] * ch);
            for c in 0..ch {
                out[x * ch + c] = row[p0 + c] as i32 * ax0[x] + row[p1 + c] as i32 * ax1[x];
            }
        }
    };
    let mut dst = vec![0u8; dw * dh * ch];
    let (mut r0, mut r1) = (vec![0i32; dw * ch], vec![0i32; dw * ch]);
    let (mut last0, mut last1) = (usize::MAX, usize::MAX);
    for y in 0..dh {
        // reutiliza las filas horizontales ya calculadas (como el búfer circular de OpenCV)
        if ys0[y] != last0 {
            if ys0[y] == last1 {
                std::mem::swap(&mut r0, &mut r1);
                last0 = last1;
                last1 = usize::MAX;
            } else {
                hrow(ys0[y], &mut r0);
                last0 = ys0[y];
            }
        }
        if ys1[y] != last1 {
            hrow(ys1[y], &mut r1);
            last1 = ys1[y];
        }
        let (b0, b1) = (by0[y], by1[y]);
        let out = &mut dst[y * dw * ch..(y + 1) * dw * ch];
        for i in 0..dw * ch {
            // VResizeLinear<uchar>: ((b0*(S0>>4))>>16) + ((b1*(S1>>4))>>16) + 2 >> 2
            let v = (((b0 * (r0[i] >> 4)) >> 16) + ((b1 * (r1[i] >> 4)) >> 16) + 2) >> 2;
            out[i] = v.clamp(0, 255) as u8;
        }
    }
    dst
}

/// Parámetros de un letterbox ya aplicado (para deshacerlo en las salidas).
#[derive(Debug, Clone, Copy)]
pub struct LetterboxInfo {
    /// tamaño de la imagen original (alto, ancho)
    pub orig: (usize, usize),
    /// tamaño de la entrada del modelo (alto, ancho)
    pub input: (usize, usize),
    /// tamaño sin relleno tras redimensionar (alto, ancho)
    pub unpad: (usize, usize),
    pub top: usize,
    pub left: usize,
}

/// Forma de la entrada que daría `LetterBox(new_shape, auto, stride)` para una imagen h×w.
/// Devuelve (alto, ancho) del lienzo, tamaño sin relleno y rellenos (arriba, abajo, izquierda, derecha).
pub fn letterbox_params(
    h: usize,
    w: usize,
    new_shape: (usize, usize),
    auto: bool,
    stride: usize,
    center: bool,
) -> LetterboxInfo {
    let r = (new_shape.0 as f64 / h as f64).min(new_shape.1 as f64 / w as f64);
    // new_unpad = (round(w*r), round(h*r)), como mínimo 1 píxel
    let uw = py_round(w as f64 * r).max(1) as usize;
    let uh = py_round(h as f64 * r).max(1) as usize;
    let (mut dw, mut dh) = ((new_shape.1 - uw) as f64, (new_shape.0 - uh) as f64);
    if auto {
        // rectángulo mínimo: solo el relleno que falta para llegar a un múltiplo del stride
        dw %= stride as f64;
        dh %= stride as f64;
    }
    if center {
        dw /= 2.0;
        dh /= 2.0;
    }
    let top = if center { py_round(dh - 0.1) } else { 0 } as usize;
    let bottom = py_round(dh + 0.1) as usize;
    let left = if center { py_round(dw - 0.1) } else { 0 } as usize;
    let right = py_round(dw + 0.1) as usize;
    LetterboxInfo { orig: (h, w), input: (uh + top + bottom, uw + left + right), unpad: (uh, uw), top, left }
}

/// Aplica el letterbox a una imagen BGR (3 canales): redimensiona con INTER_LINEAR y rellena con `pad`.
pub fn letterbox(bgr: &[u8], info: &LetterboxInfo, pad: u8) -> Vec<u8> {
    let (h, w) = info.orig;
    let (uh, uw) = info.unpad;
    let resized = resize_linear(bgr, w, h, 3, uw, uh);
    let (ih, iw) = info.input;
    let mut out = vec![pad; ih * iw * 3];
    for y in 0..uh {
        let d = ((y + info.top) * iw + info.left) * 3;
        out[d..d + uw * 3].copy_from_slice(&resized[y * uw * 3..(y + 1) * uw * 3]);
    }
    out
}

/// Pasa una imagen BGR HWC de 8 bits a un tensor RGB CHW en f32 con `(v/escala - media)/desv` por canal
/// (en orden RGB). Se añade al final de `dst`, para poder construir lotes.
pub fn bgr_to_chw(bgr: &[u8], h: usize, w: usize, scale: f32, mean: [f32; 3], std: [f32; 3], dst: &mut Vec<f32>) {
    let n = h * w;
    let base = dst.len();
    dst.resize(base + 3 * n, 0.0);
    let (r, rest) = dst[base..].split_at_mut(n);
    let (g, b) = rest.split_at_mut(n);
    // tabla por canal de los 256 valores posibles, con la MISMA cuenta en f32 (resultado idéntico bit a bit):
    // tres lecturas por píxel en vez de dos divisiones y una resta por canal (SAM a 1024: 6.6 → ~2 ms)
    let lut = |c: usize| -> [f32; 256] { std::array::from_fn(|v| (v as f32 / scale - mean[c]) / std[c]) };
    let (lr, lg, lb) = (lut(0), lut(1), lut(2));
    for (i, p) in bgr[..n * 3].chunks_exact(3).enumerate() {
        r[i] = lr[p[2] as usize];
        g[i] = lg[p[1] as usize];
        b[i] = lb[p[0] as usize];
    }
}

/// Recorta un rectángulo [x0, x1) × [y0, y1) de una imagen de 3 canales.
pub fn crop(bgr: &[u8], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<u8> {
    let cw = x1.saturating_sub(x0);
    let mut out = Vec::with_capacity(cw * y1.saturating_sub(y0) * 3);
    for y in y0..y1 {
        out.extend_from_slice(&bgr[(y * w + x0) * 3..(y * w + x1) * 3]);
    }
    out
}

/// Filtro bicúbico de PIL (a = −0.5), soporte 2.
fn bicubic(x: f64) -> f64 {
    const A: f64 = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * A
    } else {
        0.0
    }
}

/// Coeficientes de un eje como `precompute_coeffs` de PIL (Resample.c), en punto fijo de 22 bits.
fn pil_coeffs(in_size: usize, out_size: usize) -> (Vec<usize>, Vec<Vec<i32>>) {
    const PRECISION_BITS: i32 = 32 - 8 - 2;
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 2.0 * filterscale;
    let mut starts = Vec::with_capacity(out_size);
    let mut all = Vec::with_capacity(out_size);
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let ss = 1.0 / filterscale;
        // (int) trunca hacia cero, como en C
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize - xmin;
        let mut k: Vec<f64> = (0..xmax).map(|x| bicubic((x as f64 + xmin as f64 - center + 0.5) * ss)).collect();
        let ww: f64 = k.iter().sum();
        if ww != 0.0 {
            for v in k.iter_mut() {
                *v /= ww;
            }
        }
        // normalize_coeffs_8bpc: redondeo hacia fuera del cero
        let ki = k
            .iter()
            .map(|&v| {
                let s = v * (1i64 << PRECISION_BITS) as f64;
                if v < 0.0 { (-0.5 + s) as i32 } else { (0.5 + s) as i32 }
            })
            .collect();
        starts.push(xmin);
        all.push(ki);
    }
    (starts, all)
}

/// `PIL.Image.resize((dw, dh), BICUBIC)` para RGB/BGR de 8 bits: pasada horizontal y luego vertical,
/// cada una redondeando a 8 bits como PIL (por eso no basta con un bicúbico en flotante).
pub fn resize_bicubic_pil(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    const PRECISION_BITS: i32 = 22;
    let clip8 = |v: i64| -> u8 { (v >> PRECISION_BITS).clamp(0, 255) as u8 };
    // horizontal
    let tmp = if dw != sw {
        let (xs, ks) = pil_coeffs(sw, dw);
        let mut t = vec![0u8; dw * sh * 3];
        for y in 0..sh {
            let row = &src[y * sw * 3..(y + 1) * sw * 3];
            for x in 0..dw {
                for c in 0..3 {
                    let mut ss: i64 = 1 << (PRECISION_BITS - 1);
                    for (i, &k) in ks[x].iter().enumerate() {
                        ss += row[(xs[x] + i) * 3 + c] as i64 * k as i64;
                    }
                    t[(y * dw + x) * 3 + c] = clip8(ss);
                }
            }
        }
        t
    } else {
        src.to_vec()
    };
    if dh == sh {
        return tmp;
    }
    // vertical
    let (ys, ks) = pil_coeffs(sh, dh);
    let mut out = vec![0u8; dw * dh * 3];
    for y in 0..dh {
        for x in 0..dw * 3 {
            let mut ss: i64 = 1 << (PRECISION_BITS - 1);
            for (i, &k) in ks[y].iter().enumerate() {
                ss += tmp[(ys[y] + i) * dw * 3 + x] as i64 * k as i64;
            }
            out[y * dw * 3 + x] = clip8(ss);
        }
    }
    out
}

/// Interpolación bilineal de un mapa f32 (alto `sh`, ancho `sw`) a `dh`×`dw`, como
/// `F.interpolate(mode="bilinear", align_corners=False)` sobre la zona [0, span_h)×[0, span_w) del origen.
/// `span_*` puede no ser entero (camino `grid_sample` de `ops.scale_masks`); los bordes se repiten.
/// Devuelve 255 donde el valor interpolado supera `thr` y 0 en el resto.
pub fn bilinear_threshold(
    src: &[f32],
    sh: usize,
    sw: usize,
    span_h: f64,
    span_w: f64,
    dh: usize,
    dw: usize,
    thr: f32,
) -> Vec<u8> {
    fn axis(span: f64, s: usize, d: usize) -> Vec<(usize, usize, f32)> {
        let scale = span / d as f64;
        (0..d)
            .map(|j| {
                let f = ((j as f64 + 0.5) * scale - 0.5).max(0.0).min((s - 1) as f64);
                let i0 = f.floor() as usize;
                let i1 = (i0 + 1).min(s - 1);
                (i0, i1, (f - i0 as f64) as f32)
            })
            .collect()
    }
    let xs = axis(span_w, sw, dw);
    let ys = axis(span_h, sh, dh);
    let mut out = vec![0u8; dh * dw];
    for (y, &(y0, y1, fy)) in ys.iter().enumerate() {
        let (r0, r1) = (&src[y0 * sw..(y0 + 1) * sw], &src[y1 * sw..(y1 + 1) * sw]);
        for (x, &(x0, x1, fx)) in xs.iter().enumerate() {
            let top = r0[x0] + (r0[x1] - r0[x0]) * fx;
            let bot = r1[x0] + (r1[x1] - r1[x0]) * fx;
            if top + (bot - top) * fy > thr {
                out[y * dw + x] = 255;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redondeo_como_python() {
        assert_eq!(py_round(2.5), 2);
        assert_eq!(py_round(3.5), 4);
        assert_eq!(py_round(1.9), 2);
        assert_eq!(py_round(-0.5), 0);
    }

    #[test]
    fn letterbox_960x540_a_960() {
        // 960x540 con imgsz 960 y auto: 544 de alto, 2 píxeles arriba y 2 abajo
        let lb = letterbox_params(540, 960, (960, 960), true, 32, true);
        assert_eq!(lb.input, (544, 960));
        assert_eq!((lb.top, lb.left), (2, 0));
        // pose a 640: 640x360 + 12 arriba/abajo
        let lb = letterbox_params(540, 960, (640, 640), true, 32, true);
        assert_eq!(lb.input, (384, 640));
        assert_eq!(lb.top, 12);
        // SAM: 1024 sin auto, relleno solo abajo
        let lb = letterbox_params(540, 960, (1024, 1024), false, 32, false);
        assert_eq!(lb.input, (1024, 1024));
        assert_eq!(lb.unpad, (576, 1024));
        assert_eq!(lb.top, 0);
    }
}
