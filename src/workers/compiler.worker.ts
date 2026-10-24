// The C# editing compiler: Roslyn on .NET WebAssembly, downloaded on first use. It
// compiles source against reference assemblies given as bytes; those are read as
// metadata and never run.
interface CompilerExports {
  Reset(): void
  AddReference(image: Uint8Array, name: string): void
  Compile(source: string, assemblyName: string): string
  Output(): Uint8Array
}
export interface CompileRequest {id: number; source: string; references: {name: string; buffer: ArrayBuffer}[]}
export interface CompileResult {Success: boolean; Diagnostics: {Id: string; Message: string; Line: number; Column: number}[]}
const scope = self as unknown as {postMessage: (message: unknown, transfer?: Transferable[]) => void; addEventListener: (type: 'message', listener: (event: MessageEvent<CompileRequest>) => void) => void}
let compiler: Promise<CompilerExports> | null = null
function load(): Promise<CompilerExports> {
  compiler ??= (async () => {
    const url = new URL(`${import.meta.env.BASE_URL}compiler/_framework/dotnet.js`, self.location.origin).href
    const {dotnet} = await import(/* @vite-ignore */ url).catch(() => {throw new Error('The C# compiler component is not available in this build. Build it with npm run build:compiler.')})
    const runtime = await dotnet.create()
    const exports = await runtime.getAssemblyExports('ILens.Compiler.dll')
    return exports.Compiler as CompilerExports
  })()
  compiler.catch(() => {compiler = null})
  return compiler
}
// A listener rather than onmessage: with onmessage set, dotnet.create() never resolves
// in a worker (the runtime treats the worker as one of its own).
scope.addEventListener('message', async event => {
  const {id, source, references} = event.data
  try {
    const exports = await load()
    exports.Reset()
    for (const reference of references) exports.AddReference(new Uint8Array(reference.buffer), reference.name)
    const result = JSON.parse(exports.Compile(source, 'ILens.CSharpEdit')) as CompileResult
    const buffer = result.Success ? exports.Output().slice().buffer : null
    scope.postMessage({id, result, buffer}, buffer ? [buffer] : [])
  } catch (error) {
    scope.postMessage({id, error: error instanceof Error ? error.message : String(error)})
  }
})
