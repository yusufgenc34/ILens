//! Conservative write validation, separate from the permissive inspection analysis.
//! Supports nongeneric reference-class methods, vectors, primitives and checked object
//! operations. Unsupported value types, generics, EH and managed addresses remain closed.
use crate::{
    assembly::Assembly,
    cfg::ControlFlowGraph,
    cil::{self, MethodBody, Operand},
    cil_encode::{self, EditableInstruction},
    error::{Error, ErrorCode, Result},
};
use serde::Serialize;
use std::collections::VecDeque;
#[derive(Debug, Clone)]
pub struct ValidatedEdit {
    pub instructions: Vec<EditableInstruction>,
    pub code: Vec<u8>,
    pub max_stack: u16,
}
#[derive(Serialize)]
pub struct EditPreview {
    pub token: u32,
    pub revision: u32,
    pub instructions: Vec<EditableInstruction>,
    pub original: Vec<EditableInstruction>,
    pub changed: bool,
    pub il: String,
    pub max_stack: u16,
    pub code_size: usize,
}
mod types;
use types::{Stack, Types, Value};
fn invalid(detail: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidEdit, detail)
}
pub fn unsupported(detail: impl Into<String>) -> Error {
    Error::new(ErrorCode::UnsupportedEdit, detail)
}
fn pop(stack: &mut Vec<Stack>) -> Result<Stack> {
    stack
        .pop()
        .ok_or_else(|| invalid("Evaluation stack underflow"))
}
fn expect(types: &Types<'_>, stack: &mut Vec<Stack>, expected: Stack) -> Result<()> {
    let actual = pop(stack)?;
    if !types.compatible(actual, expected)? {
        return Err(invalid(format!(
            "Expected {}, found {}",
            types.describe(expected),
            types.describe(actual)
        )));
    }
    Ok(())
}
fn index(stack: &mut Vec<Stack>) -> Result<()> {
    if !matches!(pop(stack)?, Stack::I4 | Stack::I) {
        return Err(invalid("Array length/index requires int32 or native int"));
    }
    Ok(())
}
fn object(stack: &mut Vec<Stack>) -> Result<Stack> {
    let value = pop(stack)?;
    if !value.reference() {
        return Err(invalid("Instruction requires an object reference"));
    }
    Ok(value)
}
fn access(owner: u32, caller: u32, flags: u16) -> Result<()> {
    let permitted = match flags & 7 {
        1 => owner == caller,
        // Cross-type protected access also depends on the receiver's tracked type.
        // Keep it closed until that separate accessibility rule is implemented.
        2 | 4 => owner == caller,
        3 | 5 | 6 => true,
        _ => false,
    };
    if !permitted {
        return Err(unsupported(
            "Member accessibility cannot be verified from this declaring type",
        ));
    }
    Ok(())
}
pub fn supported_body(a: &Assembly, token: u32) -> Result<MethodBody> {
    let row = a.method_row(token)?;
    let method = a.method_ref(token)?;
    if method.signature.generic_count != 0 || a.generics.contains_key(&method.owner) {
        return Err(unsupported(
            "Generic methods and generic declaring types require generic-aware IL validation",
        ));
    }
    if method.signature.explicit_this
        || method.signature.vararg_at.is_some()
        || method.signature.convention != 0
        || row.flags & 0x2000 != 0
        || row.impl_flags & 7 != 0
    {
        return Err(unsupported(
            "Only ordinary managed CIL calling conventions can be edited",
        ));
    }
    if method.name.starts_with('.') {
        return Err(unsupported(
            "Constructor bodies and type initializers require initialization-state validation; calling supported constructors with newobj is allowed",
        ));
    }
    if method.signature.has_this == (row.flags & 0x10 != 0) {
        return Err(invalid("Method signature disagrees with its static flag"));
    }
    let mut types = Types::new(a);
    if method.signature.has_this {
        let owner = types.token(method.owner)?;
        if !types.value(owner).reference() {
            return Err(unsupported(
                "Value-type instance methods require managed-address validation",
            ));
        }
    }
    for ty in &method.signature.parameters {
        types.signature(ty)?;
    }
    if !method.signature.return_type.is_void() {
        types.signature(&method.signature.return_type)?;
    }
    let body = a
        .body(token)?
        .ok_or_else(|| unsupported("This member has no CIL body"))?;
    if !body.exceptions.is_empty() {
        return Err(unsupported(
            "Methods with exception handlers are read-only in this release",
        ));
    }
    let locals = a.locals(&body)?;
    if !locals.is_empty() && !body.init_locals {
        return Err(unsupported(
            "Uninitialized local-variable verification is not implemented",
        ));
    }
    for ty in locals {
        types.signature(&ty)?;
    }
    Ok(body)
}
pub fn validate(a: &Assembly, token: u32, rows: &[EditableInstruction]) -> Result<ValidatedEdit> {
    let mut body = supported_body(a, token)?;
    let code = cil_encode::encode(rows)?;
    body.instructions = cil::decode(&code)?;
    body.code_size = code.len() as u32;
    let max_stack = verify(a, token, &body)?;
    Ok(ValidatedEdit {
        instructions: cil_encode::from_body(&body),
        code,
        max_stack,
    })
}
pub fn verify(a: &Assembly, token: u32, body: &MethodBody) -> Result<u16> {
    if !body.exceptions.is_empty() {
        return Err(unsupported(
            "Exception-region edits require handler boundary and stack validation",
        ));
    }
    let method = a.method_ref(token)?;
    let sig = &method.signature;
    let mut types = Types::new(a);
    let mut args = Vec::new();
    if sig.has_this {
        let id = types.token(method.owner)?;
        args.push(types.stack(id));
    }
    for ty in &sig.parameters {
        let id = types.signature(ty)?;
        args.push(types.stack(id));
    }
    let result = if sig.return_type.is_void() {
        None
    } else {
        let id = types.signature(&sig.return_type)?;
        Some(types.stack(id))
    };
    let mut locals = Vec::new();
    for ty in a.locals(body)? {
        let id = types.signature(&ty)?;
        locals.push(types.stack(id));
    }
    let cfg = ControlFlowGraph::build(body)?;
    let mut entries: Vec<Option<Vec<Stack>>> = vec![None; cfg.blocks.len()];
    entries[0] = Some(vec![]);
    let mut queue = VecDeque::from([0]);
    let mut queued = vec![false; cfg.blocks.len()];
    queued[0] = true;
    let mut maximum = 0;
    let mut work = 0;
    let mut cells = 0;
    while let Some(id) = queue.pop_front() {
        queued[id] = false;
        let mut stack = entries[id]
            .clone()
            .ok_or_else(|| invalid("Missing verifier stack"))?;
        let block = &cfg.blocks[id];
        for instruction in &body.instructions[block.first..block.last] {
            work += 1;
            if work > 1_000_000 {
                return Err(Error::limit("IL write verification work limit exceeded"));
            }
            transfer(
                &mut types,
                method.owner,
                sig.has_this,
                &args,
                &locals,
                result,
                &mut stack,
                instruction,
            )
            .map_err(|mut e| {
                e.detail = format!(
                    "IL_{:04X} ({}): {}",
                    instruction.offset, instruction.name, e.detail
                );
                e
            })?;
            maximum = maximum.max(stack.len());
            if maximum > 1024 {
                return Err(invalid("Evaluation stack exceeds 1024 values"));
            }
        }
        for next in &block.successors {
            let changed = if let Some(old) = &mut entries[*next] {
                if old.len() != stack.len() {
                    return Err(invalid(format!(
                        "Incompatible stack heights at IL_{:04X}",
                        cfg.blocks[*next].start
                    )));
                }
                let mut changed = false;
                for (before, after) in old.iter_mut().zip(&stack) {
                    let merged = types.merge(*before, *after)?;
                    if merged != *before {
                        *before = merged;
                        changed = true;
                    }
                }
                changed
            } else {
                cells += stack.len();
                if cells > 262_144 {
                    return Err(Error::limit(
                        "IL validation CFG stack storage exceeds its budget",
                    ));
                }
                entries[*next] = Some(stack.clone());
                true
            };
            if changed && !queued[*next] {
                queue.push_back(*next);
                queued[*next] = true;
            }
        }
    }
    if entries.iter().any(Option::is_none) {
        return Err(unsupported(
            "Unreachable blocks require additional verification; remove them before export",
        ));
    }
    Ok(maximum as u16)
}
#[allow(clippy::too_many_arguments)]
fn transfer(
    types: &mut Types<'_>,
    owner: u32,
    has_this: bool,
    args: &[Stack],
    locals: &[Stack],
    result: Option<Stack>,
    stack: &mut Vec<Stack>,
    i: &cil::Instruction,
) -> Result<()> {
    let var = crate::analysis::variable(i).unwrap_or(usize::MAX);
    match i.opcode {
        0 | 1 => {}
        0x02..=0x05 | 0x0e | 0xfe09 => stack.push(
            *args
                .get(var)
                .ok_or_else(|| invalid("Argument index out of bounds"))?,
        ),
        0x06..=0x09 | 0x11 | 0xfe0c => stack.push(
            *locals
                .get(var)
                .ok_or_else(|| invalid("Local index out of bounds"))?,
        ),
        0x0a..=0x0d | 0x13 | 0xfe0e => expect(
            types,
            stack,
            *locals
                .get(var)
                .ok_or_else(|| invalid("Local index out of bounds"))?,
        )?,
        0x10 | 0xfe0b => {
            if has_this && var == 0 {
                return Err(unsupported("Replacing the this argument is not supported"));
            }
            expect(
                types,
                stack,
                *args
                    .get(var)
                    .ok_or_else(|| invalid("Argument index out of bounds"))?,
            )?;
        }
        0x14 => stack.push(Stack::Null),
        0x15..=0x20 => stack.push(Stack::I4),
        0x21 => stack.push(Stack::I8),
        0x22 | 0x23 => stack.push(Stack::F),
        0x25 => {
            let value = pop(stack)?;
            stack.extend([value, value]);
        }
        0x26 => {
            pop(stack)?;
        }
        0x28 | 0x6f | 0x73 => call(types, owner, stack, i)?,
        0x2a => {
            if let Some(result) = result {
                expect(types, stack, result)?;
            }
            if !stack.is_empty() {
                return Err(invalid("Return leaves values on the evaluation stack"));
            }
        }
        0x2b | 0x38 => {}
        0x2c | 0x2d | 0x39 | 0x3a => {
            if pop(stack)? == Stack::F {
                return Err(invalid(
                    "Floating-point value cannot be used by brtrue/brfalse",
                ));
            }
        }
        0x2e..=0x37 | 0x3b..=0x44 | 0xfe01..=0xfe05 => {
            let right = pop(stack)?;
            let left = pop(stack)?;
            let equality = matches!(i.opcode, 0x2e | 0x33 | 0x3b | 0x40 | 0xfe01);
            // cgt.un is also the C# reference != null idiom.
            let null_test = i.opcode == 0xfe03 && left.reference() && right == Stack::Null;
            if !(left.numeric() && left == right
                || equality && left.reference() && right.reference()
                || null_test)
            {
                return Err(invalid(
                    "Comparison requires compatible numeric values or a supported reference comparison",
                ));
            }
            if i.opcode >= 0xfe01 {
                stack.push(Stack::I4)
            }
        }
        0x45 => expect(types, stack, Stack::I4)?,
        0x58..=0x61 | 0xd6..=0xdb => {
            let right = pop(stack)?;
            let left = pop(stack)?;
            if left != right || !left.numeric() {
                return Err(invalid("Arithmetic requires matching numeric stack types"));
            }
            if matches!(i.opcode, 0x5c | 0x5e | 0x5f..=0x61 | 0xd6..=0xdb) && left == Stack::F {
                return Err(invalid("This arithmetic opcode requires integers"));
            }
            stack.push(left);
        }
        0x62..=0x64 => {
            index(stack)?;
            let left = pop(stack)?;
            if !matches!(left, Stack::I4 | Stack::I8 | Stack::I) {
                return Err(invalid("Shift requires an integer value"));
            }
            stack.push(left);
        }
        0x65 | 0x66 => {
            let value = pop(stack)?;
            if !value.numeric() || i.opcode == 0x66 && value == Stack::F {
                return Err(invalid("Unary opcode has an incompatible type"));
            }
            stack.push(value);
        }
        0x67..=0x6e | 0x76 | 0xd1..=0xd3 | 0xe0 => {
            let value = pop(stack)?;
            if !value.numeric() || i.opcode == 0x76 && value == Stack::F {
                return Err(invalid(
                    "Numeric conversion requires a compatible numeric value",
                ));
            }
            stack.push(match i.opcode {
                0x6a | 0x6e => Stack::I8,
                0x6b | 0x6c | 0x76 => Stack::F,
                0xd3 | 0xe0 => Stack::I,
                _ => Stack::I4,
            });
        }
        0x72 => {
            let token = operand(i)?;
            if token >> 24 != 0x70 {
                return Err(invalid("ldstr requires a user-string token"));
            }
            types.a.user_string(token)?;
            stack.push(Stack::Ref(types.intern(Value::String)?));
        }
        0x74 | 0x75 => {
            object(stack)?;
            let id = types.token(operand(i)?)?;
            let reference = if types.value(id).reference() {
                id
            } else {
                types.intern(Value::Boxed(id))?
            };
            stack.push(Stack::Ref(reference));
        }
        0x7a => {
            object(stack)?;
            stack.clear();
        }
        0x7b | 0x7d | 0x7e | 0x80 => field(types, owner, stack, i)?,
        0x8c => {
            let id = types.token(operand(i)?)?;
            if types.value(id).reference() {
                return Err(unsupported("box on reference types is not supported"));
            }
            expect(types, stack, types.stack(id))?;
            stack.push(Stack::Ref(types.intern(Value::Boxed(id))?));
        }
        0x8d => {
            index(stack)?;
            let element = types.token(operand(i)?)?;
            stack.push(Stack::Ref(types.intern(Value::Array(element))?));
        }
        0x8e => {
            array_element(types, stack)?;
            stack.push(Stack::I);
        }
        0x90..=0xa4 => array(types, stack, i)?,
        0xa5 => {
            object(stack)?;
            let id = types.token(operand(i)?)?;
            stack.push(types.stack(id));
        }
        _ => {
            return Err(unsupported(format!(
                "Write validation for {} is not implemented",
                i.name
            )));
        }
    }
    Ok(())
}
fn operand(i: &cil::Instruction) -> Result<u32> {
    i.token()
        .ok_or_else(|| invalid("Instruction is missing a metadata token"))
}
fn call(
    types: &mut Types<'_>,
    caller: u32,
    stack: &mut Vec<Stack>,
    i: &cil::Instruction,
) -> Result<()> {
    let token = operand(i)?;
    if !matches!(token >> 24, 6 | 10) {
        return Err(unsupported(
            "Only nongeneric MethodDef/MemberRef calls can be edited",
        ));
    }
    types.a.validate_token(token)?;
    let m = types.a.method_ref(token)?;
    let sig = &m.signature;
    if sig.generic_count != 0
        || !m.generic_args.is_empty()
        || sig.explicit_this
        || sig.vararg_at.is_some()
        || sig.convention != 0
    {
        return Err(unsupported(
            "Generic, explicit-this and vararg calls require additional validation",
        ));
    }
    if types.a.generics.contains_key(&m.owner) || m.owner >> 24 == 27 {
        return Err(unsupported(
            "Calls on constructed generic/array types require additional validation",
        ));
    }
    let constructor = i.opcode == 0x73;
    if constructor {
        if m.name != ".ctor" || !sig.has_this || !sig.return_type.is_void() {
            return Err(invalid(
                "newobj requires an instance constructor returning void",
            ));
        }
        // External constructors need their defining metadata to distinguish structs,
        // reference classes, delegates, abstract types and accessibility.
        if token >> 24 != 6 {
            return Err(unsupported(
                "Creating external types requires constructor/type definitions; local reference-class constructors are supported",
            ));
        }
        if types.a.method_row(token)?.impl_flags & 0x1007 != 0 {
            return Err(unsupported(
                "Runtime-provided constructors, including delegates, require specialized validation",
            ));
        }
        types.token(m.owner)?;
        let row = &types.a.metadata().type_defs[(m.owner & 0xffffff) as usize - 1];
        if row.flags & 0xa0 != 0 {
            return Err(invalid(
                "newobj cannot instantiate an abstract class or interface",
            ));
        }
        // Delegate construction has additional function-pointer provenance rules.
        // Only fully local ordinary class chains ending at System.Object qualify.
        let mut parent = crate::assembly::coded(row.extends);
        let mut seen = std::collections::HashSet::from([m.owner]);
        while types.core_name(parent)?.as_deref() != Some("Object") {
            if parent >> 24 != 2 {
                return Err(unsupported(
                    "Constructor base types require resolved ordinary classes; delegate and external-base construction is not supported",
                ));
            }
            if seen.len() >= 64 || !seen.insert(parent) {
                return Err(unsupported(
                    "Constructor inheritance chain is cyclic or too deep",
                ));
            }
            types.token(parent)?;
            parent = crate::assembly::coded(
                types.a.metadata().type_defs[(parent & 0xffffff) as usize - 1].extends,
            );
        }
    } else if m.name.starts_with('.') {
        return Err(unsupported(
            "Direct constructor calls require initialization-state validation",
        ));
    }
    if i.opcode == 0x6f && !sig.has_this {
        return Err(invalid("callvirt requires an instance method"));
    }
    if token >> 24 == 6 {
        let row = types.a.method_row(token)?;
        if sig.has_this == (row.flags & 16 != 0) {
            return Err(invalid("Call signature disagrees with static method flags"));
        }
        if constructor && row.flags & 0x1800 != 0x1800 {
            return Err(invalid("newobj target lacks constructor metadata flags"));
        }
        if i.opcode == 0x28 && row.flags & 0x400 != 0 {
            return Err(invalid("An abstract method requires virtual dispatch"));
        }
        access(m.owner, caller, row.flags)?;
    }
    let mut parameters = Vec::new();
    for p in &sig.parameters {
        let id = types.signature(p)?;
        parameters.push(types.stack(id));
    }
    let result = if sig.return_type.is_void() {
        None
    } else {
        let id = types.signature(&sig.return_type)?;
        Some(types.stack(id))
    };
    for p in parameters.into_iter().rev() {
        expect(types, stack, p)?;
    }
    if sig.has_this {
        let target = types.token(m.owner)?;
        if !types.value(target).reference() {
            return Err(unsupported(
                "Value-type instance calls require managed-address validation",
            ));
        }
        if constructor {
            stack.push(types.stack(target))
        } else {
            expect(types, stack, types.stack(target))?;
        }
    }
    if let Some(result) = result {
        stack.push(result)
    }
    Ok(())
}
fn field(
    types: &mut Types<'_>,
    caller: u32,
    stack: &mut Vec<Stack>,
    i: &cil::Instruction,
) -> Result<()> {
    let token = operand(i)?;
    if token >> 24 != 4 {
        return Err(unsupported(
            "External field edits require resolved field definitions; local fields are supported",
        ));
    }
    types.a.validate_token(token)?;
    let row = &types.a.metadata().fields[(token & 0xffffff) as usize - 1];
    let (owner, _, ty) = types.a.field_ref(token)?;
    types.token(owner)?;
    let declaring = &types.a.metadata().type_defs[(owner & 0xffffff) as usize - 1];
    if declaring.flags & 0x18 == 0x10 {
        return Err(unsupported(
            "Explicit-layout fields require overlap and accessibility analysis",
        ));
    }
    let is_static = matches!(i.opcode, 0x7e | 0x80);
    let store = matches!(i.opcode, 0x7d | 0x80);
    if (row.flags & 16 != 0) != is_static {
        return Err(invalid(
            "Field opcode disagrees with the field's static flag",
        ));
    }
    if row.flags & 0x40 != 0 || store && row.flags & 0x20 != 0 {
        return Err(invalid(
            "Literal/init-only fields cannot be assigned by this method",
        ));
    }
    access(owner, caller, row.flags)?;
    let value = types.signature(&ty)?;
    if store {
        expect(types, stack, types.stack(value))?;
    }
    if !is_static {
        let target = types.token(owner)?;
        if !types.value(target).reference() {
            return Err(unsupported(
                "Value-type fields require managed-address validation",
            ));
        }
        expect(types, stack, types.stack(target))?;
    }
    if !store {
        stack.push(types.stack(value));
    }
    Ok(())
}
fn array_element(types: &Types<'_>, stack: &mut Vec<Stack>) -> Result<usize> {
    let value = pop(stack)?;
    if let Stack::Ref(id) = value
        && let Value::Array(element) = types.value(id)
    {
        return Ok(element);
    }
    Err(invalid(format!(
        "A typed zero-based array is required, found {}",
        types.describe(value)
    )))
}
fn array(types: &mut Types<'_>, stack: &mut Vec<Stack>, i: &cil::Instruction) -> Result<()> {
    let store = matches!(i.opcode, 0x9b..=0xa2 | 0xa4);
    let stored = if store { Some(pop(stack)?) } else { None };
    index(stack)?;
    let element = array_element(types, stack)?;
    let value = types.value(element);
    let valid = match i.opcode {
        0x90 | 0x91 | 0x9c => matches!(value, Value::Bool | Value::I1 | Value::U1),
        0x92 | 0x93 | 0x9d => matches!(value, Value::Char | Value::I2 | Value::U2),
        0x94 | 0x95 | 0x9e => matches!(value, Value::I4 | Value::U4),
        0x96 | 0x9f => matches!(value, Value::I8 | Value::U8),
        0x97 | 0x9b => matches!(value, Value::I | Value::U),
        0x98 | 0xa0 => value == Value::R4,
        0x99 | 0xa1 => value == Value::R8,
        0x9a | 0xa2 => value.reference(),
        0xa3 | 0xa4 => types.token(operand(i)?)? == element,
        _ => false,
    };
    if !valid {
        return Err(invalid(
            "Array opcode/operand does not match the array element type",
        ));
    }
    if let Some(stored) = stored {
        if !types.compatible(stored, types.stack(element))? {
            return Err(invalid(format!(
                "Array element requires {}, found {}",
                types.describe(types.stack(element)),
                types.describe(stored)
            )));
        }
    } else {
        stack.push(types.stack(element));
    }
    Ok(())
}
pub fn preview(
    a: &Assembly,
    token: u32,
    revision: u32,
    edit: Option<&ValidatedEdit>,
) -> Result<EditPreview> {
    let mut body = supported_body(a, token)?;
    let original = cil_encode::from_body(&body);
    if edit.is_none() {
        verify(a, token, &body)?;
    }
    if let Some(edit) = edit {
        body.instructions = cil::decode(&edit.code)?;
        body.code_size = edit.code.len() as u32;
        body.max_stack = edit.max_stack;
    }
    for i in &mut body.instructions {
        if let Operand::Token(t) = i.operand {
            i.resolved = Some(a.resolve(t));
        }
    }
    Ok(EditPreview {
        token,
        revision,
        instructions: cil_encode::from_body(&body),
        original,
        changed: edit.is_some(),
        il: crate::emit::il(a, token, &body),
        max_stack: body.max_stack,
        code_size: body.code_size as usize,
    })
}
