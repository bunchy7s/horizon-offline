//! The sprites of item particles: `BreakingItemParticle.ItemParticleProvider.getSprite`
//! resolves the stack's model for the ground and picks a layer's particle sprite.
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    item_render::stack_view,
    items::ItemStack,
    particles::{self, ParticleOptions, ParticleType},
};
use horizon_core::random::RandomSource;
use horizon_world::player_inventory::EquipmentSlot;
use serde_json::Value;

impl Game<'_> {
    /// `ItemParticleOption(ITEM, stack)`: the layer particle sprites of the
    /// stack's model resolved for the ground, with seed 0.
    pub(super) fn item_particle(&self, stack: &ItemStack) -> ParticleOptions {
        let state = self.resolve_item(stack, DisplayContext::Ground, None, 0);
        ParticleOptions::item(state.layers.iter().map(|l| l.particle.clone()).collect())
    }
    /// `LivingEntity.spawnItemParticles(stack, count)` for the local player,
    /// drawing from the world random as its eating sounds do.
    pub(super) fn eat_particles(&mut self, stack: &ItemStack, count: u32) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let eye_y = self.eye_position().map_or(p.y, |e| e[1]);
        let options = self.item_particle(stack);
        let mut spawned = Vec::new();
        particles::item_use_particles(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            [p.x, p.y, p.z],
            eye_y,
            count,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `LivingEntity.handleEntityEvent` 47 to 52, 65 and 68 from a server:
    /// the entity's item in `slot` broke, so `breakItem` the stack this
    /// client still has there (the slot is emptied by a later update).
    pub(super) fn remote_equipment_break(&mut self, id: i32, slot: EquipmentSlot) {
        if self.remote.as_ref().is_some_and(|r| r.entity_id == id) {
            let stack = match slot {
                EquipmentSlot::MainHand => self.held().cloned(),
                EquipmentSlot::OffHand => self.offhand.clone(),
                EquipmentSlot::Head => self.armor[0].clone(),
                EquipmentSlot::Chest => self.armor[1].clone(),
                EquipmentSlot::Legs => self.armor[2].clone(),
                EquipmentSlot::Feet => self.armor[3].clone(),
                EquipmentSlot::Body | EquipmentSlot::Saddle => None,
            };
            let Some(stack) = stack else {
                return;
            };
            if let Some(p) = self.player.as_ref().map(|p| p.position) {
                self.break_sound_at(&stack, [p.x, p.y, p.z]);
            }
            self.eat_particles(&stack, 5);
            return;
        }
        let Some(m) = self.mobs.iter().find(|m| m.id == id as u32) else {
            return;
        };
        let Some(stack) = m.item_by_slot(slot) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let (eye, pitch, yaw, silent) = (m.eye_height(), m.head_pitch, m.yaw, m.silent);
        if !silent {
            self.break_sound_at(&stack, at);
        }
        let options = self.item_particle(&stack);
        let mut spawned = Vec::new();
        particles::item_use_particles(
            &mut self.world.random,
            pitch,
            yaw,
            at,
            at[1] + eye,
            5,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `LivingEntity.breakItem`'s sound: the stack's `break_sound` (none when
    /// the component is removed), heard by this client only
    /// (`playLocalSound`) at the entity's feet, at 0.8 volume and a pitch of
    /// 0.8 to 1.2.
    pub(super) fn break_sound_at(&mut self, stack: &ItemStack, at: [f64; 3]) {
        let models = &self.pack.item_renderer.models;
        let sound = match models
            .components(&stack_view(stack))
            .get("minecraft:break_sound")
        {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Object(inline)) => match inline.get("sound_id") {
                Some(Value::String(id)) => id.clone(),
                _ => return,
            },
            _ => return,
        };
        let pitch = 0.8 + self.world.random.next_float() * 0.4;
        self.audio.play_unshared(&sound, Some(at), 0.8, pitch);
    }
    /// The slime ball, snowball and cobweb models behind `item_slime`,
    /// `item_snowball` and `item_cobweb`, resolved on first use.
    pub(super) fn ensure_item_particle_sprites(&mut self) {
        if !self.particles.item_sprites.is_empty() {
            return;
        }
        for (ty, item) in [
            (ParticleType::ItemSlime, "minecraft:slime_ball"),
            (ParticleType::ItemSnowball, "minecraft:snowball"),
            (ParticleType::ItemCobweb, "minecraft:cobweb"),
        ] {
            let sprites = self
                .item_particle(&ItemStack::new(item, 1))
                .item
                .unwrap_or_default();
            self.particles.item_sprites.insert(ty, sprites);
        }
    }
}
