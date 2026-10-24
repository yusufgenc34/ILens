export interface FrameworkPackage {id:string; version:string; directory:string}
export interface Diagnostic { code: string; message: string; detail: string }
export interface Identity { name: string; version: string; culture: string; public_key_token: string }
export interface FrameworkInfo { display_name: string; moniker: string | null; version: string | null; source: string | null; status: 'declared' | 'not_declared' | 'invalid' | 'ambiguous' | 'incomplete' }
export interface ObfuscationInfo { status: 'markers_found' | 'possible' | 'no_markers' | 'inconclusive'; scan_complete: boolean; evidence: {kind: string; tool: string | null; confidence: 'marker' | 'heuristic'; detail: string; token: number | null}[] }
export interface AssemblyInfo { module_name?: string; ready_to_run?: boolean; reference_assembly?: boolean; identity: Identity; runtime: string; target_framework: FrameworkInfo; obfuscation: ObfuscationInfo; size: number; machine: string; type_count: number; method_count: number; diagnostics: string[] }
export interface Loaded { id: number; info: AssemblyInfo }
export interface TypeLink { token: number; name: string }
export interface Declaration { token: number; parent: number; kind: string; name: string; display_name?: string; accessor_of?: number | null; namespace: string; full_name: string; flags: number; base?: TypeLink; interfaces?: TypeLink[] }
export interface HeaderField { name: string; offset: number; size: number; value: string; info: string }
export interface HeaderGroup { id: string; title: string; offset: number; size: number; fields: HeaderField[] }
export interface SectionInfo { name: string; virtual_address: number; virtual_size: number; raw_offset: number; raw_size: number; characteristics: number; flags: string; entropy: number }
export interface Detection { name: string; kind: string; confidence: 'signature' | 'heuristic'; evidence: string[] }
export interface DetectionReport { managed: boolean; verdict: 'packed' | 'protected' | 'obfuscated' | 'suspicious' | 'no_findings'; detections: Detection[]; signals: {kind: string; detail: string}[] }
export interface PeHeaders { groups: HeaderGroup[]; sections: SectionInfo[]; overlay: [number, number] | null; entropy: number }
export interface SearchHit { token: number; kind: string; name: string; context: string }
export interface Reference { token: number; identity: Identity; resolved_id: number | null; status: 'resolved' | 'unresolved' }
export interface Instruction { offset: number; size: number; opcode: number; name: string; bytes: string; operand: { kind: string; value?: unknown }; resolved: string | null }
export interface MethodBody { code_size: number; max_stack: number; file_offset: number; local_signature: number; init_locals: boolean; instructions: Instruction[]; exceptions: {kind: string; try_start: number; try_end: number; handler_start: number; handler_end: number; catch_type: number | null; filter_start: number | null}[] }
export interface Block { id: number; start: number; end: number; successors: number[]; predecessors: number[]; exception_successors: number[] }
export interface SourceReference {start: number; end: number; token: number}
export interface MethodAnalysis { references: SourceReference[]; token: number; csharp: string; source_declaration?: string | null; source_body?: string | null; il: string; quality: 'csharp_like' | 'annotated_il' | 'declaration'; diagnostics: Diagnostic[]; body: MethodBody | null; cfg: { blocks: Block[] } | null; stack: { maximum: number; reachable_blocks: number } | null }
export interface AnalysisHit {assembly: number; token: number; name: string; offset: number | null; relation: string}
export interface AnalysisPage {hits: AnalysisHit[]; next: number; total: number; done: boolean; skipped: number; truncated: boolean}
export interface XrefPage { hits: {token: number; name: string; offset: number}[]; skipped: {token: number; error: Diagnostic}[]; next: number; total: number; done: boolean }
export interface TypeView { declaration: string; metadata: Record<string, unknown>; references?: SourceReference[]; partial?: boolean }
export interface Overview { info: AssemblyInfo; pe: Record<string, unknown>; streams: {name: string; offset: number; size: number}[]; tables: {name: string; id: number; rows: number}[]; references: Reference[]; resources: {token: number; name: string; embedded: boolean; offset: number; flags: number}[] }
export interface ResourcePreview {size: number; kind: string; text: string | null; hex: string; entries: {index:number; name:string; kind:string; size:number; value:string | null}[]; diagnostic: string | null}
export interface Operations {
  resolveFrameworkReference: {input:{assembly:number; token:number}; output:Loaded & {package: FrameworkPackage}}
  getResourcePreview: {input: {assembly:number; token:number; entry:number}; output:ResourcePreview}
  getResourceBytes: {input: {assembly:number; token:number; entry:number}; output:{buffer:ArrayBuffer}}

