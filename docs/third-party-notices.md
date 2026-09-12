# Third-party notices

## .NET runtime opcode definitions

`crates/decompiler-core/src/opcodes.rs` is generated from [`dotnet/runtime/src/coreclr/inc/opcode.def`](https://github.com/dotnet/runtime/blob/main/src/coreclr/inc/opcode.def), retrieved 2026-09-12. The resulting bounded lookup table is checked in; builds do not download or regenerate it. Only standard encoded opcodes are emitted; reserved/internal pseudo-opcodes are excluded.

Licensed to the .NET Foundation under one or more agreements. The .NET Foundation licenses this material under the MIT license.

Copyright (c) .NET Foundation and Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Dependencies and template

Cargo.lock and package-lock.json identify the dependency versions. Dependencies retain their upstream licenses in their distributions. Primary dependencies include clrmeta (MIT), goblin (MIT), wasm-bindgen (MIT/Apache-2.0), serde (MIT/Apache-2.0), Rari (MIT), React (MIT), CodeMirror (MIT), and Lucide (ISC).

The application was scaffolded with create-rari-app 0.5.28, using its default template (MIT, Ryan Skinner). Its app-router layout, entry HTML, TypeScript configuration, and Vite/Rari integration guided this project's configuration. Application screens and decompiler logic are original implementations.

No Mono runtime or mono-wasm source code is included. See [mono-wasm-notes.md](mono-wasm-notes.md).
