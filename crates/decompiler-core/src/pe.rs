//! The container layer is separate from ECMA metadata so WebCIL can be added here.
use crate::{
    error::{Error, ErrorCode, Result},
    reader::*,
};
use goblin::pe::{header::Header, section_table::SectionTable};
use serde::Serialize;
use std::ops::Range;
pub const MAX_FILE: usize = 64 * 1024 * 1024;
#[derive(Debug, Serialize)]
pub struct Image {
    pub machine: u16,
    pub pe64: bool,
    pub subsystem: u16,
    pub cli_flags: u32,
    pub entry_point: u32,
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
        let clr = o
            .data_directories
            .get_clr_runtime_header()
            .ok_or_else(|| Error::new(ErrorCode::NotManaged, "No CLI directory"))?;
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
            metadata: 0..0,
            resources_rva: 0,
            resources_size: 0,
            sections,
            size_of_headers: o.windows_fields.size_of_headers,
        };
        if clr.virtual_address == 0 {
            return Err(Error::new(ErrorCode::NotManaged, "Zero CLI RVA"));
        }
        if clr.size < 72 {
            return Err(Error::new(
                ErrorCode::InvalidClr,
                "CLI directory smaller than 72 bytes",
            ));
        }
        let cp = image
            .range(bytes, clr.virtual_address, 72)
            .map_err(|e| Error::new(ErrorCode::InvalidClr, e.detail))?;
        let c = &bytes[cp];
        if u32_at(c, 0)? < 72 {
            return Err(Error::new(ErrorCode::InvalidClr, "Invalid CLI header size"));
        }
        image.cli_flags = u32_at(c, 16)?;
        image.entry_point = u32_at(c, 20)?;
        if u32_at(c, 64)? != 0 || u32_at(c, 68)? != 0 {
            return Err(Error::new(
                ErrorCode::UnsupportedNative,
                "ReadyToRun/native managed header detected",
            ));
        }
        if image.cli_flags & 1 == 0 || image.cli_flags & 0x10 != 0 {
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
