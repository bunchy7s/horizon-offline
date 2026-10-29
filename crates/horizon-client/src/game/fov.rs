//! Field of view (`Camera.tickFov`, `calculateFov`,
//! `AbstractClientPlayer.getFieldOfViewModifier`), view and hurt bobbing
//! (`GameRenderer.bobHurt`, `bobView`), the portal and Nausea spin
//! (`GameRenderer.renderLevel`) and the spyglass (`SpyglassItem`,
//! `Hud.extractSpyglassOverlay`). The arithmetic is `horizon_client::camera`.
use super::Game;
use horizon_client::{
    camera::{
        BobState, CameraFluid, FovPlayer, HurtBob, UseItem, bob_matrix, field_of_view_modifier,
        spinning_effect_matrix,
    },
    fog::{FogType, fog_type},
    hud::HudBatch,
    hud_layout, hud_overlays,
};

/// `Abilities.walkingSpeed`.
const WALKING_SPEED: f32 = 0.1;

impl Game<'_> {
    /// `Player.isScoping`: using a spyglass.
    pub(super) fn scoping(&self) -> bool {
        self.using
            .and_then(|(slot, _)| self.stack_in(slot))
            .is_some_and(|s| &*s.item == "minecraft:spyglass")
    }
    /// `options.fovEffectScale`.
    pub(super) fn fov_effect_scale(&self) -> f32 {
        self.settings.file.double("fovEffectScale") as f32
    }
    /// `options.screenEffectScale` (Distortion Effects).
    pub(super) fn screen_effect_scale(&self) -> f32 {
        self.settings.file.double("screenEffectScale") as f32
    }
    /// `LocalPlayer.getEffectBlendFactor(NAUSEA, partial)`: 0 without the effect.
    pub(super) fn nausea_blend(&self, partial: f32) -> f32 {
        if self.potions.effects.get("nausea").is_none() {
            return 0.0;
        }
        let fx = &self.screen_fx;
        fx.o_nausea + partial * (fx.nausea - fx.o_nausea)
    }
    /// The spin `GameRenderer.renderLevel` multiplies into the level's
    /// projection: the larger of the portal and Nausea intensities, scaled by
    /// Distortion Effects squared, turning by `LocalPlayer.getSpinningEffectAngle`.
    pub(super) fn spinning_matrix(&self, partial: f32) -> glam::Mat4 {
        let fx = &self.screen_fx;
        let portal = fx.o_portal + partial * (fx.portal - fx.o_portal);
        let scale = self.screen_effect_scale();
        let intensity = portal.max(self.nausea_blend(partial)) * (scale * scale);
        spinning_effect_matrix(intensity, fx.spinning_time + partial * fx.spinning_speed)
    }
    /// What `AbstractClientPlayer.getFieldOfViewModifier` reads.
    fn fov_player(&self) -> Option<FovPlayer> {
        let p = self.player.as_ref()?;
        let using = self.using.and_then(|(slot, ticks)| {
            let stack = self.stack_in(slot)?;
            let item = match &*stack.item {
                "minecraft:bow" => UseItem::Bow,
                "minecraft:spyglass" => UseItem::Spyglass,
                _ => UseItem::Other,
            };
            Some((item, ticks as i32))
        });
        Some(FovPlayer {
            flying: p.flying,
            walking_speed: WALKING_SPEED,
            // Player.aiStep keeps `speed` as the float movement speed attribute.
            movement_speed: f64::from(p.speed),
            using,
        })
    }
    /// `Camera.tick`: the eye height and the field-of-view modifier ease
    /// halfway toward the player's each tick.
    pub(super) fn tick_fov(&mut self) {
        let first_person = self.camera_type.first_person();
        let effect = self.fov_effect_scale();
        let target = self
            .fov_player()
            .map_or(1.0, |p| field_of_view_modifier(&p, first_person, effect));
        let target = self.zoomed_fov_target(target);
        let eye = self.player.as_ref().map(|p| {
            if self.sleeping.is_some() {
                0.2
            } else {
                p.dimensions.eye_height
            }
        });
        self.view.tick(eye, target);
    }
    /// The player's `deathTime` while dead or dying: whole ticks counted by
    /// `LivingEntity.tickDeath`, so `deathTime + partialTicks` in
    /// `Camera.extractRenderState` rises smoothly across frames.
    pub(super) fn death_time(&self) -> Option<i32> {
        let p = self.player.as_ref()?;
        p.health
            .dead_or_dying()
            .then(|| self.ui.death_time.min(i32::MAX as u32) as i32)
    }
    /// The fluid at the camera, as `Camera.getFluidInCamera` reports it.
    pub(super) fn camera_fluid(&self, camera: [f64; 3]) -> CameraFluid {
        match fog_type(&self.world, self.res.facts(), camera, self.forward()) {
            FogType::Water => CameraFluid::Water,
            FogType::Lava => CameraFluid::Lava,
            FogType::PowderSnow => CameraFluid::PowderSnow,
            FogType::Atmospheric => CameraFluid::None,
        }
    }
    /// `Camera.calculateFov` with `modifyFovBasedOnDeathOrFluid`.
    pub(super) fn fov_degrees(&self, partial: f32, camera: [f64; 3]) -> f32 {
        let fov = self.view.calculate_fov(
            partial,
            self.settings.fov as i32,
            self.fov_effect_scale(),
            self.death_time(),
            self.camera_fluid(camera),
            false,
        );
        self.zoomed_fov_degrees(fov)
    }
    /// `Camera.calculateHudFov`: the first-person hand's field of view.
    pub(super) fn hud_fov_degrees(&self, partial: f32, camera: [f64; 3]) -> f32 {
        self.view.calculate_hud_fov(
            partial,
            self.fov_effect_scale(),
            self.death_time(),
            self.camera_fluid(camera),
        )
    }
    /// The pose `GameRenderer.renderLevel` multiplies into the projection:
    /// `bobHurt`, then `bobView` with the View Bobbing option.
    pub(super) fn bob_matrix(&self, partial: f32) -> glam::Mat4 {
        let Some(p) = self.player.as_ref() else {
            return glam::Mat4::IDENTITY;
        };
        let hurt = &p.health.hurt;
        let state = BobState {
            living: Some(HurtBob {
                hurt_time: p.health.hurt_animation_time as f32 - partial,
                hurt_duration: hurt.hurt_duration,
                hurt_dir: p.health.hurt_dir,
                death_time: self.death_time().map(|t| t as f32 + partial),
            }),
            player: Some((
                self.avatar.backwards_interpolated_walk_distance(partial),
                self.avatar.interpolated_bob(partial),
            )),
        };
        let tilt = self.settings.file.double("damageTiltStrength");
        glam::Mat4::from_cols_array(&bob_matrix(&state, tilt, self.settings.view_bobbing).m)
    }
    /// `AbstractClientPlayer.tick` and `aiStep` (`ClientAvatarState.tick`,
    /// `updateBob`), `rideTick`'s `resetBob` and `LocalPlayer.move`'s walked
    /// distance, after the player's movement this tick. `before` is the
    /// ground state, velocity and position at the start of the tick.
    pub(super) fn tick_avatar(&mut self, before: (bool, [f64; 3], [f64; 3]), riding: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (ground, velocity, start) = before;
        let swimming = p.pose == horizon_client::pose::Pose::Swimming;
        let alive = !p.health.dead_or_dying();
        let (dx, dz) = (
            (p.position.x - start[0]) as f32,
            (p.position.z - start[2]) as f32,
        );
        self.avatar.tick(start);
        self.avatar
            .update_bob(ground && alive && !swimming, velocity);
        if riding {
            self.avatar.reset_bob();
        } else {
            self.avatar.add_walked(dx, dz);
        }
    }
    /// `SpyglassItem.use`: start scoping.
    pub(super) fn use_spyglass(&mut self) {
        if self.using.is_none() {
            self.using = Some((self.held_slot(), 0));
            let at = self.eye_position();
            self.audio.play("minecraft:item.spyglass.use", at, 1.0, 1.0);
        }
    }
    /// `SpyglassItem.releaseUsing` / `finishUsingItem` (after 1200 ticks).
    pub(super) fn stop_spyglass(&mut self) {
        let at = self.eye_position();
        self.audio
            .play("minecraft:item.spyglass.stop_using", at, 1.0, 1.0);
    }
    /// `Hud.extractCameraOverlays`: the spyglass scope, its size easing
    /// towards 1.125 by half the frame's game ticks and reset to 0.5 while
    /// not scoping in first person.
    pub(super) fn draw_spyglass(&mut self, batch: &mut HudBatch) {
        self.scope_scale += 0.5 * self.frame_ticks * (1.125 - self.scope_scale);
        if !self.camera_type.first_person() {
            return;
        }
        if !self.scoping() {
            self.scope_scale = 0.5;
            return;
        }
        let draws =
            hud_overlays::spyglass(batch.width as i32, batch.height as i32, self.scope_scale);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}
