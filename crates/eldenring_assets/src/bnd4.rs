//! BND4 binders: the containers inside `.mapbnd`, `.geombnd`, `.chrbnd`
//! (after DCX). Each holds named files (FLVER, TPF, HKX, ...).
//!
//! Reference: SoulsFormats `BND4.cs`, `BinderFileHeader.cs`.
use crate::error::{Error, Result};
use crate::reader::Bytes;

/// One file in a binder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BndFile {
    pub id: i32,
    /// Full path as stored, e.g. `N:\GR\data\INTERROOT_win64\chr\c4100\c4100.flver`.
    pub name: String,
    pub data: Vec<u8>,
}

impl BndFile {
    /// The name after the last path separator.
    pub fn file_name(&self) -> &str {
        self.name.rsplit(['\\', '/']).next().unwrap_or(&self.name)
    }

    pub fn has_extension(&self, ext: &str) -> bool {
        self.file_name().to_ascii_lowercase().ends_with(&ext.to_ascii_lowercase())
    }
}

// Binder.Format bits.
const IDS: u8 = 0x02;
const NAMES1: u8 = 0x04;
const NAMES2: u8 = 0x08;
const LONG_OFFSETS: u8 = 0x10;
const COMPRESSION: u8 = 0x20;

fn file_header_size(format: u8) -> usize {
    let mut size = 0x10; // flags, padding, -1, compressed size
    if format & COMPRESSION != 0 {
        size += 8;
    }
    size += if format & LONG_OFFSETS != 0 { 8 } else { 4 };
    if format & IDS != 0 {
        size += 4;
    }
    if format & (NAMES1 | NAMES2) != 0 {
        size += 4;
    }
    if format == NAMES1 {
        size += 8;
    }
    size
}

/// Whether `data` is a BND4.
pub fn is_bnd4(data: &[u8]) -> bool {
    data.starts_with(b"BND4")
}

/// Parses a (decompressed) BND4.
pub fn parse(data: &[u8]) -> Result<Vec<BndFile>> {
    let b = Bytes::new(data, "BND4");
    b.expect_magic(0, b"BND4")?;
    if b.u8(9)? != 0 {
        return Err(Error::format("BND4", "big-endian binders are not supported"));
    }
    let count = b.count_i32(0x0C)?;
    let header_size = b.u64(0x20)? as usize;
    let unicode = b.u8(0x30)? != 0;
    let raw = b.u8(0x31)?;
    // The format byte is stored bit-reversed unless the "bit big-endian"
    // flag (0x0A, inverted) says otherwise (SoulsFormats `Binder.ReadFormat`).
    // Both readings can give the same header size, so the rule decides and
    // the recorded size only confirms it.
    let bit_big_endian = b.u8(0x0A)? == 0;
    let as_stored = bit_big_endian || (raw & 1 != 0 && raw & 0x80 == 0);
    let (first, second) = if as_stored { (raw, raw.reverse_bits()) } else { (raw.reverse_bits(), raw) };
    let format = [first, second]
        .into_iter()
        .find(|&f| file_header_size(f) == header_size)
        .ok_or_else(|| Error::format("BND4", format!("format {raw:#04x} does not fit header size {header_size}")))?;
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let h = 0x40 + i * header_size;
        let compressed_size = b.u64(h + 8)? as usize;
        let mut o = h + 0x10;
        if format & COMPRESSION != 0 {
            o += 8;
        }
        let offset = if format & LONG_OFFSETS != 0 {
            o += 8;
            b.u64(o - 8)? as usize
        } else {
            o += 4;
            b.u32(o - 4)? as usize
        };
        let mut id = -1;
        if format & IDS != 0 {
            id = b.i32(o)?;
            o += 4;
        }
        let mut name = String::new();
        if format & (NAMES1 | NAMES2) != 0 {
            name = b.string(b.offset_i32(o)?, unicode)?;
            o += 4;
        }
        if format == NAMES1 {
            id = b.i32(o)?;
        }
        files.push(BndFile {
            id,
            name,
            data: b.slice(offset, compressed_size)?.to_vec(),
        });
    }
    Ok(files)
}

/// Builds a minimal unicode BND4 (format IDs|Names1|Names2|Compression,
/// as Elden Ring's), for tests and tools.
pub fn build(files: &[(i32, &str, &[u8])]) -> Vec<u8> {
    let format: u8 = IDS | NAMES1 | NAMES2 | COMPRESSION;
    let header_size = file_header_size(format);
    let names_start = 0x40 + files.len() * header_size;
    let mut names = Vec::new();
    let mut name_offsets = Vec::new();
    for (_, name, _) in files {
        name_offsets.push(names_start + names.len());
        for u in name.encode_utf16().chain([0]) {
            names.extend_from_slice(&u.to_le_bytes());
        }
    }
    let data_start = (names_start + names.len()).next_multiple_of(16);
    let mut out = Vec::new();
    out.extend_from_slice(b"BND4");
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 0]);
    out.extend_from_slice(&(files.len() as i32).to_le_bytes());
    out.extend_from_slice(&0x40u64.to_le_bytes());
    out.extend_from_slice(b"00000000");
    out.extend_from_slice(&(header_size as u64).to_le_bytes());
    out.extend_from_slice(&(data_start as u64).to_le_bytes());
    out.extend_from_slice(&[1, format.reverse_bits(), 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&0u64.to_le_bytes());
    let mut offset = data_start;
    for (i, (id, _, data)) in files.iter().enumerate() {
        out.extend_from_slice(&[0x40, 0, 0, 0]);
        out.extend_from_slice(&(-1i32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(name_offsets[i] as u32).to_le_bytes());
        offset = (offset + data.len()).next_multiple_of(16);
    }
    out.extend_from_slice(&names);
    for (_, _, data) in files {
        out.resize(out.len().next_multiple_of(16), 0);
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let bnd = build(&[(200, r"N:\GR\chr\c4100\c4100.flver", b"FLVER\0"), (100, "c4100.tpf", b"TPF\0xx")]);
        let files = parse(&bnd).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].id, 200);
        assert_eq!(files[0].file_name(), "c4100.flver");
        assert!(files[0].has_extension(".FLVER"));
        assert_eq!(files[1].data, b"TPF\0xx");
    }
}
