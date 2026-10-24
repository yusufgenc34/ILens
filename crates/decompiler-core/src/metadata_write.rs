//! Metadata growth for edited assemblies. Existing heap offsets, table rows and tokens
//! never move: user strings and blobs are appended to their heaps, rows to free-standing
//! tables, and the streams are laid out again behind an unchanged metadata root.
use crate::{
    error::{Error, Result},
    reader::Reader,
};

/// Edits refer to user strings that do not exist yet through these placeholder tokens;
/// the writer replaces them with appended #US offsets.
pub const PENDING_STRING: u32 = 0x70F0_0000;
/// #US offsets are 24-bit token values and must stay below the placeholders.
pub const MAX_USER_STRINGS: usize = (PENDING_STRING & 0xff_ffff) as usize;

fn compressed(n: usize, out: &mut Vec<u8>) -> Result<()> {
    match n {
        0..=0x7f => out.push(n as u8),
        0x80..=0x3fff => out.extend_from_slice(&(0x8000 | n as u16).to_be_bytes()),
        0x4000..=0x1fff_ffff => out.extend_from_slice(&(0xc000_0000 | n as u32).to_be_bytes()),
        _ => return Err(Error::limit("Metadata blob exceeds 512 MiB")),
    }
    Ok(())
}

/// The #US encoding of `text`: compressed length, UTF-16LE characters and the trailing
/// byte that marks strings needing more than an ordinal comparison (ECMA-335 II.24.2.4).
pub fn user_string_entry(text: &str) -> Result<Vec<u8>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let special = units
        .iter()
        .any(|u| *u > 0xff || matches!(*u as u8, 0x01..=0x08 | 0x0e..=0x1f | 0x27 | 0x2d | 0x7f));
    let mut out = Vec::with_capacity(units.len() * 2 + 5);
    compressed(units.len() * 2 + 1, &mut out)?;
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.push(u8::from(special));
    Ok(out)
}

/// The token of an existing #US entry equal to `text`.
pub fn find_user_string(heap: &[u8], text: &str) -> Option<u32> {
    let wanted: Vec<u16> = text.encode_utf16().collect();
    let mut offset = 1;
    while offset < heap.len() {
        let mut r = Reader::new(&heap[offset..]);
        let n = r.compressed().ok()? as usize;
        let start = offset + r.pos;
        let bytes = heap.get(start..start + n)?;
        if n > 0 && n % 2 == 1 {
            let units = bytes[..n - 1]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]));
            if units.eq(wanted.iter().copied()) {
                return Some(0x7000_0000 | offset as u32);
            }
        }
        // Trailing zero padding ends the heap.
        if n == 0 && heap[start..].iter().all(|b| *b == 0) {
            return None;
        }
        offset = start + n;
    }
    None
}

struct StreamHeader {
    offset: usize,
    size: usize,
    name: Vec<u8>,
}

/// Metadata with some streams replaced (by name). Other streams are copied unchanged;
/// stream offsets follow the original order, 4-byte aligned, behind the same root.
pub fn rebuild(metadata: &[u8], replaced: &[(&str, Vec<u8>)]) -> Result<Vec<u8>> {
    let bad = || Error::metadata("Malformed metadata root");
    let mut r = Reader::new(metadata);
    if r.u32()? != 0x424a_5342 {
        return Err(bad());
    }
    r.take(8)?;
    let version_len = r.u32()? as usize;
    r.take(version_len)?;
    r.u16()?;
    let count = r.u16()? as usize;
    let headers_at = r.pos;
    let mut streams = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = r.u32()? as usize;
        let size = r.u32()? as usize;
        let name_start = r.pos;
        while r.u8()? != 0 {}
        let name = metadata[name_start..r.pos - 1].to_vec();
        r.take((4 - r.pos % 4) % 4)?;
        streams.push(StreamHeader { offset, size, name });
    }
    let headers_end = r.pos;
    for (name, _) in replaced {
        if !streams.iter().any(|s| s.name == name.as_bytes()) {
            return Err(Error::metadata(format!(
                "The assembly has no {name} stream"
            )));
        }
    }
    let mut out = metadata[..headers_at].to_vec();
    let mut data = Vec::new();
    let mut layout = Vec::with_capacity(count);
    for s in &streams {
        let original = metadata.get(s.offset..s.offset + s.size).ok_or_else(bad)?;
        let body = replaced
            .iter()
            .find(|(name, _)| name.as_bytes() == s.name)
            .map_or(original, |(_, bytes)| bytes.as_slice());
        let start = headers_end + data.len();
        data.extend_from_slice(body);
        data.resize(data.len().next_multiple_of(4), 0);
        layout.push((start, headers_end + data.len() - start));
    }
    for (s, (offset, size)) in streams.iter().zip(&layout) {
        out.extend_from_slice(&(*offset as u32).to_le_bytes());
        out.extend_from_slice(&(*size as u32).to_le_bytes());
        out.extend_from_slice(&s.name);
        out.push(0);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    debug_assert_eq!(out.len(), headers_end);
    out.extend_from_slice(&data);
    Ok(out)
}

