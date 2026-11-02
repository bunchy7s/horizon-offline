//! The horse inventory (`HorseInventoryMenu`, `HorseInventoryScreen`): the
//! saddle and body armour slots (a llama's carpet, a nautilus's armour), the
//! chest of donkeys, mules and llamas and the mount drawn following the
//! cursor. A nautilus has the same two slots and no chest
//! (`NautilusInventoryMenu`). It opens with E while riding a tamed mount or
//! with shift + use on a tamed one. On a
//! server the screen shows the server's menu (`ClientboundMountScreenOpenPacket`)
//! and its clicks go out as container clicks; a server's players get theirs
//! from game/lan_mount_menu.rs.
use super::{Game, Screen};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
    mobs::{Mob, Species, equine, llama, nautilus},
};

/// Menu slot ids: the saddle, the body armour and the chest from 710.
pub(super) const SADDLE: usize = 700;
pub(super) const ARMOR: usize = 701;
pub(super) const CHEST: usize = 710;
/// `AbstractChestedHorse.INV_CHEST_COUNT` and `getInventoryColumns`.
const CHEST_SIZE: usize = 15;
const COLUMNS: usize = 5;

/// `AbstractHorse.getInventoryColumns`: five for a donkey's or mule's chest,
/// a llama's strength for its chest, else none.
pub(super) fn mount_columns(m: &Mob) -> usize {
    let species = m.species();
    if llama::is_llama(species) {
        llama::inventory_columns(m).clamp(0, 5) as usize
    } else if m.extra.chested && matches!(species, Species::Donkey | Species::Mule) {
        COLUMNS
    } else {
        0
    }
}
/// Whether the mount has a horse inventory (`AbstractHorse`, `AbstractNautilus`).
pub(super) fn has_mount_menu(species: Species) -> bool {
    equine::is_equine(species) || llama::is_llama(species) || nautilus::is_nautilus(species)
}
/// The saddle slot is active (`canUseSlot(SADDLE)` and `CAN_EQUIP_SADDLE`):
/// not on llamas.
fn shows_saddle(species: Species) -> bool {
    !llama::is_llama(species)
}
/// The body slot is active: horse armour, a llama's carpet, or nautilus armour.
fn shows_body(species: Species) -> bool {
    can_wear_horse_armor(species) || llama::is_llama(species) || nautilus::is_nautilus(species)
}

/// The open mount menu: copies of the mount's slots, written back on every change.
#[derive(Default)]
pub(super) struct MountMenu {
    open: Option<Open>,
}
impl MountMenu {
    /// The mount whose menu is open.
    pub(super) fn opened_mob(&self) -> Option<u32> {
        self.open.as_ref().map(|o| o.mob)
    }
}
struct Open {
    mob: u32,
    species: Species,
    columns: usize,
    saddle: Option<ItemStack>,
    armor: Option<ItemStack>,
    items: Vec<Option<ItemStack>>,
    /// `shouldRenderSaddleSlot` / `shouldRenderArmorSlot`.
    show_saddle: bool,
    show_armor: bool,
}

/// `EntityTypeTags.CAN_WEAR_HORSE_ARMOR`.
fn can_wear_horse_armor(species: Species) -> bool {
    matches!(species, Species::Horse | Species::ZombieHorse)
}
/// Whether `item` fits the body slot of the species (`isEquippableInSlot`).
fn body_armor_fits(species: Species, item: &str) -> bool {
    if llama::is_llama(species) {
        return item.ends_with("_carpet");
    }
    // EntityTypeTags.CAN_WEAR_NAUTILUS_ARMOR: the nautilus armours.
    if nautilus::is_nautilus(species) {
        return item.ends_with("_nautilus_armor");
    }
    can_wear_horse_armor(species) && item.ends_with("_horse_armor")
}

