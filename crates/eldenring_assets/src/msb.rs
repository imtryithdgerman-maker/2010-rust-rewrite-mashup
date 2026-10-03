//! MSBE map layouts (Elden Ring `.msb`): which models are placed where.
//!
//! Reference: SoulsFormats `MSBE.cs`, `MSBE.Model.cs`, `MSBE.Part.cs`.
//! Only the MODEL_PARAM_ST and PARTS_PARAM_ST sections are read; map
//! pieces, assets and enemies are extracted. Entries that fail to parse are
//! skipped and reported in [`MapLayout::warnings`].
use crate::error::{Error, Result};
use crate::reader::Bytes;

/// Part type codes (SoulsFormats `MSBE.PartType`).
pub mod part_type {
    pub const MAP_PIECE: i32 = 0;
    pub const ENEMY: i32 = 2;
    pub const PLAYER: i32 = 4;
    pub const COLLISION: i32 = 5;
    pub const DUMMY_ASSET: i32 = 9;
    pub const DUMMY_ENEMY: i32 = 10;
    pub const CONNECT_COLLISION: i32 = 11;
    pub const ASSET: i32 = 13;
}

/// A static model placed in the map (map piece or asset).
#[derive(Clone, Debug, PartialEq)]
pub struct MapPiece {
    pub name: String,
    /// Model name, e.g. `m000100` or `AEG099_001`.
    pub model: String,
    pub position: [f32; 3],
    /// Euler degrees.
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

/// An enemy spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemyPlacement {
    pub name: String,
    /// Character model, e.g. `c4100` (Margit).
    pub model: String,
    pub position: [f32; 3],
    /// Euler degrees.
    pub rotation: [f32; 3],
    pub npc_param_id: i32,
    pub think_param_id: i32,
}

/// The parts of a map relevant to building a playable scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapLayout {
    /// MODEL_PARAM_ST names, indexed by a part's model index.
    pub models: Vec<String>,
    pub map_pieces: Vec<MapPiece>,
    pub assets: Vec<MapPiece>,
    pub enemies: Vec<EnemyPlacement>,
    pub warnings: Vec<String>,
}

const MODEL_PARAM: &str = "MODEL_PARAM_ST";
const PARTS_PARAM: &str = "PARTS_PARAM_ST";

/// Entry offsets of every param section, keyed by section name.
fn sections(b: &Bytes<'_>) -> Result<Vec<(String, Vec<usize>)>> {
    let mut out = Vec::new();
    let mut at = 0x10;
    // A handful of sections exist; the bound guards against cycles.
    for _ in 0..32 {
        let count = b.count_i32(at + 4)?;
        if count == 0 {
            return Err(Error::format("MSB", format!("param at {at:#x} has no offsets")));
        }
        let name = b.utf16z(b.offset_i64(at + 8)?)?;
        let entries = (0..count - 1).map(|i| b.offset_i64(at + 0x10 + i * 8)).collect::<Result<Vec<_>>>()?;
        let next = b.offset_i64(at + 0x10 + (count - 1) * 8)?;
        out.push((name, entries));
        if next == 0 || next <= at {
            break;
        }
        at = next;
    }
    Ok(out)
}

/// Parses a (decompressed) MSBE file.
pub fn parse(data: &[u8]) -> Result<MapLayout> {
    let b = Bytes::new(data, "MSB");
    b.expect_magic(0, b"MSB ")?;
    if b.i32(4)? != 1 || b.i32(8)? != 0x10 {
        return Err(Error::format("MSB", "unexpected header"));
    }
    let sections = sections(&b)?;
    let find = |name: &str| sections.iter().find(|(n, _)| n == name).map(|(_, e)| e.as_slice());

    let mut layout = MapLayout::default();
    let models = find(MODEL_PARAM).ok_or_else(|| Error::format("MSB", "MODEL_PARAM_ST missing"))?;
    for &at in models {
        let name = b.offset_i64(at).and_then(|o| b.utf16z(o)).unwrap_or_else(|e| {
            layout.warnings.push(format!("model at {at:#x}: {e}"));
            String::new()
        });
        layout.models.push(name);
    }

    let parts = find(PARTS_PARAM).ok_or_else(|| Error::format("MSB", "PARTS_PARAM_ST missing"))?;
    for &at in parts {
        if let Err(e) = read_part(&b, at, &mut layout) {
            layout.warnings.push(format!("part at {at:#x}: {e}"));
        }
    }
    Ok(layout)
}

fn read_part(b: &Bytes<'_>, at: usize, layout: &mut MapLayout) -> Result<()> {
    // TODO: verify against game files (MSBE part header offsets)
    let name = b.utf16z(b.offset_i64(at)?)?;
    let kind = b.i32(at + 0x08)?;
    let model_index = b.i32(at + 0x10)?;
    let position = b.vec3(at + 0x20)?;
    let rotation = b.vec3(at + 0x2C)?;
    let scale = b.vec3(at + 0x38)?;
    let model = usize::try_from(model_index)
        .ok()
        .and_then(|i| layout.models.get(i))
        .cloned()
        .unwrap_or_else(String::new);

    match kind {
        part_type::MAP_PIECE | part_type::ASSET => {
            let piece = MapPiece {
                name,
                model,
                position,
                rotation,
                scale,
            };
            if kind == part_type::MAP_PIECE {
                layout.map_pieces.push(piece);
            } else {
                layout.assets.push(piece);
            }
        }
        part_type::ENEMY => {
            // TODO: verify against game files (type-data pointer slot and
            // the ThinkParamID/NPCParamID offsets inside it)
            let type_data = b.offset_i64(at + 0x68)?;
            let think_param_id = b.i32(type_data + 0x08)?;
            let npc_param_id = b.i32(type_data + 0x0C)?;
            layout.enemies.push(EnemyPlacement {
                name,
                model,
                position,
                rotation,
                npc_param_id,
                think_param_id,
            });
        }
        _ => {}
    }
    Ok(())
}

