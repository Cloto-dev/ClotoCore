import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from 'tailwindcss'
import autoprefixer from 'autoprefixer'
import pkg from './package.json'

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  plugins: [react()],
  css: {
    postcss: {
      plugins: [
        tailwindcss(),
        autoprefixer(),
      ],
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      output: {
        // Function form (required by Vite 8 / Rolldown; also valid on Rollup).
        // Groups three.js and all @pixiv/three-vrm* packages into one chunk.
        manualChunks(id) {
          if (id.includes('node_modules/three/') || id.includes('node_modules/@pixiv/three-vrm')) {
            return 'three-vrm'
          }
        },
      },
    },
  }
})