impl Game<'_> {
    /// Whether the mount menu is the open screen.
    pub(super) fn mount_active(&self) -> bool {
        self.screen == Screen::Inventory && self.mount_menu.open.is_some()
    }
    fn mount_mob_index(&self, id: u32) -> Option<usize> {
        self.mobs.iter().position(|m| m.id == id)
    }
    /// `AbstractHorse.openCustomInventoryScreen` for a tamed adult mount.
    pub(super) fn open_mount_menu(&mut self, index: usize) {
        let m = &self.mobs[index];
        let species = m.species();
        if !has_mount_menu(species) {
            return;
        }
        let mut items = m.extra.items.clone();
        items.resize(CHEST_SIZE, None);
        let show_armor = shows_body(species);
        self.mount_menu.open = Some(Open {
            mob: m.id,
            species,
            columns: mount_columns(m),
            saddle: m.saddle_item(),
            armor: m
                .extra
                .head
                .as_deref()
                .filter(|_| show_armor)
                .map(|i| ItemStack::new(i, 1)),
            items,
            show_saddle: shows_saddle(species),
            show_armor,
        });
        self.show_mount_menu();
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a server's horse
    /// inventory over mount `entity`, its slots to come in the menu's content.
    pub(super) fn open_remote_mount_menu(&mut self, entity: u32, columns: usize) -> bool {
        let Some(species) = self
            .mobs
            .iter()
            .find(|m| m.id == entity)
            .map(Mob::species)
            .filter(|s| has_mount_menu(*s))
        else {
            return false;
        };
        self.mount_menu.open = Some(Open {
            mob: entity,
            species,
            columns: columns.min(COLUMNS),
            saddle: None,
            armor: None,
            items: vec![None; CHEST_SIZE],
            show_saddle: shows_saddle(species),
            show_armor: shows_body(species),
        });
        self.show_mount_menu();
        true
    }
    fn show_mount_menu(&mut self) {
        if self.remote.is_none() && self.lan_acting.is_none() {
            self.carried = None;
        }
        self.station = None;
        self.container = None;
        self.craft_size = 2;
        self.screen = Screen::Inventory;
        self.capture(false);
    }
    /// E while riding a tamed mount opens its inventory instead of the player's.
    pub(super) fn open_ridden_mount_menu(&mut self) -> bool {
        let Some(id) = self.riding else {
            return false;
        };
        let Some(index) = self.mount_mob_index(id) else {
            return false;
        };
        let m = &self.mobs[index];
        if !has_mount_menu(m.species()) || !m.extra.tame || !m.alive() {
            return false;
        }
        self.open_mount_menu(index);
        true
    }
    /// Riding a mount that has a horse inventory (`HasCustomInventoryScreen`).
    pub(super) fn riding_mount_with_menu(&self) -> bool {
        self.riding
            .and_then(|id| self.mount_mob_index(id))
            .is_some_and(|i| has_mount_menu(self.mobs[i].species()))
    }
    /// A server closed its window: the screen's copy goes.
    pub(super) fn mount_menu_reset(&mut self) {
        self.mount_menu.open = None;
    }
    /// The entity type of the open mount menu's mount (`minecraft:horse`).
    pub(super) fn mount_menu_entity(&self) -> Option<String> {
        let open = self.mount_menu.open.as_ref()?;
        let m = self.mobs.iter().find(|m| m.id == open.mob)?;
        Some(m.kind.id.to_string())
    }
    /// The chest columns of the open mount menu (the server's window's).
    pub(super) fn mount_menu_columns(&self) -> Option<usize> {
        self.mount_menu.open.as_ref().map(|o| o.columns)
    }
    /// The menu ends when the mount dies or is more than 4 blocks away
    /// (`AbstractMountInventoryMenu.stillValid`).
    pub(super) fn tick_mount_menu(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() {
            return;
        }
        let Some(open) = self.mount_menu.open.as_ref() else {
            return;
        };
        let valid = self.mount_mob_index(open.mob).is_some_and(|i| {
            let m = &self.mobs[i];
            let c = m.center();
            m.alive()
                && self.eye_position().is_some_and(|e| {
                    let d = (e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2) + (e[2] - c[2]).powi(2);
                    d <= 16.0 + 4.0
                })
        });
        if !valid {
            self.close_container();
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// Closing the screen keeps what is in the slots on the mount.
    pub(super) fn mount_closed(&mut self) {
        self.mount_sync();
        self.mount_menu.open = None;
    }
    /// Writes the menu slots to the mount, with the equip sounds. A
    /// server's menu is the server's to write.
    fn mount_sync(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let Some(open) = self.mount_menu.open.as_ref() else {
            return;
        };
        let Some(index) = self.mount_mob_index(open.mob) else {
            return;
        };
        let (saddle, armor) = (open.saddle.clone(), open.armor.clone());
        let body = open.show_armor.then_some(armor);
        let items = open.items.clone();
        self.mount_store(index, saddle, body, items);
    }
    /// The slots of mount `index` as its menu shows them: the saddle, the
    /// body item (None when the slot isn't active), the chest.
    pub(super) fn mount_contents(
        &self,
        index: usize,
    ) -> (
        Option<ItemStack>,
        Option<Option<ItemStack>>,
        Vec<Option<ItemStack>>,
    ) {
        let m = &self.mobs[index];
        let species = m.species();
        let body =
            shows_body(species).then(|| m.extra.head.as_deref().map(|i| ItemStack::new(i, 1)));
        let mut items = m.extra.items.clone();
        items.resize(mount_columns(m) * 3, None);
        (m.saddle_item(), body, items)
    }
    /// The menu's slots back on mount `index` (`body` None: not its slot),
    /// with the equip sound of what was put on (`Equippable.equipSound`).
    pub(super) fn mount_store(
        &mut self,
        index: usize,
        saddle: Option<ItemStack>,
        body: Option<Option<ItemStack>>,
        items: Vec<Option<ItemStack>>,
    ) {
        let m = &self.mobs[index];
        let at = m.center();
        let saddle_put_on = saddle.is_some() && !m.extra.saddled;
        let put_on = if saddle_put_on {
            saddle.as_ref()
        } else {
            body.as_ref()
                .and_then(Option::as_ref)
                .filter(|_| m.extra.head.is_none())
        };
        let sound = if saddle_put_on && nautilus::is_nautilus(m.species()) {
            // AbstractNautilus.getEquipSound: its own saddle sounds.
            Some(format!("minecraft:{}", nautilus::saddle_sound(m)))
        } else {
            put_on.map(|s| {
                self.res
                    .items()
                    .get(&s.item)
                    .and_then(|i| i.equip_sound.clone())
                    .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into())
            })
        };
        let m = &mut self.mobs[index];
        m.set_saddle(saddle);
        if let Some(body) = body {
            m.extra.head = body.map(|a| a.item);
        }
        let mut items = items;
        let keep = m.extra.items.len().max(CHEST_SIZE);
        items.resize(keep, None);
        m.extra.items = items;
        let (x, z) = (at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4);
        self.world.modified.insert((x, z));
        if let Some(sound) = sound {
            self.audio.play(&sound, Some(at), 0.5, 1.0);
        }
    }
    pub(super) fn mount_slots(&self) -> Option<Vec<usize>> {
        let open = self
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        let mut v: Vec<usize> = (0..36).collect();
        if open.show_saddle {
            v.push(SADDLE);
        }
        if open.show_armor {
            v.push(ARMOR);
        }
        v.extend((0..open.columns * 3).map(|i| CHEST + i));
        Some(v)
    }
    /// How many slots `mount_slots` lists, counted without building it
    /// (`Game::slot_count` asks every frame).
    pub(super) fn mount_slot_count(&self) -> Option<usize> {
        let open = self
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        Some(36 + usize::from(open.show_saddle) + usize::from(open.show_armor) + open.columns * 3)
    }
    pub(super) fn mount_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let open = self
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        match i {
            SADDLE => Some((8.0, 18.0)),
            ARMOR => Some((8.0, 36.0)),
            _ if i >= CHEST => {
                let n = i - CHEST;
                (n < open.columns * 3).then(|| {
                    (
                        80.0 + (n % open.columns) as f32 * 18.0,
                        18.0 + (n / open.columns) as f32 * 18.0,
                    )
                })
            }
            _ => None,
        }
    }
    pub(super) fn mount_slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        match self.mount_menu.open.as_mut() {
            Some(open) if i == SADDLE => &mut open.saddle,
            Some(open) if i == ARMOR => &mut open.armor,
            Some(open) if i >= CHEST && i - CHEST < CHEST_SIZE => &mut open.items[i - CHEST],
            _ => &mut self.scratch_slot,
        }
    }
    /// `ArmorSlot.mayPlace` and the stack limits of the mount's slots.
    pub(super) fn mount_slot_rules(&self, slot: usize, stack: &ItemStack) -> (bool, u32) {
        let Some(open) = self.mount_menu.open.as_ref() else {
            return (false, 64);
        };
        match slot {
            SADDLE => (&*stack.item == "minecraft:saddle", 1),
            ARMOR => (body_armor_fits(open.species, &stack.item), 1),
            _ => (true, 64),
        }
    }
    /// A click in the mount menu before the generic handling.
    pub(super) fn mount_click(&mut self, right: bool) -> bool {
        // A server's menu: the click goes out like any container click.
        if !self.mount_active() || self.remote.is_some() {
            return false;
        }
        let Some(r) = self.renderer.as_ref() else {
            return true;
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return true;
        };
        let shift = self.has_shift_down();
        if shift {
            self.mount_quick_move(slot);
        } else if slot >= SADDLE {
            self.click_filtered(slot, right);
        } else {
            return false;
        }
        self.mount_sync();
        true
    }
    /// The inventory slot ids in menu order.
    fn mount_player_slots() -> Vec<usize> {
        (9..36).chain(0..9).collect()
    }
    /// `AbstractMountInventoryMenu.quickMoveStack`.
    fn mount_quick_move(&mut self, slot: usize) {
        let Some(open) = self.mount_menu.open.as_ref() else {
            return;
        };
        let (columns, show_saddle, show_armor) = (open.columns, open.show_saddle, open.show_armor);
        let Some(mut stack) = self.slot_mut(slot).take() else {
            return;
        };
        if slot >= SADDLE {
            let player = Self::mount_player_slots();
            self.move_to(&mut stack, &player, true);
        } else {
            let armor_free = self.mount_slot_mut(ARMOR).is_none();
            let saddle_free = self.mount_slot_mut(SADDLE).is_none();
            let armor_ok = show_armor && self.mount_slot_rules(ARMOR, &stack).0 && armor_free;
            let saddle_ok = show_saddle && self.mount_slot_rules(SADDLE, &stack).0 && saddle_free;
            if armor_ok {
                self.move_to(&mut stack, &[ARMOR], false);
            } else if saddle_ok {
                self.move_to(&mut stack, &[SADDLE], false);
            } else {
                let chest: Vec<usize> = (0..columns * 3).map(|i| CHEST + i).collect();
                let moved = columns > 0 && self.move_to(&mut stack, &chest, false);
                if !moved {
                    // Between the hotbar and the main inventory.
                    let other: Vec<usize> = if slot < 9 {
                        (9..36).collect()
                    } else {
                        (0..9).collect()
                    };
                    self.move_to(&mut stack, &other, false);
                }
            }
        }
        if stack.count > 0 {
            *self.slot_mut(slot) = Some(stack);
        }
    }
    /// `HorseInventoryScreen`'s background, slot frames and the mount.
    pub(super) fn draw_mount_background(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(open) = self.mount_menu.open.as_ref() else {
            return;
        };
        let atlas = &self.gui.atlas;
        // NautilusInventoryScreen draws its own background.
        let background = if nautilus::is_nautilus(open.species) {
            "gui/container/nautilus"
        } else {
            "gui/container/horse"
        };
        batch.sprite_region(atlas, background, [ox, oy, 176.0, 166.0], 256.0, 256.0);
        if open.columns > 0 {
            batch.sprite_region(
                atlas,
                "gui/sprites/container/horse/chest_slots",
                [ox + 79.0, oy + 17.0, open.columns as f32 * 18.0, 54.0],
                90.0,
                54.0,
            );
        }
        if open.show_saddle {
            batch.sprite(atlas, "gui/sprites/container/slot", ox + 7.0, oy + 17.0);
            if open.saddle.is_none() {
                batch.sprite(
                    atlas,
                    "gui/sprites/container/slot/saddle",
                    ox + 8.0,
                    oy + 18.0,
                );
            }
        }
        if open.show_armor {
            batch.sprite(atlas, "gui/sprites/container/slot", ox + 7.0, oy + 35.0);
            if open.armor.is_none() {
                let sprite = if llama::is_llama(open.species) {
                    "gui/sprites/container/slot/llama_armor"
                } else if nautilus::is_nautilus(open.species) {
                    "gui/sprites/container/slot/nautilus_armor_inventory"
                } else {
                    "gui/sprites/container/slot/horse_armor"
                };
                batch.sprite(atlas, sprite, ox + 8.0, oy + 36.0);
            }
        }
        let grey = rgba(64, 64, 64, 255);
        let title = match self.mobs.iter().find(|m| m.id == open.mob) {
            Some(m) => m.custom_name.clone().unwrap_or_else(|| {
                let key = format!(
                    "entity.minecraft.{}",
                    m.kind.id.trim_start_matches("minecraft:")
                );
                self.translate(&key, &key)
            }),
            None => String::new(),
        };
        super::screen_info::name_container(|| title.clone());
        batch.text(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        let inventory = self.translate("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
        if let Some(m) = self.mobs.iter().find(|m| m.id == open.mob) {
            self.draw_mob_preview(
                batch,
                m,
                ox + 26.0,
                oy + 18.0,
                ox + 78.0,
                oy + 70.0,
                17.0,
                0.25,
            );
        }
    }
    /// `extractEntityInInventoryFollowsMouse` for a mob: depth-sorted,
    /// back-face-culled quads turned toward the cursor.
    #[allow(clippy::too_many_arguments)]
    fn draw_mob_preview(
        &self,
        batch: &mut HudBatch,
        mob: &Mob,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        size: f32,
        offset_y: f32,
    ) {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let x_angle = ((cx - self.cursor.0) / 40.0).atan();
        let y_angle = ((cy - self.cursor.1) / 40.0).atan();
        let mut m = mob.clone();
        m.previous = m.position;
        m.body_yaw = 180.0 + x_angle * 20.0;
        m.previous_body_yaw = m.body_yaw;
        m.head_yaw = m.body_yaw + x_angle * 20.0;
        m.previous_head_yaw = m.head_yaw;
        m.head_pitch = -y_angle * 20.0;
        m.previous_head_pitch = m.head_pitch;
        m.hurt_time = 0;
        m.death_time = 0;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        // A screen preview: custom entity model animations see is_in_gui = 1.
        horizon_client::mobs::cem_anim::in_gui(|| {
            horizon_client::mobs::render::render_with(
                &m,
                1.0,
                camera,
                &lookup,
                240 | 240 << 8,
                self.res.mob_data(),
                &[],
                &mut verts,
            );
        });
        let height = m.kind.height as f32;
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, height / 2.0 + offset_y, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x((y_angle * 20.0).to_radians());
        let mut quads: Vec<(f32, [[f32; 2]; 4], [[f32; 2]; 4], u32)> = Vec::new();
        for q in verts.as_chunks::<4>().0 {
            let p: [glam::Vec3; 4] =
                std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.z >= 0.0 {
                continue;
            }
            let n = n.normalize_or_zero();
            let shade = (0.55 + 0.45 * -n.z - 0.15 * n.y).clamp(0.4, 1.0);
            let c = q[0].color;
            let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
            let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
            let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
            quads.push((
                depth,
                p.map(|v| [v.x, v.y]),
                std::array::from_fn(|i| q[i].uv),
                color,
            ));
        }
        quads.sort_by(|a, b| a.0.total_cmp(&b.0));
        let start = batch.vertices.len();
        for (_, p, uv, color) in quads {
            batch.quad(p, uv, color);
        }
        batch.clip_since(start, x0, y0, x1, y1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_horses_wear_horse_armour() {
        assert!(body_armor_fits(
            Species::Horse,
            "minecraft:iron_horse_armor"
        ));
        assert!(body_armor_fits(
            Species::ZombieHorse,
            "minecraft:diamond_horse_armor"
        ));
        assert!(!body_armor_fits(
            Species::Donkey,
            "minecraft:iron_horse_armor"
        ));
        assert!(!body_armor_fits(
            Species::Horse,
            "minecraft:iron_chestplate"
        ));
    }

    #[test]
    fn llamas_wear_carpets_and_no_saddle() {
        assert!(body_armor_fits(Species::Llama, "minecraft:red_carpet"));
        assert!(!body_armor_fits(
            Species::Llama,
            "minecraft:iron_horse_armor"
        ));
        assert!(shows_body(Species::TraderLlama));
        assert!(!shows_saddle(Species::Llama));
        assert!(shows_saddle(Species::Donkey) && !shows_body(Species::Donkey));
    }
}
