//! Security envelope and parser adapter. Third-party parsing only sees validated ranges.
use crate::{
    error::{Error, ErrorCode, Result},
    reader::*,
};
use clrmeta::{CodedIndexKind as K, Metadata};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum Column {
    U16,
    U32,
    String,
    Blob,
    Guid,
    Table(u8),
    List(u8),
    Coded(K),
}
use Column::*;
fn schema(id: usize) -> Vec<Column> {
    match id {
        0 => vec![U16, String, Guid, Guid, Guid],
        1 => vec![Coded(K::ResolutionScope), String, String],
        2 => vec![
            U32,
            String,
            String,
            Coded(K::TypeDefOrRef),
            List(4),
            List(6),
        ],
        3 => vec![Table(4)],
        4 => vec![U16, String, Blob],
        5 => vec![Table(6)],
        6 => vec![U32, U16, U16, String, Blob, List(8)],
        7 => vec![Table(8)],
        8 => vec![U16, U16, String],
        9 => vec![Table(2), Coded(K::TypeDefOrRef)],
        10 => vec![Coded(K::MemberRefParent), String, Blob],
        11 => vec![U16, Coded(K::HasConstant), Blob],
        12 => vec![
            Coded(K::HasCustomAttribute),
            Coded(K::CustomAttributeType),
            Blob,
        ],
        13 => vec![Coded(K::HasFieldMarshal), Blob],
        14 => vec![U16, Coded(K::HasDeclSecurity), Blob],
        15 => vec![U16, U32, Table(2)],
        16 => vec![U32, Table(4)],
        17 => vec![Blob],
        18 => vec![Table(2), List(20)],
        19 => vec![Table(20)],
        20 => vec![U16, String, Coded(K::TypeDefOrRef)],
        21 => vec![Table(2), List(23)],
        22 => vec![Table(23)],
        23 => vec![U16, String, Blob],
        24 => vec![U16, Table(6), Coded(K::HasSemantics)],
        25 => vec![Table(2), Coded(K::MethodDefOrRef), Coded(K::MethodDefOrRef)],
        26 => vec![String],
        27 => vec![Blob],
        28 => vec![U16, Coded(K::MemberForwarded), String, Table(26)],
        29 => vec![U32, Table(4)],
        30 => vec![U32, U32],
        31 => vec![U32],
        32 => vec![U32, U16, U16, U16, U16, U32, Blob, String, String],
        33 => vec![U32],
        34 => vec![U32, U32, U32],
        35 => vec![U16, U16, U16, U16, U32, Blob, String, String, Blob],
        36 => vec![U32, Table(35)],
        37 => vec![U32, U32, U32, Table(35)],
        38 => vec![U32, String, Blob],
        39 => vec![U32, U32, String, String, Coded(K::Implementation)],
        40 => vec![U32, U32, String, Coded(K::Implementation)],
        41 => vec![Table(2), Table(2)],
        42 => vec![U16, U16, Coded(K::TypeOrMethodDef), String],
        43 => vec![Coded(K::MethodDefOrRef), Blob],
        44 => vec![Table(42), Coded(K::TypeDefOrRef)],
        _ => vec![],
    }
}
fn coded_tables(k: K) -> Vec<Option<u8>> {
    // ECMA-335 HasCustomAttribute tag 8 is DeclSecurity (clrmeta 0.1 calls it unused).
    let mut tables: Vec<_> = k.tables().iter().map(|v| v.map(|t| t as u8)).collect();
    if k == K::HasCustomAttribute {
        tables[8] = Some(14);
    }
    tables
}
fn width(c: Column, heaps: u8, counts: &[u32; 64]) -> usize {
    match c {
        U16 => 2,
        U32 => 4,
        String => {
            if heaps & 1 != 0 {
                4
            } else {
                2
            }
        }
        Guid => {
            if heaps & 2 != 0 {
                4
            } else {
                2
            }
        }
        Blob => {
            if heaps & 4 != 0 {
                4
            } else {
                2
            }
        }
        Table(t) | List(t) => {
            if counts[t as usize] >= 65536 {
                4
            } else {
                2
            }
        }
        Coded(k) => {
            if coded_tables(k)
                .iter()
                .flatten()
                .any(|t| counts[*t as usize] >= k.max_small_rows())
            {
                4
            } else {
                2
            }
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Stream {
    pub name: std::string::String,
    pub offset: usize,
    pub size: usize,
}
#[derive(Debug, Clone)]
pub struct TableLayout {
    pub offset: usize,
    pub width: usize,
    pub count: u32,
}
pub struct MetadataStore {
    pub parsed: Metadata,
    pub streams: Vec<Stream>,
    pub tables: Vec<TableLayout>,
}
impl MetadataStore {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.u32()? != 0x424a5342 {
            return Err(Error::metadata("Missing BSJB metadata signature"));
        }
        r.take(8)?;
        let version_len = r.u32()? as usize;
        if version_len > 256 || !version_len.is_multiple_of(4) {
            return Err(Error::metadata("Invalid runtime version length"));
        }
        r.take(version_len)?;
        r.u16()?;
        let stream_count = r.u16()?;
        if stream_count > 16 {
            return Err(Error::limit("More than 16 metadata streams"));
        }
        let mut streams = Vec::new();
        let mut seen = HashSet::new();
        for _ in 0..stream_count {
            let offset = r.u32()? as usize;
            let size = r.u32()? as usize;
            slice(bytes, offset, size)?;
            let name_start = r.pos;
            while r.u8()? != 0 {
                if r.pos - name_start > 31 {
                    return Err(Error::metadata("Metadata stream name exceeds 31 bytes"));
                }
            }
            let name = std::str::from_utf8(&bytes[name_start..r.pos - 1])
                .map_err(|_| Error::metadata("Non-UTF8 stream name"))?
                .to_owned();
            r.take((4 - r.pos % 4) % 4)?;
            if !seen.insert(name.clone()) {
                return Err(Error::metadata("Duplicate metadata stream"));
            }
            streams.push(Stream { name, offset, size });
        }
        let root_end = r.pos;
        let mut ranges: Vec<_> = streams.iter().filter(|s| s.size > 0).collect();
        ranges.sort_by_key(|s| s.offset);
        let mut end = root_end;
        for s in ranges {
            if s.offset < end {
                return Err(Error::metadata("Overlapping metadata streams"));
            }
            end = s.offset + s.size;
        }
        let heap = |name: &str| -> &[u8] {
            streams
                .iter()
                .find(|s| s.name == name)
                .map(|s| &bytes[s.offset..s.offset + s.size])
                .unwrap_or(&[])
        };
        let ts = streams
            .iter()
            .find(|s| s.name == "#~" || s.name == "#-")
            .ok_or_else(|| Error::metadata("No tables stream"))?;
        if streams
            .iter()
            .filter(|s| s.name == "#~" || s.name == "#-")
            .count()
            != 1
        {
            return Err(Error::metadata("Multiple tables streams"));
        }
        let mut r = Reader::new(&bytes[ts.offset..ts.offset + ts.size]);
        r.u32()?;
        let major = r.u8()?;
        r.u8()?;
        let heaps = r.u8()?;
        r.u8()?;
        if major != 2 || heaps & !7 != 0 {
            return Err(Error::new(
                ErrorCode::UnsupportedMetadata,
                "Unsupported tables version/heap flags (including edit-and-continue extra data)",
            ));
        }
        let valid = r.u64()?;
        r.u64()?;
        if valid >> 45 != 0 {
            return Err(Error::new(
                ErrorCode::UnsupportedMetadata,
                "Unknown metadata table (portable PDB is not an assembly)",
            ));
        }
        let mut counts = [0u32; 64];
        let mut total = 0u32;
        for (i, count) in counts.iter_mut().enumerate() {
            if valid & (1u64 << i) != 0 {
                *count = r.u32()?;
                total = total
                    .checked_add(*count)
                    .ok_or_else(|| Error::limit("Row count overflow"))?;
            }
        }
        if counts[35] > 4096 || counts[40] > 4096 || counts[32] > 1 {
            return Err(Error::limit(
                "Maximum references/resources: 4,096; assembly definitions: one",
            ));
        }
        if counts[2] > 10_000 {
            return Err(Error::limit("Maximum type definitions: 10,000"));
        }
        if total > 500_000 {
            return Err(Error::limit("Maximum total metadata rows: 500,000"));
        }
        if counts[2] + counts[4] + counts[6] + counts[20] + counts[23] > 150_000 {
            return Err(Error::limit("Maximum navigable declarations: 150,000"));
        }
        if [3, 5, 7, 19, 22].iter().any(|i| counts[*i] > 0) {
            return Err(Error::new(
                ErrorCode::UnsupportedMetadata,
                "Unoptimized pointer tables are not yet normalized",
            ));
        }
        let mut tables = Vec::new();
        let strings = heap("#Strings");
        let blobs = heap("#Blob");
        let guids = heap("#GUID");
        let mut checked_strings = HashSet::new();
        let mut checked_blobs = HashSet::new();
        for (id, count) in counts.iter().enumerate().take(45) {
            let columns = schema(id);
            let row_width: usize = columns.iter().map(|c| width(*c, heaps, &counts)).sum();
            slice(r.data, r.pos, row_width * *count as usize)?;
            tables.push(TableLayout {
                offset: ts.offset + r.pos,
                width: row_width,
                count: *count,
            });
            for row in 0..*count {
                for c in &columns {
                    let v = if width(*c, heaps, &counts) == 2 {
                        r.u16()? as u32
                    } else {
                        r.u32()?
                    };
                    let bad = || {
                        Error::metadata(format!(
                            "Invalid index {v:#x} in table {id:#x}, row {}",
                            row + 1
                        ))
                    };
                    match *c {
                        String if v != 0 && checked_strings.insert(v) => {
                            let s = strings.get(v as usize..).ok_or_else(bad)?;
                            let n = s.iter().take(4097).position(|b| *b == 0).ok_or_else(|| {
                                Error::limit("Metadata name exceeds 4096 bytes or is unterminated")
                            })?;
                            std::str::from_utf8(&s[..n]).map_err(|_| bad())?;
                        }
                        Blob if v != 0 && checked_blobs.insert(v) => {
                            let mut b = Reader::new(blobs.get(v as usize..).ok_or_else(bad)?);
                            let len = b.compressed()? as usize;
                            b.take(len)?;
                        }
                        Guid if v != 0 => {
                            if v as usize > guids.len() / 16 {
                                return Err(bad());
                            }
                        }
                        Table(t) => {
                            if v == 0 || v > counts[t as usize] {
                                return Err(bad());
                            }
                        }
                        List(t) => {
                            if v == 0 || v > counts[t as usize] + 1 {
                                return Err(bad());
                            }
                        }
                        Coded(k) if v != 0 => {
                            let bits = k.tag_bits();
                            let t = coded_tables(k)
                                .get((v & ((1 << bits) - 1)) as usize)
                                .copied()
                                .flatten()
                                .ok_or_else(bad)?;
                            if v >> bits == 0 || v >> bits > counts[t as usize] {
                                return Err(bad());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if counts[0] != 1 {
            return Err(Error::metadata("Exactly one Module row is required"));
        }
        let parsed = Metadata::parse(bytes)?;
        // Range partitions must be monotonic, not just individually in bounds.
        for pair in parsed.type_defs.windows(2) {
            if pair[0].field_list > pair[1].field_list || pair[0].method_list > pair[1].method_list
            {
                return Err(Error::metadata("Non-monotonic TypeDef member lists"));
            }
        }
        for pair in parsed.method_defs.windows(2) {
            if pair[0].param_list > pair[1].param_list {
                return Err(Error::metadata("Non-monotonic parameter list"));
            }
        }
        for pair in parsed.property_maps.windows(2) {
            if pair[0].property_list > pair[1].property_list {
                return Err(Error::metadata("Non-monotonic property list"));
            }
        }
        for pair in parsed.event_maps.windows(2) {
            if pair[0].event_list > pair[1].event_list {
                return Err(Error::metadata("Non-monotonic event list"));
            }
        }
        Ok(Self {
            parsed,
            streams,
            tables,
        })
    }
    pub fn raw_row<'a>(&self, metadata: &'a [u8], token: u32) -> Result<&'a [u8]> {
        let table = self
            .tables
            .get((token >> 24) as usize)
            .ok_or_else(|| Error::metadata("Invalid token table"))?;
        let row = token & 0x00ffffff;
        if row == 0 || row > table.count {
            return Err(Error::metadata("Invalid token row"));
        }
        slice(
            metadata,
            table.offset + (row as usize - 1) * table.width,
            table.width,
        )
    }
}
