# IL write verification

IL editing uses a separate verifier from the decompiler's permissive inspection analysis. The input assembly is immutable. Successful edits are overlays with a revision, encoded CIL and a computed maximum stack size. Downloading reruns validation, writes the PE, reparses it and compares preserved metadata/resources/methods. No input or output code is executed.

## Types and control flow

`signature.rs` preserves CLASS versus VALUETYPE and SZARRAY versus general ARRAY, including rank-one non-vector arrays. A named struct is not treated as an object reference; a general rank-one array cannot pass as a zero-based vector.

`edit/types.rs` interns storage types separately from stack categories. For example, `byte`, `int` and `bool` all occupy an int32 evaluation-stack slot, but `ldelem.i4` cannot read a byte array. Array element types include their actual storage widths. References retain identity, array shape or boxed primitive type. Interned IDs keep CFG stack snapshots bounded rather than cloning recursively nested signatures into every cell.

`edit.rs` propagates stacks over basic blocks to a fixed point. A null/reference join becomes the reference type. Compatible references widen to the assignable type; otherwise reference joins conservatively widen to object. Changed entries requeue their successors. Widening can make a later operation unsupported or invalid; a successful first visit is not sufficient. Incompatible numeric categories/heights are rejected. Every block must be reachable for this verification domain. Signature resolution, reference comparisons and metadata graph scans also share a one-million-step budget; a nested scan cannot bypass the instruction budget.

Reference assignment accepts exact identities, reference-to-object, reference-array covariance, and explicitly declared local base/interface relationships. Array covariance never turns a primitive array into object[]. Unknown external ancestry is not assumed. Primitive core aliases require a scoped framework assembly reference. Verification still relies on the input's external declarations; it does not load or execute runtime code to prove them.

## Supported expansion

- Static and instance methods on classes and local structs; struct instance methods receive `this` as a managed pointer.
- Instance constructors track an uninitialized `this` until their `base(...)`/`this(...)` call: only stores into the type's own fields may precede it, every path must make it before `ret`, and paths reaching a block must agree. Struct constructors and type initializers are ordinary bodies. Init-only fields may be assigned by their own type's constructor or initializer.
- Generic methods and members of generic types: type parameters are opaque values (references under `where T : class`) that must be boxed to become objects; MethodSpec calls and members of instantiated types verify against substituted signatures; fields of local generic types are reached through their instantiation.
- Primitive, string, object, class, struct, enum, vector, multidimensional array and `ref T` parameters, results and locals. Edits may declare new locals; methods returning managed pointers stay closed.
- Managed pointers: `ldloca`/`ldarga`/`ldflda` produce them, `ldind`/`stind` widths must match the pointed-to storage, `ldobj`/`stobj`/`initobj` need a pointer to exactly their type, struct fields are reached through a pointer (loads also through the value), and `new S(...)` may construct in place on an address. Pointers merge only with identical pointers and cannot be stored in fields, arrays or boxes.
- Vectors, jagged and multidimensional arrays (runtime `Get`/`Set`/`Address` and constructors, one int32 index per dimension).
- Boxing, `unbox.any`, `castclass`/`isinst`; `call`/`callvirt`/`newobj` with signature, static-flag, receiver and accessibility checks; `newobj` for concrete local classes and structs, and for external ones when their assembly is loaded.
- Exception regions (catch, finally, fault): ranges on instruction boundaries and disjoint or nested; protected ranges entered at their first instruction with an empty stack; `leave` exits enclosing ranges but never a finally/fault handler; `endfinally` only in finally/fault handlers; `rethrow` only in catch handlers; no `ret` inside a range. Catch handlers start with the caught exception, others with an empty stack.
- New user-string literals (`ldstr "text"`) through placeholder tokens that the writer replaces after appending them to #US.
- Accessibility (ECMA-335 I.8.5.3): nested types see their enclosing types' private members; family members are visible to derived types, instance members only through a receiver of the accessing type. Members of other assemblies are visible when public, or family from derived types.

With dependencies loaded, the session resolves the assembly's TypeRefs and MemberRefs against them: external classes, interfaces, structs and enums (with their underlying type), base types and interfaces for assignability, member flags for accessibility and external fields. Without the dependency, nothing about an external type is inferred from its name.

## Remaining work

Filter clauses, explicit-layout and generic structs, methods returning managed pointers, unmanaged and function pointers, prefixes (`constrained.`, `volatile.`, `tail.`) and exception regions inside instance constructors remain explicit limitations. New types and references cannot be created; new fields and methods can, with token renumbering. There is no unchecked or force-save bypass for IL edits; raw hex patches are a separate, explicitly unverified path.

This is a bounded verifier for the supported editing domain, not a complete ECMA-335 verifier. It does not prove runtime behavior, array bounds, successful casts, dependency availability or equivalence to the original program. A legal edit can intentionally change behavior.

## Tests

`crates/decompiler-core/tests/edit_types.rs` covers actual fixture signatures, array operations, class/instance calls, local fields, reference merges and invalid operand/receiver/type cases. Existing export tests cover stale revisions, stack errors, body growth and preserved PE data. `samples/Editing` adds compiled, repository-owned examples with no PDB or host paths; fixture privacy checks include its DLL.

`tests/e2e/expanded-edit.spec.ts` changes MakeArray's constant, rejects an incompatible store opcode, downloads the DLL and reopens its changed C#. It also edits methods covering arrays, element widths, constructors, properties, interfaces, fields, casts and reference joins in one assembly. The downloaded outputs are independently checked using Microsoft's ILVerify. Compilation/verification are developer tests only, with no managed execution.

References: [ECMA-335, partitions I and III](https://ecma-international.org/publications-and-standards/standards/ecma-335/), [Microsoft's ILVerify](https://github.com/dotnet/runtime/tree/main/src/coreclr/tools/ILVerify), and [stelem.ref behavior](https://learn.microsoft.com/en-us/dotnet/api/system.reflection.emit.opcodes.stelem_ref?view=net-10.0).

### Local enum editing

Local nongeneric enums are admitted only after checking their scoped System.Enum base, sealed value-type flags and single integral `value__` storage field. The verifier retains enum identity for boxing and typed tokens while checking byte/short/int/long array opcodes against the underlying storage width. Local structs and, with their assembly loaded, external enums and structs are supported as described above. `samples/Workspace` covers enum parameters, returns, array loads/stores and boxing/unboxing; Rust rejects a mismatched element-width edit, and browser tests statically verify the downloaded assembly using ILVerify and reopen it.
