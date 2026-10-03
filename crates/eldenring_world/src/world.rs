//! The server-side ER world: arena geometry plus the enemy roster.
use crate::ai::tick_enemy;
use crate::arena::{arena_scene, default_spawns};
use crate::boss_bar::BossBarState;
use crate::client_view::{EnemySnapshot, ErWorldSnapshot};
use crate::damage::apply_bullet_hit;
use crate::enemy::{EnemyKind, ErEnemy};
use glam::Vec3;
use minecraft_terrain::scene::HandcraftedScene;

/// Offset added to enemy indices to form `sim::voxel` mob keys
/// (`"ER\0\0"` in the high bytes, far above MinecraftOSS entity ids).
pub const ER_MOB_KEY_BASE: u64 = 0x4552_0000_0000_0000;

/// A kill reported by [`ErWorldSim::bullet_hit`], kept for the HUD feed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErKill {
    /// Index of the enemy that died.
    pub enemy_idx: usize,
    /// Its kind.
    pub kind: EnemyKind,
    /// Runes paid out.
    pub runes: u32,
}

/// Elden Ring arena simulation, owned by the server alongside IW4L's sim.
pub struct ErWorldSim {
    /// Arena geometry; hand it to the renderer / collision like the
    /// Minecraft map's `HandcraftedScene`.
    pub arena: HandcraftedScene,
    /// Every enemy spawned this game, living or dead. Indices are stable.
    pub enemies: Vec<ErEnemy>,
    tick_count: u64,
    player_damage_buffer: f32,
    runes_earned: u32,
    kills: Vec<ErKill>,
}

impl Default for ErWorldSim {
    fn default() -> Self {
        Self::new()
    }
}

impl ErWorldSim {
    /// Create a fresh arena with the default enemy spawn set:
    ///   - 6 GodrickSoldiers patrolling
    ///   - 2 GodrickKnights guarding the boss room entrance
    ///   - Margit at center
    pub fn new() -> Self {
        let enemies = default_spawns()
            .into_iter()
            .enumerate()
            .map(|(i, (kind, pos, yaw))| ErEnemy::new(kind, pos, yaw, i as u32 + 1))
            .collect();
        Self {
            arena: arena_scene(),
            enemies,
            tick_count: 0,
            player_damage_buffer: 0.0,
            runes_earned: 0,
            kills: Vec::new(),
        }
    }

    /// Server ticks simulated since [`ErWorldSim::new`].
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Server tick: advance AI, accumulate player damage, check phase transitions.
    /// `player_pos` is the player's feet position, or `None` while dead or
    /// spectating. Damage accumulates until [`ErWorldSim::take_player_damage`].
    pub fn tick(&mut self, player_pos: Option<Vec3>) {
        self.tick_count += 1;
        for enemy in &mut self.enemies {
            self.player_damage_buffer += tick_enemy(enemy, player_pos);
        }
        self.separate_enemies();
    }

