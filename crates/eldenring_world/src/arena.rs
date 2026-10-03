//! The Stormveil courtyard arena, authored as Minecraft blocks so the existing
//! `minecraft_terrain` mesher, lighting and collision see it like any other
//! handcrafted map.
//!
//! Layout (block coordinates, Y up, north = -Z like Minecraft):
//!
//! ```text
//!   z=0  ####### [fog gate] #######     north wall + glass fog gate (x 31..=33)
//!        # C        K   K        C #    C = mossy 3x3 columns, K = knights
//!        #     r          r        #    r = cracked-brick rubble
//!        #        B    M     B     #    B = broken wall chunks, M = Margit
//!        #     r          r        #
//!        # C   S  S  S  S  S  S  C #    S = soldiers near the south wall
//!   z=63 ###########################
//! ```
use crate::enemy::EnemyKind;
use glam::Vec3;
use minecraft_terrain::scene::{Block, BlockPos, HandcraftedScene};

/// Map name used by the map picker and `map` console command.
pub const ARENA_MAP_NAME: &str = "stormveil_arena";
/// Arena edge length in blocks (x and z run `0..ARENA_SIZE`).
pub const ARENA_SIZE: i32 = 64;
/// Top of the walkable floor: feet stand at this Y.
pub const FLOOR_TOP_Y: f32 = 1.0;
/// Height of the perimeter wall (Y 1..=WALL_HEIGHT).
pub const WALL_HEIGHT: i32 = 6;
/// Fog gate opening, inclusive X range in the north wall.
pub const FOG_GATE_X: std::ops::RangeInclusive<i32> = 31..=33;
/// Fog gate height in blocks.
pub const FOG_GATE_HEIGHT: i32 = 10;

const STONE_BRICKS: &str = "minecraft:stone_bricks";
const MOSSY_STONE_BRICKS: &str = "minecraft:mossy_stone_bricks";
const CRACKED_STONE_BRICKS: &str = "minecraft:cracked_stone_bricks";
const CHISELED_STONE_BRICKS: &str = "minecraft:chiseled_stone_bricks";
const GLASS: &str = "minecraft:glass";

/// Rubble piles: `(x, z, height)`. Kept off spawn points and the gate lane.
const RUBBLE: &[(i32, i32, i32)] = &[
    (12, 18, 1),
    (13, 18, 2),
    (12, 19, 1),
    (50, 17, 2),
    (51, 17, 1),
    (50, 18, 1),
    (18, 44, 2),
    (19, 44, 1),
    (45, 46, 1),
    (46, 46, 2),
    (46, 47, 1),
    (26, 24, 1),
    (38, 41, 1),
    (9, 33, 2),
    (54, 30, 2),
    (54, 31, 1),
];

fn set(scene: &mut HandcraftedScene, pos: BlockPos, id: &str) {
    scene.set(pos, Some(Block::new(id)));
}

