'use client'
import type {CompileRequest, CompileResult} from '../workers/compiler.worker'
export type {CompileResult}
// One compiler worker per page: the .NET runtime and Roslyn load once, on first use.
let worker: Worker | null = null
let next = 0
const pending = new Map<number, {resolve: (value: {result: CompileResult; buffer: ArrayBuffer | null}) => void; reject: (error: Error) => void}>()
function start(): Worker {
  if (worker) return worker
  worker = new Worker(new URL('../workers/compiler.worker.ts', import.meta.url), {type: 'module', name: 'ilens-csharp-compiler'})
  worker.onmessage = (event: MessageEvent<{id: number; result?: CompileResult; buffer?: ArrayBuffer | null; error?: string}>) => {
    const request = pending.get(event.data.id)
    if (!request) return
    pending.delete(event.data.id)
    if (event.data.error !== undefined) request.reject(new Error(event.data.error))
    else request.resolve({result: event.data.result!, buffer: event.data.buffer ?? null})
  }
  worker.onerror = event => {
    for (const request of pending.values()) request.reject(new Error(event.message || 'The C# compiler failed to load. Build it with npm run build:compiler.'))
    pending.clear(); worker?.terminate(); worker = null
  }
  return worker
}
/** Compiles C# against reference assemblies (metadata only) in the compiler worker. */
export function compileCSharp(source: string, references: {name: string; buffer: ArrayBuffer}[]): Promise<{result: CompileResult; buffer: ArrayBuffer | null}> {
  const id = ++next
  return new Promise((resolve, reject) => {
    pending.set(id, {resolve, reject})
    start().postMessage({id, source, references} satisfies CompileRequest, references.map(r => r.buffer))
  })
}
