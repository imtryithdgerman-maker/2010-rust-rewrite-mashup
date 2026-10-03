//! DCX: FromSoftware's compression wrapper (`DCX\0` / `DCS\0` / `DCP\0` /
//! `DCA\0` chunks, big-endian sizes). Elden Ring uses KRAK (Oodle Kraken)
//! for most files and DFLT (zlib) for some.
use std::io::Read;

use crate::error::{Error, Result};
use crate::oodle::Oodle;
use crate::reader::Bytes;

/// Whether `data` is DCX-wrapped.
pub fn is_dcx(data: &[u8]) -> bool {
    data.starts_with(b"DCX\0")
}

/// The DCX header fields needed to decompress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DcxHeader {
    /// `KRAK`, `DFLT`, `ZSTD`, ...
    pub format: [u8; 4],
    pub uncompressed_size: usize,
    pub compressed_size: usize,
    /// Where the compressed stream starts.
    pub data_offset: usize,
}

impl DcxHeader {
    pub fn format_str(&self) -> String {
        String::from_utf8_lossy(&self.format).into_owned()
    }
}

/// Reads a DCX header.
pub fn header(data: &[u8]) -> Result<DcxHeader> {
    let b = Bytes::new(data, "DCX");
    b.expect_magic(0, b"DCX\0")?;
    b.expect_magic(0x18, b"DCS\0")?;
    let uncompressed_size = b.u32_be(0x1C)? as usize;
    let compressed_size = b.u32_be(0x20)? as usize;
    b.expect_magic(0x24, b"DCP\0")?;
    let mut format = [0; 4];
    format.copy_from_slice(b.slice(0x28, 4)?);
    // The DCA chunk's size field counts its own 8-byte header.
    b.expect_magic(0x44, b"DCA\0")?;
    let data_offset = 0x44 + b.u32_be(0x48)? as usize;
    Ok(DcxHeader {
        format,
        uncompressed_size,
        compressed_size,
        data_offset,
    })
}

/// Decompresses a DCX file. KRAK needs `oodle`; DFLT never does.
pub fn decompress(data: &[u8], oodle: Option<&Oodle>) -> Result<Vec<u8>> {
    let h = header(data)?;
    let end = (h.data_offset + h.compressed_size).min(data.len());
    let src = data
        .get(h.data_offset..end)
        .ok_or_else(|| Error::format("DCX", "data offset past end of file"))?;
    let out = match &h.format {
        b"DFLT" => inflate_zlib(src, h.uncompressed_size)?,
        b"KRAK" => oodle
            .ok_or_else(|| Error::OodleUnavailable("KRAK file and no Oodle library loaded".into()))?
            .decompress(src, h.uncompressed_size)?,
        // TODO: verify against game files: patches after 1.12 may use ZSTD.
        _ => return Err(Error::UnsupportedDcx(h.format_str())),
    };
    if out.len() != h.uncompressed_size {
        return Err(Error::Decompress(format!(
            "DCX {}: got {} bytes, header says {}",
            h.format_str(),
            out.len(),
            h.uncompressed_size
        )));
    }
    Ok(out)
}

/// Returns `data` decompressed if it is DCX, else a copy.
pub fn decompress_if_needed(data: Vec<u8>, oodle: Option<&Oodle>) -> Result<Vec<u8>> {
    if is_dcx(&data) { decompress(&data, oodle) } else { Ok(data) }
}

fn inflate_zlib(src: &[u8], size_hint: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size_hint);
    if flate2::read::ZlibDecoder::new(src).read_to_end(&mut out).is_ok() {
        return Ok(out);
    }
    // Some files carry a bare deflate stream without the zlib header.
    out.clear();
    flate2::read::DeflateDecoder::new(src)
        .read_to_end(&mut out)
        .map_err(|e| Error::Decompress(format!("DFLT: {e}")))?;
    Ok(out)
}

/// Builds a DCX_DFLT file (Elden Ring's layout), for tests and tools.
pub fn compress_dflt(raw: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
    encoder.write_all(raw).expect("writing to a Vec cannot fail");
    let compressed = encoder.finish().expect("writing to a Vec cannot fail");
    let be = |v: u32| v.to_be_bytes();
    let mut out = Vec::with_capacity(0x4C + compressed.len());
    out.extend_from_slice(b"DCX\0");
    for v in [0x11000, 0x18, 0x24, 0x44, 0x4C] {
        out.extend_from_slice(&be(v));
    }
    out.extend_from_slice(b"DCS\0");
    out.extend_from_slice(&be(raw.len() as u32));
    out.extend_from_slice(&be(compressed.len() as u32));
    out.extend_from_slice(b"DCP\0DFLT");
    out.extend_from_slice(&be(0x20));
    out.extend_from_slice(&[9, 0, 0, 0]);
    for v in [0, 0, 0, 0x10100] {
        out.extend_from_slice(&be(v));
    }
    out.extend_from_slice(b"DCA\0");
    out.extend_from_slice(&be(8));
    out.extend_from_slice(&compressed);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dflt_round_trip() {
        let raw: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let dcx = compress_dflt(&raw);
        assert!(is_dcx(&dcx));
        let h = header(&dcx).unwrap();
        assert_eq!(&h.format, b"DFLT");
        assert_eq!(h.data_offset, 0x4C);
        assert_eq!(decompress(&dcx, None).unwrap(), raw);
    }

    #[test]
    fn krak_without_oodle_is_an_error() {
        let mut dcx = compress_dflt(b"abc");
        dcx[0x28..0x2C].copy_from_slice(b"KRAK");
        assert!(matches!(decompress(&dcx, None), Err(Error::OodleUnavailable(_))));
    }
}
