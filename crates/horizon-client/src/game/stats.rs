//! Player statistics (`ServerStatsCounter`): the custom counters, blocks
//! mined, items used/crafted/broken/picked up/dropped and mobs killed or
//! killed by, saved to `players/stats/<uuid>.json`, plus the phantom
//! spawner that reads the time since rest (`PhantomSpawner`).
use super::{Dim, Game};
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// `stats[type][key]`, with `type` and `key` namespaced ids.
#[derive(Default)]
pub(super) struct Stats {
    pub(super) values: BTreeMap<String, BTreeMap<String, i32>>,
    pub(super) loaded: bool,
    /// Ticks until the phantom spawner runs again.
    phantom_tick: i32,
    /// Statistic changes (type, key, amount; `None` = reset) for the
    /// scoreboard criteria to follow.
    pub(super) events: Vec<(String, String, Option<i32>)>,
    /// Changed since the client was last sent them (`StatsCounter`'s dirty
    /// set), by type.
    dirty: BTreeMap<String, BTreeSet<String>>,
}

/// `minecraft:` and `name`, given to `f` from the stack when short: some
/// counters go up every tick for every player, and they are found without
/// a string made for each.
fn namespaced<T>(name: &str, f: impl FnOnce(&str) -> T) -> T {
    const NAMESPACE: &str = "minecraft:";
    let mut buf = [0u8; 96];
    let len = NAMESPACE.len() + name.len();
    if len > buf.len() {
        return f(&format!("{NAMESPACE}{name}"));
    }
    buf[..NAMESPACE.len()].copy_from_slice(NAMESPACE.as_bytes());
    buf[NAMESPACE.len()..len].copy_from_slice(name.as_bytes());
    // Two strings one after the other are one.
    f(std::str::from_utf8(&buf[..len]).unwrap_or_default())
}

/// The map under `key`, made when missing, found without an owned key
/// when there.
fn entry<'a, V: Default>(map: &'a mut BTreeMap<String, V>, key: &str) -> &'a mut V {
    if !map.contains_key(key) {
        map.insert(key.to_owned(), V::default());
    }
    map.get_mut(key).expect("just inserted")
}

