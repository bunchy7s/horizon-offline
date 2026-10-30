//! Items drawn through the resolved item model pipeline
//! (`horizon_client::item_render`): GUI icons (`GuiGraphics.renderItem`),
//! dropped items (`ItemEntityRenderer`) and items posed in the world (hands,
//! frames, armour stands). This binds the resolver's host services
//! (`ItemEnv`) and the special renderers' registries to the game.
use super::{Dim, Game};
use horizon_assets::item_model::{
    Components, DisplayContext, GlobalPos, ItemEnv, LevelState, OwnerState, StackView,
};
use horizon_client::{
    block_entity_render::{diffuse_lights, vertex_light},
    hud::{HudBatch, Rect},
    item_render::{self, ItemRenderState, SpecialHost, WorldSink},
    items::ItemStack,
    mesher::Vertex,
};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap};

/// The game's answers to what the resolver cannot read from the stack.
pub(super) struct Env<'a, 'g> {
    game: &'a Game<'g>,
}
impl ItemEnv for Env<'_, '_> {
    fn potion_effect_color(&self, contents: &Value) -> Option<i32> {
        // `PotionContents.getColorOr`: the effect mix when any effect is visible.
        let id = contents["potion"].as_str();
        let effects = id
            .and_then(horizon_client::brewing::potion)
            .is_some_and(|p| !p.effects.is_empty());
        effects.then(|| (0xFF00_0000 | horizon_client::brewing::potion_color(id)) as i32)
    }
    fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        self.game
            .pack
            .item_renderer
            .grass_color(temperature, downfall)
    }
    fn bundle_fullness(&self, _stack: &StackView, components: &Components) -> f32 {
        // `BundleContents.weight`: count / max stack size, summed.
        let Some(items) = components
            .get("minecraft:bundle_contents")
            .and_then(Value::as_array)
        else {
            return 0.0;
        };
        let prototypes = &self.game.pack.item_renderer.models.prototypes;
        let mut total = 0.0f64;
        for item in items {
            let id = item["id"].as_str().unwrap_or("");
            let count = item["count"].as_i64().unwrap_or(1) as f64;
            let max = prototypes
                .get(id)
                .and_then(|c| c.get("minecraft:max_stack_size"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as f64;
            total += count / max;
        }
        total as f32
    }
    fn has_shift_down(&self) -> bool {
        self.game.has_shift_down()
    }
    fn local_time(&self, pattern: &str, _locale: &str, _time_zone: Option<&str>) -> String {
        local_time(pattern)
    }
    fn random_float(&self) -> f32 {
        let nanos = self.game.started.elapsed().subsec_nanos();
        (nanos % 1_000_000) as f32 / 1_000_000.0
    }
}
/// The days since 1970-01-01 as a civil (year, month, day) date.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
/// `DateTimeFormatter.ofPattern(pattern).format(now)` for the month, day,
/// year and time letters item models use, in the system time zone
/// (`LocalTime` without a `time_zone`).
pub(super) fn local_time(pattern: &str) -> String {
    let now = chrono::Local::now();
    let secs = now.timestamp() + i64::from(now.offset().local_minus_utc());
    let (y, mo, d) = civil(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    let (h, mi, se) = (s / 3600, s / 60 % 60, s % 60);
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let num = |v: i64| format!("{v:0n$}");
        match c {
            'M' => out += &num(i64::from(mo)),
            'd' => out += &num(i64::from(d)),
            'y' | 'u' => out += &if n == 2 { num(y % 100) } else { num(y) },
            'H' => out += &num(h),
            'm' => out += &num(mi),
            's' => out += &num(se),
            '\'' => {
                // Quoted literal text.
                let end = chars[i + 1..].iter().position(|&x| x == '\'');
                if let Some(e) = end {
                    out.extend(&chars[i + 1..i + 1 + e]);
                    i += e + 2;
                    continue;
                }
                n = 1;
            }
            other => out.extend(std::iter::repeat_n(other, n)),
        }
        i += n;
    }
    out
}

/// Banner and shield pattern textures from the banner pattern registry.
pub(super) struct Host<'a> {
    banners: &'a horizon_client::banner::Banners,
    sprites: &'a HashMap<String, Rect>,
    skins: &'a RefCell<horizon_client::skins::Downloader>,
    profiles: &'a RefCell<horizon_client::skins::Resolver>,
}
impl SpecialHost for Host<'_> {
    fn banner_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.banner_texture(pattern))
    }
    fn shield_texture(&self, pattern: &str) -> Option<String> {
        self.banners
            .pattern(pattern)
            .map(|_| self.banners.shield_texture(pattern))
    }
    fn player_skin(&self, profile: &Value) -> Option<String> {
        // SkullBlockRenderer.getPlayerSkin: SkinManager.getInsecureSkinInformation
        // accepts a texture property without a signature, unlike the tab list.
        // A `Dynamic` profile (a name or id only) resolves through the
        // services (`PlayerSkinRenderCache.getOrDefault`).
        Some(horizon_client::skins::head_skin(
            profile,
            &mut self.profiles.borrow_mut(),
            &mut self.skins.borrow_mut(),
            |location| self.sprites.contains_key(location),
        ))
    }
}

