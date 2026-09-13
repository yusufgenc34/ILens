//! Typed three-address IR. Each pushed value is captured at its original IL offset.
//! This preserves side effects, local mutation, dup, and evaluation order before simplification.
use crate::{
    analysis::{self, StackAnalysis},
    assembly::{Assembly, MethodRef},
    cfg::ControlFlowGraph,
    cil::{Instruction, MethodBody, Operand},
    error::{Error, Result},
    signature::Type,
};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct Expr {
    pub ty: Type,
    pub kind: ExprKind,
}
#[derive(Debug, Clone, Serialize)]
pub enum ExprKind {
    Variable(String),
    Constant(String),
    String(String),
    Null,
    Binary {
        op: String,
        left: Box<Expr>,
        right: Box<Expr>,
        checked: bool,
        unsigned: bool,
    },
    Unary {
        op: String,
        value: Box<Expr>,
    },
    Call {
        token: u32,
        owner: u32,
        name: String,
        instance: Option<Box<Expr>>,
        args: Vec<Expr>,
        generic_args: Vec<Type>,
        virtual_call: bool,
    },
    New {
        owner: u32,
        args: Vec<Expr>,
    },
    Field {
        token: u32,
        owner: u32,
        name: String,
        instance: Option<Box<Expr>>,
    },
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },
    NewArray {
        element: Type,
        length: Box<Expr>,
    },
    ArrayInitializer {
        element: Type,
        values: Vec<Expr>,
    },
    Length(Box<Expr>),
    Cast {
        mode: String,
        value: Box<Expr>,
        target: Type,
    },
    Address(Box<Expr>),
    Deref(Box<Expr>),
    Default(Type),
    FunctionPointer {
        token: u32,
        instance: Option<Box<Expr>>,
    },
    TypeToken(u32),
    SizeOf(Type),
}
#[derive(Debug, Clone, Serialize)]
pub enum Statement {
    Let { name: String, ty: Type, value: Expr },
    Assign { target: Expr, value: Expr },
    Evaluate(Expr),
}
#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub target: usize,
    pub values: Vec<Expr>,
}
#[derive(Debug, Clone, Serialize)]
pub enum Terminator {
    Jump(Edge),
    Condition {
        condition: Expr,
        yes: Edge,
        no: Edge,
    },
    Switch {
        value: Expr,
        arms: Vec<Edge>,
        default: Edge,
    },
    Return(Option<Expr>),
    Throw(Option<Expr>),
    EndFinally,
}
#[derive(Debug, Clone, Serialize)]
pub struct IrBlock {
    pub id: usize,
    pub offset: u32,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}
