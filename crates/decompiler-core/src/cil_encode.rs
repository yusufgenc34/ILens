//! Label-based CIL encoding. Floating operands can use exact IEEE bit patterns.
use crate::{
    cil::{self, MethodBody, Operand, OperandKind},
    error::{Error, ErrorCode, Result},
    opcodes::opcode,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditableInstruction {
    pub id: String,
    pub opcode: String,
    pub operand: String,
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidEdit, message)
}
pub fn from_body(body: &MethodBody) -> Vec<EditableInstruction> {
    body.instructions
        .iter()
        .map(|i| EditableInstruction {
            id: format!("IL_{:04X}", i.offset),
            opcode: i.name.clone(),
            operand: match &i.operand {
                Operand::None => String::new(),
                Operand::Integer(v) => v.to_string(),
                Operand::Variable(v) => v.to_string(),
                Operand::Token(t) => format!("0x{t:08X}"),
                Operand::Branch(t) => format!("IL_{t:04X}"),
                Operand::Switch(ts) => ts
                    .iter()
                    .map(|t| format!("IL_{t:04X}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                Operand::Float(_) => format!(
                    "bits:0x{}",
                    i.bytes
                        .split_whitespace()
                        .skip(usize::from(i.opcode > 255) + 1)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<String>()
                ),
            },
        })
        .collect()
}
pub fn catalog() -> Vec<(String, String)> {
    (0..=255)
        .chain(0xfe00..=0xfeff)
        .filter_map(|v| opcode(v).map(|o| (o.name.to_owned(), format!("{:?}", o.operand))))
        .collect()
}
fn number(text: &str) -> Result<i128> {
    let text = text.trim();
    if let Some(n) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        i128::from_str_radix(n, 16)
    } else {
        text.parse::<i128>()
    }
    .map_err(|_| invalid(format!("Invalid integer operand: {text}")))
}
fn width(kind: OperandKind, operand: &str) -> Result<usize> {
    use OperandKind::*;
    Ok(match kind {
        InlineNone => 0,
        ShortInlineVar | ShortInlineI | ShortInlineBrTarget => 1,
        InlineVar => 2,
        InlineI8 | InlineR => 8,
        InlineSwitch => 4 + targets(operand)?.len() * 4,
        _ => 4,
    })
}
fn targets(text: &str) -> Result<Vec<&str>> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let targets: Vec<_> = text.split(',').map(str::trim).collect();
    if targets.len() > 16384 || targets.iter().any(|s| s.is_empty()) {
        return Err(invalid("Invalid switch label list"));
    }
    Ok(targets)
}
fn checked_num(text: &str, min: i128, max: i128) -> Result<i128> {
    let n = number(text)?;
    if n < min || n > max {
        return Err(invalid(format!("Operand {text} is outside {min}..{max}")));
    }
    Ok(n)
}
pub fn encode(rows: &[EditableInstruction]) -> Result<Vec<u8>> {
    if rows.is_empty() {
        return Err(invalid("A method must contain at least one instruction"));
    }
    if rows.len() > 10_000 {
        return Err(Error::limit(
            "IL editing supports at most 10,000 instructions",
        ));
    }
    let by_name: HashMap<_, _> = (0..=255)
        .chain(0xfe00..=0xfeff)
        .filter_map(|v| opcode(v).map(|o| (o.name, v)))
        .collect();
    let mut labels = HashMap::new();
    let mut values = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.id.is_empty()
            || row.id.len() > 64
            || !row
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || labels.insert(row.id.as_str(), index).is_some()
        {
            return Err(invalid(format!(
                "Invalid or duplicate instruction label: {}",
                row.id
            )));
        }
        if row.operand.len() > 256 * 1024 {
            return Err(Error::limit("IL operand text exceeds 256 KiB"));
        }
        values.push(
            *by_name
                .get(row.opcode.as_str())
                .ok_or_else(|| invalid(format!("Unknown opcode: {}", row.opcode)))?,
        );
    }
    let mut offsets = vec![0; rows.len() + 1];
    // Widening is monotonic; at most one widening per short branch.
    for pass in 0..=rows.len() {
        for (i, row) in rows.iter().enumerate() {
            offsets[i + 1] = offsets[i]
                + usize::from(values[i] > 255)
                + 1
                + width(
                    opcode(values[i])
                        .ok_or_else(|| invalid("Unknown opcode"))?
                        .operand,
                    &row.operand,
                )?;
        }
        if offsets[rows.len()] > cil::MAX_CODE {
            return Err(Error::limit("Encoded method exceeds 1 MiB"));
        }
        let mut changed = false;
        for (i, row) in rows.iter().enumerate() {
            if matches!(
                opcode(values[i])
                    .ok_or_else(|| invalid("Unknown opcode"))?
                    .operand,
                OperandKind::ShortInlineBrTarget
            ) {
                let target = *labels
                    .get(row.operand.trim())
                    .ok_or_else(|| invalid(format!("Unknown branch label: {}", row.operand)))?;
                let delta = offsets[target] as i64 - offsets[i + 1] as i64;
                if !(-128..=127).contains(&delta) {
                    values[i] = if values[i] == 0xde {
                        0xdd
                    } else {
                        values[i] + 0x0d
                    };
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
        if pass == rows.len() {
            return Err(invalid("Branch layout did not converge"));
        }
    }
    let mut out = Vec::with_capacity(offsets[rows.len()]);
    for (i, row) in rows.iter().enumerate() {
        let value = values[i];
        if value > 255 {
            out.push(0xfe);
        }
        out.push(value as u8);
        use OperandKind::*;
        match opcode(value)
            .ok_or_else(|| invalid("Unknown opcode"))?
            .operand
        {
            InlineNone => {
                if !row.operand.trim().is_empty() {
                    return Err(invalid(format!("{} takes no operand", row.opcode)));
                }
            }
            ShortInlineVar => out.push(checked_num(&row.operand, 0, 255)? as u8),
            InlineVar => {
                out.extend_from_slice(&(checked_num(&row.operand, 0, 65535)? as u16).to_le_bytes())
            }
            ShortInlineI => out.push(checked_num(
                &row.operand,
                if value == 0xfe12 { 0 } else { -128 },
                if value == 0xfe12 { 255 } else { 127 },
            )? as u8),
            InlineI => out.extend_from_slice(
                &(checked_num(&row.operand, i32::MIN as i128, i32::MAX as i128)? as i32)
                    .to_le_bytes(),
            ),
            InlineI8 => out.extend_from_slice(
                &(checked_num(&row.operand, i64::MIN as i128, i64::MAX as i128)? as i64)
                    .to_le_bytes(),
            ),
            InlineMethod | InlineField | InlineType | InlineString | InlineTok | InlineSig => out
                .extend_from_slice(
                    &(checked_num(&row.operand, 1, u32::MAX as i128)? as u32).to_le_bytes(),
                ),
            ShortInlineR | InlineR => {
                let small = value == 0x22;
                if let Some(bits) = row.operand.trim().strip_prefix("bits:") {
                    let bits = checked_num(
                        bits,
                        0,
                        if small {
                            u32::MAX as i128
                        } else {
                            u64::MAX as i128
                        },
                    )? as u64;
                    out.extend_from_slice(&bits.to_le_bytes()[..if small { 4 } else { 8 }]);
                } else if small {
                    out.extend_from_slice(
                        &row.operand
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| invalid("Invalid float32 operand"))?
                            .to_le_bytes(),
                    );
                } else {
                    out.extend_from_slice(
                        &row.operand
                            .trim()
                            .parse::<f64>()
                            .map_err(|_| invalid("Invalid float64 operand"))?
                            .to_le_bytes(),
                    );
                }
            }
            kind @ (ShortInlineBrTarget | InlineBrTarget) => {
                let target = *labels
                    .get(row.operand.trim())
                    .ok_or_else(|| invalid(format!("Unknown branch label: {}", row.operand)))?;
                let delta = offsets[target] as i32 - offsets[i + 1] as i32;
                if matches!(kind, ShortInlineBrTarget) {
                    out.push(delta as i8 as u8);
                } else {
                    out.extend_from_slice(&delta.to_le_bytes());
                }
            }
            InlineSwitch => {
                let ts = targets(&row.operand)?;
                out.extend_from_slice(&(ts.len() as u32).to_le_bytes());
                for t in ts {
                    let target = *labels
                        .get(t)
                        .ok_or_else(|| invalid(format!("Unknown switch label: {t}")))?;
                    out.extend_from_slice(
                        &(offsets[target] as i32 - offsets[i + 1] as i32).to_le_bytes(),
                    );
                }
            }
        }
    }
    cil::decode(&out).map_err(|e| invalid(e.detail))?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn body(code: &[u8]) -> MethodBody {
        MethodBody {
            max_stack: 8,
            init_locals: false,
            local_signature: 0,
            code_size: code.len() as u32,
            file_offset: 0,
            instructions: cil::decode(code).unwrap(),
            exceptions: vec![],
        }
    }
    #[test]
    fn exact_numeric_roundtrip() {
        let mut code = vec![0x21];
        code.extend_from_slice(&i64::MIN.to_le_bytes());
        code.push(0x26);
        code.push(0x22);
        code.extend_from_slice(&0x7fc12345u32.to_le_bytes());
        code.push(0x26);
        code.push(0x23);
        code.extend_from_slice(&0x8000000000000000u64.to_le_bytes());
        code.extend([0x26, 0x2a]);
        assert_eq!(encode(&from_body(&body(&code))).unwrap(), code);
    }
    #[test]
    fn widens_branches_and_checks_labels() {
        let mut rows = from_body(&body(&[0x2b, 0, 0x2a]));
        for i in 0..130 {
            rows.insert(
                1,
                EditableInstruction {
                    id: format!("new{i}"),
                    opcode: "nop".into(),
                    operand: String::new(),
                },
            );
        }
        let code = encode(&rows).unwrap();
        assert_eq!(code[0], 0x38);
        assert_eq!(cil::decode(&code).unwrap()[0].targets(), vec![135]);
        rows.pop();
        assert!(encode(&rows).is_err());
    }
    #[test]
    fn switch_roundtrip_and_operand_bounds() {
        let code = [0x45, 1, 0, 0, 0, 0, 0, 0, 0, 0x2a];
        assert_eq!(encode(&from_body(&body(&code))).unwrap(), code);
        assert!(
            encode(&[EditableInstruction {
                id: "a".into(),
                opcode: "ldc.i4.s".into(),
                operand: "128".into()
            }])
            .is_err()
        );
    }
}
