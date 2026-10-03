//! BHD5/BDT archives: `DataN.bhd` is the (RSA-encrypted) header listing
//! files by path hash, `DataN.bdt` holds their bytes, parts of which are
//! AES-128-ECB encrypted.
//!
//! References: SoulsFormats `BHD5.cs`, BinderTool, UXM Selective Unpack.
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use num_bigint::BigUint;

use crate::dcx;
use crate::error::{Error, Result};
use crate::oodle::Oodle;
use crate::reader::Bytes;

/// Elden Ring's 64-bit path hash: lowercase, forward slashes, a leading
/// slash, then `h = h * 133 + c` over the characters.
pub fn path_hash(path: &str) -> u64 {
    normalise_path(path)
        .chars()
        .fold(0u64, |h, c| h.wrapping_mul(133).wrapping_add(u64::from(c)))
}

/// The form of a path that is hashed: `/map/mapstudio/m10_00_00_00.msb.dcx`.
pub fn normalise_path(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/").to_ascii_lowercase();
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    p
}

/// AES key and the encrypted byte ranges of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AesKey {
    pub key: [u8; 16],
    /// `(start, end)` byte ranges into the file; `-1` marks an unused range.
    pub ranges: Vec<(i64, i64)>,
}

/// One file of a BHD5 header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub hash: u64,
    /// Bytes stored in the BDT (AES pads to 16).
    pub padded_size: u32,
    /// Bytes of the file itself; 0 when equal to `padded_size`.
    pub unpadded_size: u32,
    pub offset: u64,
    pub aes: Option<AesKey>,
}

/// A parsed BHD5 header.
#[derive(Clone, Debug, Default)]
pub struct Bhd5 {
    pub salt: String,
    pub entries: Vec<FileEntry>,
}

/// Size of an Elden Ring BHD5 file header.
// TODO: verify against game files: 40 bytes as SoulsFormats reads ER
// (u64 hash, i32 padded, i32 unpadded, i64 offset, i64 sha, i64 aes).
const FILE_HEADER_SIZE: usize = 40;

impl Bhd5 {
    /// Parses a decrypted BHD5 header.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let b = Bytes::new(data, "BHD5");
        b.expect_magic(0, b"BHD5")?;
        if b.u8(4)? != 0xFF {
            return Err(Error::format("BHD5", "big-endian headers are not Elden Ring's"));
        }
        let bucket_count = b.count_i32(0x10)?;
        let buckets_offset = b.offset_i32(0x14)?;
        let salt_len = b.count_i32(0x18)?;
        let salt = String::from_utf8_lossy(b.slice(0x1C, salt_len)?).into_owned();
        let mut entries = Vec::new();
        for i in 0..bucket_count {
            let bucket = buckets_offset + i * 8;
            let count = b.count_i32(bucket)?;
            let offset = b.offset_i32(bucket + 4)?;
            for j in 0..count {
                let h = offset + j * FILE_HEADER_SIZE;
                let aes_offset = b.i64(h + 32)?;
                let aes = if aes_offset > 0 {
                    Some(read_aes_key(&b, aes_offset as usize)?)
                } else {
                    None
                };
                entries.push(FileEntry {
                    hash: b.u64(h)?,
                    padded_size: b.u32(h + 8)?,
                    unpadded_size: b.u32(h + 12)?,
                    offset: b.u64(h + 16)?,
                    aes,
                });
            }
        }
        Ok(Self { salt, entries })
    }
}

fn read_aes_key(b: &Bytes<'_>, at: usize) -> Result<AesKey> {
    let mut key = [0; 16];
    key.copy_from_slice(b.slice(at, 16)?);
    let count = b.count_i32(at + 16)?;
    let ranges = (0..count)
        .map(|i| Ok((b.i64(at + 20 + i * 16)?, b.i64(at + 28 + i * 16)?)))
        .collect::<Result<_>>()?;
    Ok(AesKey { key, ranges })
}

