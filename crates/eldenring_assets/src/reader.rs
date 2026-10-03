//! Bounds-checked little-endian reads at absolute offsets.
use crate::error::{Error, Result};

#[derive(Clone, Copy)]
pub(crate) struct Bytes<'a> {
    pub data: &'a [u8],
    pub what: &'static str,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, what }
    }

    pub fn slice(&self, offset: usize, len: usize) -> Result<&'a [u8]> {
        offset
            .checked_add(len)
            .and_then(|end| self.data.get(offset..end))
            .ok_or(Error::Truncated {
                format: self.what,
                offset,
                len,
            })
    }

    fn array<const N: usize>(&self, offset: usize) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.slice(offset, N)?);
        Ok(out)
    }

    pub fn u8(&self, o: usize) -> Result<u8> {
        Ok(self.array::<1>(o)?[0])
    }
    pub fn u16(&self, o: usize) -> Result<u16> {
        self.array(o).map(u16::from_le_bytes)
    }
    pub fn i16(&self, o: usize) -> Result<i16> {
        self.array(o).map(i16::from_le_bytes)
    }
    pub fn u32(&self, o: usize) -> Result<u32> {
        self.array(o).map(u32::from_le_bytes)
    }
    pub fn i32(&self, o: usize) -> Result<i32> {
        self.array(o).map(i32::from_le_bytes)
    }
    pub fn u32_be(&self, o: usize) -> Result<u32> {
        self.array(o).map(u32::from_be_bytes)
    }
    pub fn u64(&self, o: usize) -> Result<u64> {
        self.array(o).map(u64::from_le_bytes)
    }
    pub fn i64(&self, o: usize) -> Result<i64> {
        self.array(o).map(i64::from_le_bytes)
    }
    pub fn f32(&self, o: usize) -> Result<f32> {
        self.array(o).map(f32::from_le_bytes)
    }
    pub fn vec3(&self, o: usize) -> Result<[f32; 3]> {
        Ok([self.f32(o)?, self.f32(o + 4)?, self.f32(o + 8)?])
    }

    /// An offset field read as `usize`, rejecting negatives.
    pub fn offset_i32(&self, o: usize) -> Result<usize> {
        let v = self.i32(o)?;
        usize::try_from(v).map_err(|_| Error::format(self.what, format!("negative offset {v} at {o:#x}")))
    }
    pub fn offset_i64(&self, o: usize) -> Result<usize> {
        let v = self.i64(o)?;
        usize::try_from(v).map_err(|_| Error::format(self.what, format!("negative offset {v} at {o:#x}")))
    }
    pub fn count_i32(&self, o: usize) -> Result<usize> {
        let v = self.i32(o)?;
        usize::try_from(v).map_err(|_| Error::format(self.what, format!("negative count {v} at {o:#x}")))
    }

    /// A NUL-terminated UTF-16LE string.
    pub fn utf16z(&self, offset: usize) -> Result<String> {
        let mut units = Vec::new();
        let mut o = offset;
        loop {
            let u = self.u16(o)?;
            if u == 0 {
                break;
            }
            units.push(u);
            o += 2;
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// A NUL-terminated single-byte string (Shift-JIS read as lossy UTF-8;
    /// FromSoftware's paths and names are ASCII in practice).
    pub fn cstr(&self, offset: usize) -> Result<String> {
        let rest = self.data.get(offset..).ok_or(Error::Truncated {
            format: self.what,
            offset,
            len: 1,
        })?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    pub fn string(&self, offset: usize, unicode: bool) -> Result<String> {
        if unicode { self.utf16z(offset) } else { self.cstr(offset) }
    }

    pub fn expect_magic(&self, offset: usize, magic: &[u8]) -> Result<()> {
        let found = self.slice(offset, magic.len())?;
        if found == magic {
            Ok(())
        } else {
            Err(Error::BadMagic {
                format: self.what,
                found: found.to_vec(),
            })
        }
    }
}
