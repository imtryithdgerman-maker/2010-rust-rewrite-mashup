//! Loads a whole map (layout, geometry, enemies, character textures) from
//! the game's archives.
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use glam::Mat4;

use crate::archive::ArchiveSet;
use crate::bnd4;
use crate::convert::{self, XModel};
use crate::error::{Error, Result};
use crate::flver::{self, Flver};
use crate::msb::{self, EnemyPlacement, MapLayout};
use crate::tpf::{self, TpfTexture};

/// Everything loaded for one map. Missing pieces are reported in
/// `warnings` rather than failing the whole load.
#[derive(Clone, Debug, Default)]
pub struct EldenRingMapData {
    pub map_id: String,
    pub layout: MapLayout,
    /// Placed world geometry (map pieces and assets).
    pub meshes: Vec<(XModel, Mat4)>,
    /// Decoded FLVERs by MSB model name (map pieces, assets, characters).
    pub flvers: HashMap<String, Flver>,
    pub enemies: Vec<EnemyPlacement>,
    /// Character textures of the enemies present.
    pub textures: Vec<TpfTexture>,
    pub warnings: Vec<String>,
}

/// The four numbers of a map id like `m20_00_00_00`.
pub fn parse_map_id(map_id: &str) -> Result<[u8; 4]> {
    let bad = || Error::BadMapId(map_id.to_owned());
    let rest = map_id.strip_prefix('m').ok_or_else(bad)?;
    let parts: Vec<&str> = rest.split('_').collect();
    if parts.len() != 4 || parts.iter().any(|p| p.len() != 2) {
        return Err(bad());
    }
    let mut out = [0; 4];
    for (o, p) in out.iter_mut().zip(parts) {
        *o = p.parse().map_err(|_| bad())?;
    }
    Ok(out)
}

// TODO: verify against game files — all archive paths below follow the
// layout seen in UXM-unpacked installs.
pub fn msb_path(map_id: &str) -> String {
    format!("/map/mapstudio/{map_id}.msb.dcx")
}

/// Map piece model `m000100` of `m20_00_00_00` →
/// `/map/m20/m20_00_00_00/m20_00_00_00_000100.mapbnd.dcx`.
pub fn map_piece_path(map_id: &str, model: &str) -> String {
    let area = &map_id[..3.min(map_id.len())];
    let suffix = model.strip_prefix('m').unwrap_or(model);
    format!("/map/{area}/{map_id}/{map_id}_{suffix}.mapbnd.dcx")
}

/// Asset model `AEG099_001` → `/asset/aeg/aeg099/aeg099_001.geombnd.dcx`.
pub fn asset_path(model: &str) -> Option<String> {
    let lower = model.to_ascii_lowercase();
    let (group, _) = lower.split_once('_')?;
    Some(format!("/asset/aeg/{group}/{lower}.geombnd.dcx"))
}

/// Character model `c4100` → `/chr/c4100.chrbnd.dcx`.
pub fn chr_path(model: &str) -> String {
    format!("/chr/{}.chrbnd.dcx", model.to_ascii_lowercase())
}

/// Character high-res textures → `/chr/c4100_h.texbnd.dcx`.
pub fn chr_texture_path(model: &str) -> String {
    format!("/chr/{}_h.texbnd.dcx", model.to_ascii_lowercase())
}

/// All FLVERs inside a (decompressed) BND4 container, or the file itself
/// if it already is a FLVER.
fn flvers_in(data: &[u8]) -> Result<Vec<Flver>> {
    if flver::is_flver(data) {
        return Ok(vec![flver::parse(data)?]);
    }
    bnd4::parse(data)?
        .iter()
        .filter(|f| f.has_extension(".flver") && flver::is_flver(&f.data))
        .map(|f| flver::parse(&f.data))
        .collect()
}

/// Merges a container's FLVERs into one (meshes concatenated).
fn load_model(archives: &ArchiveSet, path: &str) -> Result<Flver> {
    let data = archives.try_extract(path)?;
    let mut all = flvers_in(&data)?.into_iter();
    let mut first = all.next().ok_or_else(|| Error::format("BND4", format!("{path}: no FLVER inside")))?;
    for extra in all {
        first.meshes.extend(extra.meshes);
    }
    Ok(first)
}