/// Build the 64x64 Stormveil courtyard.
pub fn arena_scene() -> HandcraftedScene {
    // `default()` is empty; `HandcraftedScene::new()` would add the demo map.
    let mut s = HandcraftedScene::default();
    let n = ARENA_SIZE;

    // Two layers of floor: Y=-1 foundation, Y=0 walking surface.
    for x in 0..n {
        for z in 0..n {
            set(&mut s, (x, -1, z), STONE_BRICKS);
            // Weathered checker so the floor reads at distance.
            let id = if (x * 7 + z * 13) % 11 == 0 {
                CRACKED_STONE_BRICKS
            } else {
                STONE_BRICKS
            };
            set(&mut s, (x, 0, z), id);
        }
    }

    // Perimeter walls Y 1..=6 with merlons every 4 blocks at Y 7.
    for i in 0..n {
        for (x, z) in [(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
            let in_gate = z == 0 && FOG_GATE_X.contains(&x);
            if in_gate {
                continue;
            }
            for y in 1..=WALL_HEIGHT {
                let id = if y == 1 && (x + z) % 5 == 0 {
                    MOSSY_STONE_BRICKS
                } else {
                    STONE_BRICKS
                };
                set(&mut s, (x, y, z), id);
            }
            if i % 4 == 0 {
                set(&mut s, (x, WALL_HEIGHT + 1, z), STONE_BRICKS);
            }
        }
    }

    // Fog gate: chiseled frame pillars either side, lintel on top, and a
    // glass "fog" pane filling the 3-wide, 10-high opening.
    let gx0 = *FOG_GATE_X.start();
    let gx1 = *FOG_GATE_X.end();
    for y in 1..=FOG_GATE_HEIGHT + 1 {
        set(&mut s, (gx0 - 1, y, 0), CHISELED_STONE_BRICKS);
        set(&mut s, (gx1 + 1, y, 0), CHISELED_STONE_BRICKS);
    }
    for x in gx0 - 1..=gx1 + 1 {
        set(&mut s, (x, FOG_GATE_HEIGHT + 1, 0), CHISELED_STONE_BRICKS);
    }
    for x in FOG_GATE_X {
        for y in 1..=FOG_GATE_HEIGHT {
            set(&mut s, (x, y, 0), GLASS);
        }
    }

    // Four mossy 3x3 columns, height 8, one near each inner corner.
    for (cx, cz) in [(4, 4), (n - 7, 4), (4, n - 7), (n - 7, n - 7)] {
        for x in cx..cx + 3 {
            for z in cz..cz + 3 {
                for y in 1..=8 {
                    let id = if y == 8 {
                        CHISELED_STONE_BRICKS
                    } else {
                        MOSSY_STONE_BRICKS
                    };
                    set(&mut s, (x, y, z), id);
                }
            }
        }
    }

    // Scattered rubble cover.
    for &(x, z, h) in RUBBLE {
        for y in 1..=h {
            set(&mut s, (x, y, z), CRACKED_STONE_BRICKS);
        }
    }

    // Two broken wall chunks mid-arena: ragged 5-long, 2-thick, stepped tops.
    for (x0, z0) in [(20, 30), (40, 32)] {
        for dx in 0..5 {
            for dz in 0..2 {
                let h = [3, 2, 3, 1, 2][dx as usize] - dz;
                for y in 1..=h.max(1) {
                    let id = if (dx + y) % 3 == 0 {
                        CRACKED_STONE_BRICKS
                    } else {
                        STONE_BRICKS
                    };
                    set(&mut s, (x0 + dx, y, z0 + dz), id);
                }
            }
        }
    }
    s
}

/// Player spawn: just inside the fog gate, facing south into the courtyard.
/// Returns `(feet_position, yaw_radians)`; yaw 0 faces +Z.
pub fn player_spawn() -> (Vec3, f32) {
    (Vec3::new(32.5, FLOOR_TOP_Y, 2.5), 0.0)
}

/// Default enemy roster: `(kind, feet_position, yaw)`.
/// 6 soldiers along the south wall, 2 knights guarding the fog gate approach,
/// Margit at the centre facing the gate.
pub fn default_spawns() -> Vec<(EnemyKind, Vec3, f32)> {
    let south = std::f32::consts::PI; // facing -Z (toward the gate)
    let mut v = Vec::with_capacity(9);
    for i in 0..6 {
        let x = 14.5 + i as f32 * 7.0;
        v.push((
            EnemyKind::GodrickSoldier,
            Vec3::new(x, FLOOR_TOP_Y, 56.5),
            south,
        ));
    }
    v.push((
        EnemyKind::GodrickKnight,
        Vec3::new(27.5, FLOOR_TOP_Y, 11.5),
        south,
    ));
    v.push((
        EnemyKind::GodrickKnight,
        Vec3::new(37.5, FLOOR_TOP_Y, 11.5),
        south,
    ));
    v.push((EnemyKind::Margit, Vec3::new(32.5, FLOOR_TOP_Y, 32.5), south));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraft_terrain::scene::Scene;

    #[test]
    fn arena_shape() {
        let s = arena_scene();
        let id = |p| s.block(p).map(|b| b.id.path.clone());
        assert_eq!(id((10, 0, 10)).as_deref().is_some(), true);
        assert!(s.block((10, -1, 10)).is_some());
        assert_eq!(id((10, 3, 0)).as_deref(), Some("stone_bricks"));
        assert_eq!(id((32, 5, 0)).as_deref(), Some("glass"));
        assert_eq!(id((5, 8, 5)).as_deref(), Some("chiseled_stone_bricks"));
        assert!(s.block((0, 7, 0)).is_some() && s.block((1, 7, 0)).is_none());
        // Spawn points stand on floor with headroom.
        for (_, p, _) in default_spawns() {
            let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
            assert!(s.block((x, 0, z)).is_some(), "floor under {p}");
            for y in 1..=3 {
                assert!(s.block((x, y, z)).is_none(), "spawn {p} blocked at y={y}");
            }
        }
        let (p, _) = player_spawn();
        assert!(s.block((p.x as i32, 1, p.z as i32)).is_none());
    }
}
