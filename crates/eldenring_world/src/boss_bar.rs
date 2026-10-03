//! Boss health bar state for the HUD.

/// What the HUD needs to draw the Elden Ring style boss bar at the bottom of
/// the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossBarState {
    /// Boss display name, drawn above the bar.
    pub name: &'static str,
    /// Remaining health, `0.0..=1.0`.
    pub health_fraction: f32,
    /// 1 or 2.
    pub phase: u8,
}
