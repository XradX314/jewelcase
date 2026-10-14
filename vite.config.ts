import { defineConfig } from 'vite';
import { resolve } from 'node:path';

// Dos páginas: el widget (index.html) y la ventana de ajustes (settings.html).
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: '127.0.0.1', watch: { ignored: ['**/src-tauri/**'] } },
  build: {
    target: 'es2022',
    rollupOptions: {
      input: { main: resolve(__dirname, 'index.html'), settings: resolve(__dirname, 'settings.html') },
    },
  },
});
