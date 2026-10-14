"""Procesa una foto de CD (tipo Medium de Cover Art Archive) para usarla como serigrafía.
Misma lógica que irá en src-tauri/src/artwork/disc_crop.rs.

1. Detecta el fondo (color de las esquinas) y el círculo del disco (centro y radio).
2. Valida que sea un CD: el agujero mide 15 mm sobre 120 mm (12,5 % del diámetro).
   Una etiqueta de vinilo tiene el agujero mucho más chico relativo a su tamaño.
3. Reescala a 1024 px = 120 mm, centrado.
4. Canal alfa: tinta = opaco; zonas plateadas sin imprimir = transparente (ahí el shader pone el espejo y el arcoíris);
   cerca del centro, transparente solo donde la foto deja ver el fondo (agujero y policarbonato sin imprimir);
   la tinta puede llegar hasta el agujero de 15 mm. Borde (> 58,8 mm) transparente.

Uso: python disc_process.py entrada.jpg salida.png
"""
import sys
import numpy as np
from PIL import Image, ImageFilter

OUT = 1024
PX_PER_MM = OUT / 120.0


def bg_color(a):
    h, w, _ = a.shape
    k = max(4, min(h, w) // 40)
    corners = np.concatenate([a[:k, :k].reshape(-1, 3), a[:k, -k:].reshape(-1, 3), a[-k:, :k].reshape(-1, 3), a[-k:, -k:].reshape(-1, 3)])
    return np.median(corners, axis=0)


def find_disc(a, bg):
    diff = np.abs(a.astype(np.float32) - bg).sum(axis=2)
    mask = diff > 40
    ys, xs = np.nonzero(mask)
    if len(xs) < 100:
        return None
    # robusto frente a manchas sueltas: percentiles en vez de min/max
    x0, x1 = np.percentile(xs, [0.3, 99.7]); y0, y1 = np.percentile(ys, [0.3, 99.7])
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    r = ((x1 - x0) + (y1 - y0)) / 4
    return cx, cy, r, mask


def ring_mean(mask, cx, cy, r0, r1):
    h, w = mask.shape
    yy, xx = np.mgrid[0:h, 0:w]
    d = np.hypot(xx - cx, yy - cy)
    sel = (d >= r0) & (d < r1)
    return mask[sel].mean() if sel.any() else 0.0


def is_cd(a, cx, cy, r):
    """Mide el radio del agujero recorriendo rayos desde el centro hasta que el color cambia.
    En un CD el agujero (más la zona transparente que deja ver lo mismo) llega al menos al 10 % del radio;
    en una etiqueta de vinilo el agujero ronda el 7 %."""
    h, w, _ = a.shape
    c0 = a[int(round(cy)), int(round(cx))].astype(np.float32)
    radii = []
    for k in range(24):
        ang = k / 24 * 2 * np.pi
        hit = 0.5
        for t in np.linspace(0.01, 0.5, 120):
            x = int(round(cx + np.cos(ang) * t * r)); y = int(round(cy + np.sin(ang) * t * r))
            if not (0 <= x < w and 0 <= y < h):
                break
            if np.abs(a[y, x].astype(np.float32) - c0).sum() > 70:
                hit = t; break
        radii.append(hit)
    hole = float(np.median(radii))
    return hole >= 0.10, hole, 0.0


def process(src, dst):
    im = Image.open(src).convert("RGB")
    a = np.asarray(im)
    bg = bg_color(a)
    found = find_disc(a, bg)
    if not found:
        return {"ok": False, "reason": "no se encontró el disco"}
    cx, cy, r, mask = found
    ok, hole, body = is_cd(a, cx, cy, r)
    if not ok:
        return {"ok": False, "reason": f"no parece un CD (agujero = {hole:.3f} del radio; un CD tiene 0,125)"}
    # recorte centrado: el radio detectado pasa a ser 60 mm
    s = (60 * PX_PER_MM) / r
    big = im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)
    ox, oy = round(cx * s - OUT / 2), round(cy * s - OUT / 2)
    canvas = Image.new("RGB", (OUT, OUT), tuple(int(v) for v in bg))
    canvas.paste(big, (-ox, -oy))
    c = np.asarray(canvas).astype(np.float32) / 255.0

    # máscara de plata: poco color, gris medio-claro y textura suave
    mx, mn = c.max(axis=2), c.min(axis=2)
    sat = (mx - mn) / np.maximum(mx, 1e-3)
    val = mx
    gray = Image.fromarray((c.mean(axis=2) * 255).astype(np.uint8))
    blur = np.asarray(gray.filter(ImageFilter.GaussianBlur(3))).astype(np.float32) / 255
    local = np.abs(c.mean(axis=2) - blur)
    silver = np.clip((0.10 - sat) / 0.05, 0, 1) * np.clip((val - 0.42) / 0.12, 0, 1) * np.clip((0.97 - val) / 0.05, 0, 1) * np.clip((0.06 - local) / 0.04, 0, 1)
    silver = np.asarray(Image.fromarray((silver * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.2))).astype(np.float32) / 255
    # solo se aplica si el disco es mayormente plateado: en fotos impresas (cielos, grises) da falsos positivos
    frac = float(silver.mean())
    alpha = 1.0 - 0.95 * silver if frac >= 0.30 else np.ones_like(silver)

    yy, xx = np.mgrid[0:OUT, 0:OUT]
    d = np.hypot(xx - OUT / 2 + 0.5, yy - OUT / 2 + 0.5) / PX_PER_MM
    # el centro no se recorta fijo: la impresión puede llegar casi al agujero (referencia: Clics modernos).
    # Solo es transparente lo que en la foto deja ver el fondo (agujero y zona transparente sin imprimir).
    bgd = np.abs(c - bg / 255.0).sum(axis=2)
    keyed = np.clip((bgd - 0.10) / 0.12, 0, 1)
    inner = np.clip((24.0 - d) / 2.0, 0, 1)          # el recorte por color de fondo solo actúa cerca del centro
    alpha *= 1.0 - inner * (1.0 - keyed)
    alpha *= np.clip((d - 7.6) / 0.3, 0, 1) * np.clip((58.8 - d) / 0.4, 0, 1)

    rgba = np.dstack([c, alpha])
    Image.fromarray((rgba * 255).astype(np.uint8), "RGBA").save(dst)
    return {"ok": True, "silver": frac if frac >= 0.30 else 0.0, "center": (cx, cy), "radius_px": r}


if __name__ == "__main__":
    print(process(sys.argv[1], sys.argv[2]))
