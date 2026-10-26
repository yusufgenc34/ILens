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
    let mut headers_end = r.pos;
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in replaced {
        if !seen.insert(*name) {
            return Err(Error::metadata("Duplicate replacement stream"));
        }
        if !streams.iter().any(|s| s.name == name.as_bytes()) {
            // Protected assemblies can omit #US entirely. Creating it does not
            // change table widths or any existing heap offset/token.
            if *name != "#US" || streams.len() >= 32 {
                return Err(Error::metadata(format!(
                    "Cannot add metadata stream {name}"
                )));
            }
            headers_end += 8 + (name.len() + 1).next_multiple_of(4);
            streams.push(StreamHeader {
                offset: 0,
                size: 0,
                name: name.as_bytes().to_vec(),
            });
        }
    }
    let mut out = metadata[..headers_at].to_vec();
    out[headers_at - 2..headers_at].copy_from_slice(&(streams.len() as u16).to_le_bytes());
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

/// Old → new row numbers after member rows were inserted: per table, rows from `at`
/// on move down by `count`; re-sorted tables carry a full permutation.
#[derive(Debug, Default, Clone)]
pub struct Remap {
    shifts: std::collections::HashMap<usize, (u32, u32)>,
    permutations: std::collections::HashMap<usize, Vec<u32>>,
}
impl Remap {
    pub fn row(&self, table: usize, row: u32) -> u32 {
        if row == 0 {
            return 0;
        }
        if let Some(order) = self.permutations.get(&table) {
            return order.get(row as usize - 1).copied().unwrap_or(row);
        }
        match self.shifts.get(&table) {
            Some((at, count)) if row >= *at => row + count,
            _ => row,
        }
    }
    /// A metadata token after the insertion.
    pub fn token(&self, token: u32) -> u32 {
        (token & 0xff00_0000) | self.row((token >> 24) as usize, token & 0xff_ffff)
    }
    pub fn moves(&self, table: usize) -> bool {
        self.shifts.contains_key(&table) || self.permutations.contains_key(&table)
    }
}
/// Sorted tables and the column each is ordered by (ECMA-335 II.22).
const SORTED: &[(usize, usize)] = &[
    (9, 0),
    (11, 1),
    (12, 0),
    (13, 0),
    (14, 1),
    (15, 2),
    (16, 1),
    (24, 2),
    (25, 0),
    (28, 1),
    (29, 1),
    (41, 0),
    (42, 2),
    (44, 0),
];
impl Tables {
    /// Rewrites every Table/Coded reference to a moved table; owner list columns are
    /// handled by the caller, which knows whose range grew.
    fn apply(&mut self, remap: &Remap) {
        use crate::metadata::Column;
        for table in 0..45 {
            let columns = crate::metadata::schema(table);
            for row in &mut self.rows[table] {
                for (c, value) in columns.iter().zip(row.iter_mut()) {
                    match *c {
                        Column::Table(t) if remap.moves(t as usize) => {
                            *value = remap.row(t as usize, *value);
                        }
                        Column::Coded(k) if *value != 0 => {
                            let bits = k.tag_bits();
                            let tag = *value & ((1 << bits) - 1);
                            if let Some(Some(t)) =
                                crate::metadata::coded_tables(k).get(tag as usize)
                                && remap.moves(*t as usize)
                            {
                                *value = (remap.row(*t as usize, *value >> bits) << bits) | tag;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    /// Restores the order of sorted tables. GenericParam and DeclSecurity rows are
    /// referenced by other tables, so their permutation is applied as well.
    fn resort(&mut self) {
        for &(table, key) in SORTED {
            let mut order: Vec<usize> = (0..self.rows[table].len()).collect();
            let rows = &self.rows[table];
            // GenericParam is ordered by owner, then number.
            order.sort_by_key(|i| (rows[*i][key], if table == 42 { rows[*i][0] } else { 0 }));
            if order.iter().enumerate().all(|(new, old)| new == *old) {
                continue;
            }
            self.rows[table] = order.iter().map(|i| rows[*i].clone()).collect();
            if matches!(table, 14 | 42) {
                let mut permutation = vec![0u32; order.len()];
                for (new, old) in order.iter().enumerate() {
                    permutation[*old] = new as u32 + 1;
                }
                let mut remap = Remap::default();
                remap.permutations.insert(table, permutation);
                self.apply(&remap);
            }
        }
    }
    /// The end of `owner`'s range in a list column: the next owner's start.
    fn range_end(&self, owner_table: usize, column: usize, owner: u32, listed: usize) -> u32 {
        self.rows[owner_table]
            .get(owner as usize)
            .map_or(self.rows[listed].len() as u32 + 1, |next| next[column])
    }
    /// Inserts a field row at the end of TypeDef `owner`'s field list.
    pub fn add_field(&mut self, owner: u32, row: Vec<u32>) -> Result<(u32, Remap)> {
        let at = self.range_end(2, 4, owner, 4);
        self.rows[4].insert(at as usize - 1, row);
        self.counts[4] += 1;
        self.valid |= 1 << 4;
        let mut remap = Remap::default();
        remap.shifts.insert(4, (at, 1));
        for (i, t) in self.rows[2].iter_mut().enumerate() {
            if i as u32 + 1 > owner && t[4] >= at {
                t[4] += 1;
            }
        }
        self.apply(&remap);
        self.resort();
        Ok((at, remap))
    }
    /// Inserts a method row with its parameter rows at the end of TypeDef `owner`'s
    /// method list; `row`'s ParamList is filled in.
    pub fn add_method(
        &mut self,
        owner: u32,
        mut row: Vec<u32>,
        params: Vec<Vec<u32>>,
    ) -> Result<(u32, Remap)> {
        let at = self.range_end(2, 5, owner, 6);
        // The new method's parameters go where the next method's begin.
        let param_at = self.rows[6]
            .get(at as usize - 1)
            .map_or(self.rows[8].len() as u32 + 1, |next| next[5]);
        let count = params.len() as u32;
        for (i, p) in params.into_iter().enumerate() {
            self.rows[8].insert(param_at as usize - 1 + i, p);
        }
        row[5] = param_at;
        self.rows[6].insert(at as usize - 1, row);
        self.counts[6] += 1;
        self.counts[8] += count;
        self.valid |= 1 << 6;
        if count > 0 {
            self.valid |= 1 << 8;
        }
        let mut remap = Remap::default();
        remap.shifts.insert(6, (at, 1));
        if count > 0 {
            remap.shifts.insert(8, (param_at, count));
        }
        for (i, t) in self.rows[2].iter_mut().enumerate() {
            if i as u32 + 1 > owner && t[5] >= at {
                t[5] += 1;
            }
        }
        // Methods after the new one start their parameters later.
        for m in self.rows[6].iter_mut().skip(at as usize) {
            m[5] += count;
        }
        self.apply(&remap);
        self.resort();
        Ok((at, remap))
    }
}
/// Appends a string to #Strings, reusing an existing occurrence (suffixes included).
pub fn append_string(heap: &mut Vec<u8>, text: &str) -> Result<u32> {
    if text.is_empty() {
        return Ok(0);
    }
    let needle = [text.as_bytes(), &[0]].concat();
    if let Some(at) = heap
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
    {
        return Ok(at as u32);
    }
    let offset = u32::try_from(heap.len()).map_err(|_| Error::limit("#Strings heap overflow"))?;
    heap.extend_from_slice(&needle);
    Ok(offset)
}
/// A token as the coded-index value of `column` in `table`.
pub fn encode_coded(table: usize, column: usize, token: u32) -> Result<u32> {
    let crate::metadata::Column::Coded(kind) = crate::metadata::schema(table)
        .get(column)
        .copied()
        .ok_or_else(|| Error::metadata("No such metadata column"))?
    else {
        return Err(Error::metadata("Column is not a coded index"));
    };
    let tag = crate::metadata::coded_tables(kind)
        .iter()
        .position(|t| *t == Some((token >> 24) as u8))
        .ok_or_else(|| Error::metadata("Token table is not allowed in this coded index"))?;
    Ok(((token & 0xff_ffff) << kind.tag_bits()) | tag as u32)
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
