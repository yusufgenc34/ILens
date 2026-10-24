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
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[derive(Debug, Clone)]
pub struct ValidatedEdit {
    pub instructions: Vec<EditableInstruction>,
    pub code: Vec<u8>,
    pub max_stack: u16,
    /// New `ldstr` literals; `code` addresses them with placeholder tokens.
    pub strings: Vec<String>,
    /// Replacement local variable types, or None to keep the method's signature.
    pub locals: Option<Vec<crate::signature::Type>>,
    /// Exception regions by instruction label, as validated.
    pub regions: Vec<EditRegion>,
    /// The same regions as code offsets, for the method's EH section.
    pub exceptions: Vec<cil::ExceptionRegion>,
}
/// An exception clause in the editor: labels name the first instruction of a range and
/// the first one after it ("" for the end of the method).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRegion {
    pub kind: String,
    pub try_start: String,
    pub try_end: String,
    pub handler_start: String,
    pub handler_end: String,
    #[serde(default)]
    pub catch_type: Option<String>,
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
    /// Local variable types, as the editor shows and accepts them.
    pub locals: Vec<String>,
    pub original_locals: Vec<String>,
    pub regions: Vec<EditRegion>,
    pub original_regions: Vec<EditRegion>,
}
mod types;
use types::{Stack, Types, Value};

