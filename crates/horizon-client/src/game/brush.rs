//! Brushing suspicious sand and gravel: `BrushItem.onUseTick` and
//! `BrushableBlockEntity`. The block's loot table stays in its saved block
//! entity (like a decorated pot's) until the first counted stroke rolls it.
use super::{Game, Hit, decorated_pot, view_vector};
use horizon_client::{
    block_entity_render::vertex_light,
    containers::BlockEntity,
    mesher::Vertex,
    particles::{ParticleOptions, ParticleType},
    save,
};
use horizon_core::random::RandomSource;
use std::collections::HashMap;

/// `BrushItem.USE_DURATION`.
const USE_DURATION: u32 = 200;

/// A brushable block's counters. Vanilla never saves them: the block's
/// `dusted` state is all that is left after a reload.
#[derive(Default)]
pub(super) struct BrushState {
    count: i32,
    resets_at: i64,
    cooldown_ends_at: i64,
    hit_face: Option<[i32; 3]>,
}

/// `BrushableBlockEntity.getCompletionState`: the `dusted` property.
fn completion_state(count: i32) -> i32 {
    match count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        _ => 3,
    }
}

pub(super) type Brushing = HashMap<[i32; 3], BrushState>;

impl Game<'_> {
    /// `BrushItem.onUseTick` for a brush used `ticks` ticks so far, the
    /// hand of `slot` acting.
    pub(super) fn tick_brush(&mut self, slot: usize, ticks: u32) {
        // No block under the crosshair: releaseUsingItem.
        let Some(hit) = self.pick() else {
            self.using = None;
            return;
        };
        let elapsed = ticks + 1;
        // The last tick before the backswing.
        if elapsed % 10 == 5 {
            self.brush_stroke(&hit, slot);
        }
        // The use ends after 200 ticks; the held button starts it again.
        self.using = (elapsed < USE_DURATION).then_some((slot, elapsed));
    }

    /// One stroke on the block aimed at: dust (this window only), the brush
    /// sound, and the block entity's count. The brush wears when the block
    /// changed state.
    fn brush_stroke(&mut self, hit: &Hit, slot: usize) {
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let name = self.res.facts().get(state).name.clone();
        let (sound, brushable) = match &*name {
            "minecraft:suspicious_sand" => ("minecraft:item.brush.brushing.sand", true),
            "minecraft:suspicious_gravel" => ("minecraft:item.brush.brushing.gravel", true),
            _ => ("minecraft:item.brush.brushing.generic", false),
        };
        if self.lan_acting.is_none() {
            self.brush_dust(hit, slot == super::shield::OFFHAND_SLOT);
        }
        let at = hit.pos.map(|v| f64::from(v) + 0.5);
        self.audio.play(sound, Some(at), 1.0, 1.0);
        if brushable && self.brush_block(hit.pos, hit.face, slot) {
            self.damage_held(1);
        }
    }

    /// `BrushItem.spawnDustParticles`.
    pub(super) fn brush_dust(&mut self, hit: &Hit, off_hand: bool) {
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let Some((sprite, tint)) = self.particle_sprite(state, hit.pos) else {
            return;
        };
        let options = ParticleOptions::block(ParticleType::Block, sprite, tint);
        // The arm that brushes: the main arm, or the other one for the off hand.
        let flip = if off_hand == self.settings.main_hand_left {
            1.0
        } else {
            -1.0
        };
        let view = view_vector(self.yaw, self.pitch);
        // DustParticlesDelta.fromDirection.
        let (dx, dz) = match hit.face {
            [0, _, 0] => (view[2], -view[0]),
            [0, 0, -1] => (1.0, -0.1),
            [0, 0, 1] => (-1.0, 0.1),
            [-1, 0, 0] => (-0.1, -1.0),
            _ => (0.1, 1.0),
        };
        let count = 7 + self.particles.level_random().next_int_bound(5).unwrap_or(0);
        let x = hit.point[0] - if hit.face == [-1, 0, 0] { 1.0e-6 } else { 0.0 };
        let z = hit.point[2] - if hit.face == [0, 0, -1] { 1.0e-6 } else { 0.0 };
        for _ in 0..count {
            let random = self.particles.level_random();
            let v = [
                dx * flip * 3.0 * random.next_double(),
                0.0,
                dz * flip * 3.0 * random.next_double(),
            ];
            self.particles.add(&options, [x, hit.point[1], z], v);
        }
    }

    /// `BrushableBlockEntity.brush`: whether the block changed (the brush wears).
    fn brush_block(&mut self, pos: [i32; 3], face: [i32; 3], slot: usize) -> bool {
        let time = self.world.game_time;
        let mut brush = self.brushing.remove(&pos).unwrap_or_default();
        brush.hit_face.get_or_insert(face);
        let direction = match brush.hit_face.unwrap() {
            [0, -1, 0] => 0,
            [0, 1, 0] => 1,
            [0, 0, -1] => 2,
            [0, 0, 1] => 3,
            [-1, 0, 0] => 4,
            [1, 0, 0] => 5,
            _ => return false,
        };
        if let Some(BlockEntity::Brushable(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.hit_direction = Some(direction);
        }
        brush.resets_at = time + 40;
        if time < brush.cooldown_ends_at {
            self.brushing.insert(pos, brush);
            return false;
        }
        brush.cooldown_ends_at = time + 10;
        self.unpack_brush_loot(pos);
        let before = completion_state(brush.count);
        brush.count += 1;
        if brush.count >= 10 {
            self.brushing_completed(pos, brush.hit_face, slot);
            return true;
        }
        self.schedule_brushable_tick(pos);
        let after = completion_state(brush.count);
        self.brushing.insert(pos, brush);
        if before != after {
            self.set_dusted(pos, after);
        }
        false
    }

    /// `BrushableBlockEntity.unpackLootTable`: the table's roll becomes the
    /// block entity's item.
    fn unpack_brush_loot(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Brushable(b)) = self.world.block_entities.get(&key) else {
            return;
        };
        let Some(horizon_world::nbt::Tag::String(table)) = &b.loot_table else {
            return;
        };
        let table = table.to_string_lossy();
        let mut tag = save::encode_block_entity(
            key,
            &BlockEntity::Brushable(b.clone()),
            "minecraft:brushable_block",
        );
        self.trigger_loot_table(&table);
        let loot = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        );
        decorated_pot::unpack_loot(
            &mut tag,
            loot.as_deref(),
            &mut self.world.random,
            &mut self.rand_sequences,
            self.level.seed,
        );
        if let Some(BlockEntity::Brushable(b)) = self.world.block_entities.get_mut(&key) {
            b.loot_table = tag.get("LootTable").cloned();
            b.loot_seed = tag.get("LootTableSeed").cloned();
            b.item = tag.get("item").and_then(save::item_from_nbt);
        }
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }

    /// `BrushableBlockEntity.brushingCompleted`: the item drops out of the
    /// brushed face and the block turns into its sand or gravel.
    fn brushing_completed(&mut self, pos: [i32; 3], face: Option<[i32; 3]>, slot: usize) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let turns_into = match &*self.res.facts().get(state).name {
            "minecraft:suspicious_sand" => "minecraft:sand",
            "minecraft:suspicious_gravel" => "minecraft:gravel",
            _ => "minecraft:air",
        };
        // dropContent: the item at the middle of the block beside that face.
        let item = match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Brushable(b)) => b.item.clone(),
            _ => None,
        };
        if let Some(mut item) = item {
            let face = face.unwrap_or([0, 1, 0]);
            let take = 10 + self.world.random.next_int_bound(21).unwrap_or(0);
            item.count = item.count.min(take as u32);
            let at = [
                f64::from(pos[0] + face[0]) + 0.5,
                f64::from(pos[1] + face[1]) + 0.625,
                f64::from(pos[2] + face[2]) + 0.5,
            ];
            let mut drops = vec![item];
            let tool = self.stack_in(slot).cloned();
            let origin = pos.map(|c| f64::from(c) + 0.5);
            self.native_use_drops(
                "",
                origin,
                Some((pos, state)),
                None,
                tool.as_ref(),
                &mut drops,
            );
            for item in drops {
                self.spawn_item(item, at, [0.0; 3], 0);
            }
        }
        // Level event 3008 for every player, then the block change.
        self.level_event_at(3008, pos, state.0 as i32);
        if let Some(base) = self.pack.assets.registry.default_state(turns_into) {
            let _ = self.level_set(pos, base);
        }
    }

    fn set_dusted(&mut self, pos: [i32; 3], dusted: i32) {
        let registry = self.pack.assets.registry;
        let Some(mut state) = registry
            .state(self.world.block(pos[0], pos[1], pos[2]))
            .cloned()
        else {
            return;
        };
        state.properties.insert("dusted".into(), dusted.to_string());
        if let Ok(state) = registry.resolve(&state) {
            let _ = self.level_set(pos, state);
        }
    }

    fn schedule_brushable_tick(&mut self, pos: [i32; 3]) {
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.schedule_tick(level, ticks, at, 2));
    }

    /// `BrushableBlockEntity.checkReset`, from `BrushableBlock.tick`: a block
    /// left alone loses two counts, again every four ticks.
    pub(super) fn brushable_tick(&mut self, pos: [i32; 3]) {
        let time = self.world.game_time;
        let Some(mut brush) = self.brushing.remove(&pos) else {
            return;
        };
        let before = completion_state(brush.count);
        if brush.count != 0 && time >= brush.resets_at {
            brush.count = (brush.count - 2).max(0);
            brush.resets_at = time + 4;
        }
        let after = completion_state(brush.count);
        if brush.count != 0 {
            self.brushing.insert(pos, brush);
            self.schedule_brushable_tick(pos);
        } else if let Some(BlockEntity::Brushable(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.hit_direction = None;
        }
        if before != after {
            self.set_dusted(pos, after);
        }
    }

    /// `BrushableBlockRenderer`: reveal the item from the first brushed face.
    pub(super) fn brushable_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        use horizon_assets::item_model::DisplayContext;
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Brushable(b) = entity else {
                continue;
            };
            let (Some(item), Some(face)) = (&b.item, b.hit_direction) else {
                continue;
            };
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let state = self.world.block(key.0, key.1, key.2);
            let progress = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("dusted"))
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);
            if progress == 0 {
                continue;
            }
            let offset = f32::from(progress) * 0.075;
            let (translation, adjacent, east_west) = match face {
                0 => (glam::Vec3::new(0.5, -0.23 - offset, 0.5), [0, -1, 0], false),
                1 => (glam::Vec3::new(0.5, 0.25 + offset, 0.5), [0, 1, 0], false),
                2 => (glam::Vec3::new(0.5, 0.0, 0.25 - offset), [0, 0, -1], false),
                3 => (glam::Vec3::new(0.5, 0.0, 0.73 + offset), [0, 0, 1], false),
                4 => (glam::Vec3::new(0.25 - offset, 0.0, 0.5), [-1, 0, 0], true),
                5 => (glam::Vec3::new(0.73 + offset, 0.0, 0.5), [1, 0, 0], true),
                _ => continue,
            };
            let item = self.resolve_item(item, DisplayContext::Fixed, None, 0);
            if item.is_empty() {
                continue;
            }
            let pose = glam::Mat4::from_translation(
                (at - camera).as_vec3() + glam::Vec3::new(0.0, 0.5, 0.0) + translation,
            ) * glam::Mat4::from_rotation_y(
                (86.0_f32 + if east_west { 90.0 } else { 0.0 }).to_radians(),
            ) * glam::Mat4::from_scale(glam::Vec3::splat(0.5));
            let light = self.world.light(
                key.0 + adjacent[0],
                key.1 + adjacent[1],
                key.2 + adjacent[2],
            );
            self.submit_item_world(&item, pose, vertex_light(light), out);
        }
    }

    /// Level event 3008 (`LevelEventHandler`): the finished block's sound
    /// and break particles.
    pub(super) fn brush_complete_event(&mut self, pos: [i32; 3], data: i32) {
        let state = horizon_world::registry::BlockStateId(data as u32);
        let sound = match &*self.res.facts().get(state).name {
            "minecraft:suspicious_sand" => "minecraft:item.brush.brushing.sand.complete",
            "minecraft:suspicious_gravel" => "minecraft:item.brush.brushing.gravel.complete",
            _ => "",
        };
        if !sound.is_empty() {
            let at = pos.map(|v| f64::from(v) + 0.5);
            // Each client plays it from the event itself.
            self.audio.play_unshared(sound, Some(at), 1.0, 1.0);
        }
        self.destroy_particles(pos, state);
    }
}
