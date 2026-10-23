import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { themePlugin } from './scripts/theme.mjs';

export default defineConfig({
  plugins: [themePlugin(), react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, fs: { allow: ['..'] } },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: { target: 'es2022' },
});