/// What a loaded dependency defines for an external type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalKind {
    Class {
        abstract_: bool,
        delegate: bool,
    },
    Interface,
    Struct,
    /// An enum and the C# keyword of its underlying type.
    Enum(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalType {
    pub kind: ExternalKind,
    /// Full names of its base types and interfaces, transitively.
    pub bases: Vec<String>,
}
/// Definitions of an assembly's TypeRefs and MemberRefs (member flags) in loaded
/// dependencies. Without them, external types stay limited to what signatures prove.
#[derive(Debug, Clone, Default)]
pub struct Externals {
    pub types: std::collections::HashMap<u32, ExternalType>,
    pub members: std::collections::HashMap<u32, u16>,
}
thread_local! {
    static EXTERNALS: std::cell::RefCell<std::rc::Rc<Externals>> = std::cell::RefCell::new(std::rc::Rc::default());
}
/// Runs `f` with external definitions visible to validation.
pub fn with_externals<T>(externals: Externals, f: impl FnOnce() -> T) -> T {
    let previous = EXTERNALS.with(|e| e.replace(std::rc::Rc::new(externals)));
    let result = f();
    EXTERNALS.with(|e| *e.borrow_mut() = previous);
    result
}
pub(crate) fn externals() -> std::rc::Rc<Externals> {
    EXTERNALS.with(|e| e.borrow().clone())
}
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
/// Member accessibility for a member of `owner` used from `caller` (ECMA-335 I.8.5.3):
/// private members are visible to the declaring type and the types nested in it;
/// family members also to derived types, for instance members only through a
/// receiver of the accessing type. `receiver` is None for static members.
fn access(
    types: &Types<'_>,
    owner: u32,
    caller: u32,
    flags: u16,
    receiver: Option<Stack>,
) -> Result<()> {
    // The caller and the types enclosing it.
    let mut scopes = vec![caller];
    while scopes.len() < 64 {
        let current = *scopes.last().expect("non-empty");
        match types
            .a
            .nodes
            .iter()
            .find(|n| n.token == current)
            .map(|n| n.parent)
            .filter(|p| p >> 24 == 2)
        {
            Some(outer) => scopes.push(outer),
            None => break,
        }
    }
    let nested = scopes.contains(&owner);
    let family = || -> Result<bool> {
        for scope in &scopes {
            if !types.derives(*scope, owner)? {
                continue;
            }
            let through = match receiver {
                None | Some(Stack::This) => true,
                Some(Stack::Ref(id)) => match types.value(id) {
                    Value::Class(t) => types.derives(t, *scope)?,
                    _ => false,
                },
                Some(_) => false,
            };
            if through {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let permitted = match flags & 7 {
        1 => nested,
        2 | 4 => nested || family()?,
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
    if a.image.recovered_cli_directory {
        return Err(unsupported(
            "Assemblies with a recovered CLR directory are read-only; their nonstandard PE layout is not supported by the writer.",
        ));
    }
    if a.image.ready_to_run {
        return Err(unsupported(
            "ReadyToRun images are read-only; their precompiled native code cannot be updated by the IL writer.",
        ));
    }
    let row = a.method_row(token)?;
    let method = a.method_ref(token)?;
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
    if method.name.starts_with('.') && !matches!(method.name.as_str(), ".ctor" | ".cctor") {
        return Err(unsupported("Special-name runtime methods are not editable"));
    }
    if method.signature.has_this == (row.flags & 0x10 != 0) {
        return Err(invalid("Method signature disagrees with its static flag"));
    }
    let mut types = Types::new(a);
    types.context = (token, method.owner);
    if method.signature.has_this {
        let owner = types.token(method.owner)?;
        if !types.value(owner).reference() && !matches!(types.value(owner), Value::Struct(_)) {
            return Err(unsupported(
                "Instance methods of enums and generic structs are not editable yet",
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
    if body.exceptions.iter().any(|e| e.kind == "filter") {
        return Err(unsupported(
            "Methods with filter clauses are read-only: filters run user code during the first exception pass",
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
    validate_with_locals(a, token, rows, None)
}
/// Validates instructions, optionally with a new list of local variable types.
pub fn validate_with_locals(
    a: &Assembly,
    token: u32,
    rows: &[EditableInstruction],
    locals: Option<&[String]>,
) -> Result<ValidatedEdit> {
    validate_request(a, token, rows, locals, None)
}
/// Validates instructions with optional new local types and exception regions.
pub fn validate_request(
    a: &Assembly,
    token: u32,
    rows: &[EditableInstruction],
    locals: Option<&[String]>,
    regions: Option<&[EditRegion]>,
) -> Result<ValidatedEdit> {
    if a.image.ready_to_run {
        return Err(unsupported(
            "ReadyToRun images are read-only; their precompiled native code cannot be updated by the IL writer.",
        ));
    }
    if a.info.reference_assembly {
        return Err(unsupported(
            "Reference assemblies contain API declarations, not editable implementations.",
        ));
    }
    let locals = match locals {
        Some(texts) => {
            if texts.len() > 1024 {
                return Err(Error::limit(
                    "A method can declare at most 1,024 edited locals",
                ));
            }
            Some(
                texts
                    .iter()
                    .map(|t| local_type(a, t))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        None => None,
    };
    validate_types(a, token, rows, locals, regions)
}
/// Validation with already parsed local types (the writer revalidates this way).
pub fn validate_types(
    a: &Assembly,
    token: u32,
    rows: &[EditableInstruction],
    locals: Option<Vec<crate::signature::Type>>,
    regions: Option<&[EditRegion]>,
) -> Result<ValidatedEdit> {
    let mut body = supported_body(a, token)?;
    // Without explicit regions, the method's own clauses keep their labels.
    let regions = match regions {
        Some(regions) => regions.to_vec(),
        None => editable_regions(&body),
    };
    let (rows, strings) = string_literals(a, rows)?;
    let code = cil_encode::encode(&rows)?;
    body.instructions = cil::decode(&code)?;
    body.code_size = code.len() as u32;
    body.exceptions = resolve_regions(&rows, &body, &regions)?;
    let original_locals = a.locals(&body)?;
    let locals = locals.filter(|l| *l != original_locals);
    let max_stack = verify_with(a, token, &body, strings.len(), locals.as_deref())?;
    let mut instructions = cil_encode::from_body(&body);
    show_literals(&mut instructions, &strings);
    Ok(ValidatedEdit {
        instructions,
        code,
        max_stack,
        strings,
        locals,
        regions,
        exceptions: body.exceptions,
    })
}
/// A method's clauses with `IL_XXXX` labels, as `cil_encode::from_body` names rows.
fn editable_regions(body: &MethodBody) -> Vec<EditRegion> {
    let label = |offset: u32| {
        if offset >= body.code_size {
            String::new()
        } else {
            format!("IL_{offset:04X}")
        }
    };
    body.exceptions
        .iter()
        .map(|e| EditRegion {
            kind: e.kind.clone(),
            try_start: label(e.try_start),
            try_end: label(e.try_end),
            handler_start: label(e.handler_start),
            handler_end: label(e.handler_end),
            catch_type: e.catch_type.map(|t| format!("0x{t:08X}")),
        })
        .collect()
}
/// Region labels as offsets in the encoded body.
fn resolve_regions(
    rows: &[EditableInstruction],
    body: &MethodBody,
    regions: &[EditRegion],
) -> Result<Vec<cil::ExceptionRegion>> {
    if regions.len() > 256 {
        return Err(Error::limit(
            "A method can have at most 256 exception clauses",
        ));
    }
    let offsets: std::collections::HashMap<&str, u32> = rows
        .iter()
        .zip(&body.instructions)
        .map(|(row, i)| (row.id.as_str(), i.offset))
        .collect();
    let at = |label: &str| -> Result<u32> {
        if label.trim().is_empty() {
            return Ok(body.code_size);
        }
        offsets.get(label.trim()).copied().ok_or_else(|| {
            invalid(format!(
                "Exception region label {label} names no instruction; keep region boundaries when deleting instructions"
            ))
        })
    };
    regions
        .iter()
        .map(|r| {
            let catch_type = match r.kind.as_str() {
                "catch" => {
                    let text = r.catch_type.as_deref().unwrap_or("").trim();
                    let token = text
                        .strip_prefix("0x")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .ok_or_else(|| invalid("A catch clause needs a type token such as 0x01000005"))?;
                    Some(token)
                }
                "finally" | "fault" => None,
                "filter" => {
                    return Err(unsupported(
                        "Filter clauses run user code during the first exception pass; they remain read-only",
                    ));
                }
                other => return Err(invalid(format!("Unknown exception clause kind: {other}"))),
            };
            Ok(cil::ExceptionRegion {
                kind: r.kind.clone(),
                try_start: at(&r.try_start)?,
                try_end: at(&r.try_end)?,
                handler_start: at(&r.handler_start)?,
                handler_end: at(&r.handler_end)?,
                catch_type,
                filter_start: None,
            })
        })
        .collect()
}
/// A protected range or handler that contains an instruction, by its offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Try(u32, u32),
    Handler(u32, u32, usize),
}
impl Part {
    fn range(self) -> (u32, u32) {
        match self {
            Part::Try(s, e) | Part::Handler(s, e, _) => (s, e),
        }
    }
}
/// The ranges containing `offset`, outermost first. Clauses sharing a try range share
/// its part.
fn region_path(exceptions: &[cil::ExceptionRegion], offset: u32) -> Vec<Part> {
    let mut parts: Vec<Part> = vec![];
    for (i, e) in exceptions.iter().enumerate() {
        if (e.try_start..e.try_end).contains(&offset)
            && !parts.contains(&Part::Try(e.try_start, e.try_end))
        {
            parts.push(Part::Try(e.try_start, e.try_end));
        }
        if (e.handler_start..e.handler_end).contains(&offset) {
            parts.push(Part::Handler(e.handler_start, e.handler_end, i));
        }
    }
    parts.sort_by_key(|p| {
        let (s, e) = p.range();
        (std::cmp::Reverse(e - s), s)
    });
    parts
}
/// ECMA-335 II.19 layout: ranges on instruction boundaries, handlers outside their try,
/// and any two ranges disjoint or nested.
fn check_region_layout(body: &MethodBody) -> Result<()> {
    let starts: std::collections::HashSet<u32> = body
        .instructions
        .iter()
        .map(|i| i.offset)
        .chain([body.code_size])
        .collect();
    let mut ranges = vec![];
    for e in &body.exceptions {
        for (s, end) in [(e.try_start, e.try_end), (e.handler_start, e.handler_end)] {
            if s >= end || !starts.contains(&s) || !starts.contains(&end) {
                return Err(invalid(
                    "Exception ranges must be non-empty and start and end on instructions",
                ));
            }
        }
        if e.handler_start < e.try_end && e.try_start < e.handler_end {
            return Err(invalid("A handler cannot overlap its protected range"));
        }
        ranges.push((e.try_start, e.try_end));
        ranges.push((e.handler_start, e.handler_end));
    }
    for (i, a) in ranges.iter().enumerate() {
        for b in &ranges[i + 1..] {
            let disjoint = a.1 <= b.0 || b.1 <= a.0;
            let nested = a.0 <= b.0 && b.1 <= a.1 || b.0 <= a.0 && a.1 <= b.1;
            if !disjoint && !nested {
                return Err(invalid("Exception ranges must be disjoint or nested"));
            }
        }
    }
    Ok(())
}
/// A local variable type as written in the editor: a C# keyword (`int`, `string`), a
/// TypeDef/TypeRef token (`0x02000003`) or a local type's full name, with `[]` for
/// vectors.
fn local_type(a: &Assembly, text: &str) -> Result<crate::signature::Type> {
    use crate::signature::Type;
    let text = text.trim();
    if let Some(element) = text.strip_suffix("[]") {
        return Ok(Type::Array(Box::new(local_type(a, element)?), 1));
    }
    const KEYWORDS: [&str; 16] = [
        "bool", "char", "sbyte", "byte", "short", "ushort", "int", "uint", "long", "ulong",
        "float", "double", "string", "object", "nint", "nuint",
    ];
    if KEYWORDS.contains(&text) {
        return Ok(Type::primitive(text));
    }
    let token = match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16)
            .map_err(|_| invalid(format!("Invalid local type token: {text}")))?,
        None => a
            .nodes
            .iter()
            .find(|n| n.token >> 24 == 2 && a.resolve_full(n.token) == text)
            .map(|n| n.token)
            .ok_or_else(|| invalid(format!("Unknown local type: {text}")))?,
    };
    let row = (token & 0xff_ffff) as usize;
    let exists = match token >> 24 {
        1 => (1..=a.metadata().type_refs.len()).contains(&row),
        2 => (1..=a.metadata().type_defs.len()).contains(&row),
        _ => false,
    };
    if !exists {
        return Err(invalid(
            "Local types must be existing TypeDef or TypeRef tokens",
        ));
    }
    // Value types are written as such; the verifier decides what it supports.
    let value_type = a
        .nodes
        .iter()
        .any(|n| n.token == token && matches!(n.kind.as_str(), "struct" | "enum"));
    Ok(if value_type {
        Type::ValueType(token)
    } else {
        Type::Named(token)
    })
}
/// `ldstr "text"` operands: an existing #US entry keeps its token, a new literal gets a
/// placeholder the writer replaces after appending it to the heap.
fn string_literals(
    a: &Assembly,
    rows: &[EditableInstruction],
) -> Result<(Vec<EditableInstruction>, Vec<String>)> {
    let heap = a.metadata().user_strings.data();
    let mut strings: Vec<String> = vec![];
    let mut total = 0;
    let rows = rows
        .iter()
        .map(|row| {
            let literal = row.operand.trim();
            if row.opcode != "ldstr" || !literal.starts_with('"') {
                return Ok(row.clone());
            }
            let text: String = serde_json::from_str(literal)
                .map_err(|_| invalid("ldstr expects a token or a JSON string literal"))?;
            let token = match crate::metadata_write::find_user_string(heap, &text) {
                Some(token) => token,
                None => {
                    let index = match strings.iter().position(|s| *s == text) {
                        Some(index) => index,
                        None => {
                            total += text.len();
                            if strings.len() >= 4096 || total > 1024 * 1024 {
                                return Err(Error::limit(
                                    "An edit can add at most 4,096 strings and 1 MiB of text",
                                ));
                            }
                            strings.push(text);
                            strings.len() - 1
                        }
                    };
                    crate::metadata_write::PENDING_STRING + index as u32
                }
            };
            Ok(EditableInstruction {
                operand: format!("0x{token:08X}"),
                ..row.clone()
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((rows, strings))
}
/// New literals read as their text in the editor, not as placeholder tokens.
fn show_literals(instructions: &mut [EditableInstruction], strings: &[String]) {
    for i in instructions {
        if let Some(text) = pending_literal(&i.operand, strings) {
            i.operand = text;
        }
    }
}
fn pending_literal(operand: &str, strings: &[String]) -> Option<String> {
    let token = u32::from_str_radix(operand.strip_prefix("0x")?, 16).ok()?;
    let index = token.checked_sub(crate::metadata_write::PENDING_STRING)? as usize;
    strings
        .get(index)
        .and_then(|s| serde_json::to_string(s).ok())
}
pub fn verify(a: &Assembly, token: u32, body: &MethodBody) -> Result<u16> {
    verify_with(a, token, body, 0, None)
}
/// Verifies a body that may address `pending_strings` new literals by placeholder and
/// may declare `locals` in place of its signature's.
pub fn verify_with(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    pending_strings: usize,
    locals: Option<&[crate::signature::Type]>,
) -> Result<u16> {
    check_region_layout(body)?;
    let method = a.method_ref(token)?;
    let sig = &method.signature;
    let mut types = Types::new(a);
    types.context = (token, method.owner);
    types.pending_strings = pending_strings;
    types.initializer = match method.name.as_str() {
        ".ctor" => Some(false),
        ".cctor" => Some(true),
        _ => None,
    };
    let owner_id = types.token(method.owner)?;
    let constructing =
        types.initializer == Some(false) && !matches!(types.value(owner_id), Value::Struct(_));
    if !constructing && types.initializer == Some(false) {
        // A struct constructor receives an initialized `this` pointer.
        types.initializer = None;
    }
    if constructing && !body.exceptions.is_empty() {
        return Err(unsupported(
            "Exception regions in instance constructors are not editable yet",
        ));
    }
    let mut args = Vec::new();
    if sig.has_this {
        let id = types.token(method.owner)?;
        // A struct's instance methods receive `this` as a managed pointer.
        args.push(if matches!(types.value(id), Value::Struct(_)) {
            Stack::Pointer(id)
        } else {
            types.stack(id)
        });
        types.arg_types.push(id);
    }
    for ty in &sig.parameters {
        let id = types.signature(ty)?;
        args.push(types.stack(id));
        types.arg_types.push(id);
    }
    let result = if sig.return_type.is_void() {
        None
    } else {
        let id = types.signature(&sig.return_type)?;
        if matches!(types.stack(id), Stack::Pointer(_)) {
            return Err(unsupported(
                "Methods returning managed pointers are not verified",
            ));
        }
        Some(types.stack(id))
    };
    let declared = match locals {
        Some(locals) => locals.to_vec(),
        None => a.locals(body)?,
    };
    let mut locals = Vec::new();
    for ty in &declared {
        let id = types.signature(ty)?;
        locals.push(types.stack(id));
        types.local_types.push(id);
    }
    let cfg = ControlFlowGraph::build(body)?;
    let mut entries: Vec<Option<Vec<Stack>>> = vec![None; cfg.blocks.len()];
    entries[0] = Some(vec![]);
    // Whether `this` is initialized on entry to each block (constructors only).
    let mut initialized: Vec<Option<bool>> = vec![None; cfg.blocks.len()];
    initialized[0] = Some(!constructing);
    let mut queue = VecDeque::from([0]);
    let mut queued = vec![false; cfg.blocks.len()];
    queued[0] = true;
    // Handlers start with the caught exception (catch) or an empty stack.
    for e in &body.exceptions {
        let block = *cfg
            .by_offset
            .get(&e.handler_start)
            .ok_or_else(|| invalid("Handler does not start a block"))?;
        let entry = match e.catch_type {
            Some(t) => {
                let id = types.class_type(t)?;
                if !types.value(id).reference() {
                    return Err(invalid("A catch clause must name a reference type"));
                }
                vec![types.stack(id)]
            }
            None => vec![],
        };
        if entries[block].as_ref().is_some_and(|old| *old != entry) {
            return Err(invalid(
                "Handlers sharing a start need the same entry stack",
            ));
        }
        entries[block] = Some(entry);
        initialized[block] = Some(true);
        if !queued[block] {
            queue.push_back(block);
            queued[block] = true;
        }
    }
    let try_starts: std::collections::HashSet<u32> =
        body.exceptions.iter().map(|e| e.try_start).collect();
    let handler_kind = |part: Part| match part {
        Part::Handler(_, _, i) => Some(body.exceptions[i].kind.as_str()),
        Part::Try(..) => None,
    };
    let mut maximum = 0;
    let mut work = 0;
    let mut cells = 0;
    while let Some(id) = queue.pop_front() {
        queued[id] = false;
        let mut stack = entries[id]
            .clone()
            .ok_or_else(|| invalid("Missing verifier stack"))?;
        types.this_initialized = initialized[id].unwrap_or(true);
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
        // Control may cross region boundaries only as ECMA-335 III.1.7.5 allows.
        if let Some(last) = body.instructions[block.first..block.last].last() {
            let from = region_path(&body.exceptions, block.start);
            let innermost = from.last().copied().and_then(handler_kind);
            let context = |e: &str| invalid(format!("IL_{:04X} ({}): {e}", last.offset, last.name));
            match last.opcode {
                0x2a if !from.is_empty() => {
                    return Err(context(
                        "ret cannot appear inside a protected range or handler; use leave",
                    ));
                }
                0xdc if !matches!(innermost, Some("finally" | "fault")) => {
                    return Err(context(
                        "endfinally is only valid in a finally or fault handler",
                    ));
                }
                0xfe1a => {
                    let catch = from.iter().rposition(|p| handler_kind(*p) == Some("catch"));
                    let finally = from
                        .iter()
                        .rposition(|p| matches!(handler_kind(*p), Some("finally" | "fault")));
                    if catch.is_none() || finally > catch {
                        return Err(context("rethrow is only valid inside a catch handler"));
                    }
                }
                _ => {}
            }
            for next in &block.successors {
                let target = cfg.blocks[*next].start;
                let to = region_path(&body.exceptions, target);
                let allowed = if matches!(last.opcode, 0xdd | 0xde) {
                    // leave exits enclosing ranges, never out of a finally or fault.
                    to.len() <= from.len()
                        && from[..to.len()] == to[..]
                        && !from[to.len()..]
                            .iter()
                            .any(|p| matches!(handler_kind(*p), Some("finally" | "fault")))
                } else {
                    to == from
                        || to.len() == from.len() + 1
                            && to[..from.len()] == from[..]
                            && matches!(to[from.len()], Part::Try(s, _) if s == target)
                };
                if !allowed {
                    return Err(context(&format!(
                        "control cannot transfer to IL_{target:04X} across an exception range; enter a try at its first instruction and exit with leave"
                    )));
                }
                if try_starts.contains(&target)
                    && !matches!(last.opcode, 0xdd | 0xde)
                    && !stack.is_empty()
                {
                    return Err(context(
                        "a protected range must be entered with an empty stack",
                    ));
                }
            }
        }
        for next in &block.successors {
            match initialized[*next] {
                Some(old) if old != types.this_initialized => {
                    return Err(invalid(format!(
                        "Paths reaching IL_{:04X} disagree on whether the base constructor was called",
                        cfg.blocks[*next].start
                    )));
                }
                _ => initialized[*next] = Some(types.this_initialized),
            }
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
        // Before the base constructor call, `this` is uninitialized.
        0x02..=0x05 | 0x0e | 0xfe09 if has_this && var == 0 && !types.this_initialized => {
            stack.push(Stack::This)
        }
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
        // Addresses of arguments and locals.
        0x0f | 0xfe0a => {
            if has_this && var == 0 {
                return Err(unsupported("Taking the address of this is not supported"));
            }
            let id = *types
                .arg_types
                .get(var)
                .ok_or_else(|| invalid("Argument index out of bounds"))?;
            stack.push(address(types, id)?);
        }
        0x12 | 0xfe0d => {
            let id = *types
                .local_types
                .get(var)
                .ok_or_else(|| invalid("Local index out of bounds"))?;
            stack.push(address(types, id)?);
        }
        0x46..=0x50 => {
            let Stack::Pointer(target) = pop(stack)? else {
                return Err(invalid("ldind requires a managed pointer"));
            };
            indirect(types, i.opcode, target)?;
            stack.push(types.stack(target));
        }
        0x51..=0x57 | 0xdf => {
            let value = pop(stack)?;
            let Stack::Pointer(target) = pop(stack)? else {
                return Err(invalid("stind requires a managed pointer"));
            };
            indirect(types, i.opcode, target)?;
            if !types.compatible(value, types.stack(target))? {
                return Err(invalid("stind value does not match the pointer's type"));
            }
        }
        0x71 | 0x81 | 0xfe15 => {
            let id = types.token(operand(i)?)?;
            if i.opcode == 0x81 {
                expect(types, stack, types.stack(id))?;
            }
            if pop(stack)? != Stack::Pointer(id) {
                return Err(invalid(format!(
                    "{} requires a managed pointer to {}",
                    i.name,
                    types.describe(types.stack(id))
                )));
            }
            if i.opcode == 0x71 {
                stack.push(types.stack(id));
            }
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
            if !types.this_initialized {
                return Err(invalid(
                    "A constructor must call a base or chained constructor before returning",
                ));
            }
            if let Some(result) = result {
                expect(types, stack, result)?;
            }
            if !stack.is_empty() {
                return Err(invalid("Return leaves values on the evaluation stack"));
            }
        }
        0x2b | 0x38 => {}
        // leave and endfinally empty the evaluation stack; rethrow ends the handler.
        0xdc..=0xde => stack.clear(),
        0xfe1a => {}
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
            let pending = token
                .checked_sub(crate::metadata_write::PENDING_STRING)
                .is_some_and(|i| (i as usize) < types.pending_strings);
            if !pending {
                types.a.user_string(token)?;
            }
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
        0x7b..=0x80 => field(types, owner, stack, i)?,
        0x8c => {
            let id = types.token(operand(i)?)?;
            let generic = matches!(types.value(id), Value::Generic(..));
            if types.value(id).reference() && !generic {
                return Err(unsupported("box on reference types is not supported"));
            }
            expect(types, stack, types.stack(id))?;
            // Boxing a reference-constrained parameter leaves the reference as is.
            stack.push(Stack::Ref(if types.value(id).reference() {
                id
            } else {
                types.intern(Value::Boxed(id))?
            }));
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
/// The (definition, arguments) of a TypeSpec that instantiates a generic type.
fn generic_instance(a: &Assembly, spec: u32) -> Option<(u32, Vec<crate::signature::Type>)> {
    use crate::signature::Type;
    let row = a
        .metadata()
        .type_specs
        .get((spec & 0xff_ffff).checked_sub(1)? as usize)?;
    match crate::signature::type_spec(a.blob(row.signature).ok()?).ok()? {
        Type::GenericInstance { base, args } => match *base {
            Type::Named(definition) | Type::ValueType(definition) => Some((definition, args)),
            _ => None,
        },
        _ => None,
    }
}
/// `T[,]::Get/Set/Address` and `newobj T[,]::.ctor` (ECMA-335 II.14.2): one int32 index
/// (or, for constructors, length and optionally lower bound) per dimension.
fn array_method(
    types: &mut Types<'_>,
    stack: &mut Vec<Stack>,
    m: &crate::assembly::MethodRef,
    element: usize,
    rank: u32,
    construct: bool,
) -> Result<()> {
    let rank = rank as usize;
    let sig = &m.signature;
    let element_stack = types.stack(element);
    let int = |t: &crate::signature::Type| *t == crate::signature::Type::primitive("int");
    let indices = |n: usize| sig.parameters.len() >= n && sig.parameters[..n].iter().all(int);
    let shape = match (m.name.as_str(), construct) {
        (".ctor", true) => {
            (sig.parameters.len() == rank || sig.parameters.len() == 2 * rank)
                && indices(sig.parameters.len())
        }
        ("Get", false) => sig.parameters.len() == rank && indices(rank),
        ("Set", false) => sig.parameters.len() == rank + 1 && indices(rank),
        ("Address", false) => sig.parameters.len() == rank && indices(rank),
        _ => false,
    };
    if !shape || !sig.has_this {
        return Err(invalid(format!(
            "Invalid array method {} for rank {rank}",
            m.name
        )));
    }
    if m.name == "Set" {
        expect(types, stack, element_stack)?;
    }
    for _ in 0..if m.name == "Set" {
        rank
    } else {
        sig.parameters.len()
    } {
        if !matches!(pop(stack)?, Stack::I4) {
            return Err(invalid("Array indices and lengths must be int32"));
        }
    }
    let array = types.token(m.owner)?;
    if construct {
        stack.push(Stack::Ref(array));
        return Ok(());
    }
    expect(types, stack, Stack::Ref(array))?;
    match m.name.as_str() {
        "Get" => stack.push(element_stack),
        "Address" => stack.push(address(types, element)?),
        _ => {}
    }
    Ok(())
}
/// Accessibility of another assembly's member: assembly-scoped access does not cross
/// assemblies, so only public and family access remain.
fn external_access(flags: u16) -> u16 {
    match flags & 7 {
        6 => 6,
        4 | 5 => (flags & !7) | 4,
        _ => flags & !7,
    }
}
/// A field of a type in a loaded dependency, with its definition's flags.
fn external_field(
    types: &mut Types<'_>,
    caller: u32,
    stack: &mut Vec<Stack>,
    i: &cil::Instruction,
    owner: u32,
    flags: u16,
    ty: &crate::signature::Type,
) -> Result<()> {
    let is_static = matches!(i.opcode, 0x7e..=0x80);
    let store = matches!(i.opcode, 0x7d | 0x80);
    let address_of = matches!(i.opcode, 0x7c | 0x7f);
    if (flags & 16 != 0) != is_static {
        return Err(invalid(
            "Field opcode disagrees with the field's static flag",
        ));
    }
    if flags & 0x40 != 0 || (store || address_of) && flags & 0x20 != 0 {
        return Err(invalid(
            "Literal/init-only fields of other assemblies cannot be assigned",
        ));
    }
    let receiver = (!is_static)
        .then(|| {
            stack
                .len()
                .checked_sub(1 + usize::from(store))
                .map(|i| stack[i])
        })
        .flatten();
    access(types, owner, caller, external_access(flags), receiver)?;
    let value = types.signature(ty)?;
    if store {
        expect(types, stack, types.stack(value))?;
    }
    if !is_static {
        let target = types.token(owner)?;
        match types.value(target) {
            Value::Struct(_) => {
                let receiver = pop(stack)?;
                if receiver != Stack::Pointer(target)
                    && (store || address_of || receiver != Stack::Value(target))
                {
                    return Err(invalid(
                        "Struct fields are reached through a pointer to the struct",
                    ));
                }
            }
            v if v.reference() => expect(types, stack, types.stack(target))?,
            _ => return Err(unsupported("Fields of this external type are not editable")),
        }
    }
    if address_of {
        stack.push(address(types, value)?);
    } else if !store {
        stack.push(types.stack(value));
    }
    Ok(())
}
/// A managed pointer to a location of type `id`; pointers to pointers do not exist.
fn address(types: &Types<'_>, id: usize) -> Result<Stack> {
    if matches!(types.value(id), Value::Pointer(_)) {
        return Err(invalid("A managed pointer has no address"));
    }
    Ok(Stack::Pointer(id))
}
/// ldind/stind access widths must match the pointed-to storage type.
fn indirect(types: &Types<'_>, opcode: u16, target: usize) -> Result<()> {
    let value = match types.value(target) {
        Value::Enum(_, underlying) => types.value(underlying),
        v => v,
    };
    let fits = match opcode {
        0x46 | 0x52 => matches!(value, Value::I1 | Value::U1 | Value::Bool),
        0x47 => matches!(value, Value::U1 | Value::Bool),
        0x48 | 0x53 => matches!(value, Value::I2 | Value::U2 | Value::Char),
        0x49 => matches!(value, Value::U2 | Value::Char),
        0x4a | 0x54 => matches!(value, Value::I4 | Value::U4),
        0x4b => value == Value::U4,
        0x4c | 0x55 => matches!(value, Value::I8 | Value::U8),
        0x4d | 0xdf => matches!(value, Value::I | Value::U),
        0x4e | 0x56 => value == Value::R4,
        0x4f | 0x57 => value == Value::R8,
        0x50 | 0x51 => value.reference(),
        _ => false,
    };
    if !fits {
        return Err(invalid(format!(
            "Indirect access width does not match {}",
            types.describe(types.stack(target))
        )));
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
    if !matches!(token >> 24, 6 | 10 | 43) {
        return Err(unsupported(
            "Only MethodDef, MemberRef and MethodSpec calls can be edited",
        ));
    }
    types.a.validate_token(token)?;
    // The definition or reference a MethodSpec instantiates; its signature arrives
    // with the type and method arguments substituted.
    let definition = if token >> 24 == 43 {
        crate::assembly::coded(
            types.a.metadata().method_specs[(token & 0xffffff) as usize - 1].method,
        )
    } else {
        token
    };
    let m = types.a.method_ref(token)?;
    let sig = &m.signature;
    if sig.generic_count as usize != m.generic_args.len() {
        return Err(invalid(
            "A generic method is called through a MethodSpec with all of its type arguments",
        ));
    }
    if sig.explicit_this || sig.vararg_at.is_some() || sig.convention != 0 {
        return Err(unsupported(
            "Explicit-this and vararg calls require additional validation",
        ));
    }
    if m.owner >> 24 == 2 && types.a.generics.contains_key(&m.owner) && m.owner != types.context.1 {
        return Err(invalid(
            "Members of a generic type are called through a TypeSpec instantiation",
        ));
    }
    // Multidimensional arrays: the runtime's Get/Set/Address methods and constructors.
    let constructed = if m.owner >> 24 == 27 {
        Some(types.token(m.owner)?)
    } else {
        None
    };
    if let Some(id) = constructed
        && let Value::MultiArray(element, rank) = types.value(id)
    {
        return array_method(types, stack, &m, element, rank, i.opcode == 0x73);
    }
    if m.owner >> 24 == 27 && generic_instance(types.a, m.owner).is_none() {
        return Err(unsupported(
            "Calls on vectors and other constructed types require additional validation",
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
            // A loaded dependency establishes an external class or struct and its
            // constructor; delegates and abstract types stay closed.
            let creatable = matches!(
                externals().types.get(&m.owner).map(|e| &e.kind),
                Some(
                    ExternalKind::Class {
                        abstract_: false,
                        delegate: false
                    } | ExternalKind::Struct
                )
            );
            if token >> 24 != 10 || !creatable || !externals().members.contains_key(&token) {
                return Err(unsupported(
                    "Creating external types requires their defining assembly; load it as a dependency (delegates and abstract types cannot be created)",
                ));
            }
        } else if types.a.method_row(token)?.impl_flags & 0x1007 != 0 {
            return Err(unsupported(
                "Runtime-provided constructors, including delegates, require specialized validation",
            ));
        }
        types.token(m.owner)?;
        // Local constructors: concrete classes whose base chain is known.
        if token >> 24 == 6 {
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
            // Local structs derive directly from System.ValueType.
            let created = types.token(m.owner)?;
            let structure = matches!(types.value(created), Value::Struct(_));
            while !(types.core_name(parent)?.as_deref() == Some("Object")
                || structure && types.core_name(parent)?.as_deref() == Some("ValueType"))
            {
                // An external base class its loaded definition establishes ends the walk.
                if parent >> 24 == 1
                    && matches!(
                        externals().types.get(&parent).map(|e| &e.kind),
                        Some(ExternalKind::Class {
                            delegate: false,
                            ..
                        })
                    )
                {
                    break;
                }
                if parent >> 24 != 2 {
                    return Err(unsupported(
                        "Constructor base types require resolved ordinary classes: delegate construction is not supported, and external base classes need their assembly loaded as a dependency",
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
        }
    } else if m.name == ".ctor" && i.opcode == 0x28 && stack.len() > sig.parameters.len() {
        // `base(...)` or `this(...)` on the uninitialized `this` of this constructor.
        let receiver = stack[stack.len() - 1 - sig.parameters.len()];
        // `new S(...)` into a location: a struct constructor called on its address.
        let target = types.token(m.owner)?;
        if matches!(types.value(target), Value::Struct(_)) && receiver == Stack::Pointer(target) {
            if !sig.has_this || !sig.return_type.is_void() {
                return Err(invalid(
                    "A constructor call must be an instance call returning void",
                ));
            }
            if definition >> 24 == 6 {
                access(
                    types,
                    m.owner,
                    caller,
                    types.a.method_row(definition)?.flags,
                    None,
                )?;
            }
            let mut parameters = Vec::new();
            for p in &sig.parameters {
                let id = types.signature(p)?;
                parameters.push(types.stack(id));
            }
            for p in parameters.into_iter().rev() {
                expect(types, stack, p)?;
            }
            pop(stack)?;
            return Ok(());
        }
        let declaring = &types.a.metadata().type_defs[(caller & 0xffffff) as usize - 1];
        let base = crate::assembly::coded(declaring.extends);
        if receiver != Stack::This || m.owner != base && m.owner != caller {
            return Err(unsupported(
                "Direct constructor calls are only verified as this constructor's base(...) or this(...) call",
            ));
        }
        if !sig.has_this || !sig.return_type.is_void() {
            return Err(invalid(
                "A constructor call must be an instance call returning void",
            ));
        }
        if token >> 24 == 6 {
            access(
                types,
                m.owner,
                caller,
                types.a.method_row(token)?.flags,
                Some(Stack::This),
            )?;
        }
        let mut parameters = Vec::new();
        for p in &sig.parameters {
            let id = types.signature(p)?;
            parameters.push(types.stack(id));
        }
        for p in parameters.into_iter().rev() {
            expect(types, stack, p)?;
        }
        types.class_type(m.owner)?;
        pop(stack)?;
        types.this_initialized = true;
        return Ok(());
    } else if m.name.starts_with('.') {
        return Err(unsupported(
            "Direct constructor calls require initialization-state validation",
        ));
    }
    if i.opcode == 0x6f && !sig.has_this {
        return Err(invalid("callvirt requires an instance method"));
    }
    if definition >> 24 == 6 {
        let row = types.a.method_row(definition)?;
        if sig.has_this == (row.flags & 16 != 0) {
            return Err(invalid("Call signature disagrees with static method flags"));
        }
        if constructor && row.flags & 0x1800 != 0x1800 {
            return Err(invalid("newobj target lacks constructor metadata flags"));
        }
        if i.opcode == 0x28 && row.flags & 0x400 != 0 {
            return Err(invalid("An abstract method requires virtual dispatch"));
        }
        // The receiver lies below the arguments; newobj and static calls have none.
        let receiver = (sig.has_this && !constructor)
            .then(|| {
                stack
                    .len()
                    .checked_sub(1 + sig.parameters.len())
                    .map(|i| stack[i])
            })
            .flatten();
        access(types, m.owner, caller, row.flags, receiver)?;
    } else if let Some(flags) = externals().members.get(&definition).copied() {
        let receiver = (sig.has_this && !constructor)
            .then(|| {
                stack
                    .len()
                    .checked_sub(1 + sig.parameters.len())
                    .map(|i| stack[i])
            })
            .flatten();
        access(types, m.owner, caller, external_access(flags), receiver)?;
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
        let structure = matches!(types.value(target), Value::Struct(_));
        if !types.value(target).reference() && !structure {
            return Err(unsupported(
                "Instance calls on enums and generic structs require managed-address validation",
            ));
        }
        if constructor {
            stack.push(types.stack(target))
        } else if structure {
            // A struct's instance methods take `this` by managed pointer.
            if pop(stack)? != Stack::Pointer(target) {
                return Err(invalid(format!(
                    "Expected a managed pointer to {}",
                    types.describe(Stack::Value(target))
                )));
            }
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
    types.a.validate_token(token)?;
    // A field of a local generic type is referenced through its instantiation; the
    // definition supplies flags and owner, the instantiation the field's type.
    let (field_token, owner, instance, ty) = match token >> 24 {
        4 => {
            let (owner, _, ty) = types.a.field_ref(token)?;
            (token, owner, owner, ty)
        }
        10 => {
            let (parent, name, ty) = types.a.field_ref(token)?;
            if parent >> 24 == 1
                && let Some(flags) = externals().members.get(&token).copied()
            {
                return external_field(types, caller, stack, i, parent, flags, &ty);
            }
            let Some((definition, args)) = (parent >> 24 == 27)
                .then(|| generic_instance(types.a, parent))
                .flatten()
            else {
                return Err(unsupported(
                    "External fields require their defining assembly; load it as a dependency",
                ));
            };
            let field = types
                .a
                .nodes
                .iter()
                .find(|n| n.parent == definition && n.token >> 24 == 4 && n.name == name)
                .ok_or_else(|| {
                    unsupported("Fields of external generic types require their definitions")
                })?
                .token;
            (
                field,
                definition,
                parent,
                crate::assembly::substitute(&ty, &args, &[]),
            )
        }
        _ => return Err(invalid("Field instructions require a field token")),
    };
    let row = &types.a.metadata().fields[(field_token & 0xffffff) as usize - 1];
    types.token(instance)?;
    let declaring = &types.a.metadata().type_defs[(owner & 0xffffff) as usize - 1];
    if declaring.flags & 0x18 == 0x10 {
        return Err(unsupported(
            "Explicit-layout fields require overlap and accessibility analysis",
        ));
    }
    let is_static = matches!(i.opcode, 0x7e..=0x80);
    let store = matches!(i.opcode, 0x7d | 0x80);
    let address_of = matches!(i.opcode, 0x7c | 0x7f);
    if (row.flags & 16 != 0) != is_static {
        return Err(invalid(
            "Field opcode disagrees with the field's static flag",
        ));
    }
    // Init-only fields are assigned by their own type's constructor or initializer.
    let initializing = types.initializer == Some(is_static) && owner == caller;
    if row.flags & 0x40 != 0 || store && row.flags & 0x20 != 0 && !initializing {
        return Err(invalid(
            "Literal/init-only fields cannot be assigned by this method",
        ));
    }
    // The receiver is on top for loads and below the value for stores.
    let receiver = (!is_static)
        .then(|| {
            stack
                .len()
                .checked_sub(1 + usize::from(store))
                .map(|i| stack[i])
        })
        .flatten();
    access(types, owner, caller, row.flags, receiver)?;
    let value = types.signature(&ty)?;
    if store {
        expect(types, stack, types.stack(value))?;
    }
    // A constructor may store its own fields before calling the base constructor.
    if store && !is_static && owner == caller && stack.last() == Some(&Stack::This) {
        pop(stack)?;
        return Ok(());
    }
    if !is_static {
        let target = types.token(instance)?;
        match types.value(target) {
            // A struct's fields are reached through a pointer to it; loads also accept
            // the struct value itself.
            Value::Struct(_) => {
                let receiver = pop(stack)?;
                if receiver != Stack::Pointer(target)
                    && (store || address_of || receiver != Stack::Value(target))
                {
                    return Err(invalid(format!(
                        "Expected a managed pointer to {}, found {}",
                        types.describe(Stack::Value(target)),
                        types.describe(receiver)
                    )));
                }
            }
            v if v.reference() => expect(types, stack, types.stack(target))?,
            _ => {
                return Err(unsupported(
                    "Fields of generic structs require managed-address validation",
                ));
            }
        }
    }
    if address_of {
        stack.push(address(types, value)?);
    } else if !store {
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
    let value = types.storage(element);
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
    let original_regions = editable_regions(&body);
    if edit.is_none() {
        verify(a, token, &body)?;
    }
    if let Some(edit) = edit {
        body.instructions = cil::decode(&edit.code)?;
        body.code_size = edit.code.len() as u32;
        body.max_stack = edit.max_stack;
    }
    let strings = edit.map_or(&[][..], |e| e.strings.as_slice());
    for i in &mut body.instructions {
        if let Operand::Token(t) = i.operand {
            i.resolved = Some(
                pending_literal(&format!("0x{t:08X}"), strings).unwrap_or_else(|| a.resolve(t)),
            );
        }
    }
    let mut instructions = cil_encode::from_body(&body);
    show_literals(&mut instructions, strings);
    let original_types = a.locals(&body)?;
    let types = edit
        .and_then(|e| e.locals.clone())
        .unwrap_or_else(|| original_types.clone());
    let shown = |types: &[crate::signature::Type]| -> Vec<String> {
        types.iter().map(|t| a.format_type(t, token)).collect()
    };
    let mut il = crate::emit::il(a, token, &body);
    if edit.is_some_and(|e| e.locals.is_some()) {
        // The listing shows the edited locals, not the method's current signature.
        let mut lines: Vec<String> = il
            .lines()
            .filter(|l| !l.starts_with(".locals"))
            .map(str::to_owned)
            .collect();
        if let Some(at) = lines.iter().position(|l| l.starts_with(".maxstack"))
            && !types.is_empty()
        {
            let list = types
                .iter()
                .enumerate()
                .map(|(i, t)| format!("[{i}] {} local{i}", a.format_type(t, token)))
                .collect::<Vec<_>>()
                .join(", ");
            lines.insert(at + 1, format!(".locals init ({list})"));
        }
        il = lines.join("\n") + "\n";
    }
    Ok(EditPreview {
        token,
        revision,
        instructions,
        original,
        changed: edit.is_some(),
        il,
        max_stack: body.max_stack,
        code_size: body.code_size as usize,
        locals: shown(&types),
        original_locals: shown(&original_types),
        regions: edit.map_or_else(|| original_regions.clone(), |e| e.regions.clone()),
        original_regions,
    })
}
