# IL write verification

IL editing uses a separate verifier from the decompiler's permissive inspection analysis. The input assembly is immutable. Successful edits are overlays with a revision, encoded CIL and a computed maximum stack size. Downloading reruns validation, writes the PE, reparses it and compares preserved metadata/resources/methods. No input or output code is executed.

## Types and control flow

`signature.rs` preserves CLASS versus VALUETYPE and SZARRAY versus general ARRAY, including rank-one non-vector arrays. A named struct is not treated as an object reference; a general rank-one array cannot pass as a zero-based vector.

`edit/types.rs` interns storage types separately from stack categories. For example, `byte`, `int` and `bool` all occupy an int32 evaluation-stack slot, but `ldelem.i4` cannot read a byte array. Array element types include their actual storage widths. References retain identity, array shape or boxed primitive type. Interned IDs keep CFG stack snapshots bounded rather than cloning recursively nested signatures into every cell.

`edit.rs` propagates stacks over basic blocks to a fixed point. A null/reference join becomes the reference type. Compatible references widen to the assignable type; otherwise reference joins conservatively widen to object. Changed entries requeue their successors. Widening can make a later operation unsupported or invalid; a successful first visit is not sufficient. Incompatible numeric categories/heights are rejected. Every block must be reachable for this verification domain. Signature resolution, reference comparisons and metadata graph scans also share a one-million-step budget; a nested scan cannot bypass the instruction budget.

Reference assignment accepts exact identities, reference-to-object, reference-array covariance, and explicitly declared local base/interface relationships. Array covariance never turns a primitive array into object[]. Unknown external ancestry is not assumed. Primitive core aliases require a scoped framework assembly reference. Verification still relies on the input's external declarations; it does not load or execute runtime code to prove them.

## Supported expansion

- Nongeneric static methods and initialized instance methods on reference classes, with `this` at argument zero.
- Primitive, string, object and supported named-reference parameters/results/initialized locals.
- Zero-based vectors and jagged arrays: `newarr`, `ldlen`, supported `ldelem`/`stelem` forms, typed indices and element checks.
- Primitive boxing, `unbox.any`, and supported `castclass`/`isinst` targets.
- Nongeneric `call` and `callvirt`, with signature/static-flag checks and typed receivers/arguments/results.
- `newobj` for concrete, non-interface local reference classes with accessible CIL constructor definitions and a fully local base chain ending at System.Object. Delegate/runtime constructors require separate function-pointer validation and remain blocked.
- Local `ldfld`, `stfld`, `ldsfld` and `stsfld`, with receiver/static/accessibility/read-only checks. Properties work through methods and backing fields.
- Numeric operations, strings, branches, switches and returns; `throw` checks an object reference and empties the stack. `ldlen` produces a native integer, which must be converted or used appropriately.

Field writes to literal/init-only fields are rejected. Explicit-layout fields and cross-type protected access remain closed pending their additional verification rules. External call signatures may be used, but external constructor/field definitions and unknown type kinds cannot be inferred from method names. Loading a dependency for navigation does not yet extend the write verifier's definition resolver.

## Remaining work

Generics and constraints, custom structs/enums, managed addresses/byrefs, constructor initialization states, prefixes, exception handler/filter/finally regions, array general shapes and external hierarchy resolution require additional work. They remain explicit limitations. New members, signatures, string tokens and metadata heap growth are outside this writer. There is no unchecked or force-save bypass.

This is a bounded verifier for the supported editing domain, not a complete ECMA-335 verifier. It does not prove runtime behavior, array bounds, successful casts, dependency availability or equivalence to the original program. A legal edit can intentionally change behavior.

## Tests

`crates/decompiler-core/tests/edit_types.rs` covers actual fixture signatures, array operations, class/instance calls, local fields, reference merges and invalid operand/receiver/type cases. Existing export tests cover stale revisions, stack errors, body growth and preserved PE data. `samples/Editing` adds compiled, repository-owned examples with no PDB or host paths; fixture privacy checks include its DLL.

`tests/e2e/expanded-edit.spec.ts` changes MakeArray's constant, rejects an incompatible store opcode, downloads the DLL and reopens its changed C#. It also edits methods covering arrays, element widths, constructors, properties, interfaces, fields, casts and reference joins in one assembly. The downloaded outputs are independently checked using Microsoft's ILVerify. Compilation/verification are developer tests only, with no managed execution.

References: [ECMA-335, partitions I and III](https://ecma-international.org/publications-and-standards/standards/ecma-335/), [Microsoft's ILVerify](https://github.com/dotnet/runtime/tree/main/src/coreclr/tools/ILVerify), and [stelem.ref behavior](https://learn.microsoft.com/en-us/dotnet/api/system.reflection.emit.opcodes.stelem_ref?view=net-10.0).
