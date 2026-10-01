//! The container layer is separate from ECMA metadata so WebCIL can be added here.
use crate::{
    error::{Error, ErrorCode, Result},
    reader::*,
};
use goblin::pe::{data_directories::DataDirectory, header::Header, section_table::SectionTable};
use serde::Serialize;
use std::ops::Range;
pub const MAX_FILE: usize = 64 * 1024 * 1024;
fn no_cli_error(bytes: &[u8], detail: &str) -> Error {
    // Passive hints only, bounded independently of the input limit. These
    // strings are not proof of compiler provenance. Never infer a missing DLL
    // from a native executable's filename or execute a stub to unpack it.
    let prefix = &bytes[..bytes.len().min(4 * 1024 * 1024)];
    let markers: [&[u8]; 3] = [b"System.SysUtils", b"Vcl.Forms", b"TForm"];
    if markers
        .iter()
        .all(|marker| prefix.windows(marker.len()).any(|window| window == *marker))
    {
        return Error {
            code: ErrorCode::UnsupportedNative,
            message: "Native Windows executable with Delphi/C++Builder VCL markers. It has no CLR metadata for .NET decompilation.".to_owned(),
            detail: format!("{detail}. Found System.SysUtils, Vcl.Forms and TForm runtime markers. Compiler attribution is heuristic. ILens reads managed metadata and CIL, not native machine code; adding .NET dependencies will not make this executable decompilable. No code was executed."),
        };
    }
    Error::new(ErrorCode::NotManaged, detail)
}
#[derive(Debug, Serialize)]
pub struct Image {
    pub machine: u16,
    pub pe64: bool,
    pub subsystem: u16,
    pub cli_flags: u32,
    pub entry_point: u32,
    pub ready_to_run: bool,
    pub recovered_cli_directory: bool,
    pub metadata: Range<usize>,
    pub resources_rva: u32,
    pub resources_size: u32,
    #[serde(skip)]
    pub sections: Vec<SectionTable>,
    pub size_of_headers: u32,
}
impl Image {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_FILE {
            return Err(Error::limit("Maximum assembly size is 64 MiB"));
        }
        if bytes.get(..4) == Some(b"WbIL") {
            return Err(Error::new(
                ErrorCode::UnsupportedMetadata,
                "WebCIL container support is planned",
            ));
        }
        // Preflight offsets before entering third-party parsing (32-bit host arithmetic).
        if bytes.get(..2) != Some(b"MZ") {
            return Err(Error::new(ErrorCode::InvalidPe, "Missing DOS signature"));
        }
        let offset =
            u32_at(bytes, 0x3c).map_err(|e| Error::new(ErrorCode::InvalidPe, e.detail))? as usize;
        let header =
            slice(bytes, offset, 24).map_err(|e| Error::new(ErrorCode::InvalidPe, e.detail))?;
        if &header[..4] != b"PE\0\0" {
            return Err(Error::new(ErrorCode::InvalidPe, "Missing PE signature"));
        }
        let optional_size = u16_at(header, 20)? as usize;
        slice(bytes, offset + 24, optional_size)?;
        let h =
            Header::parse(bytes).map_err(|e| Error::new(ErrorCode::InvalidPe, e.to_string()))?;
        if h.coff_header.number_of_sections > 96 {
            return Err(Error::limit("More than 96 PE sections"));
        }
        let o = h
            .optional_header
            .ok_or_else(|| Error::new(ErrorCode::InvalidPe, "Missing optional header"))?;
        let clr = o.data_directories.get_clr_runtime_header();
        let mut p = offset + 24 + optional_size;
        slice(bytes, p, h.coff_header.number_of_sections as usize * 40)?;
        let mut sections = Vec::new();
        for _ in 0..h.coff_header.number_of_sections {
            let s = SectionTable::parse(bytes, &mut p, 0)
                .map_err(|e| Error::new(ErrorCode::InvalidPe, e.to_string()))?;
            slice(
                bytes,
                s.pointer_to_raw_data as usize,
                s.size_of_raw_data as usize,
            )?;
            s.virtual_address
                .checked_add(s.virtual_size.max(s.size_of_raw_data))
                .ok_or_else(|| Error::new(ErrorCode::InvalidPe, "Section RVA overflow"))?;
            sections.push(s);
        }
        let mut image = Self {
            machine: h.coff_header.machine,
            pe64: o.standard_fields.magic == 0x20b,
            subsystem: o.windows_fields.subsystem,
            cli_flags: 0,
            entry_point: 0,
            ready_to_run: false,
            recovered_cli_directory: false,
            metadata: 0..0,
            resources_rva: 0,
            resources_size: 0,
            sections,
            size_of_headers: o.windows_fields.size_of_headers,
        };
        let clr = if let Some(clr) = clr {
            *clr
        } else {
            // Some protected managed images understate NumberOfRvaAndSizes.
            // Read only the standard CLI slot, and only if SizeOfOptionalHeader
            // physically includes it. Do not scan for or invent metadata RVAs.
            let directories = match o.standard_fields.magic {
                0x10b => 96,
                0x20b => 112,
                _ => {
                    return Err(Error::new(
                        ErrorCode::InvalidPe,
                        "Unknown optional header magic",
                    ));
                }
            };
            if o.windows_fields.number_of_rva_and_sizes >= 15
                || optional_size < directories + 15 * 8
            {
                return Err(no_cli_error(bytes, "PE header has no CLI directory"));
            }
            let slot = offset + 24 + directories + 14 * 8;
            let candidate = DataDirectory {
                virtual_address: u32_at(bytes, slot)?,
                size: u32_at(bytes, slot + 4)?,
            };
            if candidate.virtual_address == 0 || candidate.size == 0 {
                return Err(no_cli_error(bytes, "PE header has no CLI directory"));
            }
            image.recovered_cli_directory = true;
            candidate
        };
        if clr.virtual_address == 0 {
            return Err(no_cli_error(bytes, "PE CLI directory RVA is zero"));
        }
        if clr.size < 72 {
            return Err(Error::new(
                ErrorCode::InvalidClr,
                "CLI directory smaller than 72 bytes",
            ));
        }
        if image.recovered_cli_directory {
            image
                .range(bytes, clr.virtual_address, clr.size as usize)
                .map_err(|e| Error::new(ErrorCode::InvalidClr, e.detail))?;
        }
        let cp = image
            .range(bytes, clr.virtual_address, 72)
            .map_err(|e| Error::new(ErrorCode::InvalidClr, e.detail))?;
        let c = &bytes[cp];
        if u32_at(c, 0)? < 72 || (image.recovered_cli_directory && u32_at(c, 0)? > clr.size) {
            return Err(Error::new(ErrorCode::InvalidClr, "Invalid CLI header size"));
        }
        image.cli_flags = u32_at(c, 16)?;
        image.entry_point = u32_at(c, 20)?;
        let native_rva = u32_at(c, 64)?;
        let native_size = u32_at(c, 68)? as usize;
        if native_rva != 0 || native_size != 0 {
            // ReadyToRun preserves ECMA metadata and CIL in ordinary component
            // images. Inspect only that CIL; never decode/execute native code.
            if native_rva == 0 || native_size < 16 {
                return Err(Error::new(
                    ErrorCode::InvalidClr,
                    "Truncated managed native header",
                ));
            }
            let range = image.range(bytes, native_rva, native_size)?;
            let native = &bytes[range];
            if u32_at(native, 0)? != 0x00525452 {
                return Err(Error::new(
                    ErrorCode::UnsupportedNative,
                    "NGen or unknown managed native format; open the original IL assembly.",
                ));
            }
            let count = u32_at(native, 12)? as usize;
            if count > 4096 {
                return Err(Error::limit("Too many ReadyToRun sections"));
            }
            slice(native, 16, count * 12)?;
            for index in 0..count {
                let at = 16 + index * 12;
                // Flag 0x2 changed meaning across R2R versions. Use the
                // component directory to recognize composite images instead.
                if u32_at(native, at)? == 115 {
                    return Err(Error::new(
                        ErrorCode::UnsupportedNative,
                        "Composite ReadyToRun images are not supported; open the component IL assemblies.",
                    ));
                }
                image.range(
                    bytes,
                    u32_at(native, at + 4)?,
                    u32_at(native, at + 8)? as usize,
                )?;
            }
            image.ready_to_run = true;
        }
        if (image.cli_flags & 1 == 0 && !image.ready_to_run) || image.cli_flags & 0x10 != 0 {
            return Err(Error::new(
                ErrorCode::UnsupportedNative,
                "Mixed-mode/native entry point detected",
            ));
        }
        let md_size = u32_at(c, 12)? as usize;
        if md_size > 32 * 1024 * 1024 {
            return Err(Error::limit("Metadata exceeds 32 MiB"));
        }
        image.metadata = image.range(bytes, u32_at(c, 8)?, md_size)?;
        if image.recovered_cli_directory
            && (md_size < 4
                || bytes.get(image.metadata.start..image.metadata.start.saturating_add(4))
                    != Some(b"BSJB"))
        {
            return Err(Error::new(
                ErrorCode::InvalidClr,
                "Undeclared CLI directory does not point to a CLR metadata root",
            ));
        }
        image.resources_rva = u32_at(c, 24)?;
        image.resources_size = u32_at(c, 28)?;
        if image.resources_size > 0 {
            image.range(bytes, image.resources_rva, image.resources_size as usize)?;
        }
        Ok(image)
    }
    pub fn range(&self, bytes: &[u8], rva: u32, len: usize) -> Result<Range<usize>> {
        let length = u32::try_from(len).map_err(|_| Error::limit("RVA length exceeds u32"))?;
        let end = rva
            .checked_add(length)
            .ok_or_else(|| Error::metadata("RVA overflow"))?;
        let mut found = None;
        for s in &self.sections {
            if rva >= s.virtual_address
                && end <= s.virtual_address.saturating_add(s.size_of_raw_data)
            {
                let offset = (s.pointer_to_raw_data as usize)
                    .checked_add((rva - s.virtual_address) as usize)
                    .ok_or_else(|| Error::metadata("File offset overflow"))?;
                slice(bytes, offset, len)?;
                if found.is_some() {
                    return Err(Error::metadata("Ambiguous overlapping PE sections"));
                }
                found = Some(offset..offset + len);
            }
        }
        if let Some(range) = found {
            return Ok(range);
        }
        if end <= self.size_of_headers {
            slice(bytes, rva as usize, len)?;
            return Ok(rva as usize..rva as usize + len);
        }
        Err(Error::metadata(format!(
            "RVA {rva:#x} + {len} has no file-backed section"
        )))
    }
}
