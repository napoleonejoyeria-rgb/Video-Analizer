//! Medidas de imagen (port de `metrics.py`). `edges_and_laplacian` reproduce la medida de foco de
//! Cutdown (metrics/focus.rs) y el Laplaciano que usan las medidas de nitidez de cada joya.

use std::sync::Arc;

use rayon::prelude::*;
use rustfft::{num_complex::Complex64, FftPlanner};

use crate::image::{self, ImageF32};

/// Máscara de bordes nítidos y Laplaciano con signo de un cuadro (se calculan una vez por cuadro).
pub struct EdgeMaps {
    pub width: usize,
    pub height: usize,
    /// 1 = píxel de detalle fino y contrastado.
    pub mask: Vec<u8>,
    /// Laplaciano de 4 vecinos con signo (`filter2D`, borde REPLICATE).
    pub lap: ImageF32,
}

/// Bordes nítidos y Laplaciano (Cutdown 1.2.1).
///
/// G = magnitud de Sobel, Lap = |Laplaciano de 4 vecinos|, d = max(60 - L, 0) * 0.2;
/// nítido si Lap >= 0.2*G y Lap > d + 10 y G > d + 15. Todo en `f32`, como en Python.
pub fn edges_and_laplacian(gray: &[u8], width: usize, height: usize) -> EdgeMaps {
    let g: Vec<f32> = gray.par_iter().map(|&v| v as f32).collect();
    let mut lap = vec![0f32; width * height];
    let mut mask = vec![0u8; width * height];
    if width > 0 && height > 0 {
        // filas de 16 en 16 por tarea: menos reparto de rayon que una tarea por fila
        const ROWS: usize = 16;
        lap.par_chunks_mut(width * ROWS).zip(mask.par_chunks_mut(width * ROWS)).enumerate().for_each(|(blk, (lblk, mblk))| {
            for (k, (lrow, mrow)) in lblk.chunks_mut(width).zip(mblk.chunks_mut(width)).enumerate() {
                let y = blk * ROWS + k;
                // BORDER_REPLICATE: el vecino fuera del cuadro es el propio borde
                let ym = y.saturating_sub(1);
                let yp = (y + 1).min(height - 1);
                let (a, b, c) = (&g[ym * width..(ym + 1) * width], &g[y * width..(y + 1) * width], &g[yp * width..(yp + 1) * width]);
                // bordes izquierdo y derecho del Laplaciano (vecino replicado)
                for x in [0, width - 1] {
                    let xm = x.saturating_sub(1);
                    let xp = (x + 1).min(width - 1);
                    lrow[x] = a[x] + b[xm] - 4.0 * b[x] + b[xp] + c[x];
                }
                if width < 3 {
                    continue;
                }
                let border_row = y == 0 || y == height - 1;
                // interior: sin comprobaciones de borde, para que el compilador vectorice
                for x in 1..width - 1 {
                    let l = a[x] + b[x - 1] - 4.0 * b[x] + b[x + 1] + c[x];
                    lrow[x] = l;
                    if border_row {
                        continue; // los bordes del cuadro nunca cuentan
                    }
                    // Sobel 3x3 en el píxel, con la misma aritmética que `image::sobel3` (en el interior
                    // no hace falta el reflejo): sumas de enteros de 8 bits, exactas en f32
                    let gx = (a[x + 1] - a[x - 1]) + 2.0 * (b[x + 1] - b[x - 1]) + (c[x + 1] - c[x - 1]);
                    let gy = (c[x - 1] + 2.0 * c[x] + c[x + 1]) - (a[x - 1] + 2.0 * a[x] + a[x + 1]);
                    let gm = (gx * gx + gy * gy).sqrt();
                    let la = l.abs();
                    let d = (60.0f32 - b[x]).max(0.0) * 0.2f32;
                    mrow[x] = (la >= 0.2f32 * gm && la > d + 10.0 && gm > d + 15.0) as u8;
                }
            }
        });
    }
    EdgeMaps { width, height, mask, lap: ImageF32 { width, height, data: lap } }
}

