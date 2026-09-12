# Framework and obfuscation inspection

Opening an assembly runs a bounded metadata inspection in `decompiler-core/src/inspection.rs`, after declaration indexing, inside the existing WASM worker. Results travel with `AssemblyInfo`; selecting a method does not repeat the scan. No constructor, attribute accessor, or method body is executed. Dependency assemblies each get their own report, shown when selected as the primary assembly.

## .NET target version

The inspector reads the applied assembly-level `System.Runtime.Versioning.TargetFrameworkAttribute(string)` constructor argument from its custom-attribute blob. It validates the constructor signature, ECMA-335 prolog, compressed SerString length, UTF-8, named string arguments, and complete blob consumption. A framework moniker is presented as `.NET Framework 4.8`, `.NET Core 3.1`, `.NET 10.0`, `.NET Standard 2.0`, etc.; profile information is preserved. The raw moniker and source remain available in the assembly Metadata view and the target's tooltip.

The metadata root's `v4.0.30319` string is shown separately as **CLR metadata**. It is not evidence that an assembly targets .NET Framework 4: modern .NET assemblies commonly contain the same metadata version. Assembly identity version is a third, independent value. The application reports **Not declared** when the target attribute is absent, instead of inventing an exact version from references. Malformed, conflicting, or scan-limited attributes produce diagnostics. This is the declared build target, not the installed runtime, minimum supported OS, or a guarantee of binary compatibility.

Primary references: [TargetFrameworkAttribute contract](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.versioning.targetframeworkattribute?view=net-10.0), [.NET implementation](https://source.dot.net/System.Private.CoreLib/src/runtime/src/libraries/System.Private.CoreLib/src/System/Runtime/Versioning/TargetFrameworkAttribute.cs.html).

## Obfuscation warnings

Warnings distinguish **metadata markers** from **heuristics**. A recognized attribute must actually be applied to the assembly or module; an unused marker type or a matching string is not enough. Current attribute rules:

| Tool family | Applied attribute | Evidence source |
| --- | --- | --- |
| ConfuserEx / Confuser family | `ConfusedByAttribute`; a serialized value beginning with `ConfuserEx` identifies the Ex family marker | [ConfuserEx watermark creation](https://github.com/yck1509/ConfuserEx/blob/master/Confuser.Core/ConfuserEngine.cs) |
| Dotfuscator | `DotfuscatorAttribute` | [PreEmptive's marker documentation](https://support.preemptive.com/hc/en-us/articles/31855725592081-DotfuscatorAttribute) |
| SmartAssembly | `SmartAssembly.Attributes.PoweredByAttribute` | [de4dot's SmartAssembly detector](https://github.com/de4dot/de4dot/blob/master/de4dot.code/deobfuscators/SmartAssembly/Deobfuscator.cs) |
| Babel | `BabelAttribute` or `BabelObfuscatorAttribute` | [de4dot's Babel detector](https://github.com/de4dot/de4dot/blob/master/de4dot.code/deobfuscators/Babel_NET/Deobfuscator.cs) |

These are independently implemented metadata-name checks; no code from those projects is included. The report displays the matched attribute, its metadata token, and a bounded string argument when valid. **Inspect** opens the actual custom-attribute row, constructor, parent token, and raw blob. A marker's presence is observable; the claim that a particular obfuscator actually processed the file is not authenticated. Markers can be removed, renamed, or imitated. Only exact names listed above are recognized; variants and other products can be missed.

Two conservative naming heuristics also produce a **Possible obfuscation** warning without naming a tool:

- Declaration names contain control characters, selected invisible format characters, or bidirectional formatting controls.
- At least 50 eligible declarations exist and at least 80% use one- or two-letter ASCII names. Constructors, common accessors, and common compiler-generated name prefixes are excluded. This threshold intentionally misses many renamed assemblies and can still flag legitimate compact naming conventions.

Ordinary non-Latin names and `System.Reflection.ObfuscationAttribute` configuration are not treated as proof of obfuscation. **No markers found** never means “not obfuscated.” This is not a universal detector, deobfuscator, unpacker, or method-decryption engine. IL remains available for bodies that parse, and unsupported C# reconstruction continues to use annotated IL.

The scan resolves at most 4,096 assembly/module attributes, caches constructor descriptors, limits inspected string attributes to 16 KiB, individual strings to 4,096 bytes, and named arguments to 16. Evidence is bounded and name analysis uses the existing bounded declaration index. Scan limits or unreadable constructors are reported as incomplete inspection, not a clean result.

## Verification

Rust tests cover framework families/versions, real compiled target attributes, truncated/malformed attribute data, applied markers, ignored unused marker types/configuration, missing targets, suspicious Unicode names, and heuristic thresholds. Playwright loads the real compiled DLLs through the worker, verifies warnings and .NET target values, follows evidence tokens, and switches to an unmarked dependency to ensure stale warnings disappear.

`ILens.InspectionMarkers.dll` is a **synthetic metadata fixture** compiled from `samples/InspectionMarkers/Markers.cs`, not a DLL actually protected by those tools. Its intentionally labeled marker values test detection rules without claiming deobfuscation coverage. `ILens.NoTarget.dll` compiles the same definitions without applied markers or a generated target-framework attribute. Neither fixture is executed. Regenerate them with `npm run fixtures` (.NET 10 SDK).
