/* Puente con Tauri. Fuera de Tauri (vite en un navegador común) todo cae a valores por defecto, útil para mirar solo el render. */
import { DEFAULTS, type Config } from './config';

export const inTauri = '__TAURI_INTERNALS__' in window;

type Unlisten = () => void;

export async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const m = await import('@tauri-apps/api/core');
  return m.invoke<T>(cmd, args);
}

export async function listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  if (!inTauri) return () => {};
  const m = await import('@tauri-apps/api/event');
  return m.listen<T>(event, e => cb(e.payload));
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  if (!inTauri) return;
  const m = await import('@tauri-apps/api/event');
  await m.emit(event, payload);
}

export async function getConfig(): Promise<Config> {
  if (!inTauri) return { ...DEFAULTS };
  return { ...DEFAULTS, ...(await invoke<Partial<Config>>('get_config')) };
}

export async function patchConfig(patch: Partial<Config>): Promise<void> {
  if (!inTauri) return;
  await invoke('set_config', { patch });
}

export function onConfig(cb: (c: Config) => void): Promise<Unlisten> {
  return listen<Partial<Config>>('config-changed', c => cb({ ...DEFAULTS, ...c }));
}
