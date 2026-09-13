# Architecture


```text
Rari server: HTML/RSC application shell and static assets only
                          │
React client: File API → transferable ArrayBuffer
                          │ typed request IDs, progress/errors, cancellation
Dedicated module worker: decompiler.worker.ts
                          │ wasm-bindgen, coarse serialized operations
Rust Session → Assembly → validated PE/CLI metadata
                          ↓
CIL → basic blocks/CFG → evaluation stack analysis → typed IR
                          ↓
                 IR simplification → AST → C# emitter
                          └────────────→ independent CIL emitter
```

`crates/decompiler-core` has no browser or framework dependency. It owns byte buffers and exposes independent parsing, signature, instruction, graph, analysis, IR, AST, transformation, rendering and session modules. `crates/decompiler-wasm` owns a session and adapts it to JavaScript. Native examples use filesystem APIs for developer diagnostics; **the browser parsing path only accepts buffers**.

The core uses pinned **goblin 0.10.7** for PE headers/sections and **clrmeta 0.1.0** for table/heap records. Our preflight validates metadata before entering clrmeta, and our bounded signature parser and CIL decoder avoid its unchecked/unbounded paths. See [the parser evaluation](parser-evaluation.md), including actual browser-WASM compile probes for clrmeta, dnfile and goblin and the reasons dotscope/windows-metadata were not selected.

The referenced historical [mono-wasm](https://github.com/migueldeicaza/mono-wasm) repository was studied before implementation. Its explicit browser bootstrap, in-memory assembly loading, memory-view lifetime and boundary ownership are relevant. Its Mono execution runtime, syscall emulation and historical LLVM build are not needed. **No Mono runtime or mono-wasm code is included.** See [the mono-wasm study](mono-wasm-notes.md) for source-specific findings and the inspected commit.

### Worker API and lifetime

`src/lib/types.ts` defines the RPC contract; `src/lib/rpc.ts` owns IDs, pending promises, transferable buffers, cancellation and the watchdog. The WASM `Decompiler` exposes:

```text
load_assembly(bytes)              get_assembly_info(id)
get_tree(id)                      get_type(id, token)
get_method(id, token)             get_metadata(id, token)
disassemble_method(id, token)     decompile_method(id, token)
get_strings(id, query)            get_references(id)
get_xrefs(id, token, cursor)      search(id, query)
close(id)                        dispose()
```

Initialization happens once per worker. Initial load indexes metadata, without decoding/decompiling every method. A method token requests only its body. A 32-entry/32-MiB-accounted LRU retains recent method analyses. Dependency definitions resolve against the session's loaded assembly identities. Large integer CIL operands serialize as decimal strings to avoid JavaScript number precision loss.

Cancellation removes queued requests and discards stale responses. A synchronous WASM operation cannot process a cancellation message midway through its execution. **Stop analysis** or the 30-second watchdog terminates the worker, clears the workspace, and releases all its memory. Closing the workspace does the same; `close(id)` and `dispose()` are also available to API clients. Rust deallocation makes memory reusable, but WASM linear memory does not shrink until the worker terminates.

### Export and edit operations

Source export adds a Rust declaration/member model and AST-derived method bodies, with a worker-side compilation-unit/archive emitter. It does not read the truncated inspection preview. Jobs process one unit at a time against a workspace revision. The Rust encoder, strict write verifier and PE writer are separate from inspection analysis. Method edits live in immutable-base overlays, and all workspace mutations invalidate cached analysis conservatively.

The typed RPC contract also exposes `beginProjectExport`, `beginExport`, `stepExport`, `finishExport`, `cancelExport`, `openMethodEdit`, `applyMethodEdit`, `discardMethodEdit`, `getEdits`, `getOpcodes` and `exportModifiedAssembly`. The client queues requests before submission; each submitted operation receives its own execution deadline. See [source export and editing](export-edit.md) for ownership, validation, compatibility and output limits.

Project export coordinates per-assembly jobs, generates namespace/type files and a solution, and resolves selected dependencies to project references. Reports and unsupported IL are isolated under `_ilens`. Source saving and binary saving remain distinct operations. The UI uses Tailwind utilities; the [Rari style adapter](css-loading.md) registers Vite’s compiled CSS with the root route.

### Source readability and evaluation order

Stack lifting deliberately creates a temporary for every pushed value. `transform` removes single-use constant/copy temporaries inside a block after checking local writes and address escapes. An effectful expression moves only into the immediately following use, and only when that use precedes other calls, heap reads and potentially throwing expressions. Conditional edge values do not make an eagerly evaluated call conditional. Duplicated effectful values retain a shared temporary.

The typed IR includes an array-initializer expression. Up to 128 consecutive stores can be folded when they fill a fresh vector in ascending index order and the array has exactly one following use in the same block. No alias, address escape, other write or catch-handler read may observe the partly initialized array. Allocation and element evaluation retain their order, following [C# array-creation semantics](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/language-specification/expressions#128175-array-creation-expressions). Unknown array receivers retain effectful RHS temporaries unless a fresh allocation and an in-range constant index are proven. Arrays that fail these checks remain explicit assignments.

Simple exception regions omit unreferenced labels and jumps that match C# lexical fallthrough, including continuation after sibling handlers. Branch targets needed by other exits remain visible. Local declarations, shared stack slots and nontrivial control flow can still require synthetic names or gotos.

Property syntax currently requires `MethodSemantics` in the analyzed assembly. External accessor references can remain `get_Text()`/`set_Text(...)`; loading a dependency enables navigation but does not yet supply its accessor semantics to the C# emitter. A method merely named `get_...` is never assumed to be a property. PDB names, VB-specific idioms, closure/state-machine reconstruction and broader local lifetime analysis remain separate work.

`samples/Readability` covers repeated property reads, a twelve-element message array, accessor-looking ordinary methods and an array observed by a catch handler. Rust regressions also cover mutation, address escapes, call ordering, allocation ordering and conditional edges. Browser tests inspect actual worker/WASM output and compile a saved, repository-owned reconstructed method for an independent C# syntax check; no analyzed assembly is executed.
