//! Captura en vivo de lo que hay detrás del widget (ventanas, íconos, wallpaper).
//! La ventana del widget se excluye de la captura (WDA_EXCLUDEFROMCAPTURE), así que el recorte muestra solo lo de atrás.
//! Se copia únicamente el rectángulo de la ventana con GDI (BitBlt desde el escritorio compuesto) y se manda al frontend
//! como bytes crudos: 8 bytes de cabecera (ancho y alto, u32 little endian) + RGBA de abajo hacia arriba (listo para una DataTexture).
use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    AppHandle, Manager,
};

type Hwnd = isize;
type Hdc = isize;

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_ppm: i32,
    y_ppm: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
    fn SetWindowDisplayAffinity(hwnd: Hwnd, affinity: u32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn CreateCompatibleBitmap(hdc: Hdc, w: i32, h: i32) -> isize;
    fn SelectObject(hdc: Hdc, obj: isize) -> isize;
    fn BitBlt(dst: Hdc, x: i32, y: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32, rop: u32) -> i32;
    fn GetDIBits(hdc: Hdc, bmp: isize, start: u32, lines: u32, bits: *mut c_void, info: *mut BitmapInfo, usage: u32) -> i32;
    fn DeleteObject(obj: isize) -> i32;
    fn DeleteDC(hdc: Hdc) -> i32;
    fn StretchBlt(dst: Hdc, x: i32, y: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> i32;
    fn SetStretchBltMode(hdc: Hdc, mode: i32) -> i32;
}

const SRCCOPY: u32 = 0x00CC_0020;
const CAPTUREBLT: u32 = 0x4000_0000;
const HALFTONE: i32 = 4;
/// Lado máximo de lo que se manda al frontend: la refracción no necesita más, y a 400 % la ventana pasa los 2000 px (16 MB por cuadro).
const MAX_SIDE: i32 = 768;
const WDA_NONE: u32 = 0;
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

/// Cada start/stop cambia la generación; el hilo viejo se da cuenta y termina.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn set_affinity(app: &AppHandle, exclude: bool) {
    if let Some(w) = app.get_webview_window("widget") {
        if let Ok(h) = w.hwnd() {
            // SAFETY: el HWND es de una ventana viva de este proceso.
            unsafe { SetWindowDisplayAffinity(h.0 as Hwnd, if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) };
        }
    }
}

pub fn stop(app: &AppHandle) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    set_affinity(app, false);
}

/// Mientras haya una app a pantalla completa (juego, video) o la pantalla esté bloqueada, no se captura nada (lo marca `lib.rs`).
pub static BUSY: AtomicBool = AtomicBool::new(false);

