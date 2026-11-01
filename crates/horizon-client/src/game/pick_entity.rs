//! Pick Block on an entity (`Minecraft.pickBlockOrEntity`'s entity case and
//! `ServerGamePacketListenerImpl.handlePickItemFromEntity`): the client aims,
//! the server answers with `Entity.getPickResult`.
use super::Game;
use horizon_client::{items::ItemStack, lan::playerdata::uuid_ints, save::uuid_text};

impl Game<'_> {
    /// The entity the crosshair hits before any block, as `Entity.getId`; an
    /// entity with nothing to pick (a projectile, a player, the dragon) is
    /// `Some(None)`. `None` when the view hits a block or nothing.
    pub(super) fn pick_entity_hit(&self) -> Option<Option<i32>> {
        let mut best: Option<(f64, Option<i32>)> = None;
        let mut offer = |distance: f64, id: Option<i32>| {
            if self.before_block(distance) && best.is_none_or(|b| distance < b.0) {
                best = Some((distance, id));
            }
        };
        if let Some(hit) = self.remote_object_hit(true) {
            // Interaction.getPickResult is Entity's: nothing to pick.
            let id = (!self.interactions.contains_key(&hit.id)).then_some(hit.id);
            offer(hit.distance, id);
        }
        if let Some((i, d)) = self.mob_hit() {
            offer(d, Some(self.mobs[i].id as i32));
        }
        if let Some((_, d)) = self.guest_hit() {
            offer(d, None);
        }
        if let Some((_, d)) = self.redirectable_hit() {
            offer(d, None);
        }
        if let Some((_, d)) = self.dragon_hit() {
            offer(d, None);
        }
        best.map(|b| b.1)
    }

    /// `ClientSuggestionProvider.getSelectedEntities`: the UUID
    /// (`Entity.getStringUUID`) of the entity the crosshair hits before any
    /// block (`Minecraft.hitResult` an `EntityHitResult`).
    pub(super) fn crosshair_entity_uuid(&self) -> Option<String> {
        // The nearest entity wins, as in `pick_entity_hit`; one whose UUID
        // the client doesn't know (the dragon's parts) suggests nothing.
        let mut best: Option<(f64, Option<[i32; 4]>)> = None;
        let mut offer = |distance: f64, uuid: Option<[i32; 4]>| {
            if self.before_block(distance) && best.is_none_or(|b| distance < b.0) {
                best = Some((distance, uuid));
            }
        };
        let uuid_of = |id: u32| self.persist.uuids.get(&id).copied();
        if let Some(hit) = self.remote_object_hit(true) {
            offer(hit.distance, uuid_of(hit.id as u32));
        }
        if let Some((i, d)) = self.mob_hit() {
            offer(d, uuid_of(self.mobs[i].id));
        }
        if let Some((conn, d)) = self.guest_hit() {
            let guest = self.lan.as_ref().and_then(|h| h.hub.guest(conn));
            offer(d, guest.map(|g| uuid_ints(g.uuid)));
        }
        if let Some((i, d)) = self.redirectable_hit() {
            offer(d, uuid_of(self.creatures.projectiles[i].id));
        }
        if let Some((_, d)) = self.dragon_hit() {
            offer(d, None);
        }
        best.and_then(|b| b.1).map(uuid_text)
    }

    /// `Entity.getPickResult` of the entity `id`: a mob's spawn egg, the item
    /// of a boat, minecart, armor stand, cushion, painting, item frame (the
    /// framed item when it holds one) or end crystal.
    pub(super) fn entity_pick_result(&self, id: i32) -> Option<ItemStack> {
        let id = u32::try_from(id).ok()?;
        let item = if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            // Mob.getPickResult: SpawnEggItem.byId.
            let egg = format!("{}_spawn_egg", m.kind.id);
            return self.items().get(&egg).map(|_| ItemStack::new(&egg, 1));
        } else if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            b.entity_type()
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            c.kind.item().to_owned()
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            "minecraft:armor_stand".to_owned()
        } else if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            c.item()
        } else if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            // ItemFrame.getPickResult: what it frames, else the frame itself.
            return Some(
                f.item
                    .clone()
                    .unwrap_or_else(|| ItemStack::new(f.kind(), 1)),
            );
        } else if self.paintings.iter().any(|p| p.id == id) {
            "minecraft:painting".to_owned()
        } else if self.crystals.iter().any(|c| c.id == id) {
            "minecraft:end_crystal".to_owned()
        } else {
            return None;
        };
        Some(ItemStack::new(&item, 1))
    }

    /// `Player.isWithinEntityInteractionRange(entity, 3.0)`: the entity's box
    /// within the interaction range plus 3 of the eyes.
    pub(super) fn within_entity_range(&self, id: i32) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let mob = u32::try_from(id)
            .ok()
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .map(|m| m.bounds());
        let Some(b) = self
            .object_target(id)
            .map(|(lo, hi, _)| (lo, hi))
            .or_else(|| {
                mob.or_else(|| self.tracked_special_box(id)).map(|b| {
                    let (lo, hi) = (b.min_position(), b.max_position());
                    ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
                })
            })
        else {
            return false;
        };
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(b.0[a], b.1[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }
}
