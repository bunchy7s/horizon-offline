//! Mobs for native server mods (docs/native-mods.md, "Mobs", API 37): spawn one,
//! read the mobs of the level in place, and move, hurt or remove one by its id.
//! They act where the world is simulated, never on a client's copy, and only on
//! the level in place, which each names by its `dimension` (`minecraft:overworld`)
//! as `native_set_block` does: another one is refused. While the game ticks its
//! mobs they are out of the level: a mob a hook spawns then joins it after the
//! tick, and the others read as gone.
use super::{Game, commands::Target};
use horizon_core::geometry::Vec3;

/// A mob as a native mod reads it.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeMob {
    pub(crate) id: u32,
    /// The level it is in (`minecraft:overworld`).
    pub(crate) dimension: String,
    /// `minecraft:zombie`.
    pub(crate) kind: String,
    pub(crate) pos: [f64; 3],
    /// Degrees; 0 faces south.
    pub(crate) yaw: f32,
    pub(crate) health: f32,
    pub(crate) max_health: f32,
}

/// What a mob starts with (`native_spawn_mob`); the default is a plain mob.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeSpawn {
    /// A name over it, shown to everyone.
    pub(crate) name: Option<String>,
    /// Cut to its maximum.
    pub(crate) health: Option<f32>,
    pub(crate) no_ai: bool,
    pub(crate) invisible: bool,
    pub(crate) silent: bool,
    pub(crate) invulnerable: bool,
    pub(crate) no_gravity: bool,
}

fn finite(v: [f64; 3]) -> bool {
    v.into_iter().all(f64::is_finite)
}

