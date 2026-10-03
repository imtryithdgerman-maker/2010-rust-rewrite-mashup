//! MW2 bullets into Elden Ring hit points.
use crate::ai::check_phase2;
use crate::enemy::{AiState, ErEnemy};

/// Ticks of red hurt flash after a hit (0.2 s).
pub const HURT_FLASH_TICKS: i32 = 4;

/// Location damage multipliers keyed by IW4L's hit location index
/// (`weapon_iw4::HITLOC_NAMES`):
///
/// | index | name                          | scale |
/// |-------|-------------------------------|-------|
/// | 0     | none (no hitloc, e.g. splash) | 1.0   |
/// | 1–3   | helmet, head, neck            | 1.5   |
/// | 4–5   | torso_upper, torso_lower      | 1.0   |
/// | 6–17  | arms, hands, legs, feet       | 0.75  |
/// | 18–19 | gun, shield                   | 0.0   |
///
/// Out-of-range indices are treated as body hits (1.0), matching
/// `weapon_iw4::location_damage_scale`'s fallback to slot 0.
pub fn hitloc_scale(hitloc: u8) -> f32 {
    match hitloc {
        1..=3 => 1.5,
        4 | 5 => 1.0,
        6..=17 => 0.75,
        18 | 19 => 0.0,
        _ => 1.0,
    }
}

/// Apply a MW2 bullet hit to an enemy.
/// `base_damage`: the weapon's raw damage value (already range/spread scaled by IW4L).
/// `hitloc`: IW4L hit location index (see [`hitloc_scale`]).
/// Returns true if the enemy was killed by this hit.
pub fn apply_bullet_hit(enemy: &mut ErEnemy, base_damage: f32, hitloc: u8) -> bool {
    if !enemy.is_alive() || !base_damage.is_finite() || base_damage <= 0.0 {
        return false;
    }
    let dealt = base_damage * hitloc_scale(hitloc);
    if dealt <= 0.0 {
        return false;
    }
    enemy.health = (enemy.health - dealt).max(0.0);
    enemy.hurt_flash = HURT_FLASH_TICKS;
    if enemy.health <= 0.0 {
        enemy.state = AiState::Dead;
        return true;
    }
    // Getting shot always provokes: idle enemies turn on the shooter.
    if matches!(enemy.state, AiState::Idle { .. }) {
        enemy.state = AiState::Alert;
    }
    check_phase2(enemy);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enemy::EnemyKind;
    use glam::Vec3;

    #[test]
    fn headshots_hurt_more_and_kill() {
        let mut e = ErEnemy::new(EnemyKind::GodrickSoldier, Vec3::ZERO, 0.0, 1);
        assert!(!apply_bullet_hit(&mut e, 40.0, 2));
        assert_eq!(e.health, 150.0 - 60.0);
        assert_eq!(e.state, AiState::Alert);
        assert!(!apply_bullet_hit(&mut e, 40.0, 12));
        assert_eq!(e.health, 90.0 - 30.0);
        assert!(apply_bullet_hit(&mut e, 100.0, 4));
        assert!(!e.is_alive());
        assert!(
            !apply_bullet_hit(&mut e, 100.0, 4),
            "dead enemies do not die twice"
        );
    }

    #[test]
    fn margit_phase2_on_half_health() {
        let mut m = ErEnemy::new(EnemyKind::Margit, Vec3::ZERO, 0.0, 1);
        apply_bullet_hit(&mut m, 1200.0, 4);
        assert!(m.phase2_triggered);
        assert_eq!(m.state, AiState::Phase2Chase);
    }
}
