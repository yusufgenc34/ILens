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
                 AST → simplification → C# emitter
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