/// `/summon`'s world bounds (`Level.isInSpawnableBounds`).
fn spawnable(pos: [f64; 3]) -> bool {
    finite(pos)
        && (-30_000_000.0..30_000_000.0).contains(&pos[0])
        && (-30_000_000.0..30_000_000.0).contains(&pos[2])
        && (-20_000_000.0..20_000_000.0).contains(&pos[1].floor())
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Whether this is a level that simulates (not a client's copy) and the one
    /// in place is `dimension`.
    fn native_mob_level(&self, dimension: &str) -> bool {
        self.remote.is_none() && dimension == self.dim.id()
    }
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos` in the level in
    /// place, `dimension`, finalized as a natural spawn is, and kept when the
    /// chunk unloads: its id. None for a kind the game doesn't know, another
    /// dimension, a position outside `/summon`'s bounds or in a chunk that isn't
    /// loaded (it would never tick or be saved), or a monster on peaceful (it
    /// would despawn at once). API 37.
    pub(super) fn native_spawn_mob(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        options: &NativeSpawn,
    ) -> Option<u32> {
        if !self.native_mob_level(dimension) || !spawnable(pos) || !self.native_chunk_loaded(pos) {
            return None;
        }
        let qualified = if kind.contains(':') {
            kind.to_owned()
        } else {
            ["minecraft:", kind].concat()
        };
        let monster = self
            .res
            .mob_types()
            .get(&qualified)
            .is_some_and(|t| t.category == "monster");
        if monster && self.ui.difficulty == 0 {
            return None;
        }
        let id = self.spawn_mob(kind, pos, true)?;
        // A jockey's other mob comes after it: this one is found by its id.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.no_ai = options.no_ai;
            m.invisible = options.invisible;
            m.silent = options.silent;
            m.invulnerable_flag = options.invulnerable;
            m.no_gravity = options.no_gravity;
            if let Some(name) = &options.name {
                m.custom_name = Some(name.clone());
                m.name_visible = true;
            }
            if let Some(health) = options.health.filter(|h| h.is_finite() && *h > 0.0) {
                m.health = health.min(m.attrs.max_health);
            }
        }
        Some(id)
    }
    /// Whether the chunk holding `pos` is loaded in the level in place.
    fn native_chunk_loaded(&self, pos: [f64; 3]) -> bool {
        let (x, z) = (pos[0].floor() as i32, pos[2].floor() as i32);
        self.world.chunks.contains_key(&(x >> 4, z >> 4))
    }
    /// The living mob `id` in the level in place, `dimension`; none on a
    /// client's copy. API 37.
    pub(super) fn native_mob(&self, dimension: &str, id: u32) -> Option<NativeMob> {
        if !self.native_mob_level(dimension) {
            return None;
        }
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        mob.alive().then(|| self.native_mob_of(mob))
    }
    /// The living mobs in the level in place, `dimension`; none on a client's
    /// copy. API 37.
    pub(super) fn native_mobs(&self, dimension: &str) -> Vec<NativeMob> {
        if !self.native_mob_level(dimension) {
            return Vec::new();
        }
        self.mobs
            .iter()
            .filter(|m| !m.removed && m.alive())
            .map(|m| self.native_mob_of(m))
            .collect()
    }
    /// Moves mob `id` to `pos` at once, as `/tp` does: it dismounts, drops its
    /// path and resets its fall. False for no such mob, another dimension or a
    /// position outside `/summon`'s bounds. API 37.
    pub(super) fn native_teleport_mob(&mut self, dimension: &str, id: u32, pos: [f64; 3]) -> bool {
        if !self.native_mob_level(dimension) || !spawnable(pos) {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        let to = Vec3::new(pos[0], pos[1], pos[2]);
        // `TeleportCommand`: stop the navigation, no vertical motion, and a
        // rider is put back on its vehicle unless it gets off first.
        m.ai.nav.stop();
        m.velocity.y = 0.0;
        m.extra.mount = None;
        m.extra.passenger = false;
        m.position = to;
        m.previous = to;
        m.fall_distance = 0.0;
        m.needs_sync = true;
        true
    }
    /// Adds `velocity` to mob `id`'s: false for no such mob, another dimension
    /// or a velocity that isn't finite. API 37.
    pub(super) fn native_push_mob(&mut self, dimension: &str, id: u32, velocity: [f64; 3]) -> bool {
        if !self.native_mob_level(dimension) || !finite(velocity) {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        m.velocity = Vec3::new(
            m.velocity.x + velocity[0],
            m.velocity.y + velocity[1],
            m.velocity.z + velocity[2],
        );
        m.needs_sync = true;
        true
    }
    /// Sets the living mob `id`'s health, cut to its maximum: false for no such
    /// mob, another dimension or a health that isn't above 0 (`native_kill_mob`
    /// kills). API 37.
    pub(super) fn native_set_mob_health(&mut self, dimension: &str, id: u32, health: f32) -> bool {
        if !self.native_mob_level(dimension) || !health.is_finite() || health <= 0.0 {
            return false;
        }
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed && m.alive())
        else {
            return false;
        };
        m.health = health.min(m.attrs.max_health);
        m.needs_sync = true;
        true
    }
    /// Kills mob `id` as `/kill` does: it falls and drops its loot. False for
    /// no such living mob or another dimension. API 37.
    pub(super) fn native_kill_mob(&mut self, dimension: &str, id: u32) -> bool {
        if self.native_mob(dimension, id).is_none() {
            return false;
        }
        self.kill_target(Target::Mob(id));
        true
    }
    /// Takes mob `id` out of the world with no death and no loot: false for no
    /// such mob or another dimension. API 37.
    pub(super) fn native_remove_mob(&mut self, dimension: &str, id: u32) -> bool {
        if !self.native_mob_level(dimension) {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        m.removed = true;
        true
    }
    fn native_mob_of(&self, m: &horizon_client::mobs::Mob) -> NativeMob {
        NativeMob {
            id: m.id,
            dimension: self.dim.id().to_owned(),
            kind: horizon_client::command::args::normalize(&m.kind.id),
            pos: [m.position.x, m.position.y, m.position.z],
            yaw: m.yaw,
            health: m.health,
            max_health: m.attrs.max_health,
        }
    }
}