    /// Push overlapping living enemies apart so a crowd does not collapse
    /// into one capsule.
    fn separate_enemies(&mut self) {
        let n = self.enemies.len();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = self.enemies.split_at_mut(j);
                let (a, b) = (&mut a[i], &mut b[0]);
                if !a.is_alive() || !b.is_alive() {
                    continue;
                }
                let mut d = b.position - a.position;
                d.y = 0.0;
                let min = a.kind.capsule_radius() + b.kind.capsule_radius();
                let len = d.length();
                if len < min {
                    let dir = if len > 1e-4 { d / len } else { Vec3::X };
                    let push = dir * ((min - len) * 0.5);
                    a.position -= push;
                    b.position += push;
                }
            }
        }
    }

    /// Capsule (bottom_center, top_center, radius) for each living enemy, in index order.
    /// Pass this list to IW4L's entity registration before each bullet trace.
    ///
    /// Dead enemies are skipped, so a position in this list is **not** an
    /// enemy index once anything has died; use [`ErWorldSim::capsule_targets`]
    /// when you need to map a hit back to `bullet_hit`.
    pub fn enemy_capsules(&self) -> Vec<(Vec3, Vec3, f32)> {
        self.enemies
            .iter()
            .filter(|e| e.is_alive())
            .map(ErEnemy::capsule)
            .collect()
    }

    /// Like [`ErWorldSim::enemy_capsules`] but each entry carries the enemy
    /// index to pass to [`ErWorldSim::bullet_hit`].
    pub fn capsule_targets(&self) -> Vec<(usize, Vec3, Vec3, f32)> {
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_alive())
            .map(|(i, e)| {
                let (bottom, top, r) = e.capsule();
                (i, bottom, top, r)
            })
            .collect()
    }

    /// Living enemies as `sim::voxel::set_mob_boxes` wants them: a key and
    /// an axis-aligned box in block space, `[min_x, min_y, min_z, max_x,
    /// max_y, max_z]`. Keys are `ER_MOB_KEY_BASE + enemy_idx`, so they never
    /// collide with MinecraftOSS entity ids; map them back with
    /// [`ErWorldSim::enemy_for_mob_key`].
    pub fn mob_boxes(&self) -> Vec<(u64, [f64; 6])> {
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_alive())
            .map(|(i, e)| {
                let r = f64::from(e.kind.capsule_radius());
                let h = f64::from(e.kind.capsule_half_height() * 2.0);
                let p = e.position.as_dvec3();
                (
                    ER_MOB_KEY_BASE + i as u64,
                    [p.x - r, p.y, p.z - r, p.x + r, p.y + h, p.z + r],
                )
            })
            .collect()
    }

    /// The enemy index behind a key from [`ErWorldSim::mob_boxes`], if it is one.
    pub fn enemy_for_mob_key(&self, key: u64) -> Option<usize> {
        let idx = usize::try_from(key.checked_sub(ER_MOB_KEY_BASE)?).ok()?;
        (idx < self.enemies.len()).then_some(idx)
    }

    /// Ray vs. living enemy capsules, for engines without an entity-capsule
    /// trace hook. Returns `(enemy_idx, fraction)` of the nearest hit along
    /// `start..end`, where fraction is in `0..=1`.
    pub fn trace_enemies(&self, start: Vec3, end: Vec3) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, bottom, top, r) in self.capsule_targets() {
            if let Some(t) = ray_capsule(start, end, bottom, top, r)
                && best.is_none_or(|(_, bt)| t < bt)
            {
                best = Some((i, t));
            }
        }
        best
    }

    /// A MW2 bullet hit enemy at `enemy_idx` for `base_damage` at hit location `hitloc`.
    /// Returns true if the enemy died. Logs rune reward on kill.
    /// Out-of-range indices and already-dead enemies are ignored (false).
    pub fn bullet_hit(&mut self, enemy_idx: usize, base_damage: f32, hitloc: u8) -> bool {
        let Some(enemy) = self.enemies.get_mut(enemy_idx) else {
            return false;
        };
        let killed = apply_bullet_hit(enemy, base_damage, hitloc);
        if killed && !enemy.runes_awarded {
            enemy.runes_awarded = true;
            let runes = enemy.kind.rune_reward();
            self.runes_earned = self.runes_earned.saturating_add(runes);
            self.kills.push(ErKill {
                enemy_idx,
                kind: enemy.kind,
                runes,
            });
            eprintln!(
                "[eldenring_world] {} felled: +{} runes (total {})",
                enemy.kind.name(),
                runes,
                self.runes_earned
            );
        }
        killed
    }

    /// Drain kills since the last call (for a "ENEMY FELLED" / rune popup).
    pub fn take_kills(&mut self) -> Vec<ErKill> {
        std::mem::take(&mut self.kills)
    }

    /// Snapshot for the renderer and HUD this tick.
    pub fn snapshot(&self) -> ErWorldSnapshot {
        let enemies = self
            .enemies
            .iter()
            .enumerate()
            .map(|(index, e)| EnemySnapshot {
                index,
                kind_name: e.kind.name(),
                position: e.position,
                yaw: e.yaw,
                health_fraction: e.health_fraction(),
                hurt_flash: e.hurt_flash > 0,
                dead: !e.is_alive(),
                phase2: e.phase2_triggered,
                radius: e.kind.capsule_radius(),
                height: e.kind.capsule_half_height() * 2.0,
            })
            .collect();
        ErWorldSnapshot {
            enemies,
            boss_bar: self.boss_bar(),
            player_damage_this_tick: self.player_damage_buffer,
            runes_earned: self.runes_earned,
        }
    }

    /// Boss bar state if Margit exists and is alive.
    pub fn boss_bar(&self) -> Option<BossBarState> {
        self.enemies
            .iter()
            .find(|e| e.kind.is_boss() && e.is_alive())
            .map(|e| BossBarState {
                name: e.kind.name(),
                health_fraction: e.health_fraction(),
                phase: if e.phase2_triggered { 2 } else { 1 },
            })
    }

    /// Drain accumulated player damage from enemy melee attacks this tick.
    pub fn take_player_damage(&mut self) -> f32 {
        std::mem::take(&mut self.player_damage_buffer)
    }

    /// Total runes earned (resets only on new game).
    pub fn runes_earned(&self) -> u32 {
        self.runes_earned
    }

    /// Whether every enemy is dead (round won).
    pub fn all_dead(&self) -> bool {
        self.enemies.iter().all(|e| !e.is_alive())
    }
}

