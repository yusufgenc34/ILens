# Export and assembly editing roadmap

Status: the first scoped implementation is available. Method/type downloads, source archives, per-assembly projects and solutions, IL editing and modified binary output are implemented within the limits documented in [Source export and IL editing](export-edit.md). The broader acceptance criteria below remain the roadmap; general compilable-project parity, full symbol mapping, broader verification and arbitrary metadata rewriting are not complete.

The agreed scope is source download, complete type export, assembly source archive, project export, IL editing, then modified DLL/EXE output. Binary export means rebuilding an edited managed assembly. Compression, obfuscation, and executable packers are outside this roadmap.

## IL editing expansion

The first expansion is implemented: primitive/reference vectors and jagged arrays, array element verification, null/reference stack merges, nongeneric reference-class instance methods, local fields/property accessors, local constructor calls, primitive boxing/unboxing and supported casts. The signature model now distinguishes class/value-type markers and vectors/general arrays. Browser tests independently verify downloaded modified assemblies with ILVerify.

The next write-verifier stages remain generic type substitution/constraints, custom value types and managed addresses, external definition resolution, constructor initialization states, and symbolic exception regions. These require dedicated verification and writer support; they are not enabled by removing capability guards. Current support is documented in [IL write verification](il-verification.md).

## Planning baseline (before the first implementation)

| Area | Existing implementation | Work required |
| --- | --- | --- |
| Method output | `decompile.rs` returns C#-like text, independent IL, quality, and diagnostics | Download support and an export contract that preserves limitations |
| Type output | `Session::type_view` prints declarations, capped at 2,000 members and 1 MiB | A complete declaration model, method bodies, accessor ownership, and explicit completeness reporting |
| Resources | `Assembly::resources` lists ManifestResource metadata | Bounded extraction of embedded resource bytes; external resources remain unresolved |
| Worker | Request IDs, cancellation between calls, transferable inputs, 30-second watchdog | Bounded export steps, structured progress, output transfers, and cleanup |
| Cache | Method results keyed by assembly ID and token | Edit revision and dependency-context awareness |
| CIL | Decoder, CFG, stack analysis, typed IR, and method-body AST | Editable instruction representation, encoder, and stricter validation for writes |
| PE/metadata | Validated reading through internal abstractions | A writer with explicit preservation rules and round-trip checks |

The method-body AST is not yet a C# compilation-unit AST. Existing `csharp_like` output must not be treated as proof of compilability or exact semantic equivalence. The existing stack analysis also must not be presented as a complete ECMA-335 verifier.

## Milestones

| Milestone | User-visible result | Depends on | Relative effort |
| --- | --- | --- | --- |
| M1 | Download the selected method as C#-like text or IL | Current method pipeline | Small |
| M2 | Download a complete supported class/type as `.cs` | M1, declaration model | Medium to large |
| M3 | Download assembly source and IL as a ZIP | M2, export jobs, resource extraction | Medium to large |
| M4 | Download a reconstructed project scaffold | M3, framework/reference mapping | Large |
| M5 | Edit and validate supported method IL with undo/redo | Editable CIL model and encoder; scheduled after M4 | Large |
| M6 | Download a modified, validated DLL/EXE | M5, PE writer | Largest |

Effort labels compare implementation risk, not delivery dates. Estimate calendar time after the first scoped pull request and the writer feasibility work. Export delivery does not depend on a binary writer; the IL encoder can be developed independently once its contract is agreed.

### M1: Selected method download

Add an Export menu to the existing workspace toolbar and the method context menu. Offer `Download C#` and `Download IL` for the selected method. Reuse the selected analysis result when its assembly and revision still match.

Produce UTF-8 text with LF endings and a safe suggested filename containing a token suffix where needed. Include a short reconstruction notice and the method token, with no hostname, local path, personal email, or capture/build timestamp.