#[derive(Debug, Clone, Serialize)]
pub struct TypedIr {
    pub blocks: Vec<IrBlock>,
    pub locals: Vec<Type>,
    pub argument_names: Vec<String>,
    pub return_type: Type,
    pub stack: StackAnalysis,
}
impl Expr {
    pub fn var(name: impl Into<String>, ty: Type) -> Self {
        Self {
            ty,
            kind: ExprKind::Variable(name.into()),
        }
    }
    pub fn constant(value: impl Into<String>, ty: Type) -> Self {
        Self {
            ty,
            kind: ExprKind::Constant(value.into()),
        }
    }
}
fn pop(stack: &mut Vec<Expr>) -> Result<Expr> {
    stack.pop().ok_or_else(|| Error::cil("IR stack underflow"))
}
fn operand_token(i: &Instruction) -> Result<u32> {
    i.token().ok_or_else(|| Error::cil("Missing token operand"))
}
fn bin(op: &str, left: Expr, right: Expr, checked: bool, unsigned: bool, ty: Type) -> Expr {
    Expr {
        ty,
        kind: ExprKind::Binary {
            op: op.to_owned(),
            left: Box::new(left),
            right: Box::new(right),
            checked,
            unsigned,
        },
    }
}
fn unary(op: &str, value: Expr, ty: Type) -> Expr {
    Expr {
        ty,
        kind: ExprKind::Unary {
            op: op.to_owned(),
            value: Box::new(value),
        },
    }
}
fn truth(value: Expr) -> Expr {
    if value.ty.is_bool() {
        value
    } else if value.ty.is_reference() {
        bin(
            "!=",
            value,
            Expr {
                ty: Type::primitive("object"),
                kind: ExprKind::Null,
            },
            false,
            false,
            Type::primitive("bool"),
        )
    } else {
        bin(
            "!=",
            value,
            Expr::constant("0", Type::primitive("int")),
            false,
            false,
            Type::primitive("bool"),
        )
    }
}
pub fn coerce(value: Expr, target: &Type) -> Expr {
    if target.is_bool() && !value.ty.is_bool() {
        match &value.kind {
            ExprKind::Constant(v) if v == "0" || v == "1" => {
                Expr::constant(if v == "0" { "false" } else { "true" }, target.clone())
            }
            _ => truth(value),
        }
    } else if value.ty != *target
        && matches!(target, Type::Primitive(t) if ["sbyte", "byte", "short", "ushort", "char", "int", "uint", "long", "ulong", "nint", "nuint", "float", "double"].contains(&t.as_str()))
        && matches!(&value.ty, Type::Primitive(t) if ["sbyte", "byte", "short", "ushort", "char", "int", "uint", "long", "ulong", "nint", "nuint", "float", "double"].contains(&t.as_str()))
    {
        Expr {
            ty: target.clone(),
            kind: ExprKind::Cast {
                mode: "unchecked".into(),
                value: Box::new(value),
                target: target.clone(),
            },
        }
    } else {
        value
    }
}
fn call_args(stack: &mut Vec<Expr>, mr: &MethodRef) -> Result<Vec<Expr>> {
    let mut args = Vec::new();
    for ty in mr.signature.parameters.iter().rev() {
        args.push(coerce(pop(stack)?, ty));
    }
    args.reverse();
    Ok(args)
}
pub fn lift(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    cfg: &ControlFlowGraph,
    stack_info: StackAnalysis,
) -> Result<TypedIr> {
    let mr = a.method_ref(token)?;
    let locals = a.locals(body)?;
    let args = analysis::arguments(a, token)?;
    let mut argument_names = a.parameter_names(token, &mr.signature);
    if mr.signature.has_this {
        argument_names.insert(0, "this".to_owned());
    }
    let mut blocks = Vec::new();
    for b in &cfg.blocks {
        let Some(entry) = &stack_info.entry_stacks[b.id] else {
            continue;
        };
        let mut stack: Vec<_> = entry
            .iter()
            .enumerate()
            .map(|(n, t)| Expr::var(format!("stack_{:04X}_{n}", b.start), t.clone()))
            .collect();
        let mut statements = Vec::new();
        let mut terminator = None;
        for i in &body.instructions[b.first..b.last] {
            let op = i.opcode;
            let mut pushed = None;
            let var = analysis::variable(i).unwrap_or(0);
            let local = || -> Result<Expr> {
                Ok(Expr::var(
                    format!("local{var}"),
                    locals
                        .get(var)
                        .cloned()
                        .ok_or_else(|| Error::cil("Local index out of bounds"))?,
                ))
            };
            let arg = || -> Result<Expr> {
                Ok(Expr::var(
                    argument_names
                        .get(var)
                        .ok_or_else(|| Error::cil("Argument index out of bounds"))?
                        .clone(),
                    args.get(var)
                        .cloned()
                        .ok_or_else(|| Error::cil("Argument type out of bounds"))?,
                ))
            };
            let edge = |target: u32, stack: &[Expr]| -> Result<Edge> {
                Ok(Edge {
                    target: *cfg
                        .by_offset
                        .get(&target)
                        .ok_or_else(|| Error::cil("Invalid IR edge"))?,
                    values: stack.to_vec(),
                })
            };
            match op {
                0x00 | 0x01 => {}
                0x02..=0x05 | 0x0e | 0xfe09 => pushed = Some(arg()?),
                0x06..=0x09 | 0x11 | 0xfe0c => pushed = Some(local()?),
                0x0f | 0xfe0a | 0x12 | 0xfe0d => {
                    let value = if matches!(op, 0x0f | 0xfe0a) {
                        arg()?
                    } else {
                        local()?
                    };
                    pushed = Some(Expr {
                        ty: Type::ByRef(Box::new(value.ty.clone())),
                        kind: ExprKind::Address(Box::new(value)),
                    });
                }
                0x0a..=0x0d | 0x13 | 0xfe0e | 0x10 | 0xfe0b => {
                    let target = if matches!(op, 0x10 | 0xfe0b) {
                        arg()?
                    } else {
                        local()?
                    };
                    let value = coerce(pop(&mut stack)?, &target.ty);
                    statements.push(Statement::Assign { target, value });
                }
                0x14 => {
                    pushed = Some(Expr {
                        ty: Type::primitive("object"),
                        kind: ExprKind::Null,
                    })
                }
                0x15..=0x1e => {
                    pushed = Some(Expr::constant(
                        (op as i32 - 0x16).to_string(),
                        Type::primitive("int"),
                    ))
                }
                0x1f..=0x21 => {
                    let v = if let Operand::Integer(v) = i.operand {
                        v
                    } else {
                        return Err(Error::cil("Expected integer"));
                    };
                    pushed = Some(Expr::constant(
                        format!("{v}{}", if op == 0x21 { "L" } else { "" }),
                        Type::primitive(if op == 0x21 { "long" } else { "int" }),
                    ));
                }
                0x22 | 0x23 => {
                    let Operand::Float(v) = &i.operand else {
                        return Err(Error::cil("Expected float"));
                    };
                    let ty = if op == 0x22 { "float" } else { "double" };
                    let literal = match v.as_str() {
                        "NaN" => format!("{ty}.NaN"),
                        "inf" => format!("{ty}.PositiveInfinity"),
                        "-inf" => format!("{ty}.NegativeInfinity"),
                        _ => format!("{v}{}", if op == 0x22 { "f" } else { "d" }),
                    };
                    pushed = Some(Expr::constant(literal, Type::primitive(ty)));
                }
                0x25 => {
                    let v = pop(&mut stack)?;
                    stack.push(v.clone());
                    stack.push(v);
                }
                0x26 => {
                    let v = pop(&mut stack)?;
                    statements.push(Statement::Evaluate(v));
                }
                0x28 | 0x6f | 0x73 => {
                    let t = operand_token(i)?;
                    let called = a.method_ref(t)?;
                    if called.signature.vararg_at.is_some() {
                        return Err(Error::limitation("Vararg calls remain in IL"));
                    }
                    let values = call_args(&mut stack, &called)?;
                    if op == 0x73 {
                        pushed = Some(Expr {
                            ty: Type::Named(called.owner),
                            kind: ExprKind::New {
                                owner: called.owner,
                                args: values,
                            },
                        });
                    } else {
                        let instance = if called.signature.has_this {
                            Some(Box::new(pop(&mut stack)?))
                        } else {
                            None
                        };
                        // Direct calls to a virtual method do not have C# virtual dispatch semantics.
                        if op == 0x28 && t >> 24 == 6 && a.method_row(t)?.flags & 0x40 != 0 {
                            return Err(Error::limitation(
                                "Nonvirtual invocation of a virtual method requires IL",
                            ));
                        }
                        let value = Expr {
                            ty: called.signature.return_type.clone(),
                            kind: ExprKind::Call {
                                token: t,
                                owner: called.owner,
                                name: called.name,
                                instance,
                                args: values,
                                generic_args: called.generic_args,
                                virtual_call: op == 0x6f,
                            },
                        };
                        if value.ty.is_void() {
                            statements.push(Statement::Evaluate(value));
                        } else {
                            pushed = Some(value);
                        }
                    }
                }
                0x2a => {
                    let value = if mr.signature.return_type.is_void() {
                        None
                    } else {
                        Some(coerce(pop(&mut stack)?, &mr.signature.return_type))
                    };
                    terminator = Some(Terminator::Return(value));
                }
                0x2b | 0x38 | 0xdd | 0xde => {
                    if matches!(op, 0xdd | 0xde) {
                        stack.clear();
                    }
                    let Operand::Branch(t) = i.operand else {
                        return Err(Error::cil("Expected branch"));
                    };
                    terminator = Some(Terminator::Jump(edge(t, &stack)?));
                }
                0x2c..=0x37 | 0x39..=0x44 => {
                    let Operand::Branch(t) = i.operand else {
                        return Err(Error::cil("Expected conditional branch"));
                    };
                    let norm = if op <= 0x37 { op + 0x0d } else { op };
                    let condition = if norm == 0x39 || norm == 0x3a {
                        let v = truth(pop(&mut stack)?);
                        if norm == 0x39 {
                            unary("!", v, Type::primitive("bool"))
                        } else {
                            v
                        }
                    } else {
                        let right = pop(&mut stack)?;
                        let left = pop(&mut stack)?;
                        let operator = match norm {
                            0x3b => "==",
                            0x3c | 0x41 => ">=",
                            0x3d | 0x42 => ">",
                            0x3e | 0x43 => "<=",
                            0x3f | 0x44 => "<",
                            0x40 => "!=",
                            _ => return Err(Error::limitation("Conditional opcode")),
                        };
                        bin(
                            operator,
                            left,
                            right,
                            false,
                            norm >= 0x40,
                            Type::primitive("bool"),
                        )
                    };
                    terminator = Some(Terminator::Condition {
                        condition,
                        yes: edge(t, &stack)?,
                        no: edge(i.offset + i.size, &stack)?,
                    });
                }
                0x45 => {
                    let value = pop(&mut stack)?;
                    let Operand::Switch(ts) = &i.operand else {
                        return Err(Error::cil("Expected switch targets"));
                    };
                    terminator = Some(Terminator::Switch {
                        value,
                        arms: ts.iter().map(|t| edge(*t, &stack)).collect::<Result<_>>()?,
                        default: edge(i.offset + i.size, &stack)?,
                    });
                }
                0x58..=0x64 | 0xd6..=0xdb | 0xfe01..=0xfe05 => {
                    let right = pop(&mut stack)?;
                    let left = pop(&mut stack)?;
                    let operator = match op {
                        0x58 | 0xd6 | 0xd7 => "+",
                        0x59 | 0xda | 0xdb => "-",
                        0x5a | 0xd8 | 0xd9 => "*",
                        0x5b | 0x5c => "/",
                        0x5d | 0x5e => "%",
                        0x5f => "&",
                        0x60 => "|",
                        0x61 => "^",
                        0x62 => "<<",
                        0x63 | 0x64 => ">>",
                        0xfe01 => "==",
                        0xfe02 | 0xfe03 => ">",
                        0xfe04 | 0xfe05 => "<",
                        _ => return Err(Error::limitation("Unknown arithmetic")),
                    };
                    let comparison = op >= 0xfe01;
                    let ty = if comparison {
                        Type::primitive("bool")
                    } else {
                        left.ty.clone()
                    };
                    let unsigned = matches!(
                        op,
                        0x5c | 0x5e | 0x64 | 0xd7 | 0xd9 | 0xdb | 0xfe03 | 0xfe05
                    );
                    // CIL unsigned comparison of object references is a non-null test.
                    let value = if op == 0xfe03
                        && matches!(right.kind, ExprKind::Null)
                        && left.ty.is_reference()
                    {
                        bin("!=", left, right, false, false, ty)
                    } else {
                        bin(
                            operator,
                            left,
                            right,
                            (0xd6..=0xdb).contains(&op),
                            unsigned,
                            ty,
                        )
                    };
                    pushed = Some(value);
                }
                0x65 | 0x66 => {
                    let v = pop(&mut stack)?;
                    pushed = Some(unary(if op == 0x65 { "-" } else { "~" }, v.clone(), v.ty));
                }
                0x67..=0x6e | 0x76 | 0x82..=0x8b | 0xb3..=0xba | 0xd1..=0xd5 | 0xe0 => {
                    let target = match op {
                        0x67 | 0x82 | 0xb3 => "sbyte",
                        0x68 | 0x83 | 0xb5 => "short",
                        0x69 | 0x84 | 0xb7 => "int",
                        0x6a | 0x85 | 0xb9 => "long",
                        0x6b => "float",
                        0x6c | 0x76 => "double",
                        0x6d | 0x88 | 0xb8 => "uint",
                        0x6e | 0x89 | 0xba => "ulong",
                        0x86 | 0xb4 | 0xd2 => "byte",
                        0x87 | 0xb6 | 0xd1 => "ushort",
                        0x8a | 0xd3 | 0xd4 => "nint",
                        _ => "nuint",
                    };
                    let mode = if matches!(op, 0x82..=0x8b | 0x76) {
                        "checked_unsigned"
                    } else if matches!(op,0xb3..=0xba|0xd4..=0xd5) {
                        "checked"
                    } else {
                        "unchecked"
                    };
                    let value = pop(&mut stack)?;
                    let target = Type::primitive(target);
                    pushed = Some(Expr {
                        ty: target.clone(),
                        kind: ExprKind::Cast {
                            mode: mode.to_owned(),
                            target,
                            value: Box::new(value),
                        },
                    });
                }
                0x72 => {
                    pushed = Some(Expr {
                        ty: Type::primitive("string"),
                        kind: ExprKind::String(a.user_string(operand_token(i)?)?),
                    })
                }
                0x74 | 0x75 | 0x79 | 0x8c | 0xa5 => {
                    let t = Type::Named(operand_token(i)?);
                    let value = pop(&mut stack)?;
                    let mode = match op {
                        0x74 => "cast",
                        0x75 => "as",
                        0x79 => {
                            return Err(Error::limitation("unbox managed pointers remain in IL"));
                        }
                        0x8c => "box",
                        _ => "unbox",
                    };
                    let ty = if op == 0x8c {
                        Type::primitive("object")
                    } else {
                        t.clone()
                    };
                    pushed = Some(Expr {
                        ty,
                        kind: ExprKind::Cast {
                            mode: mode.to_owned(),
                            target: t,
                            value: Box::new(value),
                        },
                    });
                }
                0x7a => terminator = Some(Terminator::Throw(Some(pop(&mut stack)?))),
                0xfe1a => terminator = Some(Terminator::Throw(None)),
                0xdc => terminator = Some(Terminator::EndFinally),
                0x7b..=0x80 => {
                    let t = operand_token(i)?;
                    let (owner, name, ty) = a.field_ref(t)?;
                    let stored = if op == 0x7d || op == 0x80 {
                        Some(pop(&mut stack)?)
                    } else {
                        None
                    };
                    let instance = if op <= 0x7d {
                        Some(Box::new(pop(&mut stack)?))
                    } else {
                        None
                    };
                    let field = Expr {
                        ty: ty.clone(),
                        kind: ExprKind::Field {
                            token: t,
                            owner,
                            name,
                            instance,
                        },
                    };
                    if let Some(value) = stored {
                        statements.push(Statement::Assign {
                            target: field,
                            value: coerce(value, &ty),
                        });
                    } else if op == 0x7c || op == 0x7f {
                        pushed = Some(Expr {
                            ty: Type::ByRef(Box::new(ty)),
                            kind: ExprKind::Address(Box::new(field)),
                        });
                    } else {
                        pushed = Some(field);
                    }
                }
                0x8d => {
                    let element = Type::Named(operand_token(i)?);
                    let length = pop(&mut stack)?;
                    pushed = Some(Expr {
                        ty: Type::Array(Box::new(element.clone()), 1),
                        kind: ExprKind::NewArray {
                            element,
                            length: Box::new(length),
                        },
                    });
                }
                0x8e => {
                    pushed = Some(Expr {
                        ty: Type::primitive("nuint"),
                        kind: ExprKind::Length(Box::new(pop(&mut stack)?)),
                    })
                }
                0x8f..=0x9a | 0xa3 => {
                    let index = pop(&mut stack)?;
                    let array = pop(&mut stack)?;
                    let ty = match &array.ty {
                        Type::Array(t, _) => *t.clone(),
                        _ => Type::Unknown,
                    };
                    let value = Expr {
                        ty: ty.clone(),
                        kind: ExprKind::Index {
                            array: Box::new(array),
                            index: Box::new(index),
                        },
                    };
                    pushed = Some(if op == 0x8f {
                        Expr {
                            ty: Type::ByRef(Box::new(ty)),
                            kind: ExprKind::Address(Box::new(value)),
                        }
                    } else {
                        value
                    });
                }
                0x9b..=0xa2 | 0xa4 => {
                    let value = pop(&mut stack)?;
                    let index = pop(&mut stack)?;
                    let array = pop(&mut stack)?;
                    let ty = match &array.ty {
                        Type::Array(t, _) => *t.clone(),
                        _ => value.ty.clone(),
                    };
                    statements.push(Statement::Assign {
                        target: Expr {
                            ty: ty.clone(),
                            kind: ExprKind::Index {
                                array: Box::new(array),
                                index: Box::new(index),
                            },
                        },
                        value: coerce(value, &ty),
                    });
                }
                0x46..=0x50 | 0x71 => {
                    let address = pop(&mut stack)?;
                    let ty = if let Type::ByRef(t) = &address.ty {
                        *t.clone()
                    } else {
                        Type::Unknown
                    };
                    pushed = Some(Expr {
                        ty,
                        kind: ExprKind::Deref(Box::new(address)),
                    });
                }
                0x51..=0x57 | 0x81 | 0xdf => {
                    let value = pop(&mut stack)?;
                    let address = pop(&mut stack)?;
                    statements.push(Statement::Assign {
                        target: Expr {
                            ty: value.ty.clone(),
                            kind: ExprKind::Deref(Box::new(address)),
                        },
                        value,
                    });
                }
                0xfe15 => {
                    let t = Type::Named(operand_token(i)?);
                    let address = pop(&mut stack)?;
                    statements.push(Statement::Assign {
                        target: Expr {
                            ty: t.clone(),
                            kind: ExprKind::Deref(Box::new(address)),
                        },
                        value: Expr {
                            ty: t.clone(),
                            kind: ExprKind::Default(t),
                        },
                    });
                }
                0xfe06 | 0xfe07 => {
                    let t = operand_token(i)?;
                    let instance = if op == 0xfe07 {
                        Some(Box::new(pop(&mut stack)?))
                    } else {
                        None
                    };
                    pushed = Some(Expr {
                        ty: Type::primitive("nint"),
                        kind: ExprKind::FunctionPointer { token: t, instance },
                    });
                }
                0xd0 => {
                    pushed = Some(Expr {
                        ty: Type::Unknown,
                        kind: ExprKind::TypeToken(operand_token(i)?),
                    })
                }
                0xfe1c => {
                    pushed = Some(Expr {
                        ty: Type::primitive("int"),
                        kind: ExprKind::SizeOf(Type::Named(operand_token(i)?)),
                    })
                }
                _ => {
                    return Err(Error::limitation(format!(
                        "{} at IL_{:04X} requires annotated CIL (prefixes, filters, indirect calls and specialized instructions are preserved)",
                        i.name, i.offset
                    )));
                }
            }
            if let Some(value) = pushed {
                let name = format!("v_{:04X}", i.offset);
                let ty = value.ty.clone();
                statements.push(Statement::Let {
                    name: name.clone(),
                    ty: ty.clone(),
                    value,
                });
                stack.push(Expr::var(name, ty));
            }
        }
        let terminator = if let Some(t) = terminator {
            t
        } else {
            let next = b
                .successors
                .first()
                .ok_or_else(|| Error::cil("Missing fallthrough"))?;
            Terminator::Jump(Edge {
                target: *next,
                values: stack,
            })
        };
        blocks.push(IrBlock {
            id: b.id,
            offset: b.start,
            statements,
            terminator,
        });
    }
    Ok(TypedIr {
        blocks,
        locals,
        argument_names,
        return_type: mr.signature.return_type,
        stack: stack_info,
    })
}
