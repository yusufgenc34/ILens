#!/usr/bin/env node
// Runs the Firefox e2e project in Docker. Firefox cannot launch on some macOS
// versions; the Linux container matches CI. Requires a running dev server on
// http://127.0.0.1:5173 (`npm run dev`) and Docker.
import {spawnSync} from 'node:child_process'
import {readFileSync} from 'node:fs'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const version = JSON.parse(readFileSync(path.join(root, 'node_modules/@playwright/test/package.json'), 'utf8')).version
const image = `ilens-e2e-firefox:${version}`
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, {stdio: 'inherit', cwd: root, ...options})
  if (result.status !== 0) process.exit(result.status ?? 1)
}
try {
  const response = await fetch('http://127.0.0.1:5173', {signal: AbortSignal.timeout(5000)})
  if (!response.ok) throw new Error(`status ${response.status}`)
} catch (error) {
  console.error(`Start the dev server first (npm run dev): ${error.message}`)
  process.exit(1)
}
if (spawnSync('docker', ['image', 'inspect', image], {stdio: 'ignore'}).status !== 0) {
  run('docker', ['build', '-f', 'docker/e2e.Dockerfile', '--build-arg', `PLAYWRIGHT_VERSION=${version}`, '-t', image, 'docker'])
}
run('docker', [
  'run', '--rm', '--ipc=host', '--add-host=host.docker.internal:host-gateway',
  '-v', `${root}:/work`, '-w', '/work',
  // NuGet packages (ILVerify, SDK packs) persist between runs.
  '-v', 'ilens-e2e-nuget:/root/.nuget',
  // The page talks to Vite (5173) and, in development, to rari's RSC server (3000)
  // on localhost; forward both to the host.
  image, 'bash', '-c', `for port in 5173 3000; do socat TCP-LISTEN:$port,fork,reuseaddr TCP:host.docker.internal:$port & done; dotnet tool restore >/dev/null && npx playwright test -c playwright.docker.config.ts ${process.argv.slice(2).map(a => `'${a.replaceAll("'", "'\\''")}'`).join(' ')}`,
])