  beginProjectExport: {input: {options: import('./export-types').ProjectExportOptions}; output: import('./export-types').ExportProgress}
  beginExport: {input: {assembly: number; options: import('./export-types').ExportOptions}; output: import('./export-types').ExportProgress}
  stepExport: {input: {job: number}; output: import('./export-types').ExportProgress}
  finishExport: {input: {job: number}; output: import('./export-types').ExportResult}
  cancelExport: {input: {job: number}; output: null}
  openMethodEdit: {input: {assembly: number; token: number}; output: EditPreview}
  applyMethodEdit: {input: {assembly: number; token: number; revision: number; instructions: EditableInstruction[]; locals?: string[]; regions?: EditRegion[]}; output: EditPreview}
  discardMethodEdit: {input: {assembly: number; token: number; revision: number}; output: EditPreview}
  getEdits: {input: {assembly: number}; output: EditsInfo}
  getOpcodes: {input: Record<string, never>; output: [string, string][]}
  exportModifiedAssembly: {input: {assembly: number; revision: number}; output: {buffer: ArrayBuffer}}
  load: { input: {buffer: ArrayBuffer}; output: Loaded }
  loadDependency: {input: {buffer: ArrayBuffer; identity: Identity}; output: Loaded | null}
  analyzeReferences: {input: {assembly: number; token: number; source: number; cursor: number; outgoing: boolean}; output: AnalysisPage}
  getTree: { input: {assembly: number}; output: Declaration[] }
  getInfo: { input: {assembly: number}; output: Overview }
  getPeHeaders: { input: {assembly: number}; output: PeHeaders }
  detectProtection: { input: {assembly: number}; output: DetectionReport }
  detectFile: { input: {buffer: ArrayBuffer}; output: DetectionReport }
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
export interface EditableInstruction {id: string; opcode: string; operand: string}
/** An exception clause; labels name instruction ids, "" is the end of the method. */
export interface EditRegion {kind: 'catch' | 'finally' | 'fault'; try_start: string; try_end: string; handler_start: string; handler_end: string; catch_type: string | null}
export interface EditPreview {token: number; revision: number; instructions: EditableInstruction[]; original: EditableInstruction[]; changed: boolean; il: string; max_stack: number; code_size: number; locals: string[]; original_locals: string[]; regions: EditRegion[]; original_regions: EditRegion[]}
export interface EditsInfo {revision: number; edit_revision: number; methods: {token: number; name: string; code_size: number}[]; writer_error: Diagnostic | null}
export type Operation = keyof Operations
export type Request = { [K in Operation]: {id: number; op: K} & Operations[K]['input'] }[Operation]
export type Response = { id: number; kind: 'success'; result: unknown } | {id: number; kind: 'error'; error: Diagnostic} | {id: number; kind: 'fatal'; error: Diagnostic} | {id: number; kind: 'progress'; message: string}
export type WorkerMessage = Request | {id: number; op: 'cancel'; target: number}
export const tokenHex = (token: number) => `0x${token.toString(16).padStart(8, '0').toUpperCase()}`
export const jsonText = (value: unknown) => JSON.stringify(value, (_, v: unknown) => typeof v === 'bigint' ? v.toString() : v, 2)
export function diagnostic(error: unknown): Diagnostic {
  if (error && typeof error === 'object' && 'code' in error && 'message' in error && 'detail' in error) return {code:String(error.code),message:String(error.message),detail:String(error.detail)}
  return {code: 'worker_error', message: error instanceof Error ? error.message : 'The analysis could not be completed.', detail: String(error)}
}
