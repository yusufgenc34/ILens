use crate::{
    assembly::{Assembly, identifier, strip_arity},
    ast::Ast,
    cfg::ControlFlowGraph,
    cil::{MethodBody, Operand},
    error::{Error, Result},
    ir::*,
    signature::Type,
};
use std::collections::BTreeMap;
use std::fmt::Write;
fn safe_comment(s: &str) -> String {
    s.replace("*/", "* /").replace(['\r', '\n'], " ")
}
fn type_name(a: &Assembly, t: &Type, context: u32) -> String {
    a.format_type(t, context)
}
fn unsigned_type(t: &Type) -> Result<&'static str> {
    if *t == Type::primitive("long") || *t == Type::primitive("ulong") {
        Ok("ulong")
    } else if *t == Type::primitive("nint") || *t == Type::primitive("nuint") {
        Ok("nuint")
    } else if matches!(t,Type::Primitive(n) if ["int","uint","bool","byte","sbyte","short","ushort","char"].contains(&n.as_str()))
    {
        Ok("uint")
    } else {
        Err(Error::limitation(
            "Unsigned/unordered comparison cannot be safely represented for this stack type",
        ))
    }
}
pub fn expression(a: &Assembly, context: u32, e: &Expr) -> Result<String> {
    let expr = |e: &Expr| expression(a, context, e);
    use ExprKind::*;
    Ok(match &e.kind {
        Variable(n) => n.clone(),
        Constant(v) => v.clone(),
        String(s) => serde_json::to_string(s).map_err(|e| Error::limitation(e.to_string()))?,
        Null => "null".to_owned(),
        Binary {
            op,
            left,
            right,
            checked,
            unsigned,
        } => {
            let mut l = expr(left)?;
            let mut r = expr(right)?;
            if *unsigned {
                if op == "!=" && left.ty.is_reference() && right.ty.is_reference() {
                } else {
                    let ty = unsigned_type(&left.ty)?;
                    l = format!("unchecked(({ty})({l}))");
                    if op != ">>" && op != "<<" {
                        let ty = unsigned_type(&right.ty)?;
                        r = format!("unchecked(({ty})({r}))");
                    }
                }
            }
            let value = format!("({l} {op} {r})");
            let value = if *checked {
                format!("checked({value})")
            } else {
                value
            };
            if *unsigned && !e.ty.is_bool() {
                // CIL carries the result bits on its original stack type. The checked
                // operation uses unsigned bounds; reinterpret the result only afterward.
                format!("unchecked(({})({value}))", type_name(a, &e.ty, context))
            } else {
                value
            }
        }
        Unary { op, value } => format!("({op}{})", expr(value)?),
        Call {
            token,
            owner,
            name,
            instance,
            args,
            generic_args,
            ..
        } => {
            let receiver = if let Some(i) = instance {
                let s = expr(i)?;
                s.strip_prefix("ref ").unwrap_or(&s).to_owned()
            } else {
                strip_arity(&a.resolve(*owner))
            };
            let args = args
                .iter()
                .map(expr)
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            if name == ".ctor" {
                return Err(Error::limitation(
                    "Constructor chaining or in-place value construction remains explicit in IL",
                ));
            }
            // Only MethodSemantics confirms a property; a get_/set_ name alone is not sufficient.
            let property = a.semantics.iter().find_map(|(p, ss)| {
                if *p >> 24 == 23 {
                    ss.iter().find(|(_, m)| m == token).map(|(f, _)| (*p, *f))
                } else {
                    None
                }
            });
            if let Some((p, f)) = property {
                if f & 2 != 0 && args.is_empty() {
                    format!("{receiver}.{}", identifier(&a.resolve(p)))
                } else if f & 1 != 0 {
                    format!("{receiver}.{} = {args}", identifier(&a.resolve(p)))
                } else {
                    format!("{receiver}.{}({args})", identifier(name))
                }
            } else {
                let generic = if generic_args.is_empty() {
                    std::string::String::new()
                } else {
                    format!(
                        "<{}>",
                        generic_args
                            .iter()
                            .map(|t| type_name(a, t, context))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!("{receiver}.{}{generic}({args})", identifier(name))
            }
        }
        New { owner, args } => {
            if args
                .iter()
                .any(|e| matches!(e.kind, FunctionPointer { .. }))
            {
                return Err(Error::limitation(
                    "Delegate construction is preserved in IL until method-group binding is proven",
                ));
            }
            format!(
                "new {}({})",
                strip_arity(&a.resolve(*owner)),
                args.iter()
                    .map(expr)
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            )
        }
        Field {
            owner,
            name,
            instance,
            ..
        } => format!(
            "{}.{}",
            if let Some(i) = instance {
                let s = expr(i)?;
                s.strip_prefix("ref ").unwrap_or(&s).to_owned()
            } else {
                strip_arity(&a.resolve(*owner))
            },
            identifier(name)
        ),
        Index { array, index } => format!("{}[{}]", expr(array)?, expr(index)?),
        NewArray { element, length } => {
            format!("new {}[{}]", type_name(a, element, context), expr(length)?)
        }
        Length(v) => format!("((nuint){}.Length)", expr(v)?),
        Cast {
            mode,
            value,
            target,
        } => {
            let t = type_name(a, target, context);
            let v = expr(value)?;
            match mode.as_str() {
                "as" => format!("({v} as {t})"),
                "box" => format!("((object)({t})({v}))"),
                "checked_unsigned" => {
                    let u = unsigned_type(&value.ty)?;
                    format!("checked(({t})(unchecked(({u})({v}))))")
                }
                "checked" | "unchecked" => format!("{mode}(({t})({v}))"),
                _ => format!("(({t})({v}))"),
            }
        }
        Address(v) => format!("ref {}", expr(v)?),
        Deref(v) => {
            if let Address(inner) = &v.kind {
                expr(inner)?
            } else if let Type::ByRef(_) = &v.ty {
                expr(v)?
            } else {
                return Err(Error::limitation(
                    "Unverified indirect memory access remains in IL",
                ));
            }
        }
        Default(t) => format!("default({})", type_name(a, t, context)),
        FunctionPointer { .. } => {
            return Err(Error::limitation(
                "Raw managed function pointer has no safe source-level binding yet",
            ));
        }
        TypeToken(t) => match t >> 24 {
            1 | 2 | 27 => format!("typeof({}).TypeHandle", a.resolve(*t)),
            _ => {
                return Err(Error::limitation(
                    "Method/field runtime handles remain in IL",
                ));
            }
        },
        SizeOf(t) => format!("sizeof({})", type_name(a, t, context)),
    })
}
fn statement(a: &Assembly, token: u32, s: &Statement) -> Result<String> {
    Ok(match s {
        Statement::Let { name, ty, value } => format!(
            "{} {name} = {};",
            type_name(a, ty, token),
            expression(a, token, value)?
        ),
        Statement::Assign { target, value } => format!(
            "{} = {};",
            expression(a, token, target)?,
            expression(a, token, value)?
        ),
        Statement::Evaluate(e) => {
            let v = expression(a, token, e)?;
            if e.ty.is_void() {
                format!("{v};")
            } else {
                format!("_ = {v};")
            }
        }
    })
}
fn line(out: &mut String, indent: usize, text: &str) {
    let _ = writeln!(out, "{}{}", "    ".repeat(indent), text);
}
fn nodes(
    a: &Assembly,
    token: u32,
    ast: &[Ast],
    out: &mut String,
    indent: usize,
    cfg: &ControlFlowGraph,
) -> Result<()> {
    for n in ast {
        match n {
            Ast::Statement(s) => line(out, indent, &statement(a, token, s)?),
            Ast::ParallelAssign { targets, values } => {
                let render = |items: &[Expr]| -> Result<String> {
                    Ok(items
                        .iter()
                        .map(|e| expression(a, token, e))
                        .collect::<Result<Vec<_>>>()?
                        .join(", "))
                };
                line(
                    out,
                    indent,
                    &format!("({}) = ({});", render(targets)?, render(values)?),
                );
            }
            Ast::Label(id) => line(out, indent, &format!("IL_{:04X}:;", cfg.blocks[*id].start)),
            Ast::Goto(id) => line(
                out,
                indent,
                &format!("goto IL_{:04X};", cfg.blocks[*id].start),
            ),
            Ast::Return(v) => line(
                out,
                indent,
                &format!(
                    "return{};",
                    v.as_ref()
                        .map(|e| expression(a, token, e).map(|s| format!(" {s}")))
                        .transpose()?
                        .unwrap_or_default()
                ),
            ),
            Ast::Throw(v) => line(
                out,
                indent,
                &format!(
                    "throw{};",
                    v.as_ref()
                        .map(|e| expression(a, token, e).map(|s| format!(" {s}")))
                        .transpose()?
                        .unwrap_or_default()
                ),
            ),
            Ast::Break => line(out, indent, "break;"),
            Ast::If { condition, yes, no } => {
                line(
                    out,
                    indent,
                    &format!("if ({})", expression(a, token, condition)?),
                );
                line(out, indent, "{");
                nodes(a, token, yes, out, indent + 1, cfg)?;
                line(out, indent, "}");
                if !no.is_empty() {
                    line(out, indent, "else");
                    line(out, indent, "{");
                    nodes(a, token, no, out, indent + 1, cfg)?;
                    line(out, indent, "}");
                }
            }
            Ast::While { condition, body } => {
                line(
                    out,
                    indent,
                    &format!("while ({})", expression(a, token, condition)?),
                );
                line(out, indent, "{");
                nodes(a, token, body, out, indent + 1, cfg)?;
                line(out, indent, "}");
            }
            Ast::For {
                initializer,
                condition,
                increment,
                body,
            } => {
                line(
                    out,
                    indent,
                    &format!(
                        "for ({}; {}; {})",
                        statement(a, token, initializer)?.trim_end_matches(';'),
                        expression(a, token, condition)?,
                        statement(a, token, increment)?.trim_end_matches(';')
                    ),
                );
                line(out, indent, "{");
                nodes(a, token, body, out, indent + 1, cfg)?;
                line(out, indent, "}");
            }
            Ast::Switch {
                value,
                arms,
                default,
            } => {
                line(
                    out,
                    indent,
                    &format!("switch ({})", expression(a, token, value)?),
                );
                line(out, indent, "{");
                for (i, arm) in arms.iter().enumerate() {
                    line(out, indent + 1, &format!("case {i}:"));
                    nodes(a, token, arm, out, indent + 2, cfg)?;
                }
                line(out, indent + 1, "default:");
                nodes(a, token, default, out, indent + 2, cfg)?;
                line(out, indent, "}");
            }
        }
    }
    Ok(())
}
fn flat_block(ir: &IrBlock, cfg: &ControlFlowGraph) -> Vec<Ast> {
    let edge = |e: &Edge| -> Vec<Ast> {
        let mut s = crate::ast::edge_assignments(e, cfg);
        s.push(Ast::Goto(e.target));
        s
    };
    let mut out = vec![Ast::Label(ir.id)];
    out.extend(ir.statements.iter().cloned().map(Ast::Statement));
    out.push(match &ir.terminator {
        Terminator::Jump(e) => {
            out.extend(edge(e));
            return out;
        }
        Terminator::Condition { condition, yes, no } => Ast::If {
            condition: condition.clone(),
            yes: edge(yes),
            no: edge(no),
        },
        Terminator::Switch {
            value,
            arms,
            default,
        } => Ast::Switch {
            value: value.clone(),
            arms: arms.iter().map(edge).collect(),
            default: edge(default),
        },
        Terminator::Return(v) => Ast::Return(v.clone()),
        Terminator::Throw(v) => Ast::Throw(v.clone()),
        Terminator::EndFinally => return out,
    });
    out
}
// Simple contiguous EH groups are rendered with explicit labels; nested/overlapping regions fall back.
fn exception_body(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    out: &mut String,
) -> Result<()> {
    let mut groups: BTreeMap<(u32, u32), Vec<_>> = BTreeMap::new();
    for e in &body.exceptions {
        if e.kind == "filter" || e.kind == "fault" {
            return Err(Error::limitation(
                "Exception filters/fault handlers remain in IL",
            ));
        }
        groups.entry((e.try_start, e.try_end)).or_default().push(e);
    }
    let mut previous_end = 0;
    for ((start, end), handlers) in &mut groups {
        handlers.sort_by_key(|e| e.handler_start);
        if *start < previous_end {
            return Err(Error::limitation(
                "Nested or overlapping exception regions require further structuring",
            ));
        }
        let mut next = *end;
        for e in handlers {
            if e.handler_start != next {
                return Err(Error::limitation(
                    "Noncontiguous exception layout requires further structuring",
                ));
            }
            next = e.handler_end;
        }
        previous_end = next;
    }
    for b in &ir.blocks {
        let mut indent = 1;
        for ((start, end), handlers) in &groups {
            if b.offset == *start {
                line(out, 1, "try");
                line(out, 1, "{");
            }
            if b.offset >= *start && b.offset < *end {
                indent = 2;
            }
            for e in handlers {
                if b.offset == e.handler_start {
                    line(out, 1, "}");
                    if e.kind == "finally" {
                        line(out, 1, "finally");
                    } else {
                        line(
                            out,
                            1,
                            &format!(
                                "catch ({} exception_{:04X})",
                                a.resolve(e.catch_type.unwrap_or(0)),
                                e.handler_start
                            ),
                        );
                    }
                    line(out, 1, "{");
                    if e.kind == "catch" {
                        line(
                            out,
                            2,
                            &format!(
                                "stack_{:04X}_0 = exception_{:04X};",
                                e.handler_start, e.handler_start
                            ),
                        );
                    }
                }
                if b.offset >= e.handler_start && b.offset < e.handler_end {
                    indent = 2;
                }
            }
            if handlers.last().is_some_and(|e| e.handler_end == b.offset) {
                line(out, 1, "}");
            }
        }
        nodes(a, token, &flat_block(b, cfg), out, indent, cfg)?;
    }
    if groups
        .values()
        .any(|h| h.last().is_some_and(|e| e.handler_end == body.code_size))
    {
        line(out, 1, "}");
    }
    Ok(())
}
pub fn csharp(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    ast: &[Ast],
) -> Result<String> {
    let mut out = String::from(
        "// Reconstructed from CIL and metadata; local names are synthetic.\n// Integer arithmetic follows unchecked CIL unless checked is explicit.\n",
    );
    line(&mut out, 0, &a.declaration(token)?);
    line(&mut out, 0, "{");
    for (i, t) in ir.locals.iter().enumerate() {
        line(
            &mut out,
            1,
            &format!(
                "{} local{i}{};",
                type_name(a, t, token),
                if body.init_locals {
                    format!(" = default({})", type_name(a, t, token))
                } else {
                    String::new()
                }
            ),
        );
    }
    for b in &cfg.blocks {
        if let Some(entry) = &ir.stack.entry_stacks[b.id] {
            for (i, t) in entry.iter().enumerate() {
                line(
                    &mut out,
                    1,
                    &format!("{} stack_{:04X}_{i};", type_name(a, t, token), b.start),
                );
            }
        }
    }
    if !ir.locals.is_empty() {
        out.push('\n');
    }
    if body.exceptions.is_empty() {
        nodes(a, token, ast, &mut out, 1, cfg)?;
    } else {
        exception_body(a, token, body, ir, cfg, &mut out)?;
    }
    line(&mut out, 0, "}");
    if out.len() > 4 * 1024 * 1024 {
        return Err(Error::limit("Reconstructed method exceeds 4 MiB"));
    }
    Ok(out)
}
pub fn il(a: &Assembly, token: u32, body: &MethodBody) -> String {
    let mut out = format!(
        "// {}\n// token 0x{token:08X}\n.maxstack {}\n",
        safe_comment(&a.resolve(token)),
        body.max_stack
    );
    if let Ok(locals) = a.locals(body)
        && !locals.is_empty()
    {
        let _ = writeln!(
            out,
            ".locals {}({})",
            if body.init_locals { "init " } else { "" },
            locals
                .iter()
                .enumerate()
                .map(|(i, t)| format!("[{i}] {} local{i}", a.format_type(t, token)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for i in &body.instructions {
        let operand = match &i.operand {
            Operand::None => String::new(),
            Operand::Integer(v) => v.to_string(),
            Operand::Float(v) => v.clone(),
            Operand::Variable(v) => v.to_string(),
            Operand::Token(t) => format!(
                "{} /* 0x{t:08X} */",
                safe_comment(i.resolved.as_deref().unwrap_or("unresolved"))
            ),
            Operand::Branch(t) => format!("IL_{t:04X}"),
            Operand::Switch(ts) => format!(
                "({})",
                ts.iter()
                    .map(|t| format!("IL_{t:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let _ = writeln!(out, "IL_{:04X}:  {:<14} {}", i.offset, i.name, operand);
    }
    for e in &body.exceptions {
        let _ = writeln!(
            out,
            "// .try IL_{:04X}..IL_{:04X} {} {} handler IL_{:04X}..IL_{:04X}{}",
            e.try_start,
            e.try_end,
            e.kind,
            e.catch_type.map(|t| a.resolve(t)).unwrap_or_default(),
            e.handler_start,
            e.handler_end,
            e.filter_start
                .map(|f| format!(" filter IL_{f:04X}"))
                .unwrap_or_default()
        );
    }
    out
}
