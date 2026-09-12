# mono-wasm architectural study

Reviewed before implementation on 2026-09-12. Reference: [migueldeicaza/mono-wasm](https://github.com/migueldeicaza/mono-wasm), commit [`c46c1e0185d1cabd4a19798e9f55673f767369da`](https://github.com/migueldeicaza/mono-wasm/tree/c46c1e0185d1cabd4a19798e9f55673f767369da). A shallow checkout was inspected, including README.md, index.js, boot.c, mono-wasm.cpp, Makefile, and the hello browser samples.

## What it actually does

This is a historical proof of concept for compiling managed programs ahead of time and running them in a browser. It is **not a decompiler**. Its build joins Mono runtime and libc code with LLVM bitcode produced from managed assemblies. Metadata-bearing DLLs still accompany the executable WASM artifact.

The [README](https://github.com/migueldeicaza/mono-wasm/blob/c46c1e0185d1cabd4a19798e9f55673f767369da/README.md) describes both whole-program LLVM bitcode linking and incremental WASM linking. It depends on experimental LLVM/clang/lld, custom Mono and libc forks, and assumptions about a 32-bit host/target. The historical GC and runtime compatibility limitations are explicit.

## Browser startup and managed assembly loading

[`index.js`](https://github.com/migueldeicaza/mono-wasm/blob/c46c1e0185d1cabd4a19798e9f55673f767369da/index.js) constructs an `env` import object, fetches `index.wasm`, compiles/instantiates it, and then fetches the DLL names in its generated `files` array. Each response becomes a Uint8Array in `files_content`. Startup waits for all assembly requests before entering the runtime.

The file/syscall shims make those cached buffers available to Mono using a filesystem-shaped interface. Those are browser memory buffers, not a normal operating-system filesystem. Other imported functions provide console I/O, allocation support, and assorted libc/Mono compatibility shims. Numerous functions are missing or stubbed, including thread operations; that is unsuitable as a foundation for a hostile-input analysis service.

ILens borrows the **explicit initialization barrier** and **in-memory assembly inventory** concepts. It initializes its parser module once per worker, then accepts user-selected transferable buffers. The bundled sample is fetched only after an explicit sample button click. User files and dependencies are never fetched from a remote resolver.

## JavaScript ↔ WASM interface and memory

The JavaScript glue wraps exported Mono functions and marshals strings/arguments through linear memory. Functions such as `heap_get_int`, `heap_get_string`, `heap_get_mono_string`, and `heap_malloc_string` illustrate the pointer/length and ownership boundary. `_MonoDomain` and assembly/method helpers expose runtime objects to JavaScript.

Its memory-growth path calls `memory.grow` and recreates the Uint8Array view over the new buffer. Recreating views after growth is still relevant: old views can become detached. Modern wasm-bindgen supplies that machinery for ILens. No hand-maintained pointer arithmetic or Mono object layout is carried over.

The original startup comments explicitly assume some allocations can survive until process exit. An interactive decompiler has a different lifetime: multiple assemblies, repeated independent requests, cancellation, and closing workspaces. ILens therefore uses owned Rust buffers, bounded cached method results, `close`/`dispose`, wasm-bindgen `free`, and worker termination to release an entire WASM instance.

## Execution is deliberately excluded

[`boot.c`](https://github.com/migueldeicaza/mono-wasm/blob/c46c1e0185d1cabd4a19798e9f55673f767369da/boot.c) registers AOT modules, configures Mono, initializes a domain, calls `mono_assembly_open`, and ultimately invokes `mono_jit_exec`. `index.js` calls `mono_wasm_main` once its loader is ready. Its JavaScript-facing assembly and method helpers are designed to invoke managed code.

None of these execution paths are used. ILens never starts Mono, invokes constructors, runs module initializers, performs P/Invoke, or calls an analyzed method. Its only executable WASM is the Rust static-analysis engine built by this project. DLL bytes are data; they are never passed to WebAssembly.compile/instantiate.

## Build system: historical rather than reusable

[`Makefile`](https://github.com/migueldeicaza/mono-wasm/blob/c46c1e0185d1cabd4a19798e9f55673f767369da/Makefile) and [`mono-wasm.cpp`](https://github.com/migueldeicaza/mono-wasm/blob/c46c1e0185d1cabd4a19798e9f55673f767369da/mono-wasm.cpp) coordinate custom toolchain paths, bitcode generation, AOT registration, LLVM/lld linking, and JavaScript/assembly packaging. ILens has no reason to inherit this machinery. Cargo targets `wasm32-unknown-unknown`; wasm-bindgen emits the loader; Vite+ emits a module worker and hashed WASM asset; Rari serves the application shell.

## Reuse and licensing

No source code from mono-wasm is copied into this repository. The influences above are architectural. The reference project is MIT-licensed; any future copied code must retain its Microsoft copyright notice and applicable license. The opcode data used here instead comes from the current dotnet/runtime opcode definition, with its notices recorded in [third-party-notices.md](third-party-notices.md).

There is no experimental Mono adapter. A runtime adapter would increase the execution surface without improving the first static-analysis milestone.
