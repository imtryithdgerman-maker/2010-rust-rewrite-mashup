//! Reads real Elden Ring game files from a Steam installation.
//!
//! Pipeline: [`steam`] finds the `Game` folder → [`archive`] opens the
//! RSA-encrypted BHD5 headers and reads (AES-decrypted) BDT entries by path
//! hash → [`dcx`] decompresses (DFLT via flate2, KRAK via the game's
//! [`oodle`] DLL) → [`bnd4`] unpacks containers → [`flver`], [`msb`] and
//! [`tpf`] decode models, map layouts and textures → [`convert`] turns them
//! into engine collision models → [`world_loader`] ties it together.
//!
//! Formats follow SoulsFormats/BinderTool; offsets not yet checked against
//! real files carry `TODO: verify against game files`.
pub mod archive;
pub mod bnd4;
pub mod convert;
pub mod dcx;
pub mod error;
pub mod flver;
pub mod keys;
pub mod msb;
pub mod oodle;
pub(crate) mod reader;
pub mod steam;
pub mod tpf;
pub mod world_loader;

pub use archive::{Archive, ArchiveSet};
pub use convert::{XModel, flver_to_xmodel, msb_to_scene, msb_to_scene_with};
pub use error::{Error, Result};
pub use flver::{Flver, FlverBone, FlverMesh};
pub use msb::{EnemyPlacement, MapLayout, MapPiece};
pub use steam::find_game_dir;
pub use tpf::TpfTexture;
pub use world_loader::{EldenRingMapData, load_map, load_map_from};
