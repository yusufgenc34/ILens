import { spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
const version = '0.2.105'
function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit', shell: false })
  if (result.error || result.status !== 0) {
    throw result.error ?? new Error(`${command} exited ${result.status}`)
  }
}
const installed = spawnSync('wasm-bindgen', ['--version'], { encoding: 'utf8' })
if (installed.status !== 0 || !installed.stdout.includes(version)) {
  console.log(`Installing wasm-bindgen-cli ${version} to match Cargo.lock…`)
  run('cargo', ['install', 'wasm-bindgen-cli', '--version', version, '--locked'])
}
run('rustup', ['target', 'add', 'wasm32-unknown-unknown'])
run('cargo', ['build', '--locked', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'decompiler-wasm'])
mkdirSync('src/wasm', { recursive: true })
run('wasm-bindgen', ['--target', 'web', '--out-dir', 'src/wasm', '--out-name', 'decompiler', 'target/wasm32-unknown-unknown/release/decompiler_wasm.wasm'])

// Keep the generated loader outside Rari's server-component registry.
const binding = 'src/wasm/decompiler.js'
writeFileSync(binding, "'use client';\n" + readFileSync(binding, 'utf8'))
