# Elden Ring layer (`eldenring_world`) — integration guide

`crates/eldenring_world` is a self-contained simulation of a Stormveil
courtyard arena with Elden Ring enemies (six Godrick Soldiers, two Godrick
Knights, and Margit, the Fell Omen) that MW2 guns can shoot and that fight
back in melee. It follows the same layout as the Minecraft map: the arena is a
`minecraft_terrain::scene::HandcraftedScene` made of blocks, so the existing
mesher, lighting and voxel collision can draw and collide with it unchanged.
The enemies use the same `sim::voxel` mob-box and player-damage channels the
Minecraft mobs already use.

**Status:** the crate builds and its unit tests pass (`cargo test -p eldenring_world`).
**None of the game-loop wiring below is in the tree yet.** This document says
where each hook goes; the code snippets are a guide and have not been compiled
inside `sim` / `render_anim` / `assets`.

---

## 1. What the crate provides

| Item | Purpose |
|------|---------|
| `ErWorldSim` | Owns the arena (`arena: HandcraftedScene`) and the `enemies: Vec<ErEnemy>`. Indices are stable for the whole game; dead enemies stay in the list as corpses. |
| `ErWorldSim::new()` | Builds the arena and spawns the default roster. |
| `ErWorldSim::tick(Option<Vec3>)` | One 20 Hz server tick. Pass the player's feet position in **block space**, or `None` while dead or spectating. |
| `ErWorldSim::take_player_damage() -> f32` | Drains melee damage the enemies dealt since the last call. |
| `ErWorldSim::bullet_hit(idx, damage, hitloc) -> bool` | Applies a MW2 bullet. Returns `true` on a kill, pays out runes once. |
| `ErWorldSim::enemy_capsules()` | `(bottom, top, radius)` for each **living** enemy, in index order. Dead enemies are skipped, so a position in this list is not an enemy index once something has died. |
| `ErWorldSim::capsule_targets()` | The same, but each entry carries its enemy index: `(idx, bottom, top, radius)`. Use this one when a hit has to map back to `bullet_hit`. |
| `ErWorldSim::mob_boxes()` | Living enemies as `(key, [minx,miny,minz,maxx,maxy,maxz])` in block space. This is what `sim::voxel::set_mob_boxes` takes. |
| `ErWorldSim::enemy_for_mob_key(key)` | Turns a `mob_boxes` key back into an enemy index. Keys are `ER_MOB_KEY_BASE + idx`. |
| `ErWorldSim::trace_enemies(start, end)` | Built-in segment-vs-capsule test, for a path that has no entity trace hook. |
| `ErWorldSim::snapshot() -> ErWorldSnapshot` | Per-tick data for the renderer and HUD: enemies, boss bar, pending damage, runes. |
| `ErWorldSim::boss_bar() -> Option<BossBarState>` | `Some` while Margit is alive, with `phase` 1 or 2. |
| `ErWorldSim::take_kills()` | Drains `ErKill { enemy_idx, kind, runes }` for a kill feed or a rune pop-up. |
| `arena::{arena_scene, player_spawn, default_spawns, ARENA_MAP_NAME}` | Arena builder, spawn point, roster, and the map name `"stormveil_arena"`. |
| `tick_enemy`, `apply_bullet_hit`, `hitloc_scale` | Lower-level per-enemy functions. |

### Coordinates
All positions are in Minecraft **block space**: 1 block = 1 m, Y up, and
north = −Z. The arena covers x, z ∈ `0..64`, and the floor top is at Y = 1.
To convert from IW4 map units, use the same helpers the Minecraft map uses:
`sim::voxel::to_block(origin, p)` and `sim::voxel::to_map(origin, b)`. One
block is `sim::voxel::BLOCK` = 36 map units.

### Stat table
| Enemy | HP | Melee | Reach | Capsule r / height | Speed | Runes |
|-------|----|-------|-------|--------------------|-------|-------|
| Godrick Soldier | 150 | 15 | 1.8 m | 0.6 / 1.8 m | 3.5 m/s | 220 |
| Godrick Knight | 400 | 25 | 2.2 m | 0.7 / 2.0 m | 3.0 m/s | 600 |
| Margit | 2400 | 40 (×1.25 in phase 2) | 3.5 m | 1.0 / 2.8 m | 4.5 m/s (×1.3 in phase 2) | 12 000 |

Margit switches to phase 2 at ≤ 50 % HP. In phase 2 her melee cooldown is
halved, her windup drops from 10 to 6 ticks, and `projectile_unlocked` is set.
The projectile attack itself is a flag only; it is not simulated yet.

