# Security and resource limits


Assembly bytes never go to Rari, an API, a database, analytics, or browser storage. The File API reads them, ownership transfers to the worker, and wasm-bindgen copies them once into Rust-owned memory. clrmeta also keeps parsed table records and copies of metadata heaps; the parser is not claimed to be zero-copy. Only appearance, editor and framework-download preferences are saved; they are validated when restored. Refreshing clears loaded assemblies.

**Uploaded assemblies are never executed.** There is no managed runtime, interpreter, JIT, constructor invocation, entry-point invocation, module initializer or P/Invoke execution path. Rendering uses React text and CodeMirror documents, not assembly-controlled HTML. There are no server actions that accept assembly contents. The explicit sample button fetches a static sample shipped with this repository; opening an EXE/DLL can automatically fetch fixed Microsoft framework packages from NuGet when enabled in Settings. No assembly contents are included in those requests.

Own Rust crates forbid `unsafe`. File spans, RVA mappings, stream extents/overlaps, heap indices, table/coded indices, signature recursion, branch targets, exception boundaries and method sizes are checked. Third-party code is inside a validation adapter. Malformed input produces structured error codes and human-readable messages with diagnostic details. A WASM trap causes the worker to be discarded, not reentered.

| Bound | Default |
| --- | --- |
| Single file / session inputs | 64 MiB / 128 MiB, at most 64 assemblies |
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

## Source and binary output

Source archives use safe names, collision/token suffixes, fixed timestamps, CRC32 and explicit size limits. Solution GUIDs are deterministic export-local identifiers. Project output names cannot inject parent paths; references use only resolved loaded identities. Generated project XML escapes XML and MSBuild expansion syntax and contains no input-provided build tasks or scripts. Copying loaded dependency DLLs into an export requires selecting that option. Original assembly strings are assembly content and are not claimed to be anonymized.

Edits are in-memory overlays with revision checks. A separate conservative write verifier blocks unsupported signatures/instructions and invalid control flow or stacks. The PE writer rejects signed/native/unknown layouts and verifies the serialized output before download. Debug-directory records and their supported payloads are cleared in edited outputs, with overlap checks; clearing a pointer alone is insufficient. No analyzed or emitted managed code is executed. The test suite only compiles repository-owned reconstructed fixtures and statically verifies owned outputs with ILVerify.

Export has a 64 MiB archive/output cap, 4 MiB entry cap, 25,000-entry cap and 32 MiB retained member-text accounting. Editing has a 1 MiB request cap, 10,000 instructions and 256 validated method overlays. These are accounting limits rather than proofs of peak heap use; see [export/edit details](export-edit.md).

## Selected folders and resource inspection

A directory picker grants access only to files explicitly selected by the user. The application retains File handles in memory, indexes DLL/EXE filenames and reads candidates only for unresolved references. Matching checks name, version, culture and public key token in the worker. Wrong identities are discarded; identical already-loaded bytes reuse their assembly ID, while different bytes with the same identity are rejected. Absolute host paths and folder contents are not stored or transmitted. Clearing the folder releases its handles; closing the workspace or refreshing releases the full session.

Folder indexing accepts 4,096 files, at most 32 candidates per assembly filename and 128 candidate inspections per resolution pass. Normal input and workspace limits still apply. Analysis scans 32 declarations per worker request, returns at most 512 hits per page and displays at most 1,000, with truncation reported.

Resource inspection is bounded to 4 MiB per embedded resource, 4,096 entries, 1,024 serialized type names, 1 MiB of entry names and 64 KiB per decoded string. Hex previews show 4,096 bytes. PNG previews require checked IHDR dimensions (at most 8,192 per side and 16 million pixels), are displayed through revocable blob URLs, and are decoded by the browser. Text and markup remain text; HTML and SVG are never injected. Serialized object payloads are labeled and are not deserialized. BAML conversion and external resource loading are unsupported.

## Framework reference downloads

Opening an assembly starts dependency resolution automatically, checking the loaded workspace and selected folder before any framework download. Following an unresolved reference can also retry it. The optional online fallback uses a fixed catalog of Microsoft package IDs and versions, selected from the declared target framework and recognized framework identities. The worker issues GET requests directly to `https://api.nuget.org/v3-flatcontainer/`; no proxy or application server receives the assembly. Requests omit credentials and referrer and reject redirects. NuGet sees the browser IP address and the selected public framework package/version. Private assembly names, metadata, local paths and bytes never form request URLs or bodies.

The catalog intentionally pins the official API host and package versions instead of accepting URLs from an input assembly or following arbitrary service-index hosts. Upstream API changes require a catalog update. Each automatic pass allows at most 64 framework attempts and shares a 128-file local candidate budget; a network/package-download failure suppresses repeat downloads of that package for the remainder of the pass. Packages are limited to 32 MiB, 10,000 ZIP entries and a 15-second download timeout. Only the requested DLL under a catalog reference directory is extracted, with an 8 MiB limit, bounds checks, local/central header consistency and CRC validation. ZIP64, encryption, ambiguous/path-traversing entries and unsupported compression are rejected. Package scripts and arbitrary files are never installed or run. Exact assembly identity is checked by the Rust parser before retention; this is not cryptographic strong-name or NuGet-signature verification. The download trusts the fixed NuGet HTTPS endpoint.

At most two compressed packages, totaling 48 MiB, are cached in worker memory. Extracted assemblies remain subject to workspace limits. Closing the workspace terminates the worker and cancels downloads; `dispose` also clears the package cache. Nothing is retained in browser storage or an application-managed persistent cache. The request uses `cache: no-store`. The Rari and Pages content security policies allow only the NuGet API origin in addition to the application's own connection origin. Disable **Download .NET references automatically** in Settings to prevent this fallback.

ReadyToRun reading is restricted to the preserved ECMA metadata and CIL. Native headers and directories are bounds checked; native machine code is never executed or reconstructed. Editing and binary writing remain blocked for these images. Companion-DLL lookup for native EXEs uses only unambiguous files already supplied by the user, never paths or URLs extracted from binary strings.
