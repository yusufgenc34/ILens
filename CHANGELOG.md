# Changelog

## Unreleased

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
