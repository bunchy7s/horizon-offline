//! The displayed items of `ShelfBlockEntity`.
use super::{Game, Hit, machines::dir_from_name};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{block_entity_render::vertex_light, containers::BlockEntity, mesher::Vertex};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use horizon_world::level::hit_slot;

impl Game<'_> {
    /// Slot and facing for a click on a shelf's front face.
    pub(super) fn shelf_hit(&self, hit: &Hit) -> Option<(usize, Direction, bool)> {
        let p = hit.pos;
        if !matches!(
            self.world.block_entities.get(&(p[0], p[1], p[2])),
            Some(BlockEntity::Shelf(_))
        ) {
            return None;
        }
        let state = self.world.block(p[0], p[1], p[2]);
        let data = self.pack.assets.registry.state(state)?;
        let facing = data
            .properties
            .get("facing")
            .and_then(|s| dir_from_name(s))?;
        let face = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == hit.face)?;
        let slot = hit_slot(
            facing,
            face,
            BlockPos::new(p[0], p[1], p[2]),
            Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            1,
            3,
        )?;
        Some((
            slot,
            facing,
            data.properties.get("powered").is_some_and(|s| s == "true"),
        ))
    }

    /// `SideChainPartBlock.getAllBlocksConnectedTo`: left to right, at most three.
    fn shelf_chain(&self, pos: [i32; 3], facing: Direction) -> Vec<[i32; 3]> {
        let left = match facing {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
            _ => return Vec::new(),
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let behaviours = self.res.behaviours();
        if !behaviours
            .tag("minecraft:wooden_shelves")
            .is_some_and(|tag| tag.contains(behaviours.block(state)))
        {
            return Vec::new();
        }
        let mut result = vec![pos];
        for (dir, end, prepend) in [(left, "left", true), (left.opposite(), "right", false)] {
            let normal = dir.normal();
            for step in 1..3 {
                let p = [pos[0] + normal[0] * step, pos[1], pos[2] + normal[2] * step];
                let state = self.world.block(p[0], p[1], p[2]);
                let Some(info) = self.pack.assets.registry.state(state) else {
                    break;
                };
                let behaviours = self.res.behaviours();
                let connectable = behaviours
                    .tag("minecraft:wooden_shelves")
                    .is_some_and(|tag| tag.contains(behaviours.block(state)))
                    && info.properties.get("powered").is_some_and(|s| s == "true")
                    && info.properties.get("facing").and_then(|s| dir_from_name(s)) == Some(facing);
                if !connectable {
                    break;
                }
                let part = info
                    .properties
                    .get("side_chain")
                    .map(String::as_str)
                    .unwrap_or("unconnected");
                if part == "center" || part == end {
                    if prepend {
                        result.insert(0, p);
                    } else {
                        result.push(p);
                    }
                }
                if part != "center" {
                    break;
                }
            }
        }
        result
    }

    /// `ShelfBlock.useItemOn`: a single swap, or a connected hotbar swap while powered.
    pub(super) fn use_shelf(&mut self, hit: &Hit) -> Option<bool> {
        use horizon_client::first_person::Hand;
        if self.hand != Hand::Main {
            return None;
        }
        let (slot, facing, powered) = self.shelf_hit(hit)?;
        let pos = hit.pos;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if powered {
            let chain = self.shelf_chain(pos, facing);
            let mut swapped = false;
            let count = chain.len();
            for (i, p) in chain.into_iter().enumerate() {
                let key = (p[0], p[1], p[2]);
                let Some(BlockEntity::Shelf(shelf)) = self.world.block_entities.get_mut(&key)
                else {
                    continue;
                };
                for n in 0..3 {
                    let index = 9_i32 - (count as i32 - i as i32) * 3 + n as i32;
                    if !(0..9).contains(&index) {
                        continue;
                    }
                    let index = index as usize;
                    swapped |= self.inventory[index].is_some() || shelf.items[n].is_some();
                    std::mem::swap(&mut self.inventory[index], &mut shelf.items[n]);
                }
                self.container_changed(key);
                let s = self.world.block(p[0], p[1], p[2]);
                self.post_block_game_event("minecraft:entity_interact", p, None, Some(s));
            }
            if !swapped {
                return Some(false);
            }
            self.audio.play(
                "minecraft:block.shelf.multi_swap",
                Some(pos.map(|n| f64::from(n) + 0.5)),
                1.0,
                1.0,
            );
            Some(true)
        } else {
            let held = self.inventory[self.selected].clone();
            let key = (pos[0], pos[1], pos[2]);
            let Some(BlockEntity::Shelf(shelf)) = self.world.block_entities.get_mut(&key) else {
                return None;
            };
            let removed = std::mem::replace(&mut shelf.items[slot], held.clone());
            self.inventory[self.selected] = if self.creative && removed.is_none() {
                held.clone()
            } else {
                removed.clone()
            };
            self.container_changed(key);
            let vibrations = self.inventory[self.selected].as_ref().is_none_or(|stack| {
                let default = self
                    .res
                    .items()
                    .get(&stack.item)
                    .is_none_or(|item| item.interact_vibrations);
                let Some(data) = stack.data() else {
                    return default;
                };
                if data
                    .other
                    .iter()
                    .any(|(key, _)| key == "!minecraft:use_effects")
                {
                    return true;
                }
                data.other
                    .iter()
                    .find(|(key, _)| key == "minecraft:use_effects")
                    .map_or(default, |(_, tag)| {
                        horizon_client::save::number(tag.get("interact_vibrations")).unwrap_or(1)
                            != 0
                    })
            });
            if vibrations {
                self.post_block_game_event(
                    "minecraft:item_interact_finish",
                    pos,
                    None,
                    Some(state),
                );
            }
            let sound = match (held.is_some(), removed.is_some()) {
                (false, false) => return None,
                (true, false) => "minecraft:block.shelf.place_item",
                (false, true) => "minecraft:block.shelf.take_item",
                (true, true) => "minecraft:block.shelf.single_swap",
            };
            self.audio
                .play(sound, Some(pos.map(|n| f64::from(n) + 0.5)), 1.0, 1.0);
            Some(true)
        }
    }

    /// `ShelfRenderer`: three items face out, aligned to the bottom or centre.
    pub(super) fn shelf_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Shelf(shelf) = entity else {
                continue;
            };
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let block = self.world.block(key.0, key.1, key.2);
            let facing = self
                .pack
                .assets
                .registry
                .state(block)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str)
                .unwrap_or("north");
            let yaw: f32 = match facing {
                "south" => 0.0,
                "west" => -90.0,
                "north" => -180.0,
                "east" => -270.0,
                _ => 180.0,
            };
            let packed = BlockPos::new(key.0, key.1, key.2).pack() as u64;
            let seed = (packed ^ (packed >> 32)) as i32;
            let light = vertex_light(self.world.light(key.0, key.1, key.2));
            for (slot, stack) in shelf.items.iter().enumerate() {
                let Some(stack) = stack else { continue };
                let item = self.resolve_item(
                    stack,
                    DisplayContext::OnShelf,
                    None,
                    seed.wrapping_add(slot as i32),
                );
                if item.is_empty() {
                    continue;
                }
                let (lo, hi) = item.bounding_box();
                let offset_y = -lo.y
                    - if shelf.align_to_bottom {
                        0.0
                    } else {
                        (hi.y - lo.y) / 2.0
                    };
                let pose =
                    glam::Mat4::from_translation((at - camera).as_vec3() + glam::Vec3::splat(0.5))
                        * glam::Mat4::from_rotation_y(yaw.to_radians())
                        * glam::Mat4::from_translation(glam::Vec3::new(
                            (slot as f32 - 1.0) * 0.3125,
                            if shelf.align_to_bottom { -0.25 } else { 0.0 },
                            -0.25,
                        ))
                        * glam::Mat4::from_scale(glam::Vec3::splat(0.25))
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, offset_y, 0.0));
                self.submit_item_world(&item, pose, light, out);
            }
        }
    }
}
