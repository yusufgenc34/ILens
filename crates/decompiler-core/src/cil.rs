//! CIL is decoded independently of reconstruction. Offsets are relative to code start.
use crate::{
    error::{Error, Result},
    opcodes::opcode,
    pe::Image,
    reader::*,
};
use serde::Serialize;
use std::collections::BTreeSet;
pub const MAX_CODE: usize = 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Flow {
    Next,
    Break,
    Call,
    Return,
    Branch,
    CondBranch,
    Throw,
    Meta,
}
#[derive(Debug, Clone, Copy)]
pub enum OperandKind {
    InlineNone,
    ShortInlineVar,
    InlineVar,
    ShortInlineI,
    InlineI,
    InlineI8,
    ShortInlineR,
    InlineR,
    ShortInlineBrTarget,
    InlineBrTarget,
    InlineSwitch,
    InlineMethod,
    InlineField,
    InlineType,
    InlineString,
    InlineTok,
    InlineSig,
}
#[derive(Debug, Clone, Copy)]
pub struct Op {
    pub name: &'static str,
    pub operand: OperandKind,
    pub flow: Flow,
    pub pop: i8,
    pub push: i8,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Operand {
    None,
    Integer(#[serde(serialize_with = "integer_text")] i64),
    Float(String),
    Variable(u16),
    Token(u32),
    Branch(u32),
    Switch(Vec<u32>),
}
#[derive(Debug, Clone, Serialize)]
pub struct Instruction {
    pub offset: u32,
    pub size: u32,
    pub opcode: u16,
    pub name: String,
    pub operand: Operand,
    pub resolved: Option<String>,
    pub bytes: String,
    #[serde(skip)]
    pub op: Op,
}
impl Instruction {
    pub fn targets(&self) -> Vec<u32> {
        match &self.operand {
            Operand::Branch(t) => vec![*t],
            Operand::Switch(t) => t.clone(),
            _ => vec![],
        }
    }
    pub fn token(&self) -> Option<u32> {
        if let Operand::Token(t) = self.operand {
            Some(t)
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ExceptionRegion {
    pub kind: String,
    pub try_start: u32,
    pub try_end: u32,
    pub handler_start: u32,
    pub handler_end: u32,
    pub catch_type: Option<u32>,
    pub filter_start: Option<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MethodBody {
    pub max_stack: u16,
    pub init_locals: bool,
    pub local_signature: u32,
    pub code_size: u32,
    pub file_offset: usize,
    pub instructions: Vec<Instruction>,
    pub exceptions: Vec<ExceptionRegion>,
}
fn target(base: usize, delta: i32, len: usize) -> Result<u32> {
    let p = base as i64 + delta as i64;
    if p < 0 || p >= len as i64 {
        Err(Error::cil(format!("Branch target {p} outside method")))
    } else {
        Ok(p as u32)
    }
}
pub fn decode(code: &[u8]) -> Result<Vec<Instruction>> {
    if code.len() > MAX_CODE {
        return Err(Error::limit("Method exceeds 1 MiB of CIL"));
    }
    let mut r = Reader::new(code);
    let mut instructions = Vec::new();
    while r.pos < code.len() {
        if instructions.len() >= 100_000 {
            return Err(Error::limit("Method exceeds 100,000 instructions"));
        }
        let offset = r.pos;
        let first = r.u8()?;
        let value = if first == 0xfe {
            0xfe00 | r.u8()? as u16
        } else {
            first as u16
        };
        let op = opcode(value)
            .ok_or_else(|| Error::cil(format!("Unknown opcode {value:#x} at IL_{offset:04x}")))?;
        use OperandKind::*;
        let operand = match op.operand {
            InlineNone => Operand::None,
            ShortInlineVar => Operand::Variable(r.u8()? as u16),
            InlineVar => Operand::Variable(r.u16()?),
            ShortInlineI => Operand::Integer(if value == 0xfe12 {
                r.u8()? as i64
            } else {
                r.u8()? as i8 as i64
            }),
            InlineI => Operand::Integer(r.u32()? as i32 as i64),
            InlineI8 => Operand::Integer(r.u64()? as i64),
            ShortInlineR => Operand::Float(f32::from_bits(r.u32()?).to_string()),
            InlineR => Operand::Float(f64::from_bits(r.u64()?).to_string()),
            InlineMethod | InlineField | InlineType | InlineString | InlineTok | InlineSig => {
                Operand::Token(r.u32()?)
            }
            ShortInlineBrTarget => {
                let d = r.u8()? as i8 as i32;
                Operand::Branch(target(r.pos, d, code.len())?)
            }
            InlineBrTarget => {
                let d = r.u32()? as i32;
                Operand::Branch(target(r.pos, d, code.len())?)
            }
            InlineSwitch => {
                let n = r.u32()? as usize;
                if n > 16384 {
                    return Err(Error::limit("Switch exceeds 16,384 targets"));
                }
                slice(code, r.pos, n * 4)?;
                let base = r.pos + n * 4;
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(target(base, r.u32()? as i32, code.len())?);
                }
                Operand::Switch(v)
            }
        };
        instructions.push(Instruction {
            offset: offset as u32,
            size: (r.pos - offset) as u32,
            opcode: value,
            name: op.name.to_owned(),
            operand,
            resolved: None,
            bytes: hex(&code[offset..r.pos]),
            op,
        });
    }
    let starts: BTreeSet<_> = instructions.iter().map(|i| i.offset).collect();
    for i in &instructions {
        for t in i.targets() {
            if !starts.contains(&t) {
                return Err(Error::cil(format!("Branch into operand at IL_{t:04x}")));
            }
        }
    }
    // A branch may only enter the first prefix in a prefix sequence.
    let mut prefix = false;
    let targeted: BTreeSet<_> = instructions.iter().flat_map(|i| i.targets()).collect();
    for i in &instructions {
        if prefix && targeted.contains(&i.offset) {
            return Err(Error::cil(
                "Branch enters the middle of a prefixed instruction",
            ));
        }
        prefix = i.op.flow == Flow::Meta;
    }
    if prefix {
        return Err(Error::cil("Dangling instruction prefix"));
    }
    Ok(instructions)
}
pub fn read_body(image: &Image, bytes: &[u8], rva: u32) -> Result<MethodBody> {
    read_body_inner(image, bytes, rva).map_err(|e| {
        if e.code == crate::error::ErrorCode::SizeLimit {
            e
        } else {
            Error::cil(e.detail)
        }
    })
}
fn read_body_inner(image: &Image, bytes: &[u8], rva: u32) -> Result<MethodBody> {
    let p = image.range(bytes, rva, 1)?.start;
    let first = bytes[p];
    let (header_size, code_size, max_stack, local_signature, flags) = match first & 3 {
        2 => (1, (first >> 2) as usize, 8, 0, 2),
        3 => {
            let range = image.range(bytes, rva, 12)?;
            let h = &bytes[range];
            let f = u16_at(h, 0)?;
            let size = ((f >> 12) as usize) * 4;
            if size != 12 {
                return Err(Error::cil("Unsupported fat header size"));
            }
            (
                size,
                u32_at(h, 4)? as usize,
                u16_at(h, 2)?,
                u32_at(h, 8)?,
                f,
            )
        }
        _ => return Err(Error::cil("Invalid tiny/fat method header")),
    };
    if code_size == 0 {
        return Err(Error::cil("Empty method body"));
    }
    if code_size > MAX_CODE {
        return Err(Error::limit("Method exceeds 1 MiB of CIL"));
    }
    if max_stack > 1024 {
        return Err(Error::limit("Method maxstack exceeds 1024"));
    }
    let body_range = image.range(bytes, rva, header_size + code_size)?;
    let instructions = decode(&bytes[p + header_size..body_range.end])?;
    let mut exceptions = Vec::new();
    if flags & 8 != 0 {
        let mut section_rva = rva
            .checked_add((header_size + code_size) as u32)
            .and_then(|x| x.checked_add(3))
            .ok_or_else(|| Error::cil("Section overflow"))?
            & !3;
        for section in 0..32 {
            let header = image.range(bytes, section_rva, 4)?;
            let h = &bytes[header];
            let kind = h[0];
            if kind & 0x3f != 1 {
                return Err(Error::cil("Unknown method data section"));
            }
            let fat = kind & 0x40 != 0;
            let size = if fat {
                (u32_at(h, 0)? >> 8) as usize
            } else {
                h[1] as usize
            };
            let width = if fat { 24 } else { 12 };
            if size < 4 || (size - 4) % width != 0 {
                return Err(Error::cil("Invalid exception section size"));
            }
            if (size - 4) / width + exceptions.len() > 4096 {
                return Err(Error::limit("Too many exception clauses"));
            }
            let range = image.range(bytes, section_rva, size)?;
            let mut r = Reader::new(&bytes[range.start + 4..range.end]);
            for _ in 0..(size - 4) / width {
                let f = if fat { r.u32()? } else { r.u16()? as u32 };
                let ts = if fat { r.u32()? } else { r.u16()? as u32 };
                let tl = if fat { r.u32()? } else { r.u8()? as u32 };
                let hs = if fat { r.u32()? } else { r.u16()? as u32 };
                let hl = if fat { r.u32()? } else { r.u8()? as u32 };
                let extra = r.u32()?;
                let te = ts
                    .checked_add(tl)
                    .ok_or_else(|| Error::cil("Exception try overflow"))?;
                let he = hs
                    .checked_add(hl)
                    .ok_or_else(|| Error::cil("Exception handler overflow"))?;
                if tl == 0 || hl == 0 || te > code_size as u32 || he > code_size as u32 {
                    return Err(Error::cil("Exception range outside method"));
                }
                let (kind, catch_type, filter_start) = match f {
                    0 => ("catch", Some(extra), None),
                    1 => ("filter", None, Some(extra)),
                    2 => ("finally", None, None),
                    4 => ("fault", None, None),
                    _ => return Err(Error::cil("Invalid exception flags")),
                };
                exceptions.push(ExceptionRegion {
                    kind: kind.to_owned(),
                    try_start: ts,
                    try_end: te,
                    handler_start: hs,
                    handler_end: he,
                    catch_type,
                    filter_start,
                });
            }
            if kind & 0x80 == 0 {
                break;
            }
            if section == 31 {
                return Err(Error::limit("Too many chained method sections"));
            }
            section_rva = section_rva
                .checked_add(size as u32)
                .and_then(|v| v.checked_add(3))
                .ok_or_else(|| Error::cil("Section offset overflow"))?
                & !3;
        }
    }
    let starts: BTreeSet<_> = instructions.iter().map(|i| i.offset).collect();
    for e in &exceptions {
        for boundary in [e.try_start, e.handler_start] {
            if !starts.contains(&boundary) {
                return Err(Error::cil("Exception starts within an operand"));
            }
        }
        for boundary in [e.try_end, e.handler_end] {
            if boundary != code_size as u32 && !starts.contains(&boundary) {
                return Err(Error::cil("Exception ends within an operand"));
            }
        }
        if let Some(f) = e.filter_start
            && (f >= e.handler_start || !starts.contains(&f))
        {
            return Err(Error::cil("Invalid filter offset"));
        }
    }
    Ok(MethodBody {
        max_stack,
        init_locals: flags & 0x10 != 0,
        local_signature,
        code_size: code_size as u32,
        file_offset: p,
        instructions,
        exceptions,
    })
}
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
fn integer_text<S: serde::Serializer>(
    value: &i64,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_i64_precision_at_the_serialization_boundary() {
        let mut code = vec![0x21];
        code.extend_from_slice(&i64::MAX.to_le_bytes());
        let instructions = decode(&code).unwrap();
        let json = serde_json::to_value(&instructions[0].operand).unwrap();
        assert_eq!(json["value"], "9223372036854775807");
    }
    #[test]
    fn instructions() {
        let il = decode(&[0x02, 0x03, 0x58, 0x2a]).unwrap();
        assert_eq!(il[2].name, "add");
        assert_eq!(il[3].offset, 3);
    }
    #[test]
    fn bad_targets() {
        assert!(decode(&[0x2b, 0xff]).is_err());
        assert!(decode(&[0x38, 0, 0, 0]).is_err());
        assert!(decode(&[0xfe]).is_err());
    }
    #[test]
    fn switches() {
        let il = decode(&[0x45, 1, 0, 0, 0, 0, 0, 0, 0, 0x2a]).unwrap();
        assert_eq!(il[0].targets(), vec![9]);
    }
}