/// `ItemOwner` of an entity holding or showing an item.
pub(super) fn owner_at(position: [f64; 3], yaw: f32, entity_type: &str) -> OwnerState {
    OwnerState {
        position,
        visual_rotation_y_degrees: yaw,
        entity_type: entity_type.to_owned(),
        ..Default::default()
    }
}
/// `UseDuration` of a stack (`Item.getUseDuration`): 72000 for drawn items
/// and for `blocks_attacks` items (shields), the consume time for food.
fn use_duration(components: &Components) -> i32 {
    if let Some(c) = components.get("minecraft:consumable") {
        let seconds = c["consume_seconds"].as_f64().unwrap_or(1.6);
        return (seconds * 20.0) as i32;
    }
    if components.get("minecraft:blocks_attacks").is_some()
        || components.get("minecraft:kinetic_weapon").is_some()
    {
        return 72_000;
    }
    let item_model = components
        .get("minecraft:item_model")
        .and_then(Value::as_str)
        .unwrap_or("");
    match item_model {
        "minecraft:bow"
        | "minecraft:crossbow"
        | "minecraft:trident"
        | "minecraft:spyglass"
        | "minecraft:goat_horn" => 72_000,
        // BrushItem.USE_DURATION.
        "minecraft:brush" => 200,
        _ => 0,
    }
}

