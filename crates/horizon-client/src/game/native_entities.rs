//! Entities for native server mods (docs/native-mods.md, "Entities"): spawn
//! any entity from its compound as `/summon` does, and move, show or remove
//! the entities Horizon keeps without simulating them (displays, markers,
//! interactions), which is what a mod needs for models that move: an
//! `item_display` showing an item model from the mod's assets. They act where
//! the world is simulated, in the level in place, which each names by its
//! `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{
    Game,
    commands::{Target, place_summoned},
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::{Compound, Tag};

/// A display's transformation and interpolation (`Display`'s
/// `transformation`, `interpolation_duration` and `teleport_duration`):
/// a translation, a left rotation (a quaternion `[x, y, z, w]`), a scale and a
/// right rotation, eased over `interpolation` ticks from what it showed;
/// `teleport` ticks ease a move (0 to 59).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NativeDisplay {
    pub translation: [f32; 3],
    pub left_rotation: [f32; 4],
    pub scale: [f32; 3],
    pub right_rotation: [f32; 4],
    pub interpolation: i32,
    pub teleport: i32,
}

impl Default for NativeDisplay {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            left_rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            right_rotation: [0.0, 0.0, 0.0, 1.0],
            interpolation: 0,
            teleport: 0,
        }
    }
}

impl NativeDisplay {
    /// Whether every value is finite, the quaternions aren't zero and the
    /// durations are in range.
    fn valid(&self) -> bool {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let unit = |q: &[f32; 4]| q.iter().map(|x| x * x).sum::<f32>() > 1.0e-12;
        finite(&self.translation)
            && finite(&self.left_rotation)
            && finite(&self.scale)
            && finite(&self.right_rotation)
            && unit(&self.left_rotation)
            && unit(&self.right_rotation)
            && (0..=1_000_000).contains(&self.interpolation)
            && (0..=59).contains(&self.teleport)
    }
    /// Writes its keys into a display's compound. `start` is the
    /// interpolation's start delay: entity data reaches clients only when it
    /// changes (as vanilla's does unless forced), and a display restarts its
    /// interpolation only when the start comes, so each write flips it
    /// between 0 and -1.
    fn write(&self, tag: &mut Compound) {
        let floats = |v: &[f32]| Tag::List(v.iter().map(|x| Tag::Float(x.to_bits())).collect());
        let mut t = Compound::new();
        t.insert("translation".into(), floats(&self.translation));
        t.insert("left_rotation".into(), floats(&self.left_rotation));
        t.insert("scale".into(), floats(&self.scale));
        t.insert("right_rotation".into(), floats(&self.right_rotation));
        tag.insert("transformation".into(), Tag::Compound(t));
        tag.insert(
            "interpolation_duration".into(),
            Tag::Int(self.interpolation),
        );
        tag.insert("teleport_duration".into(), Tag::Int(self.teleport));
        let start = match tag.get(&"start_interpolation".into()) {
            Some(Tag::Int(0)) => -1,
            _ => 0,
        };
        tag.insert("start_interpolation".into(), Tag::Int(start));
    }
}

/// Whether a position is one `/summon` takes.
fn position_ok(pos: [f64; 3]) -> bool {
    pos.iter().all(|v| v.is_finite())
        && pos[0].abs() < 30_000_000.0
        && pos[2].abs() < 30_000_000.0
        && (-20_000_000.0..20_000_000.0).contains(&pos[1])
}

/// The id of a summoned stack's root: its command target's, else `first`,
/// the first id handed out, when an entity no command target stands for
/// (`live`) holds it.
fn root_id(target: Option<Target>, first: u32, live: impl Fn(u32) -> bool) -> Option<u32> {
    match target {
        Some(t) => target_id(t),
        None => live(first).then_some(first),
    }
}

/// The entity id a summoned stack's root is known by.
fn target_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Item(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Crystal(id)
        | Target::Dragon(id) => Some(id),
        Target::Player | Target::Guest(_) => None,
    }
}