Only enable C# download when the method has C#-like output or a bodyless declaration. An `annotated_il` result must download as `.il`, accompanied by its diagnostic; do not place raw IL in a `.cs` file. A bodyless method is labeled a declaration rather than an implementation.

Acceptance:

- The downloaded method matches the selected result, including integer precision, escaping, and diagnostics.
- Changing selection or closing the assembly during an export cannot download a different method under the old filename.
- Known arithmetic/loop fixtures produce C#-like downloads; async/iterator limitations produce honest IL downloads.
- Unsafe metadata names cannot create unsafe filenames; temporary Blob URLs are revoked.
- Chromium tests exercise actual download events and inspect downloaded bytes.

### M2: Complete type export

Introduce a declaration model for compilation units, namespaces, types, fields, constructors, methods, properties, events, and attributes. Associate declarations with metadata tokens and use a shared symbol-name map for declarations and references. Refactor method reconstruction to make its body AST available to the type emitter without parsing previously emitted C# text.

Start with the checked fixture corpus: classes, structs, interfaces, enum constants, delegates, generic parameters and constraints, and nested types. Emit constructors and constructor initializers deliberately. Associate getter/setter and add/remove bodies with their owning property/event so that accessors are not also emitted as duplicate methods. Preserve members that cannot be reconstructed in the accompanying IL/report rather than silently dropping them.

Separate inspection output from compilable output. If a type contains unsupported members, its bundle may contain a clearly labeled partial `.cs` representation, independent `.il` files, and a diagnostic report. Do not manufacture throwing/default-return implementations to make an incomplete export appear complete.

Acceptance:

- Export enumerates all members; it does not reuse the truncated type preview as source.
- Every member token has a reported outcome: reconstructed, declaration-only, IL fallback, or error.
- Generic arity, overloads, nested types, accessor ownership, field constants, and constructor handling have fixtures.
- Name collisions and invalid C# identifiers use stable token-based mappings applied consistently to references.
- Fully supported fixture types compile in an isolated test project; compilation is never claimed for partial types.

### M3: Assembly source archive

Add `Export assembly sources` with a summary, progress, cancellation, and a final report. Organize supported type output by namespace and retain nested types with their containing type. Always allow inclusion of independent IL so the user can inspect unsupported reconstruction.

Proposed archive layout:

```text
Example.sources.zip
  src/Example/Calculator.cs
  il/Example/Calculator.il
  resources/...
  assembly.json
  export-report.json
  README.txt
```

Extract embedded managed resources by validating the CLI resource range, per-resource offset, length prefix, and payload bounds. Copy the payload as data. Do not instantiate resource objects, use managed deserialization, resolve external resource files from the network, or claim BAML-to-XAML / designer recovery. External resource references are recorded as unavailable.

The report maps metadata tokens to output paths and records reconstruction status, renamed symbols, missing dependencies, unsupported resources, and failed members. Use a separate `complete`, `partial`, `cancelled`, or `failed` job outcome. An individual malformed member may yield a partial archive; cancellation or a fatal worker failure must not silently return a successful archive.

Acceptance:

- Export processes every selected type/member or reports why it was omitted; one unsupported body does not discard other valid output.
- ZIP entry paths reject traversal, absolute paths, drive prefixes, separators in names, reserved device names, control characters, and case/Unicode collisions.
- ZIP metadata uses fixed neutral timestamps and contains no exporter host paths or personal identifiers. Original assembly metadata is assembly content, not host metadata.
- Worker progress remains responsive, cancellation releases job buffers, and large exports remain within explicit memory/output limits.
- Tests unpack the archive and verify file contents, resource hashes, token coverage, and diagnostics.

### M4: Reconstructed project export

Add a project-export option that builds on the source archive. Generate `.csproj` only for explicitly supported target-framework and project configurations. Infer configuration from declared TargetFrameworkAttribute and compatible metadata; never treat `v4.0.30319` as an exact framework target. Ask for a target or export sources only when the target cannot be established.

