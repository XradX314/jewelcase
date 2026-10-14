import '@fontsource/chivo/400.css';
import '@fontsource/chivo/500.css';
import '@fontsource/chivo/700.css';
import '@fontsource/chivo-mono/400.css';
import { getConfig, onConfig, patchConfig, emit, listen, invoke, inTauri } from '../shared/bridge';
import { DEFAULTS, type Config } from '../shared/config';

type Ctl = HTMLInputElement | HTMLSelectElement;
const ctls = [...document.querySelectorAll<Ctl>('[data-k]')];
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const PRESETS: Record<string, Partial<Config>> = {
  new: { count: 0, wearI: 0, wearLen: 22, swirl: 30, dust: 0 },
  light: { count: 40, wearI: 55, wearLen: 22, swirl: 30, dust: 10 },
  used: { count: 140, wearI: 75, wearLen: 30, swirl: 45, dust: 25 },
  glovebox: { count: 360, wearI: 95, wearLen: 42, swirl: 55, dust: 55 },
};

function label(c: Ctl, v: unknown) {
  const out = document.getElementById(c.id + 'O');
  if (out) out.textContent = String(v) + (c.dataset.fmt ?? '');
}

function show(c: Ctl, v: unknown) {
  if (c instanceof HTMLInputElement && c.type === 'checkbox') c.checked = !!v;
  else if (document.activeElement !== c) c.value = String(v ?? '');
  label(c, v);
}
function render(cfg: Config) { for (const c of ctls) show(c, (cfg as Record<string, unknown>)[c.dataset.k!]); }

function valueOf(c: Ctl): unknown {
  if (c instanceof HTMLInputElement && c.type === 'checkbox') return c.checked;
  if (c instanceof HTMLInputElement && (c.type === 'range' || c.type === 'number')) return Number(c.value);
  if (c.dataset.num) return Number(c.value);
  return c.value;
}

let timer: number | undefined;
let pending: Record<string, unknown> = {};
function queue(patch: Record<string, unknown>) {            // junta los cambios de un slider en un solo guardado
  Object.assign(pending, patch);
  clearTimeout(timer);
  timer = window.setTimeout(() => { const p = pending; pending = {}; void patchConfig(p as Partial<Config>); }, 60);
}

for (const c of ctls) {
  const isToggle = c instanceof HTMLSelectElement || (c as HTMLInputElement).type === 'checkbox';
  const ev = c.dataset.commit ?? (isToggle ? 'change' : 'input');
  c.addEventListener(ev, () => {
    const k = c.dataset.k!;
    const patch: Record<string, unknown> = { [k]: valueOf(c) };
    if (c.dataset.wear) { patch.wearPreset = 'custom'; $<HTMLSelectElement>('wearPreset').value = 'custom'; }
    if (k === 'wearPreset' && PRESETS[c.value]) Object.assign(patch, PRESETS[c.value]);
    queue(patch);
    label(c, valueOf(c));
  });
}

$('dice').addEventListener('click', () => {
  const s = 1 + Math.floor(Math.random() * 99999);
  $<HTMLInputElement>('seed').value = String(s);
  queue({ seed: s, wearPreset: 'custom' });
});
$('resetBtn').addEventListener('click', () => queue({ yaw: DEFAULTS.yaw, pitch: DEFAULTS.pitch, roll: 0 }));
$('moveBtn').addEventListener('click', () => void emit('toggle-move'));
$('spinBtn').addEventListener('click', () => void emit('spin-box'));
await listen<boolean>('spin-state', on => { $('spinBtn').textContent = on ? 'Detener giro' : 'Girar'; });
$('lidBtn').addEventListener('click', () => void emit('toggle-lid'));
$('reloadArt').addEventListener('click', () => void emit('reload-art'));

