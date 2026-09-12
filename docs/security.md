# Security and resource limits


Assembly bytes never go to Rari, an API, a database, analytics, or browser storage. The File API reads them, ownership transfers to the worker, and wasm-bindgen copies them once into Rust-owned memory. clrmeta also keeps parsed table records and copies of metadata heaps; the parser is not claimed to be zero-copy. Only appearance and editor preferences are saved; they are validated when restored. Refreshing clears loaded assemblies.

**Uploaded assemblies are never executed.** There is no managed runtime, interpreter, JIT, constructor invocation, entry-point invocation, module initializer or P/Invoke execution path. Rendering uses React text and CodeMirror documents, not assembly-controlled HTML. There are no server actions that accept assembly contents. The explicit sample button fetches a static sample shipped with this repository; dependencies are never fetched automatically.

Own Rust crates forbid `unsafe`. File spans, RVA mappings, stream extents/overlaps, heap indices, table/coded indices, signature recursion, branch targets, exception boundaries and method sizes are checked. Third-party code is inside a validation adapter. Malformed input produces structured error codes and human-readable messages with diagnostic details. A WASM trap causes the worker to be discarded, not reentered.

| Bound | Default |
| --- | --- |
| Single file / session inputs | 64 MiB / 128 MiB, at most 16 assemblies |
| WASM linear memory | 512 MiB maximum |
| Metadata / total rows | 32 MiB / 500,000 |
| Type definitions / navigable declarations | 10,000 / 150,000 |
| Metadata name / expanded declaration-name index | 4 KiB / 32 MiB |
| References / resource declarations | 4,096 each |
| Method body / decoded instructions | 1 MiB / 100,000 |
| High-level reconstruction | 10,000 instructions, with IL available above that bound |
| CFG blocks / stack slots | 8,192 / 1,024 |
| Signature/type nesting | 48 levels |
| Worker request queue / execution watchdog | 128 / 30 seconds |

These are engineering safeguards, not a claim of formal verification or a completed independent security audit. Size checks, deterministic mutation tests and browser recovery tests cover hostile input; more fuzzing and real-world corpus testing remain valuable before exposing an installation to sustained adversarial workloads. Larger legitimate assemblies may require raising limits after profiling.
