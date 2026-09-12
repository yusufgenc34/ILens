# Performance check

Measured on the development machine (macOS ARM64, Rust 1.92 release build, headless Chromium), 2026-09-12. These are single-run engineering checks, not cross-device benchmarks or latency guarantees.

## Input and results

A one-time C# fixture generator emitted 500 classes with 80 static methods each. The .NET compiler adds 500 constructors, producing **40,500 method definitions**, **501 TypeDefs** including `<Module>`, and a **1,911,296-byte** IL-only DLL. The analyzed assembly was never executed.

| Measurement | Observed |
| --- | --- |
| Native release: PE/metadata load + index | 23.9 ms |
| Native release: bounded name search | 3.2 ms |
| Native release: selected method analysis/emission | 0.10 ms |
| Chromium: file selection → populated primary selector | 198 ms |
| Chromium: search input → selected method C# visible, including 250 ms debounce | 525 ms |
| Browser main-thread tasks over 50 ms during that interval | None observed |
| Main-thread 16 ms timer callbacks during that interval | 39 |
| Mounted tree rows with the selected method expanded | 37 |

The browser run exercised the actual File API, transferable worker RPC and release Rust WASM. A PerformanceObserver recorded main-thread long tasks. Rust work did not run on the page thread. The roughly 604 KiB WASM artifact is loaded once per worker. Compilation/cache warmup and hardware influence timings; malformed/obfuscated methods, deep signatures and exceptional control-flow graphs have different costs.

## Profiling a current input

The one-time generator used for the measurement above is not part of the maintained build. The native profiler remains available for any managed input:

```sh
cargo run --release -p decompiler-core --example profile -- path/to/assembly.dll
npm run dev
```

Open the same DLL in the browser and select a method. Use browser developer tools to observe worker execution, memory, and main-thread tasks. Compare repeated measurements on the same hardware and input; the historical figures above are not a claim about a different assembly.

## Limits and next profiling work

Metadata indexing is eager; method decoding and reconstruction are lazy. The metadata library copies heaps, and returning the declaration index creates JavaScript objects in the worker and main thread. The tree only mounts visible rows. Bounds prevent one input from consuming unlimited work, and the hard watchdog releases an unresponsive worker. WASM deallocation reuses memory rather than shrinking linear memory; terminate the workspace to release it completely.

This check does not substitute for a diverse .NET Framework/modern .NET corpus, a sustained fuzzing campaign, or Firefox/Safari/mobile measurements. Signature-heavy and highly connected methods, especially exception-rich ones, should be included in future performance work before increasing current limits.
