# Source export and IL editing

The File menu contains project export, source saving and modified module saving. IL editing is available in the code viewer’s IL tab. All analysis runs in the browser worker. No uploaded assembly is executed or sent to the application server.

## Export to Project

Choose **File > Export to Project**. Check the assemblies to export, select a target framework where metadata does not provide a supported target, and choose whether to create a solution. Each selected assembly becomes a separate C# SDK-style project. The browser downloads their folders as one ZIP.

```text
Example.zip
  Example.sln
  Example/
    Example.csproj
    Program.cs
    Models/Item.cs
    Properties/AssemblyInfo.cs
    Resources/Example.message.txt
    References/External.Dependency.dll    (optional)
    _ilens/export-report.json
    _ilens/assembly.json
    _ilens/il/06000001.il                 (unsupported methods, or all if selected)
    _ilens/metadata/06000001.json
  Dependency/
    Dependency.csproj
    ...
  _ilens/export-report.json
  README.txt
```

Source filenames retain their type names, with suffixes only for collisions. Namespace folders are relative to the project's common namespace. Nested types remain with their containing type. The exporter enumerates all members and uses AST-derived method bodies independently of the truncated code-view preview. Unsupported methods remain diagnostic comments with separate IL; it does not invent throwing/default-return stubs.

Projects preserve the safe assembly output name, declared version/culture, target framework, PE/CLI-derived Library/Exe/WinExe output, platform, supported startup type and raw embedded resources with their original logical names. Unsupported target profiles require a manual framework choice; the CLR metadata version is never treated as the target framework. Selecting both assemblies in a resolved reference produces `ProjectReference`; other loaded dependencies can optionally be copied into `References`. Unavailable references retain their metadata identity. ILens does not fetch references or guess NuGet packages.

`_ilens/export-report.json` records project outcomes and links to per-project reports. Failed/unsupported bodies do not discard other valid sources. Checking **Include IL and metadata for all members** retains the inspection data for supported methods too. Archive size-limit failures abort the download.

This workflow follows the [dnSpy export study](dnspy-export-notes.md). Project files are reconstructed, not original build files or a guarantee of compilation. Arbitrary attributes, signing, application icons/manifests, BAML/XAML, resx conversion and designer reconstruction remain incomplete. The fixed template has explicit items and no assembly-controlled tasks, imports, scripts, analyzers or package references. XML and MSBuild expressions are escaped. The report does not claim `build_ready`. Tests compile a downloaded repository-owned arithmetic solution without executing it and verify references between exported projects.

## Save Code

**File > Save Code** or a type/method explorer context action opens source downloads. Save the selected method as C# or IL, a supported type as a `.cs` compilation unit, or a type/assembly inspection source bundle. A method with only annotated IL cannot be downloaded under a misleading `.cs` extension. Partial single-type reconstruction requires the bundle so diagnostics and IL are retained. Inspection bundles keep token-suffixed names; project export uses the conventional layout above.

Generic constraints and declarations have an initial representation, but nested arity, variance, symbol collisions, explicit interface binding, interop and attribute emission still require review. Complex state machines, closures and some constructors/accessors remain IL. Source downloads describe the original assembly bytes; download and reopen a modified assembly before exporting its reconstructed project.

## IL editor

Select a method, open the **IL** tab, and choose **Edit IL** in the code viewer. The instruction table provides opcode and operand inputs, insertion/deletion, paged rows, token search, undo/redo, and separate original/validated IL views. Branch operands refer to labels, and switch operands use comma-separated labels. Retarget branches before deleting an instruction they reference. The encoder widens short branches when necessary.

Integer operands use decimal strings and preserve 64-bit precision. Existing floating-point operands use `bits:0x...` to preserve exact IEEE representations, including NaN payloads and negative zero; decimal input is also accepted. Validation checks the selected opcode's operand range and encoding.

**Validate and apply** validates a draft and stores it in an in-memory overlay. It does not overwrite the input file. Unvalidated drafts are discarded when the editor closes; validated changes remain until discarded or the workspace closes. Source export describes the original bytes. To inspect C# for edited IL, download the modified assembly and reopen it.

The write verifier accepts ordinary nongeneric static methods and instance methods on reference classes. Supported types include primitives, strings, object references, zero-based vectors and jagged arrays. It tracks array element storage types separately from evaluation-stack categories, including small integer widths, floating-point widths and native integers. Null/reference joins and compatible local inheritance/interface relationships are checked by a bounded fixed-point analysis.

Supported instructions include numeric/string operations, branches, switches, array creation/length/load/store, primitive boxing and `unbox.any`, supported reference casts/tests, local instance/static field access, and nongeneric static/instance calls. Property accessors work through their ordinary method and field instructions. `newobj` supports CIL constructors on ordinary local reference classes whose local base chain ends at System.Object; delegate/runtime constructors and unresolved external bases remain blocked; constructor **bodies** still require initialization-state tracking and remain read-only. Methods are checked before opening the instruction editor, and validation errors identify the IL offset and instruction.

