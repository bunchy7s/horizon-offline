//! Beacons (`BeaconBlockEntity`, `BeaconMenu`, `BeaconScreen`,
//! `BeaconRenderer`): the beam sections tinted by stained glass, the pyramid
//! levels every 80 ticks, the primary/secondary effects for players in
//! range, the activate/ambient/deactivate sounds, the 230x219 menu with its
//! power buttons and payment slot, and the rotating, scrolling beam.
use super::{
    Game,
    stations::{INPUT, Kind},
};
use horizon_client::{
    containers::{Beacon, BlockEntity},
    effects::EffectInstance,
    hud::{HudBatch, rgba},
    items::ItemStack,
    mesher::Vertex,
    mobs::render::{DYE, DYE_NAMES},
    test_instance_block::Status,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

/// The beacon screen is 54 px wider than the 176-px menus it shares an origin with.
pub(super) const SHIFT: f32 = -27.0;
pub(super) const HEIGHT: f32 = 219.0;
/// `BEACON_EFFECTS` in order, with the pyramid level each needs.
pub(super) const EFFECTS: [(&str, i32); 6] = [
    ("speed", 1),
    ("haste", 1),
    ("resistance", 2),
    ("jump_boost", 2),
    ("strength", 3),
    ("regeneration", 4),
];
const PAYMENT: [&str; 5] = [
    "minecraft:iron_ingot",
    "minecraft:gold_ingot",
    "minecraft:emerald",
    "minecraft:diamond",
    "minecraft:netherite_ingot",
];
/// `BeaconMenu.PaymentSlot` and the player inventory, relative to the 176 origin.
pub(super) fn slot_position(i: usize) -> Option<(f32, f32)> {
    Some(match i {
        0..9 => (36.0 + SHIFT + i as f32 * 18.0, 195.0),
        9..36 => (
            36.0 + SHIFT + ((i - 9) % 9) as f32 * 18.0,
            137.0 + ((i - 9) / 9) as f32 * 18.0,
        ),
        INPUT => (136.0 + SHIFT, 110.0),
        _ => return None,
    })
}
/// A power button: (effect index, x, y, primary) relative to the beacon's left edge.
fn buttons() -> Vec<(usize, f32, f32, bool)> {
    let mut out = Vec::new();
    let tiers: [&[usize]; 3] = [&[0, 1], &[2, 3], &[4]];
    for (tier, effects) in tiers.iter().enumerate() {
        let count = effects.len() as i32;
        let total = count * 22 + (count - 1) * 2;
        for (c, &e) in effects.iter().enumerate() {
            let x = 76 + c as i32 * 24 - total / 2;
            out.push((e, x as f32, 22.0 + tier as f32 * 25.0, true));
        }
    }
    // Tier 4: regeneration, then the upgrade (primary II) button.
    let total = 2 * 22 + 2;
    out.push((5, (167 - total / 2) as f32, 47.0, false));
    out
}
fn upgrade_button() -> (f32, f32) {
    let total = 2 * 22 + 2;
    ((167 + 24 - total / 2) as f32, 47.0)
}
fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
fn effect_id(name: &str) -> Option<&'static str> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    EFFECTS.iter().map(|(e, _)| *e).find(|e| *e == n)
}
/// `ARGB.average`.
fn average(a: u32, b: u32) -> u32 {
    let ch = |c: u32, s: u32| (c >> s) & 255;
    [16, 8, 0]
        .iter()
        .fold(0, |acc, &s| acc | ((ch(a, s) + ch(b, s)) / 2) << s)
}
impl Game<'_> {
    /// `TestInstanceBlockEntity.getBeamSections`: grey while running, green
    /// for a pass, red for a failed required test and orange for an optional
    /// one; each section is one block tall and the last runs to the sky.
    fn test_instance_beam(&self, e: &BlockEntity) -> Option<Vec<(u32, i32)>> {
        let BlockEntity::TestInstanceBlock(b) = e else {
            return None;
        };
        let color = match b.data.status {
            Status::Cleared => return None,
            Status::Running => 0x80_8080,
            Status::Finished if b.data.error_message.is_none() => 0x00_FF00,
            Status::Finished => {
                let required = b
                    .data
                    .test
                    .as_deref()
                    .and_then(|key| self.test_def(key))
                    .is_none_or(|t| t.required);
                if required { 0xFF_0000 } else { 0xFF_8000 }
            }
        };
        Some(vec![(color, 1)])
    }

    /// The beam color of a `BeaconBeamBlock` (stained glass and panes, the beacon).
    fn beam_color(&self, name: &str) -> Option<u32> {
        if name == "minecraft:beacon" {
            return Some(DYE[0]);
        }
        let n = name.strip_prefix("minecraft:")?;
        let dye = n
            .strip_suffix("_stained_glass")
            .or_else(|| n.strip_suffix("_stained_glass_pane"))?;
        DYE_NAMES.iter().position(|d| *d == dye).map(|i| DYE[i])
    }
    /// `getHeight(WORLD_SURFACE)`: above the highest non-air block.
    pub(super) fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world
            .world_surface(x, z, |id| self.res.facts().is_air(id))
    }
    /// `BeaconBlockEntity.tick` of the beacon at `key`.
    pub(super) fn tick_beacon_at(&mut self, key: (i32, i32, i32)) {
        let Some(BlockEntity::Beacon(mut b)) = self.world.block_entities.take(&key) else {
            return;
        };
        self.tick_beacon(key, &mut b, false);
        self.world.block_entities.put(key, BlockEntity::Beacon(b));
    }
    /// `ClientLevel.tickBlockEntities` for beacons: a server's chunk data
    /// carries no beam sections, so the client builds them too; effects,
    /// sounds and the advancement stay with the server.
    pub(super) fn client_tick_beacons(&mut self) {
        let keys: Vec<_> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(k, e)| matches!(e, BlockEntity::Beacon(_)).then_some(*k))
            .collect();
        for key in keys {
            let Some(BlockEntity::Beacon(mut b)) = self.world.block_entities.take(&key) else {
                continue;
            };
            self.tick_beacon(key, &mut b, true);
            self.world.block_entities.put(key, BlockEntity::Beacon(b));
        }
    }
    fn tick_beacon(&mut self, (x, y, z): (i32, i32, i32), b: &mut Beacon, client: bool) {
        let mut check = if b.last_check_y < y {
            b.checking.clear();
            b.last_check_y = y - 1;
            y
        } else {
            b.last_check_y + 1
        };
        let surface = self.world_surface(x, z);
        let mut i = 0;
        while i < 10 && check <= surface {
            let state = self.world.block(x, check, z);
            let f = self.res.facts().get(state);
            if let Some(color) = self.beam_color(&f.name) {
                if b.checking.len() <= 1 {
                    b.checking.push((color, 1));
                } else if let Some(last) = b.checking.last_mut() {
                    if color == last.0 {
                        last.1 += 1;
                    } else {
                        let mixed = average(last.0, color);
                        b.checking.push((mixed, 1));
                    }
                }
            } else {
                let Some(last) = b
                    .checking
                    .last_mut()
                    .filter(|_| f.dampening < 15 || &*f.name == "minecraft:bedrock")
                else {
                    b.checking.clear();
                    b.last_check_y = surface;
                    break;
                };
                last.1 += 1;
            }
            check += 1;
            b.last_check_y += 1;
            i += 1;
        }
        let previous = b.levels;
        if self.world.game_time % 80 == 0 {
            if !b.sections.is_empty() {
                b.levels = self.beacon_base(x, y, z);
            }
            if !client && b.levels > 0 && !b.sections.is_empty() {
                self.beacon_effects([x, y, z], b);
                self.beacon_sound([x, y, z], "minecraft:block.beacon.ambient");
            }
        }
        if b.last_check_y >= surface {
            b.last_check_y = self.world.bottom_y() - 1;
            b.sections = std::mem::take(&mut b.checking);
            let (was, is) = (previous > 0, b.levels > 0);
            if !was && is {
                if !client {
                    self.beacon_sound([x, y, z], "minecraft:block.beacon.activate");
                    let levels = b.levels;
                    self.for_each_player(|g| g.beacon_constructed([x, y, z], levels));
                }
            } else if was && !is && !client {
                self.beacon_sound([x, y, z], "minecraft:block.beacon.deactivate");
            }
        }
    }
    /// `BeaconBlockEntity.tick` after activating: `CONSTRUCT_BEACON` for a
    /// player in the box `AABB(x, y, z, x, y - 4, z).inflate(10, 5, 10)`.
    fn beacon_constructed(&mut self, p: [i32; 3], levels: i32) {
        let Some(bb) = self
            .player
            .as_ref()
            .map(|pl| pl.dimensions.bounds(pl.position))
        else {
            return;
        };
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let [x, y, z] = p.map(f64::from);
        if lo.x < x + 10.0
            && hi.x > x - 10.0
            && lo.y < y + 5.0
            && hi.y > y - 9.0
            && lo.z < z + 10.0
            && hi.z > z - 10.0
        {
            self.trigger_construct_beacon(levels);
        }
    }
    pub(super) fn beacon_sound(&mut self, p: [i32; 3], sound: &str) {
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// `updateBase`: complete layers of `beacon_base_blocks` below.
    fn beacon_base(&self, x: i32, y: i32, z: i32) -> i32 {
        let base = self
            .res
            .items()
            .block_tags
            .get("minecraft:beacon_base_blocks");
        let mut levels = 0;
        for step in 1..=4 {
            let ly = y - step;
            if ly < self.world.bottom_y() {
                break;
            }
            let ok = (x - step..=x + step).all(|lx| {
                (z - step..=z + step).all(|lz| {
                    let name = &self.res.facts().get(self.world.block(lx, ly, lz)).name;
                    base.is_some_and(|t| t.contains(&**name))
                })
            });
            if !ok {
                break;
            }
            levels = step;
        }
        levels
    }
    /// `applyEffects`: the powers for players within `levels * 10 + 10` (any height above).
    /// `BeaconBlockEntity.applyEffects`: every player in range.
    fn beacon_effects(&mut self, p: [i32; 3], b: &Beacon) {
        self.for_each_player(|g| g.beacon_effects_on_player(p, b));
    }
    fn beacon_effects_on_player(&mut self, p: [i32; 3], b: &Beacon) {
        let Some(primary) = b.primary.as_deref().and_then(effect_id) else {
            return;
        };
        let range = f64::from(b.levels * 10 + 10);
        let Some(pp) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let inside = pp.x >= f64::from(p[0]) - range
            && pp.x <= f64::from(p[0] + 1) + range
            && pp.z >= f64::from(p[2]) - range
            && pp.z <= f64::from(p[2] + 1) + range
            && pp.y >= f64::from(p[1]) - range;
        if !inside {
            return;
        }
        let secondary = b.secondary.as_deref().and_then(effect_id);
        let amplifier = i32::from(b.levels >= 4 && secondary == Some(primary));
        let duration = (9 + b.levels * 2) * 20;
        let ambient = |id, amp| EffectInstance {
            ambient: true,
            ..EffectInstance::new(id, duration, amp)
        };
        self.potions.effects.add(ambient(primary, amplifier));
        if let Some(s) = secondary.filter(|&s| b.levels >= 4 && s != primary) {
            self.potions.effects.add(ambient(s, 0));
        }
    }
    /// Opening the menu shows the beacon's current powers.
    pub(super) fn beacon_opened(&mut self) {
        let Some(s) = self.station.as_ref().filter(|s| s.kind == Kind::Beacon) else {
            return;
        };
        let pos = s.pos;
        let (p, sec) = match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Beacon(b)) => (b.primary.clone(), b.secondary.clone()),
            _ => (None, None),
        };
        let index =
            |e: Option<String>| e.and_then(|e| EFFECTS.iter().position(|(n, _)| e.ends_with(n)));
        if let Some(s) = self.station.as_mut() {
            s.beacon = [index(p), index(sec)];
        }
    }
    fn beacon_levels(&self) -> i32 {
        let Some(s) = self.station.as_ref() else {
            return 0;
        };
        if let Some(levels) = s.beacon_levels {
            return levels;
        }
        let pos = s.pos;
        match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Beacon(b)) => b.levels,
            _ => 0,
        }
    }
    /// Clicks on the power, confirm and cancel buttons.
    /// The effect, upgrade, confirm and cancel buttons a controller can
    /// focus (GUI origin relative).
    pub(super) fn beacon_targets(&self) -> Vec<[f32; 4]> {
        let (ux, uy) = upgrade_button();
        buttons()
            .into_iter()
            .map(|(_, x, y, _)| (x, y))
            .chain([(ux, uy), (164.0, 107.0), (190.0, 107.0)])
            .map(|(x, y)| [x + SHIFT, y, 22.0, 22.0])
            .collect()
    }
    pub(super) fn beacon_click(&mut self, local: (f32, f32)) -> bool {
        let l = (local.0 - SHIFT, local.1);
        let levels = self.beacon_levels();
        let Some(sel) = self.station.as_ref().map(|s| s.beacon) else {
            return false;
        };
        for (e, x, y, primary) in buttons() {
            if !inside(l, x, y, 22.0, 22.0) {
                continue;
            }
            let tier = EFFECTS[e].1;
            let selected = if primary {
                sel[0] == Some(e)
            } else {
                sel[1] == Some(e)
            };
            if tier <= levels.min(if primary { 3 } else { 4 })
                && !selected
                && (primary || levels >= 4)
            {
                if let Some(s) = self.station.as_mut() {
                    if primary {
                        s.beacon[0] = Some(e);
                        if s.beacon[1] != Some(e) {
                            s.beacon[1] = None;
                        }
                    } else {
                        s.beacon[1] = Some(e);
                    }
                }
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
            }
            return true;
        }
        let (ux, uy) = upgrade_button();
        if inside(l, ux, uy, 22.0, 22.0) {
            if levels >= 4
                && let Some(p) = sel[0]
                && sel[1] != Some(p)
                && let Some(s) = self.station.as_mut()
            {
                s.beacon[1] = Some(p);
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
            }
            return true;
        }
        if inside(l, 164.0, 107.0, 22.0, 22.0) {
            let paid = self.station.as_ref().is_some_and(|s| s.inputs[0].is_some());
            if paid && sel[0].is_some() {
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                self.confirm_beacon(sel);
                self.close_beacon_screen();
            }
            return true;
        }
        if inside(l, 190.0, 107.0, 22.0, 22.0) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.close_beacon_screen();
            return true;
        }
        false
    }
    fn close_beacon_screen(&mut self) {
        self.close_container();
        self.screen = super::Screen::Playing;
        self.capture(true);
    }
    /// `BeaconMenu.updateEffects`: spend the payment and set the powers. On
    /// a server, `BeaconScreen`'s confirm button sends them
    /// (`ServerboundSetBeaconPacket`) and the server's menu does it.
    fn confirm_beacon(&mut self, sel: [Option<usize>; 2]) {
        if self.remote.is_some() {
            let id = |e: Option<usize>| e.and_then(|e| self.remote_effect_network_id(EFFECTS[e].0));
            let (primary, secondary) = (id(sel[0]), id(sel[1]));
            self.send(S::SetBeacon(play::SetBeacon { primary, secondary }));
            return;
        }
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let pos = s.pos;
        s.inputs[0] = None;
        let name = |e: Option<usize>| e.map(|e| format!("minecraft:{}", EFFECTS[e].0));
        let mut select_sound = false;
        if let Some(BlockEntity::Beacon(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.primary = name(sel[0]);
            b.secondary = name(sel[1]);
            select_sound = !b.sections.is_empty();
        }
        if select_sound {
            self.beacon_sound(pos, "minecraft:block.beacon.power_select");
        }
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }
    /// `BeaconScreen`: background, labels, payment icons, power buttons.
    pub(super) fn draw_beacon(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = self.inventory_origin(batch);
        let (bx, by) = (ox + SHIFT, oy);
        let local = (self.cursor.0 - bx, self.cursor.1 - by);
        let levels = self.beacon_levels();
        let sel = self.station.as_ref().map_or([None; 2], |s| s.beacon);
        let paid = self.station.as_ref().is_some_and(|s| s.inputs[0].is_some());
        // BeaconScreen draws no title, but has the menu's.
        super::screen_info::name_container(|| {
            self.remote_menu_title().map_or_else(
                || self.translate("container.beacon", "Beacon"),
                |t| t.plain(),
            )
        });
        let atlas = &self.gui.atlas;
        batch.fill(
            atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(16, 16, 16, 160),
        );
        if let Some(mut r) = atlas.sprites.get("gui/container/beacon").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 230.0 * du;
            r.v1 = r.v0 + HEIGHT * dv;
            batch.rect_uv(bx, by, 230.0, HEIGHT, r, u32::MAX);
        }
        let label = 0xFFE0_E0E0;
        for (key, fallback, cx) in [
            ("block.minecraft.beacon.primary", "Primary Power", 62.0),
            ("block.minecraft.beacon.secondary", "Secondary Power", 169.0),
        ] {
            let text = self.translate(key, fallback);
            let atlas = &self.gui.atlas;
            let w = HudBatch::text_width(atlas, &text);
            batch.text(
                atlas,
                &text,
                bx + cx - (w / 2.0).floor(),
                by + 10.0,
                label,
                false,
            );
        }
        for (i, item) in PAYMENT.iter().rev().enumerate() {
            let x = [20.0, 41.0, 63.0, 86.0, 108.0][i];
            self.draw_item(batch, &ItemStack::new(item, 1), bx + x, by + 109.0);
        }
        let atlas = &self.gui.atlas;
        let mut tooltip = None;
        let button =
            |batch: &mut HudBatch, x: f32, y: f32, active: bool, selected: bool, icon: &str| {
                let hover = inside(local, x, y, 22.0, 22.0);
                let sprite = if !active {
                    "button_disabled"
                } else if selected {
                    "button_selected"
                } else if hover {
                    "button_highlighted"
                } else {
                    "button"
                };
                batch.sprite(
                    atlas,
                    &format!("gui/sprites/container/beacon/{sprite}"),
                    bx + x,
                    by + y,
                );
                batch.sprite_scaled(
                    atlas,
                    icon,
                    bx + x + 2.0,
                    by + y + 2.0,
                    18.0,
                    18.0,
                    u32::MAX,
                );
                hover
            };
        for (e, x, y, primary) in buttons() {
            let tier = EFFECTS[e].1;
            let active = if primary {
                tier <= levels && tier < 4
            } else {
                levels >= 4
            };
            let selected = if primary {
                sel[0] == Some(e)
            } else {
                sel[1] == Some(e)
            };
            let icon = format!("mob_effect/{}", EFFECTS[e].0);
            if button(batch, x, y, active, selected, &icon) {
                tooltip = Some(format!("effect.minecraft.{}", EFFECTS[e].0));
            }
        }
        if let Some(p) = sel[0] {
            let (x, y) = upgrade_button();
            let icon = format!("mob_effect/{}", EFFECTS[p].0);
            if button(batch, x, y, levels >= 4, sel[1] == Some(p), &icon) {
                tooltip = Some(format!("effect.minecraft.{}", EFFECTS[p].0));
            }
        }
        button(
            batch,
            164.0,
            107.0,
            paid && sel[0].is_some(),
            false,
            "gui/sprites/container/beacon/confirm",
        );
        button(
            batch,
            190.0,
            107.0,
            true,
            false,
            "gui/sprites/container/beacon/cancel",
        );
        let slots: Vec<usize> = (0..36).chain([INPUT]).collect();
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        for i in slots {
            let (sx, sy) = self.slot_position(i);
            if let Some(stack) = self.slot_mut(i).clone() {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        let mut stack_tip = None;
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            stack_tip = self.slot_mut(slot).clone();
        }
        if self.carried.is_some() {
            self.draw_carried(batch);
            if let Some(stack) = stack_tip.filter(|s| self.tooltip_with_carried(s)) {
                self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
            }
        } else if let Some(stack) = stack_tip {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        } else if let Some(key) = tooltip {
            let text = self.translate(&key, &key);
            self.draw_tooltip_lines(
                batch,
                &[(horizon_client::text::Text::literal(text), u32::MAX)],
                self.cursor.0,
                self.cursor.1,
            );
        }
    }
    /// `BeaconRenderer`: each section's rotating solid beam and its glow.
    pub(super) fn beacon_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .gui
            .atlas
            .sprites
            .get("entity/beacon/beacon_beam")
            .copied()
        else {
            return;
        };
        let time = (self.world.game_time.rem_euclid(40)) as f32 + alpha as f32;
        let scoping = self.scoping();
        // `BeaconRenderer.shouldRender` culls at the render distance; far
        // terrain extends that to its own distance.
        let view = f64::from(self.terrain_distance() * 16);
        let world = &self.world;
        if self.lod.is_some() {
            // The beam stays after its chunk unloads; once the chunk is back,
            // its block entity decides whether the beam is still there.
            self.far_beacons.retain(|&pos, _| {
                let (x, _, z) = pos;
                !world.chunks.contains_key(&(x >> 4, z >> 4))
                    || world
                        .block_entities
                        .get(&pos)
                        .is_some_and(|e| beam(e).is_some() || is_test_instance_beam(e))
            });
        } else {
            self.far_beacons.clear();
        }
        let test_beams: Vec<_> = world
            .block_entities
            .iter()
            .filter_map(|(k, e)| Some((*k, self.test_instance_beam(e)?)))
            .collect();
        for (k, e) in &world.block_entities {
            if let Some(sections) = beam(e) {
                self.far_beacons.insert(*k, sections.to_vec());
            }
        }
        for (k, sections) in test_beams {
            self.far_beacons.insert(k, sections);
        }
        for (k, sections) in &self.far_beacons {
            let base = glam::DVec3::new(f64::from(k.0) + 0.5, f64::from(k.1), f64::from(k.2) + 0.5);
            let offset = base - camera;
            let horizontal = glam::DVec2::new(offset.x, offset.z).length();
            if horizontal >= view {
                continue;
            }
            let distance = horizontal as f32;
            let scale = if scoping {
                1.0
            } else {
                (distance / 96.0).max(1.0)
            };
            let origin = (glam::DVec3::new(f64::from(k.0), f64::from(k.1), f64::from(k.2))
                - camera)
                .as_vec3();
            let start = out.len();
            horizon_client::block_entity_render::beacon_beam(origin, sections, time, scale, s, out);
            // HDR output: the beam shines (entity.wgsl fs_hdr_mark).
            for v in &mut out[start..] {
                v.layer |= u32::from(horizon_client::mesher::BRIGHT) << 16;
            }
        }
    }
}
/// Whether a test instance block shows a beam (`getBeamSections` not empty).
fn is_test_instance_beam(e: &BlockEntity) -> bool {
    matches!(e, BlockEntity::TestInstanceBlock(b) if b.data.status != Status::Cleared)
}
/// The beam sections of an active beacon (`BeaconRenderer` draws no beam for
/// a beacon without a pyramid or with all sections cleared).
fn beam(e: &BlockEntity) -> Option<&[(u32, i32)]> {
    match e {
        BlockEntity::Beacon(b) if b.levels > 0 && !b.sections.is_empty() => Some(&b.sections),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_active_beacons_have_beams() {
        let mut b = Beacon::default();
        assert!(beam(&BlockEntity::Beacon(Box::new(b.clone()))).is_none());
        b.levels = 1;
        b.sections.push((0xFFFFFF, 10));
        assert_eq!(
            beam(&BlockEntity::Beacon(Box::new(b))),
            Some(&[(0xFFFFFF, 10)][..])
        );
    }
}
