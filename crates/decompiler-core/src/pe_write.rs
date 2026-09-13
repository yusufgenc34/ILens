//! Managed PE writer with immutable input, unchanged metadata tokens and bounded layout.
//! Only unsigned x86/x64 IL-only images and known directories are accepted.
use crate::{
    assembly::Assembly,
    cil,
    edit::{self, ValidatedEdit},
    error::{Error, Result},
    reader::{slice, u16_at, u32_at},
};
use std::{collections::BTreeMap, ops::Range};
struct Layout {
    pe: usize,
    optional: usize,
    directories: usize,
    section_headers: usize,
    file_align: usize,
    section_align: usize,
    cli: Range<usize>,
    protected: Vec<Range<usize>>,
    debug: Vec<Range<usize>>,
}
fn unsupported(s: impl Into<String>) -> Error {
    edit::unsupported(s)
}
fn overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
fn align(n: usize, boundary: usize) -> Result<usize> {
    n.checked_add(boundary - 1)
        .map(|v| v & !(boundary - 1))
        .ok_or_else(|| unsupported("PE alignment overflow"))
}
fn put32(bytes: &mut [u8], p: usize, value: u32) -> Result<()> {
    let out = bytes
        .get_mut(p..p + 4)
        .ok_or_else(|| unsupported("PE write outside buffer"))?;
    out.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
fn put16(bytes: &mut [u8], p: usize, value: u16) -> Result<()> {
    let out = bytes
        .get_mut(p..p + 2)
        .ok_or_else(|| unsupported("PE write outside buffer"))?;
    out.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
fn preflight(a: &Assembly) -> Result<Layout> {
    if !a.metadata().field_rvas.is_empty() {
        return Err(unsupported(
            "Field-RVA storage needs size/alias analysis before this assembly can be rewritten",
        ));
    }
    let bytes = &a.bytes;
    let pe = u32_at(bytes, 0x3c)? as usize;
    let optional = pe + 24;
    let optional_size = u16_at(bytes, pe + 20)? as usize;
    if !matches!(a.image.machine, 0x14c | 0x8664) {
        return Err(unsupported(
            "Binary writing currently supports x86 and x64 managed PE images",
        ));
    }
    if a.info.identity.public_key_token != "null" || a.image.cli_flags & 8 != 0 {
        return Err(unsupported(
            "Strong-name signed or delay-signed assemblies cannot be rewritten yet",
        ));
    }
    if u32_at(bytes, pe + 12)? != 0 || u32_at(bytes, pe + 16)? != 0 {
        return Err(unsupported(
            "COFF symbol tables are not preserved by this writer",
        ));
    }
    let directories = optional + if a.image.pe64 { 112 } else { 96 };
    if u32_at(bytes, directories - 4)? != 16 || optional_size < directories - optional + 128 {
        return Err(unsupported(
            "Nonstandard PE optional header/directory layout",
        ));
    }
    let file_align = u32_at(bytes, optional + 36)? as usize;
    let section_align = u32_at(bytes, optional + 32)? as usize;
    if !(512..=65536).contains(&file_align)
        || !file_align.is_power_of_two()
        || !section_align.is_power_of_two()
        || section_align < file_align
        || section_align < 4096
    {
        return Err(unsupported("Nonstandard PE file/section alignment"));
    }
    let section_headers = optional + optional_size;
    let mut physical: Vec<_> = a
        .image
        .sections
        .iter()
        .filter(|s| s.size_of_raw_data > 0)
        .map(|s| {
            s.pointer_to_raw_data as usize
                ..s.pointer_to_raw_data as usize + s.size_of_raw_data as usize
        })
        .collect();
    physical.sort_by_key(|r| r.start);
    if physical.last().map(|r| r.end) != Some(bytes.len()) {
        return Err(unsupported(
            "PE overlays or external debug payloads are not supported for binary export",
        ));
    }
    let mut previous = a.image.size_of_headers as usize;
    for r in &physical {
        if r.start < previous || r.start % file_align != 0 || r.len() % file_align != 0 {
            return Err(unsupported("Overlapping or unaligned PE sections"));
        }
        previous = r.end;
    }
    let mut virtuals: Vec<_> = a
        .image
        .sections
        .iter()
        .map(|s| {
            s.virtual_address as usize
                ..s.virtual_address as usize + s.virtual_size.max(s.size_of_raw_data) as usize
        })
        .collect();
    virtuals.sort_by_key(|r| r.start);
    previous = align(a.image.size_of_headers as usize, section_align)?;
    for r in virtuals {
        if r.start < previous || r.start % section_align != 0 {
            return Err(unsupported("Overlapping or unaligned section RVAs"));
        }
        previous = align(r.end, section_align)?;
    }
    for s in &a.image.sections {
        if s.pointer_to_relocations != 0
            || s.pointer_to_linenumbers != 0
            || s.number_of_relocations != 0
            || s.number_of_linenumbers != 0
        {
            return Err(unsupported(
                "Section relocation/line-number file tables are unsupported",
            ));
        }
    }
    let mut protected = vec![
        0..a.image.size_of_headers as usize,
        a.image.metadata.clone(),
    ];
    let mut debug = Vec::new();
    let cli = a
        .image
        .range(bytes, u32_at(bytes, directories + 14 * 8)?, 72)?;
    if u32_at(bytes, cli.start)? != 72 || a.image.cli_flags & !(1 | 2 | 0x20000) != 0 {
        return Err(unsupported("Nonstandard CLI header/flags"));
    }
    for p in [32, 40, 48, 56, 64] {
        if u32_at(bytes, cli.start + p)? != 0 || u32_at(bytes, cli.start + p + 4)? != 0 {
            return Err(unsupported(
                "Strong names, vtable fixups, native exports and runtime headers cannot be rewritten",
            ));
        }
    }
    protected.push(cli.clone());
    if a.image.resources_size > 0 {
        protected.push(a.image.range(
            bytes,
            a.image.resources_rva,
            a.image.resources_size as usize,
        )?);
    }
    for i in 0..16 {
        let rva = u32_at(bytes, directories + i * 8)?;
        let size = u32_at(bytes, directories + i * 8 + 4)? as usize;
        if rva == 0 && size == 0 {
            continue;
        }
        if !matches!(i, 1 | 2 | 5 | 6 | 12 | 14) {
            return Err(unsupported(format!(
                "PE directory {i} is unsupported for writing (including Authenticode signatures)"
            )));
        }
        if rva == 0 || size == 0 {
            return Err(unsupported("Incomplete PE directory"));
        }
        let range = a.image.range(bytes, rva, size)?;
        if i != 6 {
            protected.push(range);
            continue;
        }
        if !size.is_multiple_of(28) || size > 28 * 128 {
            return Err(unsupported("Invalid/oversized debug directory"));
        }
        for p in (range.start..range.end).step_by(28) {
            let kind = u32_at(bytes, p + 12)?;
            let n = u32_at(bytes, p + 16)? as usize;
            let address = u32_at(bytes, p + 20)?;
            let pointer = u32_at(bytes, p + 24)? as usize;
            if !matches!(kind, 2 | 16 | 17 | 19) {
                return Err(unsupported(format!(
                    "Debug record type {kind} has no removal policy"
                )));
            }
            if n > 0 {
                let payload = a.image.range(bytes, address, n)?;
                if payload.start != pointer || overlap(&payload, &range) {
                    return Err(unsupported("Ambiguous debug payload location"));
                }
                debug.push(payload);
            }
        }
        debug.push(range);
    }
    for r in &debug {
        if protected.iter().any(|p| overlap(r, p)) {
            return Err(unsupported(
                "Debug information overlaps preserved metadata or PE directories",
            ));
        }
    }
    Ok(Layout {
        pe,
        optional,
        directories,
        section_headers,
        file_align,
        section_align,
        cli,
        protected,
        debug,
    })
}
pub fn capability(a: &Assembly) -> Result<()> {
    preflight(a).map(|_| ())
}
fn header_and_code(a: &Assembly, token: u32) -> Result<Range<usize>> {
    let rva = a.method_row(token)?.rva;
    let start = a.image.range(&a.bytes, rva, 1)?.start;
    let first = a.bytes[start];
    let header = if first & 3 == 2 { 1 } else { 12 };
    let size = if header == 1 {
        (first >> 2) as usize
    } else {
        u32_at(&a.bytes, start + 4)? as usize
    };
    a.image.range(&a.bytes, rva, header + size)
}
fn body_bytes(a: &Assembly, token: u32, edit: &ValidatedEdit) -> Result<Vec<u8>> {
    let original = a
        .body(token)?
        .ok_or_else(|| unsupported("Cannot write a bodyless member"))?;
    let mut out = Vec::new();
    if edit.code.len() < 64
        && edit.max_stack <= 8
        && original.local_signature == 0
        && !original.init_locals
    {
        out.push(((edit.code.len() as u8) << 2) | 2);
    } else {
        out.extend_from_slice(
            &(0x3003u16 | if original.init_locals { 0x10 } else { 0 }).to_le_bytes(),
        );
        out.extend_from_slice(&edit.max_stack.to_le_bytes());
        out.extend_from_slice(&(edit.code.len() as u32).to_le_bytes());
        out.extend_from_slice(&original.local_signature.to_le_bytes());
    }
    out.extend_from_slice(&edit.code);
    Ok(out)
}
pub fn write(a: &Assembly, edits: &BTreeMap<u32, ValidatedEdit>) -> Result<Vec<u8>> {
    let layout = preflight(a)?;
    if edits.is_empty() {
        return Ok(a.bytes.clone());
    } // Explicit byte-preserving no-change policy.
    if edits.len() > 256 {
        return Err(Error::limit("At most 256 changed methods can be exported"));
    }
    let mut output = a.bytes.clone();
    let mut growing = Vec::new();
    let mut occupied = layout.protected.clone();
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        if row.rva != 0 {
            occupied.push(header_and_code(a, 0x06000001 + index as u32)?);
        }
    }
    for debug in &layout.debug {
        if occupied.iter().any(|r| overlap(r, debug)) {
            return Err(unsupported(
                "Debug payload overlaps a managed method or preserved data",
            ));
        }
    }
    for (&token, edit) in edits {
        let validated = edit::validate(a, token, &edit.instructions)?;
        if validated.code != edit.code || validated.max_stack != edit.max_stack {
            return Err(unsupported("Edit validation is stale"));
        }
        let range = header_and_code(a, token)?;
        if occupied.iter().filter(|r| overlap(r, &range)).count() != 1 {
            return Err(unsupported(
                "Method body has ambiguous ownership or overlaps metadata",
            ));
        }
        let bytes = body_bytes(a, token, edit)?;
        if bytes.len() <= range.len()
            && (bytes[0] & 3 != 3 || a.method_row(token)?.rva.is_multiple_of(4))
        {
            output[range.clone()].fill(0);
            output[range.start..range.start + bytes.len()].copy_from_slice(&bytes);
        } else {
            growing.push((token, bytes, range));
        }
    }
    if !growing.is_empty() {
        let count = a.image.sections.len();
        let section = layout.section_headers + count * 40;
        let first_raw = a
            .image
            .sections
            .iter()
            .filter(|s| s.size_of_raw_data > 0)
            .map(|s| s.pointer_to_raw_data as usize)
            .min()
            .unwrap_or(0);
        let new_section = count < 96
            && section + 40 <= first_raw
            && section + 40 <= a.image.size_of_headers as usize
            && slice(&output, section, 40)?.iter().all(|b| *b == 0);
        let last = a
            .image
            .sections
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| s.pointer_to_raw_data)
            .ok_or_else(|| unsupported("Missing file-backed section"))?;
        // With no spare header, extend only the physically and virtually last,
        // fully file-backed .text/.rsrc/.reloc section. Existing RVAs stay fixed.
        if !new_section
            && (last.1.virtual_size > last.1.size_of_raw_data
                || a.image
                    .sections
                    .iter()
                    .any(|s| s.virtual_address > last.1.virtual_address)
                || !matches!(last.1.name().unwrap_or(""), ".text" | ".rsrc" | ".reloc"))
        {
            return Err(unsupported(
                "No safe section-header slot or extensible final section for growing methods",
            ));
        }
        let raw = align(output.len(), layout.file_align)?;
        let rva = if new_section {
            align(
                a.image
                    .sections
                    .iter()
                    .map(|s| {
                        s.virtual_address as usize + s.virtual_size.max(s.size_of_raw_data) as usize
                    })
                    .max()
                    .unwrap_or(0),
                layout.section_align,
            )?
        } else {
            last.1.virtual_address as usize + last.1.size_of_raw_data as usize
        };
        let mut data = Vec::new();
        for (token, bytes, old) in growing {
            data.resize(align(data.len(), 4)?, 0);
            let new_rva =
                u32::try_from(rva + data.len()).map_err(|_| unsupported("Method RVA overflow"))?;
            let table = &a.store.tables[6];
            let row = a.image.metadata.start
                + table.offset
                + ((token & 0xffffff) - 1) as usize * table.width;
            put32(&mut output, row, new_rva)?;
            data.extend(bytes);
            output[old].fill(0);
        }
        let virtual_size = data.len();
        data.resize(align(data.len(), layout.file_align)?, 0);
        if raw + data.len() > crate::pe::MAX_FILE {
            return Err(Error::limit("Modified assembly would exceed 64 MiB"));
        }
        output.resize(raw, 0);
        output.extend_from_slice(&data);
        if new_section {
            output[section..section + 8].copy_from_slice(b".ilens\0\0");
            put32(&mut output, section + 8, virtual_size as u32)?;
            put32(
                &mut output,
                section + 12,
                u32::try_from(rva).map_err(|_| unsupported("Section RVA overflow"))?,
            )?;
            put32(&mut output, section + 16, data.len() as u32)?;
            put32(&mut output, section + 20, raw as u32)?;
            put32(&mut output, section + 36, 0x60000020)?;
            put16(&mut output, layout.pe + 6, (count + 1) as u16)?;
        } else {
            let header = layout.section_headers + last.0 * 40;
            put32(
                &mut output,
                header + 8,
                last.1.size_of_raw_data + virtual_size as u32,
            )?;
            put32(
                &mut output,
                header + 16,
                last.1.size_of_raw_data + data.len() as u32,
            )?;
            // Relocation sections are normally discardable; new managed code is not.
            put32(
                &mut output,
                header + 36,
                (last.1.characteristics | 0x60000020) & !0x02000000,
            )?;
            if last.1.characteristics & 0x40 != 0 {
                let size = u32_at(&output, layout.optional + 8)?
                    .checked_add(data.len() as u32)
                    .ok_or_else(|| unsupported("Initialized data size overflow"))?;
                put32(&mut output, layout.optional + 8, size)?;
            }
        }
        put32(
            &mut output,
            layout.optional + 56,
            u32::try_from(align(rva + virtual_size, layout.section_align)?)
                .map_err(|_| unsupported("Image size overflow"))?,
        )?;
        let size_code = u32_at(&output, layout.optional + 4)?
            .checked_add(
                data.len() as u32
                    + if !new_section && last.1.characteristics & 0x20 == 0 {
                        last.1.size_of_raw_data
                    } else {
                        0
                    },
            )
            .ok_or_else(|| unsupported("Code section size overflow"))?;
        put32(&mut output, layout.optional + 4, size_code)?;
    }
    // Clear both records and payload bytes; no orphaned PDB paths remain in edited output.
    for range in layout.debug {
        output[range].fill(0);
    }
    output[layout.directories + 6 * 8..layout.directories + 7 * 8].fill(0);
    put32(&mut output, layout.optional + 64, 0)?;
    put32(&mut output, layout.pe + 8, 0)?;
    let parsed = Assembly::load(output.clone())?;
    if parsed.info.identity != a.info.identity
        || parsed.image.entry_point != a.image.entry_point
        || parsed.image.cli_flags != a.image.cli_flags
    {
        return Err(unsupported(
            "Output identity/entry point changed unexpectedly",
        ));
    }
    let mut expected = a.bytes[a.image.metadata.clone()].to_vec();
    for token in edits.keys() {
        let table = &a.store.tables[6];
        let row = table.offset + ((token & 0xffffff) - 1) as usize * table.width;
        put32(&mut expected, row, parsed.method_row(*token)?.rva)?;
    }
    if parsed.bytes[parsed.image.metadata.clone()] != expected {
        return Err(unsupported("Unrelated metadata changed during writing"));
    }
    if a.image.resources_size > 0 {
        let before = a.image.range(
            &a.bytes,
            a.image.resources_rva,
            a.image.resources_size as usize,
        )?;
        let after = parsed.image.range(
            &output,
            parsed.image.resources_rva,
            parsed.image.resources_size as usize,
        )?;
        if a.bytes[before] != output[after] {
            return Err(unsupported("Managed resources changed unexpectedly"));
        }
    }
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        if row.rva == 0 {
            continue;
        }
        let token = 0x06000001 + index as u32;
        if let Some(edit) = edits.get(&token) {
            let body = parsed
                .body(token)?
                .ok_or_else(|| unsupported("Written method has no body"))?;
            edit::verify(&parsed, token, &body)?;
            let bytes = cil_encode_bytes(&body)?;
            if bytes != edit.code {
                return Err(unsupported("Written method differs from validated IL"));
            }
        } else {
            let before = a.body(token)?;
            let after = parsed.body(token)?;
            if serde_json::to_vec(&before).ok() != serde_json::to_vec(&after).ok() {
                return Err(unsupported(format!(
                    "Unchanged method 0x{token:08X} was not preserved"
                )));
            }
        }
    }
    // Independent PE parser also checks native import names/addresses remain unchanged.
    let before = goblin::pe::PE::parse(&a.bytes).map_err(|e| unsupported(e.to_string()))?;
    let after = goblin::pe::PE::parse(&output).map_err(|e| unsupported(e.to_string()))?;
    if format!("{:?}", before.imports) != format!("{:?}", after.imports)
        || before.entry != after.entry
    {
        return Err(unsupported(
            "Native PE imports/entry point were not preserved",
        ));
    }
    if output[layout.cli]
        != a.bytes[a
            .image
            .range(&a.bytes, u32_at(&a.bytes, layout.directories + 14 * 8)?, 72)?]
    {
        return Err(unsupported("CLI header was modified unexpectedly"));
    }
    Ok(output)
}
fn cil_encode_bytes(body: &cil::MethodBody) -> Result<Vec<u8>> {
    crate::cil_encode::encode(&crate::cil_encode::from_body(body))
}