/// Decrypts AES-128-ECB ranges of a file in place.
pub fn decrypt_aes_ranges(data: &mut [u8], aes: &AesKey) {
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockDecrypt, KeyInit};
    let cipher = aes::Aes128::new(GenericArray::from_slice(&aes.key));
    for &(start, end) in &aes.ranges {
        if start < 0 || end <= start {
            continue;
        }
        let (start, end) = (start as usize, (end as usize).min(data.len()));
        let Some(range) = data.get_mut(start..end) else {
            continue;
        };
        for block in range.as_chunks_mut::<16>().0 {
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
        }
    }
}

/// An RSA public key (modulus, exponent).
#[derive(Clone, Debug)]
pub struct RsaPublicKey {
    pub n: BigUint,
    pub e: BigUint,
}

impl RsaPublicKey {
    /// Parses a PKCS#1 `-----BEGIN RSA PUBLIC KEY-----` PEM.
    pub fn from_pem(pem: &str) -> Result<Self> {
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64_decode(body.trim()).ok_or_else(|| Error::format("RSA key", "bad base64"))?;
        // RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }
        let (tag, seq, _) = der_item(&der).ok_or_else(|| Error::format("RSA key", "bad DER"))?;
        if tag != 0x30 {
            return Err(Error::format("RSA key", "not a SEQUENCE"));
        }
        let (t1, n, rest) = der_item(seq).ok_or_else(|| Error::format("RSA key", "bad modulus"))?;
        let (t2, e, _) = der_item(rest).ok_or_else(|| Error::format("RSA key", "bad exponent"))?;
        if t1 != 2 || t2 != 2 {
            return Err(Error::format("RSA key", "expected INTEGERs"));
        }
        Ok(Self {
            n: BigUint::from_bytes_be(n),
            e: BigUint::from_bytes_be(e),
        })
    }

    /// "Decrypts" data encrypted with the private key, as BinderTool does:
    /// raw RSA over key-sized blocks, each yielding `(bits - 1) / 8` bytes.
    pub fn decrypt(&self, data: &[u8]) -> Vec<u8> {
        let bits = self.n.bits() as usize;
        let in_block = bits.div_ceil(8);
        let out_block = (bits - 1) / 8;
        let mut out = Vec::with_capacity(data.len() / in_block * out_block);
        for chunk in data.chunks(in_block) {
            let m = BigUint::from_bytes_be(chunk).modpow(&self.e, &self.n).to_bytes_be();
            let m = &m[m.len().saturating_sub(out_block)..];
            out.resize(out.len() + out_block - m.len(), 0);
            out.extend_from_slice(m);
        }
        out
    }
}

/// One DER TLV: (tag, contents, rest).
fn der_item(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *d.first()?;
    let first = *d.get(1)?;
    let (len, header) = if first & 0x80 == 0 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7F) as usize;
        let len = d.get(2..2 + n)?.iter().fold(0usize, |a, &b| (a << 8) | b as usize);
        (len, 2 + n)
    };
    let body = d.get(header..header + len)?;
    Some((tag, body, &d[header + len..]))
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut nbits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | value(c)?;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}

/// One opened `.bhd`/`.bdt` pair.
#[derive(Debug)]
pub struct Archive {
    pub name: String,
    pub bdt_path: PathBuf,
    pub salt: String,
    entries: HashMap<u64, FileEntry>,
    bdt: Mutex<Option<File>>,
}

