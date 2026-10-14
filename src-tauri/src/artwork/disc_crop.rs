//! Procesa una foto de CD (tipo Medium de Cover Art Archive) para usarla como serigrafía.
//! Puerto de prototype/disc_process.py: detecta fondo y círculo, valida que sea un CD midiendo el agujero,
//! centra a 1024 px = 120 mm y arma el canal alfa (tinta opaca, zonas plateadas sin imprimir transparentes).
use image::{imageops, GrayImage, ImageBuffer, Luma, Rgb, RgbImage, Rgba, RgbaImage};
use std::path::Path;

const OUT: u32 = 1024;
/// Detalle local mínimo para considerar que una foto de disco es el lado impreso (ver `process_with`).
const MIN_PRINTED_DETAIL: f32 = 0.008;
const PX_PER_MM: f32 = OUT as f32 / 120.0;
/// Cuánto de la foto se deja pasar a espejo/arcoíris en las zonas plateadas (1 = todo). Con 0,95 (prototipo) un disco
/// plateado impreso se veía gris oscuro de frente, porque el espejo solo refleja el estudio; con 0,5 conserva la mitad del color.
const SILVER_KEY: f32 = 0.5;

fn clip01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Color de fondo: mediana de los cuatro rincones.
fn bg_color(im: &RgbImage) -> [f32; 3] {
    let (w, h) = im.dimensions();
    let k = (w.min(h) / 40).max(4).min(w.min(h) / 2);
    let mut ch: [Vec<f32>; 3] = [vec![], vec![], vec![]];
    for (x0, y0) in [(0, 0), (w - k, 0), (0, h - k), (w - k, h - k)] {
        for y in y0..y0 + k {
            for x in x0..x0 + k {
                let p = im.get_pixel(x, y).0;
                for i in 0..3 {
                    ch[i].push(p[i] as f32);
                }
            }
        }
    }
    [median(&mut ch[0]), median(&mut ch[1]), median(&mut ch[2])]
}

/// Percentil `p` (0..100) de las posiciones donde hay máscara, a partir de un histograma por columna o fila.
fn percentile(counts: &[u64], p: f64) -> f64 {
    let total: u64 = counts.iter().sum();
    let target = total as f64 * p / 100.0;
    let mut acc = 0.0;
    for (i, &c) in counts.iter().enumerate() {
        acc += c as f64;
        if acc >= target {
            return i as f64;
        }
    }
    (counts.len() - 1) as f64
}

/// Centro y radio del disco: lo que difiere del fondo, con percentiles para ser robusto frente a manchas sueltas.
fn find_disc(im: &RgbImage, bg: [f32; 3]) -> Option<(f64, f64, f64)> {
    let (w, h) = im.dimensions();
    let (mut cols, mut rows, mut n) = (vec![0u64; w as usize], vec![0u64; h as usize], 0u64);
    for (x, y, p) in im.enumerate_pixels() {
        let d = (p[0] as f32 - bg[0]).abs() + (p[1] as f32 - bg[1]).abs() + (p[2] as f32 - bg[2]).abs();
        if d > 40.0 {
            cols[x as usize] += 1;
            rows[y as usize] += 1;
            n += 1;
        }
    }
    if n < 100 {
        return None;
    }
    let (x0, x1) = (percentile(&cols, 0.3), percentile(&cols, 99.7));
    let (y0, y1) = (percentile(&rows, 0.3), percentile(&rows, 99.7));
    Some(((x0 + x1) / 2.0, (y0 + y1) / 2.0, ((x1 - x0) + (y1 - y0)) / 4.0))
}

/// Mide el radio del agujero recorriendo rayos desde el centro hasta que el color cambia.
/// En un CD llega al menos al 10 % del radio; en una etiqueta de vinilo ronda el 7 %.
fn hole_ratio(im: &RgbImage, cx: f64, cy: f64, r: f64) -> f32 {
    let (w, h) = im.dimensions();
    let c0 = im.get_pixel(cx.round().clamp(0.0, (w - 1) as f64) as u32, cy.round().clamp(0.0, (h - 1) as f64) as u32).0;
    let mut radii = Vec::new();
    for k in 0..24 {
        let ang = k as f64 / 24.0 * std::f64::consts::TAU;
        let mut hit = 0.5f32;
        for i in 0..120 {
            let t = 0.01 + (0.5 - 0.01) * i as f64 / 119.0;
            let x = (cx + ang.cos() * t * r).round();
            let y = (cy + ang.sin() * t * r).round();
            if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
                break;
            }
            let p = im.get_pixel(x as u32, y as u32).0;
            let d: i32 = (0..3).map(|i| (p[i] as i32 - c0[i] as i32).abs()).sum();
            if d > 70 {
                hit = t as f32;
                break;
            }
        }
        radii.push(hit);
    }
    median(&mut radii)
}

