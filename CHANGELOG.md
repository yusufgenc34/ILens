# Changelog

## Unreleased

- C# views now read like dnSpy/ILSpy: usings and short names, token comments, records and `with`, async/iterator methods (including try/finally iterators and `IAsyncEnumerable`), cached lambdas and local functions, `?:`/`??`/`?.`, type patterns, string `switch`, `lock`, interpolated strings, object/collection initializers, definite assignment and multidimensional arrays. Every sample's exported project compiles, checked by a browser test.
- IL editing now covers constructors and type initializers, generic methods and members of generic types, local structs, ref/out parameters and managed pointers, multidimensional arrays, try/catch/finally/fault regions, nested and protected access, and external types and members from loaded dependencies.
- Edits can add `ldstr` string literals and locals; the writer grows #US/#Blob, re-encodes metadata tables and relocates the metadata, then verifies every original row and heap byte.
- Added Add Field and Add Method, which renumber field/method/parameter tokens across all tables, method bodies and the entry point, and verify every member and body afterwards.
- Added Edit Method (C#): Roslyn compiled to WebAssembly runs in the browser, the compiled body is merged into the assembly with new references appended, then verified like an IL edit. `npm run build:compiler` builds the component.
- Added a whole-file hex editor with checked raw patches, a metadata table viewer, a docked Search window, and settings for token comments, Explorer tokens and compiler-generated members.
- Added `npm run test:e2e:firefox`, which runs Firefox in Docker; it found and fixed two Firefox-only viewer bugs.

- Fixed managed EXEs being rejected when their PE directory count omits a physically present CLR entry. Validated recovery supports read-only metadata and IL inspection.
- Improved native EXE diagnostics, including passive Delphi/C++Builder VCL markers, without assuming every native executable has a companion DLL.
- Reworked Explorer around assembly/module/namespace/type/member nodes, with direct member signatures, nested property/event accessors, and keyboard navigation.
- Added read-only inspection of preserved CIL in ReadyToRun images and automatic local companion-DLL opening for EXEs without CLR metadata. Unsupported native formats now explain the next step.
- Added automatic dependency resolution from a user-selected folder.
- Added multi-assembly Explorer roots and reference navigation, with automatic Microsoft .NET reference-pack downloads when EXE/DLL files open.
- Marked reference-only assemblies as API metadata and prevented placeholder implementation reconstruction and IL editing.
- Improved external property reconstruction using loaded dependency metadata.
- Added C# go-to-definition, document tabs, navigation history and workspace bookmarks.
- Added cross-assembly static reference analysis and embedded resource previews/downloads.
- Expanded validated IL editing to supported local enums.

- Added GitHub Pages hosting with the same explorer, decompiler, export, and IL editor.
- Added automatic static-site deployment and browser checks for repository subpaths.

## 2026-09-13

- Added project export with Visual Studio solutions, C# projects, resources, and dependency references.
- Added validated IL editing with undo/redo and modified DLL/EXE downloads for supported methods.
- Improved C# readability with fewer temporary variables, clearer array initializers, and fewer redundant labels.
- Refined top-menu navigation and workspace styling.

Editing and reconstruction remain scoped; see [support details](docs/export-edit.md). Tracked in [#1](https://github.com/yusufgenc34/ILens/issues/1).
