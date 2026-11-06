//! Operaciones de imagen básicas, reproduciendo la aritmética exacta de OpenCV 5 (la que usa Python).
//!
//! Todo trabaja sobre buffers planos por filas. Las conversiones de 8 bits usan la misma aritmética
//! de punto fijo que OpenCV (comprobado contra `cv2` en el venv: 0 diferencias en imágenes aleatorias).

use rayon::prelude::*;

/// Imagen de un canal en `f32` (gradientes, Laplaciano).
#[derive(Debug, Clone)]
pub struct ImageF32 {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

/// BGR → gris como `cv2.cvtColor(..., COLOR_BGR2GRAY)` en OpenCV 5:
/// `(B*3735 + G*19235 + R*9798 + 2^14) >> 15` (coeficientes hallados contra cv2, 0 diferencias).
#[inline]
pub fn gray_px(b: u8, g: u8, r: u8) -> u8 {
    ((b as u32 * 3735 + g as u32 * 19235 + r as u32 * 9798 + (1 << 14)) >> 15) as u8
}

/// Convierte un cuadro BGR completo a gris (en paralelo por filas).
pub fn bgr_to_gray(bgr: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    out.par_chunks_mut(width.max(1)).enumerate().for_each(|(y, row)| {
        let src = &bgr[y * width * 3..(y + 1) * width * 3];
        for (x, o) in row.iter_mut().enumerate() {
            *o = gray_px(src[3 * x], src[3 * x + 1], src[3 * x + 2]);
        }
    });
    out
}

/// Un píxel BGR → YCrCb de 8 bits (aritmética de `cv2.COLOR_BGR2YCrCb`).
#[inline]
pub fn ycrcb_px(b: u8, g: u8, r: u8) -> [u8; 3] {
    let (b, g, r) = (b as i32, g as i32, r as i32);
    let y = (b * 1868 + g * 9617 + r * 4899 + 8192) >> 14;
    let cr = ((r - y) * 11682 + (128 << 14) + 8192) >> 14;
    let cb = ((b - y) * 9241 + (128 << 14) + 8192) >> 14;
    [y.clamp(0, 255) as u8, cr.clamp(0, 255) as u8, cb.clamp(0, 255) as u8]
}

/// Un píxel YCrCb → BGR de 8 bits (aritmética de `cv2.COLOR_YCrCb2BGR`).
#[inline]
pub fn bgr_from_ycrcb_px(y: u8, cr: u8, cb: u8) -> [u8; 3] {
    let y = y as i32;
    let cr = cr as i32 - 128;
    let cb = cb as i32 - 128;
    let ds = |v: i32| (v + 8192) >> 14;
    [(y + ds(cb * 29049)).clamp(0, 255) as u8, (y + ds(cb * -5636 + cr * -11698)).clamp(0, 255) as u8, (y + ds(cr * 22987)).clamp(0, 255) as u8]
}

/// BGR → YCrCb de 8 bits como `cv2.COLOR_BGR2YCrCb` (punto fijo de 14 bits).
pub fn bgr_to_ycrcb(bgr: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; bgr.len()];
    out.par_chunks_mut(3 * 4096).zip(bgr.par_chunks(3 * 4096)).for_each(|(o, s)| {
        for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
            d.copy_from_slice(&ycrcb_px(p[0], p[1], p[2]));
        }
    });
    out
}

/// YCrCb → BGR de 8 bits como `cv2.COLOR_YCrCb2BGR` (punto fijo de 14 bits).
pub fn ycrcb_to_bgr(ycc: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; ycc.len()];
    out.par_chunks_mut(3 * 4096).zip(ycc.par_chunks(3 * 4096)).for_each(|(o, s)| {
        for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
            d.copy_from_slice(&bgr_from_ycrcb_px(p[0], p[1], p[2]));
        }
    });
    out
}

/// `cv2.transform` de 8 bits con matriz 3x3: OpenCV redondea la matriz a punto fijo de 10 bits
/// y hace `(Σ m·x + 512) >> 10` (comprobado contra cv2: 0 diferencias). `m` está en orden BGR.
pub fn transform_u8(bgr: &[u8], m: &[[f32; 3]; 3]) -> Vec<u8> {
    let mi: [[i32; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| (m[r][c] as f64 * 1024.0).round_ties_even() as i32));
    let mut out = vec![0u8; bgr.len()];
    out.par_chunks_mut(3 * 4096).zip(bgr.par_chunks(3 * 4096)).for_each(|(o, s)| {
        for (d, p) in o.chunks_exact_mut(3).zip(s.chunks_exact(3)) {
            let (a, b, c) = (p[0] as i32, p[1] as i32, p[2] as i32);
            for r in 0..3 {
                let v = (mi[r][0] * a + mi[r][1] * b + mi[r][2] * c + 512) >> 10;
                d[r] = v.clamp(0, 255) as u8;
            }
        }
    });
    out
}

/// Índice con borde BORDER_REFLECT_101 (`gfedcb|abcdefgh|gfedcba`).
#[inline]
fn reflect101(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n == 1 {
        return 0;
    }
    let mut i = i;
    if i < 0 {
        i = -i;
    }
    if i >= n {
        i = 2 * n - 2 - i;
    }
    i as usize
}

