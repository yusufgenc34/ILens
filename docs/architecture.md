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

### Static hosting target

`pages/main.tsx` is a client-only entry for the same `DecompilerApp`. A separate Vite configuration builds it into `dist-pages`, using the same Tailwind stylesheet and module worker, without Rari server endpoints. The Rari routes and Docker runtime remain the default target. Asset URLs use the configured site base, including the optional sample download. See [GitHub Pages](github-pages.md).

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

### Deobfuscation jobs

`deobfuscate.rs` owns incremental jobs, options, transforms, reports and pending validated edits. `emulate.rs` provides the optional bounded int32/UTF-16 evaluator, separate from normal inspection and C# reconstruction. Each `deobfuscateStep` examines one method; the client yields between requests. `deobfuscateBegin`, `deobfuscateFinish`, `deobfuscateCancel`, `deobfuscateDownload` and `deobfuscateOpen` complete the typed RPC lifecycle. Evaluation has internal work budgets because cancellation cannot interrupt a synchronous WASM call.

The initial passes preserve metadata tokens and declarations, using the existing strict verifier and `pe_write` output checks. This is not the future full metadata rebuild/remapping system. Generic passes skip EH methods and unsupported writer layouts. Symbol renaming, proxy/forwarder unwrapping, debugger answers and the literal string writer are same-size patches instead, so they still apply where the general writer cannot rewrite whole methods; `pe_write::write_same_size_calls` is the shared call-site rewriter behind the last two, and it re-verifies that every original instruction boundary survives. `deobfuscate/flow.rs` straightens switch dispatchers whose state transitions store a literal, and `deobfuscate/guard.rs` answers the modeled `System.Diagnostics.Debugger` queries. The recognized legacy resource-string pass in `deobfuscate/eaz.rs` instead uses `pe_write::write_literals`: a same-size, stack-effect-preserving literal replacement with checked branch/EH boundaries and full method-section ownership. This path can preserve known fixed-size Field-RVA buffers and EH data without moving them; unknown layouts are rejected. It adds `#US` when absent, verifies unchanged metadata/bytes and does not remap tokens. Only instruction blocks proven unreachable after a validated local branch transformation can be pruned. No-op output preserves all input bytes.

The session permits one deobfuscation job at a time and invalidates it on workspace revision changes. Stale request IDs cannot cancel a newer job. An output opened in the workspace has explicit `derived_from` provenance, its own assembly ID and unchanged assembly identity. Derived snapshots are excluded from canonical reference/member/type resolution, including when their source is closed; they cannot silently replace a dependency. Opening a snapshot invalidates cached analysis and charges its memory against the normal workspace limit. `embedded.rs` lists managed assemblies carried in embedded resources by parsing those bytes as PE/CLI images; it never decompresses, decrypts or executes a payload. See [deobfuscation](deobfuscation.md) for exact scope, limits and test coverage.

### Source readability and evaluation order

Stack lifting deliberately creates a temporary for every pushed value. `transform` removes single-use constant/copy temporaries inside a block after checking local writes and address escapes. An effectful expression moves only into the immediately following use, and only when that use precedes other calls, heap reads and potentially throwing expressions. Conditional edge values do not make an eagerly evaluated call conditional. Duplicated effectful values retain a shared temporary.

The typed IR includes an array-initializer expression. Up to 128 consecutive stores can be folded when they fill a fresh vector in ascending index order and the array has exactly one following use in the same block. No alias, address escape, other write or catch-handler read may observe the partly initialized array. Allocation and element evaluation retain their order, following [C# array-creation semantics](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/language-specification/expressions#128175-array-creation-expressions). Unknown array receivers retain effectful RHS temporaries unless a fresh allocation and an in-range constant index are proven. Arrays that fail these checks remain explicit assignments.

Simple exception regions omit unreferenced labels and jumps that match C# lexical fallthrough, including continuation after sibling handlers. Branch targets needed by other exits remain visible. Local declarations, shared stack slots and nontrivial control flow can still require synthetic names or gotos.

Property syntax requires verified `MethodSemantics` and a compatible non-indexed property signature in the declaring assembly. The session supplies accessor semantics from exact-identity loaded dependencies to both the C# emitter and source export. Direct calls to virtual accessors retain call syntax; unsupported or unresolved references keep `get_Text()`/`set_Text(...)`. Loading or closing a dependency invalidates cached method output. A method merely named `get_...` is never assumed to be a property. PDB names, VB-specific idioms, closure/state-machine reconstruction and broader local lifetime analysis remain separate work.

`samples/Readability` covers repeated property reads, a twelve-element message array, accessor-looking ordinary methods and an array observed by a catch handler. Rust regressions also cover mutation, address escapes, call ordering, allocation ordering and conditional edges. Browser tests inspect actual worker/WASM output and compile a saved, repository-owned reconstructed method for an independent C# syntax check; no analyzed assembly is executed.

### Workspace inspection

`resolution.rs` compares member signatures structurally, including named-type assembly identities. `emit.rs` produces UTF-16 source-reference ranges for emitted call, property, field and object-creation identifiers; the viewer uses these ranges rather than guessing names in source text. Unresolved external scopes can invoke the dependency resolver before falling back to their metadata. Explorer keys and navigation targets include the assembly ID as well as the token.

`analyzer.rs` scans bounded pages of declarations across loaded assemblies. `resources.rs` reads `.resources` v2 records as data and never invokes managed deserialization. The worker exposes `loadDependency`, `analyzeReferences`, `getResourcePreview` and `getResourceBytes`; all heavy parsing stays in Rust/WASM. Folder indexing, tab/history state and bookmarks are browser-session data. See [workspace inspection](workspace-inspection.md).

`framework-catalog.ts` maps supported declared frameworks to fixed Microsoft NuGet packages. `framework-resolver.ts` handles bounded, cancellable downloads inside the worker; `nuget-package.ts` validates ZIP32 records and extracts only the requested DLL. Rust validates its exact identity through `load_reference_dependency`. A download's target-framework context is retained so references followed from that reference assembly can reuse the matching catalog even if its own target attribute is absent. Reference-only provenance suppresses placeholder C# bodies and edits. The in-memory package cache is cleared on disposal; closing the workspace terminates its worker. See [reference navigation](workspace-inspection.md#reference-navigation-and-framework-packs).

`automatic-dependencies.ts` orchestrates dependency resolution immediately after file opening. The input is published before network work begins, and later Explorer updates do not reset navigation. It first resolves the selected folder, then walks supported framework references with bounded attempts, identity deduplication and inherited reference-pack framework context. New dependencies again prefer the folder. Downloads and extraction still run in the worker; failed optional downloads are summarized while the input remains open.