/* ---------- Qué disco se está usando ---------- */
interface ArtInfo { px?: number | null; upgraded?: boolean; qartist?: string; qalbum?: string; searching?: boolean; found: boolean; artist: string; title: string; disc: boolean; source: string | null; url: string | null; release: string | null; report?: string[]; attempt?: number; incomplete?: boolean; retrying?: boolean; failed?: boolean }
let lastArt: { artist: string; album: string } | null = null;
await listen<ArtInfo>('art-info', i => {
  if (i.qartist && i.qalbum) lastArt = { artist: i.qartist, album: i.qalbum };
  const head = `${i.artist} — ${i.title}`;
  if (i.searching) { $('artInfo').textContent = `${head}\nBuscando el disco…`; return; }
  const tries = i.attempt && i.attempt > 1 ? ` (intento ${i.attempt})` : '';
  const net = i.retrying ? '\nFalló alguna consulta por red: reintentando…' : i.incomplete ? '\nAlguna consulta falló por red y se agotaron los reintentos (se vuelve a intentar al recargar el arte).' : '';
  const rep = i.report?.length ? `\n\nFuentes consultadas:\n• ${i.report.join('\n• ')}` : '';
  if (!i.found) $('artInfo').textContent = `${head}${tries}\nNo se encontró el álbum en MusicBrainz${i.failed ? ' (error de red)' : ''}: se usa la serigrafía generada.${net}`;
  else if (!i.disc) $('artInfo').textContent = `${head}${tries}\nCD: serigrafía generada. Ninguna fuente tiene una foto de este disco.${net}${rep}`;
  else if (i.source === 'Foto manual') $('artInfo').textContent = `${head}${tries}\nCD: la foto manual que elegiste${i.url ? `\n${i.url}` : ''}${rep}`;
  else $('artInfo').textContent = `${head}${tries}\nCD: foto real de ${i.source ?? 'un proveedor'}${i.px && i.px < 100000 ? ` (${i.px} px${i.upgraded ? ', mejorada' : ''})` : ''}${i.release ? `\nEdición de MusicBrainz: ${i.release}` : ''}${i.url ? `\n${i.url}` : ''}${rep}`;
});
void emit('art-info-request');
$('discUrlSet').addEventListener('click', async () => {
  const url = $<HTMLInputElement>('discUrl').value.trim();
  if (!lastArt) { $('artInfo').textContent = 'Todavía no hay un álbum cargado: poné música en Spotify o escribí artista y álbum.'; return; }
  if (!url) { $('artInfo').textContent = 'Pegá la URL de la foto del CD.'; return; }
  $('artInfo').textContent = 'Bajando la foto y ubicando el disco…';
  try { await invoke('set_disc_override', { artist: lastArt.artist, album: lastArt.album, url }); void emit('reload-art'); }
  catch (e) { $('artInfo').textContent = String(e); }
});
$('discUrlClear').addEventListener('click', async () => {
  if (!lastArt) return;
  await invoke('clear_disc_override', { artist: lastArt.artist, album: lastArt.album });
  $<HTMLInputElement>('discUrl').value = '';
  void emit('reload-art');
});

/* ---------- Spotify ---------- */
const spStatus = $('spStatus');
async function refreshSpotify(msg?: string) {
  if (!inTauri) return;
  const connected = await invoke<boolean>('spotify_status');
  spStatus.textContent = msg ?? (connected ? 'Conectado a Spotify' : 'Sin conectar');
}
$('spLogin').addEventListener('click', async () => {
  const id = $<HTMLInputElement>('spotifyClientId').value.trim();
  if (!id) { spStatus.textContent = 'Pegá primero el Client ID de tu app de Spotify'; return; }
  await patchConfig({ spotifyClientId: id });
  spStatus.textContent = 'Esperando la autorización en el navegador…';
  try { await invoke('spotify_login', { clientId: id }); await refreshSpotify(); }
  catch (e) { await refreshSpotify(String(e)); }
});
$('spLogout').addEventListener('click', async () => { await invoke('spotify_logout'); await refreshSpotify(); });
await listen<string>('spotify-error', e => { spStatus.textContent = e; });
await refreshSpotify();

render(await getConfig());
await onConfig(render);
