//! Boats, minecarts, armor stands, item frames, paintings and fence knots on a server's
//! client (`ClientPacketListener.handleAddEntity`, `handleSetEntityData`,
//! `handleSetEquipment`): the server simulates them, the client places them
//! where it says and interpolates their moves like a mob's.
use super::Game;
use horizon_client::entities::{ArmorStand, Cushion, ItemFrame, Painting};
use horizon_client::mobs::leash::Holder;
use horizon_client::particles::{ParticleOptions, ParticleType};
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;
use horizon_protocol::entity_data::Value;
use std::sync::Arc;

impl Game<'_> {
    /// `handleAddEntity` for an object type; false when `kind` isn't one.
    /// Hanging entities come at their block, with their direction as the
    /// spawn data (`Direction.from3DDataValue`).
    pub(super) fn spawn_remote_object(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        yaw: f32,
        data: i32,
    ) -> bool {
        let id = id as u32;
        let at = Vec3::new(position[0], position[1], position[2]);
        let block = position.map(|v| v.floor() as i32);
        self.remove_remote_object(id);
        if kind == "minecraft:leash_knot" {
            // A lead that came before its knot waits as a mob holder with the
            // knot's id (handleEntityLinkPacket's delayed holder); now it can
            // be tied to the fence.
            self.remote_knots.insert(id, block);
            for m in &mut self.mobs {
                if m.extra.leash == Some(Holder::Mob(id)) {
                    m.extra.leash = Some(Holder::Knot(block));
                }
            }
            for b in &mut self.boats {
                if b.leash == Some(Holder::Mob(id)) {
                    b.leash = Some(Holder::Knot(block));
                }
            }
        } else if let Some(shows) = match kind {
            "minecraft:text_display" => Some(super::text_displays::Shows::Text),
            "minecraft:block_display" => Some(super::text_displays::Shows::Block),
            "minecraft:item_display" => Some(super::text_displays::Shows::Item),
            _ => None,
        } {
            self.text_displays.insert(
                id as i32,
                super::text_displays::RemoteTextDisplay::new(shows, position, yaw),
            );
        } else if kind == "minecraft:ominous_item_spawner" {
            self.add_remote_ominous_item_spawner(id, position);
        } else if kind == "minecraft:interaction" {
            // The width, height and response follow in SetEntityData.
            self.interactions.insert(
                id as i32,
                super::interactions::RemoteInteraction::new(position),
            );
        } else if let Some(cart) = super::minecarts::CartKind::from_id(kind) {
            let mut c = super::minecarts::Minecart::new(id, cart, at);
            c.yaw = yaw;
            c.previous_yaw = yaw;
            self.carts.push(c);
        } else if let Some((wood, chest)) = super::boats::any_boat_wood(kind)
            && self.boat_models.has(wood, chest)
        {
            let mut b = super::boats::Boat::new(id, Arc::from(wood), at, yaw);
            if chest {
                b = b.with_chest();
            }
            self.boats.push(b);
        } else if kind == "minecraft:armor_stand" {
            self.armor_stands.push(ArmorStand::new(id, at, yaw));
        } else if kind == "minecraft:item_frame" || kind == "minecraft:glow_item_frame" {
            self.item_frames.push(ItemFrame {
                id,
                pos: block,
                facing: data.clamp(0, 5) as u8,
                glow: kind == "minecraft:glow_item_frame",
                item: None,
                rotation: 0,
                drop_chance: 1.0,
                fixed: false,
                invisible: false,
                check_interval: 0,
            });
        } else if kind == "minecraft:cushion" {
            // BlockAttachedEntity.pos: `BlockPos.containing` of the spawn.
            self.cushions.push(Cushion {
                id,
                position: at,
                pos: block,
                yaw: super::cushions::snap_yaw(yaw),
                color: 0,
                check_interval: 0,
            });
        } else if kind == "minecraft:lightning_bolt" {
            self.spawn_remote_bolt(id, position);
        } else if kind == "minecraft:painting" {
            // The variant comes with the entity data; until then the
            // registry's first one (VariantUtils.getAny).
            let (variant, width, height) = self
                .remote_painting_variant(0)
                .map_or((Arc::from("minecraft:kebab"), 1, 1), |v| {
                    (v.id, v.width, v.height)
                });
            // Direction 3D data value (north 2, south 3, west 4, east 5) to 2D.
            let facing = match data {
                2 => 2,
                4 => 1,
                5 => 3,
                _ => 0,
            };
            self.paintings.push(Painting {
                id,
                pos: block,
                facing,
                variant,
                width,
                height,
                check_interval: 0,
            });
        } else {
            return false;
        }
        true
    }

    /// A `painting_variant` by its network id, from the server's registry.
    fn remote_painting_variant(&self, id: i32) -> Option<super::paintings::Variant> {
        let r = self.remote.as_ref()?;
        let name = r.registries.name("minecraft:painting_variant", id)?;
        self.painting_variants.get(name).cloned()
    }

    /// `handleSetEntityData` for an object: what changes how it looks.
    pub(super) fn remote_object_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        if self.text_displays.contains_key(&id) {
            // ItemDisplay.DATA_ITEM_STACK, read with the server's registries.
            let item = values
                .iter()
                .find_map(|(index, _, value)| match (index, value) {
                    (23, Value::Item(stack)) => Some(self.to_stack(stack)),
                    _ => None,
                });
            if let Some(display) = self.text_displays.get_mut(&id) {
                display.show(values, item);
                display.metadata(values);
            }
            return;
        }
        if self.remote_interaction_data(id, values) {
            return;
        }
        let id = id as u32;
        for (index, _, value) in values {
            if let (8, Value::Item(stack)) = (index, value)
                && self.ominous_spawners.iter().any(|s| s.id == id)
            {
                // OminousItemSpawner.DATA_ITEM.
                if let (Some(item), Some(s)) = (
                    self.to_stack(stack),
                    self.ominous_spawners.iter_mut().find(|s| s.id == id),
                ) {
                    s.item = item;
                }
            } else if self.item_frames.iter().any(|f| f.id == id) {
                // HangingEntity.DATA_DIRECTION, ItemFrame.DATA_ITEM and DATA_ROTATION.
                let item = match (index, value) {
                    (9, Value::Item(stack)) => Some(self.to_stack(stack)),
                    _ => None,
                };
                let Some(f) = self.item_frames.iter_mut().find(|f| f.id == id) else {
                    continue;
                };
                match (index, value) {
                    (0, Value::Byte(flags)) => f.invisible = (*flags as u8 & 0x20) != 0,
                    (8, Value::Id(d)) => f.facing = (*d).clamp(0, 5) as u8,
                    (10, Value::Int(r)) => f.rotation = r.rem_euclid(8) as u8,
                    _ => {}
                }
                if let Some(item) = item {
                    f.item = item;
                }
            } else if self.paintings.iter().any(|p| p.id == id) {
                // Painting.DATA_PAINTING_VARIANT_ID: a registry holder (id + 1).
                if let (9, Value::Id(holder)) = (index, value)
                    && let Some(v) = self.remote_painting_variant(holder - 1)
                    && let Some(p) = self.paintings.iter_mut().find(|p| p.id == id)
                {
                    p.variant = v.id;
                    p.width = v.width;
                    p.height = v.height;
                }
            } else if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
                match (index, value) {
                    (0, Value::Byte(flags)) => a.invisible = flags & 0x20 != 0,
                    // Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE.
                    (2, Value::OptText(name)) => {
                        a.custom_name = name.as_ref().map(horizon_client::entity_nbt::name_text);
                    }
                    (2, Value::Text(name)) => {
                        a.custom_name = Some(horizon_client::entity_nbt::name_text(name));
                    }
                    (3, Value::Bool(visible)) => a.name_visible = *visible,
                    (9, Value::Float(health)) => a.health = *health,
                    // ArmorStand.DATA_HEAD_POSE to DATA_RIGHT_LEG_POSE.
                    (16..=21, Value::Floats(r)) if r.len() == 3 => {
                        a.pose[usize::from(index - 16)] = [r[0], r[1], r[2]];
                    }
                    // ArmorStand.DATA_CLIENT_FLAGS.
                    (15, Value::Byte(flags)) => {
                        a.small = flags & 1 != 0;
                        a.show_arms = flags & 4 != 0;
                        a.no_base_plate = flags & 8 != 0;
                        a.marker = flags & 16 != 0;
                    }
                    _ => {}
                }
            } else if let Some(c) = self.cushions.iter_mut().find(|c| c.id == id) {
                // Cushion.DATA_COLOR (DyeColor, a byte-backed id).
                if let (8, Value::Id(color)) = (index, value) {
                    c.color = (*color).clamp(0, 15) as u8;
                }
            } else if let Some(b) = self.boats.iter_mut().find(|b| b.id == id) {
                // VehicleEntity's hurt, direction and damage; the paddles and
                // the bubble time.
                match (index, value) {
                    (8, Value::Int(v)) => b.hurt_time = *v,
                    (9, Value::Int(v)) => b.hurt_dir = *v,
                    (10, Value::Float(v)) => b.damage = *v,
                    (11, Value::Bool(v)) => b.paddling[0] = *v,
                    (12, Value::Bool(v)) => b.paddling[1] = *v,
                    // AbstractBoat.DATA_ID_BUBBLE_TIME.
                    (13, Value::Int(v)) => b.bubble_time = *v,
                    _ => {}
                }
            } else if let Some(c) = self.carts.iter_mut().find(|c| c.id == id) {
                match (index, value) {
                    (8, Value::Int(v)) => c.hurt_time = *v,
                    (9, Value::Int(v)) => c.hurt_dir = *v,
                    (10, Value::Float(v)) => c.damage = *v,
                    (13, Value::Bool(lit)) if c.kind == super::minecarts::CartKind::Furnace => {
                        c.fuel = i32::from(*lit);
                    }
                    // MinecartCommandBlock.onSyncedDataUpdated: setCommand
                    // (which resets the success count) and setLastOutput.
                    (13, Value::String(command))
                        if c.kind == super::minecarts::CartKind::CommandBlock =>
                    {
                        c.command.set_command(command);
                    }
                    (14, Value::Text(output))
                        if c.kind == super::minecarts::CartKind::CommandBlock =>
                    {
                        c.command.last_output = Some(horizon_client::scoreboard::comp_text(
                            &horizon_client::scoreboard::tag_to_json(output),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    /// `handleSetEquipment` for an armor stand (EquipmentSlot ordinals 0 to
    /// 5: main hand, off hand, feet, legs, chest, head, its storage order).
    pub(super) fn remote_object_equipment(
        &mut self,
        id: i32,
        slots: &[(u8, Option<horizon_protocol::item::Stack>)],
    ) {
        if !self.armor_stands.iter().any(|a| a.id == id as u32) {
            return;
        }
        let items: Vec<_> = slots
            .iter()
            .map(|(slot, stack)| (usize::from(*slot), self.to_stack(stack)))
            .collect();
        if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
            for (slot, item) in items {
                if let Some(s) = a.equipment.get_mut(slot) {
                    *s = item;
                }
            }
        }
    }

    /// A move's rotation and ground state for an object.
    pub(super) fn turn_remote_object(&mut self, id: i32, yaw: Option<f32>, on_ground: bool) {
        let id = id as u32;
        // The boat the player drives turns here (isLocalInstanceAuthoritative).
        let driven = self.riding;
        if let Some(b) = self.boats.iter_mut().find(|b| b.id == id) {
            if Some(id) == driven {
                return;
            }
            if let Some(yaw) = yaw {
                b.yaw = yaw;
            }
            b.on_ground = on_ground;
        } else if let Some(c) = self.carts.iter_mut().find(|c| c.id == id)
            && let Some(yaw) = yaw
        {
            c.yaw = yaw;
        }
    }

    /// One tick of interpolation: `step` moves a position a step toward
    /// the entity's last sent one, if it has a move in progress. A minecart
    /// on `NewMinecartBehavior` is moved by `lerp_cart` (true when it did)
    /// instead.
    pub(super) fn tick_remote_objects(
        &mut self,
        mut step: impl FnMut(u32, Vec3) -> Vec3,
        mut lerp_cart: impl FnMut(&mut super::minecarts::Minecart) -> bool,
    ) {
        for c in &mut self.carts {
            c.previous = c.position;
            c.previous_yaw = c.yaw;
            if !lerp_cart(c) {
                c.position = step(c.id, c.position);
            }
            if c.kind == super::minecarts::CartKind::Furnace
                && c.fuel > 0
                && self.world.random.next_int_bound(4).unwrap_or(1) == 0
            {
                self.particles.add(
                    &ParticleOptions::simple(ParticleType::LargeSmoke),
                    [c.position.x, c.position.y + 0.8, c.position.z],
                    [0.0; 3],
                );
            }
        }
        // The boat the player drives moves here, not where the server says.
        let driven = self.riding;
        for b in &mut self.boats {
            if Some(b.id) == driven {
                continue;
            }
            b.previous = b.position;
            b.previous_yaw = b.yaw;
            b.position = step(b.id, b.position);
        }
        for a in &mut self.armor_stands {
            a.position = step(a.id, a.position);
        }
        self.tick_remote_boat_bubbles();
    }

    /// The boats, minecarts, armor stands, item frames and paintings in the
    /// level (a server's client: the ones it was sent), for `status`.
    pub(super) fn objects_status(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        let at = |p: Vec3| [p.x, p.y, p.z];
        let mut out = Vec::new();
        for b in &self.boats {
            out.push(json!({"id": b.id, "kind": b.entity_type(), "position": at(b.position), "yaw": b.yaw}));
        }
        for c in &self.carts {
            out.push(
                json!({"id": c.id, "kind": c.kind.id(), "position": at(c.position), "yaw": c.yaw}),
            );
        }
        for a in &self.armor_stands {
            let items: Vec<_> = a
                .equipment
                .iter()
                .map(|i| i.as_ref().map(|i| &*i.item))
                .collect();
            out.push(json!({"id": a.id, "kind": "minecraft:armor_stand", "position": at(a.position), "yaw": a.yaw,
                "small": a.small, "arms": a.show_arms, "no_base_plate": a.no_base_plate, "equipment": items}));
        }
        for c in &self.cushions {
            out.push(
                json!({"id": c.id, "kind": "minecraft:cushion", "position": at(c.position),
                "yaw": c.yaw, "color": c.color_name()}),
            );
        }
        for (&id, i) in &self.interactions {
            out.push(json!({"id": id, "kind": "minecraft:interaction",
                "position": at(i.position), "width": i.width, "height": i.height,
                "response": i.response}));
        }
        for f in &self.item_frames {
            out.push(
                json!({"id": f.id, "kind": f.kind(), "position": f.pos, "facing": f.facing,
                "item": f.item.as_ref().map(|i| &*i.item), "rotation": f.rotation}),
            );
        }
        for p in &self.paintings {
            out.push(json!({"id": p.id, "kind": "minecraft:painting", "position": p.pos, "facing": p.facing,
                "variant": &*p.variant}));
        }
        out
    }

    /// Every object a server sent (a new connection, level or dimension).
    pub(super) fn clear_remote_objects(&mut self) {
        self.text_displays.clear();
        self.interactions.clear();
        self.ominous_spawners.clear();
        self.remote_knots.clear();
        self.carts.clear();
        self.boats.clear();
        self.armor_stands.clear();
        self.cushions.clear();
        self.item_frames.clear();
        self.paintings.clear();
    }

    pub(super) fn remove_remote_object(&mut self, id: u32) {
        self.text_displays.remove(&(id as i32));
        self.interactions.remove(&(id as i32));
        self.ominous_spawners.retain(|s| s.id != id);
        self.remote_knots.remove(&id);
        self.carts.retain(|c| c.id != id);
        self.boats.retain(|b| b.id != id);
        self.armor_stands.retain(|a| a.id != id);
        // `Entity.setRemoved`: the cushion's rider gets off where it sat
        // (`LivingEntity.dismountVehicle`), not inside the block below.
        if self.riding == Some(id)
            && let Some(y) = self
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.position.y)
        {
            self.dismount_removed_vehicle(y);
        }
        self.cushions.retain(|c| c.id != id);
        self.item_frames.retain(|f| f.id != id);
        self.paintings.retain(|p| p.id != id);
        self.bolts.retain(|b| b.id != id);
    }
}
