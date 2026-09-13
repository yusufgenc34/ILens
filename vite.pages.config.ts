import path from 'node:path'
import {defineConfig} from 'vite-plus'
import tailwindcss from '@tailwindcss/vite'

// Pages serves project sites beneath /<repository>/; custom domains use /.
const base = process.env.ILENS_PAGES_BASE ?? '/ILens/'
if (!base.startsWith('/') || !base.endsWith('/') || base.includes('..') || /[?#\\]/.test(base)) {
  throw new Error('ILENS_PAGES_BASE must be an absolute pathname ending in /.')
}
export default defineConfig({
  root: path.resolve(import.meta.dirname, 'pages'),
  publicDir: path.resolve(import.meta.dirname, 'public'),
  base,
  appType: 'mpa',
  plugins: [tailwindcss()],
  resolve: {alias: {'@': path.resolve(import.meta.dirname, 'src')}},
  worker: {format: 'es'},
  build: {outDir: path.resolve(import.meta.dirname, 'dist-pages'), emptyOutDir: true, sourcemap: false},
  preview: {host: '127.0.0.1', port: 4173, strictPort: true},
})
