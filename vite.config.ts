import path from 'node:path'
import { rari } from 'rari/vite'
import { defineConfig } from 'vite-plus'

const codespaceHost = process.env.CODESPACE_NAME && process.env.GITHUB_CODESPACES_PORT_FORWARDING_DOMAIN
  ? `${process.env.CODESPACE_NAME}-5173.${process.env.GITHUB_CODESPACES_PORT_FORWARDING_DOMAIN}`
  : undefined

export default defineConfig({
  plugins: [rari({
    csp: {scriptSrc: ["'self'", "'unsafe-inline'", "'wasm-unsafe-eval'"], workerSrc: ["'self'"]},
    cacheControl: {routes: {'/': 'no-cache'}},
  })],
  // Rari discovers client components dynamically; prebundle their dependencies
  // before the first browser request to avoid a dependency-discovery reload.
  optimizeDeps: {include: ['react', 'react-dom', 'react-dom/client', 'lucide-react', '@codemirror/commands', '@codemirror/language', '@codemirror/legacy-modes/mode/clike', '@codemirror/search', '@codemirror/state', '@codemirror/view', '@lezer/highlight']},
  resolve: { alias: { '@': path.resolve(import.meta.dirname, 'src') } },
  worker: { format: 'es' },
  server: { host: process.env.ILENS_DEV_HOST ?? '127.0.0.1', port: 5173, strictPort: true, allowedHosts: codespaceHost ? [codespaceHost] : undefined, watch: { ignored: ['**/tests/**', '**/*config.ts', '**/target/**', '**/crates/**', '**/samples/**', '**/*.d.ts', '**/src/workers/**'] } },
})
