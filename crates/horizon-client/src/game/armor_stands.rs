//! Armor stands: `ArmorStandItem.useOn` placement (snapped to 45 degrees),
//! falling, `interactAt` equipment swaps by clicked height, the two-hit break
//! of `hurtServer` (creative breaks at once), and `ArmorStandRenderer` with
//! the default pose, the hit wobble and humanoid armor layers.
use super::Game;
use horizon_client::{
    entities::ArmorStand,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Mob, Part, anim::Poses},
    trims::{TrimOverride, TrimRegistry, TrimTexture},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// `EquipmentLayerRenderer.getColorForLayer`: a dyeable layer (one with a
/// colour when undyed) takes the stack's `minecraft:dyed_color`, else that
/// colour; any other layer stays white, dyed or not. The result is in the
/// renderer's byte order (R and B swapped).
fn layer_color(undyed: Option<u32>, stack: Option<&ItemStack>) -> u32 {
    let dyed = stack
        .and_then(|s| s.data())
        .and_then(|d| d.dyed_color)
        .map(|c| c as u32 & 0xFF_FFFF);
    undyed.map_or(0xFFFF_FFFF, |undyed| {
        let c = dyed.unwrap_or(undyed);
        0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
    })
}

/// Armor stand models and the humanoid layers of every equipment asset.
pub(super) struct ArmorStandAssets {
    pub main: Option<Part>,
    pub small: Option<Part>,
    /// (slot model suffix, small) -> model.
    pub layers: HashMap<(&'static str, bool), Part>,
    /// The player's armour models by slot (helmet, chestplate, leggings, boots).
    pub player: HashMap<&'static str, Part>,
    /// asset id -> layer kind (humanoid / humanoid_leggings) -> (texture, undyed colour).
    pub equipment: HashMap<String, HashMap<String, (String, Option<u32>)>>,
    /// asset id -> `trim_overrides`.
    pub trim_overrides: HashMap<String, Vec<TrimOverride>>,
    pub trims: TrimRegistry,
    /// The resources the trim textures are made from.
    pub root: PathBuf,
}
impl ArmorStandAssets {
    pub fn load(
        root: &std::path::Path,
        document: Option<&horizon_client::mobs::EntityModelDocument>,
    ) -> Self {
        let load = |key: &str| document.and_then(|document| document.model(key));
        let mut layers = HashMap::new();
        let mut player = HashMap::new();
        for slot in ["helmet", "chestplate", "leggings", "boots"] {
            if let Some(part) = load(&format!("minecraft:player#{slot}")) {
                player.insert(slot, part);
            }
            for (small, model) in [(false, "armor_stand"), (true, "armor_stand_small")] {
                if let Some(part) = load(&format!("minecraft:{model}#{slot}")) {
                    layers.insert((slot, small), part);
                }
            }
        }
        for (key, name) in [
            ("minecraft:elytra#main", "elytra"),
            ("minecraft:elytra_baby#main", "elytra_baby"),
        ] {
            if let Some(part) = load(key) {
                player.insert(name, part);
            }
        }
        let mut equipment = HashMap::new();
        let mut trim_overrides = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(root.join("assets/minecraft/equipment")) {
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(v) =
                    std::fs::read(&path).map(|b| serde_json::from_slice::<serde_json::Value>(&b))
                else {
                    continue;
                };
                let Ok(v) = v else { continue };
                let mut kinds = HashMap::new();
                for kind in ["humanoid", "humanoid_leggings", "wings"] {
                    if let Some(layer) = v["layers"][kind].get(0) {
                        let tex = layer["texture"]
                            .as_str()
                            .unwrap_or(name)
                            .trim_start_matches("minecraft:");
                        let undyed = layer["dyeable"]["color_when_undyed"]
                            .as_i64()
                            .map(|c| c as u32 & 0xFF_FFFF);
                        kinds.insert(
                            kind.to_owned(),
                            (format!("entity/equipment/{kind}/{tex}"), undyed),
                        );
                    }
                }
                trim_overrides.insert(format!("minecraft:{name}"), TrimOverride::list(&v));
                equipment.insert(format!("minecraft:{name}"), kinds);
            }
        }
        Self {
            main: load("minecraft:armor_stand#main"),
            small: load("minecraft:armor_stand_small#main"),
            layers,
            player,
            equipment,
            trim_overrides,
            trims: TrimRegistry::load(root),
            root: root.to_owned(),
        }
    }
    /// Trim data of reloaded resources (the models stay).
    pub fn reload_trims(&mut self, root: &std::path::Path) {
        let fresh = Self::load(root, None);
        self.trim_overrides = fresh.trim_overrides;
        self.trims = fresh.trims;
        self.root = fresh.root;
    }
    /// `TrimTextureKey`: the trim texture of `stack` on its humanoid
    /// layer (`humanoid` or `humanoid_leggings`), if it has a trim.
    fn trim_texture(&self, asset: &str, kind: &str, stack: &ItemStack) -> Option<TrimTexture> {
        let (material, pattern) = stack.extra.as_ref()?.trim.as_ref()?;
        let overrides = self
            .trim_overrides
            .get(asset)
            .map_or(&[][..], Vec::as_slice);
        TrimTexture::resolve(&self.trims, overrides, material, pattern, kind)
    }
}
impl Game<'_> {
    /// `HumanoidArmorLayer` for the player: (model, texture rect, colour,
    /// overlay) of each worn piece, head to feet.
    pub(super) fn player_armor_layers(&self) -> Vec<(&Part, [f32; 4], u32, u32)> {
        self.stack_armor_layers(&self.armor)
    }
    /// `HumanoidArmorLayer` for worn stacks (head to feet), trims included.
    pub(super) fn stack_armor_layers(
        &self,
        armor: &[Option<ItemStack>; 4],
    ) -> Vec<(&Part, [f32; 4], u32, u32)> {
        self.armor_layers(
            armor
                .each_ref()
                .map(|s| s.as_ref().map(|s| (&*s.item, Some(s)))),
        )
    }
    /// `HumanoidArmorLayer` for another player's model: the armour it wears
    /// (`extra.head` .. `extra.feet`).
    pub(super) fn worn_armor_layers(&self, m: &Mob) -> Vec<(&Part, [f32; 4], u32, u32)> {
        let e = &m.extra;
        self.armor_layers(
            [
                (e.head.as_deref(), e.armor[0].as_ref()),
                (e.chest.as_deref(), e.armor[1].as_ref()),
                (e.legs.as_deref(), e.armor[2].as_ref()),
                (e.feet.as_deref(), e.armor[3].as_ref()),
            ]
            .map(|(item, stack)| item.map(|i| (i, stack))),
        )
    }
    /// The layers of the armour items `worn` (with their stacks when known,
    /// for trims), head to feet.
    fn armor_layers(
        &self,
        worn: [Option<(&str, Option<&ItemStack>)>; 4],
    ) -> Vec<(&Part, [f32; 4], u32, u32)> {
        let assets = &self.armor_stand_assets;
        let mut out = Vec::new();
        for (item, layer) in worn
            .into_iter()
            .zip(["helmet", "chestplate", "leggings", "boots"])
        {
            let Some((item, stack)) = item else {
                continue;
            };
            let Some(asset) = self
                .res
                .items()
                .get(item)
                .and_then(|i| i.equip_asset.clone())
            else {
                continue;
            };
            let kind = if layer == "leggings" {
                "humanoid_leggings"
            } else {
                "humanoid"
            };
            let Some((texture, undyed)) = assets.equipment.get(&asset).and_then(|k| k.get(kind))
            else {
                continue;
            };
            let (Some(r), Some(part)) = (
                self.gui.atlas.sprites.get(texture.as_str()),
                assets.player.get(layer),
            ) else {
                continue;
            };
            let color = layer_color(*undyed, stack);
            // EquipmentLayerRenderer.renderLayers: NO_OVERLAY, the armour
            // glint on the layer and its trim when `stack.hasFoil()`.
            let item_glint = self
                .res
                .items()
                .enchantments
                .props
                .get(item)
                .is_some_and(|p| p.glint);
            let overlay = if stack.is_some_and(|s| s.has_foil(item_glint)) {
                horizon_client::mobs::render::ARMOR_GLINT
            } else {
                0
            };
            out.push((part, [r.u0, r.v0, r.u1, r.v1], color, overlay));
            // EquipmentLayerRenderer: the trim layer over the armour.
            if let Some(r) = stack
                .and_then(|s| assets.trim_texture(&asset, kind, s))
                .and_then(|t| self.gui.atlas.sprites.get(&t.sprite()))
            {
                out.push((part, [r.u0, r.v0, r.u1, r.v1], 0xFFFF_FFFF, overlay));
            }
        }
        out
    }
    /// `PalettedTextureManager.getOrPrepare` for the trims drawn this frame
    /// (the player's armour and the armour stands'): each generated texture
    /// goes into an atlas trim slot, a free one, else the least recently
    /// drawn.
    pub(super) fn prepare_trims(&mut self) {
        use horizon_client::hud::TRIM_SLOTS;
        let assets = &self.armor_stand_assets;
        let items = self.items();
        let mut wanted: Vec<TrimTexture> = Vec::new();
        let guests = self.lan.iter().flat_map(|host| host.hub.guests.iter());
        let stacks = self
            .armor
            .iter()
            .chain(guests.flat_map(|g| g.armor.iter()))
            .chain(self.mobs.iter().flat_map(|m| m.extra.armor.iter()))
            .zip(
                ["helmet", "chestplate", "leggings", "boots"]
                    .into_iter()
                    .cycle(),
            )
            .chain(self.armor_stands.iter().flat_map(|a| {
                [5usize, 4, 3, 2]
                    .map(|slot| &a.equipment[slot])
                    .into_iter()
                    .zip(["helmet", "chestplate", "leggings", "boots"])
            }));
        for (stack, layer) in stacks {
            let Some(stack) = stack.as_ref().filter(|s| s.extra.is_some()) else {
                continue;
            };
            let Some(asset) = items
                .get(&stack.item)
                .and_then(|i| i.equip_asset.as_deref())
            else {
                continue;
            };
            let kind = if layer == "leggings" {
                "humanoid_leggings"
            } else {
                "humanoid"
            };
            if let Some(t) = assets.trim_texture(asset, kind, stack)
                && !wanted.contains(&t)
            {
                wanted.push(t);
            }
        }
        for t in wanted {
            let name = t.sprite();
            // Entries the atlas no longer has (rebuilt by a resource reload) are free.
            let sprites = &self.gui.atlas.sprites;
            self.trim_slots.retain(|(n, _)| sprites.contains_key(n));
            if let Some(i) = self.trim_slots.iter().position(|e| e.0 == name) {
                let e = self.trim_slots.remove(i);
                self.trim_slots.push(e);
                continue;
            }
            let Some((w, h, rgba)) = t.generate(&self.armor_stand_assets.root) else {
                continue;
            };
            let used: Vec<usize> = self.trim_slots.iter().map(|e| e.1).collect();
            let slot = match (0..TRIM_SLOTS).find(|i| !used.contains(i)) {
                Some(free) => free,
                None => {
                    let (old, slot) = self.trim_slots.remove(0);
                    self.gui.atlas.sprites.remove(&old);
                    slot
                }
            };
            let Some(area) = self
                .gui
                .atlas
                .sprites
                .get(&format!("horizon/trim{slot}"))
                .copied()
            else {
                continue;
            };
            if w > area.w || h > area.h {
                continue;
            }
            let (aw, ah) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
            let (x, y) = ((area.u0 * aw).round(), (area.v0 * ah).round());
            self.map_patches.push(horizon_client::font::Patch {
                x: x as u32,
                y: y as u32,
                width: w,
                height: h,
                rgba,
            });
            let rect = horizon_client::hud::Rect {
                u0: area.u0,
                v0: area.v0,
                u1: (x + w as f32) / aw,
                v1: (y + h as f32) / ah,
                w,
                h,
            };
            self.gui.atlas.sprites.insert(name.clone(), rect);
            self.trim_slots.push((name, slot));
        }
    }
    /// `ArmorStandItem.useOn`.
    /// `ArmorStandItem.useOn` up to its `FAIL`s, which a client checks too:
    /// not on a bottom face, and nothing in the new stand's box. The block it
    /// stands in and the stand.
    pub(super) fn armor_stand_spawn(&self, hit: &super::Hit) -> Option<([i32; 3], ArmorStand)> {
        if hit.face == [0, -1, 0] {
            return None;
        }
        // BlockPlaceContext: a replaceable clicked block is replaced.
        let clicked = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        let pos = if clicked.replaceable {
            hit.pos
        } else {
            std::array::from_fn(|i| hit.pos[i] + hit.face[i])
        };
        let foot = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let yaw =
            (((self.yaw - 180.0 + 180.0).rem_euclid(360.0) - 180.0 + 22.5) / 45.0).floor() * 45.0;
        let stand = ArmorStand::new(self.next_entity + 1, foot, yaw);
        let b = stand.bounds();
        // Touching the floor is not a collision.
        let (lo, hi) = (b.min_position(), b.max_position());
        let e = 1.0e-7;
        let bounds = Aabb::new(
            Vec3::new(lo.x + e, lo.y + e, lo.z + e),
            Vec3::new(hi.x - e, hi.y - e, hi.z - e),
        );
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false)
            || self
                .armor_stands
                .iter()
                .any(|o| overlaps(o.bounds(), bounds))
        {
            return None;
        }
        Some((pos, stand))
    }
    pub(super) fn place_armor_stand(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:armor_stand" {
            return false;
        }
        let Some((pos, stand)) = self.armor_stand_spawn(hit) else {
            return false;
        };
        let foot = stand.position;
        self.next_entity += 1;
        self.audio.play(
            "minecraft:entity.armor_stand.place",
            Some([foot.x, foot.y, foot.z]),
            0.75,
            0.8,
        );
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.armor_stands.push(stand);
        if !self.creative {
            self.consume_held();
        }
        // ArmorStandItem.useOn returns SUCCESS: the hand swings.
        self.swing_arm();
        true
    }
    /// The armor stand under the crosshair, its distance and the hit point.
    pub(super) fn armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, a) in self.armor_stands.iter().enumerate() {
            if a.marker {
                continue;
            }
            let b = a.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for k in 0..3 {
                if dir[k].abs() < 1e-9 {
                    hit &= eye[k] >= lo[k] && eye[k] <= hi[k];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[k] - eye[k]) / dir[k], (hi[k] - eye[k]) / dir[k]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0, std::array::from_fn(|k| eye[k] + dir[k] * t0)));
            }
        }
        best
    }
    fn equipment_slot(&self, stack: &ItemStack) -> usize {
        match self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.equip_slot.as_deref())
        {
            Some("feet") => 2,
            Some("legs") => 3,
            Some("chest") => 4,
            Some("head") => 5,
            Some("offhand") => 1,
            _ => 0,
        }
    }
    /// `ArmorStand.interactAt`.
    pub(super) fn interact_armor_stand(&mut self) -> bool {
        let Some((i, d, point)) = self.armor_stand_hit() else {
            return false;
        };
        if !self.before_block(d) {
            return false;
        }
        let held = self.held().cloned();
        if held
            .as_ref()
            .is_some_and(|h| &*h.item == "minecraft:name_tag")
        {
            return false;
        }
        let a = &self.armor_stands[i];
        let slot = match &held {
            Some(h) => {
                let slot = self.equipment_slot(h);
                if slot < 2 && !a.show_arms {
                    return true;
                }
                slot
            }
            None => {
                // getClickedSlot by the height of the click.
                let y = (point[1] - a.position.y) / if a.small { 0.5 } else { 1.0 };
                let has = |s: usize| a.equipment[s].is_some();
                let small = a.small;
                if y >= 0.1 && y < 0.1 + if small { 0.8 } else { 0.45 } && has(2) {
                    2
                } else if y >= 0.9 + if small { 0.3 } else { 0.0 }
                    && y < 0.9 + if small { 1.0 } else { 0.7 }
                    && has(4)
                {
                    4
                } else if y >= 0.4 && y < 0.4 + if small { 1.0 } else { 0.8 } && has(3) {
                    3
                } else if y >= 1.6 && has(5) {
                    5
                } else if !has(0) && has(1) {
                    1
                } else {
                    0
                }
            }
        };
        let current = self.armor_stands[i].equipment[slot].clone();
        if held.is_none() && current.is_none() {
            return false;
        }
        // ArmorStand.swapItem.
        match held {
            // A player with infinite materials puts a copy of one on an empty
            // slot and keeps the stack.
            Some(h) if self.creative && current.is_none() => {
                self.armor_stands[i].equipment[slot] = Some(ItemStack { count: 1, ..h });
            }
            Some(h) if h.count > 1 => {
                if current.is_some() {
                    return true;
                }
                self.armor_stands[i].equipment[slot] = Some(ItemStack {
                    count: 1,
                    ..h.clone()
                });
                *self.held_mut() = Some(ItemStack {
                    count: h.count - 1,
                    ..h
                });
            }
            h => {
                self.armor_stands[i].equipment[slot] = h;
                *self.held_mut() = current;
            }
        }
        let p = self.armor_stands[i].position;
        self.world
            .modified
            .insert(((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4));
        self.swing_arm();
        true
    }
    /// `ArmorStand.hurtServer` from a player attack.
    pub(super) fn attack_armor_stand(&mut self) -> bool {
        let Some((i, d, _)) = self.armor_stand_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // The stand's box and id, before the hit may remove it.
        let (area, id) = (self.armor_stands[i].bounds(), self.armor_stands[i].id);
        let (_, _, _, magic) = self.melee_damage("minecraft:armor_stand", true);
        let hurt = self.punch_armor_stand(i);
        if hurt {
            self.attack_effects(
                area,
                Some(id as i32),
                super::mace::Spared::Stand(id),
                magic.then_some(id as i32),
            );
            self.swing_arm();
        }
        hurt
    }
    /// `ArmorStand.hurtServer` for a player's attack (also a sweep's): a
    /// creative player breaks it, otherwise the first hit wobbles it and a
    /// second within five ticks breaks it.
    pub(super) fn punch_armor_stand(&mut self, i: usize) -> bool {
        let time = self.world.game_time;
        let at = self.armor_stands[i].position;
        let at = [at.x, at.y, at.z];
        if self.creative {
            self.armor_stands.remove(i);
            self.audio
                .play("minecraft:entity.armor_stand.break", Some(at), 1.0, 1.0);
            return true;
        }
        if time - self.armor_stands[i].last_hit > 5 {
            // First hit: entity event 32, the wobble and the hit sound.
            self.armor_stands[i].last_hit = time;
            self.audio
                .play("minecraft:entity.armor_stand.hit", Some(at), 0.3, 1.0);
            return true;
        }
        // brokenByPlayer: the stand item, then brokenByAnything: the equipment.
        let a = self.armor_stands.remove(i);
        self.audio
            .play("minecraft:entity.armor_stand.break", Some(at), 1.0, 1.0);
        let center = [at[0], at[1] + if a.small { 0.4875 } else { 0.9875 }, at[2]];
        let stand = ItemStack {
            item: Arc::from("minecraft:armor_stand"),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(stand, center, [0.0; 3], 10);
        for stack in a.equipment.into_iter().flatten() {
            let r = &mut self.world.random;
            let v = [
                f64::from(r.next_float()) * 0.2 - 0.1,
                0.2,
                f64::from(r.next_float()) * 0.2 - 0.1,
            ];
            self.spawn_item(stack, center, v, 10);
        }
        self.world
            .modified
            .insert(((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4));
        true
    }
    pub(super) fn tick_armor_stands(&mut self) {
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let mut dead = Vec::new();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        for (i, a) in self.armor_stands.iter_mut().enumerate() {
            if !ticking(a.position.x.floor() as i32, a.position.z.floor() as i32) {
                continue;
            }
            let in_water = a.in_water(&self.world, &self.res.motion().fluids);
            let (world, registry, facts, collision) = (
                &self.world,
                self.pack.assets.registry,
                self.res.facts(),
                self.collision,
            );
            a.tick(&mut scene, in_water, &mut |bounds, velocity| {
                horizon_client::motion::bubble_column_push(
                    world, registry, facts, collision, bounds, velocity,
                );
            });
            if !a.environment_tick(&self.world, self.res.facts(), self.pack.assets.registry) {
                dead.push(i);
            }
        }
        for i in dead.into_iter().rev() {
            let a = self.armor_stands.remove(i);
            let at = [a.position.x, a.position.y, a.position.z];
            let (pos, drops) = burnt_armor_stand_equipment(a);
            for stack in drops {
                self.pop_resource(pos, stack);
            }
            self.audio
                .play("minecraft:entity.armor_stand.break", Some(at), 1.0, 1.0);
            self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        }
    }
    /// ArmorStandRenderer: the stand with its default pose, then armor layers.
    pub(super) fn armor_stand_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let assets = &self.armor_stand_assets;
        let sprites = &self.gui.atlas.sprites;
        let uv_of = |name: &str| sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let Some(wood) = uv_of("entity/armorstand/armorstand") else {
            return;
        };
        for a in &self.armor_stands {
            let entity_start = out.len();
            // ArmorStand.shouldRenderAtSqrDistance: 256 times the box's mean
            // edge (4 for a marker's empty box), whatever the entity distance.
            // LivingEntityRenderer grows the culling box by 0.5 for a dragon head.
            let (width, height) = match (a.marker, a.small) {
                (true, _) => (0.0, 0.0),
                (false, true) => (0.25, 0.9875),
                (false, false) => (0.5, 1.975),
            };
            let edge = (width + height + width) / 3.0 * 4.0;
            // 26.3 compares the size with RenderSystem.DEFAULT_DEPTH_CLEAR_VALUE,
            // which is 0.0 (reverse Z; the bytecode is `dconst_0`): an empty
            // marker box falls back to 4, so a visible marker stand draws.
            // Holograms stay hidden through the invisible flag instead.
            let range = if edge == 0.0 { 4.0 } else { edge } * 64.0;
            let dragon = a.equipment[5]
                .as_ref()
                .is_some_and(|i| &*i.item == "minecraft:dragon_head");
            let position = super::entity_culling::at(a.position);
            if !self.standing_entity_in_view(
                camera,
                position,
                position,
                alpha,
                width,
                height,
                range,
                if dragon { 0.5 } else { 0.0 },
            ) {
                continue;
            }
            let Some(model) = (if a.small { &assets.small } else { &assets.main }) else {
                continue;
            };
            let p = glam::DVec3::new(a.position.x, a.position.y, a.position.z) - camera;
            // setupRotations: 180 - body yaw, plus the wobble for 5 ticks after a hit.
            let wiggle = (self.world.game_time - a.last_hit) as f32 + alpha as f32;
            let wobble = if wiggle < 5.0 {
                (wiggle / 1.5 * std::f32::consts::PI).sin() * 3.0
            } else {
                0.0
            };
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((180.0 - a.yaw + wobble).to_radians())
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
            let mut poses = Poses::new(model);
            // ArmorStandModel.setupAnim: the stand's pose and part visibility.
            apply_pose(&mut poses, &a.pose);
            // Item layers pose from the hidden arms too (`visible` only skips drawing).
            let posed = poses.clone();
            let mut hide = |name: &str| {
                if poses.has(name) {
                    let pose = poses.get(name);
                    pose[6] = 0.0;
                    pose[7] = 0.0;
                    pose[8] = 0.0;
                }
            };
            if !a.show_arms {
                hide("left_arm");
                hide("right_arm");
            }
            if a.no_base_plate {
                hide("base_plate");
            }
            let light = self.world.light(
                a.position.x.floor() as i32,
                (a.position.y + 0.5).floor() as i32,
                a.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // LivingEntityRenderer.submit: an invisible stand has no body render type, but its
            // layers (held items, custom head, armour) are submitted all the same.
            if !a.invisible {
                horizon_client::mobs::render::draw(
                    model,
                    base,
                    &poses,
                    wood,
                    0xFFFF_FFFF,
                    packed,
                    0,
                    out,
                );
            }
            self.armor_stand_items(a, model, base, &posed, packed, out);
            // HumanoidArmorLayer: head, chest, legs, feet.
            for (slot, layer) in [
                (5usize, "helmet"),
                (4, "chestplate"),
                (3, "leggings"),
                (2, "boots"),
            ] {
                let Some(stack) = &a.equipment[slot] else {
                    continue;
                };
                let Some(asset) = self
                    .res
                    .items()
                    .get(&stack.item)
                    .and_then(|i| i.equip_asset.clone())
                else {
                    continue;
                };
                let kind = if layer == "leggings" {
                    "humanoid_leggings"
                } else {
                    "humanoid"
                };
                let Some((texture, undyed)) =
                    assets.equipment.get(&asset).and_then(|k| k.get(kind))
                else {
                    continue;
                };
                let (Some(uv), Some(part)) = (uv_of(texture), assets.layers.get(&(layer, a.small)))
                else {
                    continue;
                };
                let color = layer_color(*undyed, Some(stack));
                // ArmorStandRenderer's HumanoidArmorLayer: the armour glint
                // when `stack.hasFoil()`, as on a living entity.
                let item_glint = self
                    .res
                    .items()
                    .enchantments
                    .props
                    .get(&stack.item)
                    .is_some_and(|p| p.glint);
                let overlay = if stack.has_foil(item_glint) {
                    horizon_client::mobs::render::ARMOR_GLINT
                } else {
                    0
                };
                // ArmorStandArmorModel.setupAnim: the same pose as the stand.
                let mut layer_poses = Poses::new(part);
                apply_pose(&mut layer_poses, &a.pose);
                horizon_client::mobs::render::draw(
                    part,
                    base,
                    &layer_poses,
                    uv,
                    color,
                    packed,
                    overlay,
                    out,
                );
                // EquipmentLayerRenderer: the trim layer over the armour.
                if let Some(trim) = assets
                    .trim_texture(&asset, kind, stack)
                    .and_then(|t| uv_of(&t.sprite()))
                {
                    horizon_client::mobs::render::draw(
                        part,
                        base,
                        &layer_poses,
                        trim,
                        0xFFFF_FFFF,
                        packed,
                        overlay,
                        out,
                    );
                }
            }
            // WingsLayer: ArmorStandRenderer adds it after the armour layers. A stand
            // never crouches or glides, so its wings rest folded.
            self.wings_geometry(
                a.equipment[4].as_ref().map(|s| &*s.item),
                None,
                a.small,
                base,
                horizon_client::mobs::ElytraAnimation::RESTING,
                false,
                packed,
                self.wings_overlay(a.equipment[4].as_ref()),
                out,
            );
            self.cull_entity_vertices(out, entity_start);
        }
    }
}
/// The `Pose` rotations on the parts of an armor stand model or its armor
/// (`ArmorStandArmorModel.setupAnim`); the body also turns the sticks.
fn apply_pose(poses: &mut Poses, pose: &[[f32; 3]; 6]) {
    let parts: [&[&str]; 6] = [
        &["head"],
        &[
            "body",
            "right_body_stick",
            "left_body_stick",
            "shoulder_stick",
        ],
        &["left_arm"],
        &["right_arm"],
        &["left_leg"],
        &["right_leg"],
    ];
    for (names, rotation) in parts.into_iter().zip(pose) {
        for name in names {
            if poses.has(name) {
                let part = poses.get(name);
                part[3] = rotation[0].to_radians();
                part[4] = rotation[1].to_radians();
                part[5] = rotation[2].to_radians();
            }
        }
    }
}
fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
impl Game<'_> {
    /// `Equippable.swapWithEquipmentSlot`: right-clicking armor swaps it into
    /// its slot (the worn piece comes back to the hand).
    pub(super) fn equip_from_hand(&mut self, stack: &ItemStack) -> bool {
        let Some(info) = self.items().get(&stack.item) else {
            return false;
        };
        let (index, slot) = match info.equip_slot.as_deref() {
            Some("head") => (0, "head"),
            Some("chest") => (1, "chest"),
            Some("legs") => (2, "legs"),
            Some("feet") => (3, "feet"),
            _ => return false,
        };
        if !info.swappable {
            return false;
        }
        let current = self.armor[index].clone();
        // PREVENT_ARMOR_CHANGE (Curse of Binding) outside creative, or the same item.
        if current.as_ref().is_some_and(|c| {
            (c.enchantment_level("minecraft:binding_curse") > 0 && !self.creative)
                || c.same_item_same_components(stack)
        }) {
            return true;
        }
        let equipped = ItemStack {
            count: 1,
            ..stack.clone()
        };
        self.armor[index] = Some(equipped.clone());
        // Player.setItemSlot: onEquipItem plays the equip sound.
        self.player_equipped(slot, Some(&equipped));
        if !self.creative {
            let mut held = stack.clone();
            held.count -= 1;
            if held.count == 0 {
                // The worn piece comes back to the hand.
                *self.held_mut() = current;
            } else {
                *self.held_mut() = Some(held);
                if let Some(old) = current
                    && let Some(rest) = self.give(old)
                    && let Some(eye) = self.eye_position()
                {
                    self.spawn_item(rest, eye, [0.0; 3], 40);
                }
            }
        }
        self.apply_equipment();
        self.swing_arm();
        true
    }
}
impl Game<'_> {
    /// `ItemInHandLayer` and `CustomHeadLayer` of an armour stand: the hand
    /// items in the third-person contexts, and a non-armour head item on the
    /// head (`translateToHead`: down a quarter, turned, 0.625 scale).
    fn armor_stand_items(
        &self,
        a: &ArmorStand,
        model: &Part,
        base: glam::Mat4,
        poses: &Poses,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use horizon_assets::item_model::DisplayContext;
        use horizon_client::mobs::render::part_matrix;
        let owner = super::item_draw::owner_at(
            [a.position.x, a.position.y, a.position.z],
            a.yaw,
            "minecraft:armor_stand",
        );
        // `useBabyOffset`: small stands use the baby hand offsets.
        let (ox, oy, oz) = if a.small {
            (0.0, 1.0, -4.5)
        } else {
            (1.0, 2.0, -10.0)
        };
        for (slot, left, context) in [
            (0usize, false, DisplayContext::ThirdPersonRightHand),
            (1, true, DisplayContext::ThirdPersonLeftHand),
        ] {
            let Some(stack) = &a.equipment[slot] else {
                continue;
            };
            let arm = if left { "left_arm" } else { "right_arm" };
            let Some(hand) = part_matrix(model, base, poses, arm, 0.0) else {
                continue;
            };
            let pose = hand
                * glam::Mat4::from_rotation_x((-90.0f32).to_radians())
                * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                * glam::Mat4::from_translation(glam::Vec3::new(
                    if left { -ox } else { ox } / 16.0,
                    oy / 16.0,
                    oz / 16.0,
                ));
            let seed = a.id as i32 + if left { 1 } else { 2 };
            let state = self.resolve_item(stack, context, Some(&owner), seed);
            self.submit_item_world(&state, pose, light, out);
        }
        // `CustomHeadLayer`: a worn skull draws the skull model.
        if let Some(stack) = &a.equipment[5]
            && let Some(kind) = skull_kind(&stack.item)
            && let Some(head) = part_matrix(model, base, poses, "head", 0.0)
        {
            let pose = head * glam::Mat4::from_scale(glam::Vec3::splat(1.1875));
            let special = horizon_assets::item_model::SpecialModel {
                kind: if kind == "player" {
                    horizon_assets::item_model::SpecialKind::PlayerHead
                } else {
                    horizon_assets::item_model::SpecialKind::Head {
                        kind: kind.to_owned(),
                        texture: None,
                        animation: 0.0,
                    }
                },
            };
            let mut sink = horizon_client::item_render::WorldSink {
                out,
                light,
                lights: Some(horizon_client::block_entity_render::diffuse_lights(
                    self.dim == super::Dim::Nether,
                )),
                glint: self.glint(),
            };
            let profile = stack
                .extra
                .as_deref()
                .and_then(|d| d.other.iter().find(|(k, _)| k == "minecraft:profile"))
                .map(|(_, t)| horizon_client::scoreboard::tag_to_json(t))
                .unwrap_or(serde_json::Value::Null);
            self.pack.item_renderer.submit_special(
                &special,
                &profile,
                false,
                pose,
                &self.gui.atlas.sprites,
                &self.item_host(),
                &mut sink,
            );
            return;
        }
        // A head item that is not worn armour.
        if let Some(stack) = &a.equipment[5]
            && self
                .res
                .items()
                .get(&stack.item)
                .is_none_or(|i| i.equip_asset.is_none())
            && let Some(head) = part_matrix(model, base, poses, "head", 0.0)
        {
            let pose = head
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0))
                * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                * glam::Mat4::from_scale(glam::Vec3::new(0.625, -0.625, -0.625));
            // `updateForLiving`: seeded by the entity id plus HEAD's ordinal.
            let state =
                self.resolve_item(stack, DisplayContext::Head, Some(&owner), a.id as i32 + 5);
            self.submit_item_world(&state, pose, light, out);
        }
    }
}
/// `AbstractSkullBlock.getType` of a skull item.
pub(super) fn skull_kind(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:skeleton_skull" => "skeleton",
        "minecraft:wither_skeleton_skull" => "wither_skeleton",
        "minecraft:zombie_head" => "zombie",
        "minecraft:creeper_head" => "creeper",
        "minecraft:dragon_head" => "dragon",
        "minecraft:piglin_head" => "piglin",
        "minecraft:player_head" => "player",
        _ => return None,
    })
}

