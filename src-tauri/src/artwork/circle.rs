//! Detección de un disco por sus bordes (transformada de Hough circular), para fotos sin fondo liso: un CD apoyado en la bandeja
//! gris de un digipak, sobre una mesa, etc. Solo se usa con la foto que el usuario elige a mano: como filtro automático no sirve,
//! porque una portada con un círculo dibujado también da un puntaje alto (medido: 0,99 contra 0,94 a 1,33 de los discos reales).
use image::{imageops, imageops::FilterType, RgbImage};

#[allow(dead_code)] // score y agujero los usa la prueba `puntajes`
pub struct Fit {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    /// bordes que apoyan el círculo exterior, por píxel de circunferencia (≈1 es un círculo completo y nítido)
    pub score: f64,
    /// radio del anillo central relativo al exterior, y su apoyo
    pub hole_ratio: f64,
    pub hole_score: f64,
    /// actividad (gradiente medio, 0 a 1) fuera del círculo: un fondo liso o suave da valores bajos; uno movido (un macro del disco) alto.
    /// 0 si el disco ocupa todo el cuadro y no queda fondo que medir.
    pub outside_busy: f64,
}

struct Edge {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
}

/// Bordes de Sobel: posición y dirección unitaria del gradiente, de los píxeles con gradiente fuerte (percentil 90).
fn edges(g: &[f32], w: usize, h: usize) -> Vec<Edge> {
    let mut grad = vec![(0f32, 0f32, 0f32); w * h];
    let mut mags = Vec::with_capacity(w * h);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: isize, dy: isize| g[(y as isize + dy) as usize * w + (x as isize + dx) as usize];
            let gx = -p(-1, -1) - 2.0 * p(-1, 0) - p(-1, 1) + p(1, -1) + 2.0 * p(1, 0) + p(1, 1);
            let gy = -p(-1, -1) - 2.0 * p(0, -1) - p(1, -1) + p(-1, 1) + 2.0 * p(0, 1) + p(1, 1);
            let m = gx.hypot(gy);
            grad[y * w + x] = (gx, gy, m);
            mags.push(m);
        }
    }
    if mags.is_empty() {
        return Vec::new();
    }
    mags.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let thr = mags[(mags.len() as f32 * 0.90) as usize].max(0.25);
    let mut out = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let (gx, gy, m) = grad[y * w + x];
            if m >= thr {
                out.push(Edge { x: x as f32, y: y as f32, dx: gx / m, dy: gy / m });
            }
        }
    }
    out
}

pub fn fit(im: &RgbImage) -> Option<Fit> {
    let (w0, h0) = im.dimensions();
    let scale = (200.0 / w0.max(h0) as f64).min(1.0);
    let (w, h) = (((w0 as f64 * scale).round() as usize).max(16), ((h0 as f64 * scale).round() as usize).max(16));
    let small = imageops::resize(im, w as u32, h as u32, FilterType::Triangle);
    let g: Vec<f32> = small.pixels().map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0).collect();
    let es = edges(&g, w, h);
    if es.len() < 150 {
        return None;
    }
    // el disco ocupa casi todo el lado menor de la foto
    let (rmin, rmax) = ((0.36 * w.min(h) as f32).floor() as usize, (0.56 * w.max(h) as f32).ceil() as usize);
    let mut acc = vec![0u16; w * h];
    let mut best = (0u32, 0usize, 0usize, 0usize); // votos, radio, cx, cy
    for r in rmin..=rmax {
        acc.iter_mut().for_each(|a| *a = 0);
        for e in &es {
            for s in [-1.0f32, 1.0] {
                let (cx, cy) = ((e.x + s * r as f32 * e.dx).round(), (e.y + s * r as f32 * e.dy).round());
                if cx >= 0.0 && cy >= 0.0 && (cx as usize) < w && (cy as usize) < h {
                    acc[cy as usize * w + cx as usize] += 1;
                }
            }
        }
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if acc[y * w + x] == 0 {
                    continue;
                }
                let mut sum = 0u32;
                for yy in y - 1..=y + 1 {
                    for xx in x - 1..=x + 1 {
                        sum += acc[yy * w + xx] as u32;
                    }
                }
                if sum > best.0 {
                    best = (sum, r, x, y);
                }
            }
        }
    }
    let (votes, r, cx, cy) = best;
    if votes == 0 {
        return None;
    }
    let score = votes as f64 / (std::f64::consts::TAU * r as f64);

    // anillo central: bordes a distancia r_in del centro con gradiente radial
    let (mut hole_r, mut hole_score) = (0.0f64, 0.0f64);
    let (lo, hi) = ((0.07 * r as f64).ceil() as usize, (0.32 * r as f64).floor() as usize);
    for r_in in lo..=hi.max(lo) {
        let mut n = 0u32;
        for e in &es {
            let (vx, vy) = (e.x as f64 - cx as f64, e.y as f64 - cy as f64);
            let d = vx.hypot(vy);
            if (d - r_in as f64).abs() <= 1.2 && d > 0.0 && ((vx * e.dx as f64 + vy * e.dy as f64) / d).abs() > 0.7 {
                n += 1;
            }
        }
        let sc = n as f64 / (std::f64::consts::TAU * r_in as f64);
        if sc > hole_score {
            hole_score = sc;
            hole_r = r_in as f64;
        }
    }
    // actividad fuera del círculo (diferencias centrales sobre la imagen chica)
    let (mut sum, mut n) = (0.0f64, 0u64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if ((x as f64 - cx as f64).hypot(y as f64 - cy as f64)) / (r as f64) > 1.07 {
                let gx = (g[y * w + x + 1] - g[y * w + x - 1]) as f64;
                let gy = (g[(y + 1) * w + x] - g[(y - 1) * w + x]) as f64;
                sum += gx.abs() + gy.abs();
                n += 1;
            }
        }
    }
    let outside_busy = if n >= 400 { sum / n as f64 } else { 0.0 };
    Some(Fit { cx: cx as f64 / scale, cy: cy as f64 / scale, r: r as f64 / scale, score, hole_ratio: hole_r / r as f64, hole_score, outside_busy })
}