/// Sobel 3x3 en x e y (como `cv2.Sobel(img, CV_32F, 1, 0, ksize=3)` y `(0, 1)`), borde REFLECT_101.
/// Con entradas enteras de 8 bits las sumas son exactas en `f32`, igual que en OpenCV.
pub fn sobel3(gray: &[f32], width: usize, height: usize) -> (Vec<f32>, Vec<f32>) {
    let mut gx = vec![0f32; width * height];
    let mut gy = vec![0f32; width * height];
    if width == 0 || height == 0 {
        return (gx, gy);
    }
    gx.par_chunks_mut(width).zip(gy.par_chunks_mut(width)).enumerate().for_each(|(y, (rx, ry))| {
        let ym = reflect101(y as isize - 1, height);
        let yp = reflect101(y as isize + 1, height);
        let (a, b, c) = (&gray[ym * width..(ym + 1) * width], &gray[y * width..(y + 1) * width], &gray[yp * width..(yp + 1) * width]);
        for x in 0..width {
            let xm = reflect101(x as isize - 1, width);
            let xp = reflect101(x as isize + 1, width);
            // dx: [-1 0 1] en filas, [1 2 1] en columnas; dy al revés
            let dm = a[xp] - a[xm];
            let d0 = b[xp] - b[xm];
            let dp = c[xp] - c[xm];
            rx[x] = dm + 2.0 * d0 + dp;
            let sm = a[xm] + 2.0 * a[x] + a[xp];
            let sp = c[xm] + 2.0 * c[x] + c[xp];
            ry[x] = sp - sm;
        }
    });
    (gx, gy)
}

/// Remuestreo por área (`cv2.INTER_AREA`) de una imagen de `ch` canales de 8 bits.
///
/// Reducción exacta a la mitad: OpenCV usa `(a+b+c+d+2) >> 2`. Otras escalas: pesos por solape en
/// `f32` y redondeo al par, como `resizeArea_` de OpenCV.
pub fn resize_area_u8(src: &[u8], w: usize, h: usize, ch: usize, dw: usize, dh: usize) -> Vec<u8> {
    if dw == w && dh == h {
        return src.to_vec();
    }
    let mut out = vec![0u8; dw * dh * ch];
    if w == dw * 2 && h == dh * 2 {
        out.par_chunks_mut(dw * ch).enumerate().for_each(|(y, row)| {
            let r0 = &src[(2 * y) * w * ch..(2 * y + 1) * w * ch];
            let r1 = &src[(2 * y + 1) * w * ch..(2 * y + 2) * w * ch];
            for x in 0..dw {
                for c in 0..ch {
                    let s = r0[(2 * x) * ch + c] as u32 + r0[(2 * x + 1) * ch + c] as u32
                        + r1[(2 * x) * ch + c] as u32 + r1[(2 * x + 1) * ch + c] as u32;
                    row[x * ch + c] = ((s + 2) >> 2) as u8;
                }
            }
        });
        return out;
    }
    let xtab = area_tab(w, dw);
    let ytab = area_tab(h, dh);
    out.par_chunks_mut(dw * ch).enumerate().for_each(|(dy, row)| {
        let mut acc = vec![0f32; dw * ch];
        for &(sy, wy) in &ytab[dy] {
            let srow = &src[sy * w * ch..(sy + 1) * w * ch];
            for (dx, tab) in xtab.iter().enumerate() {
                for c in 0..ch {
                    let mut s = 0f32;
                    for &(sx, wx) in tab {
                        s += srow[sx * ch + c] as f32 * wx;
                    }
                    acc[dx * ch + c] += s * wy;
                }
            }
        }
        for (o, a) in row.iter_mut().zip(acc) {
            *o = a.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    });
    out
}

/// Pesos de `INTER_AREA` para un eje (como `computeResizeAreaTab` de OpenCV).
fn area_tab(ssize: usize, dsize: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = ssize as f64 / dsize as f64;
    (0..dsize)
        .map(|dx| {
            let fsx1 = dx as f64 * scale;
            let fsx2 = fsx1 + scale;
            let cell = scale.min(ssize as f64 - fsx1);
            let mut sx1 = fsx1.ceil() as isize;
            let mut sx2 = fsx2.floor() as isize;
            sx2 = sx2.min(ssize as isize - 1);
            sx1 = sx1.min(sx2);
            let mut tab = Vec::new();
            if sx1 as f64 - fsx1 > 1e-3 {
                tab.push(((sx1 - 1) as usize, ((sx1 as f64 - fsx1) / cell) as f32));
            }
            for sx in sx1..sx2 {
                tab.push((sx as usize, (1.0 / cell) as f32));
            }
            if fsx2 - sx2 as f64 > 1e-3 {
                tab.push((sx2 as usize, ((fsx2 - sx2 as f64).min(1.0).min(cell) / cell) as f32));
            }
            tab
        })
        .collect()
}

/// Histograma de 256 niveles de una imagen de 8 bits.
pub fn histogram(img: &[u8]) -> [u64; 256] {
    img.par_chunks(1 << 16)
        .map(|c| {
            let mut h = [0u64; 256];
            for &v in c {
                h[v as usize] += 1;
            }
            h
        })
        .reduce(|| [0u64; 256], |mut a, b| {
            for i in 0..256 {
                a[i] += b[i];
            }
            a
        })
}

/// Percentiles de una imagen de 8 bits a partir del histograma (`_u8_percentiles` de Python):
/// el primer nivel cuyo acumulado alcanza `q/100 * total` (`np.searchsorted` por la izquierda).
pub fn u8_percentiles(img: &[u8], qs: &[f64]) -> Vec<f64> {
    let h = histogram(img);
    let mut cum = [0f64; 256];
    let mut acc = 0f32; // np.cumsum de un histograma float32 acumula en float32
    for i in 0..256 {
        acc += h[i] as f32;
        cum[i] = acc as f64;
    }
    let total = cum[255];
    qs.iter()
        .map(|q| {
            let v = q / 100.0 * total;
            cum.iter().position(|&c| c >= v).unwrap_or(256) as f64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflect() {
        assert_eq!(reflect101(-1, 5), 1);
        assert_eq!(reflect101(5, 5), 3);
        assert_eq!(reflect101(2, 5), 2);
    }
}