/// Loads `map_id` from the installation at `game_dir` (its `Game` folder).
pub fn load_map(game_dir: &Path, map_id: &str) -> Result<EldenRingMapData> {
    let archives = ArchiveSet::open(game_dir)?;
    load_map_from(&archives, map_id)
}

/// Loads `map_id` from already opened archives.
pub fn load_map_from(archives: &ArchiveSet, map_id: &str) -> Result<EldenRingMapData> {
    parse_map_id(map_id)?;
    let msb_data = archives.try_extract(&msb_path(map_id))?;
    let layout = msb::parse(&msb_data)?;

    let mut data = EldenRingMapData {
        map_id: map_id.to_owned(),
        enemies: layout.enemies.clone(),
        warnings: layout.warnings.clone(),
        ..EldenRingMapData::default()
    };

    let mut xmodels: HashMap<String, XModel> = HashMap::new();
    let piece_models: BTreeSet<&str> = layout.map_pieces.iter().map(|p| p.model.as_str()).collect();
    let asset_models: BTreeSet<&str> = layout.assets.iter().map(|p| p.model.as_str()).collect();
    let jobs = piece_models
        .into_iter()
        .map(|m| (m, Some(map_piece_path(map_id, m))))
        .chain(asset_models.into_iter().map(|m| (m, asset_path(m))));
    for (model, path) in jobs {
        let Some(path) = path else {
            data.warnings.push(format!("{model}: unknown asset naming"));
            continue;
        };
        match load_model(archives, &path) {
            Ok(flver) => {
                xmodels.insert(model.to_owned(), convert::flver_file_to_xmodel(model, &flver));
                data.flvers.insert(model.to_owned(), flver);
            }
            Err(e) => data.warnings.push(format!("{model} ({path}): {e}")),
        }
    }
    data.meshes = convert::msb_to_scene_with(&layout, &xmodels);

    let chr_models: BTreeSet<String> = layout.enemies.iter().map(|e| e.model.to_ascii_lowercase()).collect();
    for model in chr_models {
        if model.is_empty() {
            continue;
        }
        match load_model(archives, &chr_path(&model)) {
            Ok(flver) => {
                data.flvers.insert(model.clone(), flver);
            }
            Err(e) => data.warnings.push(format!("{model}: {e}")),
        }
        match load_chr_textures(archives, &model) {
            Ok(tex) => data.textures.extend(tex),
            Err(e) => data.warnings.push(format!("{model} textures: {e}")),
        }
    }

    data.layout = layout;
    Ok(data)
}

fn load_chr_textures(archives: &ArchiveSet, model: &str) -> Result<Vec<TpfTexture>> {
    let data = archives.try_extract(&chr_texture_path(model))?;
    let mut out = Vec::new();
    for file in bnd4::parse(&data)?.iter().filter(|f| f.has_extension(".tpf")) {
        out.extend(tpf::parse(&file.data, archives.oodle())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_ids_and_paths() {
        assert_eq!(parse_map_id("m20_00_00_00").unwrap(), [20, 0, 0, 0]);
        assert!(parse_map_id("m20_00_00").is_err());
        assert!(parse_map_id("x20_00_00_00").is_err());
        assert_eq!(msb_path("m20_00_00_00"), "/map/mapstudio/m20_00_00_00.msb.dcx");
        assert_eq!(
            map_piece_path("m20_00_00_00", "m000100"),
            "/map/m20/m20_00_00_00/m20_00_00_00_000100.mapbnd.dcx"
        );
        assert_eq!(asset_path("AEG099_001").unwrap(), "/asset/aeg/aeg099/aeg099_001.geombnd.dcx");
        assert_eq!(chr_path("c4100"), "/chr/c4100.chrbnd.dcx");
    }

    #[test]
    fn flvers_in_bnd() {
        let f = flver::build_minimal(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], &[0, 1, 2]);
        let bnd = bnd4::build(&[(200, "N:\\m000100.flver", &f), (300, "x.txt", b"hi")]);
        let flvers = flvers_in(&bnd).unwrap();
        assert_eq!(flvers.len(), 1);
        assert_eq!(flvers[0].meshes[0].indices.len(), 3);
    }

    #[test]
    fn missing_game_dir_errors() {
        assert!(load_map(Path::new("/nonexistent/elden"), "m20_00_00_00").is_err());
    }
}