fn blur_f32(a: &[f32], sigma: f32) -> Vec<f32> {
    let img: GrayImage = ImageBuffer::from_fn(OUT, OUT, |x, y| Luma([(a[(y * OUT + x) as usize] * 255.0).round().clamp(0.0, 255.0) as u8]));
    imageops::blur(&img, sigma).pixels().map(|p| p[0] as f32 / 255.0).collect()
}

/// Disco ya recortado con alfa: el círculo sale de la máscara de alfa y el agujero se mide por donde empieza lo opaco.
/// Se centra a 1024 px = 120 mm y se conserva el alfa original (recortado a 7,6–58,8 mm). No hay máscara de plata: es arte impreso.
fn process_cdart(im: &RgbaImage, dst: &Path) -> Result<u32, String> {
    let (w, h) = im.dimensions();
    let (mut cols, mut rows, mut n) = (vec![0u64; w as usize], vec![0u64; h as usize], 0u64);
    for (x, y, p) in im.enumerate_pixels() {
        if p[3] > 128 {
            cols[x as usize] += 1;
            rows[y as usize] += 1;
            n += 1;
        }
    }
    if n < 100 {
        return Err("imagen casi vacía".into());
    }
    let (x0, x1) = (percentile(&cols, 0.3), percentile(&cols, 99.7));
    let (y0, y1) = (percentile(&rows, 0.3), percentile(&rows, 99.7));
    let (cx, cy, r) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, ((x1 - x0) + (y1 - y0)) / 4.0);
    let mut radii = Vec::new();
    for k in 0..24 {
        let ang = k as f64 / 24.0 * std::f64::consts::TAU;
        let mut hit = 0.5f32;
        for i in 0..120 {
            let t = 0.01 + (0.5 - 0.01) * i as f64 / 119.0;
            let (x, y) = ((cx + ang.cos() * t * r).round(), (cy + ang.sin() * t * r).round());
            if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
                break;
            }
            if im.get_pixel(x as u32, y as u32)[3] > 128 {
                hit = t as f32;
                break;
            }
        }
        radii.push(hit);
    }
    let hole = median(&mut radii);
    if hole < 0.10 {
        return Err(format!("no parece un CD (agujero = {hole:.3} del radio; un CD tiene 0,125)"));
    }
    let s = (60.0 * PX_PER_MM as f64) / r;
    let big = imageops::resize(im, ((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1), imageops::FilterType::Lanczos3);
    let (ox, oy) = ((cx * s - OUT as f64 / 2.0).round() as i64, (cy * s - OUT as f64 / 2.0).round() as i64);
    let out: RgbaImage = ImageBuffer::from_fn(OUT, OUT, |x, y| {
        let (sx, sy) = (x as i64 + ox, y as i64 + oy);
        let p = if sx >= 0 && sy >= 0 && (sx as u32) < big.width() && (sy as u32) < big.height() { *big.get_pixel(sx as u32, sy as u32) } else { Rgba([0, 0, 0, 0]) };
        let d = ((x as f32 - OUT as f32 / 2.0 + 0.5).hypot(y as f32 - OUT as f32 / 2.0 + 0.5)) / PX_PER_MM;
        let a = p[3] as f32 / 255.0 * clip01((d - 7.6) / 0.3) * clip01((58.8 - d) / 0.4);
        Rgba([p[0], p[1], p[2], (a * 255.0).round().clamp(0.0, 255.0) as u8])
    });
    out.save(dst).map_err(|e| e.to_string())?;
    Ok((2.0 * r) as u32)
}

/// Procesa los bytes de la imagen y guarda un PNG RGBA de 1024 px. Error si no es un CD.
/// Medidas de una foto de disco ya procesada.
#[derive(Clone, Copy)]
pub struct Stats {
    /// fracción plateada (0 a 1)
    pub silver: f32,
    /// detalle local medio en la zona de datos: texto y dibujo dan valores altos; un reverso liso, muy bajos
    pub detail: f32,
    /// resolución: diámetro del disco en la imagen original, en píxeles (la salida es de 1024)
    pub px: u32,
}