/// Densidad de bordes del cuadro entero, sin la fila/columna exterior (`mask[1:-1, 1:-1].mean()`).
pub fn density_full(m: &EdgeMaps) -> f64 {
    if m.width < 3 || m.height < 3 {
        return f64::NAN;
    }
    let n: u64 = (1..m.height - 1)
        .into_par_iter()
        .map(|y| m.mask[y * m.width + 1..(y + 1) * m.width - 1].iter().map(|&v| v as u64).sum::<u64>())
        .sum();
    n as f64 / ((m.width - 2) * (m.height - 2)) as f64
}

/// Región de medida: un rectángulo o una máscara de cuadro completo (1 = dentro).
#[derive(Clone, Copy)]
pub enum Region<'a> {
    /// Rectángulo `[x0, y0, x1, y1)`.
    Rect([usize; 4]),
    /// Máscara del tamaño del cuadro.
    Mask(&'a [u8]),
    /// Máscara local de un rectángulo (`(x1-x0)*(y1-y0)` valores, 1 = dentro).
    RectMask([usize; 4], &'a [u8]),
}

impl Region<'_> {
    /// Recorre los índices de los píxeles de la región, en orden de filas.
    pub fn for_each(&self, width: usize, mut f: impl FnMut(usize)) {
        match *self {
            Region::Rect([x0, y0, x1, y1]) => {
                for y in y0..y1 {
                    for x in x0..x1 {
                        f(y * width + x);
                    }
                }
            }
            Region::Mask(m) => {
                for (i, &v) in m.iter().enumerate() {
                    if v != 0 {
                        f(i);
                    }
                }
            }
            Region::RectMask([x0, y0, x1, y1], m) => {
                let rw = x1 - x0;
                for y in y0..y1 {
                    for x in x0..x1 {
                        if m[(y - y0) * rw + (x - x0)] != 0 {
                            f(y * width + x);
                        }
                    }
                }
            }
        }
    }

    /// Número de píxeles de la región.
    pub fn count(&self) -> usize {
        match *self {
            Region::Rect([x0, y0, x1, y1]) => (x1.saturating_sub(x0)) * (y1.saturating_sub(y0)),
            Region::Mask(m) | Region::RectMask(_, m) => m.iter().filter(|&&v| v != 0).count(),
        }
    }
}

/// Densidad de bordes dentro de una región (0 si la región está vacía).
pub fn density(m: &EdgeMaps, region: Region) -> f64 {
    let mut n = 0u64;
    let mut hits = 0u64;
    region.for_each(m.width, |i| {
        n += 1;
        hits += m.mask[i] as u64;
    });
    if n == 0 { 0.0 } else { hits as f64 / n as f64 }
}

/// Varianza del Laplaciano en una región (o en todo el cuadro con `None`).
pub fn laplacian_variance(lap: &ImageF32, region: Option<Region>) -> f64 {
    match region {
        None => {
            let n = lap.data.len() as f64;
            let mean = lap.data.par_iter().map(|&v| v as f64).sum::<f64>() / n;
            lap.data.par_iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n
        }
        Some(r) => {
            let mut vals = Vec::with_capacity(r.count());
            r.for_each(lap.width, |i| vals.push(lap.data[i] as f64));
            if vals.is_empty() {
                return f64::NAN;
            }
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
        }
    }
}

/// Percentil `q` de |Laplaciano| en la región (`np.percentile`, interpolación lineal): el detalle
/// más nítido presente. Separó joyas nítidas de blandas mejor que la densidad (AUC ~0.95).
pub fn laplacian_peak(lap: &ImageF32, region: Region, q: f64) -> f64 {
    let mut vals: Vec<f32> = Vec::with_capacity(region.count());
    region.for_each(lap.width, |i| vals.push(lap.data[i].abs()));
    percentile(&mut vals, q)
}