Use loaded dependency identities to describe references. Missing references are reported rather than downloaded. Do not guess NuGet package IDs from assembly names. Including loaded dependency DLLs in the archive is an explicit option, disabled by default; the manifest otherwise lists dependencies the user needs to provide.

Generate project XML from a fixed allowlisted template with explicit compile/resource items. Escape both XML and MSBuild expressions in all derived values. Do not reproduce arbitrary imports, targets, build tasks, analyzers, source generators, post-build commands, or package scripts from an input. Do not run the exported project in the browser.

Acceptance:

- Selected fully supported fixture projects compile against the required framework/reference assemblies without running their output.
- Generic constraints, overrides, explicit interface implementations, layout/interop declarations, and emitted attributes are either supported correctly or explicitly reported as blockers.
- An export with missing dependencies, unsupported members, or a guessed configuration cannot be labeled build-ready.
- WPF/WinForms designer projects, BAML conversion, original project settings, and original source formatting are explicitly outside the first project-export milestone.

### M5: IL editing and validation

Keep the original assembly bytes immutable. Store method edits in an in-memory overlay keyed by assembly ID, method token, and revision. Add an instruction editor with opcode/operand inputs, resolved token search, insertion/deletion, undo/redo, an original-versus-edited view, and `Validate` / `Discard changes` actions.

Use stable instruction IDs and symbolic labels. Branch and switch operands reference labels rather than mutable byte offsets. Treat instruction prefixes and their target instruction as an atomic group where applicable. Preserve exact integer widths and floating-point bit patterns, including non-finite values, in the editable representation.

Build a CIL encoder that computes offsets to a fixed point, widens short branches when necessary, checks operand ranges, and creates the method header. Validation checks token kinds, call signatures, variable indices, control-flow boundaries, stack heights/types and merge compatibility, return values, and prefix legality. Report unsupported verification separately from a valid method. Existing analysis can contribute evidence but cannot alone authorize writing an arbitrary edited body.

First editable scope: managed CIL bodies with existing signatures, existing metadata tokens, and unchanged local signatures. Initial UI editing excludes methods with exception regions and unsupported instruction/prefix patterns. Exception editing follows with symbolic region boundaries and checks for legal branch/leave transitions, nesting, handler entry stacks, and filters. New strings, locals, members, types, references, and C# recompilation are later work.

Acceptance:

- Decode/encode/decode preserves supported instruction semantics and exact numeric operands; untouched raw encodings can be retained.
- Insert/delete/undo operations update labels consistently and cannot leave dangling branch targets.
- Short-branch boundaries, switches, invalid merges, malformed operands, and unsupported patterns have focused tests.
- Original and edited views remain distinguishable; stale validation responses cannot approve a later revision.
- Preview analysis runs on the edited method model without executing any uploaded code.

### M6: Modified assembly export

Add `Download modified assembly` after method validation and a whole-output preflight. Display the changed methods and any unsupported input features before generating a new filename such as `Example.modified.dll`. Reparse and validate the complete output before enabling its download.

Implement the writer in bounded steps:

1. A no-change serialization path that preserves the original file byte-for-byte under an explicit preserve policy.
2. Replacement bodies that fit their original allocation, with unchanged metadata identity, tokens, and local signatures.
3. Relocated/growing method bodies in a validated output layout, with updated MethodDef RVAs, PE sections, alignment, sizes, and directories. Insufficient header space or unsupported layouts produce diagnostics.
4. Metadata heap/table growth only after a separate writer milestone covers index-width changes, coded indices, signatures, user strings, and token/reference remapping.

The first downloadable edited binaries are unsigned, IL-only managed PE DLLs/EXEs accepted by a writer-specific capability check. Reading support does not automatically imply writing support. Preserve architecture, managed entry point, existing metadata, embedded resources, and unaffected native PE structures only where the writer has explicit tests. Reject unhandled overlays, relocations, directories, and container variants instead of silently discarding them. ReadyToRun, mixed-mode, NativeAOT, and WebCIL remain outside scope.

