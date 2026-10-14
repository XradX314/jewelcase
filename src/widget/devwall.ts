/* Fondo de prueba para mirar el widget en un navegador común (sin Tauri). Copiado de la demo. */
export function drawDevWall(kind: 'bloom' | 'dusk' | 'graphite' | 'light' = 'bloom'): HTMLCanvasElement {
  const wallCanvas = document.createElement('canvas'); wallCanvas.width = 1920; wallCanvas.height = 1080;
  const c = wallCanvas.getContext('2d')!, W = 1920, H = 1080;
  if (kind === 'bloom') {
    c.fillStyle = '#050d24'; c.fillRect(0, 0, W, H);
    c.filter = 'blur(46px)';
    const petal = (x: number, y: number, rx: number, ry: number, rot: number, c1: string, c2: string) => { c.save(); c.translate(x, y); c.rotate(rot); const g = c.createLinearGradient(-rx, 0, rx, 0); g.addColorStop(0, c1); g.addColorStop(1, c2); c.fillStyle = g; c.beginPath(); c.ellipse(0, 0, rx, ry, 0, 0, Math.PI * 2); c.fill(); c.restore(); };
    for (let k = 0; k < 7; k++) { const a = k / 7 * Math.PI * 2; petal(W * 0.55 + Math.cos(a) * 170, H * 0.55 + Math.sin(a) * 150, 330, 120, a, '#0b2e8a', k % 2 ? '#4f8dff' : '#2f62e8'); }
    c.filter = 'blur(80px)'; const g = c.createRadialGradient(W * 0.55, H * 0.55, 10, W * 0.55, H * 0.55, 260); g.addColorStop(0, 'rgba(170,210,255,.9)'); g.addColorStop(1, 'rgba(40,90,220,0)'); c.fillStyle = g; c.fillRect(0, 0, W, H);
  } else if (kind === 'dusk') {
    const g = c.createLinearGradient(0, 0, 0, H); g.addColorStop(0, '#1f1640'); g.addColorStop(0.5, '#8c3f6e'); g.addColorStop(0.72, '#e98d5c'); g.addColorStop(1, '#f5c27a'); c.fillStyle = g; c.fillRect(0, 0, W, H);
  } else if (kind === 'graphite') {
    const g = c.createRadialGradient(W * 0.5, H * 0.45, 50, W * 0.5, H * 0.5, W * 0.7); g.addColorStop(0, '#2e3036'); g.addColorStop(1, '#0f1013'); c.fillStyle = g; c.fillRect(0, 0, W, H);
  } else {
    const g = c.createLinearGradient(0, 0, W, H); g.addColorStop(0, '#e4eaf1'); g.addColorStop(1, '#b6c3d3'); c.fillStyle = g; c.fillRect(0, 0, W, H);
  }
  return wallCanvas;
}
