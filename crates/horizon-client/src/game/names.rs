//! Custom names: `NameTagItem.interactLivingEntity` and the name tag
//! (`EntityRenderer.submitNameTag`): above the head, facing the camera,
//! on a translucent black background, shown when the name is visible or the
//! mob is under the crosshair within 64 blocks, as its team's name tag
//! visibility allows, in the team's colours, with its score in the
//! objective displayed below names. Other players' names (on a LAN host
//! or a server) show whenever `LivingEntityRenderer.shouldShowName` allows,
//! and the player's own in third person with the `ownNameTag` extra.
use super::{Game, view_vector};
use horizon_client::{
    block_entity_render::vertex_light, hud::HudBatch, mesher::Vertex, mobs::Mob, save::uuid_text,
    scoreboard as sb, text::Text,
};

impl Game<'_> {
    /// A named name tag names the mob under the crosshair.
    pub(super) fn name_mob(&mut self, index: usize) -> bool {
        let Some(name) = self
            .held()
            .filter(|h| &*h.item == "minecraft:name_tag")
            .and_then(|h| h.custom_name().map(str::to_owned))
        else {
            return false;
        };
        let m = &mut self.mobs[index];
        if !m.alive() {
            return false;
        }
        m.custom_name = Some(name);
        m.persistent = true;
        // itemStack.shrink(1), in creative too.
        self.consume_held();
        self.swing_arm();
        true
    }
    /// `Entity.belowNameDisplay`: the score holder's score in the objective
    /// shown below names, then a space and the objective's name.
    fn below_name_text(&self, holder: &str) -> Option<Text> {
        let board = &self.score.board;
        let objective = board.display_objective(sb::SLOT_BELOW_NAME)?;
        let score = board.score(holder, &objective.name)?;
        // numberFormatOrDefault(StyledFormat.NO_STYLE).
        let format = score
            .number_format
            .clone()
            .or_else(|| objective.number_format.clone())
            .unwrap_or(sb::NumberFormat::Styled(serde_json::json!({})));
        let mut t = Text::empty();
        t.push(format.format(score.value));
        t.push(Text::literal(" "));
        t.push(objective.display_text());
        Some(t)
    }
    /// The other players' models and names: the guests on a LAN host, the
    /// players a server sends on a client.
    fn other_players(&self) -> Vec<(&Mob, String)> {
        let mut out = Vec::new();
        if let Some(host) = &self.lan {
            for (conn, m) in &host.models {
                if let Some(g) = host.hub.guest(*conn) {
                    out.push((m, g.name.clone()));
                }
            }
        }
        for (id, name) in self.remote_player_names() {
            if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
                out.push((m, name));
            }
        }
        out
    }
    /// Horizon's own name tag extra (`ownNameTag`): the player's own model and
    /// name, while that model shows in third person. Vanilla never names the
    /// camera entity (`LivingEntityRenderer.shouldShowName`).
    fn own_name_tag(&self) -> Option<(&Mob, String)> {
        if !self.settings.extras.own_name_tag
            || self.camera_type.first_person()
            || self.sleeping.is_some()
            || self.spectator()
            || self.potions.effects.get("invisibility").is_some()
        {
            return None;
        }
        let m = self.player_model.as_ref()?;
        // The name others see: the profile's on a server (single player's
        // server thread included), else the local player's.
        let name = match self.remote.as_ref() {
            Some(r) if !r.profile_name.is_empty() => r.profile_name.clone(),
            _ => self.acting_player_name().to_owned(),
        };
        Some((m, name))
    }
    /// The names the name tags show this frame (the control socket's status).
    pub(super) fn shown_name_tags(&self, camera: glam::DVec3) -> Vec<String> {
        self.name_tags(camera, 1.0)
            .into_iter()
            .filter_map(|t| t.lines.last().map(|(t, _)| t.plain()))
            .collect()
    }
    /// Each name tag this frame.
    fn name_tags(&self, camera: glam::DVec3, alpha: f64) -> Vec<NameTag> {
        let target = self.mob_hit().map(|(i, _)| self.mobs[i].id);
        let viewer = self.player_team();
        let mut tags = Vec::new();
        let players = self.other_players();
        let named = self.mobs.iter().filter_map(|m| {
            let name = m.custom_name.as_ref()?;
            // The tag's 64 block distance is measured to the entity: copy the
            // name only for a mob the frame can show it on.
            let at = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
            (at.distance_squared(camera) < 64.0 * 64.0).then(|| (m, name.clone(), false))
        });
        let players = players
            .into_iter()
            .chain(self.own_name_tag())
            .map(|(m, name)| (m, name, true));
        // Who is riding whom: asked once per tag below, and rescanning every mob
        // for each tag made this quadratic in the number of loaded mobs.
        let ridden: std::collections::HashSet<u32> =
            self.mobs.iter().filter_map(|o| o.extra.mount).collect();
        for (m, name, player) in named.chain(players) {
            if !m.alive() {
                continue;
            }
            // EntityAttachment.NAME_TAG: 0.5 above the (crouching) height.
            let height = if player && m.extra.crouching {
                1.5
            } else {
                f64::from(m.height())
            };
            let p = glam::DVec3::new(
                m.previous.x + (m.position.x - m.previous.x) * alpha,
                m.previous.y + (m.position.y - m.previous.y) * alpha + height + 0.5,
                m.previous.z + (m.position.z - m.previous.z) * alpha,
            );
            // The name tag distance (64) is measured to the entity.
            let distance_sqr =
                glam::DVec3::new(m.position.x, m.position.y, m.position.z).distance_squared(camera);
            if distance_sqr >= 64.0 * 64.0 {
                continue;
            }
            // A player's scores and team go by name, a mob's by UUID.
            let holder = if player {
                Some(name.clone())
            } else {
                self.persist.uuids.get(&m.id).map(|u| uuid_text(*u))
            };
            let team = if player {
                self.score.board.players_team(&name)
            } else {
                self.mob_team(m.id)
            };
            // LivingEntityRenderer.shouldShowName (team rules included), then
            // MobRenderer: a visible name, or the mob under the crosshair.
            let invisible = sb::invisible_to(
                team,
                viewer,
                self.spectator(),
                m.invisible || m.has_effect("minecraft:invisibility"),
            );
            // Entity.isDiscrete: a crouching player.
            let discrete = player && m.extra.crouching;
            let view = sb::NameView {
                discrete,
                distance_sqr,
                invisible_to_viewer: invisible,
                hud_hidden: self.ui.hide_gui,
                is_camera: false,
                is_vehicle: ridden.contains(&m.id),
            };
            // MobRenderer.shouldShowName: a visible name, or the mob under the
            // crosshair; players (LivingEntityRenderer) need neither.
            if !sb::name_shown(team, viewer, view)
                || !(player || m.name_visible || Some(m.id) == target)
            {
                continue;
            }
            // getDisplayName: the name in the team's prefix, suffix and colour.
            let member = if player {
                Text::literal(&name)
            } else {
                m.custom_name_text
                    .clone()
                    .unwrap_or_else(|| Text::literal(&name))
            };
            // StringDecomposer.iterateFormatted: `§` codes a server leaves
            // inside the team prefix or the name itself apply at draw time.
            let shown = match team {
                Some(t) => t.formatted_name(member),
                None => member,
            }
            .resolve_legacy();
            let mut lines = Vec::new();
            // EntityRenderer.extractNameTags: the score shows within the
            // below-name distance (10), under the name.
            let mut lift = 0.0;
            if distance_sqr < 10.0 * 10.0
                && let Some(score) = holder.and_then(|h| self.below_name_text(&h))
            {
                lines.push((score.resolve_legacy(), 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            let l = self.world.light(
                m.position.x.floor() as i32,
                (m.position.y + height).floor() as i32,
                m.position.z.floor() as i32,
            );
            tags.push(NameTag {
                at: p,
                lines,
                light: l,
                discrete,
            });
        }
        // Armor stands name themselves: an invisible one with a visible name
        // is a hologram (its model is skipped, its name tag is not).
        for a in &self.armor_stands {
            let Some(name) = &a.custom_name else {
                continue;
            };
            if !a.name_visible {
                continue;
            }
            let height = match (a.marker, a.small) {
                (true, _) => 0.0,
                (false, true) => 0.9875,
                (false, false) => 1.975,
            };
            let distance_sqr =
                glam::DVec3::new(a.position.x, a.position.y, a.position.z).distance_squared(camera);
            if distance_sqr >= 64.0 * 64.0 {
                continue;
            }
            let team = self.mob_team(a.id);
            let invisible = sb::invisible_to(team, viewer, self.spectator(), false);
            let view = sb::NameView {
                discrete: false,
                distance_sqr,
                invisible_to_viewer: invisible,
                hud_hidden: self.ui.hide_gui,
                is_camera: false,
                is_vehicle: false,
            };
            if !sb::name_shown(team, viewer, view) {
                continue;
            }
            let shown = match team {
                Some(t) => t.formatted_name(name.clone()),
                None => name.clone(),
            }
            .resolve_legacy();
            let mut lines = Vec::new();
            let mut lift = 0.0;
            let holder = self.persist.uuids.get(&a.id).map(|u| uuid_text(*u));
            if distance_sqr < 10.0 * 10.0
                && let Some(score) = holder.and_then(|h| self.below_name_text(&h))
            {
                lines.push((score.resolve_legacy(), 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            let l = self.world.light(
                a.position.x.floor() as i32,
                (a.position.y + height).floor() as i32,
                a.position.z.floor() as i32,
            );
            tags.push(NameTag {
                at: glam::DVec3::new(a.position.x, a.position.y + height + 0.5, a.position.z),
                lines,
                light: l,
                discrete: false,
            });
        }
        // ItemFrameRenderer.shouldShowName/getNameTag: only the targeted
        // frame's custom-named item, with the HUD visible (also glow frames).
        if !self.ui.hide_gui
            && let Some((i, distance)) = self.frame_hit()
            && self.before_block(distance)
            && self.mob_hit().is_none_or(|(_, d)| d > distance)
            && let Some(name) = self.item_frames[i]
                .item
                .as_ref()
                .and_then(|item| item.custom_name())
        {
            let frame = &self.item_frames[i];
            let (lo, hi) = frame.bounds();
            let center = glam::DVec3::from_array(std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0));
            if center.distance_squared(camera) < 64.0 * 64.0 {
                // NAME_TAG is at the entity height (0.5), then the name
                // submitter lifts it 0.5. ItemFrameRenderer offsets rendering
                // 0.3 toward its facing and -0.25 vertically.
                let step = frame.step();
                tags.push(NameTag {
                    at: center
                        + glam::DVec3::new(
                            f64::from(step[0] as f32 * 0.3),
                            0.75,
                            f64::from(step[2] as f32 * 0.3),
                        ),
                    lines: vec![(Text::literal(name).resolve_legacy(), 0.0)],
                    light: self.world.light(
                        center.x.floor() as i32,
                        center.y.floor() as i32,
                        center.z.floor() as i32,
                    ),
                    discrete: false,
                });
            }
        }
        tags
    }
    /// `SubmitNodeCollection.submitNameTag`: a discrete name is drawn once,
    /// faded, on its background; any other in full over the world, its faded
    /// copy and background in `see_through`, over everything.
    pub(super) fn name_tag_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        see_through: &mut Vec<Vertex>,
    ) {
        // The text alpha that goes with getBackgroundOpacity(0.25):
        // max((0.25 + 0.75) / 2, 0.5), as ARGB.color (the floor of 255 times it).
        const FADED: u32 = 0x7F;
        let mut patches = Vec::new();
        for tag in self.name_tags(camera, alpha) {
            let o = tag.at - camera;
            // `gbuffers_entities` draws name tag text (2.3); a sneaking player's faded tag, on its
            // background, blends.
            if tag.discrete {
                self.mark_translucent_entity(out.len(), "minecraft:name_tag");
                let light = vertex_light(tag.light);
                patches.extend(self.name_tag_quads(o, &tag.lines, FADED, true, 0, light, out));
                continue;
            }
            // LightCoordsUtil.lightCoordsWithEmission(lightCoords, 2).
            let lit = (tag.light >> 4).max(2) << 4 | (tag.light & 15).max(2);
            self.mark_entity(out.len(), "minecraft:name_tag");
            let light = vertex_light(lit);
            patches.extend(self.name_tag_quads(o, &tag.lines, 0xFF, false, 0, light, out));
            // RenderPipelines.TEXT_SEE_THROUGH: no depth test, no lightmap.
            patches.extend(self.name_tag_quads(o, &tag.lines, FADED, true, 2, 0, see_through));
        }
        self.map_patches.extend(patches);
    }
    /// The lines of one name tag, `o` from the camera: the text at
    /// `text_alpha` in its layer, on the translucent black background
    /// (in `text_layer` like the glyphs, a hair behind) when `background`.
    /// Returns the glyph pixels the atlas still needs.
    #[allow(clippy::too_many_arguments)]
    fn name_tag_quads(
        &self,
        o: glam::DVec3,
        lines: &[(Text, f64)],
        text_alpha: u32,
        background: bool,
        text_layer: u32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> Vec<horizon_client::font::Patch> {
        // Options.getBackgroundOpacity(0.25) black, as ARGB.color.
        const BACKGROUND: u32 = 0x3F00_0000;
        // The camera's facing (Camera.rotation()), turned round in third-person
        // front view, so the text reads the right way from where it is seen.
        let (view_yaw, view_pitch) =
            if self.camera_type == super::perspective::CameraType::ThirdPersonFront {
                (self.yaw + 180.0, -self.pitch)
            } else {
                (self.yaw, self.pitch)
            };
        let forward = glam::DVec3::from_array(view_vector(view_yaw, view_pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let away = forward.as_vec3() * 0.002;
        let mut batch = HudBatch::new([1, 1]);
        for (text, lift) in lines {
            let o = (o + glam::DVec3::new(0.0, *lift, 0.0)).as_vec3();
            let (r, u) = (right.as_vec3() * 0.025, up.as_vec3() * 0.025);
            // Text space: x right, y down (scale 0.025, -0.025).
            let at = |x: f32, y: f32| o + r * x - u * y;
            let y0 = if text.plain() == "deadmau5" {
                -10.0
            } else {
                0.0
            };
            batch.vertices.clear();
            // Font.prepareText, as any text: bold, italic, fonts and each
            // glyph's own box (accented.png's are 12 pixels tall).
            let behind = batch.name_tag(
                &self.gui.atlas,
                text,
                y0,
                text_alpha << 24 | 0xFF_FFFF,
                if background { BACKGROUND } else { 0 },
            );
            for (i, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
                let shift = if i * 6 < behind {
                    away
                } else {
                    glam::Vec3::ZERO
                };
                // The corners from the bottom left, as the world draws quads.
                for v in [quad[5], quad[2], quad[1], quad[0]] {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: (at(v.pos[0], v.pos[1]) + shift).to_array(),
                        uv: v.uv,
                        layer: text_layer,
                        color: v.color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
        // Glyphs rasterized on first use (Unifont's) upload with the frame.
        batch.patches
    }
}

/// A name tag: where it hangs, its lines (text, lift), the packed light at
/// the entity and whether it is discrete.
struct NameTag {
    at: glam::DVec3,
    lines: Vec<(Text, f64)>,
    light: u8,
    discrete: bool,
}
