//! Read-only PE/CLI header views for the explorer's PE node. Every field records its
//! file offset and width so the UI can present it like a structured hex view.
use crate::{
    assembly::Assembly,
    error::Result,
    reader::{slice, u16_at, u32_at},
};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct HeaderField {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub value: String,
    pub info: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct HeaderGroup {
    pub id: &'static str,
    pub title: &'static str,
    pub offset: usize,
    pub size: usize,
    pub fields: Vec<HeaderField>,
}
#[derive(Debug, Clone, Serialize)]
pub struct SectionInfo {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
    pub flags: String,
    pub entropy: f64,
}
#[derive(Debug, Clone, Serialize)]
pub struct PeHeaders {
    pub groups: Vec<HeaderGroup>,
    pub sections: Vec<SectionInfo>,
    /// File bytes after the last section's raw data.
    pub overlay: Option<(usize, usize)>,
    pub entropy: f64,
}
const DIRECTORIES: [&str; 16] = [
    "Export",
    "Import",
    "Resource",
    "Exception",
    "Security",
    "Base relocation",
    "Debug",
    "Architecture",
    "Global pointer",
    "TLS",
    "Load configuration",
    "Bound import",
    "Import address table",
    "Delay import",
    "CLR runtime header",
    "Reserved",
];
/// Shannon entropy in bits per byte (0..=8).
pub fn entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for b in bytes {
        counts[*b as usize] += 1;
    }
    let total = bytes.len() as f64;
    counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f64 / total;
            -p * p.log2()
        })
        .sum()
}
fn flags(value: u32, names: &[(u32, &str)]) -> String {
    names
        .iter()
        .filter(|(bit, _)| value & bit != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(" | ")
}
// Civil date from a Unix timestamp, without a date dependency.
fn timestamp(seconds: u32) -> String {
    if seconds == 0 {
        return "not set".to_owned();
    }
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // Reproducible builds store a hash here rather than a time.
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC (may be a deterministic hash)",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}
struct Builder<'a> {
    bytes: &'a [u8],
    fields: Vec<HeaderField>,
}
impl<'a> Builder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            fields: vec![],
        }
    }
    fn read(&self, offset: usize, size: usize) -> Result<u64> {
        let data = slice(self.bytes, offset, size)?;
        Ok(data
            .iter()
            .rev()
            .fold(0u64, |value, b| (value << 8) | u64::from(*b)))
    }
    fn field(
        &mut self,
        name: &str,
        offset: usize,
        size: usize,
        info: impl Into<String>,
    ) -> Result<u64> {
        let value = self.read(offset, size)?;
        self.fields.push(HeaderField {
            name: name.to_owned(),
            offset,
            size,
            value: format!("0x{value:0width$X}", width = size * 2),
            info: info.into(),
        });
        Ok(value)
    }
    fn raw(
        &mut self,
        name: &str,
        offset: usize,
        size: usize,
        info: impl Into<String>,
    ) -> Result<()> {
        slice(self.bytes, offset, size)?;
        self.fields.push(HeaderField {
            name: name.to_owned(),
            offset,
            size,
            value: String::new(),
            info: info.into(),
        });
        Ok(())
    }
    fn finish(
        &mut self,
        id: &'static str,
        title: &'static str,
        offset: usize,
        size: usize,
    ) -> HeaderGroup {
        HeaderGroup {
            id,
            title,
            offset,
            size,
            fields: std::mem::take(&mut self.fields),
        }
    }
}
fn machine(value: u16) -> &'static str {
    match value {
        0x14c => "x86 (I386)",
        0x8664 => "x64 (AMD64)",
        0x1c4 => "ARMv7 Thumb-2",
        0xaa64 => "ARM64",
        0x200 => "IA-64",
        // ReadyToRun images XOR the machine with an OS-specific constant.
        _ => "unknown or OS-specific ReadyToRun machine",
    }
}
pub fn headers(a: &Assembly) -> Result<PeHeaders> {
    let bytes = &a.bytes;
    let mut b = Builder::new(bytes);
    let mut groups = vec![];
    b.field("e_magic", 0, 2, "MZ signature")?;
    for (name, offset) in [
        ("e_cblp", 2),
        ("e_cp", 4),
        ("e_crlc", 6),
        ("e_cparhdr", 8),
        ("e_minalloc", 10),
        ("e_maxalloc", 12),
        ("e_ss", 14),
        ("e_sp", 16),
        ("e_csum", 18),
        ("e_ip", 20),
        ("e_cs", 22),
        ("e_lfarlc", 24),
        ("e_ovno", 26),
    ] {
        b.field(name, offset, 2, "")?;
    }
    b.raw("e_res", 28, 8, "Reserved")?;
    b.field("e_oemid", 36, 2, "")?;
    b.field("e_oeminfo", 38, 2, "")?;
    b.raw("e_res2", 40, 20, "Reserved")?;
    let pe = b.field("e_lfanew", 0x3c, 4, "File offset of the PE signature")? as usize;
    groups.push(b.finish("dos", "DOS Header", 0, 64));

    b.field("Signature", pe, 4, "PE\\0\\0")?;
    let file = pe + 4;
    b.field("Machine", file, 2, machine(u16_at(bytes, file)?))?;
    let count = b.field("NumberOfSections", file + 2, 2, "")? as usize;
    b.field(
        "TimeDateStamp",
        file + 4,
        4,
        timestamp(u32_at(bytes, file + 4)?),
    )?;
    b.field("PointerToSymbolTable", file + 8, 4, "")?;
    b.field("NumberOfSymbols", file + 12, 4, "")?;
    let optional_size = b.field("SizeOfOptionalHeader", file + 16, 2, "")? as usize;
    let characteristics = u16_at(bytes, file + 18)? as u32;
    b.field(
        "Characteristics",
        file + 18,
        2,
        flags(
            characteristics,
            &[
                (0x0001, "RELOCS_STRIPPED"),
                (0x0002, "EXECUTABLE_IMAGE"),
                (0x0020, "LARGE_ADDRESS_AWARE"),
                (0x0100, "32BIT_MACHINE"),
                (0x0200, "DEBUG_STRIPPED"),
                (0x1000, "SYSTEM"),
                (0x2000, "DLL"),
            ],
        ),
    )?;
    groups.push(b.finish("file", "File Header", pe, 24));

    let optional = file + 20;
    let magic = b.field("Magic", optional, 2, "")?;
    let pe64 = magic == 0x20b;
    if let Some(f) = b.fields.last_mut() {
        f.info = if pe64 { "PE32+" } else { "PE32" }.to_owned();
    }
    b.field("MajorLinkerVersion", optional + 2, 1, "")?;
    b.field("MinorLinkerVersion", optional + 3, 1, "")?;
    b.field("SizeOfCode", optional + 4, 4, "")?;
    b.field("SizeOfInitializedData", optional + 8, 4, "")?;
    b.field("SizeOfUninitializedData", optional + 12, 4, "")?;
    b.field(
        "AddressOfEntryPoint",
        optional + 16,
        4,
        "RVA of the native entry stub",
    )?;
    b.field("BaseOfCode", optional + 20, 4, "")?;
    let mut p = optional + 24;
    if !pe64 {
        b.field("BaseOfData", p, 4, "")?;
        p += 4;
        b.field("ImageBase", p, 4, "")?;
        p += 4;
    } else {
        b.field("ImageBase", p, 8, "")?;
        p += 8;
    }
    for (name, size) in [
        ("SectionAlignment", 4),
        ("FileAlignment", 4),
        ("MajorOperatingSystemVersion", 2),
        ("MinorOperatingSystemVersion", 2),
        ("MajorImageVersion", 2),
        ("MinorImageVersion", 2),
        ("MajorSubsystemVersion", 2),
        ("MinorSubsystemVersion", 2),
        ("Win32VersionValue", 4),
        ("SizeOfImage", 4),
        ("SizeOfHeaders", 4),
        ("CheckSum", 4),
    ] {
        b.field(name, p, size, "")?;
        p += size;
    }
    let subsystem = u16_at(bytes, p)?;
    b.field(
        "Subsystem",
        p,
        2,
        match subsystem {
            2 => "Windows GUI",
            3 => "Windows console",
            9 => "Windows CE GUI",
            _ => "",
        },
    )?;
    p += 2;
    let dll = u16_at(bytes, p)? as u32;
    b.field(
        "DllCharacteristics",
        p,
        2,
        flags(
            dll,
            &[
                (0x0020, "HIGH_ENTROPY_VA"),
                (0x0040, "DYNAMIC_BASE"),
                (0x0080, "FORCE_INTEGRITY"),
                (0x0100, "NX_COMPAT"),
                (0x0200, "NO_ISOLATION"),
                (0x0400, "NO_SEH"),
                (0x0800, "NO_BIND"),
                (0x1000, "APPCONTAINER"),
                (0x2000, "WDM_DRIVER"),
                (0x4000, "GUARD_CF"),
                (0x8000, "TERMINAL_SERVER_AWARE"),
            ],
        ),
    )?;
    p += 2;
    let width = if pe64 { 8 } else { 4 };
    for name in [
        "SizeOfStackReserve",
        "SizeOfStackCommit",
        "SizeOfHeapReserve",
        "SizeOfHeapCommit",
    ] {
        b.field(name, p, width, "")?;
        p += width;
    }
    b.field("LoaderFlags", p, 4, "")?;
    let directory_count = b.field("NumberOfRvaAndSizes", p + 4, 4, "")? as usize;
    let directories = p + 8;
    groups.push(b.finish(
        "optional",
        "Optional Header",
        optional,
        directories - optional,
    ));

    // A recovered CLR entry can sit in the standard slot beyond the declared count.
    let slots = directory_count
        .clamp(15, 16)
        .min((optional + optional_size).saturating_sub(directories) / 8);
    let mut cli_rva = 0;
    for (i, name) in DIRECTORIES.iter().enumerate().take(slots) {
        let rva = u32_at(bytes, directories + i * 8)?;
        let section = a
            .image
            .sections
            .iter()
            .find(|s| {
                rva >= s.virtual_address
                    && rva < s.virtual_address + s.virtual_size.max(s.size_of_raw_data)
            })
            .and_then(|s| s.name().ok())
            .unwrap_or("");
        let beyond = if i >= directory_count {
            " · beyond NumberOfRvaAndSizes"
        } else {
            ""
        };
        b.field(
            &format!("{name} RVA"),
            directories + i * 8,
            4,
            format!("{section}{beyond}"),
        )?;
        b.field(&format!("{name} size"), directories + i * 8 + 4, 4, "")?;
        if i == 14 {
            cli_rva = rva;
        }
    }
    groups.push(b.finish("directories", "Data Directories", directories, slots * 8));

    let section_headers = optional + optional_size;
    for i in 0..count.min(96) {
        let h = section_headers + i * 40;
        let name = String::from_utf8_lossy(slice(bytes, h, 8)?)
            .trim_end_matches('\0')
            .to_owned();
        b.raw(&format!("{name} Name"), h, 8, name.clone())?;
        b.field(&format!("{name} VirtualSize"), h + 8, 4, "")?;
        b.field(&format!("{name} VirtualAddress"), h + 12, 4, "")?;
        b.field(&format!("{name} SizeOfRawData"), h + 16, 4, "")?;
        b.field(&format!("{name} PointerToRawData"), h + 20, 4, "")?;
        b.field(
            &format!("{name} Characteristics"),
            h + 36,
            4,
            section_flags(u32_at(bytes, h + 36)?),
        )?;
    }
    groups.push(b.finish(
        "sections",
        "Section Headers",
        section_headers,
        count.min(96) * 40,
    ));

    if cli_rva != 0
        && let Ok(cli) = a.image.range(bytes, cli_rva, 72)
    {
        let c = cli.start;
        b.field("cb", c, 4, "Header size")?;
        b.field("MajorRuntimeVersion", c + 4, 2, "")?;
        b.field("MinorRuntimeVersion", c + 6, 2, "")?;
        b.field("MetaData RVA", c + 8, 4, "")?;
        b.field("MetaData size", c + 12, 4, "")?;
        b.field(
            "Flags",
            c + 16,
            4,
            flags(
                u32_at(bytes, c + 16)?,
                &[
                    (0x1, "ILONLY"),
                    (0x2, "32BITREQUIRED"),
                    (0x4, "IL_LIBRARY"),
                    (0x8, "STRONGNAMESIGNED"),
                    (0x10, "NATIVE_ENTRYPOINT"),
                    (0x10000, "TRACKDEBUGDATA"),
                    (0x20000, "32BITPREFERRED"),
                ],
            ),
        )?;
        let entry = u32_at(bytes, c + 20)?;
        b.field(
            "EntryPointToken",
            c + 20,
            4,
            if entry == 0 {
                "none".to_owned()
            } else {
                a.resolve(entry)
            },
        )?;
        for (name, offset) in [
            ("Resources", 24),
            ("StrongNameSignature", 32),
            ("CodeManagerTable", 40),
            ("VTableFixups", 48),
            ("ExportAddressTableJumps", 56),
            ("ManagedNativeHeader", 64),
        ] {
            b.field(&format!("{name} RVA"), c + offset, 4, "")?;
            b.field(&format!("{name} size"), c + offset + 4, 4, "")?;
        }
        groups.push(b.finish("cor20", "Cor20 Header", c, 72));
    }

    let root = a.image.metadata.start;
    b.field("Signature", root, 4, "BSJB")?;
    b.field("MajorVersion", root + 4, 2, "")?;
    b.field("MinorVersion", root + 6, 2, "")?;
    b.field("Reserved", root + 8, 4, "")?;
    let length = b.field("Length", root + 12, 4, "")? as usize;
    let version = String::from_utf8_lossy(slice(bytes, root + 16, length.min(255))?)
        .trim_end_matches('\0')
        .to_owned();
    b.raw("Version", root + 16, length.min(255), version)?;
    let mut p = root + 16 + length;
    b.field("Flags", p, 2, "")?;
    b.field("Streams", p + 2, 2, "")?;
    p += 4;
    for stream in &a.store.streams {
        b.field(
            &format!("{} Offset", stream.name),
            p,
            4,
            format!("file 0x{:X}", root + stream.offset),
        )?;
        b.field(&format!("{} Size", stream.name), p + 4, 4, "")?;
        let name = (stream.name.len() + 4) & !3;
        b.raw(
            &format!("{} Name", stream.name),
            p + 8,
            name,
            stream.name.clone(),
        )?;
        p += 8 + name;
    }
    groups.push(b.finish("metadata", "Storage Signature and Streams", root, p - root));

    if let Some(stream) = a
        .store
        .streams
        .iter()
        .find(|s| s.name == "#~" || s.name == "#-")
    {
        let t = root + stream.offset;
        b.field("Reserved", t, 4, "")?;
        b.field("MajorVersion", t + 4, 1, "")?;
        b.field("MinorVersion", t + 5, 1, "")?;
        b.field(
            "HeapSizes",
            t + 6,
            1,
            flags(
                b.read(t + 6, 1)? as u32,
                &[
                    (1, "#Strings 4-byte"),
                    (2, "#GUID 4-byte"),
                    (4, "#Blob 4-byte"),
                ],
            ),
        )?;
        b.field("Reserved2", t + 7, 1, "")?;
        let valid = b.field("Valid", t + 8, 8, "Present tables")?;
        b.field("Sorted", t + 16, 8, "")?;
        let names: Vec<_> = a
            .store
            .parsed
            .tables_header
            .tables()
            .map(|(table, _)| format!("{table:?}"))
            .collect();
        let mut row = t + 24;
        let mut index = 0;
        for bit in 0..64 {
            if valid & (1u64 << bit) != 0 {
                let name = names
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| format!("Table 0x{bit:02X}"));
                b.field(
                    &format!("{name} rows"),
                    row,
                    4,
                    format!("table 0x{bit:02X}"),
                )?;
                row += 4;
                index += 1;
            }
        }
        groups.push(b.finish("tables", "Tables Stream Header", t, row - t));
    }

    let mut sections = vec![];
    let mut end = a.image.size_of_headers as usize;
    for s in &a.image.sections {
        let start = s.pointer_to_raw_data as usize;
        let data = bytes
            .get(
                start
                    ..start
                        .saturating_add(s.size_of_raw_data as usize)
                        .min(bytes.len()),
            )
            .unwrap_or_default();
        end = end.max(start + data.len());
        sections.push(SectionInfo {
            name: s.name().unwrap_or("").to_owned(),
            virtual_address: s.virtual_address,
            virtual_size: s.virtual_size,
            raw_offset: s.pointer_to_raw_data,
            raw_size: s.size_of_raw_data,
            characteristics: s.characteristics,
            flags: section_flags(s.characteristics),
            entropy: entropy(data),
        });
    }
    Ok(PeHeaders {
        groups,
        sections,
        overlay: (end < bytes.len()).then(|| (end, bytes.len() - end)),
        entropy: entropy(bytes),
    })
}
fn section_flags(value: u32) -> String {
    flags(
        value,
        &[
            (0x20, "CODE"),
            (0x40, "INITIALIZED_DATA"),
            (0x80, "UNINITIALIZED_DATA"),
            (0x02000000, "DISCARDABLE"),
            (0x10000000, "SHARED"),
            (0x20000000, "EXECUTE"),
            (0x40000000, "READ"),
            (0x80000000, "WRITE"),
        ],
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entropy_bounds_and_dates() {
        assert_eq!(entropy(&[]), 0.0);
        assert_eq!(entropy(&[7; 32]), 0.0);
        let all: Vec<u8> = (0..=255).collect();
        assert!((entropy(&all) - 8.0).abs() < 1e-9);
        assert!(timestamp(0).starts_with("not set"));
        assert!(timestamp(1_700_000_000).starts_with("2023-11-14 22:13:20"));
    }
}
