import '@fontsource/chivo/400.css';
import '@fontsource/chivo/500.css';
import '@fontsource/chivo/700.css';
import '@fontsource/chivo-mono/400.css';
import '@fontsource/chivo-mono/500.css';
import { createJewelCase } from '../render/jewelcase';
import { inTauri, getConfig, onConfig, patchConfig, listen, invoke, emit } from '../shared/bridge';
import type { Config } from '../shared/config';
import { drawDevWall } from './devwall';
import { loadWallpaper, composeMonitor, cropRect, type Layout, type WallInfo } from './wallpaper';

const canvas = document.getElementById('stage') as HTMLCanvasElement;

async function main() {
  try {
    await Promise.race([document.fonts.ready, new Promise(r => setTimeout(r, 1500))]);
    await document.fonts.load('700 40px Chivo'); await document.fonts.load('400 20px "Chivo Mono"');
  } catch { /* sigue con la fuente de respaldo */ }

  const jc = createJewelCase(canvas);
  let cfg: Config = await getConfig();
  if (!inTauri) {   // en el navegador: ?style=upload&yaw=0&pitch=0&disc=/dev/disc.png&front=/dev/front.jpg para mirar arte real sin Tauri
    const q = new URLSearchParams(location.search);
    for (const [k, v] of q) if (k in cfg) (cfg as Record<string, unknown>)[k] = typeof (cfg as Record<string, unknown>)[k] === 'number' ? Number(v) : v === 'true' ? true : v === 'false' ? false : v;
    const load = (u: string) => new Promise<HTMLImageElement>((res, rej) => { const i = new Image(); i.onload = () => res(i); i.onerror = rej; i.src = u; });
    if (q.get('front')) jc.setCoverImage(await load(q.get('front')!));
    if (q.get('disc')) jc.setDiscImage(await load(q.get('disc')!));
  }
  jc.setConfig(cfg);
  await onConfig(c => { cfg = c; jc.setConfig(c); });
  addEventListener('resize', () => jc.resize());

  // Eventos desde la bandeja y la ventana de ajustes
  await listen('toggle-lid', () => jc.setOpen(!jc.isOpen()));
  // con una app a pantalla completa (juego, video) o la pantalla bloqueada no se dibuja ni se captura nada
  await listen<boolean>('app-busy', busy => jc.setPaused(!!busy));
  // giro continuo de la caja: el botón de Ajustes alterna; al frenar se guarda el ángulo donde quedó
  jc.onSpinStop((yaw: number) => { void patchConfig({ yaw: Math.round(yaw) }); });
  await listen('spin-box', () => { void emit('spin-state', jc.toggleBoxSpin()); });
  await listen('reload-art', () => { void reloadWallpaper(true); void loadArt(true, spOver ?? undefined); });

  /* ---------- Fondo ---------- */
  let monCanvas: HTMLCanvasElement | null = null;
  let wall: { info: WallInfo; img: ImageBitmap | null } | null = null;
  let layout: Layout | null = null;

  function applyWall() {
    if (!inTauri || !layout || !wall) return;
    monCanvas = composeMonitor(wall.info, wall.img, layout.mon.w, layout.mon.h, monCanvas ?? undefined);
    jc.setWall(monCanvas, cropRect(layout));
  }
  async function reloadWallpaper(force = false) {
    const prev = wall?.info;
    const next = await loadWallpaper();
    if (!force && prev && prev.path === next.info.path && prev.mtime === next.info.mtime && prev.style === next.info.style && prev.tile === next.info.tile) return;
    wall = next; applyWall();
  }
  if (inTauri) {
    await listen<Layout>('widget-layout', l => {
      const monChanged = !layout || layout.mon.w !== l.mon.w || layout.mon.h !== l.mon.h;
      layout = l;
      if (monChanged) applyWall(); else jc.setWallRect(cropRect(l));
    });
    layout = await invoke<Layout>('layout_info');
    await reloadWallpaper(true);
    setInterval(() => { void reloadWallpaper(); }, 15000);   // presentación de diapositivas o cambio de fondo
  } else {
    jc.setWall(drawDevWall('bloom'), [0.3, 0.3, 0.4, 0.4]);
  }

  /* ---------- Captura en vivo de lo que hay detrás ----------
     Rust excluye la ventana de la captura y manda el recorte de atrás (RGBA, abajo-arriba, con 8 bytes de cabecera). */
  let capturing = false;
  async function setCapture(on: boolean) {
    if (!inTauri || on === capturing) return;
    capturing = on;
    if (on) {
      const { Channel } = await import('@tauri-apps/api/core');
      const ch = new Channel<ArrayBuffer>();
      ch.onmessage = buf => {
        if (!capturing) return;
        const dv = new DataView(buf);
        jc.setLiveFrame(dv.getUint32(0, true), dv.getUint32(4, true), new Uint8Array(buf, 8));
      };
      await invoke('start_capture', { onFrame: ch, fps: 30 });
    } else {
      await invoke('stop_capture');
      jc.useWallpaper();
    }
  }
  void setCapture(cfg.live && cfg.refr);
  await onConfig(c => { void setCapture(c.live && c.refr); });

  /* ---------- Arte real (resolvedor en Rust) ---------- */
  interface Resolved {
    found: boolean; artist: string; title: string; year?: number | null;
    tracks: [string, string][]; disc?: string | null; front?: string | null;
    disc_source?: string | null; disc_url?: string | null; disc_release?: string | null;
    report?: string[]; incomplete?: boolean; disc_px?: number | null;
  }
  /* Qué disco terminó usándose: lo muestra la ventana de Ajustes (que puede abrirse después, así que se guarda y se reenvía). */
  let artInfo: Record<string, unknown> | null = null;
  const sendArtInfo = (i: Record<string, unknown>) => { artInfo = i; void emit('art-info', i); };
  await listen('art-info-request', () => { if (artInfo) void emit('art-info', artInfo); });
  async function bitmapFromCache(name: string): Promise<HTMLImageElement | null> {
    try {
      const buf = await invoke<ArrayBuffer>('read_cache_file', { name });
      const url = URL.createObjectURL(new Blob([buf]));
      const img = new Image(); img.src = url; await img.decode(); return img;
    } catch { return null; }
  }
  async function remoteImage(url: string): Promise<HTMLImageElement | null> {
    try { const img = new Image(); img.crossOrigin = 'anonymous'; img.src = url; await img.decode(); return img; } catch { return null; }
  }
  interface Over { artist: string; album: string; upc?: string | null; tracks?: [string, string][]; cover?: string | null; year?: number | null }
  /* Mejoras en segundo plano: el resolvedor no corta las demás fuentes cuando llega el primer disco; si después alguna entrega uno de mayor
     resolución, reemplaza al actual y avisa por este evento (solo se aplica si sigue siendo el álbum que se está mostrando). */
  let curQuery: { artist: string; album: string; haveFront: boolean; haveTracks: boolean } | null = null;
  interface ArtUpdate { qartist: string; qalbum: string; artist: string; title: string; year?: number | null; front?: string | null; tracks?: [string, string][]; disc?: string | null; disc_source?: string | null; disc_url?: string | null; disc_release?: string | null; disc_px?: number | null; report?: string[] }
  const sameAlbum = (u: ArtUpdate) => !!curQuery && u.qartist === curQuery.artist && u.qalbum === curQuery.album;
  await listen<ArtUpdate>('art-update', async u => {
    if (!sameAlbum(u) || !curQuery) return;
    const q = curQuery;
    // la lista de canciones y la portada pueden llegar después del primer resultado (el resolvedor no espera a que estén para mostrar el disco)
    if (u.tracks?.length && !q.haveTracks) { q.haveTracks = true; jc.setAlbum({ artist: u.artist, title: u.title, year: u.year ?? 0, tracks: u.tracks }); }
    if (u.front && !q.haveFront) {
      const f = await bitmapFromCache(u.front);
      if (f && sameAlbum(u)) { q.haveFront = true; jc.setCoverImage(f); }
    }
    if (!u.disc) return;
    const img = await bitmapFromCache(u.disc);
    if (!img || !sameAlbum(u)) return;
    jc.setDiscImage(img);
    sendArtInfo({ found: true, artist: u.artist, title: u.title, disc: true, source: u.disc_source ?? null, url: u.disc_url ?? null, release: u.disc_release ?? null, report: u.report ?? [], qartist: u.qartist, qalbum: u.qalbum, px: u.disc_px ?? null, upgraded: true });
  });
  let artSeq = 0;
  let artReady = false;   // el álbum que suena ya tiene su primer resultado
  /* `over` viene de Spotify: manda su lista de canciones (el índice tiene que coincidir con el del álbum para reproducir) y, si el
     resolvedor no encuentra la portada, usa la de Spotify. Sin `over` se usa el álbum manual de la config. */
  async function loadArt(force = false, over?: Over) {
    if (!inTauri) return;
    const seq = ++artSeq;
    const artist = over?.artist ?? cfg.artist, album = over?.album ?? cfg.album;
    curQuery = { artist, album, haveFront: false, haveTracks: !!over?.tracks?.length };
    artReady = false;
    if (!over) jc.clearArt();   // con Spotify ya se limpió junto con la lista nueva
    sendArtInfo({ searching: true, artist, title: album, qartist: artist, qalbum: album });
    // Varios intentos durante los primeros 10 s: cada fuente puede fallar por red (500/503 intermitentes de Cover Art Archive y
    // MusicBrainz). Un resultado "incompleto" (algo falló por red) no se guarda en caché, así que se vuelve a pedir; lo que ya llegó
    // se muestra en cada intento. Si todas las fuentes respondieron y no hay disco, no tiene sentido insistir.
    const t0 = Date.now();
    for (let attempt = 1; ; attempt++) {
      let r: Resolved, failed = false;
      // con Spotify ya hay lista de canciones y portada: el resolvedor se ahorra pedirlas (una consulta menos a MusicBrainz)
      try { r = await invoke<Resolved>('resolve_album', { artist, album, upc: over?.upc ?? null, force: force || attempt > 1, wantTracks: !over?.tracks?.length, needFront: !over?.cover }); }
      catch (e) {   // sin red o servicio caído: se queda lo que hay (con Spotify, igual se usa su portada y su lista)
        console.warn('resolvedor de arte:', e);
        r = { found: false, artist, title: album, tracks: [] }; failed = true;
      }
      if (seq !== artSeq) return;
      if (!r.found) console.warn('álbum sin resolver', artist, album);
      const incomplete = failed || !!r.incomplete;
      const retry = incomplete && Date.now() - t0 < 10000;
      sendArtInfo({ found: r.found, artist: r.found ? r.artist : artist, title: r.found ? r.title : album, disc: !!r.disc, source: r.disc_source ?? null, url: r.disc_url ?? null, release: r.disc_release ?? null, px: r.disc_px ?? null, report: r.report ?? [], attempt, incomplete, retrying: retry, failed, qartist: artist, qalbum: album });
      // con Spotify la lista ya se aplicó antes (su orden es el que sirve para reproducir)
      if (!over?.tracks?.length && r.found && r.tracks.length) jc.setAlbum({ artist: r.artist, title: r.title, year: r.year ?? 0, tracks: r.tracks });
      if (r.found || over?.cover) {
        const [front, disc] = await Promise.all([r.front ? bitmapFromCache(r.front) : null, r.disc ? bitmapFromCache(r.disc) : null]);
        if (seq !== artSeq) return;
        jc.setCoverImage(front ?? (over?.cover ? await remoteImage(over.cover) : null)); jc.setDiscImage(disc);
        if (curQuery) { curQuery.haveFront = !!r.front; curQuery.haveTracks = curQuery.haveTracks || r.tracks.length > 0; }
      }
      artReady = true;
      // con disco ya no se reintenta: lo que falte (portada, lista, un disco mejor) llega solo por 'art-update'
      if (!retry || r.disc) return;
      await new Promise(res => setTimeout(res, 1200));
      if (seq !== artSeq) return;
    }
  }
  let artKey = `${cfg.artist}|${cfg.album}`;
  await onConfig(c => { const k = `${c.artist}|${c.album}`; if (k !== artKey) { artKey = k; if (!spAlbum) void loadArt(); } });

  /* ---------- Spotify (etapa 4) ----------
     Cada ~1 s se pregunta qué suena. Si cambia el álbum se trae de Spotify (canciones, UPC) y el resolvedor busca el arte real.
     El CD gira solo mientras suena. Tocar un renglón del librito reproduce esa canción (requiere Premium). */
  interface SpAlbum { id: string; uri: string; title: string; artist: string; year: number | null; upc: string | null; cover: string | null; tracks: { id: string; name: string; dur: string }[] }
  interface SpNow { active: boolean; is_playing?: boolean; progress_ms?: number; track?: { id: string; duration_ms?: number }; album?: { id: string } }
  let spAlbum: SpAlbum | null = null;
  let spOver: Over | null = null;
  let spTrackId: string | null = null;
  let spBusy = false;
  /* Ritmo de los pedidos a Spotify (su límite es por ventana de 30 s y en modo desarrollo es bajo). Sabemos cuánto falta para que
     termine la canción (duración - progreso), así que no se consulta hasta entonces. Los cambios manuales (saltar, pausar, otra app)
     los avisa el control de medios de Windows (SMTC, sin costo: es local) y recién ahí se consulta. Sin SMTC (Spotify en el celular o
     en la web) se consulta cada 5 s mientras suena. Tras un error (429 incluido) se espera largo. */
  let spDelay = 5000;
  let spSmtc = false;                        // ¿Windows ve una sesión de Spotify?
  let spSig = '';                            // canción + estado de la última consulta
  let spRetry = 0;                           // reintentos tras un aviso de SMTC cuya consulta todavía muestra lo mismo (la API tarda ~1 s)
  let spTimer = 0;
  const spError = (e: unknown) => {
    void emit('spotify-error', String(e)); console.warn('spotify', e);
    const m = /\((\d+) s/.exec(String(e));
    spDelay = Math.min(300_000, Math.max(15_000, m ? (Number(m[1]) + 2) * 1000 : spDelay * 2));
  };
  async function spotifyTick() {
    if (!inTauri || spBusy) return;
    spBusy = true;
    try {
      const clientId = cfg.spotifyClientId;
      if (!clientId || !(await invoke<boolean>('spotify_status'))) { spDelay = 15_000; if (spAlbum) { spAlbum = null; spOver = null; jc.setPlaying(null); void loadArt(); } return; }
      const now = await invoke<SpNow>('spotify_now', { clientId });
      if (!now.active || !now.album?.id) { jc.setPlaying(null); spDelay = spSmtc ? 60_000 : 15_000; spSig = ''; return; }
      jc.setPlaying(!!now.is_playing);
      const left = Math.max(0, (now.track?.duration_ms ?? 0) - (now.progress_ms ?? 0));
      const sig = `${now.track?.id}|${!!now.is_playing}`;
      if (spRetry > 0 && sig === spSig) { spRetry--; spDelay = 2000; }   // SMTC avisó pero la API todavía no refleja el cambio
      else {
        spRetry = 0;
        if (spSmtc) spDelay = now.is_playing && left > 0 ? Math.min(left + 1500, 120_000) : 60_000;   // hasta el final de la canción; SMTC avisa antes si cambia
        else spDelay = now.is_playing ? (left > 0 ? Math.min(5000, Math.max(left + 800, 1000)) : 5000) : 15_000;
      }
      spSig = sig;
      if (spAlbum?.id !== now.album.id) {
        const a = await invoke<SpAlbum>('spotify_album', { clientId, id: now.album.id });
        spAlbum = a; spTrackId = null;
        jc.resetArt();   // el arte del álbum anterior sale ya, no recién cuando llega el nuevo
        const tracks = a.tracks.map(t => [t.name, t.dur] as [string, string]);
        jc.setAlbum({ artist: a.artist, title: a.title, year: a.year ?? 0, tracks });   // la lista ya, sin esperar al arte
        // el arte se busca aparte (MusicBrainz tarda: 1 consulta por segundo) para no frenar el seguimiento de canciones
        // la portada de Spotify se muestra ya; el resolvedor la reemplaza si encuentra una mejor
        if (a.cover) void remoteImage(a.cover).then(img => { if (img && spAlbum?.id === a.id) jc.setCoverImage(img); });
        spOver = { artist: a.artist, album: a.title, upc: a.upc, year: a.year, cover: a.cover, tracks };
        void loadArt(false, spOver);
      }
      if (spAlbum && now.track?.id && now.track.id !== spTrackId) {
        spTrackId = now.track.id;
        const i = spAlbum.tracks.findIndex(t => t.id === spTrackId);
        if (i >= 0) jc.setCurrentTrack(i);
      }
    } catch (e) { spError(e); } finally { spBusy = false; }
  }
  /* Precarga del siguiente álbum: la cola de Spotify dice qué viene; si es otro álbum se resuelve ya en segundo plano (UPC, disco, portada),
     así cuando empieza ya está en el caché. Solo después de mostrar el actual, para no competir por MusicBrainz (1 consulta por segundo). */
  const prefetched = new Set<string>();
  let queueFor: string | null = null;   // la cola se consulta una vez por canción, no cada pocos segundos
  async function prefetchNext() {
    const clientId = cfg.spotifyClientId;
    if (!inTauri || !clientId || !artReady || !spTrackId || queueFor === spTrackId) return;
    queueFor = spTrackId;
    try {
      const q = await invoke<{ albums: { id: string; name: string; artist: string }[] }>('spotify_queue', { clientId });
      const next = q.albums[0];
      if (!next || prefetched.has(next.id) || next.id === spAlbum?.id) return;
      prefetched.add(next.id);
      if (prefetched.size > 50) prefetched.delete(prefetched.values().next().value as string);
      const a = await invoke<SpAlbum>('spotify_album', { clientId, id: next.id });
      // mismos nombres y mismos pedidos que cuando suene de verdad (con Spotify: sin lista ni portada), así el caché coincide
      await invoke('resolve_album', { artist: a.artist, album: a.title, upc: a.upc, force: false, wantTracks: false, needFront: !a.cover });
    } catch (e) { console.warn('precarga del siguiente álbum:', e); }
  }
  const spLoop = async () => {
    try { spSmtc = await invoke<boolean>('smtc_status'); } catch { spSmtc = false; }
    await spotifyTick().then(() => prefetchNext());
    clearTimeout(spTimer);
    spTimer = window.setTimeout(() => { void spLoop(); }, spDelay);
  };
  void spLoop();
  // Windows avisa que cambió la canción o el estado de reproducción de Spotify: se consulta ya (con un pequeño margen para agrupar avisos)
  let smtcKick = 0;
  await listen('smtc-changed', () => {
    clearTimeout(smtcKick);
    smtcKick = window.setTimeout(() => { spRetry = 3; clearTimeout(spTimer); void spLoop(); }, 400);
  });
  await listen('spotify-changed', () => { spAlbum = null; spRetry = 0; void spLoop(); });
  async function playTrack(i: number) {
    jc.setCurrentTrack(i);
    if (!spAlbum || !cfg.spotifyClientId) return;
    try { await invoke('spotify_play', { clientId: cfg.spotifyClientId, albumUri: spAlbum.uri, position: i }); spTrackId = spAlbum.tracks[i]?.id ?? null; jc.setPlaying(true); }
    catch (e) { spError(e); }
  }
  void loadArt();

  /* ---------- Arrastre de la ventana ----------
     startDragging (el del sistema) no deja sacar la ventana de la pantalla, y la caja tiene margen transparente alrededor:
     con ese arrastre el borde visible no llega al borde del monitor. Acá se posiciona la ventana a mano, sin límites. */
  async function dragWindow(e: PointerEvent, el: HTMLElement) {
    if (!win) return;
    const { PhysicalPosition } = await import('@tauri-apps/api/dpi');
    const start = await win.outerPosition();
    const sx = e.screenX, sy = e.screenY, dpr = devicePixelRatio;
    el.setPointerCapture(e.pointerId);
    let target: { x: number; y: number } | null = null, busy = false;
    const pump = async () => {
      if (busy) return; busy = true;
      while (target) { const t = target; target = null; await win.setPosition(new PhysicalPosition(t.x, t.y)); }
      busy = false;
    };
    const move = (ev: PointerEvent) => { target = { x: Math.round(start.x + (ev.screenX - sx) * dpr), y: Math.round(start.y + (ev.screenY - sy) * dpr) }; void pump(); };
    const up = () => { el.removeEventListener('pointermove', move); el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up); };
    el.addEventListener('pointermove', move); el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
  }

  /* ---------- Modo mover ----------
     Toda la ventana se arrastra (sin click-through). Se activa desde la bandeja o desde Ajustes; doble clic o el mismo botón lo termina.
     Alt + arrastrar sobre la caja también mueve la ventana. */
  let moveMode = false;
  const moveEl = document.getElementById('move')!;
  async function setMoveMode(on: boolean) {
    moveMode = on;
    document.body.classList.toggle('moving', on);
    if (win) { await win.setIgnoreCursorEvents(false); ignoring = false; }
  }
  moveEl.addEventListener('pointerdown', e => { if (e.button === 0) void dragWindow(e, moveEl); });
  moveEl.addEventListener('dblclick', () => void setMoveMode(false));
  await listen('toggle-move', () => void setMoveMode(!moveMode));

  /* ---------- Interacción ---------- */
  const win = inTauri ? (await import('@tauri-apps/api/window')).getCurrentWindow() : null;
  let ignoring = false;
  let down: { x: number; y: number; yaw: number; pitch: number; moved: boolean } | null = null;
  const wrap = (v: number) => ((v + 540) % 360) - 180;

  canvas.addEventListener('pointerdown', e => {
    if (e.altKey && win) { void dragWindow(e, canvas); return; }    // Alt + arrastrar: mover la ventana
    const o = jc.getOrient();
    down = { x: e.clientX, y: e.clientY, yaw: o.yaw, pitch: o.pitch, moved: false };
    jc.setInteracting(true);
    canvas.setPointerCapture(e.pointerId);
  });
  canvas.addEventListener('pointermove', e => {
    if (!down) return;
    const dx = e.clientX - down.x, dy = e.clientY - down.y;
    if (!down.moved && Math.hypot(dx, dy) > 4) { down.moved = true; canvas.classList.add('dragging'); }
    if (down.moved) jc.setOrient(Math.round(wrap(down.yaw + dx * 0.45)), Math.round(wrap(down.pitch + dy * 0.45)), jc.getOrient().roll);
  });
  const endDrag = () => { canvas.classList.remove('dragging'); down = null; jc.setInteracting(false); };
  canvas.addEventListener('pointerup', e => {
    if (!down) return;
    const wasClick = !down.moved; const moved = down.moved; endDrag();
    if (moved) { const o = jc.getOrient(); void patchConfig({ yaw: o.yaw, pitch: o.pitch }); return; }
    if (wasClick) {
      const r = canvas.getBoundingClientRect();
      const hit = jc.hitTest(e.clientX - r.left, e.clientY - r.top);
      if (hit?.kind === 'track') { void playTrack(hit.index!); return; }      // con Spotify conectado reproduce esa canción
      if (hit) jc.setOpen(!jc.isOpen());
    }
  });
  canvas.addEventListener('pointercancel', endDrag);

  /* ---------- Click-through ----------
     Con la ventana ignorando el mouse el DOM no recibe nada; Rust manda la posición global del cursor (~30 Hz),
     acá se hace el raycast y se activa o desactiva el click-through según haya o no caja debajo. */
  if (win) {
    await win.setIgnoreCursorEvents(true); ignoring = true;
    await listen<{ x: number; y: number }>('cursor', async p => {
      if (down || moveMode) return;                       // arrastrando o moviendo: no soltar el mouse a mitad de gesto
      const dpr = devicePixelRatio;
      const hit = jc.hitTest(p.x / dpr, p.y / dpr);
      const want = !hit;
      if (want !== ignoring) { ignoring = want; await win.setIgnoreCursorEvents(want); }
    });
  }

  // posición de la ventana → config (para recordarla)
  if (win) {
    let t: number | undefined;
    await win.onMoved(() => { clearTimeout(t); t = window.setTimeout(async () => { const p = await win.outerPosition(); void patchConfig({ x: p.x, y: p.y }); }, 400); });
  }
}

void main();
