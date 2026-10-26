//! Bounded evaluation of a deliberately small, pure CIL subset. This interprets
//! target instructions in an ILens value model; it never invokes a CLR/runtime.
//! There is no field store, initializer runner, host reflection or native fallback.
use crate::{
    assembly::{Assembly, coded},
    cil::{MethodBody, Operand},
    signature::Type,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub const MAX_INSTRUCTIONS: usize = 2_000;
pub const MAX_UNITS: usize = 32_768;
#[derive(Debug, Clone)]
pub enum Value {
    I32(i32),
    String(Rc<Vec<u16>>, bool), // true: literal or explicit String.Intern
    Chars(Rc<RefCell<Vec<u16>>>),
    Null,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop(pub String);
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Stop {}
impl From<crate::error::Error> for Stop {
    fn from(e: crate::error::Error) -> Self {
        Self(e.detail.chars().take(200).collect())
    }
}
type Result<T> = std::result::Result<T, Stop>;
fn stop(s: &str) -> Stop {
    Stop(s.to_owned())
}
#[derive(Debug)]
pub struct Budget {
    pub steps: usize,
    pub allocated: usize,
    remaining: usize,
    memory: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new(20_000, 1024 * 1024)
    }
}
impl Budget {
    pub fn new(steps: usize, memory: usize) -> Self {
        Self {
            steps: 0,
            allocated: 0,
            remaining: steps.min(20_000),
            memory: memory.min(1024 * 1024),
        }
    }
    fn tick(&mut self) -> Result<()> {
        if self.remaining == 0 {
            return Err(stop("Evaluation instruction budget exhausted"));
        }
        self.remaining -= 1;
        self.steps += 1;
        Ok(())
    }
    fn allocate(&mut self, bytes: usize) -> Result<()> {
        if bytes > self.memory.saturating_sub(self.allocated) {
            return Err(stop("Evaluation allocation budget exhausted"));
        }
        self.allocated += bytes;
        Ok(())
    }
    fn units(&mut self, count: usize) -> Result<()> {
        if count > MAX_UNITS {
            return Err(stop(
                "Evaluation string/array length exceeds 32,768 UTF-16 units",
            ));
        }
        self.allocate(count * 2 + 64)
    }
}
fn pop(stack: &mut Vec<Value>) -> Result<Value> {
    stack
        .pop()
        .ok_or_else(|| stop("Evaluation stack underflow"))
}
fn integer(value: Value) -> Result<i32> {
    if let Value::I32(n) = value {
        Ok(n)
    } else {
        Err(stop("Expected int32 value"))
    }
}
fn chars(value: Value) -> Result<Rc<RefCell<Vec<u16>>>> {
    if let Value::Chars(s) = value {
        Ok(s)
    } else {
        Err(stop("Expected non-null char array"))
    }
}
fn string(value: Value) -> Result<Rc<Vec<u16>>> {
    if let Value::String(s, _) = value {
        Ok(s)
    } else {
        Err(stop("Expected non-null string"))
    }
}
fn truth(value: Value) -> bool {
    !matches!(value, Value::Null | Value::I32(0))
}
pub fn binary(op: &str, a: i32, b: i32) -> Option<i32> {
    Some(match op {
        "add" => a.wrapping_add(b),
        "sub" => a.wrapping_sub(b),
        "mul" => a.wrapping_mul(b),
        "div" => a.checked_div(b)?,
        "rem" => a.checked_rem(b)?,
        "div.un" => ((a as u32).checked_div(b as u32)?) as i32,
        "rem.un" => ((a as u32).checked_rem(b as u32)?) as i32,
        "and" => a & b,
        "or" => a | b,
        "xor" => a ^ b,
        "shl" => a.wrapping_shl(b as u32),
        "shr" => a.wrapping_shr(b as u32),
        "shr.un" => (a as u32).wrapping_shr(b as u32) as i32,
        "ceq" => i32::from(a == b),
        "cgt" => i32::from(a > b),
        "cgt.un" => i32::from((a as u32) > (b as u32)),
        "clt" => i32::from(a < b),
        "clt.un" => i32::from((a as u32) < (b as u32)),
        _ => return None,
    })
}
pub fn condition(op: &str, a: i32, b: i32) -> Option<bool> {
    Some(match op.trim_end_matches(".s") {
        "beq" => a == b,
        "bne.un" => a != b,
        "bge" => a >= b,
        "bgt" => a > b,
        "ble" => a <= b,
        "blt" => a < b,
        "bge.un" => (a as u32) >= (b as u32),
        "bgt.un" => (a as u32) > (b as u32),
        "ble.un" => (a as u32) <= (b as u32),
        "blt.un" => (a as u32) < (b as u32),
        _ => return None,
    })
}
pub fn constant(op: &str, operand: &str) -> Option<i32> {
    match op {
        "ldc.i4.m1" => Some(-1),
        "ldc.i4" | "ldc.i4.s" => operand.parse().ok(),
        _ => op
            .strip_prefix("ldc.i4.")?
            .parse::<i32>()
            .ok()
            .filter(|n| (0..=8).contains(n)),
    }
}
fn fits(value: &Value, ty: &Type) -> bool {
    match (value, ty) {
        (Value::I32(v), Type::Primitive(n)) => match n.as_str() {
            "int" | "uint" => true,
            "bool" => matches!(*v, 0 | 1),
            "char" | "ushort" => u16::try_from(*v).is_ok(),
            "short" => i16::try_from(*v).is_ok(),
            "byte" => u8::try_from(*v).is_ok(),
            "sbyte" => i8::try_from(*v).is_ok(),
            _ => false,
        },
        (Value::String(..) | Value::Null, Type::Primitive(n)) => n == "string",
        (Value::Chars(_) | Value::Null, Type::Array(t, 1)) => **t == Type::primitive("char"),
        _ => false,
    }
}

pub struct Evaluator<'a> {
    a: &'a Assembly,
    pub budget: Budget,
    initializers: HashSet<u32>,
    cache: HashMap<u32, Rc<MethodBody>>,
}
impl<'a> Evaluator<'a> {
    pub fn new(a: &'a Assembly, budget: Budget) -> Self {
        let initializers = a
            .nodes
            .iter()
            .filter(|n| n.name == ".cctor")
            .map(|n| n.parent)
            .collect();
        Self {
            a,
            budget,
            initializers,
            cache: HashMap::new(),
        }
    }
    /// Decode UTF-16 directly, avoiding the inspection UI's lossy string rendering.
    pub fn literal(&mut self, token: u32) -> Result<Value> {
        if token >> 24 != 0x70 {
            return Err(stop("Expected a user-string token"));
        }
        let heap = self.a.metadata().user_strings.data();
        let data = heap
            .get((token & 0xffffff) as usize..)
            .ok_or_else(|| stop("Invalid user string"))?;
        let mut r = crate::reader::Reader::new(data);
        let len = r.compressed()? as usize;
        if len == 0 {
            self.budget.units(0)?;
            return Ok(Value::String(Rc::new(vec![]), true));
        }
        if len % 2 != 1 {
            return Err(stop("Invalid UTF-16 user string"));
        }
        self.budget.units(len / 2)?;
        let bytes = r.take(len)?;
        Ok(Value::String(
            Rc::new(
                bytes[..len - 1]
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect(),
            ),
            true,
        ))
    }
    pub fn call(&mut self, token: u32, args: Vec<Value>) -> Result<Value> {
        self.run(token, args, 0)
    }
    fn run(&mut self, token: u32, mut args: Vec<Value>, depth: usize) -> Result<Value> {
        self.budget.tick()?;
        if depth >= 8 {
            return Err(stop("Evaluation call-depth limit reached"));
        }
        if token >> 24 != 6 {
            return Err(stop("Only same-module MethodDef calls can be evaluated"));
        }
        let row = self.a.method_row(token)?;
        let method = self.a.method_ref(token)?;
        if row.flags & 0x10 == 0
            || row.flags & 0x2000 != 0
            || row.impl_flags != 0
            || method.name.starts_with('.')
            || method.signature.has_this
            || method.signature.explicit_this
            || method.signature.generic_count != 0
            || method.signature.convention != 0
            || method.signature.vararg_at.is_some()
            || self.a.generics.contains_key(&method.owner)
            || args.len() > 8
            || args.len() != method.signature.parameters.len()
        {
            return Err(stop(
                "Evaluation requires a nongeneric static managed helper with at most 8 arguments",
            ));
        }
        // Never infer initial state or remove the observable initialization of a call.
        if self.initializers.contains(&method.owner) || self.initializers.contains(&0x02000001) {
            return Err(stop(
                "Type/module initializer prevents pure helper evaluation",
            ));
        }
        for (v, t) in args.iter().zip(&method.signature.parameters) {
            if !fits(v, t) {
                return Err(stop("Unsupported helper argument type"));
            }
            let len = match v {
                Value::String(s, _) => s.len(),
                Value::Chars(s) => s.borrow().len(),
                _ => 0,
            };
            if len > MAX_UNITS {
                return Err(stop("Helper argument exceeds length limit"));
            }
        }
        let body = if let Some(body) = self.cache.get(&token) {
            body.clone()
        } else {
            let body = self
                .a
                .body(token)?
                .ok_or_else(|| stop("Helper has no CIL body"))?;
            if body.instructions.len() > MAX_INSTRUCTIONS || !body.exceptions.is_empty() {
                return Err(stop(
                    "Helper exceeds instruction limit or contains exception handlers",
                ));
            }
            // This is also a structural/type check, not just a successful concrete path.
            // Only the modeled core String type has an intrinsic constructor.
            // Supplying this type fact does not widen the execution allowlist.
            let mut externals = crate::edit::Externals::default();
            for instruction in body.instructions.iter().filter(|i| i.name == "newobj") {
                let Some(t) = instruction.token() else {
                    continue;
                };
                if let Ok(m) = self.a.method_ref(t)
                    && t >> 24 == 10
                    && self.core_type(m.owner, "System.String")
                    && m.name == ".ctor"
                    && m.signature.has_this
                    && m.signature.return_type.is_void()
                    && m.signature.parameters == [Type::Array(Box::new(Type::primitive("char")), 1)]
                {
                    externals.types.insert(
                        m.owner,
                        crate::edit::ExternalType {
                            kind: crate::edit::ExternalKind::Class {
                                abstract_: false,
                                delegate: false,
                            },
                            bases: vec!["System.Object".into()],
                        },
                    );
                    externals.members.insert(t, 0x1806);
                }
            }
            crate::edit::with_externals(externals, || crate::edit::verify(self.a, token, &body))?;
            self.budget
                .allocate(body.code_size as usize * 8 + body.instructions.len() * 128)?;
            let body = Rc::new(body);
            self.cache.insert(token, body.clone());
            body
        };
        let types = self.a.locals(&body)?;
        if types.len() > 64 {
            return Err(stop("Evaluation supports at most 64 locals"));
        }
        self.budget.allocate((types.len() + args.len() + 64) * 32)?;
        let mut locals: Vec<Option<Value>> = types
            .iter()
            .map(|t| {
                if !body.init_locals {
                    None
                } else if fits(&Value::I32(0), t) {
                    Some(Value::I32(0))
                } else if fits(&Value::Null, t) {
                    Some(Value::Null)
                } else {
                    None
                }
            })
            .collect();
        let offsets: HashMap<_, _> = body
            .instructions
            .iter()
            .enumerate()
            .map(|(i, v)| (v.offset, i))
            .collect();
        let mut stack = Vec::<Value>::new();
        let mut pc = 0;
        loop {
            self.budget.tick()?;
            if stack.len() > 64 {
                return Err(stop("Evaluation stack limit reached"));
            }
            let instruction = body
                .instructions
                .get(pc)
                .ok_or_else(|| stop("Helper fell through its body"))?;
            pc += 1;
            let op = instruction.name.as_str();
            let index = match instruction.operand {
                Operand::Variable(i) => i as usize,
                _ => op
                    .rsplit('.')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(usize::MAX),
            };
            let mut branch = None;
            if let Some(n) = constant(
                op,
                &match instruction.operand {
                    Operand::Integer(n) => n.to_string(),
                    _ => String::new(),
                },
            ) {
                stack.push(Value::I32(n));
                continue;
            }
            match op {
                "nop" => {}
                "ldnull" => stack.push(Value::Null),
                "ldarg.0" | "ldarg.1" | "ldarg.2" | "ldarg.3" | "ldarg.s" | "ldarg" => stack.push(
                    args.get(index)
                        .ok_or_else(|| stop("Invalid argument index"))?
                        .clone(),
                ),
                "starg" | "starg.s" => {
                    let value = pop(&mut stack)?;
                    let ty = method
                        .signature
                        .parameters
                        .get(index)
                        .ok_or_else(|| stop("Invalid argument index"))?;
                    if !fits(&value, ty) {
                        return Err(stop("Argument type mismatch"));
                    }
                    args[index] = value;
                }
                "ldloc.0" | "ldloc.1" | "ldloc.2" | "ldloc.3" | "ldloc.s" | "ldloc" => stack.push(
                    locals
                        .get(index)
                        .and_then(Clone::clone)
                        .ok_or_else(|| stop("Uninitialized or unsupported local"))?,
                ),
                "stloc.0" | "stloc.1" | "stloc.2" | "stloc.3" | "stloc.s" | "stloc" => {
                    let v = pop(&mut stack)?;
                    let ty = types
                        .get(index)
                        .ok_or_else(|| stop("Invalid local index"))?;
                    if !fits(&v, ty) {
                        return Err(stop("Local type mismatch"));
                    }
                    locals[index] = Some(v);
                }
                "dup" => {
                    let v = stack.last().ok_or_else(|| stop("Empty dup"))?.clone();
                    stack.push(v);
                }
                "pop" => {
                    pop(&mut stack)?;
                }
                "add" | "sub" | "mul" | "div" | "div.un" | "rem" | "rem.un" | "and" | "or"
                | "xor" | "shl" | "shr" | "shr.un" | "ceq" | "cgt" | "cgt.un" | "clt"
                | "clt.un" => {
                    let b = integer(pop(&mut stack)?)?;
                    let a = integer(pop(&mut stack)?)?;
                    stack.push(Value::I32(
                        binary(op, a, b).ok_or_else(|| stop("Integer operation may throw"))?,
                    ));
                }
                "neg" | "not" | "conv.i1" | "conv.u1" | "conv.i2" | "conv.u2" | "conv.i4"
                | "conv.u4" => {
                    let v = integer(pop(&mut stack)?)?;
                    stack.push(Value::I32(match op {
                        "neg" => v.wrapping_neg(),
                        "not" => !v,
                        "conv.i1" => (v as i8) as i32,
                        "conv.u1" => (v as u8) as i32,
                        "conv.i2" => (v as i16) as i32,
                        "conv.u2" => (v as u16) as i32,
                        _ => v,
                    }));
                }
                "br" | "br.s" => branch = Some(true),
                "brfalse" | "brfalse.s" => branch = Some(!truth(pop(&mut stack)?)),
                "brtrue" | "brtrue.s" => branch = Some(truth(pop(&mut stack)?)),
                "beq" | "beq.s" | "bne.un" | "bne.un.s" | "bge" | "bge.s" | "bge.un"
                | "bge.un.s" | "bgt" | "bgt.s" | "bgt.un" | "bgt.un.s" | "ble" | "ble.s"
                | "ble.un" | "ble.un.s" | "blt" | "blt.s" | "blt.un" | "blt.un.s" => {
                    let b = integer(pop(&mut stack)?)?;
                    let a = integer(pop(&mut stack)?)?;
                    branch = condition(op, a, b);
                }
                "switch" => {
                    let n = integer(pop(&mut stack)?)? as u32 as usize;
                    if let Operand::Switch(targets) = &instruction.operand {
                        if let Some(target) = targets.get(n) {
                            pc = *offsets
                                .get(target)
                                .ok_or_else(|| stop("Invalid switch target"))?;
                        }
                    } else {
                        return Err(stop("Invalid switch"));
                    }
                }
                "ldstr" => stack.push(
                    self.literal(
                        instruction
                            .token()
                            .ok_or_else(|| stop("Missing string token"))?,
                    )?,
                ),
                "newarr" => {
                    if !self.core_type(instruction.token().unwrap_or(0), "System.Char") {
                        return Err(stop("Only char arrays are supported"));
                    }
                    let n = integer(pop(&mut stack)?)?;
                    if n < 0 {
                        return Err(stop("Negative array length"));
                    }
                    self.budget.units(n as usize)?;
                    stack.push(Value::Chars(Rc::new(RefCell::new(vec![0; n as usize]))));
                }
                "ldlen" => {
                    let array = chars(pop(&mut stack)?)?;
                    stack.push(Value::I32(array.borrow().len() as i32));
                }
                "ldelem.u2" => {
                    let index = integer(pop(&mut stack)?)? as usize;
                    let array = chars(pop(&mut stack)?)?;
                    let value = *array
                        .borrow()
                        .get(index)
                        .ok_or_else(|| stop("Array index would throw"))?;
                    stack.push(Value::I32(i32::from(value)));
                }
                "stelem.i2" => {
                    let value = integer(pop(&mut stack)?)?;
                    let index = integer(pop(&mut stack)?)? as usize;
                    let array = chars(pop(&mut stack)?)?;
                    *array
                        .borrow_mut()
                        .get_mut(index)
                        .ok_or_else(|| stop("Array index would throw"))? = value as u16;
                }
                "call" | "callvirt" | "newobj" => {
                    let token = instruction
                        .token()
                        .ok_or_else(|| stop("Missing call token"))?;
                    let called = self.a.method_ref(token)?;
                    let count = called.signature.parameters.len()
                        + usize::from(called.signature.has_this && op != "newobj");
                    if count > 8 || count > stack.len() {
                        return Err(stop("Invalid call argument count"));
                    }
                    let values = stack.split_off(stack.len() - count);
                    let value = if token >> 24 == 6 && op == "call" {
                        self.run(token, values, depth + 1)?
                    } else {
                        self.bcl(token, op, values)?
                    };
                    stack.push(value);
                }
                "ret" => {
                    let result = pop(&mut stack)?;
                    if !stack.is_empty() || !fits(&result, &method.signature.return_type) {
                        return Err(stop("Invalid helper return type or stack"));
                    }
                    return Ok(result);
                }
                _ => {
                    return Err(Stop(format!(
                        "Unsupported pure-evaluation instruction: {op}"
                    )));
                }
            }
            if branch == Some(true) {
                if let Operand::Branch(target) = instruction.operand {
                    pc = *offsets
                        .get(&target)
                        .ok_or_else(|| stop("Invalid branch target"))?;
                } else {
                    return Err(stop("Missing branch target"));
                }
            }
        }
    }
    // Scope checks select modeled BCL semantics; this does not authenticate a DLL.
    fn core_type(&self, token: u32, name: &str) -> bool {
        if token >> 24 != 1 || self.a.resolve_full(token) != name {
            return false;
        }
        let Some(row) = self
            .a
            .metadata()
            .type_refs
            .get((token & 0xffffff).saturating_sub(1) as usize)
        else {
            return false;
        };
        let scope = coded(row.resolution_scope);
        self.a.references.iter().any(|r| {
            r.token == scope
                && r.identity.culture == "neutral"
                && matches!(
                    (
                        r.identity.name.as_str(),
                        r.identity.public_key_token.as_str()
                    ),
                    ("mscorlib", "b77a5c561934e089")
                        | ("System.Runtime", "b03f5f7f11d50a3a")
                        | ("System.Private.CoreLib", "7cec85d7bea7798e")
                        | ("netstandard", "cc7b13ffcd2ddd51")
                )
        })
    }
    fn bcl(&mut self, token: u32, op: &str, mut args: Vec<Value>) -> Result<Value> {
        let m = self.a.method_ref(token)?;
        let sig = &m.signature;
        if token >> 24 != 10
            || !self.core_type(m.owner, "System.String")
            || sig.generic_count != 0
            || sig.explicit_this
            || sig.convention != 0
            || sig.vararg_at.is_some()
        {
            return Err(stop("External call is outside the pure BCL allowlist"));
        }
        let p = |name| Type::primitive(name);
        let array = Type::Array(Box::new(p("char")), 1);
        match (
            m.name.as_str(),
            sig.has_this,
            sig.parameters.as_slice(),
            &sig.return_type,
            op,
        ) {
            (".ctor", true, [t], r, "newobj") if *t == array && r.is_void() => {
                let a = chars(args.remove(0))?;
                self.budget.units(a.borrow().len())?;
                let units = a.borrow().clone();
                Ok(Value::String(Rc::new(units), false))
            }
            ("ToCharArray", true, [], r, "call" | "callvirt") if *r == array => {
                let s = string(args.remove(0))?;
                self.budget.units(s.len())?;
                Ok(Value::Chars(Rc::new(RefCell::new(s.as_ref().clone()))))
            }
            ("get_Length", true, [], r, "call" | "callvirt") if *r == p("int") => {
                Ok(Value::I32(string(args.remove(0))?.len() as i32))
            }
            ("get_Chars", true, [t], r, "call" | "callvirt")
                if *t == p("int") && *r == p("char") =>
            {
                let s = string(args.remove(0))?;
                let i = integer(args.remove(0))? as usize;
                Ok(Value::I32(i32::from(
                    *s.get(i).ok_or_else(|| stop("String index would throw"))?,
                )))
            }
            ("Intern", false, [t], r, "call") if *t == p("string") && *r == p("string") => {
                Ok(Value::String(string(args.remove(0))?, true))
            }
            ("Concat", false, [t, u], r, "call")
                if *t == p("string") && *u == p("string") && *r == p("string") =>
            {
                let units = |v| {
                    if matches!(v, Value::Null) {
                        Ok(Rc::new(vec![]))
                    } else {
                        string(v)
                    }
                };
                let a = units(args.remove(0))?;
                let b = units(args.remove(0))?;
                self.budget.units(a.len() + b.len())?;
                Ok(Value::String(
                    Rc::new([a.as_slice(), b.as_slice()].concat()),
                    false,
                ))
            }
            _ => Err(stop(
                "External call signature is outside the pure BCL allowlist",
            )),
        }
    }
}