impl Archive {
    /// Opens `<stem>.bhd`, decrypting it with its RSA key when needed.
    pub fn open(bhd_path: &Path) -> Result<Self> {
        let raw = std::fs::read(bhd_path).map_err(|e| Error::io(bhd_path, e))?;
        let stem = bhd_path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        let header = if raw.starts_with(b"BHD5") {
            raw
        } else {
            let pem = crate::keys::for_archive(&stem).ok_or_else(|| Error::MissingKey(stem.clone()))?;
            RsaPublicKey::from_pem(&pem)?.decrypt(&raw)
        };
        let bhd = Bhd5::parse(&header)?;
        Ok(Self {
            name: stem,
            bdt_path: bhd_path.with_extension("bdt"),
            salt: bhd.salt,
            entries: bhd.entries.into_iter().map(|e| (e.hash, e)).collect(),
            bdt: Mutex::new(None),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, path: &str) -> Option<&FileEntry> {
        self.entries.get(&path_hash(path))
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entry(path).is_some()
    }

    /// The stored bytes of a file, AES-decrypted (still DCX if it was).
    pub fn read_entry(&self, entry: &FileEntry) -> Result<Vec<u8>> {
        let mut guard = self.bdt.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            *guard = Some(File::open(&self.bdt_path).map_err(|e| Error::io(&self.bdt_path, e))?);
        }
        let file = guard.as_mut().expect("opened above");
        let mut data = vec![0; entry.padded_size as usize];
        file.seek(SeekFrom::Start(entry.offset))
            .and_then(|_| file.read_exact(&mut data))
            .map_err(|e| Error::io(&self.bdt_path, e))?;
        drop(guard);
        if let Some(aes) = &entry.aes {
            decrypt_aes_ranges(&mut data, aes);
        }
        if entry.unpadded_size > 0 && (entry.unpadded_size as usize) < data.len() {
            data.truncate(entry.unpadded_size as usize);
        }
        Ok(data)
    }
}

/// Every archive of an install, plus loose (UXM-unpacked) files, with
/// DCX decompression.
#[derive(Debug)]
pub struct ArchiveSet {
    pub game_dir: PathBuf,
    pub archives: Vec<Archive>,
    /// Archives that failed to open, and why.
    pub open_errors: Vec<(PathBuf, String)>,
    oodle: OnceLock<Option<Oodle>>,
}

impl ArchiveSet {
    /// Opens every `.bhd` in the game folder and its `sd` folder.
    pub fn open(game_dir: &Path) -> Result<Self> {
        let mut bhds = Vec::new();
        for dir in [game_dir.to_path_buf(), game_dir.join("sd")] {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bhd")) {
                    bhds.push(path);
                }
            }
        }
        bhds.sort();
        let mut set = Self {
            game_dir: game_dir.to_path_buf(),
            archives: Vec::new(),
            open_errors: Vec::new(),
            oodle: OnceLock::new(),
        };
        for bhd in bhds {
            match Archive::open(&bhd) {
                Ok(archive) => set.archives.push(archive),
                Err(error) => set.open_errors.push((bhd, error.to_string())),
            }
        }
        if set.archives.is_empty() && !game_dir.join("map").is_dir() {
            return Err(Error::NotFound(format!(
                "no readable archives or unpacked files in {} ({:?})",
                game_dir.display(),
                set.open_errors
            )));
        }
        Ok(set)
    }

    /// The game's Oodle, loaded on first use.
    pub fn oodle(&self) -> Option<&Oodle> {
        self.oodle.get_or_init(|| Oodle::load(&self.game_dir).ok()).as_ref()
    }

    /// The stored bytes of a file: a loose copy first, then the archives.
    pub fn read_raw(&self, path: &str) -> Result<Vec<u8>> {
        let normal = normalise_path(path);
        let loose = self.game_dir.join(normal.trim_start_matches('/'));
        if loose.is_file() {
            return std::fs::read(&loose).map_err(|e| Error::io(loose, e));
        }
        for archive in &self.archives {
            if let Some(entry) = archive.entry(&normal) {
                return archive.read_entry(entry);
            }
        }
        Err(Error::NotFound(normal))
    }

    /// A file's bytes, DCX-decompressed.
    pub fn try_extract(&self, path: &str) -> Result<Vec<u8>> {
        dcx::decompress_if_needed(self.read_raw(path)?, self.oodle())
    }

    /// A file's bytes, DCX-decompressed, or `None` if absent or unreadable.
    pub fn extract(&self, path: &str) -> Option<Vec<u8>> {
        self.try_extract(path).ok()
    }

    pub fn contains(&self, path: &str) -> bool {
        let normal = normalise_path(path);
        self.game_dir.join(normal.trim_start_matches('/')).is_file() || self.archives.iter().any(|a| a.contains(&normal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_normalises_paths() {
        assert_eq!(path_hash("MAP\\MapStudio\\m10_00_00_00.msb.dcx"), path_hash("/map/mapstudio/m10_00_00_00.msb.dcx"));
        assert_eq!(path_hash("/a"), u64::from(b'/') * 133 + u64::from(b'a'));
    }

    #[test]
    fn builtin_keys_parse_as_2048_bit() {
        for stem in ["Data0", "Data1", "Data2", "Data3", "sd"] {
            let key = RsaPublicKey::from_pem(crate::keys::builtin(stem).unwrap()).unwrap();
            assert_eq!(key.n.bits(), 2048, "{stem}");
            assert!(key.e > BigUint::from(2u8), "{stem}");
        }
    }

    #[test]
    fn rsa_decrypt_round_trips_with_a_toy_key() {
        // p = 61, q = 53: n = 3233, e = 17, d = 2753. 12-bit modulus gives
        // 2-byte input blocks and 1-byte output blocks.
        let key = RsaPublicKey {
            n: BigUint::from(3233u32),
            e: BigUint::from(17u32),
        };
        let d = BigUint::from(2753u32);
        let plain = b"BHD5";
        let mut cipher = Vec::new();
        for &m in plain {
            let c = BigUint::from(m).modpow(&d, &key.n).to_u32_digits().first().copied().unwrap_or(0);
            cipher.extend_from_slice(&(c as u16).to_be_bytes());
        }
        assert_eq!(key.decrypt(&cipher), plain);
    }

    #[test]
    fn parses_a_minimal_bhd5() {
        let mut d = Vec::new();
        d.extend_from_slice(b"BHD5");
        d.extend_from_slice(&[0xFF, 0, 1, 0]);
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&0i32.to_le_bytes()); // file size
        d.extend_from_slice(&1i32.to_le_bytes()); // bucket count
        d.extend_from_slice(&0x20i32.to_le_bytes()); // buckets offset
        d.extend_from_slice(&4i32.to_le_bytes());
        d.extend_from_slice(b"salt");
        assert_eq!(d.len(), 0x20);
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&0x28i32.to_le_bytes());
        let hash = path_hash("/x.bin");
        d.extend_from_slice(&hash.to_le_bytes());
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&5u32.to_le_bytes());
        d.extend_from_slice(&0x100u64.to_le_bytes());
        d.extend_from_slice(&0i64.to_le_bytes());
        d.extend_from_slice(&0i64.to_le_bytes());
        let bhd = Bhd5::parse(&d).unwrap();
        assert_eq!(bhd.salt, "salt");
        assert_eq!(bhd.entries.len(), 1);
        assert_eq!(bhd.entries[0].hash, hash);
        assert_eq!(bhd.entries[0].offset, 0x100);
        assert_eq!(bhd.entries[0].unpadded_size, 5);
    }

    #[test]
    fn aes_ranges_decrypt() {
        use aes::cipher::generic_array::GenericArray;
        use aes::cipher::{BlockEncrypt, KeyInit};
        let key = [7u8; 16];
        let plain = [42u8; 48];
        let mut data = plain;
        let cipher = aes::Aes128::new(GenericArray::from_slice(&key));
        cipher.encrypt_block(GenericArray::from_mut_slice(&mut data[16..32]));
        decrypt_aes_ranges(
            &mut data,
            &AesKey {
                key,
                ranges: vec![(16, 32), (-1, -1)],
            },
        );
        assert_eq!(data, plain);
    }
}