impl Stats {
    pub(super) fn add(&mut self, ty: &str, key: &str, n: i32) {
        namespaced(ty, |ty| {
            let v = entry(entry(&mut self.values, ty), key);
            *v = v.saturating_add(n);
            self.events.push((ty.to_owned(), key.to_owned(), Some(n)));
            self.mark_dirty(ty, key);
        });
    }
    fn mark_dirty(&mut self, ty: &str, key: &str) {
        let keys = entry(&mut self.dirty, ty);
        if !keys.contains(key) {
            keys.insert(key.to_owned());
        }
    }
    pub(super) fn get(&self, ty: &str, key: &str) -> i32 {
        self.values
            .get(&format!("minecraft:{ty}"))
            .and_then(|m| m.get(key))
            .copied()
            .unwrap_or(0)
    }
    pub(super) fn reset(&mut self, ty: &str, key: &str) {
        if let Some(m) = self.values.get_mut(&format!("minecraft:{ty}")) {
            m.insert(key.to_owned(), 0);
        }
        self.mark_dirty(&format!("minecraft:{ty}"), key);
        self.events
            .push((format!("minecraft:{ty}"), key.to_owned(), None));
    }
    pub(super) fn custom(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.add("custom", key, n));
    }
    /// `ServerStatsCounter.markAllDirty`: everything goes out on the next request.
    pub(super) fn mark_all_dirty(&mut self) {
        self.dirty = self
            .values
            .iter()
            .map(|(ty, m)| (ty.clone(), m.keys().cloned().collect()))
            .collect();
    }
    /// `ServerStatsCounter.sendStats`' entries: the dirty statistics and
    /// their values, the set cleared.
    pub(super) fn take_dirty(&mut self) -> Vec<(String, String, i32)> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .flat_map(|(ty, keys)| keys.into_iter().map(move |key| (ty.clone(), key)))
            .map(|(ty, key)| {
                let v = self.values.get(&ty).and_then(|m| m.get(&key)).copied();
                (ty, key, v.unwrap_or(0))
            })
            .collect()
    }
    /// `ServerStatsCounter.parseLocal`: the counters of a stats file.
    pub(super) fn read(path: &Path) -> Self {
        let mut stats = Self {
            loaded: true,
            ..Self::default()
        };
        let Some(json) = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        else {
            return stats;
        };
        if let Some(all) = json.get("stats").and_then(|s| s.as_object()) {
            for (ty, entries) in all {
                let Some(entries) = entries.as_object() else {
                    continue;
                };
                let map = stats.values.entry(ty.clone()).or_default();
                for (key, v) in entries {
                    if let Some(n) = v.as_i64() {
                        map.insert(key.clone(), n as i32);
                    }
                }
            }
        }
        stats
    }
    /// `ServerStatsCounter.save`.
    pub(super) fn write(&self, path: &Path) {
        let stats: serde_json::Map<String, serde_json::Value> = self
            .values
            .iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(ty, m)| {
                let entries: serde_json::Map<String, serde_json::Value> =
                    m.iter().map(|(k, v)| (k.clone(), (*v).into())).collect();
                (ty.clone(), serde_json::Value::Object(entries))
            })
            .collect();
        let json = serde_json::json!({
            "stats": stats,
            "DataVersion": horizon_world::DATA_VERSION,
        });
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
    }
}
/// `players/<dir>/<uuid>.json` of a world (advancements, stats).
pub(super) fn player_json(world: &Path, dir: &str, uuid: [i32; 4]) -> PathBuf {
    let hex: String = uuid.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    let text = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    world.join("players").join(dir).join(format!("{text}.json"))
}
impl Game<'_> {
    fn stats_path(&self) -> Option<PathBuf> {
        Some(player_json(
            &self.world_root,
            "stats",
            self.level.player_uuid?,
        ))
    }
    /// `ServerStatsCounter.parseLocal`.
    pub(super) fn load_stats(&mut self) {
        if self.stats.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.stats.loaded = true;
        if let Some(path) = self.stats_path() {
            self.stats.values = Stats::read(&path).values;
        }
    }
    /// `ServerStatsCounter.save`.
    pub(super) fn save_stats(&self) {
        if !self.stats.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        if let Some(path) = self.stats_path() {
            self.stats.write(&path);
        }
    }
    /// `ServerPlayer.awardStat`: `n` more of the statistic `ty` (a bare type
    /// such as `used`) and `key`, unless a native mod cancels the award.
    pub(super) fn award_stat(&mut self, ty: &str, key: &str, n: i32) {
        // No mod, no name copied: the per-tick counters come through here.
        let acting = if horizon_client::mod_api::mods().is_empty() {
            None
        } else {
            self.lan_acting_uuid.zip(self.lan_acting_name.clone())
        };
        if let Some((player, name)) = acting {
            let mut cancel = false;
            namespaced(ty, |stat_type| {
                self.native_player_stat_award(player, &name, stat_type, key, n, &mut cancel)
            });
            if cancel {
                return;
            }
        }
        self.stats.add(ty, key, n);
    }
    /// `Player.awardStat(Identifier)`: `n` more of the custom statistic `key`.
    pub(super) fn award_custom_stat(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.award_stat("custom", key, n));
    }
    /// The per-tick counters (`Player.tick`/`ServerPlayer.tick` awardStat).
    pub(super) fn tick_stats(&mut self) {
        self.load_stats();
        self.count_tick_stats(self.sleeping.is_some());
        self.tick_movement_stats();
    }
    /// `Player.tick`'s counters for the player in `self.player`.
    pub(super) fn count_tick_stats(&mut self, sleeping: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let alive = !p.health.dead_or_dying();
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        self.award_custom_stat("play_time", 1);
        self.award_custom_stat("total_world_time", 1);
        if alive {
            self.award_custom_stat("time_since_death", 1);
            if !sleeping {
                self.award_custom_stat("time_since_rest", 1);
            }
        }
        if crouching {
            self.award_custom_stat("sneak_time", 1);
        }
    }
    fn tick_movement_stats(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let d = p.position.subtract(self.previous_position);
        self.movement_stats(d);
    }
    /// `Player.checkMovementStatistics` / `checkRidingStatistics` for one move.
    pub(super) fn movement_stats(&mut self, d: Vec3) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let horizontal = ((d.x * d.x + d.z * d.z).sqrt() * 100.0).round() as i32;
        let total = ((d.x * d.x + d.y * d.y + d.z * d.z).sqrt() * 100.0).round() as i32;
        // Teleports are not travel.
        if total <= 0 || total > 1000 {
            return;
        }
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let eye_in_water = self.res.facts().get(self.world.block(
            feet[0],
            (p.position.y + f64::from(p.dimensions.eye_height)).floor() as i32,
            feet[2],
        ));
        let in_water = self
            .res
            .facts()
            .get(self.world.block(feet[0], feet[1], feet[2]))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let on_ground = p.ground.on_ground;
        let (sprinting, gliding, swimming) = (
            p.sprinting,
            p.fall_flying,
            p.pose == horizon_client::pose::Pose::Swimming,
        );
        let climbing = p.last_climbable.is_some() && d.y > 0.0;
        let riding = self.riding.map(|id| {
            self.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species())
                .ok_or(())
        });
        match riding {
            Some(Ok(sp)) => {
                use horizon_client::mobs::Species;
                // `checkRidingStatistics`: only these mounts count, the horse
                // family by the full 3D distance; any other mount nothing.
                match sp {
                    Species::Pig => self.award_custom_stat("pig_one_cm", total),
                    Species::Strider => self.award_custom_stat("strider_one_cm", total),
                    Species::HappyGhast => self.award_custom_stat("happy_ghast_one_cm", total),
                    Species::Nautilus | Species::ZombieNautilus => {
                        self.award_custom_stat("nautilus_one_cm", total)
                    }
                    sp if horizon_client::mobs::equine::is_equine(sp) => {
                        self.award_custom_stat("horse_one_cm", total)
                    }
                    _ => {}
                }
            }
            Some(Err(())) => {
                if self.boats.iter().any(|b| Some(b.id) == self.riding) {
                    self.award_custom_stat("boat_one_cm", total)
                } else if self.carts.iter().any(|c| Some(c.id) == self.riding) {
                    self.award_custom_stat("minecart_one_cm", total)
                }
            }
            None if swimming => self.award_custom_stat("swim_one_cm", total),
            None if eye_in_water.fluid == horizon_client::blocks::Fluid::Water => {
                self.award_custom_stat("walk_under_water_one_cm", total)
            }
            None if in_water => {
                if horizontal > 0 {
                    self.award_custom_stat("walk_on_water_one_cm", horizontal)
                }
            }
            None if climbing => {
                self.award_custom_stat("climb_one_cm", (d.y * 100.0).round() as i32)
            }
            None if on_ground => {
                if horizontal > 0 {
                    if sprinting {
                        self.award_custom_stat("sprint_one_cm", horizontal)
                    } else if crouching {
                        self.award_custom_stat("crouch_one_cm", horizontal)
                    } else {
                        self.award_custom_stat("walk_one_cm", horizontal)
                    }
                }
            }
            None if gliding => self.award_custom_stat("aviate_one_cm", total),
            // Airborne and not gliding: more than 25 cm sideways per move
            // (creative flight, or a long jump), not only flying.
            None if horizontal > 25 => self.award_custom_stat("fly_one_cm", horizontal),
            None => {}
        }
    }
    /// `PhantomSpawner.tick`: tired players under an open night sky draw
    /// phantoms. A custom spawner ticks only while `spawn_mobs` is on
    /// (`ServerChunkCache.tickChunks`), and this one only spawns enemies
    /// (`ServerLevel.isSpawningMonsters`: `spawn_monsters` too). Every
    /// player of the level counts: the game's own and the hosted ones (a
    /// server thread's players are all hosted).
    pub(super) fn spawn_phantoms(&mut self) {
        let rules = &self.ui.rules;
        if !rules.bool("spawn_mobs")
            || !rules.bool("spawn_monsters")
            || !rules.bool("spawn_phantoms")
            || self.dim != Dim::Overworld
        {
            return;
        }
        self.stats.phantom_tick -= 1;
        if self.stats.phantom_tick > 0 {
            return;
        }
        let r = &mut self.world.random;
        self.stats.phantom_tick += (60 + r.next_int_bound(60).unwrap_or(0)) * 20;
        if self.sky_darken() < 5 {
            return;
        }
        let rest = |s: &Stats| s.get("custom", "minecraft:time_since_rest");
        // `level.players()`: (block position, time since rest) of each player
        // who isn't a spectator.
        let mut players = Vec::new();
        if let Some(p) = self.player.as_ref()
            && !self.spectator()
        {
            players.push((p.position, rest(&self.stats)));
        }
        if let Some(host) = self.lan.as_ref() {
            let level = self.dim.id();
            for g in host.hub.guests.iter().filter(|g| {
                g.placed
                    && g.dimension == level
                    && g.game_mode != horizon_client::lan::guest::SPECTATOR
            }) {
                let since = host.play.get(&g.conn).map_or(0, |p| rest(&p.stats));
                players.push((
                    Vec3::new(g.position[0], g.position[1], g.position[2]),
                    since,
                ));
            }
        }
        for (position, since) in players {
            self.spawn_phantoms_near(position, since);
        }
    }
    /// One player's turn of `PhantomSpawner.tick`.
    fn spawn_phantoms_near(&mut self, position: Vec3, since_rest: i32) {
        let pos = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        if pos[1] < 63 || !self.can_see_sky(pos) {
            return;
        }
        // `getCurrentDifficultyAt(playerPos).isHarderThan(random * 3)`.
        let difficulty = self.local_difficulty([position.x, position.y, position.z]);
        let r = &mut self.world.random;
        if difficulty.effective <= r.next_float() * 3.0 {
            return;
        }
        if r.next_int_bound(since_rest.max(1)).unwrap_or(0) < 72000 {
            return;
        }
        // `playerPos.above(20 + nextInt(15)).east(-10 + nextInt(21)).south(...)`:
        // the height is drawn first.
        let up = 20 + r.next_int_bound(15).unwrap_or(0);
        let east = -10 + r.next_int_bound(21).unwrap_or(10);
        let south = -10 + r.next_int_bound(21).unwrap_or(10);
        let at = [pos[0] + east, pos[1] + up, pos[2] + south];
        if !self
            .res
            .facts()
            .is_air(self.world.block(at[0], at[1], at[2]))
        {
            return;
        }
        let group = 1 + self.world.random.next_int_bound(3).unwrap_or(0);
        for _ in 0..group {
            let _ = self.spawn_mob(
                "minecraft:phantom",
                [
                    f64::from(at[0]) + 0.5,
                    f64::from(at[1]),
                    f64::from(at[2]) + 0.5,
                ],
                false,
            );
        }
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> bool {
        (pos[1] + 1..pos[1] + 256).all(|y| {
            let f = self.res.facts().get(self.world.block(pos[0], y, pos[2]));
            !f.full_collision && f.fluid == horizon_client::blocks::Fluid::None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_events_and_dirty_entries() {
        let mut s = Stats::default();
        s.custom("play_time", 1);
        s.custom("play_time", 1);
        s.add("mined", "minecraft:stone", 2);
        s.custom(&"x".repeat(100), 3);
        assert_eq!(s.get("custom", "minecraft:play_time"), 2);
        assert_eq!(s.get("mined", "minecraft:stone"), 2);
        assert_eq!(
            s.get("custom", &format!("minecraft:{}", "x".repeat(100))),
            3
        );
        assert_eq!(s.events.len(), 4);
        assert_eq!(
            s.events[1],
            (
                "minecraft:custom".into(),
                "minecraft:play_time".into(),
                Some(1)
            )
        );
        let long = format!("minecraft:{}", "x".repeat(100));
        let sent = |v: &[(&str, &str, i32)]| -> Vec<(String, String, i32)> {
            v.iter()
                .map(|&(t, k, n)| (t.to_owned(), k.to_owned(), n))
                .collect()
        };
        assert_eq!(
            s.take_dirty(),
            sent(&[
                ("minecraft:custom", "minecraft:play_time", 2),
                ("minecraft:custom", &long, 3),
                ("minecraft:mined", "minecraft:stone", 2),
            ])
        );
        assert!(s.take_dirty().is_empty());
        s.reset("mined", "minecraft:stone");
        assert_eq!(
            s.take_dirty(),
            sent(&[("minecraft:mined", "minecraft:stone", 0)])
        );
        s.mark_all_dirty();
        assert_eq!(s.take_dirty().len(), 3);
    }
}