/// `np.percentile(v, q)` con el método lineal por defecto (0 si no hay valores).
pub fn percentile(vals: &mut [f32], q: f64) -> f64 {
    let n = vals.len();
    if n == 0 {
        return 0.0;
    }
    let pos = q / 100.0 * (n - 1) as f64;
    let lo = pos.floor() as usize;
    let t = pos - lo as f64;
    let (_, a, rest) = vals.select_nth_unstable_by(lo, |a, b| a.total_cmp(b));
    let a = *a as f64;
    if lo + 1 >= n || t == 0.0 {
        return a;
    }
    let b = rest.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    // numpy `_lerp`: a + (b-a)*t, o b - (b-a)*(1-t) cuando t >= 0.5
    let d = b - a;
    if t >= 0.5 { b - d * (1.0 - t) } else { a + d * t }
}

/// Anisotropía del gradiente en una caja: 1 = gradientes en todas direcciones (nítido); hacia 0 = una
/// dirección borrada (barrido por movimiento). El Sobel se hace sobre el recorte (borde REFLECT_101).
pub fn blur_anisotropy(gray: &[u8], width: usize, bx: [usize; 4]) -> f64 {
    let [x0, y0, x1, y1] = bx;
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    if h < 8 || w < 8 {
        return 1.0;
    }
    let mut roi = Vec::with_capacity(w * h);
    for y in y0..y1 {
        roi.extend(gray[y * width + x0..y * width + x1].iter().map(|&v| v as f32));
    }
    let (gx, gy) = image::sobel3(&roi, w, h);
    let (mut jxx, mut jyy, mut jxy) = (0f64, 0f64, 0f64);
    for (&a, &b) in gx.iter().zip(&gy) {
        let (a, b) = (a as f64, b as f64);
        jxx += a * a;
        jyy += b * b;
        jxy += a * b;
    }
    let tr = jxx + jyy;
    if tr <= 1e-6 {
        return 1.0;
    }
    let disc = (((jxx - jyy) / 2.0).powi(2) + jxy * jxy).max(0.0).sqrt();
    let (l1, l2) = (tr / 2.0 + disc, tr / 2.0 - disc);
    if l1 > 0.0 { l2 / l1 } else { 1.0 }
}

/// Exposición: luma media (0..1) y fracción de píxeles quemados en negro (<=2) y blanco (>=253).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exposure {
    pub mean_luma: f64,
    pub clip_black: f64,
    pub clip_white: f64,
}

/// Exposición de todo el cuadro gris.
pub fn exposure(gray: &[u8]) -> Exposure {
    if gray.is_empty() {
        return Exposure { mean_luma: 0.0, clip_black: 0.0, clip_white: 0.0 };
    }
    let h = image::histogram(gray);
    let n = gray.len() as f64;
    let sum: u64 = h.iter().enumerate().map(|(v, &c)| v as u64 * c).sum();
    let black: u64 = h[..=2].iter().sum();
    let white: u64 = h[253..].iter().sum();
    Exposure { mean_luma: sum as f64 / n / 255.0, clip_black: black as f64 / n, clip_white: white as f64 / n }
}

/// Toma LOG: tonos apretados en una banda estrecha (sin negros ni blancos de verdad).
pub fn is_flat(bgr: &[u8], width: usize, height: usize, span_max: f64) -> bool {
    let gray = image::bgr_to_gray(bgr, width, height);
    let p = image::u8_percentiles(&gray, &[0.5, 99.5]);
    p[1] - p[0] < span_max
}

/// Tabla de croma de `normalize_contrast`: el LOG también comprime la saturación, se abre alrededor del neutro.
fn chroma_lut() -> [u8; 256] {
    std::array::from_fn(|i| (128f32 + (i as f32 - 128f32) * 1.4f32).clamp(0.0, 255.0) as u8)
}