/// `strict`: para imágenes sin garantía de ser un disco (Discogs): exige además que fuera del círculo el fondo sea uniforme.
/// Devuelve cuánto del disco es plateado (0 a 1): el lado de datos de un CD es casi todo plata; el lado impreso, mucho menos.
pub fn process_with(bytes: &[u8], dst: &Path, strict: bool) -> Result<Stats, String> {
    let decoded = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    // "cdart" (Fanart.tv, TheAudioDB): PNG ya recortado, con el fondo y el agujero transparentes
    if decoded.color().has_alpha() {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        let k = (w.min(h) / 40).max(2).min(w.min(h) / 2);
        let corner_alpha = [(0, 0), (w - k, 0), (0, h - k), (w - k, h - k)].iter().map(|&(x, y)| rgba.get_pixel(x, y)[3]).max().unwrap_or(255);
        if corner_alpha < 16 {
            return process_cdart(&rgba, dst).map(|px| Stats { silver: 0.0, detail: 1.0, px });
        }
    }
    let im = decoded.to_rgb8();
    let bg = bg_color(&im);
    // método clásico: fondo liso + agujero. Con imágenes sin garantía (Discogs), si falla se prueba la detección por bordes.
    let classic: Result<(f64, f64, f64), String> = match find_disc(&im, bg) {
        None => Err("no se encontró el disco".into()),
        Some((cx, cy, r)) => {
            let hole = hole_ratio(&im, cx, cy, r);
            if hole < 0.10 {
                Err(format!("no parece un CD (agujero = {hole:.3} del radio; un CD tiene 0,125)"))
            } else if strict && !uniform_outside(&im, bg, cx, cy, r) {
                Err("el fondo no es uniforme: no parece una foto de disco".into())
            } else {
                Ok((cx, cy, r))
            }
        }
    };
    let (cx, cy, r) = match classic {
        Ok(c) => c,
        Err(e) if strict => crate::artwork::circle::fit_auto(&im).ok_or(e)?,
        Err(e) => return Err(e),
    };
    render_disc(&im, bg, cx, cy, r, dst, strict)
}

/// Fuera del círculo el fondo es (casi) del mismo color que los rincones.
fn uniform_outside(im: &RgbImage, bg: [f32; 3], cx: f64, cy: f64, r: f64) -> bool {
    let (mut outside, mut differ) = (0u64, 0u64);
    for (x, y, p) in im.enumerate_pixels() {
        if (x as f64 - cx).hypot(y as f64 - cy) / r > 1.08 {
            outside += 1;
            let d = (p[0] as f32 - bg[0]).abs() + (p[1] as f32 - bg[1]).abs() + (p[2] as f32 - bg[2]).abs();
            if d > 40.0 {
                differ += 1;
            }
        }
    }
    outside == 0 || differ as f64 / outside as f64 <= 0.03
}

/// Foto que el usuario eligió como disco (URL manual): si el método clásico (fondo liso + agujero) la reconoce, se usa; si no,
/// se ubica el círculo por sus bordes y se afina. No se exige más validación: el usuario garantiza que es el lado impreso.
pub fn process_manual(bytes: &[u8], dst: &Path) -> Result<Stats, String> {
    let decoded = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    if decoded.color().has_alpha() {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        let k = (w.min(h) / 40).max(2).min(w.min(h) / 2);
        let corner_alpha = [(0, 0), (w - k, 0), (0, h - k), (w - k, h - k)].iter().map(|&(x, y)| rgba.get_pixel(x, y)[3]).max().unwrap_or(255);
        if corner_alpha < 16 {
            return process_cdart(&rgba, dst).map(|px| Stats { silver: 0.0, detail: 1.0, px });
        }
    }
    let im = decoded.to_rgb8();
    let bg = bg_color(&im);
    let classic = find_disc(&im, bg).filter(|&(cx, cy, r)| hole_ratio(&im, cx, cy, r) >= 0.10 && uniform_outside(&im, bg, cx, cy, r));
    let (cx, cy, r) = match classic {
        Some(c) => c,
        None => crate::artwork::circle::fit_manual(&im).ok_or("no se pudo ubicar el disco en la imagen")?,
    };
    render_disc(&im, bg, cx, cy, r, dst, false)
}

