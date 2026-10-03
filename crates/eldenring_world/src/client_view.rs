//! Read-only per-tick view of the ER world for the renderer and HUD.
use crate::boss_bar::BossBarState;
use glam::Vec3;

/// One enemy as the renderer sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemySnapshot {
    /// Index into `ErWorldSim::enemies` (stable for the life of the sim).
    pub index: usize,
    /// Display name of the enemy kind; also selects the placeholder model.
    pub kind_name: &'static str,
    /// Feet position in world metres.
    pub position: Vec3,
    /// Facing yaw in radians (0 = +Z).
    pub yaw: f32,
    /// Remaining health, `0.0..=1.0`.
    pub health_fraction: f32,
    /// Tint the model red this frame.
    pub hurt_flash: bool,
    /// Draw as a corpse (or fade out).
    pub dead: bool,
    /// Margit's phase 2 (golden glow, cane).
    pub phase2: bool,
    /// Capsule radius in metres (placeholder model scale).
    pub radius: f32,
    /// Capsule full height in metres (placeholder model scale).
    pub height: f32,
}

/// Everything the client draws for the ER layer this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErWorldSnapshot {
    /// All enemies, living and dead, in index order.
    pub enemies: Vec<EnemySnapshot>,
    /// Boss bar, when Margit is alive.
    pub boss_bar: Option<BossBarState>,
    /// Melee damage enemies dealt to the player and not yet drained.
    pub player_damage_this_tick: f32,
    /// Total runes earned this game.
    pub runes_earned: u32,
}