/// Estira la luma a todo el rango, como un paso rápido de LOG a display, conservando colores.
///
/// Los modelos y los umbrales de nitidez se ajustaron sobre material de display; el LOG (S-Log,
/// V-Log…) es plano y gris, esconde las joyas y hace que todo mida blando. Hecho con tablas de 256.
pub fn normalize_contrast(bgr: &[u8], width: usize, height: usize, lo_pct: f64, hi_pct: f64) -> Vec<u8> {
    let _ = (width, height);
    // solo la luma para los percentiles; el YCrCb completo se calcula al vuelo en la pasada final
    let mut y = vec![0u8; bgr.len() / 3];
    y.par_chunks_mut(4096).zip(bgr.par_chunks(3 * 4096)).for_each(|(o, s)| {
        for (d, p) in o.iter_mut().zip(s.chunks_exact(3)) {
            *d = image::ycrcb_px(p[0], p[1], p[2])[0];
        }
    });
    let p = image::u8_percentiles(&y, &[lo_pct, hi_pct]);
    let (lo, hi) = (p[0], p[1]);
    if hi - lo < 1.0 {
        return bgr.to_vec();
    }
    // misma aritmética en float32 que numpy (los floats de Python se convierten a float32)
    let lo32 = lo as f32;
    let k32 = (255.0 / (hi - lo)) as f32;
    let pi32 = std::f64::consts::PI as f32;
    let luma: [u8; 256] = std::array::from_fn(|i| {
        let y = ((i as f32 - lo32) * k32).clamp(0.0, 255.0);
        // curva en S suave: los medios del LOG quedan altos, se llevan a un contraste de display
        let y2 = 255f32 * (0.5f32 - 0.5f32 * (pi32 * y / 255f32).cos()) * 0.35f32 + y * 0.65f32;
        y2.clamp(0.0, 255.0) as u8
    });
    let chroma = chroma_lut();
    // una sola pasada: BGR → YCrCb → tablas → BGR, píxel a píxel (misma aritmética que en tres pasadas)
    let mut out = vec![0u8; bgr.len()];
    out.par_chunks_mut(3 * 4096).zip(bgr.par_chunks(3 * 4096)).for_each(|(o, s)| {
        for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
            let [yy, cr, cb] = image::ycrcb_px(p[0], p[1], p[2]);
            d.copy_from_slice(&image::bgr_from_ycrcb_px(luma[yy as usize], chroma[cr as usize], chroma[cb as usize]));
        }
    });
    out
}

/// Correlación de fase entre muestras: desplazamiento global (dx, dy, confianza) en píxeles del cuadro.
///
/// Se hace a media resolución (4 veces menos píxeles; los movimientos de cámara abarcan muchos
/// píxeles) y se reescala, como `cv2.phaseCorrelate` con ventana de Hanning.
pub struct CameraShift {
    planner: FftPlanner<f64>,
    window: Option<(usize, usize, Arc<Vec<f32>>)>,
    /// Espectro del cuadro anterior (se reutiliza: cada cuadro se transforma una sola vez).
    prev: Option<(usize, usize, Vec<Complex64>)>,
}

impl Default for CameraShift {
    fn default() -> Self {
        Self::new()
    }
}

/// Mitad de resolución por área de un cuadro gris (`cv2.resize(..., fx=0.5, fy=0.5, INTER_AREA)`).
pub fn half_gray(gray: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    // tamaño de destino como OpenCV: round(w * 0.5)
    let dw = ((width as f64) * 0.5).round() as usize;
    let dh = ((height as f64) * 0.5).round() as usize;
    (image::resize_area_u8(gray, width, height, 1, dw, dh), dw, dh)
}

/// Menor tamaño >= n de la forma 2^a·3^b·5^c (`cv2.getOptimalDFTSize`).
pub fn optimal_dft_size(n: usize) -> usize {
    let mut m = n.max(1);
    loop {
        let mut k = m;
        for p in [2, 3, 5] {
            while k % p == 0 {
                k /= p;
            }
        }
        if k == 1 {
            return m;
        }
        m += 1;
    }
}

impl CameraShift {
    pub fn new() -> Self {
        CameraShift { planner: FftPlanner::new(), window: None, prev: None }
    }

