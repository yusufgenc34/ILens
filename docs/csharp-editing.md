# Editing methods in C#

Status: implemented for single method bodies (option A below). Right-click a method and choose
**Edit Method (C#)…**. The compiler component is built with `npm run build:compiler` (requires the
.NET 10 SDK) into `public/compiler`; builds without it show how to add it.

How it works now:

- `csharp_edit::source` reconstructs the method and wraps it in a skeleton of its declaring
  type: other members become `=> throw null`, constructors `extern`, so source types win over
  the referenced original (CS0436 is only a warning).
- `src/workers/compiler.worker.ts` runs Roslyn (trimmed, about 19 MB, 5 MB compressed) on .NET
  WebAssembly. References are the loaded assemblies' bytes, read as metadata. Like dnSpy it sets
  Roslyn's internal `IgnoreAccessibility` binder flag so private and internal members are usable,
  and it disables concurrent builds because browser WebAssembly is single-threaded.
- `csharp_edit::merge` maps the compiled body's tokens into the edited assembly, reusing existing
  rows by identity and appending new AssemblyRef/TypeRef/MemberRef/TypeSpec/MethodSpec rows;
  `pe_write::add_rows` writes them and re-verifies. The body then goes through the IL write
  verifier like any IL edit, and Save Module downloads the result.
- Refused with an explanation: signature changes, records, and bodies needing compiler-generated
  types (lambdas, local functions, iterators, async), until the writer can add TypeDefs.

The rest of this note is the original proposal.

dnSpy's **Edit Method (C#)** decompiles a method, lets the user change the C#, compiles it
with Roslyn against the module and its references, and merges the compiled IL back into
the module. This note describes how ILens could offer the same in the browser without
breaking its rules: analyzed assemblies are never executed or uploaded, and anything that
cannot be verified is refused rather than written.

## What has to happen

1. **Source.** The editor starts from the reconstructed C# of one method (or one type for
   adding members), wrapped in a compilable stub: the declaring type with its other members
   as `extern`/`throw null` signatures, so the edited body sees the same fields, methods and
   base types.
2. **Compile.** A C# compiler turns the stub into IL. References are the edited assembly
   itself and the loaded dependencies, read as metadata only.
3. **Merge.** The compiled method body replaces the original. Its tokens point into the
   compiled stub's metadata, so each one is mapped into the edited module: existing
   definitions by identity, new strings into #US, new TypeRefs/MemberRefs/TypeSpecs/
   MethodSpecs appended to their free-standing tables (the writer already supports this),
   new locals as a StandAloneSig.
4. **Verify and write.** The merged body goes through the same write verifier as IL edits;
   unsupported constructs are rejected with the verifier's diagnostics. Compiler-generated
   helper types (lambdas' closure classes, state machines, `<PrivateImplementationDetails>`)
   need new TypeDefs and are out of scope for the first version: a body that needs them is
   refused with an explanation.

## Options for the compiler

| Option | Size / effort | Coverage | Notes |
| --- | --- | --- | --- |
| A. Roslyn on .NET WebAssembly (browser-wasm), loaded on first use in its own worker | ~25–35 MB download, cached; moderate integration effort | Full C# | Executes only the compiler, never the analyzed code. References are passed as byte arrays (metadata). |
| B. A C# subset compiler in Rust (expressions, statements, calls, fields, locals) | No download; large effort | Partial C#; grows over time | Reuses the verifier's type model; every unsupported construct is a clear error. |
| C. Server-side compilation | Small client | Full C# | Rejected: sends assembly contents to a server. |

## Recommendation

Option A for coverage, behind an explicit, lazily downloaded "C# editing" component
(similar to how dnSpy depends on Roslyn), with Option B's verifier-first merge step shared
by both. The download is opt-in, cached by the browser, and pinned by hash. The first
version supports editing one method body at a time; adding members and editing whole
types follow once TypeDef insertion (needed for closure classes) exists in the writer.

## Decisions

- The compiler is downloaded once, on first use, from the same origin.
- The first version refuses lambdas, iterators and async methods (they need new types).