Declared signatures can establish external reference types for exact-identity calls/assignments. External inheritance, field flags and constructor properties are not guessed, even when other assemblies are loaded for inspection. Unknown external type kinds produce a specific limitation. Core primitive aliases check reference assembly scope, not just type names. The verifier does not claim complete ECMA-335 or whole-program verification, runtime bounds checking, or proof that an unavailable reference implements its declaration.

Generic methods/types, custom structs/enums, multidimensional/non-zero-based arrays, exception regions, constructor bodies, byref/pointer signatures, indirect memory access, prefixes, uninitialized locals and unreachable blocks remain blocked. Cross-type protected access and explicit-layout fields need additional checks. Existing inspection support is broader than editing support. New metadata strings, local signatures, members, types and references cannot be created. See [IL write verification](il-verification.md) for the precise model and remaining work.

## Modified DLL/EXE output

After applying edits, choose **File > Save Module** (or **Export modified assembly** in the editor) and **Download modified assembly**. The panel lists changed methods and input-level writer diagnostics. The writer reruns validation before writing and before offering the result.

Supported inputs are unsigned x86/x64 IL-only managed PE files with standard alignment and known directories. Strong names, delay signing, Authenticode, mixed/native/ReadyToRun images, unsupported directories, Field-RVA storage, overlays and ambiguous layouts are blocked. Unknown preservation requirements produce errors.

Replacement bodies are written in place when their complete header/body fits and alignment permits it. Growing bodies use a new `.ilens` section when a header slot is available. Otherwise the writer may extend the last fully file-backed `.text`, `.rsrc` or `.reloc` section, only when it is also last in RVA order. That section becomes readable executable code and is no longer discardable. Existing native RVAs remain fixed. MethodDef RVAs, section sizes, image/code sizes and alignment are updated; metadata heaps and tokens do not grow or change.

An explicit internal no-change path preserves bytes exactly. Edited downloads clear stale debug-directory entries **and their payload bytes**, along with checksum/reproducibility timestamps. Supported debug records include CodeView, embedded PDB, PDB checksum and deterministic markers. Unknown or overlapping records block export. Original assembly metadata/content can still contain its own strings; this is not a universal content anonymizer.

The output is reparsed, with identity, entry points, metadata, managed resources, unchanged methods and native imports compared against the input. Tests independently open PE output with goblin and run Microsoft's ILVerify against owned fixtures and controlled reference assemblies. Chromium reopens the downloaded file in a fresh worker and checks the requested IL/C# change. The application never runs constructors, entry points or the output assembly.

## Jobs, memory and cancellation

One source export runs at a time. `beginProjectExport` / `beginExport`, `stepExport`, `finishExport` and `cancelExport` operate on a stable workspace revision. Project jobs process one assembly at a time; each step processes one member, type, resource or dependency and returns to the worker event loop. Closing/loading assemblies or applying/discarding edits cancels the source job. The client serializes requests, applying its 30-second hard watchdog to execution rather than time spent waiting in its queue. Aborting an active call does not disable that watchdog.

ZIP creation uses a dependency-free ZIP32 STORE writer in the worker. Files are uncompressed, with CRC32, fixed 1980-01-01 timestamps and no extra fields/comments. Safe ASCII names and collision suffixes prevent traversal, reserved device names and case/Unicode collisions. Independent Python `zipfile` tests validate the archives. Output buffers are owned JavaScript buffers transferred to the main thread; a live WASM memory view is never transferred. Download Blob URLs are revoked.

| Export/edit limit | Bound |
| --- | --- |
| Archive | 64 MiB, uncompressed STORE format |
| Archive entry / path | 4 MiB / 1,024 bytes |
| Archive entries | 25,000 |
| Retained member serialization accounting | 32 MiB (UTF-16 text accounting) |
| Editable instructions / request JSON | 10,000 / 1 MiB |
| Verifier type identities / CFG stack cells | 4,096 / 262,144 |
| Verifier stack depth / instruction visits | 1,024 / 1,000,000 |
| Validated overlays | 256 methods per workspace |
| Undo snapshots | Up to 40, with 8 MiB serialized-text accounting |
| Modified assembly | 64 MiB |

These accounting caps are not an exact bound on browser heap usage. Object overhead, archive finalization, WASM allocations and Blob storage can increase peak memory. Size-limit failures produce no silently truncated successful download. Large source jobs may require exporting individual types. General large-assembly memory profiling and broader writer compatibility remain ongoing work.

## Verification

Install .NET 10 and Python 3 for the export tests, then restore the pinned test-only IL verifier:

```sh
dotnet tool restore
npm run build:wasm
npm test
npm run typecheck
npm run lint
npm run test:e2e
```

The `.NET` SDK and ILVerify are developer/test tools; they are not shipped in the browser or production image. `NuGet.Config` gives fixture/tool builds a single explicit public source rather than inheriting machine-specific feeds.

Format references: [PKWARE APPNOTE](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT), [ECMA-335](https://ecma-international.org/publications-and-standards/standards/ecma-335/), [Microsoft PE format](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format), and [ILVerify](https://github.com/dotnet/runtime/tree/main/src/coreclr/tools/ILVerify).
