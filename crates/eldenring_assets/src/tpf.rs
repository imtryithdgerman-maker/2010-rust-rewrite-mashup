//! TPF texture packs: a list of named DDS files.
//!
//! Reference: SoulsFormats `TPF.cs` (PC platform).
use crate::dcx;
use crate::error::{Error, Result};
use crate::oodle::Oodle;
use crate::reader::Bytes;

/// One texture of a TPF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TpfTexture {
    pub name: String,
    /// FromSoftware's texture format code (not the DXGI format).
    pub format: u8,
    /// 0 texture, 1 cubemap, 2 volume.
    pub kind: u8,
    pub mipmaps: u8,
    /// A complete `.dds` file.
    pub dds: Vec<u8>,
}

impl TpfTexture {
    /// `(width, height)` from the DDS header.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        dds_dimensions(&self.dds)
    }
}

/// `(width, height)` of a DDS file.
pub fn dds_dimensions(dds: &[u8]) -> Option<(u32, u32)> {
    if !dds.starts_with(b"DDS ") {
        return None;
    }
    let b = Bytes::new(dds, "DDS");
    Some((b.u32(16).ok()?, b.u32(12).ok()?))
}

const ENTRY_SIZE: usize = 0x14;

/// Parses a (decompressed) TPF. Textures stored DCX-compressed inside it
/// are decompressed with `oodle` when needed.
pub fn parse(data: &[u8], oodle: Option<&Oodle>) -> Result<Vec<TpfTexture>> {
    let b = Bytes::new(data, "TPF");
    b.expect_magic(0, b"TPF\0")?;
    let count = b.count_i32(0x08)?;
    let platform = b.u8(0x0C)?;
    if platform != 0 {
        return Err(Error::format("TPF", format!("platform {platform} is not PC")));
    }
    // 1 = UTF-16 names, 0/2 = Shift-JIS.
    let unicode = b.u8(0x0E)? == 1;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let e = 0x10 + i * ENTRY_SIZE;
        let offset = b.u32(e)? as usize;
        let size = b.count_i32(e + 4)?;
        let flags1 = b.u8(e + 11)?;
        let mut dds = b.slice(offset, size)?.to_vec();
        // TODO: verify against game files: flags1 2/3 marks DCX-wrapped data.
        if flags1 & 2 != 0 || dcx::is_dcx(&dds) {
            dds = dcx::decompress_if_needed(dds, oodle)?;
        }
        out.push(TpfTexture {
            name: b.string(b.offset_i32(e + 12)?, unicode)?,
            format: b.u8(e + 8)?,
            kind: b.u8(e + 9)?,
            mipmaps: b.u8(e + 10)?,
            dds,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_texture() {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&64u32.to_le_bytes());
        dds[16..20].copy_from_slice(&128u32.to_le_bytes());
        let name_off = 0x10 + ENTRY_SIZE;
        let data_off = 0x40;
        let mut d = Vec::new();
        d.extend_from_slice(b"TPF\0");
        d.extend_from_slice(&(dds.len() as i32).to_le_bytes());
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&[0, 3, 1, 0]);
        d.extend_from_slice(&(data_off as u32).to_le_bytes());
        d.extend_from_slice(&(dds.len() as i32).to_le_bytes());
        d.extend_from_slice(&[0x66, 0, 8, 0]);
        d.extend_from_slice(&(name_off as u32).to_le_bytes());
        d.extend_from_slice(&0i32.to_le_bytes());
        for u in "c4100_a".encode_utf16().chain([0]) {
            d.extend_from_slice(&u.to_le_bytes());
        }
        d.resize(data_off, 0);
        d.extend_from_slice(&dds);
        let t = parse(&d, None).unwrap();
        assert_eq!(t[0].name, "c4100_a");
        assert_eq!(t[0].mipmaps, 8);
        assert_eq!(t[0].dimensions(), Some((128, 64)));
    }
}
