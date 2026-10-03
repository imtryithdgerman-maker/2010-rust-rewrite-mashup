//! The crate's error type.
use std::path::PathBuf;

/// Everything that can go wrong reading Elden Ring's files.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{format}: bad magic {found:02x?}")]
    BadMagic { format: &'static str, found: Vec<u8> },
    #[error("{format}: data truncated reading {len} bytes at {offset:#x}")]
    Truncated {
        format: &'static str,
        offset: usize,
        len: usize,
    },
    #[error("{format}: {message}")]
    Format { format: &'static str, message: String },
    #[error("unsupported DCX compression {0:?}")]
    UnsupportedDcx(String),
    #[error("Oodle unavailable: {0}")]
    OodleUnavailable(String),
    #[error("decompression failed: {0}")]
    Decompress(String),
    #[error("no RSA key for encrypted archive {0}")]
    MissingKey(String),
    #[error("file not found in the archives: {0}")]
    NotFound(String),
    #[error("Elden Ring installation not found (set ELDEN_RING_PATH to its Game folder)")]
    GameNotFound,
    #[error("invalid map id {0:?}, expected mAA_BB_CC_DD")]
    BadMapId(String),
}

impl Error {
    pub(crate) fn format(format: &'static str, message: impl Into<String>) -> Self {
        Self::Format {
            format,
            message: message.into(),
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
