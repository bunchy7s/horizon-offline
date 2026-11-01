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

/// The world size of one text pixel in a name tag: `Matrix4f.scale(0.025F, -0.025F, 0.025F)`.
pub(super) const NAME_TAG_UNIT: f32 = 0.025;

impl Game<'_> {
    /// A named name tag names the mob under the crosshair.
    pub(super) fn name_mob(&mut self, index: usize) -> bool {
        let Some(name) = self
            .held()
            .filter(|h| &*h.item == "minecraft:name_tag")
            .and_then(|h| h.custom_name())
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
            || self.own_invisible()
        {
            return None;
        }
        let m = self.player_model.as_ref()?;
        Some((m, self.own_profile_name()))
    }
    /// The local player's profile name as others see it: the session's on a
    /// server (single player's server thread included), else the local player's.
    pub(super) fn own_profile_name(&self) -> String {
        match self.remote.as_ref() {
            Some(r) if !r.profile_name.is_empty() => r.profile_name.clone(),
            _ => self.acting_player_name().to_owned(),
        }
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
            // A mannequin with no name still shows its description.
            let name = match &m.custom_name {
                Some(name) => name.clone(),
                None if m.species() == horizon_client::mobs::Species::Mannequin => String::new(),
                None => return None,
            };
            // The tag's 64 block distance is measured to the entity: copy the
            // name only for a mob the frame can show it on.
            let at = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
            (at.distance_squared(camera) < 64.0 * 64.0).then_some((m, name, false))
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
            // EntityAttachment.NAME_TAG: 0.5 above the box of the current pose.
            let p = name_tag_at(m, alpha);
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
                self.mob_team_of(m)
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
            // `ClientMannequin.belowNameDisplay`: the description is `scoreText`, drawn
            // within the below-name distance (10) whether the name shows or not.
            let description = (m.species() == horizon_client::mobs::Species::Mannequin
                && distance_sqr < 10.0 * 10.0)
                .then(|| m.extra.description.component())
                .flatten()
                .map(|tag| horizon_client::entity_nbt::name_text(&tag).resolve_legacy());
            let name_shown = sb::name_shown(team, viewer, view)
                && (player || m.name_visible || Some(m.id) == target)
                && (player || m.custom_name.is_some());
            if !name_shown {
                if let Some(text) = description {
                    let l = self.world.light(
                        m.position.x.floor() as i32,
                        (m.position.y + f64::from(m.height())).floor() as i32,
                        m.position.z.floor() as i32,
                    );
                    tags.push(NameTag {
                        at: p,
                        lines: vec![(text, 0.0)],
                        light: l,
                        discrete,
                    });
                }
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
            if m.species() == horizon_client::mobs::Species::Mannequin {
                if let Some(text) = description {
                    lines.push((text, 0.0));
                    lift = f64::from(9.0f32 * 1.15 * 0.025);
                }
            } else if distance_sqr < 10.0 * 10.0
                && let Some(score) = holder.and_then(|h| self.below_name_text(&h))
            {
                lines.push((score.resolve_legacy(), 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            let l = self.world.light(
                m.position.x.floor() as i32,
                (m.position.y + f64::from(m.height())).floor() as i32,
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
            // The tag rides with the body, between last tick's position and this one's.
            let at = a.render_position(alpha);
            let l = self.world.light(
                at.x.floor() as i32,
                (at.y + height).floor() as i32,
                at.z.floor() as i32,
            );
            tags.push(NameTag {
                at: glam::DVec3::new(at.x, at.y + height + 0.5, at.z),
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
                patches.extend(self.name_tag_quads(
                    o,
                    &tag.lines,
                    FADED,
                    true,
                    0,
                    light,
                    NAME_TAG_UNIT,
                    out,
                ));
                continue;
            }
            // LightCoordsUtil.lightCoordsWithEmission(lightCoords, 2).
            let lit = (tag.light >> 4).max(2) << 4 | (tag.light & 15).max(2);
            self.mark_entity(out.len(), "minecraft:name_tag");
            let light = vertex_light(lit);
            patches.extend(self.name_tag_quads(
                o,
                &tag.lines,
                0xFF,
                false,
                0,
                light,
                NAME_TAG_UNIT,
                out,
            ));
            // RenderPipelines.TEXT_SEE_THROUGH: no depth test, no lightmap.
            patches.extend(self.name_tag_quads(
                o,
                &tag.lines,
                FADED,
                true,
                2,
                0,
                NAME_TAG_UNIT,
                see_through,
            ));
        }
        self.map_patches.extend(patches);
    }
    /// The lines of one name tag, `o` from the camera: the text at
    /// `text_alpha` in its layer, on the translucent black background
    /// (in `text_layer` like the glyphs, a hair behind) when `background`.
    /// Returns the glyph pixels the atlas still needs.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn name_tag_quads(
        &self,
        o: glam::DVec3,
        lines: &[(Text, f64)],
        text_alpha: u32,
        background: bool,
        text_layer: u32,
        light: u32,
        unit: f32,
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
            let (r, u) = (right.as_vec3() * unit, up.as_vec3() * unit);
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

/// `EntityAttachment.NAME_TAG`: `(0, bbHeight, 0)` of the box of the entity's
/// current pose, which the submitter lifts 0.5 more, between the last two tick
/// positions. The player box is `Pose::dimensions(1.0)`.
/// shortcut: the scale attribute is not in the tag height, add `model_scale`
/// when tags on scaled players matter.
fn name_tag_at(m: &Mob, alpha: f64) -> glam::DVec3 {
    glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * alpha,
        m.previous.y + (m.position.y - m.previous.y) * alpha + f64::from(m.height()) + 0.5,
        m.previous.z + (m.position.z - m.previous.z) * alpha,
    )
}

/// A name tag: where it hangs, its lines (text, lift), the packed light at
/// the entity and whether it is discrete.
struct NameTag {
    at: glam::DVec3,
    lines: Vec<(Text, f64)>,
    light: u8,
    discrete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mobs::{Anim, Brain, MobType, Part, Species};
    use horizon_client::pose::Pose;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// The player model `perspective::player_type` builds from the prepared
    /// resources, without them: its box only needs the id and the pointer.
    fn player_mob() -> Mob {
        let part = Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children: Vec::new(),
        };
        let kind = MobType {
            id: "minecraft:player".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            model: Arc::new(part),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: false },
            sounds: [
                "entity.player.hurt",
                "entity.player.hurt",
                "entity.player.death",
            ],
            step_sound: None,
            food: Default::default(),
            tempt: Default::default(),
        };
        Mob::new(0, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// #4129: the tag hangs from the box of the pose (0.6 swimming, 1.5
    /// crouching, 1.8 standing) and never from the sneak flag, which a
    /// sprint-swimming or gliding player still has set. Today's
    /// `if player && m.extra.crouching { 1.5 }` gives 2.0 for the first.
    #[test]
    fn a_name_tag_hangs_from_the_pose_box_not_the_sneak_flag() {
        let mut m = player_mob();
        let at_y = |m: &Mob| name_tag_at(m, 1.0).y;
        m.extra.crouching = true;
        m.extra.avatar_pose = Pose::Swimming;
        assert!((at_y(&m) - 1.1).abs() < 1e-6, "0.6 + 0.5, not 2.0");
        // A crouching player's tag keeps the height it had: 1.5 + 0.5.
        m.extra.avatar_pose = Pose::Crouching;
        assert!((at_y(&m) - 2.0).abs() < 1e-6, "1.5 + 0.5, as before");
        m.extra.avatar_pose = Pose::Standing;
        m.extra.crouching = false;
        assert!((at_y(&m) - 2.3).abs() < 1e-6, "1.8 + 0.5");
    }
}