/// Test helper: builds an MSB with the given models and parts
/// `(name, type, model_index, position, npc_param_id)`.
#[cfg(test)]
pub(crate) fn build(models: &[&str], parts: &[(&str, i32, i32, [f32; 3], i32)]) -> Vec<u8> {
    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }
    fn section(out: &mut Vec<u8>, name: &str, entries: Vec<Vec<u8>>, last: bool) {
        let start = out.len();
        let count = entries.len() + 1;
        let header = 0x10 + count * 8;
        let name_at = start + header;
        let name_bytes = utf16(name);
        let mut entry_at = (name_at + name_bytes.len()).next_multiple_of(8);
        let mut offsets = Vec::new();
        let mut body = Vec::new();
        for e in &entries {
            offsets.push(entry_at);
            body.extend_from_slice(e);
            entry_at += e.len();
        }
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&(count as i32).to_le_bytes());
        out.extend_from_slice(&(name_at as i64).to_le_bytes());
        for o in &offsets {
            out.extend_from_slice(&(*o as i64).to_le_bytes());
        }
        let next = if last { 0 } else { entry_at.next_multiple_of(8) };
        out.extend_from_slice(&(next as i64).to_le_bytes());
        out.extend_from_slice(&name_bytes);
        out.resize(out.len().next_multiple_of(8), 0);
        // Entries contain absolute offsets relative to their own start; patch.
        for (e, o) in entries.iter().zip(&offsets) {
            let mut e = e.clone();
            let rel = i64::from_le_bytes(e[0..8].try_into().unwrap());
            e[0..8].copy_from_slice(&(rel + *o as i64).to_le_bytes());
            if e.len() > 0x70 {
                let td = i64::from_le_bytes(e[0x68..0x70].try_into().unwrap());
                e[0x68..0x70].copy_from_slice(&(td + *o as i64).to_le_bytes());
            }
            out.extend_from_slice(&e);
        }
        out.resize(out.len().next_multiple_of(8), 0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"MSB ");
    out.extend_from_slice(&1i32.to_le_bytes());
    out.extend_from_slice(&0x10i32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 1, 0xFF]);

    let model_entries = models
        .iter()
        .map(|m| {
            let mut e = vec![0u8; 0x20];
            e[0..8].copy_from_slice(&0x20i64.to_le_bytes());
            e.extend(utf16(m));
            e.resize(e.len().next_multiple_of(8), 0);
            e
        })
        .collect();
    section(&mut out, MODEL_PARAM, model_entries, false);

    let part_entries = parts
        .iter()
        .map(|(name, kind, model, pos, npc)| {
            let mut e = vec![0u8; 0x80];
            e[0..8].copy_from_slice(&0x80i64.to_le_bytes());
            e[8..12].copy_from_slice(&kind.to_le_bytes());
            e[0x10..0x14].copy_from_slice(&model.to_le_bytes());
            for (k, c) in pos.iter().enumerate() {
                e[0x20 + k * 4..0x24 + k * 4].copy_from_slice(&c.to_le_bytes());
            }
            for k in 0..3 {
                e[0x38 + k * 4..0x3C + k * 4].copy_from_slice(&1f32.to_le_bytes());
            }
            let mut name_bytes = utf16(name);
            name_bytes.resize(name_bytes.len().next_multiple_of(8), 0);
            let td = 0x80 + name_bytes.len();
            e[0x68..0x70].copy_from_slice(&(td as i64).to_le_bytes());
            e.extend(name_bytes);
            let mut type_data = vec![0u8; 0x20];
            type_data[0x08..0x0C].copy_from_slice(&1i32.to_le_bytes());
            type_data[0x0C..0x10].copy_from_slice(&npc.to_le_bytes());
            e.extend(type_data);
            e
        })
        .collect();
    section(&mut out, PARTS_PARAM, part_entries, true);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_msb() {
        let data = build(
            &["m000100", "c4100", "AEG099_001"],
            &[
                ("m000100_0000", part_type::MAP_PIECE, 0, [1.0, 2.0, 3.0], 0),
                ("c4100_9000", part_type::ENEMY, 1, [10.0, 0.0, -5.0], 41000000),
                ("AEG099_001_1000", part_type::ASSET, 2, [0.0; 3], 0),
                ("collision", part_type::COLLISION, 0, [0.0; 3], 0),
            ],
        );
        let layout = parse(&data).unwrap();
        assert!(layout.warnings.is_empty(), "{:?}", layout.warnings);
        assert_eq!(layout.models, vec!["m000100", "c4100", "AEG099_001"]);
        assert_eq!(layout.map_pieces.len(), 1);
        assert_eq!(layout.map_pieces[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(layout.map_pieces[0].scale, [1.0; 3]);
        assert_eq!(layout.assets[0].model, "AEG099_001");
        let margit = &layout.enemies[0];
        assert_eq!(margit.model, "c4100");
        assert_eq!(margit.name, "c4100_9000");
        assert_eq!(margit.npc_param_id, 41000000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"MSB \x01\0\0\0").is_err());
        assert!(parse(b"NOPE").is_err());
    }
}