/// Afina un círculo (en píxeles de la imagen original) buscando alrededor el que tenga más borde radial en toda su circunferencia.
/// El Hough grueso se equivoca unos píxeles; así el radio queda exacto y el disco no sale cortado ni con margen.
pub fn refine(im: &RgbImage, cx: f64, cy: f64, r: f64) -> (f64, f64, f64) {
    let (w, h) = im.dimensions();
    let gray = |x: f64, y: f64| -> Option<f32> {
        if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
            return None;
        }
        let p = im.get_pixel(x as u32, y as u32);
        Some(0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
    };
    let score = |cx: f64, cy: f64, r: f64| -> f64 {
        let (mut sum, mut n) = (0.0f64, 0u32);
        for k in 0..240 {
            let a = k as f64 / 240.0 * std::f64::consts::TAU;
            let (c, s) = (a.cos(), a.sin());
            if let (Some(i), Some(o)) = (gray(cx + (r - 3.0) * c, cy + (r - 3.0) * s), gray(cx + (r + 3.0) * c, cy + (r + 3.0) * s)) {
                sum += (i - o).abs() as f64;
                n += 1;
            }
        }
        if n < 100 { 0.0 } else { sum / n as f64 }
    };
    let (mut best, mut best_s) = ((cx, cy, r), score(cx, cy, r));
    // de grueso a fino: el paso baja con el radio de búsqueda
    for step in [r * 0.02, r * 0.01, r * 0.004, 1.0] {
        let step = step.max(1.0);
        let center = best;
        for ix in -3i32..=3 {
            for iy in -3i32..=3 {
                for ir in -4i32..=4 {
                    let cand = (center.0 + ix as f64 * step, center.1 + iy as f64 * step, center.2 + ir as f64 * step);
                    let sc = score(cand.0, cand.1, cand.2);
                    if sc > best_s {
                        best_s = sc;
                        best = cand;
                    }
                }
            }
        }
    }
    // El máximo de contraste suele caer en el borde del área impresa (negro contra el aro plateado), unos 3 % por dentro del borde real
    // del disco (R 60 mm). Se toma el borde más exterior cuyo contraste sea comparable, para que el disco no salga ampliado y cortado.
    let (bx, by, br) = best;
    let steps: Vec<f64> = (0..=((0.14 * br) / 0.25) as usize).map(|i| br * 0.95 + i as f64 * 0.25).collect();
    let prof: Vec<f64> = steps.iter().map(|&rr| score(bx, by, rr)).collect();
    let max = prof.iter().cloned().fold(0.0f64, f64::max);
    let mut outer = br;
    for i in (4..prof.len().saturating_sub(4)).rev() {
        if prof[i] >= 0.55 * max && (1..=4).all(|k| prof[i] >= prof[i - k] && prof[i] >= prof[i + k]) {
            outer = steps[i];
            break;
        }
    }
    (bx, by, outer.max(br))
}

/// Umbrales de la detección automática (imágenes de Discogs), calibrados con 33 imágenes reales y 5 discos de Cover Art Archive:
/// los escaneos buenos dan puntaje 0,56 a 1,33 y fondo 0,000 a 0,008; el ruido (contratapas, fotos sueltas) puntaje ≤ 0,37; el macro
/// del centro de un disco con fondo de arcoíris, fondo 0,10.
const AUTO_SCORE_MIN: f64 = 0.5;
const AUTO_BUSY_MAX: f64 = 0.03;

/// Detección automática para fotos de disco sin fondo uniforme (escaneos con sombras, degradé o bordes): círculo nítido, fondo
/// tranquilo (no un macro) y un anillo central. El radio se afina contra el borde exterior real del disco.
pub fn fit_auto(im: &RgbImage) -> Option<(f64, f64, f64)> {
    let f = fit(im)?;
    let ring = f.hole_ratio >= 0.09 && f.hole_ratio <= 0.33 && f.hole_score >= 0.3;
    if f.score >= AUTO_SCORE_MIN && f.outside_busy <= AUTO_BUSY_MAX && ring {
        Some(refine(im, f.cx, f.cy, f.r))
    } else {
        None
    }
}

/// Para una foto que el usuario eligió como disco: ubica el círculo sin exigir más validación y lo afina.
pub fn fit_manual(im: &RgbImage) -> Option<(f64, f64, f64)> {
    let f = fit(im)?;
    Some(refine(im, f.cx, f.cy, f.r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Imprime los puntajes de todas las imágenes de una carpeta: `CIRCLE_DIR=... cargo test puntajes -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn puntajes() {
        let dir = std::env::var("CIRCLE_DIR").expect("CIRCLE_DIR");
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let Ok(img) = image::open(&p) else { continue };
            let t = std::time::Instant::now();
            match fit(&img.to_rgb8()) {
                Some(f) => println!(
                    "{:40} círculo r={:.0} centro=({:.0},{:.0}) puntaje={:.2} | agujero {:.3} del radio, apoyo {:.2} | fondo {:.4} ({:?})",
                    p.file_name().unwrap().to_string_lossy(), f.r, f.cx, f.cy, f.score, f.hole_ratio, f.hole_score, f.outside_busy, t.elapsed()
                ),
                None => println!("{:40} sin círculo", p.file_name().unwrap().to_string_lossy()),
            }
        }
    }
}