/// Segment `start..end` vs capsule; returns the entry fraction in `0..=1`.
fn ray_capsule(start: Vec3, end: Vec3, a: Vec3, b: Vec3, r: f32) -> Option<f32> {
    let seg = end - start;
    let len = seg.length();
    if len <= 1e-6 {
        return None;
    }
    // March-free closed form: closest approach between the ray and the axis
    // segment, then refine by sphere test at the clamped axis point.
    let dir = seg / len;
    let axis = b - a;
    let mut best: Option<f32> = None;
    let mut sphere = |c: Vec3| {
        let m = start - c;
        let bq = m.dot(dir);
        let cq = m.dot(m) - r * r;
        let disc = bq * bq - cq;
        if disc >= 0.0 {
            let t = (-bq - disc.sqrt()).max(0.0);
            if t <= len && best.is_none_or(|bt| t < bt) {
                best = Some(t);
            }
        }
    };
    sphere(a);
    sphere(b);
    // Infinite cylinder around the axis, clipped to the segment.
    let axis_len2 = axis.length_squared();
    if axis_len2 > 1e-8 {
        let m = start - a;
        let md = m.dot(axis);
        let nd = dir.dot(axis);
        let qa = axis_len2 - nd * nd;
        let qb = axis_len2 * m.dot(dir) - md * nd;
        let qc = axis_len2 * (m.dot(m) - r * r) - md * md;
        if qa.abs() > 1e-8 {
            let disc = qb * qb - qa * qc;
            if disc >= 0.0 {
                let t = ((-qb - disc.sqrt()) / qa).max(0.0);
                let s = md + t * nd;
                if t <= len && (0.0..=axis_len2).contains(&s) && best.is_none_or(|bt| t < bt) {
                    best = Some(t);
                }
            }
        }
    }
    best.map(|t| t / len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enemy::AiState;

    #[test]
    fn default_roster() {
        let w = ErWorldSim::new();
        let count = |k| w.enemies.iter().filter(|e| e.kind == k).count();
        assert_eq!(count(EnemyKind::GodrickSoldier), 6);
        assert_eq!(count(EnemyKind::GodrickKnight), 2);
        assert_eq!(count(EnemyKind::Margit), 1);
        assert_eq!(w.enemy_capsules().len(), 9);
        let bar = w.boss_bar().unwrap();
        assert_eq!((bar.phase, bar.health_fraction), (1, 1.0));
    }

    #[test]
    fn kill_margit_pays_runes_and_hides_bar() {
        let mut w = ErWorldSim::new();
        let m = w
            .enemies
            .iter()
            .position(|e| e.kind == EnemyKind::Margit)
            .unwrap();
        assert!(!w.bullet_hit(m, 1300.0, 4));
        assert_eq!(w.boss_bar().unwrap().phase, 2);
        assert!(w.bullet_hit(m, 5000.0, 4));
        assert!(!w.bullet_hit(m, 5000.0, 4));
        assert_eq!(w.runes_earned(), 12_000);
        assert!(w.boss_bar().is_none());
        assert_eq!(w.enemy_capsules().len(), 8);
        assert!(w.capsule_targets().iter().all(|t| t.0 != m));
        assert_eq!(w.take_kills().len(), 1);
        assert!(!w.bullet_hit(999, 10.0, 0));
        let boxes = w.mob_boxes();
        assert_eq!(boxes.len(), 8);
        assert!(
            boxes
                .iter()
                .all(|(k, _)| w.enemy_for_mob_key(*k).is_some_and(|i| i != m))
        );
        assert_eq!(w.enemy_for_mob_key(42), None);
    }

    #[test]
    fn enemies_damage_a_standing_player() {
        let mut w = ErWorldSim::new();
        let player = Some(Vec3::new(32.5, 1.0, 40.0));
        let mut total = 0.0;
        for _ in 0..20 * 30 {
            w.tick(player);
            total += w.take_player_damage();
        }
        assert!(total > 0.0);
        assert!(
            w.enemies
                .iter()
                .any(|e| !matches!(e.state, AiState::Idle { .. }))
        );
    }

    #[test]
    fn trace_hits_nearest_enemy() {
        let w = ErWorldSim::new();
        let m = w
            .enemies
            .iter()
            .position(|e| e.kind == EnemyKind::Margit)
            .unwrap();
        let (idx, t) = w
            .trace_enemies(Vec3::new(32.5, 2.0, 20.0), Vec3::new(32.5, 2.0, 45.0))
            .unwrap();
        assert_eq!(idx, m);
        assert!((t - (32.5 - 1.0 - 20.0) / 25.0).abs() < 1e-3);
        assert!(
            w.trace_enemies(Vec3::new(2.0, 30.0, 2.0), Vec3::new(60.0, 30.0, 2.0))
                .is_none()
        );
    }
}
