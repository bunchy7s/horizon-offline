//! Parrots on a player's shoulder: a tame parrot lands on its owner
//! (`LandOnOwnersShoulderGoal`), is kept as the player's `ShoulderEntityLeft`
//! or `ShoulderEntityRight`, and flies off again when the player is hurt,
//! falls, swims, flies or sleeps (`ServerPlayer.handleShoulderEntities`).
use super::Game;
use horizon_client::mobs::Species;
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::nbt::Tag;

const KEYS: [&str; 2] = ["ShoulderEntityLeft", "ShoulderEntityRight"];

/// A shoulder slot holds a parrot when its compound is not empty.
fn occupied(extra: &Tag, key: &str) -> bool {
    matches!(extra.get(key), Some(Tag::Compound(c)) if !c.is_empty())
}

/// `ServerPlayer.extractParrotVariant` for both slots: the variant of the
/// parrot kept there (`Parrot.Variant.byId` clamps the saved `Variant`, 0
/// when absent), `None` for an empty slot or another entity.
pub(super) fn shoulder_variants(extra: Option<&Tag>) -> [Option<i32>; 2] {
    KEYS.map(|key| {
        let parrot = extra?.get(key)?;
        match parrot.get("id") {
            Some(Tag::String(id)) if id.to_string_lossy() == "minecraft:parrot" => {}
            _ => return None,
        }
        Some(match parrot.get("Variant") {
            Some(Tag::Int(v)) => (*v).clamp(0, 4),
            _ => 0,
        })
    })
}

impl Game<'_> {
    /// `ServerPlayer.setEntityOnShoulder` for a parrot that touched its
    /// owner: stored in the first free slot, the parrot gone from the world.
    pub(super) fn parrot_to_shoulder(&mut self, mob: u32) {
        let Some(i) = self
            .mobs
            .iter()
            .position(|m| m.id == mob && m.alive() && !m.removed && m.species() == Species::Parrot)
        else {
            return;
        };
        let Some(p) = self.player.as_ref() else {
            return;
        };
        // The acting player must be the saved owner (`getOwner`).
        if !horizon_client::mobs::tame::owned_by(self.mobs[i].extra.owner, self.acting_uuid()) {
            return;
        }
        if self.riding.is_some()
            || !p.ground.on_ground
            || p.fluids.was_touching_water
            || p.in_powder_snow
            || p.flying
        {
            return;
        }
        let extra = self
            .level
            .player_extra
            .get_or_insert_with(|| Tag::Compound(Default::default()));
        let Some(key) = KEYS.into_iter().find(|k| !occupied(extra, k)) else {
            return;
        };
        let tag = self.mob_tree_nbt(i);
        if let Some(Tag::Compound(map)) = self.level.player_extra.as_mut() {
            map.insert(key.into(), tag);
        }
        let (uuid, now) = (self.acting_uuid(), self.world.game_time);
        self.creatures.shoulder_time.insert(uuid, now);
        self.mobs[i].removed = true;
    }

    /// `ServerPlayer.handleShoulderEntities`: the parrots leave the shoulder
    /// of a player who falls, swims, flies, sleeps, is hurt, dying, a
    /// spectator or spinning with a riptide.
    pub(super) fn handle_shoulder_entities(&mut self) {
        for key in KEYS {
            self.play_shoulder_entity_ambient_sound(key);
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let any = self
            .level
            .player_extra
            .as_ref()
            .is_some_and(|e| KEYS.iter().any(|k| occupied(e, k)));
        if any
            && (p.fall_distance > 0.5
                || p.fluids.was_touching_water
                || p.flying
                || p.spectator
                || p.in_powder_snow
                || p.auto_spin_attack_ticks > 0
                || p.health.hurt.hurt_time > 0
                || p.health.dead_or_dying()
                || self.sleeping.is_some())
        {
            self.remove_entities_on_shoulder();
        }
    }

    /// `ServerPlayer.playShoulderEntityAmbientSound`: a parrot that is not
    /// `Silent` chirps or imitates a mob within 20 blocks, one tick in 200
    /// (drawn from the level's random; the player's own is not kept).
    fn play_shoulder_entity_ambient_sound(&mut self, key: &str) {
        let Some(Tag::Compound(tag)) = self.level.player_extra.as_ref().and_then(|e| e.get(key))
        else {
            return;
        };
        let silent = matches!(tag.get(&"Silent".into()), Some(Tag::Byte(b)) if *b != 0);
        let parrot = matches!(
            tag.get(&"id".into()),
            Some(Tag::String(id)) if id.to_string_lossy() == "minecraft:parrot"
        );
        if tag.is_empty() || silent || self.world.random.next_int_bound(200).unwrap_or(1) != 0 {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if !parrot {
            return;
        }
        let area = p.bounds().inflate(Vec3::new(20.0, 20.0, 20.0));
        let at = [p.position.x, p.position.y, p.position.z];
        let alive = !p.health.dead_or_dying();
        let nearby: Vec<_> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && !m.removed && m.bounds().intersects(area))
            .map(|m| (m.species(), m.silent))
            .collect();
        let peaceful = self.ui.difficulty == 0;
        if let Some((sound, volume, pitch)) = horizon_client::mobs::parrot::shoulder_sound(
            &mut self.world.random,
            alive,
            false,
            peaceful,
            &nearby,
        ) {
            self.audio
                .play(&format!("minecraft:{sound}"), Some(at), volume, pitch);
        }
    }

    /// `ServerPlayer.removeEntitiesOnShoulder` (not within a second of the
    /// landing) and `respawnEntityOnShoulder`: the parrots come back at the
    /// player, still owned by them.
    fn remove_entities_on_shoulder(&mut self) {
        let (uuid, now) = (self.acting_uuid(), self.world.game_time);
        if self
            .creatures
            .shoulder_time
            .get(&uuid)
            .is_some_and(|t| t + 20 >= now)
        {
            return;
        }
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let Some(Tag::Compound(map)) = self.level.player_extra.as_mut() else {
            return;
        };
        let tags: Vec<Tag> = KEYS
            .iter()
            .filter_map(|k| map.remove(&(*k).into()))
            .collect();
        let pos = [p.x, p.y + f64::from(0.7_f32), p.z];
        for mut tag in tags {
            let Tag::Compound(c) = &mut tag else {
                continue;
            };
            if c.is_empty() {
                continue;
            }
            c.insert(
                "Pos".into(),
                Tag::List(pos.iter().map(|v| Tag::Double(v.to_bits())).collect()),
            );
            c.insert("Owner".into(), Tag::IntArray(uuid.to_vec()));
            self.load_mob(tag, None);
        }
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;
    #[test]
    fn shoulder_slots_hold_parrots_by_variant() {
        let parrot = |variant: Option<i32>, id: &str| {
            let mut entries = vec![("id", Tag::String(id.into()))];
            if let Some(v) = variant {
                entries.push(("Variant", Tag::Int(v)));
            }
            Tag::compound(entries)
        };
        let extra = Tag::compound([
            ("ShoulderEntityLeft", parrot(Some(9), "minecraft:parrot")),
            ("ShoulderEntityRight", parrot(None, "minecraft:parrot")),
        ]);
        // Variant.byId clamps; a parrot saved without one is the first.
        assert_eq!(shoulder_variants(Some(&extra)), [Some(4), Some(0)]);
        let other = Tag::compound([("ShoulderEntityRight", parrot(Some(1), "minecraft:cat"))]);
        assert_eq!(shoulder_variants(Some(&other)), [None, None]);
        assert_eq!(shoulder_variants(None), [None, None]);
    }
}
