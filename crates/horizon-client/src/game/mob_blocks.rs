//! Blocks mobs carry: block items in a hand (`ItemInHandLayer`) and the
//! block an enderman holds (`CarriedBlockLayer`), drawn as baked block models.
//! Also the block models mobs wear (`SnowGolemHeadLayer`,
//! `MushroomCowMushroomLayer`).
use super::Game;
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::{Mob, Species, render},
};

/// `CustomHeadLayer.translateToHead` and the banner's `head` display.
fn banner_matrix(head: glam::Mat4) -> glam::Mat4 {
    head * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0))
        * glam::Mat4::from_rotation_y(std::f32::consts::PI)
        * glam::Mat4::from_scale(glam::Vec3::new(0.625, -0.625, -0.625))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.0, 7.0 / 16.0))
        * glam::Mat4::from_rotation_y(std::f32::consts::PI)
        * glam::Mat4::from_scale(glam::Vec3::splat(1.5))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.5, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0))
}

/// `ElderGuardianParticle.lifetime`.
pub(super) const APPARITION_TICKS: i32 = 30;

impl Game<'_> {
    /// Count the elder guardian's apparition down.
    pub(super) fn tick_apparition(&mut self) {
        self.creatures.elder_apparition = (self.creatures.elder_apparition - 1).max(0);
    }
    /// `ElderGuardianParticleGroup`: the ghostly elder guardian that swings
    /// up in front of the camera while the curse is laid, fading in and out.
    pub(super) fn elder_apparition_geometry(&self, alpha: f64, out: &mut Vec<Vertex>) {
        let left = self.creatures.elder_apparition;
        if left <= 0 {
            return;
        }
        let Some(kind) = self.res.mob_types().get("minecraft:elder_guardian") else {
            return;
        };
        let Some(r) = self.gui.atlas.sprites.get("entity/guardian_elder").copied() else {
            return;
        };
        let age = (APPARITION_TICKS - left) as f32 + alpha as f32;
        let t = (age / APPARITION_TICKS as f32).clamp(0.0, 1.0);
        let fade = 0.05 + 0.5 * (t * std::f32::consts::PI).sin();
        let color = (((fade * 255.0) as u32) << 24) | 0x00FF_FFFF;
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        // The camera's rotation: local +x right, +y up, +z behind.
        let camera = glam::Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            (-forward).extend(0.0),
            glam::Vec4::W,
        );
        let k = 0.425_531_92;
        let m = camera
            * glam::Mat4::from_rotation_x((60.0 - 150.0 * t).to_radians())
            * glam::Mat4::from_scale(glam::Vec3::new(k, -k, -k))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.56, 3.5));
        let poses = horizon_client::mobs::anim::Poses::new(&kind.model);
        render::draw(
            &kind.model,
            m,
            &poses,
            [r.u0, r.v0, r.u1, r.v1],
            color,
            240 | 240 << 8,
            0,
            out,
        );
    }
    /// The ominous banner a raid captain wears on its head: the banner model
    /// with its eight pattern layers (`CustomHeadLayer`).
    fn captain_banner_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use horizon_client::mobs::{Part, anim::Poses, load_model};
        static MODELS: std::sync::OnceLock<Option<(Part, Part)>> = std::sync::OnceLock::new();
        if !horizon_client::mobs::raider::is_captain(mob) {
            return;
        }
        let Some((pole, flag)) = MODELS.get_or_init(|| {
            let root = &self.options.resources;
            Some((
                load_model(root, "minecraft:standing_banner#main")?,
                load_model(root, "minecraft:standing_banner#flag")?,
            ))
        }) else {
            return;
        };
        let Some(head) =
            render::part_transform(mob, alpha as f32, camera, self.res.mob_data(), "head")
        else {
            return;
        };
        let m = banner_matrix(head);
        let atlas = &self.gui.atlas;
        let uv = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // BannerRenderer: the flag sways with a per-banner phase.
        let phase =
            (((u64::from(mob.id) * 7 + u64::from(mob.age)) % 100) as f32 + alpha as f32) / 100.0;
        let mut flag_poses = Poses::new(flag);
        flag_poses.xrot(
            "flag",
            (-0.0125 + 0.01 * (2.0 * std::f32::consts::PI * phase).cos()) * std::f32::consts::PI,
        );
        let still = Poses::new(pole);
        let white = 0xFFFF_FFFF;
        if let Some(r) = uv("entity/banner/banner_base") {
            render::draw(pole, m, &still, r, white, light, 0, out);
            render::draw(flag, m, &flag_poses, r, white, light, 0, out);
        }
        let layers =
            std::iter::once(("base", "white")).chain(horizon_client::item_data::OMINOUS_LAYERS);
        for (pattern, dye) in layers {
            let Some(r) = uv(&format!("entity/banner/{pattern}")) else {
                continue;
            };
            let index = render::DYE_NAMES
                .iter()
                .position(|n| *n == dye)
                .unwrap_or(0);
            let color = render::abgr(render::DYE[index]);
            render::draw(flag, m, &flag_poses, r, color, light, 0, out);
        }
    }
    /// The blocks one mob holds or carries.
    pub(super) fn mob_block_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        self.captain_banner_geometry(mob, camera, alpha, light, out);
        self.held_item_geometry(mob, camera, alpha, light, true, out);
        self.worn_block_geometry(mob, camera, alpha, light, out);
        self.golem_flower_geometry(mob, camera, alpha, light, out);
        if mob.species() == Species::Enderman
            && let Some(state) = mob.extra.carried
            && let Some((_, tint)) = self.block_face_sprite(state)
        {
            // CarriedBlockLayer: OverlayTexture.NO_OVERLAY, never the hurt tint;
            // the block's baked model (seed 42) on the unit cube, not centred.
            let base = render::base_transform(mob, alpha as f32, camera);
            let t = base
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.6875, -0.75))
                * glam::Mat4::from_rotation_x(20.0f32.to_radians())
                * glam::Mat4::from_rotation_y(45.0f32.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.25, 0.1875, 0.25))
                * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5))
                * glam::Mat4::from_rotation_y(90.0f32.to_radians());
            self.block_model_vertices_tinted(state, 42, t, light, true, tint, out);
        }
    }
    /// `IronGolemFlowerLayer`: the poppy in the right arm while the golem
    /// offers it (`getOfferFlowerTick() > 0`), drawn with `NO_OVERLAY`.
    fn golem_flower_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use glam::{Mat4, Vec3};
        if mob.species() != Species::IronGolem
            || mob.extra.counter <= 0
            || horizon_client::mobs::trader::is_invisible(mob)
        {
            return;
        }
        let Some(state) = self.pack.assets.registry.default_state("minecraft:poppy") else {
            return;
        };
        let Some(arm) =
            render::part_transform(mob, alpha as f32, camera, self.res.mob_data(), "right_arm")
        else {
            return;
        };
        let t = arm
            * Mat4::from_translation(Vec3::new(-1.1875, 1.0625, -0.9375))
            * Mat4::from_translation(Vec3::splat(0.5))
            * Mat4::from_scale(Vec3::splat(0.5))
            * Mat4::from_rotation_x((-90.0f32).to_radians())
            * Mat4::from_translation(Vec3::splat(-0.5));
        self.block_model_vertices(state, 42, t, light, true, out);
    }
    /// `SnowGolemHeadLayer` and `MushroomCowMushroomLayer`: the carved pumpkin
    /// on a snow golem's head, and the mushrooms on an adult mooshroom's back
    /// and head, as baked block models.
    fn worn_block_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use glam::{Mat4, Vec3};
        let species = mob.species();
        let on_head = match species {
            Species::SnowGolem => !mob.extra.sheared,
            Species::Mooshroom => !mob.is_baby(),
            _ => false,
        };
        // The layers draw nothing on an invisible mob (the glow outline is not drawn).
        if !on_head || horizon_client::mobs::trader::is_invisible(mob) {
            return;
        }
        let registry = &self.pack.assets.registry;
        let block = match species {
            Species::SnowGolem => "minecraft:carved_pumpkin",
            _ if mob.extra.variant == 1 => "minecraft:brown_mushroom",
            _ => "minecraft:red_mushroom",
        };
        let Some(state) = registry.default_state(block) else {
            return;
        };
        let alpha = alpha as f32;
        let Some(head) = render::part_transform(mob, alpha, camera, self.res.mob_data(), "head")
        else {
            return;
        };
        let unit = Mat4::from_translation(Vec3::splat(-0.5));
        let flip = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
        let at = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
        let spin = |degrees: f32| Mat4::from_rotation_y(degrees.to_radians());
        let transforms = if species == Species::SnowGolem {
            vec![
                head * at(0.0, -0.34375, 0.0)
                    * spin(180.0)
                    * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
                    * unit,
            ]
        } else {
            let base = render::base_transform(mob, alpha, camera);
            vec![
                base * at(0.2, -0.35, 0.5) * spin(-48.0) * flip * unit,
                base * at(0.2, -0.35, 0.5)
                    * spin(42.0)
                    * at(0.1, 0.0, -0.6)
                    * spin(-48.0)
                    * flip
                    * unit,
                head * at(0.0, -0.7, -0.2) * spin(-78.0) * flip * unit,
            ]
        };
        // The hurt tint reaches these blocks as it does the body.
        let overlay = horizon_client::overlay::living(0.0, mob.hurt_time > 0 || mob.death_time > 0);
        for t in transforms {
            let start = out.len();
            self.block_model_vertices(state, 0, t, light, true, out);
            for vertex in &mut out[start..] {
                vertex.layer = overlay;
            }
        }
    }
    /// `ItemInHandLayer`: both hands through the item model pipeline; `lit`
    /// applies the level's diffuse lighting (off for GUI previews, which
    /// light the projected quads themselves).
    pub(super) fn held_item_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        lit: bool,
        out: &mut Vec<Vertex>,
    ) {
        // ItemInHandLayer submits with OverlayTexture.NO_OVERLAY: no hurt tint.
        for (item, off_hand) in [(&mob.extra.main_hand, false), (&mob.extra.off_hand, true)] {
            let Some(item) = item else { continue };
            let left = off_hand != mob.main_arm_left();
            let Some(t) =
                render::held_item_transform(mob, alpha as f32, camera, self.res.mob_data(), left)
            else {
                continue;
            };
            let slot = if off_hand {
                horizon_client::mobs::gear::OFF
            } else {
                horizon_client::mobs::gear::MAIN
            };
            let local_player = self
                .player_model
                .as_ref()
                .is_some_and(|p| p.id == mob.id && p.kind.id == mob.kind.id);
            let mut stack = if local_player {
                let Some(stack) = self.stack_in(if off_hand {
                    super::shield::OFFHAND_SLOT
                } else {
                    self.selected
                }) else {
                    continue;
                };
                stack.clone()
            } else {
                ItemStack::new(item, 1)
            };
            if !local_player && horizon_client::mobs::gear::enchanted(mob, slot) {
                // The gear's enchantments are not tracked, only that it has some.
                stack.enchant("minecraft:unbreaking", 1);
            }
            if &**item == "minecraft:crossbow" && mob.extra.flag2 {
                stack.data_mut().charged = vec![ItemStack::new("minecraft:arrow", 1)];
            }
            let mut owner = super::item_draw::owner_at(
                [mob.position.x, mob.position.y, mob.position.z],
                mob.body_yaw,
                &mob.kind.id,
            );
            // The local player's rod while its hook is out.
            owner.fishing_cast_with_this_item = self.cast_rod().is_some()
                && self.main_hand_rod() != off_hand
                && self
                    .player_model
                    .as_ref()
                    .is_some_and(|p| std::ptr::eq(p, mob));
            owner.main_arm_left = mob.main_arm_left();
            if off_hand == mob.extra.using_off_hand && mob.extra.using_item > 0 {
                owner.using_this_item = true;
                owner.use_duration = 72_000;
                owner.use_item_remaining_ticks = 72_000 - mob.extra.using_item;
                owner.crossbow_charge_duration = 25;
            }
            use horizon_assets::item_model::DisplayContext as Context;
            // PandaHoldsItemLayer and FoxHeldItemLayer show the item as it lies
            // on the ground.
            let (context, ordinal) = match (mob.species(), left) {
                (Species::Panda | Species::Fox, _) => (Context::Ground, 7),
                (_, true) => (Context::ThirdPersonLeftHand, 1),
                (_, false) => (Context::ThirdPersonRightHand, 2),
            };
            // `updateForLiving`: seeded by the entity id plus the context ordinal.
            let seed = mob.id as i32 + ordinal;
            let state = self.resolve_item(&stack, context, Some(&owner), seed);
            if lit {
                self.submit_item_world(&state, t, light, out);
            } else {
                self.submit_item_unlit(&state, t, light, out);
            }
        }
    }
}
