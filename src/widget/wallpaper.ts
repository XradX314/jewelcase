/* Wallpaper real: Rust da la ruta/estilo/color (y los bytes), acá se compone un lienzo del tamaño del monitor
   con el mismo ajuste que usa Windows, y después se recorta lo que queda detrás de la ventana. */
import { invoke } from '../shared/bridge';

export interface WallInfo { path: string; style: number; tile: number; color: [number, number, number]; mtime: number; }
export interface Layout {
  mon: { x: number; y: number; w: number; h: number };   // monitor donde está el widget (px físicos)
  win: { x: number; y: number; w: number; h: number };   // área cliente del widget (px físicos)
}

export async function loadWallpaper(): Promise<{ info: WallInfo; img: ImageBitmap | null }> {
  const info = await invoke<WallInfo>('wallpaper_info');
  let img: ImageBitmap | null = null;
  if (info.path) {
    try {
      const buf = await invoke<ArrayBuffer>('wallpaper_bytes');
      img = await createImageBitmap(new Blob([buf]));
    } catch (e) { console.warn('no se pudo leer el wallpaper', e); }
  }
  return { info, img };
}

/* Lienzo con el escritorio del monitor: color de fondo + imagen según WallpaperStyle/TileWallpaper. */
export function composeMonitor(info: WallInfo, img: ImageBitmap | null, mw: number, mh: number, into?: HTMLCanvasElement): HTMLCanvasElement {
  const cv = into ?? document.createElement('canvas');
  if (cv.width !== mw || cv.height !== mh) { cv.width = mw; cv.height = mh; }
  const c = cv.getContext('2d')!;
  c.fillStyle = `rgb(${info.color[0]},${info.color[1]},${info.color[2]})`; c.fillRect(0, 0, mw, mh);
  if (!img) return cv;
  const iw = img.width, ih = img.height;
  c.imageSmoothingQuality = 'high';
  const style = info.style, tile = info.tile;
  if (style === 0 && tile === 1) {            // mosaico
    for (let y = 0; y < mh; y += ih) for (let x = 0; x < mw; x += iw) c.drawImage(img, x, y);
  } else if (style === 0) {                   // centrar
    c.drawImage(img, Math.round((mw - iw) / 2), Math.round((mh - ih) / 2));
  } else if (style === 2) {                   // estirar
    c.drawImage(img, 0, 0, mw, mh);
  } else if (style === 6) {                   // ajustar
    const s = Math.min(mw / iw, mh / ih); const w = iw * s, h = ih * s;
    c.drawImage(img, (mw - w) / 2, (mh - h) / 2, w, h);
  } else {                                    // 10 rellenar (y 22 extender: tratado como rellenar por monitor)
    const s = Math.max(mw / iw, mh / ih); const w = iw * s, h = ih * s;
    c.drawImage(img, (mw - w) / 2, (mh - h) / 2, w, h);
  }
  return cv;
}

/* Recorte [u0, v0, du, dv] de la textura del monitor que queda detrás de la ventana (v hacia arriba, como en WebGL). */
export function cropRect(l: Layout): [number, number, number, number] {
  const wx = l.win.x - l.mon.x, wy = l.win.y - l.mon.y;
  return [wx / l.mon.w, 1 - (wy + l.win.h) / l.mon.h, l.win.w / l.mon.w, l.win.h / l.mon.h];
}