Signed inputs are blocked initially. Later support must distinguish strong-name signatures from Authenticode, require an explicit signing policy, and never imply that an old signature remains valid after modification. Do not silently remove an assembly's identity or signatures.

Edited downloads default to removing stale debug information: CodeView records, embedded PDBs, and associated payload bytes must actually be omitted or cleared, not merely made unreachable through directory pointers. This export policy is distinct from the internal byte-preserving no-change test. Reparse the final bytes, compare unchanged metadata and resource hashes, and rerun method/CFG/stack validation. Assembly bytes are never executed during export or validation.

Acceptance:

- Supported no-op round trips are byte-identical under the preserve policy.
- In-place and growing-body fixtures retain unaffected methods, tokens, entry points, references, resources, and PE layout invariants.
- An independent PE/CLI reader accepts the output; an independent static IL verifier checks supported fixtures with their reference assemblies available.
- Invalid or incompletely validated edits cannot be downloaded as a successful modified assembly.
- Browser tests reopen exported binaries in a fresh worker and confirm the requested IL change.
- Final output contains no host identifiers introduced by ILens and no stale debug paths under the default edited-export policy.

## Architecture changes

Proposed modules, added only as their milestone needs them:

| Location | Responsibility |
| --- | --- |
| `decompiler-core/src/declarations.rs` | Compilation-unit/type/member model and symbol mapping |
| `decompiler-core/src/export/` | Export jobs, manifest, reports, type/project emission, bounded resource extraction |
| `decompiler-core/src/edit.rs` | Immutable-base overlays, revisions, edit transactions, validation results |
| `decompiler-core/src/cil_encode.rs` | Labels, instruction encoding, branch sizing, method bodies |
| `decompiler-core/src/pe_write.rs` | Writer preflight, layout, body placement, final PE output |
| `decompiler-wasm` | Coarse export/edit APIs and owned output buffers |
| `src/lib/types.ts`, `rpc.ts` | Typed job/edit requests, structured progress, revision checks |
| `src/workers/decompiler.worker.ts` | Job scheduling, archive creation, cancellation, buffer transfer |
| `src/components` | Export dialog, progress/report view, IL editor, changes panel |

Keep method reconstruction independent from PE writing. A source export must not acquire write access to the original assembly. Dependency changes and method edits invalidate affected analysis caches; initially, conservatively invalidate the affected assembly or the whole dependency context rather than risk stale xrefs or declarations.

## Worker and memory contract

Proposed coarse operations are `beginExport`, `stepExport`, `finishExport`, `cancelExport`, `openMethodEdit`, `applyMethodEdits`, `validateMethodEdit`, `discardMethodEdit`, and `exportModifiedAssembly`. Exact Rust/TypeScript shapes are fixed in the first relevant implementation PR.

Export jobs own a stable assembly/dependency revision snapshot. Closing an assembly or changing an edited revision cancels or invalidates the affected job. Every response includes its request ID and relevant job/revision ID. Editing and export do not mutate the selected method through an unrelated stale response.

Start with one export per workspace and one export step in flight. Process bounded groups of members and yield between WASM calls so cancellation can run. Measure step duration and adapt batch size; a single pathological method remains subject to the existing hard watchdog. The current client starts its timeout when a request is submitted, including queue time; the new scheduler must account for queueing and keep the execution deadline bounded. Apply the 30-second execution watchdog to individual work steps, not the total legitimate export duration. A long-running operation cannot be made cancellable merely by increasing that timeout.

Move large output chunks as transferable buffers. Copy bytes out of WASM-owned memory before freeing or reusing that memory; do not transfer a live WASM memory view. Keep archive generation in the worker, apply backpressure, and avoid returning the whole archive through JSON. Prefer transient export results over filling the interactive method cache. All completed, cancelled, failed, closed, and superseded jobs must release their buffers and Blob URLs.

