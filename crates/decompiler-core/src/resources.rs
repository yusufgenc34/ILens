//! Bounded .resources v2 inspection. No object deserialization, type loading, or execution.
use crate::{
    error::{Error, Result},
    reader::{Reader, slice},
};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize)]
pub struct ResourceEntry {
    pub index: usize,
    pub name: String,
    pub kind: String,
    pub size: usize,
    pub value: Option<String>,
}
#[derive(Serialize)]
pub struct Preview {
    pub size: usize,
    pub kind: String,
    pub text: Option<String>,
    pub hex: String,
    pub entries: Vec<ResourceEntry>,
    pub diagnostic: Option<String>,
}
struct Entry {
    name: String,
    start: usize,
    end: usize,
}

fn seven(r: &mut Reader<'_>) -> Result<usize> {
    let mut result = 0u32;
    for shift in [0, 7, 14, 21, 28] {
        let byte = r.u8()?;
        if shift == 28 && byte > 7 {
            return Err(Error::metadata("Invalid resource length"));
        }
        result |= (byte as u32 & 127) << shift;
        if byte & 128 == 0 {
            return Ok(result as usize);
        }
    }
    Err(Error::metadata("Invalid resource integer"))
}
fn string(r: &mut Reader<'_>) -> Result<String> {
    let length = seven(r)?;
    if length > 65536 {
        return Err(Error::limit("Resource string exceeds 64 KiB preview limit"));
    }
    String::from_utf8(r.take(length)?.to_vec())
        .map_err(|_| Error::metadata("Invalid resource UTF-8"))
}
fn entries(data: &[u8]) -> Result<(Vec<Entry>, Vec<String>)> {
    let mut r = Reader::new(data);
    if r.u32()? != 0xbeefcace {
        return Err(Error::metadata("Invalid .resources magic"));
    }
    if r.u32()? == 0 {
        return Err(Error::metadata("Invalid resource manager version"));
    }
    let skip = r.u32()? as usize;
    r.take(skip)?;
    if r.u32()? != 2 {
        return Err(Error::new(
            crate::error::ErrorCode::UnsupportedMetadata,
            "Only .resources format v2 is supported; raw download is available.",
        ));
    }
    let count = r.u32()? as usize;
    let types = r.u32()? as usize;
    if count > 4096 || types > 1024 {
        return Err(Error::limit(
            "Resource count/type table exceeds inspection limits",
        ));
    }
    let mut type_names = Vec::new();
    for _ in 0..types {
        type_names.push(string(&mut r)?);
    }
    r.take((8 - r.pos % 8) % 8)?;
    r.take(count * 4)?; // Name hash table; positions are validated independently.
    let mut positions = Vec::new();
    for _ in 0..count {
        positions.push(r.u32()? as usize);
    }
    let data_start = r.u32()? as usize;
    let names_start = r.pos;
    if data_start < names_start || data_start > data.len() {
        return Err(Error::metadata("Invalid resource data section"));
    }
    let names = slice(data, names_start, data_start - names_start)?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut name_bytes = 0;
    for position in positions {
        let mut name_reader = Reader::new(slice(
            names,
            position,
            names.len().saturating_sub(position),
        )?);
        let length = seven(&mut name_reader)?;
        name_bytes += length;
        if length % 2 != 0 || name_bytes > 1024 * 1024 {
            return Err(Error::metadata("Invalid or excessive resource names"));
        }
        let units: Vec<_> = name_reader
            .take(length)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let name = String::from_utf16(&units)
            .map_err(|_| Error::metadata("Invalid resource UTF-16 name"))?;
        if !seen.insert(name.clone()) {
            return Err(Error::metadata("Duplicate resource name"));
        }
        let relative = name_reader.u32()? as usize;
        let start = data_start
            .checked_add(relative)
            .filter(|p| *p < data.len())
            .ok_or_else(|| Error::metadata("Invalid resource value offset"))?;
        result.push(Entry {
            name,
            start,
            end: data.len(),
        });
    }
    let mut offsets: Vec<_> = result.iter().map(|e| e.start).collect();
    offsets.sort_unstable();
    offsets.dedup();
    for entry in &mut result {
        entry.end = offsets
            .iter()
            .copied()
            .find(|p| *p > entry.start)
            .unwrap_or(data.len());
    }
    Ok((result, type_names))
}
// Kind, textual preview, and optional byte/stream payload range in the record.
type ResourceValue = (String, Option<String>, Option<(usize, usize)>);
fn value(data: &[u8], types: &[String]) -> Result<ResourceValue> {
    let mut r = Reader::new(data);
    let code = seven(&mut r)?;
    let (kind, value) = match code {
        0 => ("null", Some("null".to_owned())),
        1 => ("string", Some(string(&mut r)?)),
        2 => {
            let value = r.u8()?;
            if value > 1 {
                return Err(Error::metadata("Invalid resource boolean"));
            }
            ("bool", Some((value != 0).to_string()))
        }
        3 => ("char", Some(format!("U+{:04X}", r.u16()?))),
        4 => ("byte", Some(r.u8()?.to_string())),
        5 => ("sbyte", Some((r.u8()? as i8).to_string())),
        6 => ("short", Some((r.u16()? as i16).to_string())),
        7 => ("ushort", Some(r.u16()?.to_string())),
        8 => ("int", Some((r.u32()? as i32).to_string())),
        9 => ("uint", Some(r.u32()?.to_string())),
        10 => ("long", Some((r.u64()? as i64).to_string())),
        11 => ("ulong", Some(r.u64()?.to_string())),
        12 => ("float", Some(f32::from_bits(r.u32()?).to_string())),
        13 => ("double", Some(f64::from_bits(r.u64()?).to_string())),
        14 => {
            r.take(16)?;
            (
                "decimal",
                Some("Decimal bits available in raw data".to_owned()),
            )
        }
        15 => (
            "DateTime",
            Some(format!("{} (encoded DateTime bits)", r.u64()?)),
        ),
        16 => ("TimeSpan", Some(format!("{} ticks", r.u64()? as i64))),
        32 | 33 => {
            let length = r.u32()? as usize;
            let start = r.pos;
            r.take(length)?;
            return Ok((
                if code == 32 { "byte[]" } else { "stream" }.to_owned(),
                None,
                Some((start, length)),
            ));
        }
        64.. => {
            return Ok((
                "serialized object".to_owned(),
                Some(format!(
                    "{} — object deserialization is disabled",
                    types
                        .get(code - 64)
                        .ok_or_else(|| Error::metadata("Invalid resource type index"))?
                )),
                None,
            ));
        }
        _ => return Err(Error::metadata("Unknown resource type code")),
    };
    Ok((kind.to_owned(), value, None))
}
pub fn preview(data: &[u8]) -> Result<Preview> {
    if data.len() > 4 * 1024 * 1024 {
        return Err(Error::limit("Resource preview exceeds 4 MiB"));
    }
    let hex = data
        .iter()
        .take(4096)
        .enumerate()
        .map(|(i, b)| {
            format!(
                "{}{b:02X}{}",
                if i % 16 == 0 {
                    format!("{i:08X}  ")
                } else {
                    String::new()
                },
                if i % 16 == 15 { "\n" } else { " " }
            )
        })
        .collect::<String>();
    let mut result = Preview {
        size: data.len(),
        kind: "binary".to_owned(),
        text: None,
        hex,
        entries: vec![],
        diagnostic: None,
    };
    if data.starts_with(&0xbeefcaceu32.to_le_bytes()) {
        let (entries, types) = entries(data)?;
        result.kind = "resources".to_owned();
        for (index, entry) in entries.iter().enumerate() {
            let (kind, value, _) = value(&data[entry.start..entry.end], &types)?;
            result.entries.push(ResourceEntry {
                index,
                name: entry.name.clone(),
                kind,
                size: entry.end - entry.start,
                value: value.map(|v| v.chars().take(256).collect()),
            });
        }
        result.diagnostic = Some("Primitive values and byte streams are inspected statically. Serialized objects and BAML are not deserialized or decompiled.".to_owned());
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 && &data[12..16] == b"IHDR"
    {
        let w = u32::from_be_bytes(
            data[16..20]
                .try_into()
                .map_err(|_| Error::metadata("Invalid PNG width"))?,
        );
        let h = u32::from_be_bytes(
            data[20..24]
                .try_into()
                .map_err(|_| Error::metadata("Invalid PNG height"))?,
        );
        if w > 0 && h > 0 && w <= 8192 && h <= 8192 && w as u64 * h as u64 <= 16_000_000 {
            result.kind = "image/png".to_owned();
        } else {
            result.diagnostic = Some(
                "Image dimensions exceed the preview limit; raw download is available.".to_owned(),
            );
        }
    } else if data.len() <= 65536 {
        let text = std::str::from_utf8(data).ok().filter(|s| {
            !s.chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        });
        if let Some(text) = text {
            result.kind = "text".to_owned();
            result.text = Some(text.to_owned());
        }
    }
    Ok(result)
}
pub fn entry_bytes(data: &[u8], index: usize) -> Result<Vec<u8>> {
    let (entries, types) = entries(data)?;
    let entry = entries
        .get(index)
        .ok_or_else(|| Error::metadata("Resource entry is out of range"))?;
    let payload = &data[entry.start..entry.end];
    let (kind, text, range) = value(payload, &types)?;
    if let Some((start, length)) = range {
        return Ok(slice(payload, start, length)?.to_vec());
    }
    if kind == "string" {
        return Ok(text.unwrap_or_default().into_bytes());
    }
    Ok(payload.to_vec())
}
