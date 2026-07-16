import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import path from 'path';
import pkg from './package.json';

export default defineConfig({
  plugins: [react()],
  // Mirror vite.config.ts's define so tests that import the app's real module
  // graph (e.g. anything reaching src/lib/tauri.ts, which reads __APP_VERSION__
  // at module load) resolve the same compile-time constants as a real build.
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      exclude: [
        'node_modules/',
        'src/test/',
        '**/*.d.ts',
        '**/*.config.*',
        '**/mockData',
      ],
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
});
