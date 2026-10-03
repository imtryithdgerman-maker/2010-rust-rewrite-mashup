//! Per-enemy AI: idle patrol -> alert -> chase -> melee, plus Margit's phase 2.
use crate::arena::ARENA_SIZE;
use crate::enemy::{AiState, EnemyKind, ErEnemy};
use crate::{TICK_SECONDS, TICKS_PER_SECOND};
use glam::Vec3;

/// Ticks of windup before a normal melee swing lands (0.5 s).
pub const MELEE_WINDUP_TICKS: i32 = 10;
/// Ticks of windup before a phase-2 swing lands (0.3 s).
pub const PHASE2_WINDUP_TICKS: i32 = 6;
/// Ticks between idle yaw changes (3 s).
pub const PATROL_INTERVAL_TICKS: i32 = 3 * TICKS_PER_SECOND;
/// Idle walking speed as a fraction of chase speed.
const PATROL_SPEED_SCALE: f32 = 0.25;
/// Idle enemies turn back toward home when further than this.
const PATROL_LEASH: f32 = 6.0;
/// Phase-2 movement speed multiplier.
const PHASE2_SPEED_SCALE: f32 = 1.3;
/// Enemies give up the chase beyond this multiple of their alert range.
const LOSE_INTEREST_SCALE: f32 = 2.0;

/// Horizontal (XZ) distance between two points.
fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    (d.x * d.x + d.z * d.z).sqrt()
}

/// Yaw that faces from `from` toward `to` (0 = +Z, toward +X positive).
fn yaw_toward(from: Vec3, to: Vec3) -> f32 {
    let d = to - from;
    d.x.atan2(d.z)
}

/// Keep feet inside the arena's walkable interior (walls occupy the border).
fn clamp_to_arena(p: Vec3) -> Vec3 {
    let lo = 1.5;
    let hi = ARENA_SIZE as f32 - 1.5;
    Vec3::new(p.x.clamp(lo, hi), p.y, p.z.clamp(lo, hi))
}

/// Step toward `target` by at most `speed * TICK_SECONDS`, stopping at `stop_at`.
fn step_toward(enemy: &mut ErEnemy, target: Vec3, speed: f32, stop_at: f32) {
    enemy.yaw = yaw_toward(enemy.position, target);
    let dist = flat_distance(enemy.position, target);
    let step = (speed * TICK_SECONDS).min((dist - stop_at).max(0.0));
    if step > 0.0 {
        let dir = Vec3::new(enemy.yaw.sin(), 0.0, enemy.yaw.cos());
        enemy.position = clamp_to_arena(enemy.position + dir * step);
    }
}

/// Start Margit's second phase when she drops to half health. Returns true if
/// the transition happened this call. Safe to call for any enemy.
pub fn check_phase2(enemy: &mut ErEnemy) -> bool {
    if enemy.kind != EnemyKind::Margit
        || enemy.phase2_triggered
        || !enemy.is_alive()
        || enemy.health > enemy.max_health * 0.5
    {
        return false;
    }
    enemy.phase2_triggered = true;
    enemy.projectile_unlocked = true;
    enemy.attack_cooldown = enemy.attack_cooldown.min(cooldown_for(enemy));
    enemy.state = AiState::Phase2Chase;
    true
}

/// Melee cooldown for this enemy in its current phase (halved in phase 2).
fn cooldown_for(enemy: &ErEnemy) -> i32 {
    let base = enemy.kind.attack_cooldown_ticks();
    if enemy.phase2_triggered {
        base / 2
    } else {
        base
    }
}

fn idle_patrol(enemy: &mut ErEnemy) {
    let timer = match enemy.state {
        AiState::Idle { patrol_timer } => patrol_timer,
        _ => 0,
    };
    let timer = if timer <= 0 {
        if flat_distance(enemy.position, enemy.home) > PATROL_LEASH {
            enemy.yaw = yaw_toward(enemy.position, enemy.home);
        } else {
            // Drift up to +-90 degrees.
            let r = (enemy.next_rand() % 1000) as f32 / 1000.0;
            enemy.yaw += (r - 0.5) * std::f32::consts::PI;
        }
        PATROL_INTERVAL_TICKS
    } else {
        timer - 1
    };
    // Margit holds her ground in the centre until provoked.
    if enemy.kind != EnemyKind::Margit {
        let dir = Vec3::new(enemy.yaw.sin(), 0.0, enemy.yaw.cos());
        let speed = enemy.kind.move_speed() * PATROL_SPEED_SCALE;
        enemy.position = clamp_to_arena(enemy.position + dir * speed * TICK_SECONDS);
    }
    enemy.state = AiState::Idle {
        patrol_timer: timer,
    };
}