### Hit locations
`hitloc` uses IW4's real indices from `weapon_iw4::HITLOC_NAMES`:
1–3 (helmet, head, neck) ×1.5, 4–5 (torso) ×1.0, 6–17 (limbs) ×0.75,
18–19 (gun, shield) ×0. Index 0, and any out-of-range index, counts as ×1.0.
(The original task described "head = 1 / chest = 2". The crate follows the
engine's actual table instead, where 2 is head and 4 is upper torso.)

---

## 2. Registering enemy hitboxes before bullet traces

The Minecraft map already has the mechanism. In
`crates/render_anim/src/minecraft_world.rs`, inside `update`, the client calls
`sim::voxel::set_mob_boxes(entities.boxes())` every frame. Then in
`crates/sim/src/combat.rs`, around line 1241, each bullet asks
`crate::voxel::mob_on_segment(start, first_surface)` for the nearest box.

For the ER map, publish the ER boxes into that same slot every frame. Do it
before the authoritative sim runs its next weapon tick:

```rust
// wherever the ER world is owned (see section 4), each frame / tick:
sim::voxel::set_mob_boxes(er.mob_boxes());
// To keep Minecraft mobs too (not needed on stormveil_arena), concatenate:
// let mut boxes = entities.boxes(); boxes.extend(er.mob_boxes()); sim::voxel::set_mob_boxes(boxes);
```

`mob_on_segment` only runs while `sim::voxel::active()`. So the ER map must
activate the voxel world with the arena's blocks, the same way
`minecraft_world.rs` does for streamed chunks (`sim::voxel::activate`,
`sim::voxel::set_chunk` / `set_block_shape`). If you feed it
`er.arena` the same way the Minecraft scene is fed, world traces, player
movement and bullets will all collide with the walls, columns and rubble.

If you go through a path that registers entity **capsules** with
`trace_iw4` instead, register `er.capsule_targets()` and keep the returned
`idx` as the entity's hit id. When
`trace_iw4::trace_get_entity_hit_id(hit.hit_type, hit.hit_id)` returns one of
those ids, pass it to `bullet_hit`.

## 3. Applying bullet hits

`combat.rs` already converts a mob strike into damage:
`bullet_damage_at_distance(&facts, dist) * facts.location_scale(mob_hitloc(up))`.
It then queues `VoxelEvent::MobShot { key, damage, from }` through
`push_mob_shot`. Because that damage is **already location-scaled**, pass
`hitloc = 0` so it is not scaled twice:

```rust
// render_anim/src/minecraft_world.rs, where MobShot events are drained (~line 686),
// or a new er_world.rs system that drains sim::voxel::take_events():
for event in sim::voxel::take_events() {
    if let sim::voxel::VoxelEvent::MobShot { key, damage, .. } = event {
        if let Some(idx) = er.enemy_for_mob_key(key) {
            let killed = er.bullet_hit(idx, damage, 0);
            if killed { /* play death sound, kill-feed entry */ }
            continue;
        }
        // otherwise: a Minecraft mob, existing path (entities.shoot(...))
    }
}
for kill in er.take_kills() { /* "+{kill.runes} runes" popup */ }
```

`take_events()` drains a global queue. If both the Minecraft handler and the
ER handler are active, drain it in **one** place and route each event by key.

If your path still has the raw, unscaled damage and the IW4 hitloc (for
example, a capsule trace path), call `er.bullet_hit(idx, raw_damage, hitloc)`
and let the crate apply its own ×1.5 / ×1.0 / ×0.75 table.

## 4. Ticking from the server loop

Own one `ErWorldSim` next to the Minecraft runtime. The minimal approach is a
`Option<ErWorldSim>` resource in a new `render_anim/src/er_world.rs`,
registered like `minecraft_world::register(app)`. Create it when the loaded
zone is the ER map (section 7), and drop it on unload.

Each 20 Hz server tick:

```rust
let player_block = alive.then(|| {
    let b = sim::voxel::to_block(origin, player_origin_map_units);
    glam::Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32)
});
er.tick(player_block);

let dmg = er.take_player_damage();
if dmg > 0.0 {
    // The same channel Minecraft mobs use (minecraft_world.rs ~line 847).
    // sim::damage::apply_block_world_damage turns it into Melee damage on the player.
    sim::voxel::push_player_damage(local_client, dmg.round() as i32, None);
}
sim::voxel::set_mob_boxes(er.mob_boxes());
```

The Minecraft runtime advances mobs with a `mob_ticks` count each frame
(`entities.tick_scene(&world.scene, mob_ticks)`). Use that same count: call
`er.tick` `mob_ticks` times so the ER AI stays in step with the sim's
20 Hz clock regardless of frame rate. The AI is deterministic (seeded
xorshift per enemy), so replays and killcams that re-run ticks will match.

## 5. Rendering

- **Arena:** `er.arena` is a normal `HandcraftedScene`. Hand it to the same
  section mesher / `MinecraftWorldView` path that renders `world.scene` in
  `minecraft_world.rs`. It only uses vanilla blocks (`stone_bricks`,
  `mossy_stone_bricks`, `cracked_stone_bricks`, `chiseled_stone_bricks`,
  `glass`), so the existing resource pack covers it.
- **Enemies:** `er.snapshot().enemies` gives, for each enemy: `position`, `yaw`,
  `radius`, `height`, `hurt_flash`, `dead`, `phase2`, and `kind_name`. The
  quickest placeholder reuses an existing mob model (for example
  `zombie_render` / `skeleton_render` in `minecraft_terrain`), scaled by
  `height / 1.95`. Build it the same way `client_mobs` meshes are built in
  `entities.meshes(...)`:
  - tint red while `hurt_flash`
  - turn gold while `phase2`
  - lie down or fade when `dead`
  - convert `yaw` with the same convention used for MinecraftOSS mobs
    (crate yaw 0 = +Z, positive toward +X)

## 6. Boss health bar

```rust
if let Some(bar) = er.boss_bar() {
    // Bottom-centre, Elden Ring style: name above a wide thin bar.
    // bar.name  -> "Margit, the Fell Omen"
    // bar.health_fraction in 0..=1
    // bar.phase == 2 -> optionally flash or change colour
}
```

Draw it as a `bevy_ui` node or as two filled rects through the existing HUD
path (`crates/hud`, `crates/hud_iw4/src/hudelem.rs`). `boss_bar()` returns
`None` once Margit dies; use that moment for an "ENEMY FELLED" banner. The
rune total is `er.runes_earned()`.

## 7. Map picker: "Elden Ring" tab → `stormveil_arena`

The Minecraft map is added in `crates/assets/src/minecraft_map.rs`. It is
listed under its own `MapPack` (label `"Minecraft"`) and stands in for the
proxy IW4 zone `iw4:mp_rust`, which provides weapons, bodies and menus. Mirror
that pattern:

```rust
// crates/assets/src/eldenring_map.rs (new)
pub const ZONE: &str = "eldenring:stormveil_arena";
const PROXY_ZONE: &str = "iw4:mp_rust";
pub fn is_eldenring(zone: &str) -> bool { zone.eq_ignore_ascii_case(ZONE) }
pub fn is_eldenring_load(zone: &str) -> bool {
    is_eldenring(zone) || zone.split_once(':').is_some_and(|(_, r)| is_eldenring(r))
}
pub fn find_zone_file(root: &GamesRoot, zone: &str) -> Result<ZoneFile, String> {
    let mut proxy = asset_transport::find_zone_file(root, PROXY_ZONE)?;
    proxy.zone_name = ZONE.to_owned();
    proxy.alias_note = Some("Elden Ring Stormveil arena".to_owned());
    Ok(proxy)
}
```

Then, in `minecraft_map::list_mp_map_packs` / `list_mp_maps` (or their
caller), push:

```rust
packs.push(asset_transport::MapPack { label: "Elden Ring".to_owned(), maps: vec![eldenring_map::ZONE.to_owned()] });
```

Every call site that branches on `is_minecraft` / `is_minecraft_load` also
needs an `is_eldenring` branch. Those are:

- `render_anim/src/minecraft_world.rs`
- `render_frontend/src/prepare/scene/spawn.rs`
- `session/src/match_apply.rs`
- `audio/src/ambient.rs`
- `audio/src/script_ambient.rs`

On that branch, build `ErWorldSim::new()` instead of loading a MinecraftOSS
world, and spawn the player at `eldenring_world::player_spawn()`, which is
just inside the fog gate facing south.

The MW2 assets themselves are still read from your Steam MW2 install, as
they are for every other map.

## 8. Build steps

```powershell
git pull
cargo test  -p eldenring_world            # crate only: arena, AI, damage, boss bar
cargo build -p launcher --release          # whole game, once the wiring above is added
```

1. Rust stable (edition 2024) is needed. The repo's `rust-toolchain.toml`
   selects it.
2. `eldenring_world` depends only on `glam` (workspace) and
   `minecraft_terrain` (path), so it builds wherever the Minecraft map builds.
3. Add the hooks from sections 2–7, in this order:
   - map picker entry (7)
   - `ErWorldSim` resource and tick (4)
   - mob boxes (2)
   - MobShot routing (3)
   - arena and enemy rendering (5)
   - boss bar (6)
4. Launch, pick **Elden Ring → stormveil_arena**, walk through the fog gate,
   and shoot.