/// The decoded `#~` stream: header fields and every row's column values.
pub struct Tables {
    header: [u8; 8],
    pub heaps: u8,
    valid: u64,
    sorted: u64,
    pub counts: [u32; 64],
    pub rows: Vec<Vec<Vec<u32>>>,
}
impl Tables {
    pub fn decode(stream: &[u8]) -> Result<Self> {
        let mut r = Reader::new(stream);
        let header: [u8; 8] = r
            .take(8)?
            .try_into()
            .map_err(|_| Error::metadata("Short tables header"))?;
        let heaps = header[6];
        let valid = r.u64()?;
        let sorted = r.u64()?;
        let mut counts = [0u32; 64];
        for (i, count) in counts.iter_mut().enumerate() {
            if valid & (1 << i) != 0 {
                *count = r.u32()?;
            }
        }
        let mut rows = vec![vec![]; 64];
        for (id, table) in rows.iter_mut().enumerate().take(45) {
            let columns = crate::metadata::schema(id);
            for _ in 0..counts[id] {
                let mut row = Vec::with_capacity(columns.len());
                for c in &columns {
                    row.push(if crate::metadata::width(*c, heaps, &counts) == 2 {
                        u32::from(r.u16()?)
                    } else {
                        r.u32()?
                    });
                }
                table.push(row);
            }
        }
        Ok(Self {
            header,
            heaps,
            valid,
            sorted,
            counts,
            rows,
        })
    }
    /// Appends a row to a table no other row ranges over; returns its token.
    pub fn push(&mut self, table: usize, row: Vec<u32>) -> Result<u32> {
        // Owner lists (TypeDef → Field/Method, MethodDef → Param, maps) and sorted
        // tables would need renumbering; only free-standing tables grow here.
        if !matches!(table, 1 | 10 | 17 | 27 | 35 | 43) {
            return Err(Error::new(
                crate::error::ErrorCode::UnsupportedEdit,
                "Rows can only be added to free-standing metadata tables",
            ));
        }
        if row.len() != crate::metadata::schema(table).len() {
            return Err(Error::metadata("Row does not match its table schema"));
        }
        self.rows[table].push(row);
        self.counts[table] += 1;
        self.valid |= 1 << table;
        Ok(((table as u32) << 24) | self.counts[table])
    }
    /// Heap index widths for heaps of these sizes (the flags only ever widen).
    pub fn widen_heaps(&mut self, strings: usize, guids: usize, blobs: usize) {
        for (bit, size) in [(1, strings), (2, guids), (4, blobs)] {
            if size >= 0x1_0000 {
                self.heaps |= bit;
            }
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.header.to_vec();
        out[6] = self.heaps;
        out.extend_from_slice(&self.valid.to_le_bytes());
        out.extend_from_slice(&self.sorted.to_le_bytes());
        for (i, count) in self.counts.iter().enumerate() {
            if self.valid & (1 << i) != 0 {
                out.extend_from_slice(&count.to_le_bytes());
            }
        }
        for (id, table) in self.rows.iter().enumerate().take(45) {
            let columns = crate::metadata::schema(id);
            for row in table {
                for (c, value) in columns.iter().zip(row) {
                    if crate::metadata::width(*c, self.heaps, &self.counts) == 2 {
                        out.extend_from_slice(&(*value as u16).to_le_bytes());
                    } else {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        out.resize(out.len().next_multiple_of(4), 0);
        out
    }
}

/// Appends `entry` to a blob heap and returns its offset.
pub fn append_blob(heap: &mut Vec<u8>, entry: &[u8]) -> Result<u32> {
    let offset = u32::try_from(heap.len()).map_err(|_| Error::limit("#Blob heap overflow"))?;
    compressed(entry.len(), heap)?;
    heap.extend_from_slice(entry);
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_string_entries_follow_ecma_encoding() {
        assert_eq!(user_string_entry("A").unwrap(), [3, b'A', 0, 0]);
        // An apostrophe needs the special-handling marker.
        assert_eq!(user_string_entry("'").unwrap(), [3, b'\'', 0, 1]);
        assert_eq!(user_string_entry("").unwrap(), [1, 0]);
        let long = "x".repeat(100);
        assert_eq!(&user_string_entry(&long).unwrap()[..2], &[0x80, 201]);
        let heap = [
            &[0u8][..],
            &user_string_entry("hi").unwrap(),
            &user_string_entry("A").unwrap(),
        ]
        .concat();
        assert_eq!(find_user_string(&heap, "A"), Some(0x7000_0007));
        assert_eq!(find_user_string(&heap, "missing"), None);
    }
}
