//! Enemy kinds, their stat blocks, and the per-enemy state record.
use glam::Vec3;

/// Which kind of Elden Ring enemy this is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EnemyKind {
    /// Rank-and-file Stormveil foot soldier.
    GodrickSoldier,
    /// Armoured knight guarding the boss approach.
    GodrickKnight,
    /// Margit, the Fell Omen: the arena boss, with a second phase.
    Margit,
}

/// AI state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AiState {
    /// Wandering; `patrol_timer` counts ticks until the next yaw change.
    Idle { patrol_timer: i32 },
    /// Spotted the player this tick; becomes [`AiState::Chase`] next tick.
    Alert,
    /// Moving toward the player.
    Chase,
    /// Winding up a melee swing; it lands when `windup` reaches 0.
    AttackMelee { windup: i32 },
    /// Margit's enraged chase (phase 2).
    Phase2Chase,
    /// Margit's faster phase-2 swing.
    Phase2Attack { windup: i32 },
    /// Dead; the corpse stays for the renderer until the world removes it.
    Dead,
}

/// One living (or recently dead) Elden Ring enemy.
#[derive(Clone, Debug, PartialEq)]
pub struct ErEnemy {
    /// Enemy kind; selects the stat block.
    pub kind: EnemyKind,
    /// Feet position in world metres (IW4L/Minecraft Y-up block space).
    pub position: Vec3,
    /// Facing yaw in radians, 0 = +Z, increasing toward +X.
    pub yaw: f32,
    /// Current hit points.
    pub health: f32,
    /// Hit points at spawn.
    pub max_health: f32,
    /// AI state.
    pub state: AiState,
    /// Ticks until next melee is allowed.
    pub attack_cooldown: i32,
    /// Ticks of red hurt flash remaining.
    pub hurt_flash: i32,
    /// Margit only: phase 2 has started.
    pub phase2_triggered: bool,
    /// Margit only: phase-2 projectile attack (golden daggers) is unlocked.
    /// Not yet simulated; exposed so the renderer/FX can react.
    pub projectile_unlocked: bool,
    /// The kill reward has been paid out.
    pub runes_awarded: bool,
    /// Where the enemy spawned; idle patrol stays near it.
    pub home: Vec3,
    /// Deterministic per-enemy RNG state for patrol drift.
    pub rng: u32,
}

impl EnemyKind {
    /// Hit points at spawn. Tuned so an M4A1 (30/20 dmg) needs ~5 body shots
    /// on a soldier and roughly three magazines on Margit.
    pub fn max_health(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 150.0,
            Self::GodrickKnight => 400.0,
            Self::Margit => 2400.0,
        }
    }
    /// Base HP damage to the player per landed melee (MW2 player has 100 HP,
    /// 30 in hardcore).
    pub fn melee_damage(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 15.0,
            Self::GodrickKnight => 25.0,
            Self::Margit => 40.0,
        }
    }
    /// Melee reach in metres, measured horizontally from the enemy axis.
    pub fn melee_range(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 1.8,
            Self::GodrickKnight => 2.2,
            Self::Margit => 3.5,
        }
    }
    /// Hitbox capsule radius in metres.
    pub fn capsule_radius(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 0.6,
            Self::GodrickKnight => 0.7,
            Self::Margit => 1.0,
        }
    }
    /// Half the total hitbox height in metres (soldier 1.8 m, knight 2.0 m,
    /// Margit 2.8 m tall).
    pub fn capsule_half_height(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 0.9,
            Self::GodrickKnight => 1.0,
            Self::Margit => 1.4,
        }
    }
    /// Distance at which an idle enemy notices the player.
    pub fn alert_range(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 16.0,
            Self::GodrickKnight => 20.0,
            Self::Margit => 40.0,
        }
    }
    /// Chase speed in metres per second.
    pub fn move_speed(&self) -> f32 {
        match self {
            Self::GodrickSoldier => 3.5,
            Self::GodrickKnight => 3.0,
            Self::Margit => 4.5,
        }
    }
    /// Ticks between melee swings.
    pub fn attack_cooldown_ticks(&self) -> i32 {
        match self {
            Self::GodrickSoldier => 30,
            Self::GodrickKnight => 40,
            Self::Margit => 36,
        }
    }
    /// Runes paid out on kill.
    pub fn rune_reward(&self) -> u32 {
        match self {
            Self::GodrickSoldier => 220,
            Self::GodrickKnight => 600,
            Self::Margit => 12_000,
        }
    }
    /// Display name (HUD, boss bar, logs).
    pub fn name(&self) -> &'static str {
        match self {
            Self::GodrickSoldier => "Godrick Soldier",
            Self::GodrickKnight => "Godrick Knight",
            Self::Margit => "Margit, the Fell Omen",
        }
    }
    /// Whether this kind shows a boss health bar.
    pub fn is_boss(&self) -> bool {
        matches!(self, Self::Margit)
    }
}

impl ErEnemy {
    /// Spawn an enemy of `kind` at `position` facing `yaw`. `seed` makes the
    /// patrol drift deterministic (replays and killcams stay in sync).
    pub fn new(kind: EnemyKind, position: Vec3, yaw: f32, seed: u32) -> Self {
        Self {
            kind,
            position,
            yaw,
            health: kind.max_health(),
            max_health: kind.max_health(),
            state: AiState::Idle { patrol_timer: 0 },
            attack_cooldown: 0,
            hurt_flash: 0,
            phase2_triggered: false,
            projectile_unlocked: false,
            runes_awarded: false,
            home: position,
            rng: seed.wrapping_mul(0x9E37_79B9) | 1,
        }
    }
    /// Whether the enemy is alive.
    pub fn is_alive(&self) -> bool {
        !matches!(self.state, AiState::Dead)
    }
    /// Health in `0.0..=1.0`.
    pub fn health_fraction(&self) -> f32 {
        if self.max_health <= 0.0 {
            0.0
        } else {
            (self.health / self.max_health).clamp(0.0, 1.0)
        }
    }
    /// Hitbox capsule as `(bottom_center, top_center, radius)`, where the
    /// centres are the sphere centres of the capsule caps (so the capsule's
    /// extent runs from the feet to `2 * half_height` above them).
    pub fn capsule(&self) -> (Vec3, Vec3, f32) {
        let r = self.kind.capsule_radius();
        let h = self.kind.capsule_half_height() * 2.0;
        let bottom = self.position + Vec3::Y * r;
        let top = self.position + Vec3::Y * (h - r).max(r);
        (bottom, top, r)
    }
    /// Next pseudo-random value (xorshift32).
    pub(crate) fn next_rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }
}