pub fn start(app: AppHandle, channel: Channel<InvokeResponseBody>, fps: u64) {
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    set_affinity(&app, true);
    std::thread::spawn(move || {
        let frame_time = Duration::from_millis(1000 / fps.max(1));
        let mut grabber = Grabber::default();
        let mut last: Vec<u8> = Vec::new();
        // cuando el escritorio no cambia se mira cada tanto en vez de a 30 cuadros por segundo; apenas cambia, vuelve la frecuencia plena
        let mut same = 0u32;
        while GENERATION.load(Ordering::SeqCst) == gen {
            let t0 = Instant::now();
            let busy = BUSY.load(Ordering::SeqCst);
            if !busy {
                if let Some(w) = app.get_webview_window("widget") {
                    if w.is_visible().unwrap_or(false) {
                        if let (Ok(p), Ok(s)) = (w.inner_position(), w.inner_size()) {
                            if let Some((tw, th)) = grabber.grab(p.x, p.y, s.width as i32, s.height as i32) {
                                let pix = &grabber.raw[..(tw * th * 4) as usize];
                                if pix != last.as_slice() {
                                    same = 0;
                                    last.clear();
                                    last.extend_from_slice(pix);
                                    // cabecera (ancho y alto) + RGBA; el intercambio BGRA -> RGBA solo se hace si hubo cambios
                                    let mut out = vec![0u8; 8 + pix.len()];
                                    out[0..4].copy_from_slice(&(tw as u32).to_le_bytes());
                                    out[4..8].copy_from_slice(&(th as u32).to_le_bytes());
                                    for (o, q) in out[8..].chunks_exact_mut(4).zip(pix.chunks_exact(4)) {
                                        o[0] = q[2];
                                        o[1] = q[1];
                                        o[2] = q[0];
                                        o[3] = 255;
                                    }
                                    if channel.send(InvokeResponseBody::Raw(out)).is_err() {
                                        break;
                                    }
                                } else {
                                    same = same.saturating_add(1);
                                }
                            }
                        }
                    }
                }
            }
            let wait = if busy {
                Duration::from_millis(500)
            } else if same >= 10 {
                Duration::from_millis(250)
            } else {
                frame_time
            };
            if let Some(rest) = wait.checked_sub(t0.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    });
}

#[derive(Default)]
struct Grabber {
    size: (i32, i32),
    mem: Hdc,
    bmp: isize,
    /// píxeles de la última copia (BGRA, de abajo hacia arriba); se reutiliza el mismo buffer en cada cuadro
    raw: Vec<u8>,
}

impl Grabber {
    fn ensure(&mut self, screen: Hdc, w: i32, h: i32) {
        if self.mem != 0 && self.size == (w, h) {
            return;
        }
        self.release();
        // SAFETY: handles de GDI creados y liberados por esta estructura.
        unsafe {
            self.mem = CreateCompatibleDC(screen);
            self.bmp = CreateCompatibleBitmap(screen, w, h);
            SelectObject(self.mem, self.bmp);
        }
        self.size = (w, h);
    }

    fn release(&mut self) {
        // SAFETY: solo libera lo que esta estructura creó.
        unsafe {
            if self.bmp != 0 {
                DeleteObject(self.bmp);
            }
            if self.mem != 0 {
                DeleteDC(self.mem);
            }
        }
        self.mem = 0;
        self.bmp = 0;
    }

    /// Copia la región de la pantalla (reducida a MAX_SIDE) a `self.raw` en BGRA, de abajo hacia arriba. Devuelve el tamaño copiado.
    fn grab(&mut self, x: i32, y: i32, w: i32, h: i32) -> Option<(i32, i32)> {
        if w <= 0 || h <= 0 {
            return None;
        }
        // si la ventana es grande, se copia ya reducida (StretchBlt con HALFTONE promedia, no salta píxeles)
        let k = (w.max(h) as f32 / MAX_SIDE as f32).max(1.0);
        let (tw, th) = (((w as f32 / k).round() as i32).max(1), ((h as f32 / k).round() as i32).max(1));
        // SAFETY: llamadas GDI con handles válidos; el buffer tiene tw*th*4 bytes.
        unsafe {
            let screen = GetDC(0);
            if screen == 0 {
                return None;
            }
            self.ensure(screen, tw, th);
            let ok = if (tw, th) == (w, h) {
                BitBlt(self.mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT)
            } else {
                SetStretchBltMode(self.mem, HALFTONE);
                StretchBlt(self.mem, 0, 0, tw, th, screen, x, y, w, h, SRCCOPY | CAPTUREBLT)
            };
            let need = (tw * th * 4) as usize;
            if self.raw.len() != need {
                self.raw.resize(need, 0);
            }
            let mut info = BitmapInfo {
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width: tw,
                    height: th, // positivo: filas de abajo hacia arriba, como espera una DataTexture
                    planes: 1,
                    bit_count: 32,
                    compression: 0,
                    size_image: 0,
                    x_ppm: 0,
                    y_ppm: 0,
                    clr_used: 0,
                    clr_important: 0,
                },
                colors: [0],
            };
            let lines = GetDIBits(self.mem, self.bmp, 0, th as u32, self.raw.as_mut_ptr() as *mut c_void, &mut info, 0);
            ReleaseDC(0, screen);
            if ok == 0 || lines == 0 {
                return None;
            }
            Some((tw, th))
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        self.release();
    }
}
