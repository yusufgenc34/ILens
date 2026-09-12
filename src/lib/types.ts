export interface Diagnostic { code: string; message: string; detail: string }
export interface Identity { name: string; version: string; culture: string; public_key_token: string }
export interface FrameworkInfo { display_name: string; moniker: string | null; version: string | null; source: string | null; status: 'declared' | 'not_declared' | 'invalid' | 'ambiguous' | 'incomplete' }
export interface ObfuscationInfo { status: 'markers_found' | 'possible' | 'no_markers' | 'inconclusive'; scan_complete: boolean; evidence: {kind: string; tool: string | null; confidence: 'marker' | 'heuristic'; detail: string; token: number | null}[] }
export interface AssemblyInfo { identity: Identity; runtime: string; target_framework: FrameworkInfo; obfuscation: ObfuscationInfo; size: number; machine: string; type_count: number; method_count: number; diagnostics: string[] }
export interface Loaded { id: number; info: AssemblyInfo }
export interface Declaration { token: number; parent: number; kind: string; name: string; namespace: string; full_name: string; flags: number }
export interface SearchHit { token: number; kind: string; name: string; context: string }
export interface Reference { token: number; identity: Identity; resolved_id: number | null; status: 'resolved' | 'unresolved' }
export interface Instruction { offset: number; size: number; opcode: number; name: string; bytes: string; operand: { kind: string; value?: unknown }; resolved: string | null }
export interface MethodBody { code_size: number; max_stack: number; file_offset: number; local_signature: number; init_locals: boolean; instructions: Instruction[]; exceptions: {kind: string; try_start: number; try_end: number; handler_start: number; handler_end: number; catch_type: number | null; filter_start: number | null}[] }
export interface Block { id: number; start: number; end: number; successors: number[]; predecessors: number[]; exception_successors: number[] }
export interface MethodAnalysis { token: number; csharp: string; il: string; quality: 'csharp_like' | 'annotated_il' | 'declaration'; diagnostics: Diagnostic[]; body: MethodBody | null; cfg: { blocks: Block[] } | null; stack: { maximum: number; reachable_blocks: number } | null }
export interface XrefPage { hits: {token: number; name: string; offset: number}[]; skipped: {token: number; error: Diagnostic}[]; next: number; total: number; done: boolean }
export interface TypeView { declaration: string; metadata: Record<string, unknown> }
export interface Overview { info: AssemblyInfo; pe: Record<string, unknown>; streams: {name: string; offset: number; size: number}[]; tables: {name: string; id: number; rows: number}[]; references: Reference[]; resources: {token: number; name: string; embedded: boolean; offset: number; flags: number}[] }
export interface Operations {
  load: { input: {buffer: ArrayBuffer}; output: Loaded }
  getTree: { input: {assembly: number}; output: Declaration[] }
  getInfo: { input: {assembly: number}; output: Overview }
  getType: { input: {assembly: number; token: number}; output: TypeView }
  getMetadata: { input: {assembly: number; token: number}; output: Record<string, unknown> }
  decompileMethod: { input: {assembly: number; token: number}; output: MethodAnalysis }
  disassembleMethod: { input: {assembly: number; token: number}; output: {il: string; body: MethodBody | null} }
  search: { input: {assembly: number; query: string}; output: SearchHit[] }
  getStrings: { input: {assembly: number; query: string}; output: SearchHit[] }
  getReferences: { input: {assembly: number}; output: Reference[] }
  getXrefs: { input: {assembly: number; token: number; cursor: number}; output: XrefPage }
  close: { input: {assembly: number}; output: null }
  dispose: { input: Record<string, never>; output: null }
}
export type Operation = keyof Operations
export type Request = { [K in Operation]: {id: number; op: K} & Operations[K]['input'] }[Operation]
export type Response = { id: number; kind: 'success'; result: unknown } | {id: number; kind: 'error'; error: Diagnostic} | {id: number; kind: 'fatal'; error: Diagnostic} | {id: number; kind: 'progress'; message: string}
export type WorkerMessage = Request | {id: number; op: 'cancel'; target: number}
export const tokenHex = (token: number) => `0x${token.toString(16).padStart(8, '0').toUpperCase()}`
export const jsonText = (value: unknown) => JSON.stringify(value, (_, v: unknown) => typeof v === 'bigint' ? v.toString() : v, 2)
export function diagnostic(error: unknown): Diagnostic {
  if (error && typeof error === 'object' && 'code' in error && 'message' in error && 'detail' in error) return error as Diagnostic
  return {code: 'worker_error', message: error instanceof Error ? error.message : 'The analysis could not be completed.', detail: String(error)}
}