/// Advance one enemy's AI by one server tick (1/20 s).
/// `player_pos`: current player feet position (or None if dead / spectating).
/// Returns accumulated player damage this tick from this enemy.
pub fn tick_enemy(enemy: &mut ErEnemy, player_pos: Option<Vec3>) -> f32 {
    if !enemy.is_alive() {
        return 0.0;
    }
    enemy.attack_cooldown = (enemy.attack_cooldown - 1).max(0);
    enemy.hurt_flash = (enemy.hurt_flash - 1).max(0);
    check_phase2(enemy);

    let Some(player) = player_pos else {
        // Nobody to fight: Margit keeps her phase but calms down.
        idle_patrol(enemy);
        return 0.0;
    };

    let kind = enemy.kind;
    let dist = flat_distance(enemy.position, player);
    let in_reach = dist <= kind.melee_range() && (player.y - enemy.position.y).abs() < 2.5;
    let mut damage = 0.0;

    let state = enemy.state;
    enemy.state = match state {
        AiState::Idle { .. } => {
            if dist <= kind.alert_range() {
                enemy.yaw = yaw_toward(enemy.position, player);
                AiState::Alert
            } else {
                idle_patrol(enemy);
                enemy.state
            }
        }
        AiState::Alert => {
            if enemy.phase2_triggered {
                AiState::Phase2Chase
            } else {
                AiState::Chase
            }
        }
        AiState::Chase | AiState::Phase2Chase => {
            let phase2 = matches!(enemy.state, AiState::Phase2Chase);
            if dist > kind.alert_range() * LOSE_INTEREST_SCALE && !kind.is_boss() {
                AiState::Idle { patrol_timer: 0 }
            } else if in_reach && enemy.attack_cooldown == 0 {
                enemy.yaw = yaw_toward(enemy.position, player);
                if phase2 {
                    AiState::Phase2Attack {
                        windup: PHASE2_WINDUP_TICKS,
                    }
                } else {
                    AiState::AttackMelee {
                        windup: MELEE_WINDUP_TICKS,
                    }
                }
            } else {
                let speed = kind.move_speed() * if phase2 { PHASE2_SPEED_SCALE } else { 1.0 };
                // Close to ~80% of reach so the swing connects.
                step_toward(enemy, player, speed, kind.melee_range() * 0.8);
                enemy.state
            }
        }
        AiState::AttackMelee { windup } | AiState::Phase2Attack { windup } => {
            let phase2 = matches!(enemy.state, AiState::Phase2Attack { .. });
            // Track the player during the windup, then commit.
            if windup > 2 {
                enemy.yaw = yaw_toward(enemy.position, player);
            }
            let windup = windup - 1;
            if windup > 0 {
                if phase2 {
                    AiState::Phase2Attack { windup }
                } else {
                    AiState::AttackMelee { windup }
                }
            } else {
                if in_reach {
                    damage = kind.melee_damage() * if phase2 { 1.25 } else { 1.0 };
                }
                enemy.attack_cooldown = cooldown_for(enemy);
                if enemy.phase2_triggered {
                    AiState::Phase2Chase
                } else {
                    AiState::Chase
                }
            }
        }
        AiState::Dead => AiState::Dead,
    };
    damage
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soldier_at(p: Vec3) -> ErEnemy {
        ErEnemy::new(EnemyKind::GodrickSoldier, p, 0.0, 7)
    }

    #[test]
    fn idle_without_player() {
        let mut e = soldier_at(Vec3::new(32.0, 1.0, 32.0));
        for _ in 0..200 {
            assert_eq!(tick_enemy(&mut e, None), 0.0);
        }
        assert!(matches!(e.state, AiState::Idle { .. }));
    }

    #[test]
    fn chases_and_hits_player() {
        let mut e = soldier_at(Vec3::new(32.0, 1.0, 20.0));
        let player = Vec3::new(32.0, 1.0, 30.0);
        let mut total = 0.0;
        for _ in 0..200 {
            total += tick_enemy(&mut e, Some(player));
        }
        assert!(total >= EnemyKind::GodrickSoldier.melee_damage());
        assert!(flat_distance(e.position, player) <= EnemyKind::GodrickSoldier.melee_range());
    }

    #[test]
    fn margit_enters_phase2() {
        let mut m = ErEnemy::new(EnemyKind::Margit, Vec3::new(32.0, 1.0, 32.0), 0.0, 1);
        m.health = m.max_health * 0.5;
        tick_enemy(&mut m, Some(Vec3::new(32.0, 1.0, 40.0)));
        assert!(m.phase2_triggered && m.projectile_unlocked);
        assert!(matches!(
            m.state,
            AiState::Phase2Chase | AiState::Phase2Attack { .. }
        ));
    }
}