fn burnt_armor_stand_equipment(a: ArmorStand) -> ([i32; 3], Vec<ItemStack>) {
    let pos = [
        a.position.x.floor() as i32,
        a.position.y.floor() as i32 + 1,
        a.position.z.floor() as i32,
    ];
    let drops = a
        .equipment
        .into_iter()
        .flatten()
        .filter(|stack| stack.enchantment_level("minecraft:vanishing_curse") == 0)
        .collect();
    (pos, drops)
}
#[cfg(test)]
mod burn_drop_tests {
    use super::*;
    #[test]
    fn burnt_equipment_drops_above_the_block_and_vanishing_items_do_not_drop() {
        let mut stand = ArmorStand::new(1, Vec3::new(-0.1, 4.8, 0.0), 0.0);
        let kept = ItemStack::new("minecraft:diamond_boots", 1);
        let mut vanished = ItemStack::new("minecraft:diamond_helmet", 1);
        vanished
            .data_mut()
            .enchantments
            .push(("minecraft:vanishing_curse".into(), 1));
        stand.equipment[0] = Some(kept.clone());
        stand.equipment[1] = Some(vanished);
        let (pos, drops) = burnt_armor_stand_equipment(stand);
        assert_eq!(pos, [-1, 5, 0]);
        assert_eq!(drops, vec![kept]);
    }
}
#[cfg(test)]
mod layer_color_tests {
    use super::*;
    #[test]
    fn a_dyed_stack_overrides_the_undyed_leather_colour() {
        let mut dyed = ItemStack::new("minecraft:leather_helmet", 1);
        dyed.data_mut().dyed_color = Some(0x1E_90FF);
        // Dyed blue, R and B swapped into the renderer's order, opaque.
        assert_eq!(layer_color(Some(0xA06540), Some(&dyed)), 0xFFFF_901E);
        // An undyed stack keeps the asset's brown.
        let plain = ItemStack::new("minecraft:leather_helmet", 1);
        assert_eq!(layer_color(Some(0xA06540), Some(&plain)), 0xFF40_65A0);
        // No dye and no undyed colour: white.
        assert_eq!(layer_color(None, Some(&plain)), 0xFFFF_FFFF);
        // A layer that isn't dyeable stays white even on a dyed stack.
        assert_eq!(layer_color(None, Some(&dyed)), 0xFFFF_FFFF);
    }
}
