# Rust parser evaluation

Investigated 2026-09-12 using upstream repositories, current Cargo manifests, source inspection, and an actual `wasm32-unknown-unknown` compile probe. Rust compiler used: 1.92.0. The selected parser's version is pinned; Cargo.lock pins its transitive dependencies.

| Candidate | Findings | Decision |
| --- | --- | --- |
| [dnfile-rs](https://github.com/marirs/dnfile-rs), crate `dnfile` 0.5.1 | `DnPe::parse(&[u8])` borrows the input. Library default features are empty. Filesystem-oriented CLI dependencies are optional. Goblin, scroll, byteorder, serde, and UUID decoding compile to browser WASM. The probe **passed**. `new_clrdata` calls `parse_functions` and eagerly constructs function bodies during load; complete method signatures are not its focus. | Usable WASM candidate, but not selected for its eager method-body architecture. |
| [clrmeta](https://github.com/coconutbird/clrmeta), 0.1.0 | PE-independent `Metadata::parse(&[u8])`; only thiserror is required. No mmap, thread pool, runtime, or OS APIs in the parsing path. The probe **passed**. Source inspection found unchecked stream slicing, unbounded row allocation, and unbounded recursive signature parsing inappropriate for direct hostile input. | Selected behind our strict preflight validator. Library handles tables/heaps; our bounded signature grammar and CIL decoder handle on-demand methods. |
| [dotscope](https://docs.rs/dotscope/0.9.1/dotscope/), 0.9.1 | Rich analysis/SSA/rewrite capabilities and buffer input, but its manifest requires Rust 1.95 and includes unconditional memmap2, rayon, and tempfile; its default feature set also includes emulation and deobfuscation. Not verified as a browser-WASM build with our toolchain. | Not selected. “Pure Rust” alone does not establish browser compatibility. No emulation code is included. |
| [windows-metadata](https://github.com/microsoft/windows-rs/tree/master/crates/libs/metadata) | Cargo selected 0.60.0 as compatible with our toolchain and reported 0.100.0 as latest. Examined the compatible source's reader: optional filesystem loading plus owned-buffer parsing, alignment-dependent unsafe reads, Windows metadata/code-generation orientation. This is not a complete lazy CIL decompiler. No claim that native Windows APIs are necessary for every reader operation. | Not selected; browser compatibility was not established for the latest release. |
| [goblin](https://docs.rs/goblin/0.10.7/goblin/pe/index.html), 0.10.7 | PE readers accept byte slices. Selected PE32/PE64/std features compile to browser WASM; the probe **passed**. It does not itself supply ECMA-335 tables or a decompiler. | Used only for PE header and section parsing. Our container adapter maps RVAs with strict file-backed bounds. |

## Reproducible compile check

The probe used the following dependencies and invoked all three byte-buffer entry points from a library:

```toml
[dependencies]
clrmeta = "=0.1.0"
dnfile = "=0.5.1"
goblin = { version = "=0.10.7", default-features = false, features = ["pe32", "pe64", "std"] }
```

```sh
cargo check --target wasm32-unknown-unknown
```

The shipped workspace repeats the target check during `npm run build:wasm`; browser tests load the emitted module and parse real PE fixtures. Passing a target check is necessary, not a security audit: the external parser only receives preflight-validated metadata.

## Security envelope around clrmeta

Before `Metadata::parse`, ILens checks the root/version span, stream directory, unique names, stream extents and overlaps, table mask, table/heap index widths, total row counts, each complete row extent, every heap index, every table/coded index, and ordered member-list partitions. Unoptimized pointer tables and unsupported table flags produce a diagnostic before entering the library. Heap names and signature recursion have separate limits.

The adapter also compensates for clrmeta 0.1.0's HasCustomAttribute tag 8 description during validation: ECMA-335 assigns that tag to DeclSecurity. Such rows remain inspectable through raw metadata; there is no security-declaration source reconstruction.

The library copies metadata heaps internally. ILens retains one assembly byte buffer plus parsed table records and heap copies; it does not pretend the whole parsing path is zero-copy. The main-thread → worker transfer is ownership transfer, and wasm-bindgen makes one copy into Rust for the input buffer. No filesystem API is part of that path.
