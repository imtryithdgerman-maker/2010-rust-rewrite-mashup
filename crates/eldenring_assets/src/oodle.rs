//! Oodle Kraken decompression through the game's own `oo2core_9_win64.dll`.
//!
//! Oodle is proprietary and cannot be shipped; Elden Ring installs the DLL
//! next to `eldenring.exe`, so it is loaded from there at run time. Without
//! it (non-Windows, or the DLL missing) KRAK-compressed DCX files are skipped
//! and only DFLT ones load.
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// DLL names tried in the game folder, newest first.
pub const DLL_NAMES: [&str; 3] = ["oo2core_9_win64.dll", "oo2core_8_win64.dll", "oo2core_6_win64.dll"];
/// Environment variable naming an Oodle library to load instead
/// (e.g. `liboo2corelinux64.so` off Windows).
pub const LIB_ENV: &str = "OODLE_LIB";

/// `OodleLZ_Decompress` as exported by oo2core 9 (14 arguments).
type DecompressFn = unsafe extern "system" fn(
    comp_buf: *const u8,
    comp_buf_size: isize,
    raw_buf: *mut u8,
    raw_len: isize,
    fuzz_safe: i32,
    check_crc: i32,
    verbosity: i32,
    dec_buf_base: *mut u8,
    dec_buf_size: isize,
    fp_callback: *mut c_void,
    callback_user_data: *mut c_void,
    decoder_memory: *mut c_void,
    decoder_memory_size: isize,
    thread_phase: i32,
) -> isize;

/// `OodleLZ_FuzzSafe_Yes`: Kraken supports bounds-checked decoding.
const FUZZ_SAFE: i32 = 1;
/// `OodleLZ_Decode_Unthreaded`.
const THREAD_PHASE_UNTHREADED: i32 = 3;

/// A loaded Oodle library.
pub struct Oodle {
    decompress: DecompressFn,
    path: PathBuf,
    // Keeps `decompress` valid; dropped last.
    _library: libloading::Library,
}

impl std::fmt::Debug for Oodle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Oodle").field("path", &self.path).finish()
    }
}

impl Oodle {
    /// Loads Oodle from `OODLE_LIB` or the game folder.
    pub fn load(game_dir: &Path) -> Result<Self> {
        let mut tried = Vec::new();
        let env = std::env::var_os(LIB_ENV).map(PathBuf::from);
        let candidates = env.into_iter().chain(DLL_NAMES.iter().map(|name| game_dir.join(name)));
        for path in candidates {
            if !path.is_file() {
                tried.push(format!("{} (missing)", path.display()));
                continue;
            }
            match Self::load_file(&path) {
                Ok(oodle) => return Ok(oodle),
                Err(error) => tried.push(format!("{}: {error}", path.display())),
            }
        }
        Err(Error::OodleUnavailable(tried.join("; ")))
    }

    /// Loads one Oodle library file.
    pub fn load_file(path: &Path) -> Result<Self> {
        // SAFETY: loading the game's own Oodle DLL runs only its loader-time
        // initialisation, which has no preconditions on the host.
        let library = unsafe { libloading::Library::new(path) }.map_err(|e| Error::OodleUnavailable(e.to_string()))?;
        // SAFETY: the symbol is `OodleLZ_Decompress` of oo2core 6..9, whose
        // signature `DecompressFn` matches.
        let decompress = unsafe {
            *library
                .get::<DecompressFn>(b"OodleLZ_Decompress\0")
                .map_err(|e| Error::OodleUnavailable(e.to_string()))?
        };
        Ok(Self {
            decompress,
            path: path.to_path_buf(),
            _library: library,
        })
    }

    /// Where this library was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Decompresses a Kraken stream to exactly `raw_len` bytes.
    pub fn decompress(&self, src: &[u8], raw_len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; raw_len];
        let src_len = isize::try_from(src.len()).map_err(|_| Error::Decompress("source too large".into()))?;
        let dst_len = isize::try_from(raw_len).map_err(|_| Error::Decompress("output too large".into()))?;
        // SAFETY: both buffers are valid for the lengths passed; the optional
        // pointers are null, which Oodle documents as "allocate internally".
        let written = unsafe {
            (self.decompress)(
                src.as_ptr(),
                src_len,
                out.as_mut_ptr(),
                dst_len,
                FUZZ_SAFE,
                0,
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                THREAD_PHASE_UNTHREADED,
            )
        };
        if written != dst_len {
            return Err(Error::Decompress(format!("Oodle wrote {written} of {raw_len} bytes")));
        }
        Ok(out)
    }
}