Initial profiling budgets to validate before M3 ships: 128 MiB total uncompressed export data, 64 MiB final archive, 4 MiB per entry, and 25,000 entries. These are output caps, not claims about browser peak memory; measure combined WASM, JS, compression, and download-buffer allocations. Existing input and WASM limits continue to apply. Exceeding a cap produces an actionable size-limit result, never silent truncation.

## Dependency decisions

Do not commit to a ZIP, compression, or PE writer dependency solely from its API description. Probe a minimal browser worker build, inspect filesystem/mmap/native/thread requirements, measure bundle and peak-memory cost, and review the exact license/version before adoption. A pure JavaScript ZIP implementation in the worker and a pure Rust byte-buffer implementation are both candidates; neither is selected by this plan.

Evaluate any PE/metadata writer behind internal abstractions. Existing parser dependencies do not imply writer compatibility. Microsoft encoding APIs are format references, not a proposal to run a managed runtime inside the browser. Do not copy dnSpy implementation code without a separate license compatibility decision.

## Validation and implementation sequence

Use small PRs with these reviewable boundaries:

1. Export contracts, safe naming, and selected-method downloads.
2. Declaration model, symbol map, and complete supported-type emission.
3. Export job scheduling, resource extraction, and source ZIP downloads.
4. Framework/reference mapping and conservative project generation.
5. Editable instruction model and encoder round-trip tests.
6. IL editor, revisioned overlays, undo/redo, and write validation.
7. Writer capability checks and in-place method-body export.
8. Growing-body layout support and independent output verification.

For compiler/writer changes, run Rust tests, formatting, Clippy, the browser-WASM build, and relevant browser tests. Run typecheck/lint and worker RPC tests for protocol changes. Validate production and Docker downloads before releasing an export milestone. Add malicious filenames, truncated methods/resources, large outputs, cancellations, and stale revisions to the test corpus.

Compile only repository-owned reconstructed fixture projects in isolated CI directories, with controlled references and no execution of emitted assemblies. Never turn project-export validation into a service that builds or executes arbitrary uploaded programs. Add focused privacy checks for archive metadata, generated files, debug records, and download names alongside the existing fixture privacy tests.

Source archives may be partial if clearly reported. Modified binaries require a stricter gate: unsupported validation or unknown preservation requirements block export. Neither feature may silently substitute fake implementations for unsupported code.

## Deferred work

- C# editing with compiler integration, arbitrary metadata/member creation, and full rebuildable-project parity.
- Advanced async/iterator/closure reconstruction and WPF/WinForms designer recovery.
- Strong-name re-signing and Authenticode workflows.
- Compression/obfuscation packers and runtime unpackers are outside the agreed binary-export scope.
- Automatic dependency downloads, execution/debugging of uploaded assemblies, server-side analysis, and persistent assembly storage.

## References

- [dnSpy](https://github.com/dnSpy/dnSpy): workflow reference for project export and IL editing.
- [ECMA-335](https://ecma-international.org/publications-and-standards/standards/ecma-335/): metadata, signatures, CIL, and exception-region requirements.
- [Microsoft PE format](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format): section layout, directories, and address mapping.
- [MethodBodyStreamEncoder](https://learn.microsoft.com/en-us/dotnet/api/system.reflection.metadata.ecma335.methodbodystreamencoder?view=net-10.0): reference for method headers, locals, stack bounds, and exception data.
- [ManagedPEBuilder](https://learn.microsoft.com/en-us/dotnet/api/system.reflection.portableexecutable.managedpebuilder?view=net-10.0): reference for PE serialization and signing boundaries.
- [C# compiler PathMap](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/compiler-options/main-compiler-option#pathmap): avoiding exporter/build-machine paths in generated artifacts.