impl Game<'_> {
    pub(super) fn item_env(&self) -> Env<'_, '_> {
        Env { game: self }
    }
    pub(super) fn item_host(&self) -> Host<'_> {
        Host {
            banners: &self.stations().banners,
            sprites: &self.gui.atlas.sprites,
            skins: &self.skin_downloads,
            profiles: &self.skin_profiles,
        }
    }
    /// The client level as item models read it: worked out once a frame (a
    /// screen of items read the same level, its sky sampled once).
    pub(super) fn item_level(&self) -> LevelState {
        let dimension = match self.dim {
            Dim::Overworld => "minecraft:overworld",
            Dim::Nether => "minecraft:the_nether",
            Dim::End => "minecraft:the_end",
        };
        let key = (
            self.frames,
            self.world.game_time,
            self.world.day_time,
            dimension,
        );
        let mut cached = self.item_level_cache.borrow_mut();
        if let Some((at, level)) = cached.as_ref()
            && *at == key
        {
            return level.clone();
        }
        let level = self.level_state_now(dimension);
        *cached = Some((key, level.clone()));
        level
    }
    fn level_state_now(&self, dimension: &'static str) -> LevelState {
        let sky = self
            .dim_environment()
            .sample(self.world.day_time, None, None);
        LevelState {
            dimension: dimension.to_owned(),
            game_time: self.world.game_time,
            sun_angle: sky.sun_angle,
            moon_phase: (self.world.day_time.div_euclid(24_000)).rem_euclid(8) as i32,
            respawn: self.spawn.map(|s| GlobalPos {
                dimension: "minecraft:overworld".to_owned(),
                pos: [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32],
            }),
            runs_normally: true,
        }
    }
    /// The local player as `ItemOwner`; `stack` is the stack drawn, to tell
    /// whether it is the one in use.
    pub(super) fn player_owner(&self, stack: Option<&ItemStack>) -> OwnerState {
        let position = self
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut owner = owner_at(position, self.yaw, "minecraft:player");
        owner.main_arm_left = self.settings.main_hand_left;
        owner.is_local_player = true;
        owner.is_view_entity = true;
        owner.last_death_location = self
            .level
            .last_death
            .clone()
            .map(|(dimension, pos)| GlobalPos { dimension, pos });
        if let (Some((slot, ticks)), Some(stack)) = (self.using, stack)
            && self.stack_in(slot) == Some(stack)
        {
            let view = item_render::stack_view(stack);
            let duration = use_duration(&self.pack.item_renderer.models.components(&view));
            owner.using_this_item = true;
            owner.use_duration = duration;
            owner.use_item_remaining_ticks = duration - ticks as i32;
            owner.crossbow_charge_duration = Self::charge_ticks(stack).max(1) as i32;
        }
        owner.fishing_cast_with_this_item = stack
            .zip(self.cast_rod())
            .is_some_and(|(s, rod)| std::ptr::eq(s, rod));
        owner.is_selected =
            stack.is_some() && self.inventory.get(self.selected).and_then(Option::as_ref) == stack;
        owner.is_carried = owner.is_selected;
        owner
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub(super) fn resolve_item(
        &self,
        stack: &ItemStack,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        seed: i32,
    ) -> ItemRenderState {
        let level = self.item_level();
        self.pack.item_renderer.resolve(
            &item_render::stack_view(stack),
            display,
            owner,
            Some(&level),
            seed,
            &self.item_env(),
        )
    }
    /// `GuiGraphics.renderItem(player, level, stack, x, y, 0)`.
    pub(super) fn draw_item_model(&self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        let owner = self.player_owner(Some(stack));
        let state = self.resolve_item(stack, DisplayContext::Gui, Some(&owner), 0);
        item_render::gui::draw(
            &state,
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            batch,
            x,
            y,
            self.started.elapsed().as_secs_f32(),
            self.settings.file.double("glintSpeed") as f32,
            self.settings.file.double("glintStrength") as f32,
        );
    }
    /// The `GLINT_TEXTURING` inputs the atlas and options provide.
    pub(super) fn glint(&self) -> Option<item_render::Glint> {
        self.gui
            .atlas
            .sprites
            .get("misc/enchanted_glint_item")
            .copied()
            .map(|_| item_render::Glint {
                speed: self.settings.file.double("glintSpeed") as f32,
                alpha: self.settings.file.double("glintStrength") as f32,
                millis: self.started.elapsed().as_secs_f32() * 1000.0,
                scale: 0.5,
            })
    }
    /// The glint scroll phase shared by world items and armour
    /// (`item_render::Glint::scroll_ticks`).
    pub(super) fn glint_ticks(&self) -> f32 {
        item_render::Glint::scroll_ticks(
            self.started.elapsed().as_secs_f32() * 1000.0,
            self.settings.file.double("glintSpeed") as f32,
        )
    }
    /// Submit a resolved item under `pose` (camera-relative) at `light`
    /// (packed lightmap coordinates).
    pub(super) fn submit_item_world(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(self.dim == Dim::Nether);
        self.submit_item_lit(state, pose, light, lights, out);
    }
    /// [`Self::submit_item_world`] shaded by other diffuse lights (a
    /// `Lighting.Entry` other than the level's), in the pose's output space.
    pub(super) fn submit_item_lit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        lights: (glam::Vec3, glam::Vec3),
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light,
            lights: Some(lights),
            glint: self.glint(),
        };
        state.submit(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// [`Self::submit_item_world`] without the level's diffuse shading.
    pub(super) fn submit_item_unlit(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let mut sink = WorldSink {
            out,
            light,
            lights: None,
            glint: self.glint(),
        };
        state.submit(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }
    /// `ItemEntityRenderer.submit` for every dropped item.
    pub(super) fn dropped_item_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let live = self.dropped.iter().map(|item| {
            let (p, q) = (item.previous, item.position);
            let at = [
                p.x + (q.x - p.x) * alpha,
                p.y + (q.y - p.y) * alpha,
                p.z + (q.z - p.z) * alpha,
            ];
            (item, at, alpha as f32)
        });
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Item(item) => {
                Some((&**item, super::pickups::position(p, alpha), 1.0))
            }
            _ => None,
        });
        for (item, at, partial) in live.chain(taken) {
            let entity_start = out.len();
            let q = item.position;
            let pos = glam::DVec3::from_array(at) - camera;
            if pos.length_squared() > 64.0 * 64.0 {
                continue;
            }
            // `extractItemGroupRenderState`: GROUND, seeded by the entity id.
            let owner = owner_at([q.x, q.y, q.z], 0.0, "minecraft:item");
            let state = self.resolve_item(
                &item.stack,
                DisplayContext::Ground,
                Some(&owner),
                item.id as i32,
            );
            if state.is_empty() {
                continue;
            }
            let age = item.age as f32 + partial;
            let (lo, _) = state.bounding_box();
            let min_offset_y = -lo.y + 0.0625;
            let bob = (age / 10.0 + item.bob_offset).sin() * 0.1 + 0.1;
            let spin = item_render::item_spin(age, item.bob_offset);
            let pose = glam::Mat4::from_translation(pos.as_vec3())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, bob + min_offset_y, 0.0))
                * glam::Mat4::from_rotation_y(spin);
            let packed = self.world.light(
                q.x.floor() as i32,
                (q.y + 0.1).floor() as i32,
                q.z.floor() as i32,
            );
            let seed = i64::from(self.pack.item_renderer.item_id(&item.stack.item))
                + i64::from(item.stack.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose,
                item_render::rendered_amount(item.stack.count),
                seed,
            ) {
                self.submit_item_world(&state, copy, vertex_light(packed), out);
            }
            if self.entity_glowing(item.id) {
                let rgb = super::glowing::team_color(self.mob_team(item.id));
                self.mark_outline(&out[entity_start..], rgb);
            } else {
                self.cull_entity_vertices(out, entity_start);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn civil_dates_follow_the_gregorian_calendar() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_716), (2023, 12, 25));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_shield_is_used_for_the_vanilla_s_time() {
        // Item.getUseDuration: an item with `blocks_attacks` (a shield) is
        // used for 72000 ticks, so its use animation (the arm held up)
        // runs instead of ending at once (#2289).
        let mut shield = Components::new();
        shield.insert("minecraft:blocks_attacks".into(), json!({}));
        assert_eq!(use_duration(&shield), 72_000);
        // Food is used for as long as it takes to eat it.
        let mut food = Components::new();
        food.insert(
            "minecraft:consumable".into(),
            json!({"consume_seconds": 1.6}),
        );
        assert_eq!(use_duration(&food), 32);
        // The bow keeps its own use time.
        let mut bow = Components::new();
        bow.insert("minecraft:item_model".into(), json!("minecraft:bow"));
        assert_eq!(use_duration(&bow), 72_000);
        // Anything else is not used at all.
        assert_eq!(use_duration(&Components::new()), 0);
    }
}
