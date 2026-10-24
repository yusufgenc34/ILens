/// <reference lib="webworker" />
import {createExportJob} from './export-job'
import {createProjectExportJob} from './project-export-job'
import init, { Decompiler } from '../wasm/decompiler'
import wasmUrl from '../wasm/decompiler_bg.wasm?url'
import { diagnostic, type Request, type Response, type WorkerMessage } from '../lib/types'
import type {AssemblyInfo, Overview, Reference, Loaded} from '../lib/types'
import {createFrameworkResolver} from './framework-resolver'
const scope = self as unknown as DedicatedWorkerGlobalScope
let engine: Decompiler | undefined
let initialization: Promise<void> | undefined
let queue: Request[] = []
let processing = false
let exportJob: ReturnType<typeof createExportJob> | ReturnType<typeof createProjectExportJob> | undefined
let nextJob = 0
let exportRequest = 0
const resolveFramework = createFrameworkResolver()
const frameworkContexts = new Map<number,AssemblyInfo>()
const downloads = new Map<number,AbortController>()
function cancelExport() {exportJob?.cancel(); exportJob = undefined}
const cancelled = new Set<number>()
const send = (message: Response, transfer: Transferable[] = []) => scope.postMessage(message, transfer)
async function initialize() {
  initialization ??= init({module_or_path: wasmUrl}).then(() => { engine = new Decompiler() })
  await initialization
  if (!engine) throw new Error('WASM initialization did not produce an engine.')
  return engine
}
async function dispatch(core: Decompiler, request: Request): Promise<unknown> {
  if (['load', 'loadDependency', 'close', 'dispose', 'applyMethodEdit', 'discardMethodEdit'].includes(request.op)) cancelExport()
  switch (request.op) {
    case 'resolveFrameworkReference': {
      const info=core.get_assembly_info(request.assembly) as Overview
      const reference=(core.get_references(request.assembly) as Reference[]).find(r=>r.token===request.token)
      if (!reference) throw new Error('Choose an assembly reference to resolve.')
      const controller=new AbortController(); downloads.set(request.id,controller)
      const framework=frameworkContexts.get(request.assembly) ?? info.info
      try {return await resolveFramework(framework,reference.identity,controller.signal,(bytes,identity)=>{
        cancelExport()
        const loaded=core.load_reference_dependency(bytes,JSON.stringify(identity)) as Loaded | null
        if (loaded && !frameworkContexts.has(loaded.id)) frameworkContexts.set(loaded.id,framework)
        return loaded
      },message=>send({id:request.id,kind:'progress',message}))}
      finally {downloads.delete(request.id)}
    }
    case 'beginProjectExport': exportRequest = request.id; cancelExport(); exportJob = createProjectExportJob(core, ++nextJob, request.options); return exportJob.progress()
    case 'getEdits': return core.get_edits(request.assembly)
    case 'getOpcodes': return core.get_opcodes()
    case 'openMethodEdit': return core.open_method_edit(request.assembly, request.token)
    case 'applyMethodEdit': return core.apply_method_edit(request.assembly, request.token, request.revision, JSON.stringify(request.locals || request.regions ? {instructions: request.instructions, locals: request.locals, regions: request.regions} : request.instructions))
    case 'discardMethodEdit': return core.discard_method_edit(request.assembly, request.token, request.revision)
    case 'exportModifiedAssembly': return {buffer: core.export_modified_assembly(request.assembly, request.revision).buffer}
    case 'beginExport': exportRequest = request.id; cancelExport(); exportJob = createExportJob(core, ++nextJob, request.assembly, request.options); return exportJob.progress()
    case 'stepExport': exportRequest = request.id; if (!exportJob || exportJob.id !== request.job) throw {code: 'cancelled', message: 'Export cancelled.', detail: 'The export job was closed or superseded.'}; return exportJob.step()
    case 'finishExport': {
      if (!exportJob || exportJob.id !== request.job) throw {code: 'cancelled', message: 'Export cancelled.', detail: 'The export job was closed or superseded.'}
      try {return exportJob.finish()} finally {exportJob = undefined}
    }
    case 'cancelExport': if (exportJob?.id === request.job) cancelExport(); return null
    case 'load': if (request.buffer.byteLength > 64 * 1024 * 1024) throw {code: 'size_limit', message: 'Assemblies must be 64 MiB or smaller.', detail: 'Input size checked before copying into WASM.'}; return core.load_assembly(new Uint8Array(request.buffer))
    case 'loadDependency': if (request.buffer.byteLength > 64 * 1024 * 1024) throw {code: 'size_limit', message: 'Dependency exceeds 64 MiB.', detail: 'Rejected before copying into WASM.'}; return core.load_dependency(new Uint8Array(request.buffer), JSON.stringify(request.identity))
    case 'analyzeReferences': return core.analyze_references(request.assembly, request.token, request.source, request.cursor, request.outgoing)
    case 'getResourcePreview': return core.resource_preview(request.assembly, request.token, request.entry)
    case 'getResourceBytes': return {buffer:core.resource_bytes(request.assembly, request.token, request.entry).buffer}
    case 'getTree': return core.get_tree(request.assembly)
    case 'getInfo': return core.get_assembly_info(request.assembly)
    case 'getPeHeaders': return core.get_pe_headers(request.assembly)
    case 'detectProtection': return core.detect_protection(request.assembly)
    case 'detectFile': if (request.buffer.byteLength > 64 * 1024 * 1024) throw {code: 'size_limit', message: 'Files must be 64 MiB or smaller.', detail: 'Input size checked before copying into WASM.'}; return core.detect_file(new Uint8Array(request.buffer))
    case 'getType': return core.get_type(request.assembly, request.token)
    case 'getMetadata': return core.get_metadata(request.assembly, request.token)
    case 'decompileMethod': return core.decompile_method(request.assembly, request.token)
    case 'disassembleMethod': return core.disassemble_method(request.assembly, request.token)
    case 'search': return core.search(request.assembly, request.query)
    case 'getStrings': return core.get_strings(request.assembly, request.query)
    case 'getReferences': return core.get_references(request.assembly)
    case 'getXrefs': return core.get_xrefs(request.assembly, request.token, request.cursor)
    case 'close': core.close(request.assembly); frameworkContexts.delete(request.assembly); return null
    case 'dispose': core.dispose(); frameworkContexts.clear(); resolveFramework.clear(); return null
  }
}
async function drain() {
  if (processing) return
  processing = true
  try {
    while (queue.length) {
      // Let cancellation and newly queued requests reach the worker between WASM operations.
      await new Promise(resolve => setTimeout(resolve, 0))
      const request = queue.shift()
      if (!request) continue
      if (cancelled.delete(request.id)) continue
      try {
        // Background scans must not replace the status of the operation the user started.
        if (request.op !== 'detectProtection') send({id: request.id, kind: 'progress', message: request.op === 'load' ? 'Validating PE headers and CLR metadata…' : request.op === 'decompileMethod' ? 'Analyzing control flow and reconstructing C#…' : 'Reading local assembly data…'})
        const core = await initialize()
        if (cancelled.delete(request.id)) continue
        const result = await dispatch(core, request)
        send({id: request.id, kind: 'success', result}, result && typeof result === 'object' && 'buffer' in result && result.buffer instanceof ArrayBuffer ? [result.buffer] : [])
      } catch (error) {
        if (error instanceof WebAssembly.RuntimeError) {
          send({id: request.id, kind: 'fatal', error: {code: 'worker_error', message: 'The isolated WASM instance stopped. Reopen your assemblies to continue.', detail: error.message}})
          // Do not reenter WASM after a trap. Termination releases the entire linear memory.
          queue = []; engine = undefined; scope.close(); return
        }
        send({id: request.id, kind: 'error', error: diagnostic(error)})
      }
    }
  } finally { processing = false; cancelled.clear() }
}
scope.onmessage = ({data}: MessageEvent<WorkerMessage>) => {
  if (data.op === 'cancel') { downloads.get(data.target)?.abort(); if (exportRequest === data.target) cancelExport(); queue = queue.filter(r => r.id !== data.target); cancelled.add(data.target); send({id: data.target, kind: 'error', error: {code: 'cancelled', message: 'Operation cancelled.', detail: 'Worker acknowledged cancellation.'}}); return }
  if (queue.length >= 128) { send({id: data.id, kind: 'error', error: {code: 'size_limit', message: 'Too many pending operations.', detail: 'Worker queue is limited to 128 requests.'}}); return }
  queue.push(data); void drain()
}
