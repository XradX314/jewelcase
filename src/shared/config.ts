import defaults from '../../src-tauri/default-config.json';

export type Config = Omit<typeof defaults, 'x' | 'y'> & { x: number | null; y: number | null };
export const DEFAULTS: Config = defaults;
