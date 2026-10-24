# Design note: editing methods in C#

Status: proposal, awaiting approval. Nothing here is implemented yet.

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

## Open questions for approval

- Is a one-time download of about 30 MB acceptable for C# editing?
- Should the first version refuse lambdas, iterators and async methods (they need new
  types), or wait until the writer can add TypeDefs?