    /// Olvida el cuadro anterior (inicio de clip).
    pub fn reset(&mut self) {
        self.prev = None;
    }

    fn hanning(&mut self, w: usize, h: usize) -> Arc<Vec<f32>> {
        if let Some((ww, hh, win)) = &self.window {
            if *ww == w && *hh == h {
                return win.clone();
            }
        }
        // cv2.createHanningWindow de OpenCV 5: sqrt(wr * wc) con 0.5*(1-cos(2πj/(n-1))) por eje
        // (comprobado contra cv2: diferencia máxima 6e-8)
        let c0 = 2.0 * std::f64::consts::PI / (w as f64 - 1.0);
        let c1 = 2.0 * std::f64::consts::PI / (h as f64 - 1.0);
        let wc: Vec<f64> = (0..w).map(|j| 0.5 * (1.0 - (c0 * j as f64).cos())).collect();
        let mut win = vec![0f32; w * h];
        for i in 0..h {
            let wr = 0.5 * (1.0 - (c1 * i as f64).cos());
            for j in 0..w {
                win[i * w + j] = (wr * wc[j]).sqrt() as f32;
            }
        }
        let win = Arc::new(win);
        self.window = Some((w, h, win.clone()));
        win
    }

    /// Espectro 2D (filas y luego columnas) del cuadro de media resolución con ventana, relleno con ceros.
    fn spectrum(&mut self, half: &[u8], w: usize, h: usize) -> (usize, usize, Vec<Complex64>) {
        let win = self.hanning(w, h);
        let (mw, mh) = (optimal_dft_size(w), optimal_dft_size(h));
        let mut buf = vec![Complex64::new(0.0, 0.0); mw * mh];
        for y in 0..h {
            for x in 0..w {
                // float32 como en OpenCV: imagen * ventana
                buf[y * mw + x] = Complex64::new((half[y * w + x] as f32 * win[y * w + x]) as f64, 0.0);
            }
        }
        fft2(&mut self.planner, &mut buf, mw, mh, false);
        (mw, mh, buf)
    }

    /// Desplazamiento entre el cuadro anterior y este (gris a resolución de análisis).
    /// Devuelve (0, 0, 0) en el primer cuadro.
    pub fn shift(&mut self, gray: &[u8], width: usize, height: usize) -> (f64, f64, f64) {
        let (half, w, h) = half_gray(gray, width, height);
        let (mw, mh, cur) = self.spectrum(&half, w, h);
        let prev = self.prev.take();
        let out = match &prev {
            Some((pw, ph, p)) if *pw == mw && *ph == mh => {
                let (dx, dy, r) = phase_correlate(&mut self.planner, p, &cur, mw, mh);
                (dx * 2.0, dy * 2.0, r)
            }
            _ => (0.0, 0.0, 0.0),
        };
        self.prev = Some((mw, mh, cur));
        out
    }
}

/// FFT 2D en sitio (sin escalar), filas y luego columnas.
fn fft2(planner: &mut FftPlanner<f64>, buf: &mut [Complex64], w: usize, h: usize, inverse: bool) {
    // Tareas de varias filas: `process` sobre un bloque hace una FFT por cada `len` valores con un solo
    // búfer auxiliar (mismo resultado que fila a fila, sin el coste de repartir cientos de tareas).
    const TASKS: usize = 8;
    let row = if inverse { planner.plan_fft_inverse(w) } else { planner.plan_fft_forward(w) };
    buf.par_chunks_mut(w * h.div_ceil(TASKS).max(1)).for_each(|r| row.process(r));
    let col = if inverse { planner.plan_fft_inverse(h) } else { planner.plan_fft_forward(h) };
    // columnas: se trasponen a un buffer contiguo, se transforman y se devuelven (en paralelo: cada
    // columna/fila de destino es independiente; mismos valores que en serie)
    let mut t = vec![Complex64::new(0.0, 0.0); w * h];
    {
        let src: &[Complex64] = buf;
        let per = w.div_ceil(TASKS).max(1);
        t.par_chunks_mut(h * per).enumerate().for_each(|(k, blk)| {
            for (j, c) in blk.chunks_mut(h).enumerate() {
                let x = k * per + j;
                for (y, v) in c.iter_mut().enumerate() {
                    *v = src[y * w + x];
                }
            }
            col.process(blk);
        });
    }
    buf.par_chunks_mut(w).enumerate().for_each(|(y, r)| {
        for (x, v) in r.iter_mut().enumerate() {
            *v = t[x * h + y];
        }
    });
}

