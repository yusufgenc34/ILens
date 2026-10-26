//! Managed PE writer with immutable input, unchanged metadata tokens and bounded layout.
//! Only unsigned x86/x64 IL-only images and known directories are accepted.
use crate::{
    assembly::Assembly,
    cil,
    edit::{self, ValidatedEdit},
    error::{Error, Result},
    reader::{slice, u16_at, u32_at},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
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
fn preflight(a: &Assembly, fixed_literals: bool) -> Result<Layout> {
    if !fixed_literals && !a.metadata().field_rvas.is_empty() {
        return Err(unsupported(
            "General rewriting of Field-RVA assemblies is unsupported. Recognized resource-string cleanup uses a separate fixed-layout writer.",
        ));
    }
    if a.image.recovered_cli_directory {
        return Err(unsupported(
            "Assemblies with a recovered CLR directory are read-only; their nonstandard PE layout is not supported by the writer.",
        ));
    }
    if a.image.ready_to_run {
        return Err(unsupported(
            "ReadyToRun images are read-only. Rewriting their CIL would invalidate precompiled native code.",
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
    for range in field_ranges(a)? {
        if protected.iter().chain(&debug).any(|p| overlap(&range, p)) {
            return Err(unsupported("Field-RVA storage overlaps preserved PE data"));
        }
        protected.push(range);
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
/// Prove file-backed storage sizes. Empty explicitly sized value types represent
/// compiler-generated byte buffers; reference-containing/auto layouts are rejected.
fn field_ranges(a: &Assembly) -> Result<Vec<Range<usize>>> {
    use crate::signature::Type;
    let mut ranges = Vec::new();
    if a.metadata().field_rvas.len() > 1024 {
        return Err(Error::limit(
            "At most 1,024 Field-RVA ranges can be preserved",
        ));
    }
    let instance_owners: BTreeSet<_> = a
        .metadata()
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| f.flags & 0x10 == 0)
        .filter_map(|(i, _)| a.owners.get(&(0x04000001 + i as u32)).copied())
        .collect();
    for row in &a.metadata().field_rvas {
        let field = a
            .metadata()
            .fields
            .get(row.field.saturating_sub(1) as usize)
            .ok_or_else(|| unsupported("Invalid Field-RVA field index"))?;
        if field.flags & 0x110 != 0x110 {
            return Err(unsupported(
                "Field-RVA requires a static field with HasFieldRVA",
            ));
        }
        let (_, _, ty) = a.field_ref(0x04000000 | row.field)?;
        let size = match ty {
            Type::Primitive(ref name) => match name.as_str() {
                "bool" | "byte" | "sbyte" => 1,
                "char" | "short" | "ushort" => 2,
                "int" | "uint" | "float" => 4,
                "long" | "ulong" | "double" => 8,
                _ => return Err(unsupported("Unsupported Field-RVA primitive storage")),
            },
            Type::ValueType(token) if token >> 24 == 2 => {
                let ty = a
                    .metadata()
                    .type_defs
                    .get((token & 0xffffff).saturating_sub(1) as usize)
                    .ok_or_else(|| unsupported("Invalid Field-RVA value type"))?;
                let layouts: Vec<_> = a
                    .metadata()
                    .class_layouts
                    .iter()
                    .filter(|l| l.parent == token & 0xffffff)
                    .collect();
                let instance_fields = instance_owners.contains(&token);
                if ty.flags & 0x18 == 0
                    || instance_fields
                    || layouts.len() != 1
                    || a.resolve_full(crate::assembly::coded(ty.extends)) != "System.ValueType"
                {
                    return Err(unsupported(
                        "Field-RVA storage requires a proven reference-free fixed layout",
                    ));
                }
                let n = layouts[0].class_size as usize;
                if n == 0
                    || n > 1024 * 1024
                    || !matches!(
                        layouts[0].packing_size,
                        0 | 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128
                    )
                {
                    return Err(unsupported("Invalid Field-RVA fixed layout size/packing"));
                }
                n
            }
            _ => {
                return Err(unsupported(
                    "Field-RVA storage size/alias analysis is unsupported for this type",
                ));
            }
        };
        let range = a.image.range(&a.bytes, row.rva, size)?;
        if ranges.iter().any(|r| overlap(r, &range)) {
            return Err(unsupported("Aliased Field-RVA storage is unsupported"));
        }
        ranges.push(range);
    }
    Ok(ranges)
}
pub fn capability(a: &Assembly) -> Result<()> {
    preflight(a, false).map(|_| ())
}
pub fn literal_capability(a: &Assembly) -> Result<()> {
    preflight(a, true).map(|_| ())
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
/// A method body; `locals` replaces the original local signature token when present.
fn body_bytes(
    a: &Assembly,
    token: u32,
    code: &[u8],
    max_stack: u16,
    locals: Option<u32>,
    exceptions: &[cil::ExceptionRegion],
) -> Result<Vec<u8>> {
    let original = a
        .body(token)?
        .ok_or_else(|| unsupported("Cannot write a bodyless member"))?;
    let signature = locals.unwrap_or(original.local_signature);
    // Compilers zero-initialize declared locals; new ones are declared the same way.
    let init = original.init_locals || locals.is_some_and(|s| s != 0);
    let mut out = Vec::new();
    let sections = !exceptions.is_empty();
    if code.len() < 64 && max_stack <= 8 && signature == 0 && !init && !sections {
        out.push(((code.len() as u8) << 2) | 2);
    } else {
        let flags = 0x3003u16 | if init { 0x10 } else { 0 } | if sections { 0x08 } else { 0 };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&max_stack.to_le_bytes());
        out.extend_from_slice(&(code.len() as u32).to_le_bytes());
        out.extend_from_slice(&signature.to_le_bytes());
    }
    out.extend_from_slice(code);
    if sections {
        // One fat EH table (ECMA-335 II.25.4.5), 4-byte aligned after the code.
        out.resize(out.len().next_multiple_of(4), 0);
        let size = 4 + 24 * exceptions.len();
        out.push(0x41);
        out.extend_from_slice(&(size as u32).to_le_bytes()[..3]);
        for e in exceptions {
            let flags: u32 = match e.kind.as_str() {
                "catch" => 0,
                "finally" => 2,
                "fault" => 4,
                _ => return Err(unsupported("Unsupported exception clause kind")),
            };
            for value in [
                flags,
                e.try_start,
                e.try_end - e.try_start,
                e.handler_start,
                e.handler_end - e.handler_start,
                e.catch_type.unwrap_or(0),
            ] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(out)
}
/// Where appended data goes: a new `.ilens` section, or the end of the last section.
struct Region {
    new_section: bool,
    last: usize,
    raw: usize,
    rva: usize,
}
fn region(a: &Assembly, layout: &Layout, output: &[u8]) -> Result<Region> {
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
        && slice(output, section, 40)?.iter().all(|b| *b == 0);
    let (last, last_section) = a
        .image
        .sections
        .iter()
        .enumerate()
        .max_by_key(|(_, s)| s.pointer_to_raw_data)
        .ok_or_else(|| unsupported("Missing file-backed section"))?;
    // With no spare header, extend only the physically and virtually last,
    // fully file-backed .text/.rsrc/.reloc section. Existing RVAs stay fixed.
    if !new_section
        && (last_section.virtual_size > last_section.size_of_raw_data
            || a.image
                .sections
                .iter()
                .any(|s| s.virtual_address > last_section.virtual_address)
            || !matches!(
                last_section.name().unwrap_or(""),
                ".text" | ".rsrc" | ".reloc"
            ))
    {
        return Err(unsupported(
            "No safe section-header slot or extensible final section for growing methods or metadata",
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
        last_section.virtual_address as usize + last_section.size_of_raw_data as usize
    };
    Ok(Region {
        new_section,
        last,
        raw,
        rva,
    })
}
fn commit_region(
    a: &Assembly,
    layout: &Layout,
    output: &mut Vec<u8>,
    region: &Region,
    mut data: Vec<u8>,
) -> Result<()> {
    let count = a.image.sections.len();
    let virtual_size = data.len();
    data.resize(align(data.len(), layout.file_align)?, 0);
    if region.raw + data.len() > crate::pe::MAX_FILE {
        return Err(Error::limit("Modified assembly would exceed 64 MiB"));
    }
    output.resize(region.raw, 0);
    output.extend_from_slice(&data);
    let last = &a.image.sections[region.last];
    if region.new_section {
        let section = layout.section_headers + count * 40;
        output[section..section + 8].copy_from_slice(b".ilens\0\0");
        put32(output, section + 8, virtual_size as u32)?;
        put32(
            output,
            section + 12,
            u32::try_from(region.rva).map_err(|_| unsupported("Section RVA overflow"))?,
        )?;
        put32(output, section + 16, data.len() as u32)?;
        put32(output, section + 20, region.raw as u32)?;
        put32(output, section + 36, 0x60000020)?;
        put16(output, layout.pe + 6, (count + 1) as u16)?;
    } else {
        let header = layout.section_headers + region.last * 40;
        put32(
            output,
            header + 8,
            last.size_of_raw_data + virtual_size as u32,
        )?;
        put32(
            output,
            header + 16,
            last.size_of_raw_data + data.len() as u32,
        )?;
        // Relocation sections are normally discardable; new managed code is not.
        put32(
            output,
            header + 36,
            (last.characteristics | 0x60000020) & !0x02000000,
        )?;
        if last.characteristics & 0x40 != 0 {
            let size = u32_at(output, layout.optional + 8)?
                .checked_add(data.len() as u32)
                .ok_or_else(|| unsupported("Initialized data size overflow"))?;
            put32(output, layout.optional + 8, size)?;
        }
    }
    put32(
        output,
        layout.optional + 56,
        u32::try_from(align(region.rva + virtual_size, layout.section_align)?)
            .map_err(|_| unsupported("Image size overflow"))?,
    )?;
    let size_code = u32_at(output, layout.optional + 4)?
        .checked_add(
            data.len() as u32
                + if !region.new_section && last.characteristics & 0x20 == 0 {
                    last.size_of_raw_data
                } else {
                    0
                },
        )
        .ok_or_else(|| unsupported("Code section size overflow"))?;
    put32(output, layout.optional + 4, size_code)?;
    Ok(())
}
/// Replaces placeholder `ldstr` tokens with the tokens of appended #US entries.
fn patch_strings(
    code: &[u8],
    strings: &[String],
    tokens: &BTreeMap<String, u32>,
) -> Result<Vec<u8>> {
    let mut out = code.to_vec();
    for i in cil::decode(code)? {
        if let (0x72, cil::Operand::Token(t)) = (i.opcode, &i.operand)
            && let Some(index) = t.checked_sub(crate::metadata_write::PENDING_STRING)
        {
            let text = strings
                .get(index as usize)
                .ok_or_else(|| unsupported("Unknown pending string"))?;
            let token = tokens[text];
            let at = i.offset as usize + 1;
            out[at..at + 4].copy_from_slice(&token.to_le_bytes());
        }
    }
    Ok(out)
}
pub(crate) fn stream<'a>(a: &'a Assembly, name: &str) -> Option<&'a [u8]> {
    let metadata = &a.bytes[a.image.metadata.clone()];
    a.store
        .streams
        .iter()
        .find(|s| s.name == name)
        .map(|s| &metadata[s.offset..s.offset + s.size])
}
pub fn write(a: &Assembly, edits: &BTreeMap<u32, ValidatedEdit>) -> Result<Vec<u8>> {
    let layout = preflight(a, false)?;
    if edits.is_empty() {
        return Ok(a.bytes.clone());
    } // Explicit byte-preserving no-change policy.
    if edits.len() > 256 {
        return Err(Error::limit("At most 256 changed methods can be exported"));
    }
    // New literals of every edit, deduplicated, appended to the #US heap in order.
    let us_size = a
        .store
        .streams
        .iter()
        .find(|s| s.name == "#US")
        .map_or(0, |s| s.size);
    let mut appended = Vec::new();
    let mut string_tokens = BTreeMap::new();
    for edit in edits.values() {
        for text in &edit.strings {
            if !string_tokens.contains_key(text) {
                let offset = u32::try_from(us_size + appended.len())
                    .map_err(|_| unsupported("#US heap overflow"))?;
                string_tokens.insert(text.clone(), 0x7000_0000 | offset);
                appended.extend(crate::metadata_write::user_string_entry(text)?);
            }
        }
    }
    // New local signatures: an identical existing StandAloneSig is reused, otherwise
    // its blob is appended and a row added.
    let mut blob_heap = stream(a, "#Blob").unwrap_or_default().to_vec();
    let existing_sigs = &a.metadata().stand_alone_sigs;
    let mut new_sigs: Vec<(Vec<u8>, u32)> = vec![];
    let mut signatures = BTreeMap::new();
    for (&token, edit) in edits {
        let Some(types) = &edit.locals else { continue };
        if types.is_empty() {
            signatures.insert(token, 0);
            continue;
        }
        let blob = crate::signature::local_signature(types)?;
        let reused = existing_sigs
            .iter()
            .position(|r| a.blob(r.signature).is_ok_and(|b| b == blob.as_slice()))
            .map(|i| 0x1100_0001 + i as u32)
            .or_else(|| {
                new_sigs
                    .iter()
                    .position(|(b, _)| *b == blob)
                    .map(|i| 0x1100_0001 + (existing_sigs.len() + i) as u32)
            });
        let sig = match reused {
            Some(sig) => sig,
            None => {
                let offset = crate::metadata_write::append_blob(&mut blob_heap, &blob)?;
                new_sigs.push((blob, offset));
                0x1100_0000 | (existing_sigs.len() + new_sigs.len()) as u32
            }
        };
        signatures.insert(token, sig);
    }
    let mut output = a.bytes.clone();
    let mut metadata = a.bytes[a.image.metadata.clone()].to_vec();
    let method_table = &a.store.tables[6];
    let rva_at =
        |token: u32| method_table.offset + ((token & 0xffffff) - 1) as usize * method_table.width;
    let mut growing = Vec::new();
    let mut codes = BTreeMap::new();
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
        let validated = edit::validate_types(
            a,
            token,
            &edit.instructions,
            edit.locals.clone(),
            Some(&edit.regions),
        )?;
        if validated.code != edit.code
            || validated.max_stack != edit.max_stack
            || validated.strings != edit.strings
            || validated.locals != edit.locals
            || serde_json::to_vec(&validated.exceptions).ok()
                != serde_json::to_vec(&edit.exceptions).ok()
        {
            return Err(unsupported("Edit validation is stale"));
        }
        let range = header_and_code(a, token)?;
        if occupied.iter().filter(|r| overlap(r, &range)).count() != 1 {
            return Err(unsupported(
                "Method body has ambiguous ownership or overlaps metadata",
            ));
        }
        let code = patch_strings(&edit.code, &edit.strings, &string_tokens)?;
        let bytes = body_bytes(
            a,
            token,
            &code,
            edit.max_stack,
            signatures.get(&token).copied(),
            &edit.exceptions,
        )?;
        codes.insert(token, code);
        if bytes.len() <= range.len()
            && (bytes[0] & 3 != 3 || a.method_row(token)?.rva.is_multiple_of(4))
        {
            output[range.clone()].fill(0);
            output[range.start..range.start + bytes.len()].copy_from_slice(&bytes);
        } else {
            growing.push((token, bytes, range));
        }
    }
    let moved = !appended.is_empty() || !new_sigs.is_empty();
    if !growing.is_empty() || moved {
        let region = region(a, &layout, &output)?;
        let mut data = Vec::new();
        for (token, bytes, old) in growing {
            data.resize(align(data.len(), 4)?, 0);
            let new_rva = u32::try_from(region.rva + data.len())
                .map_err(|_| unsupported("Method RVA overflow"))?;
            put32(&mut metadata, rva_at(token), new_rva)?;
            data.extend(bytes);
            output[old].fill(0);
        }
        if moved {
            // The grown metadata follows the bodies; the CLI header points at it.
            data.resize(align(data.len(), 4)?, 0);
            let rva = u32::try_from(region.rva + data.len())
                .map_err(|_| unsupported("Metadata RVA overflow"))?;
            let mut replaced = vec![];
            if !appended.is_empty() {
                let heap = [stream(a, "#US").unwrap_or_default(), &appended].concat();
                if heap.len() >= crate::metadata_write::MAX_USER_STRINGS {
                    return Err(Error::limit("The #US heap would exceed its token range"));
                }
                replaced.push(("#US", heap));
            }
            if !new_sigs.is_empty() {
                // Re-encode the tables from the RVA-patched copy with the new rows.
                let s = a
                    .store
                    .streams
                    .iter()
                    .find(|s| s.name == "#~")
                    .ok_or_else(|| unsupported("Missing tables stream"))?;
                let mut tables =
                    crate::metadata_write::Tables::decode(&metadata[s.offset..s.offset + s.size])?;
                for (_, offset) in &new_sigs {
                    tables.push(17, vec![*offset])?;
                }
                tables.widen_heaps(
                    stream(a, "#Strings").map_or(0, <[u8]>::len),
                    stream(a, "#GUID").map_or(0, <[u8]>::len),
                    blob_heap.len(),
                );
                replaced.push(("#~", tables.encode()));
                replaced.push(("#Blob", blob_heap.clone()));
            }
            metadata = crate::metadata_write::rebuild(&metadata, &replaced)?;
            put32(&mut output, layout.cli.start + 8, rva)?;
            put32(&mut output, layout.cli.start + 12, metadata.len() as u32)?;
            data.extend_from_slice(&metadata);
        }
        commit_region(a, &layout, &mut output, &region, data)?;
    }
    if !moved {
        output[a.image.metadata.clone()].copy_from_slice(&metadata);
    }
    // Clear both records and payload bytes; no orphaned PDB paths remain in edited output.
    for range in layout.debug.clone() {
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
    // Independently of how it was written: every original table row and heap byte is
    // unchanged, apart from edited MethodDef RVAs and the appended rows and heap data.
    let decode = |assembly: &Assembly| {
        crate::metadata_write::Tables::decode(
            stream(assembly, "#~").ok_or_else(|| unsupported("Missing tables stream"))?,
        )
    };
    let (before_tables, after_tables) = (decode(a)?, decode(&parsed)?);
    for (table, (before, after)) in before_tables
        .rows
        .iter()
        .zip(&after_tables.rows)
        .enumerate()
    {
        let added: Vec<Vec<u32>> = if table == 17 {
            new_sigs.iter().map(|(_, offset)| vec![*offset]).collect()
        } else {
            vec![]
        };
        if after.len() != before.len() + added.len() || after[before.len()..] != added[..] {
            return Err(unsupported(format!(
                "Metadata table {table} changed unexpectedly"
            )));
        }
        for (row, (x, y)) in before.iter().zip(after).enumerate() {
            let edited = table == 6 && edits.contains_key(&(0x0600_0001 + row as u32));
            if if edited { x[1..] != y[1..] } else { x != y } {
                return Err(unsupported(format!(
                    "Metadata table {table} row {} changed unexpectedly",
                    row + 1
                )));
            }
        }
    }
    for s in &a.store.streams {
        if s.name == "#~" {
            continue;
        }
        let before = stream(a, &s.name).unwrap_or_default();
        let after = stream(&parsed, &s.name).unwrap_or_default();
        let same = if s.name == "#US" || s.name == "#Blob" {
            after.starts_with(before)
        } else {
            after == before
        };
        if !same {
            return Err(unsupported(format!(
                "Metadata stream {} changed unexpectedly",
                s.name
            )));
        }
    }
    for (text, token) in &string_tokens {
        if parsed.user_string(*token)? != *text {
            return Err(unsupported("An appended user string did not round-trip"));
        }
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
        if let Some(code) = codes.get(&token) {
            let body = parsed
                .body(token)?
                .ok_or_else(|| unsupported("Written method has no body"))?;
            edit::verify(&parsed, token, &body)?;
            let bytes = cil_encode_bytes(&body)?;
            let clauses = |exceptions: &[cil::ExceptionRegion]| serde_json::to_vec(exceptions).ok();
            if bytes != *code || clauses(&body.exceptions) != clauses(&edits[&token].exceptions) {
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
    // The CLI header changes only in its metadata directory, and only when it moved.
    let mut cli = a.bytes[layout.cli.clone()].to_vec();
    if moved {
        cli[8..16].copy_from_slice(&output[layout.cli.start + 8..layout.cli.start + 16]);
    }
    if output[layout.cli] != cli[..] {
        return Err(unsupported("CLI header was modified unexpectedly"));
    }
    Ok(output)
}
fn cil_encode_bytes(body: &cil::MethodBody) -> Result<Vec<u8>> {
    crate::cil_encode::encode(&crate::cil_encode::from_body(body))
}

/// A member to add to a type. New methods start with a default body (`ret`, or the
/// return type's zero value) that the IL editor then changes.
#[derive(Debug, Clone)]
pub enum NewMember {
    Field {
        name: String,
        ty: crate::signature::Type,
        flags: u16,
    },
    Method {
        name: String,
        return_type: crate::signature::Type,
        parameters: Vec<(String, crate::signature::Type)>,
        flags: u16,
    },
}
/// The rewritten assembly and the new member's token.
pub struct Added {
    pub bytes: Vec<u8>,
    pub token: u32,
}
fn put_compressed(n: usize, out: &mut Vec<u8>) -> Result<()> {
    match n {
        0..=0x7f => out.push(n as u8),
        0x80..=0x3fff => out.extend_from_slice(&(0x8000 | n as u16).to_be_bytes()),
        _ => return Err(unsupported("Too many parameters")),
    }
    Ok(())
}
/// A method body that returns the return type's zero value.
fn default_body(ty: &crate::signature::Type) -> Result<Vec<u8>> {
    use crate::signature::Type;
    let mut code = match ty {
        _ if ty.is_void() => vec![],
        Type::Primitive(name) => match name.as_str() {
            "bool" | "char" | "sbyte" | "byte" | "short" | "ushort" | "int" | "uint" => vec![0x16],
            "long" | "ulong" => vec![0x16, 0x6a],
            "nint" | "nuint" => vec![0x16, 0xd3],
            "float" => vec![0x22, 0, 0, 0, 0],
            "double" => vec![0x23, 0, 0, 0, 0, 0, 0, 0, 0],
            "string" | "object" => vec![0x14],
            _ => return Err(unsupported(format!("No default value for {name}"))),
        },
        Type::Named(_) | Type::Array(..) => vec![0x14],
        _ => {
            return Err(unsupported(
                "New methods return void, a primitive or a reference type; change the signature by editing IL afterwards",
            ));
        }
    };
    code.push(0x2a);
    Ok(code)
}
pub fn add_member(a: &Assembly, owner: u32, member: &NewMember) -> Result<Added> {
    use crate::signature::{Type, encode_type};
    let layout = preflight(a, false)?;
    let node = a
        .nodes
        .iter()
        .find(|n| n.token == owner && n.token >> 24 == 2)
        .ok_or_else(|| unsupported("Members can only be added to a type definition"))?;
    if owner & 0xff_ffff == 1 || !matches!(node.kind.as_str(), "class" | "struct") {
        return Err(unsupported(
            "Members can be added to classes and structs; interfaces, enums, delegates and <Module> stay unchanged",
        ));
    }
    let name = match member {
        NewMember::Field { name, .. } | NewMember::Method { name, .. } => name.trim(),
    };
    if name.is_empty() || name.len() > 1024 || name.contains('\0') || name.starts_with('.') {
        return Err(unsupported(
            "Enter a member name (up to 1,024 characters, not a special name)",
        ));
    }
    let mut strings = stream(a, "#Strings").unwrap_or_default().to_vec();
    let mut blobs = stream(a, "#Blob").unwrap_or_default().to_vec();
    let table_stream = stream(a, "#~").ok_or_else(|| unsupported("Missing tables stream"))?;
    let mut tables = crate::metadata_write::Tables::decode(table_stream)?;
    let owner_row = owner & 0xff_ffff;
    let mut body = None;
    let (table, (row, remap)) = match member {
        NewMember::Field { ty, flags, .. } => {
            if a.nodes
                .iter()
                .any(|n| n.parent == owner && n.token >> 24 == 4 && n.name == name)
            {
                return Err(unsupported(format!(
                    "{} already has a field named {name}",
                    node.name
                )));
            }
            let mut signature = vec![0x06];
            encode_type(ty, &mut signature)?;
            let row = vec![
                u32::from(*flags),
                crate::metadata_write::append_string(&mut strings, name)?,
                crate::metadata_write::append_blob(&mut blobs, &signature)?,
            ];
            (4, tables.add_field(owner_row, row)?)
        }
        NewMember::Method {
            return_type,
            parameters,
            flags,
            ..
        } => {
            let instance = flags & 0x10 == 0;
            let mut signature = vec![if instance { 0x20 } else { 0 }];
            put_compressed(parameters.len(), &mut signature)?;
            encode_type(return_type, &mut signature)?;
            for (_, ty) in parameters {
                if matches!(ty, Type::Primitive(n) if n == "void") {
                    return Err(unsupported("Parameters cannot be void"));
                }
                encode_type(ty, &mut signature)?;
            }
            let duplicate = a.nodes.iter().any(|n| {
                n.parent == owner
                    && n.token >> 24 == 6
                    && n.name == name
                    && a.method_ref(n.token).is_ok_and(|m| {
                        m.signature.parameters
                            == parameters
                                .iter()
                                .map(|(_, t)| t.clone())
                                .collect::<Vec<_>>()
                    })
            });
            if duplicate {
                return Err(unsupported(format!(
                    "{} already has a method {name} with these parameter types",
                    node.name
                )));
            }
            body = Some(default_body(return_type)?);
            let params = parameters
                .iter()
                .enumerate()
                .map(|(i, (pname, _))| {
                    Ok(vec![
                        0,
                        i as u32 + 1,
                        crate::metadata_write::append_string(&mut strings, pname.trim())?,
                    ])
                })
                .collect::<Result<Vec<_>>>()?;
            let row = vec![
                0,
                0,
                u32::from(*flags),
                crate::metadata_write::append_string(&mut strings, name)?,
                crate::metadata_write::append_blob(&mut blobs, &signature)?,
                0,
            ];
            (6, tables.add_method(owner_row, row, params)?)
        }
    };
    let token = ((table as u32) << 24) | row;
    // Every method body's field and method tokens follow the renumbering.
    let mut output = a.bytes.clone();
    for (index, method) in a.metadata().method_defs.iter().enumerate() {
        if method.rva == 0 {
            continue;
        }
        let old = 0x0600_0001 + index as u32;
        let range = header_and_code(a, old)?;
        let header = if a.bytes[range.start] & 3 == 2 { 1 } else { 12 };
        let Some(code) = a.body(old)? else { continue };
        for i in &code.instructions {
            if let cil::Operand::Token(t) = i.operand
                && matches!(t >> 24, 4 | 6 | 8)
                && remap.token(t) != t
            {
                let at =
                    range.start + header + i.offset as usize + if i.opcode > 0xff { 2 } else { 1 };
                put32(&mut output, at, remap.token(t))?;
            }
        }
    }
    let entry = u32_at(&output, layout.cli.start + 20)?;
    if entry >> 24 == 6 {
        put32(&mut output, layout.cli.start + 20, remap.token(entry))?;
    }
    let region = region(a, &layout, &output)?;
    let mut data = Vec::new();
    if let Some(code) = body {
        let rva = u32::try_from(region.rva).map_err(|_| unsupported("Method RVA overflow"))?;
        tables.rows[6][row as usize - 1][0] = rva;
        data.push(((code.len() as u8) << 2) | 2);
        data.extend_from_slice(&code);
    }
    data.resize(align(data.len(), 4)?, 0);
    tables.widen_heaps(
        strings.len(),
        stream(a, "#GUID").map_or(0, <[u8]>::len),
        blobs.len(),
    );
    let metadata = crate::metadata_write::rebuild(
        &a.bytes[a.image.metadata.clone()],
        &[
            ("#~", tables.encode()),
            ("#Strings", strings),
            ("#Blob", blobs),
        ],
    )?;
    let metadata_rva =
        u32::try_from(region.rva + data.len()).map_err(|_| unsupported("Metadata RVA overflow"))?;
    put32(&mut output, layout.cli.start + 8, metadata_rva)?;
    put32(&mut output, layout.cli.start + 12, metadata.len() as u32)?;
    data.extend_from_slice(&metadata);
    commit_region(a, &layout, &mut output, &region, data)?;
    for range in layout.debug.clone() {
        output[range].fill(0);
    }
    output[layout.directories + 6 * 8..layout.directories + 7 * 8].fill(0);
    put32(&mut output, layout.optional + 64, 0)?;
    put32(&mut output, layout.pe + 8, 0)?;
    verify_addition(a, &output, &remap, token, name)?;
    Ok(Added {
        bytes: output,
        token,
    })
}
/// Independently of the rewrite: every original member keeps its name, signature and
/// flags under its new token, every body decodes to the same instructions with
/// renumbered tokens, and the new member exists.
fn verify_addition(
    a: &Assembly,
    output: &[u8],
    remap: &crate::metadata_write::Remap,
    token: u32,
    name: &str,
) -> Result<()> {
    let parsed = Assembly::load(output.to_vec())?;
    if parsed.info.identity != a.info.identity || parsed.image.cli_flags != a.image.cli_flags {
        return Err(unsupported("Output identity changed unexpectedly"));
    }
    let same = |old: u32| -> Result<()> {
        let new = remap.token(old);
        let before = a.nodes.iter().find(|n| n.token == old);
        let after = parsed.nodes.iter().find(|n| n.token == new);
        let ok = match (before, after) {
            (Some(x), Some(y)) => x.name == y.name && x.flags == y.flags,
            (None, None) => true,
            _ => false,
        };
        if !ok {
            return Err(unsupported(format!(
                "Member 0x{old:08X} did not survive renumbering"
            )));
        }
        Ok(())
    };
    for i in 0..a.metadata().fields.len() as u32 {
        same(0x0400_0001 + i)?;
    }
    for (index, method) in a.metadata().method_defs.iter().enumerate() {
        let old = 0x0600_0001 + index as u32;
        same(old)?;
        let new = remap.token(old);
        let signature = |assembly: &Assembly, t: u32| {
            assembly
                .method_row(t)
                .ok()
                .and_then(|r| assembly.blob(r.signature).ok().map(<[u8]>::to_vec))
        };
        if signature(a, old) != signature(&parsed, new) {
            return Err(unsupported("A method signature changed during renumbering"));
        }
        if method.rva == 0 {
            continue;
        }
        let (Some(before), Some(after)) = (a.body(old)?, parsed.body(new)?) else {
            return Err(unsupported("A method body was lost during renumbering"));
        };
        let expected: Vec<_> = before
            .instructions
            .iter()
            .map(|i| match i.operand {
                cil::Operand::Token(t) => (i.opcode, Some(remap.token(t))),
                _ => (i.opcode, None),
            })
            .collect();
        let actual: Vec<_> = after
            .instructions
            .iter()
            .map(|i| match i.operand {
                cil::Operand::Token(t) => (i.opcode, Some(t)),
                _ => (i.opcode, None),
            })
            .collect();
        if expected != actual || before.code_size != after.code_size {
            return Err(unsupported(format!(
                "Method 0x{old:08X} changed during renumbering"
            )));
        }
    }
    if !parsed
        .nodes
        .iter()
        .any(|n| n.token == token && n.name == name)
    {
        return Err(unsupported("The new member is missing from the output"));
    }
    goblin::pe::PE::parse(output).map_err(|e| unsupported(e.to_string()))?;
    Ok(())
}

/// Appends reference rows (AssemblyRef, TypeRef, MemberRef, TypeSpec, MethodSpec) that a
/// C# edit needs. Nothing is renumbered; the grown metadata moves to the appended region.
pub fn add_rows(a: &Assembly, rows: &[crate::csharp_edit::NewRow]) -> Result<Vec<u8>> {
    use crate::csharp_edit::NewRow;
    use crate::metadata_write::{append_blob, append_string, encode_coded};
    let layout = preflight(a, false)?;
    if rows.is_empty() {
        return Ok(a.bytes.clone());
    }
    if rows.len() > 4096 {
        return Err(Error::limit("At most 4,096 new references per edit"));
    }
    let mut strings = stream(a, "#Strings").unwrap_or_default().to_vec();
    let mut blobs = stream(a, "#Blob").unwrap_or_default().to_vec();
    let before = crate::metadata_write::Tables::decode(
        stream(a, "#~").ok_or_else(|| unsupported("Missing tables stream"))?,
    )?;
    let mut tables = crate::metadata_write::Tables::decode(stream(a, "#~").unwrap_or_default())?;
    let mut expected: Vec<(usize, Vec<u32>)> = vec![];
    for row in rows {
        let values = match row {
            NewRow::AssemblyRef {
                version,
                flags,
                public_key,
                name,
                culture,
            } => vec![
                u32::from(version[0]),
                u32::from(version[1]),
                u32::from(version[2]),
                u32::from(version[3]),
                *flags,
                append_blob(&mut blobs, public_key)?,
                append_string(&mut strings, name)?,
                append_string(&mut strings, culture)?,
                0,
            ],
            NewRow::TypeRef {
                scope,
                name,
                namespace,
            } => vec![
                encode_coded(1, 0, *scope)?,
                append_string(&mut strings, name)?,
                append_string(&mut strings, namespace)?,
            ],
            NewRow::MemberRef {
                parent,
                name,
                signature,
            } => vec![
                encode_coded(10, 0, *parent)?,
                append_string(&mut strings, name)?,
                append_blob(&mut blobs, signature)?,
            ],
            NewRow::TypeSpec { signature } => vec![append_blob(&mut blobs, signature)?],
            NewRow::MethodSpec {
                method,
                instantiation,
            } => vec![
                encode_coded(43, 0, *method)?,
                append_blob(&mut blobs, instantiation)?,
            ],
        };
        tables.push(row.table(), values.clone())?;
        expected.push((row.table(), values));
    }
    tables.widen_heaps(
        strings.len(),
        stream(a, "#GUID").map_or(0, <[u8]>::len),
        blobs.len(),
    );
    let metadata = crate::metadata_write::rebuild(
        &a.bytes[a.image.metadata.clone()],
        &[
            ("#~", tables.encode()),
            ("#Strings", strings),
            ("#Blob", blobs),
        ],
    )?;
    let mut output = a.bytes.clone();
    let region = region(a, &layout, &output)?;
    let rva = u32::try_from(region.rva).map_err(|_| unsupported("Metadata RVA overflow"))?;
    put32(&mut output, layout.cli.start + 8, rva)?;
    put32(&mut output, layout.cli.start + 12, metadata.len() as u32)?;
    commit_region(a, &layout, &mut output, &region, metadata)?;
    for range in layout.debug.clone() {
        output[range].fill(0);
    }
    output[layout.directories + 6 * 8..layout.directories + 7 * 8].fill(0);
    put32(&mut output, layout.optional + 64, 0)?;
    put32(&mut output, layout.pe + 8, 0)?;
    // Every original row and heap byte is unchanged; only the listed rows were added.
    let parsed = Assembly::load(output.clone())?;
    let after = crate::metadata_write::Tables::decode(
        stream(&parsed, "#~").ok_or_else(|| unsupported("Missing tables stream"))?,
    )?;
    for (table, (old, new)) in before.rows.iter().zip(&after.rows).enumerate() {
        let added: Vec<Vec<u32>> = expected
            .iter()
            .filter(|(t, _)| *t == table)
            .map(|(_, r)| r.clone())
            .collect();
        if new.len() != old.len() + added.len()
            || new[..old.len()] != old[..]
            || new[old.len()..] != added[..]
        {
            return Err(unsupported(format!(
                "Metadata table {table} changed unexpectedly"
            )));
        }
    }
    for s in &a.store.streams {
        let old = stream(a, &s.name).unwrap_or_default();
        let new = stream(&parsed, &s.name).unwrap_or_default();
        let same = match s.name.as_str() {
            "#~" => true,
            "#Strings" | "#Blob" => new.starts_with(old),
            _ => new == old,
        };
        if !same {
            return Err(unsupported(format!(
                "Metadata stream {} changed unexpectedly",
                s.name
            )));
        }
    }
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        let token = 0x0600_0001 + index as u32;
        if row.rva != 0
            && serde_json::to_vec(&a.body(token)?).ok()
                != serde_json::to_vec(&parsed.body(token)?).ok()
        {
            return Err(unsupported("A method body changed while adding references"));
        }
    }
    goblin::pe::PE::parse(&output).map_err(|e| unsupported(e.to_string()))?;
    Ok(output)
}

/// Rewrites declaration-name cells only: #Strings grows, every other table cell,
/// heap byte, token and method body is proven unchanged after reparsing.
pub(crate) fn write_names(
    a: &Assembly,
    cells: &[crate::deobfuscate::rename::Cell],
) -> Result<Vec<u8>> {
    use crate::metadata_write::Tables;
    let layout = preflight(a, true)?;
    if cells.is_empty() {
        return Ok(a.bytes.clone());
    }
    let mut strings = stream(a, "#Strings").unwrap_or(&[0]).to_vec();
    if strings.is_empty() {
        strings.push(0);
    }
    let source = stream(a, "#~").ok_or_else(|| unsupported("Missing tables stream"))?;
    let before = Tables::decode(source)?;
    let mut tables = Tables::decode(source)?;
    let mut offsets: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut changed = std::collections::HashSet::new();
    for (table, row, column, name) in cells {
        let offset = if name.is_empty() {
            0
        } else if let Some(o) = offsets.get(name.as_str()) {
            *o
        } else {
            let o =
                u32::try_from(strings.len()).map_err(|_| Error::limit("#Strings heap overflow"))?;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            offsets.insert(name, o);
            o
        };
        *tables
            .rows
            .get_mut(*table)
            .and_then(|t| t.get_mut(*row))
            .and_then(|r| r.get_mut(*column))
            .ok_or_else(|| unsupported("Invalid rename cell"))? = offset;
        changed.insert((*table, *row, *column));
    }
    tables.widen_heaps(
        strings.len(),
        stream(a, "#GUID").map_or(0, <[u8]>::len),
        stream(a, "#Blob").map_or(0, <[u8]>::len),
    );
    let metadata = crate::metadata_write::rebuild(
        &a.bytes[a.image.metadata.clone()],
        &[("#~", tables.encode()), ("#Strings", strings)],
    )?;
    let mut output = a.bytes.clone();
    let region = region(a, &layout, &output)?;
    let rva = u32::try_from(region.rva).map_err(|_| unsupported("Metadata RVA overflow"))?;
    put32(&mut output, layout.cli.start + 8, rva)?;
    put32(&mut output, layout.cli.start + 12, metadata.len() as u32)?;
    commit_region(a, &layout, &mut output, &region, metadata)?;
    for range in layout.debug.clone() {
        output[range].fill(0);
    }
    output[layout.directories + 6 * 8..layout.directories + 7 * 8].fill(0);
    put32(&mut output, layout.optional + 64, 0)?;
    put32(&mut output, layout.pe + 8, 0)?;
    let parsed = Assembly::load(output.clone())?;
    let after =
        Tables::decode(stream(&parsed, "#~").ok_or_else(|| unsupported("Missing tables stream"))?)?;
    if after.rows != tables.rows {
        return Err(unsupported("Renamed metadata did not round-trip"));
    }
    for (t, (old, new)) in before.rows.iter().zip(&after.rows).enumerate() {
        for (r, (o, n)) in old.iter().zip(new).enumerate() {
            for (c, (x, y)) in o.iter().zip(n).enumerate() {
                if x != y && !changed.contains(&(t, r, c)) {
                    return Err(unsupported(format!(
                        "Metadata table {t} changed unexpectedly"
                    )));
                }
            }
        }
    }
    for (table, row, column, name) in cells {
        if parsed.string(after.rows[*table][*row][*column])? != name {
            return Err(unsupported("Renamed string failed round-trip"));
        }
    }
    for s in &a.store.streams {
        let old = stream(a, &s.name).unwrap_or_default();
        let new = stream(&parsed, &s.name).unwrap_or_default();
        let same = match s.name.as_str() {
            "#~" => true,
            "#Strings" => new.starts_with(old),
            _ => new == old,
        };
        if !same {
            return Err(unsupported(format!(
                "Metadata stream {} changed unexpectedly",
                s.name
            )));
        }
    }
    // Compare encoded bodies, not resolved display text, which carries the new names.
    let shape = |b: Option<cil::MethodBody>| {
        b.map(|b| {
            let code: Vec<_> = b
                .instructions
                .iter()
                .map(|i| (i.offset, i.bytes.clone()))
                .collect();
            (
                code,
                b.max_stack,
                b.local_signature,
                b.init_locals,
                serde_json::to_vec(&b.exceptions).ok(),
            )
        })
    };
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        let token = 0x0600_0001 + index as u32;
        if row.rva != 0 && shape(a.body(token)?) != shape(parsed.body(token)?) {
            return Err(unsupported("A method body changed while renaming"));
        }
    }
    if parsed.info.identity != a.info.identity || parsed.image.entry_point != a.image.entry_point {
        return Err(unsupported(
            "Renaming changed assembly identity/entry point",
        ));
    }
    goblin::pe::PE::parse(&output).map_err(|e| unsupported(e.to_string()))?;
    Ok(output)
}

/// Restricted proof-carrying replacement. Unlike the IL editor this never moves
/// instructions, locals, EH clauses, method RVAs or Field-RVA data.
#[derive(Debug, Clone)]
pub(crate) struct LiteralPatch {
    pub method: u32,
    pub argument_offset: u32,
    pub call_offset: u32,
    pub helper: u32,
    pub text: String,
}
pub(crate) fn literal_boundaries(body: &cil::MethodBody) -> BTreeSet<u32> {
    body.instructions
        .iter()
        .flat_map(|i| i.targets())
        .chain(body.exceptions.iter().flat_map(|e| {
            [
                Some(e.try_start),
                Some(e.try_end),
                Some(e.handler_start),
                Some(e.handler_end),
                e.filter_start,
            ]
            .into_iter()
            .flatten()
        }))
        .collect()
}
pub(crate) fn literal_site(
    body: &cil::MethodBody,
    index: usize,
    boundaries: &BTreeSet<u32>,
) -> bool {
    if index == 0 || index >= body.instructions.len() {
        return false;
    }
    let arg = &body.instructions[index - 1];
    let call = &body.instructions[index];
    if crate::deobfuscate::eaz::constant(arg).is_none()
        || call.opcode != 0x28
        || call.size != 5
        || arg.offset + arg.size != call.offset
    {
        return false;
    }
    if index >= 2 && body.instructions[index - 2].op.flow == cil::Flow::Meta {
        return false;
    }
    boundaries
        .range((
            std::ops::Bound::Excluded(arg.offset),
            std::ops::Bound::Included(call.offset),
        ))
        .next()
        .is_none()
}
/// Include all method data sections in the protected range, not only header/CIL.
fn full_method_range(a: &Assembly, token: u32) -> Result<Range<usize>> {
    let rva = a.method_row(token)?.rva;
    let mut range = header_and_code(a, token)?;
    let kind = a.bytes[range.start] & 3;
    if !matches!(kind, 2 | 3) || (kind == 3 && u16_at(&a.bytes, range.start)? >> 12 != 3) {
        return Err(unsupported("Unsupported method header in literal output"));
    }
    if a.bytes[range.start] & 3 != 3 || u16_at(&a.bytes, range.start)? & 8 == 0 {
        return Ok(range);
    }
    let mut next = (rva as usize + range.len() + 3) & !3;
    for _ in 0..32 {
        let header = a.image.range(
            &a.bytes,
            u32::try_from(next).map_err(|_| unsupported("EH RVA overflow"))?,
            4,
        )?;
        let kind = a.bytes[header.start];
        if kind & 0x3f != 1 {
            return Err(unsupported("Unknown method data section"));
        }
        let size = if kind & 0x40 != 0 {
            (u32_at(&a.bytes, header.start)? >> 8) as usize
        } else {
            a.bytes[header.start + 1] as usize
        };
        let width = if kind & 0x40 != 0 { 24 } else { 12 };
        if size < 4 || !(size - 4).is_multiple_of(width) {
            return Err(unsupported("Invalid EH data section"));
        }
        let section = a.image.range(&a.bytes, next as u32, size)?;
        if section.start < range.end {
            return Err(unsupported("Overlapping method data sections"));
        }
        range.end = section.end;
        if kind & 0x80 == 0 {
            return Ok(range);
        }
        next = (next + size + 3) & !3;
    }
    Err(unsupported("Too many EH sections"))
}
/// Rewrites every `call <forwarder>` into a direct call to the forwarder's target,
/// in place (5-byte opcode+token overwrite). Instruction offsets, locals, maxstack,
/// exception regions, metadata and all other bytes are preserved exactly. Returns the
/// new image and the number of rewritten call sites.
pub(crate) fn write_proxies(a: &Assembly) -> Result<(Vec<u8>, usize)> {
    let table = crate::deobfuscate::proxy::detect(a);
    if table.is_empty() {
        return Ok((a.bytes.clone(), 0));
    }
    // Reject layouts the same-size writer does not support (ReadyToRun, recovered CLI,
    // and unproven Field-RVA storage), mirroring the literal writer.
    preflight(a, true)?;
    let mut patches: Vec<Range<usize>> = Vec::new();
    let mut out = a.bytes.clone();
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        if row.rva == 0 {
            continue;
        }
        let token = 0x0600_0001 + index as u32;
        let Some(body) = a.body(token)? else { continue };
        let code_start = body.file_offset
            + if a.bytes[body.file_offset] & 3 == 2 {
                1
            } else {
                12
            };
        for instruction in &body.instructions {
            if instruction.opcode != 0x28 || instruction.size != 5 {
                continue;
            }
            let Some((opcode, target)) = instruction.token().and_then(|t| table.resolve(t)) else {
                continue;
            };
            let at = code_start + instruction.offset as usize;
            if at + 5 > a.bytes.len() {
                return Err(unsupported("Proxy call site outside the image"));
            }
            out[at] = opcode as u8;
            out[at + 1..at + 5].copy_from_slice(&target.to_le_bytes());
            patches.push(at..at + 5);
        }
    }
    if patches.is_empty() {
        return Ok((a.bytes.clone(), 0));
    }
    if patches.len() > 500_000 {
        return Err(Error::limit("At most 500,000 proxy call sites"));
    }
    // Every byte outside a proved call-site range must be identical to the input.
    patches.sort_by_key(|r| r.start);
    let mut cursor = 0;
    for r in &patches {
        if r.start < cursor {
            return Err(unsupported("Overlapping proxy call sites"));
        }
        if r.start > cursor && out[cursor..r.start] != a.bytes[cursor..r.start] {
            return Err(unsupported("Proxy patch changed preserved bytes"));
        }
        cursor = r.end;
    }
    if out[cursor..] != a.bytes[cursor..] {
        return Err(unsupported("Proxy patch changed preserved tail"));
    }
    // Reparse and confirm identity, entry point and that each site now decodes to its
    // target with the same instruction size (offsets and EH therefore unchanged).
    let parsed = Assembly::load(out.clone())?;
    if parsed.info.identity != a.info.identity
        || parsed.image.entry_point != a.image.entry_point
        || parsed.image.cli_flags != a.image.cli_flags
    {
        return Err(unsupported(
            "Proxy output changed assembly identity/entry point",
        ));
    }
    let count = patches.len();
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        let token = 0x0600_0001 + index as u32;
        if row.rva == 0 {
            continue;
        }
        let before = a
            .body(token)?
            .ok_or_else(|| unsupported("Missing source body"))?;
        let after = parsed
            .body(token)?
            .ok_or_else(|| unsupported("Missing output body"))?;
        if before.code_size != after.code_size
            || before.file_offset != after.file_offset
            || before.max_stack != after.max_stack
            || before.local_signature != after.local_signature
            || serde_json::to_vec(&before.exceptions).ok()
                != serde_json::to_vec(&after.exceptions).ok()
            || before.instructions.len() != after.instructions.len()
        {
            return Err(unsupported("Proxy patch changed method layout or EH"));
        }
    }
    Ok((out, count))
}
pub(crate) fn write_literals(a: &Assembly, patches: &[LiteralPatch]) -> Result<Vec<u8>> {
    let layout = preflight(a, true)?;
    if patches.is_empty() {
        return Ok(a.bytes.clone());
    }
    if patches.len() > 100_000 {
        return Err(Error::limit("At most 100,000 literal patches"));
    }
    let mut occupied = layout.protected.clone();
    let mut spans = BTreeMap::new();
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        if row.rva != 0 {
            let token = 0x06000001 + index as u32;
            let span = full_method_range(a, token)?;
            occupied.push(span.clone());
            spans.insert(token, span);
        }
    }
    // Sort once; shared/aliased method bodies must not cause quadratic work.
    let mut sorted: Vec<_> = spans.values().collect();
    sorted.sort_by_key(|r| (r.start, r.end));
    // Identical bodies may be shared (deduplicated by the compiler/protector);
    // only partially overlapping storage is ambiguous.
    sorted.dedup();
    if sorted.windows(2).any(|r| overlap(r[0], r[1])) {
        return Err(unsupported("Overlapping method storage"));
    }
    for span in &sorted {
        if layout.protected.iter().any(|r| overlap(r, span)) {
            return Err(unsupported(
                "Method storage overlaps another method or preserved data",
            ));
        }
    }
    for debug in &layout.debug {
        if occupied.iter().any(|r| overlap(r, debug)) {
            return Err(unsupported("Debug payload overlaps preserved data"));
        }
    }
    let mut heap = stream(a, "#US").unwrap_or(&[0]).to_vec();
    if heap.is_empty() {
        heap.push(0);
    }
    let mut strings = BTreeMap::new();
    let mut output = a.bytes.clone();
    let mut grouped: BTreeMap<u32, Vec<&LiteralPatch>> = BTreeMap::new();
    for patch in patches {
        grouped.entry(patch.method).or_default().push(patch);
    }
    for (&method, changes) in &grouped {
        let body = a
            .body(method)?
            .ok_or_else(|| unsupported("Missing literal-patch method"))?;
        let code_start = body.file_offset
            + if a.bytes[body.file_offset] & 3 == 2 {
                1
            } else {
                12
            };
        let boundaries = literal_boundaries(&body);
        let mut seen = std::collections::BTreeSet::new();
        for patch in changes {
            let index = body
                .instructions
                .binary_search_by_key(&patch.call_offset, |i| i.offset)
                .map_err(|_| unsupported("Missing string call"))?;
            if !literal_site(&body, index, &boundaries)
                || body.instructions[index - 1].offset != patch.argument_offset
                || body.instructions[index].token() != Some(patch.helper)
                || !seen.insert(patch.call_offset)
            {
                return Err(unsupported(
                    "Literal replacement crosses a branch/EH/prefix boundary or is stale",
                ));
            }
            let m = a.method_ref(patch.helper)?;
            if patch.helper >> 24 != 6
                || m.signature.has_this
                || m.signature.generic_count != 0
                || m.signature.return_type != crate::signature::Type::primitive("string")
                || m.signature.parameters != [crate::signature::Type::primitive("int")]
            {
                return Err(unsupported(
                    "Literal replacement requires static string(int)",
                ));
            }
            let token = if let Some(t) = strings.get(&patch.text) {
                *t
            } else {
                let t = 0x70000000 | heap.len() as u32;
                let entry = crate::metadata_write::user_string_entry(&patch.text)?;
                if heap.len() + entry.len() >= crate::metadata_write::MAX_USER_STRINGS {
                    return Err(Error::limit("Literal output exceeds #US token range"));
                }
                heap.extend(entry);
                strings.insert(patch.text.clone(), t);
                t
            };
            let from = code_start + patch.argument_offset as usize;
            let call = code_start + patch.call_offset as usize;
            output[from..call].fill(0);
            output[call] = 0x72;
            output[call + 1..call + 5].copy_from_slice(&token.to_le_bytes());
        }
    }
    let metadata =
        crate::metadata_write::rebuild(&a.bytes[a.image.metadata.clone()], &[("#US", heap)])?;
    let region = region(a, &layout, &output)?;
    put32(&mut output, layout.cli.start + 8, region.rva as u32)?;
    put32(&mut output, layout.cli.start + 12, metadata.len() as u32)?;
    commit_region(a, &layout, &mut output, &region, metadata)?;
    for r in &layout.debug {
        output[r.clone()].fill(0);
    }
    output[layout.directories + 48..layout.directories + 56].fill(0);
    put32(&mut output, layout.optional + 64, 0)?;
    put32(&mut output, layout.pe + 8, 0)?;
    let parsed = Assembly::load(output.clone())?;
    if parsed.info.identity != a.info.identity
        || parsed.image.entry_point != a.image.entry_point
        || parsed.image.cli_flags != a.image.cli_flags
    {
        return Err(unsupported(
            "Literal output changed assembly identity/entry point",
        ));
    }
    // Exact table/heap preservation is stronger here than token remapping checks.
    for s in &a.store.streams {
        let before = stream(a, &s.name).unwrap_or_default();
        let after = stream(&parsed, &s.name).unwrap_or_default();
        if if s.name == "#US" {
            !after.starts_with(before)
        } else {
            before != after
        } {
            return Err(unsupported("Literal output changed existing metadata"));
        }
    }
    for (text, token) in &strings {
        if parsed.user_string(*token)? != *text {
            return Err(unsupported("Literal failed #US round-trip"));
        }
    }
    for (&token, span) in &spans {
        if let Some(changes) = grouped.get(&token) {
            let before = a
                .body(token)?
                .ok_or_else(|| unsupported("Missing source body"))?;
            let after = parsed
                .body(token)?
                .ok_or_else(|| unsupported("Missing output body"))?;
            if before.code_size != after.code_size
                || before.file_offset != after.file_offset
                || before.max_stack != after.max_stack
                || before.local_signature != after.local_signature
                || before.init_locals != after.init_locals
                || serde_json::to_vec(&before.exceptions).ok()
                    != serde_json::to_vec(&after.exceptions).ok()
            {
                return Err(unsupported("Literal patch changed method layout or EH"));
            }
            let start = before.file_offset
                + if a.bytes[before.file_offset] & 3 == 2 {
                    1
                } else {
                    12
                };
            let mut expected = a.bytes[span.clone()].to_vec();
            for p in changes {
                let begin = start + p.argument_offset as usize - span.start;
                let call = start + p.call_offset as usize - span.start;
                expected[begin..call].fill(0);
                expected[call] = 0x72;
                expected[call + 1..call + 5].copy_from_slice(&strings[&p.text].to_le_bytes());
            }
            if output[span.clone()] != expected {
                return Err(unsupported("Unexpected method byte changes"));
            }
        } else if output[span.clone()] != a.bytes[span.clone()] {
            return Err(unsupported("Unchanged method bytes modified"));
        }
    }
    // Every original non-header byte outside a proved patch/debug range is equal.
    // Covers resources, Field-RVA storage, native imports, and original metadata.
    let mut allowed = vec![
        0..a.image.size_of_headers as usize,
        layout.cli.start + 8..layout.cli.start + 16,
    ];
    allowed.extend(layout.debug);
    for p in patches {
        let span = &spans[&p.method];
        let code = span.start + if a.bytes[span.start] & 3 == 2 { 1 } else { 12 };
        allowed.push(code + p.argument_offset as usize..code + p.call_offset as usize + 5);
    }
    allowed.sort_by_key(|r| r.start);
    let mut cursor = 0;
    for r in allowed {
        if r.start > cursor && output[cursor..r.start] != a.bytes[cursor..r.start] {
            return Err(unsupported("Preserved bytes changed"));
        }
        cursor = cursor.max(r.end);
    }
    if output[cursor..a.bytes.len()] != a.bytes[cursor..] {
        return Err(unsupported("Preserved tail changed"));
    }
    Ok(output)
}

#[cfg(test)]
mod literal_tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../../../samples/fixtures/ILens.ResourceStrings.dll");
    fn token(a: &Assembly, name: &str) -> u32 {
        a.nodes
            .iter()
            .find(|n| n.token >> 24 == 6 && n.name == name)
            .unwrap()
            .token
    }
    fn patches(a: &Assembly, name: &str) -> Vec<LiteralPatch> {
        let method = token(a, name);
        let body = a.body(method).unwrap().unwrap();
        let helper = token(a, "Lookup");
        body.instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| i.opcode == 0x28 && i.token() == Some(helper))
            .map(|(index, i)| LiteralPatch {
                method,
                argument_offset: body.instructions[index - 1].offset,
                call_offset: i.offset,
                helper,
                text: format!("owned {}", index),
            })
            .collect()
    }
    #[test]
    fn literal_writer_preserves_field_storage_eh_and_original_tokens() {
        let a = Assembly::load(FIXTURE.to_vec()).unwrap();
        assert_eq!(a.metadata().field_rvas.len(), 1);
        literal_capability(&a).unwrap();
        assert!(capability(&a).is_err());
        let mut p = patches(&a, "Message");
        p.extend(patches(&a, "WithHandlers"));
        assert_eq!(p.len(), 3);
        let bytes = write_literals(&a, &p).unwrap();
        let output = Assembly::load(bytes).unwrap();
        for patch in p {
            let body = output.body(patch.method).unwrap().unwrap();
            let i = body
                .instructions
                .iter()
                .find(|i| i.offset == patch.call_offset)
                .unwrap();
            assert_eq!(i.opcode, 0x72);
            assert_eq!(output.user_string(i.token().unwrap()).unwrap(), patch.text);
            crate::edit::verify(&output, patch.method, &body).unwrap();
        }
        for r in field_ranges(&a).unwrap() {
            assert_eq!(a.bytes[r.clone()], output.bytes[r]);
        }
        assert_eq!(stream(&a, "#~"), stream(&output, "#~"));
        assert_eq!(write_literals(&a, &[]).unwrap(), FIXTURE);
    }
    #[test]
    fn literal_writer_rejects_stale_duplicate_and_nonliteral_patches() {
        let a = Assembly::load(FIXTURE.to_vec()).unwrap();
        let p = patches(&a, "Message");
        assert!(write_literals(&a, &[p[0].clone(), p[0].clone()]).is_err());
        let mut bad = p.clone();
        bad[0].argument_offset += 1;
        assert!(write_literals(&a, &bad).is_err());
        assert!(write_literals(&a, &patches(&a, "RuntimeInput")).is_err());
        let mut body = a.body(token(&a, "Message")).unwrap().unwrap();
        let index = body
            .instructions
            .iter()
            .position(|i| i.opcode == 0x28)
            .unwrap();
        let target = body.instructions[index].offset;
        body.exceptions.push(cil::ExceptionRegion {
            kind: "catch".into(),
            try_start: 0,
            try_end: target,
            handler_start: target,
            handler_end: body.code_size,
            catch_type: None,
            filter_start: None,
        });
        assert!(!literal_site(&body, index, &literal_boundaries(&body)));
        body.exceptions.clear();
        body.instructions.last_mut().unwrap().operand = cil::Operand::Branch(target);
        assert!(!literal_site(&body, index, &literal_boundaries(&body)));
    }
    #[test]
    fn field_storage_aliases_and_unknown_layouts_are_rejected() {
        let a = Assembly::load(FIXTURE.to_vec()).unwrap();
        let rva_at = a.image.metadata.start + a.store.tables[29].offset;
        let mut bytes = a.bytes.clone();
        bytes[rva_at..rva_at + 4].copy_from_slice(
            &a.method_row(token(&a, "Message"))
                .unwrap()
                .rva
                .to_le_bytes(),
        );
        let corrupt = Assembly::load(bytes).unwrap();
        assert!(write_literals(&corrupt, &patches(&corrupt, "Message")).is_err());
        let mut bytes = a.bytes.clone();
        let size_at = a.image.metadata.start + a.store.tables[15].offset + 2;
        bytes[size_at..size_at + 4].fill(0);
        let corrupt = Assembly::load(bytes).unwrap();
        assert!(literal_capability(&corrupt).is_err());
    }
    #[test]
    fn shared_method_bodies_are_patched_once_and_stay_shared() {
        let a = Assembly::load(FIXTURE.to_vec()).unwrap();
        let message = token(&a, "Message");
        let alias = a
            .metadata()
            .method_defs
            .iter()
            .enumerate()
            .map(|(i, _)| 0x06000001 + i as u32)
            .find(|t| *t != message && a.method_row(*t).unwrap().rva != 0)
            .unwrap();
        // Point another method at Message's body, as body deduplication does.
        let table = &a.store.tables[6];
        let at =
            a.image.metadata.start + table.offset + table.width * ((alias & 0xffffff) as usize - 1);
        let mut bytes = a.bytes.clone();
        bytes[at..at + 4].copy_from_slice(&a.method_row(message).unwrap().rva.to_le_bytes());
        let shared = Assembly::load(bytes).unwrap();
        let mut p = patches(&shared, "Message");
        let mut copies = p.clone();
        for c in &mut copies {
            c.method = alias;
        }
        p.extend(copies);
        let output = Assembly::load(write_literals(&shared, &p).unwrap()).unwrap();
        assert_eq!(
            output.method_row(alias).unwrap().rva,
            output.method_row(message).unwrap().rva
        );
        let body = output.body(alias).unwrap().unwrap();
        assert!(body.instructions.iter().any(|i| i.opcode == 0x72));
    }
    #[test]
    fn missing_user_string_stream_is_created_without_remapping() {
        let a = Assembly::load(FIXTURE.to_vec()).unwrap();
        let mut bytes = a.bytes.clone();
        let metadata = &mut bytes[a.image.metadata.clone()];
        let at = metadata.windows(4).position(|b| b == b"#US\0").unwrap();
        metadata[at..at + 4].copy_from_slice(b"#XU\0");
        let a = Assembly::load(bytes).unwrap();
        assert!(stream(&a, "#US").is_none());
        let output = Assembly::load(write_literals(&a, &patches(&a, "Message")).unwrap()).unwrap();
        assert!(stream(&output, "#US").is_some());
        assert_eq!(stream(&a, "#~"), stream(&output, "#~"));
        assert_eq!(stream(&a, "#XU"), stream(&output, "#XU"));
    }
}
