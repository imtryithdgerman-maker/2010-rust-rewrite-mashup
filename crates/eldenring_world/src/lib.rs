//! Elden Ring layer for IW4L: a Stormveil courtyard arena built from
//! Minecraft blocks (so the existing `minecraft_terrain` renderer draws it)
//! and a set of Elden Ring enemies that MW2 guns can shoot and that hit back.
//!
//! The crate is pure simulation: it never touches the renderer, the network
//! or IW4L's trace system directly. The game loop feeds it the player
//! position each server tick, registers [`ErWorldSim::enemy_capsules`] as
//! bullet targets, and forwards hits to [`ErWorldSim::bullet_hit`].
//! See `docs/ELDENRING_INTEGRATION.md` for the wiring.

pub mod ai;
pub mod arena;
pub mod boss_bar;
pub mod client_view;
pub mod damage;
pub mod enemy;
pub mod world;

pub use ai::tick_enemy;
pub use arena::{ARENA_MAP_NAME, ARENA_SIZE, arena_scene, default_spawns, player_spawn};
pub use boss_bar::BossBarState;
pub use client_view::{EnemySnapshot, ErWorldSnapshot};
pub use damage::{apply_bullet_hit, hitloc_scale};
pub use enemy::{AiState, EnemyKind, ErEnemy};
pub use world::{ER_MOB_KEY_BASE, ErKill, ErWorldSim};

/// Server ticks per second (IW4L's `sv_fps` default of 20).
pub const TICKS_PER_SECOND: i32 = 20;
/// Seconds per server tick.
pub const TICK_SECONDS: f32 = 1.0 / TICKS_PER_SECOND as f32;