/// Núcleo de `cv2.phaseCorrelate` a partir de los dos espectros: espectro cruzado normalizado,
/// transformada inversa, cuadrantes centrados, pico y centroide ponderado 5x5.
fn phase_correlate(planner: &mut FftPlanner<f64>, f1: &[Complex64], f2: &[Complex64], w: usize, h: usize) -> (f64, f64, f64) {
    let eps = f64::EPSILON;
    let mut c: Vec<Complex64> = f1
        .par_iter()
        .zip(f2.par_iter())
        .map(|(a, b)| {
            let p = a * b.conj();
            let m = p.norm();
            p * (m / (m * m + eps))
        })
        .collect();
    // OpenCV trata los coeficientes puramente reales (DC y Nyquist) con |P|² en vez de |P|: se reproduce
    let mut fix = |x: usize, y: usize| {
        let i = y * w + x;
        let p = f1[i] * f2[i].conj();
        let v = p.re;
        c[i] = Complex64::new(v / (v * v + eps), 0.0);
    };
    fix(0, 0);
    if h % 2 == 0 {
        fix(0, h / 2);
    }
    if w % 2 == 0 {
        fix(w / 2, 0);
        if h % 2 == 0 {
            fix(w / 2, h / 2);
        }
    }
    fft2(planner, &mut c, w, h, true);
    // fftShift: el origen pasa al centro
    let (sx, sy) = (w / 2, h / 2);
    let mut r = vec![0f64; w * h];
    for y in 0..h {
        for x in 0..w {
            r[((y + sy) % h) * w + (x + sx) % w] = c[y * w + x].re;
        }
    }
    // pico (el primero en orden de filas, como minMaxLoc)
    let mut best = 0usize;
    for i in 1..r.len() {
        if r[i] > r[best] {
            best = i;
        }
    }
    let (px, py) = ((best % w) as isize, (best / w) as isize);
    let (minr, maxr) = ((py - 2).max(0), (py + 2).min(h as isize - 1));
    let (minc, maxc) = ((px - 2).max(0), (px + 2).min(w as isize - 1));
    let (mut cx, mut cy, mut sum) = (0f64, 0f64, 0f64);
    for y in minr..=maxr {
        for x in minc..=maxc {
            let v = r[y as usize * w + x as usize];
            cx += v * x as f64;
            cy += v * y as f64;
            sum += v;
        }
    }
    let response = sum / (w * h) as f64;
    let sum = sum + f64::EPSILON;
    let (tx, ty) = (cx / sum, cy / sum);
    let (cxm, cym) = (w as f64 / 2.0, h as f64 / 2.0);
    (cxm - tx, cym - ty, response)
}

/// Miniatura 96x96 del gris para saber si algo cambió desde la última muestra (`thumb` de Python).
pub fn thumb(gray: &[u8], width: usize, height: usize) -> Vec<u8> {
    image::resize_area_u8(gray, width, height, 1, 96, 96)
}

/// Verdadero salvo que las dos miniaturas sean casi idénticas (diferencia media absoluta <= `tol`).
pub fn changed(a: Option<&[u8]>, b: &[u8], tol: f64) -> bool {
    match a {
        None => true,
        Some(a) => {
            let s: u64 = a.iter().zip(b).map(|(&x, &y)| (x as i32 - y as i32).unsigned_abs() as u64).sum();
            s as f64 / b.len().max(1) as f64 > tol
        }
    }
}
