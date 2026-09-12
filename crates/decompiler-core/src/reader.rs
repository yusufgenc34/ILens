use crate::error::{Error, Result};

pub fn slice(data: &[u8], start: usize, len: usize) -> Result<&[u8]> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| Error::metadata("Offset overflow"))?;
    data.get(start..end).ok_or_else(|| {
        Error::metadata(format!(
            "Range {start:#x}..{end:#x} exceeds {} bytes",
            data.len()
        ))
    })
}
pub fn u16_at(data: &[u8], p: usize) -> Result<u16> {
    let b = slice(data, p, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
pub fn u32_at(data: &[u8], p: usize) -> Result<u32> {
    let b = slice(data, p, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
#[derive(Clone)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = slice(self.data, self.pos, n)?;
        self.pos += n;
        Ok(b)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(
            b.try_into().map_err(|_| Error::metadata("Invalid u64"))?,
        ))
    }
    pub fn compressed(&mut self) -> Result<u32> {
        let b = self.u8()?;
        if b & 0x80 == 0 {
            Ok(b as u32)
        } else if b & 0xc0 == 0x80 {
            Ok(((b as u32 & 0x3f) << 8) | self.u8()? as u32)
        } else if b & 0xe0 == 0xc0 {
            Ok(((b as u32 & 0x1f) << 24)
                | ((self.u8()? as u32) << 16)
                | ((self.u8()? as u32) << 8)
                | self.u8()? as u32)
        } else {
            Err(Error::metadata("Invalid compressed integer"))
        }
    }
    pub fn compressed_signed(&mut self) -> Result<i32> {
        let start = self.pos;
        let value = self.compressed()?;
        let bits = match self.pos - start {
            1 => 7,
            2 => 14,
            _ => 29,
        };
        Ok(if value & 1 == 0 {
            (value >> 1) as i32
        } else {
            ((value >> 1) as i32) | (-1i32 << (bits - 1))
        })
    }
}