#[allow(dead_code)]
impl Game<'_> {
    /// Whether this game simulates the level `dimension`.
    fn native_entity_level(&self, dimension: &str) -> bool {
        (self.serving.is_some() || self.options.dedicated.is_some()) && dimension == self.dim.id()
    }
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// level in place, `dimension`, from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its id. None on a
    /// client's copy of the world, for another dimension, a position outside
    /// `/summon`'s bounds, a chunk that isn't loaded, a kind Horizon can't
    /// load, or one a mod kept out (`native_entity_join_level`). API
    /// 59.
    pub(super) fn native_summon(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        nbt: Compound,
    ) -> Option<u32> {
        if !self.native_entity_level(dimension) || !position_ok(pos) {
            return None;
        }
        let chunk = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
        if !self.world.chunks.contains_key(&chunk) {
            return None;
        }
        let mut tag = nbt;
        tag.insert("id".into(), Tag::String(kind.into()));
        // As `/summon` does: every entity of the stack, passengers included,
        // stands at `pos`.
        let mut tag = Tag::Compound(tag);
        place_summoned(&mut tag, pos);
        let from = self.mobs.len();
        let first = self.next_entity + 1;
        let deferred = (self.tnts.len(), self.falling.len(), self.rockets.len());
        let target = self.spawn_entity_from_nbt(tag);
        // These roots load with id 0 and have no command Target variant.
        // Select the newly appended root, never an older waiting entity.
        let root = match kind {
            "minecraft:tnt" => self.tnts.get_mut(deferred.0).map(|e| &mut e.id),
            "minecraft:falling_block" => self.falling.get_mut(deferred.1).map(|e| &mut e.id),
            "minecraft:firework_rocket" => self.rockets.get_mut(deferred.2).map(|e| &mut e.id),
            _ => None,
        };
        let root = root.map(|id| super::lan_entities::assign_entity_id(&mut self.next_entity, id));
        self.native_mobs_joined(from, false);
        // Otherwise the root is the first entity spawned, also for kinds no
        // command target stands for (experience orbs, end crystals, cushions). An id handed out to an
        // entity a mod then kept out is not given back, so the root must be
        // there.
        root.or_else(|| {
            root_id(target, first, |id| {
                self.orbs.iter().any(|o| o.id == id)
                    || self.crystals.iter().any(|c| c.id == id)
                    || self.cushions.iter().any(|c| c.id == id)
            })
        })
    }
    /// Moves entity `id`, one Horizon keeps without simulating (a display, a
    /// marker, an interaction), to `pos` facing `yaw` and `pitch` in degrees;
    /// a display eases there over its `teleport` ticks. False for no such
    /// entity, another dimension or a position outside `/summon`'s bounds.
    /// Mobs move with `native_teleport_mob`. API 59.
    pub(super) fn native_move_entity(
        &mut self,
        dimension: &str,
        id: u32,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_entity_level(dimension)
            || !position_ok(pos)
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            return false;
        }
        let Some(o) = self.others.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        o.position = Vec3::new(pos[0], pos[1], pos[2]);
        o.yaw = yaw.rem_euclid(360.0);
        o.pitch = pitch.clamp(-90.0, 90.0);
        true
    }
    /// Sets the transformation and interpolation of display `id` (a
    /// `block_display`, `item_display` or `text_display`): players see it
    /// ease from what it showed over `interpolation` ticks. False for no such
    /// display, another dimension, or a value that isn't finite, a zero
    /// quaternion, or a duration out of range. API 59.
    pub(super) fn native_set_display(
        &mut self,
        dimension: &str,
        id: u32,
        display: &NativeDisplay,
    ) -> bool {
        if !self.native_entity_level(dimension) || !display.valid() {
            return false;
        }
        let is_display = self.others.iter().any(|o| {
            o.id == id
                && matches!(
                    o.kind.as_str(),
                    "minecraft:block_display" | "minecraft:item_display" | "minecraft:text_display"
                )
        });
        if !is_display {
            return false;
        }
        let tag = self
            .persist
            .raw
            .entry(id)
            .or_insert_with(|| Tag::Compound(Compound::new()));
        let Tag::Compound(c) = tag else {
            return false;
        };
        display.write(c);
        true
    }
    /// Removes entity `id`, one Horizon keeps without simulating, as `/kill`
    /// does (it drops nothing and isn't saved). False for no such entity or
    /// another dimension. Mobs go with `native_remove_mob`. API
    /// 59.
    pub(super) fn native_remove_entity(&mut self, dimension: &str, id: u32) -> bool {
        if !self.native_entity_level(dimension) || !self.others.iter().any(|o| o.id == id) {
            return false;
        }
        self.others.retain(|o| o.id != id);
        self.persist.raw.remove(&id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_write_flips_the_start_so_clients_restart_the_ease() {
        let display = NativeDisplay {
            translation: [0.0, 1.5, 0.0],
            scale: [2.0; 3],
            interpolation: 10,
            ..NativeDisplay::default()
        };
        assert!(display.valid());
        let mut tag = Compound::new();
        display.write(&mut tag);
        assert_eq!(tag.get(&"start_interpolation".into()), Some(&Tag::Int(0)));
        assert_eq!(
            tag.get(&"interpolation_duration".into()),
            Some(&Tag::Int(10))
        );
        display.write(&mut tag);
        assert_eq!(tag.get(&"start_interpolation".into()), Some(&Tag::Int(-1)));
        display.write(&mut tag);
        assert_eq!(tag.get(&"start_interpolation".into()), Some(&Tag::Int(0)));
        let Some(Tag::Compound(t)) = tag.get(&"transformation".into()) else {
            panic!("no transformation");
        };
        assert_eq!(
            t.get(&"scale".into()),
            Some(&Tag::List(vec![Tag::Float(2.0f32.to_bits()); 3]))
        );
    }

    #[test]
    fn a_display_refuses_values_it_cant_show() {
        let ok = NativeDisplay::default();
        assert!(ok.valid());
        assert!(
            !NativeDisplay {
                scale: [f32::NAN, 1.0, 1.0],
                ..ok
            }
            .valid()
        );
        assert!(
            !NativeDisplay {
                left_rotation: [0.0; 4],
                ..ok
            }
            .valid()
        );
        assert!(!NativeDisplay { teleport: 60, ..ok }.valid());
        assert!(
            !NativeDisplay {
                interpolation: -1,
                ..ok
            }
            .valid()
        );
    }

    #[test]
    fn a_passenger_without_a_position_stands_where_the_stack_is_summoned() {
        let mut rider = Compound::new();
        rider.insert("id".into(), Tag::String("minecraft:pig".into()));
        let mut root = Compound::new();
        root.insert("id".into(), Tag::String("minecraft:pig".into()));
        root.insert("Passengers".into(), Tag::List(vec![Tag::Compound(rider)]));
        let mut tag = Tag::Compound(root);
        place_summoned(&mut tag, [1.5, 70.0, -2.5]);
        let pos = Tag::List(
            [1.5, 70.0, -2.5]
                .map(|v: f64| Tag::Double(v.to_bits()))
                .to_vec(),
        );
        let Tag::Compound(root) = &tag else { panic!() };
        assert_eq!(root.get(&"Pos".into()), Some(&pos));
        let Some(Tag::List(riders)) = root.get(&"Passengers".into()) else {
            panic!("no passengers");
        };
        let Tag::Compound(rider) = &riders[0] else {
            panic!()
        };
        assert_eq!(rider.get(&"Pos".into()), Some(&pos));
    }

    #[test]
    fn an_orb_root_has_an_id_only_while_it_is_in_the_level() {
        // Spawned: the orb holds the first id.
        assert_eq!(root_id(None, 36, |id| id == 36), Some(36));
        // A mod kept it out: id 36 was handed out, but no entity holds it.
        assert_eq!(root_id(None, 36, |_| false), None);
        // A command target answers for itself.
        assert_eq!(root_id(Some(Target::Mob(40)), 36, |_| false), Some(40));
        assert_eq!(root_id(Some(Target::Player), 36, |_| true), None);
    }

    #[test]
    fn summon_positions_are_summons() {
        assert!(position_ok([0.5, 64.0, 0.5]));
        assert!(!position_ok([3.0e7, 64.0, 0.5]));
        assert!(!position_ok([0.5, f64::NAN, 0.5]));
    }
}