/// Recorta el disco centrado a 1024 px = 120 mm y arma el alfa (ver el comentario del módulo).
fn render_disc(im: &RgbImage, bg: [f32; 3], cx: f64, cy: f64, r: f64, dst: &Path, strict: bool) -> Result<Stats, String> {
    // recorte centrado: el radio detectado pasa a ser 60 mm
    let s = (60.0 * PX_PER_MM as f64) / r;
    let big = imageops::resize(im, ((im.width() as f64 * s).round() as u32).max(1), ((im.height() as f64 * s).round() as u32).max(1), imageops::FilterType::Lanczos3);
    let (ox, oy) = ((cx * s - OUT as f64 / 2.0).round() as i64, (cy * s - OUT as f64 / 2.0).round() as i64);
    let bgp = Rgb([bg[0].round() as u8, bg[1].round() as u8, bg[2].round() as u8]);
    let canvas: RgbImage = ImageBuffer::from_fn(OUT, OUT, |x, y| {
        let (sx, sy) = (x as i64 + ox, y as i64 + oy);
        if sx >= 0 && sy >= 0 && (sx as u32) < big.width() && (sy as u32) < big.height() { *big.get_pixel(sx as u32, sy as u32) } else { bgp }
    });
    let n = (OUT * OUT) as usize;
    let c: Vec<[f32; 3]> = canvas.pixels().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();

    // máscara de plata: poco color, gris medio-claro y textura suave
    let mean: Vec<f32> = c.iter().map(|p| (p[0] + p[1] + p[2]) / 3.0).collect();
    let blurred = blur_f32(&mean, 3.0);
    let mut silver: Vec<f32> = (0..n)
        .map(|i| {
            let p = c[i];
            let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
            let sat = (mx - mn) / mx.max(1e-3);
            let local = (mean[i] - blurred[i]).abs();
            clip01((0.10 - sat) / 0.05) * clip01((mx - 0.42) / 0.12) * clip01((0.97 - mx) / 0.05) * clip01((0.06 - local) / 0.04)
        })
        .collect();
    silver = blur_f32(&silver, 1.2);
    // solo se aplica si el disco es mayormente plateado: en fotos impresas (cielos, grises) da falsos positivos
    let frac = silver.iter().sum::<f32>() / n as f32;
    let mut alpha: Vec<f32> = if frac >= 0.30 { silver.iter().map(|s| 1.0 - SILVER_KEY * s).collect() } else { vec![1.0; n] };

    for y in 0..OUT {
        for x in 0..OUT {
            let i = (y * OUT + x) as usize;
            let d = ((x as f32 - OUT as f32 / 2.0 + 0.5).hypot(y as f32 - OUT as f32 / 2.0 + 0.5)) / PX_PER_MM;
            // el centro no se recorta fijo: la impresión puede llegar casi al agujero (referencia: Clics modernos).
            // Solo es transparente lo que en la foto deja ver el fondo (agujero y zona transparente sin imprimir).
            let p = c[i];
            let bgd = (p[0] - bg[0] / 255.0).abs() + (p[1] - bg[1] / 255.0).abs() + (p[2] - bg[2] / 255.0).abs();
            let keyed = clip01((bgd - 0.10) / 0.12);
            let inner = clip01((24.0 - d) / 2.0); // el recorte por color de fondo solo actúa cerca del centro
            alpha[i] *= 1.0 - inner * (1.0 - keyed);
            alpha[i] *= clip01((d - 7.6) / 0.3) * clip01((58.8 - d) / 0.4);
        }
    }
    let out: RgbaImage = ImageBuffer::from_fn(OUT, OUT, |x, y| {
        let i = (y * OUT + x) as usize;
        let p = canvas.get_pixel(x, y).0;
        Rgba([p[0], p[1], p[2], (alpha[i] * 255.0).round().clamp(0.0, 255.0) as u8])
    });
    out.save(dst).map_err(|e| e.to_string())?;
    // detalle local medio en el anillo de datos (24 a 56 mm): distingue una etiqueta impresa de un reverso liso
    let (mut sum, mut cnt) = (0.0f64, 0u64);
    for y in 0..OUT {
        for x in 0..OUT {
            let d = ((x as f32 - OUT as f32 / 2.0 + 0.5).hypot(y as f32 - OUT as f32 / 2.0 + 0.5)) / PX_PER_MM;
            if d > 24.0 && d < 56.0 {
                let i = (y * OUT + x) as usize;
                sum += (mean[i] - blurred[i]).abs() as f64;
                cnt += 1;
            }
        }
    }
    let detail = (sum / cnt.max(1) as f64) as f32;
    // Discogs no dice de qué lado es cada foto: un reverso (lado de datos) es casi liso, una etiqueta impresa tiene texto y dibujo.
    // Medido: etiquetas 0,017 a 0,053; reverso 0,002. Solo se aplica a imágenes sin garantía (modo estricto).
    if strict && detail < MIN_PRINTED_DETAIL {
        let _ = std::fs::remove_file(dst);
        return Err(format!("parece el lado de datos del CD, sin impresión (detalle {detail:.4})"));
    }
    Ok(Stats { silver: frac, detail, px: (2.0 * r) as u32 })
}
