import {test, expect} from '@playwright/test'
import {readFileSync} from 'node:fs'
test('WASM parser survives truncations, produces real CFGs, and explicitly releases memory', async ({page}) => {
 await page.goto('/')
 const bytes = [...readFileSync('samples/fixtures/ILens.Patterns.dll')]
 const result = await page.evaluate(async input => {
  const {default: init, Decompiler} = await import('/src/wasm/decompiler.js')
  const wasm = await init({module_or_path: '/src/wasm/decompiler_bg.wasm'})
  const engine = new Decompiler()
  const errors: unknown[] = []
  for (const length of [0, 1, 60, 256, 700, 1700, input.length - 1]) {
   try {engine.load_assembly(new Uint8Array(input.slice(0, length))); errors.push(null)} catch (e) {errors.push(e)}
  }
  const assembly = engine.load_assembly(new Uint8Array(input))
  const tree = engine.get_tree(assembly.id) as {token: number; name: string; full_name: string}[]
  const method = tree.find(n => n.name === 'Sum')!
  const analysis = engine.decompile_method(assembly.id, method.token)
  engine.close(assembly.id)
  let closed = false
  try {engine.get_tree(assembly.id)} catch {closed = true}
  engine.dispose(); engine.free()
  return {errors, analysis, closed, memoryBytes: wasm.memory.buffer.byteLength}
 }, bytes)
 expect(result.errors.every(Boolean)).toBe(true)
 expect(result.analysis.quality).toBe('csharp_like')
 expect(result.analysis.cfg.blocks.length).toBe(4)
 expect(result.closed).toBe(true)
 expect(result.memoryBytes).toBeLessThan(536870913)
})
