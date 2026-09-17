import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from 'tailwindcss'
import autoprefixer from 'autoprefixer'
import pkg from './package.json'

// The bundled typefaces (@fontsource) list a WOFF fallback after every WOFF2
// source. Every WebView and browser the dashboard runs in reads WOFF2, so the
// fallback only adds files nobody loads (4.7 MB, measured 2026-09-17). Dropping
// the reference before Vite resolves url() keeps them out of dist.
function woff2Only(): Plugin {
  return {
    name: 'fontsource-woff2-only',
    enforce: 'pre',
    transform(code, id) {
      if (!id.includes('/@fontsource/') || !id.endsWith('.css')) return null
      return code.replace(/,\s*url\([^)]*\.woff\) format\('woff'\)/g, '')
    },
  }
}

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  plugins: [react(), woff2Only()],
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