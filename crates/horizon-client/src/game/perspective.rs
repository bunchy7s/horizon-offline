//! The camera type (`CameraType`: first person, third person behind and
//! in front, cycled by F5), the detached camera (`Camera.setup` with
//! `getMaxZoom`), and the local player drawn as a model: in third person
//! and in the inventory (`InventoryScreen.extractEntityInInventoryFollowsMouse`).
use super::{Game, Screen};
use horizon_client::{
    blocks::BlockFacts,
    camera::{CameraEntity, LivingCamera},
    collisions::SceneCollisions,
};
use horizon_client::{
    hud::HudBatch,
    mobs::{Anim, Brain, Mob, MobType},
    mouse::Rotation,
    pose::Pose,
};
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::EntityIdentity,
    registry::BlockStateId,
    world_ray::{RayBlockMode, RayFluidMode, RayWorld, clip_world},
};
use std::{io, path::Path, sync::Arc};

/// `Level.clip` with `ClipContext.Block.VISUAL` and no fluids:
/// `BlockBehaviour.getVisualShape` is the collision shape except for the
/// blocks that override it.
struct VisualRays<'a, 'r> {
    scene: SceneCollisions<'a, 'r>,
    facts: &'a BlockFacts,
    context: CollisionContext<'static>,
}
impl RayWorld for VisualRays<'_, '_> {
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = ();
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.scene.world.block(p.x, p.y, p.z))
    }
    fn fluid_state(&mut self, _: BlockPos) -> io::Result<()> {
        Ok(())
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        _: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        let facts = self.facts.get(state);
        let name = &*facts.name;
        // TransparentBlock (glass, stained and tinted glass, copper grates),
        // IronBarsBlock (bars and panes) and PowderSnowBlock: nothing;
        // SoulSandBlock and MudBlock: a full block; SnowLayerBlock and
        // FenceBlock: their outline.
        if name.ends_with("glass")
            || name.ends_with("glass_pane")
            || name.ends_with("_bars")
            || name.ends_with("copper_grate")
            || name == "minecraft:powder_snow"
        {
            return Ok(VoxelShape::empty());
        }
        if name == "minecraft:soul_sand" || name == "minecraft:mud" {
            return Ok(VoxelShape::block());
        }
        if name == "minecraft:snow" || name.ends_with("_fence") {
            let mut shape = VoxelShape::empty();
            for b in facts.outline.iter() {
                let b = b.map(f64::from);
                let part = VoxelShape::from_box(Aabb::new(
                    Vec3::new(b[0], b[1], b[2]),
                    Vec3::new(b[3], b[4], b[5]),
                ))
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
                shape = shape
                    .join(&part, ShapeOp::OR)
                    .map_err(|e| io::Error::other(format!("{e:?}")))?;
            }
            return Ok(shape);
        }
        let binding = self.scene.binding;
        binding
            .collision_shape_with_context(&mut self.scene, state, p, &self.context)
            .map_err(|e| io::Error::other(format!("{e:?}")))
    }
    fn interaction_shape(&mut self, _: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
    fn fluid_is_source(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_is_empty(&mut self, _: ()) -> io::Result<bool> {
        Ok(true)
    }
    fn fluid_is_water(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}
impl CameraType {
    pub(super) fn first_person(self) -> bool {
        self == CameraType::FirstPerson
    }
    fn cycle(self) -> Self {
        match self {
            CameraType::FirstPerson => CameraType::ThirdPersonBack,
            CameraType::ThirdPersonBack => CameraType::ThirdPersonFront,
            CameraType::ThirdPersonFront => CameraType::FirstPerson,
        }
    }
}
/// `LivingEntityRenderer.isBodyVisible` and `isInvisibleTo` for the player's
/// own body, seen by itself: the Invisibility effect (`Entity.isInvisible`)
/// hides it unless its team sees friendly invisibles, where vanilla draws it
/// translucent.
fn body_visible(
    effects: &horizon_client::effects::PlayerEffects,
    team: Option<&horizon_client::scoreboard::Team>,
) -> bool {
    let invisible = effects.get("invisibility").is_some();
    !horizon_client::scoreboard::invisible_to(team, team, false, invisible)
}
/// The player's model type (`PlayerModel` with the default skin).
pub(super) fn player_type(root: &Path) -> Option<Arc<MobType>> {
    let models = horizon_client::mobs::EntityModelDocument::load(root).ok()?;
    models.model("minecraft:player#main").map(|model| {
        Arc::new(MobType {
            id: "minecraft:player".into(),
            // Players render as a humanoid; the species only drives local AI.
            species: horizon_client::mobs::Species::Zombie,
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
            model: Arc::new(model),
            // PlayerModel's slim arms, for slim skins, and PlayerCapeModel.
            layers: ["player_slim#main", "player#cape"]
                .into_iter()
                .filter_map(|key| {
                    let model = models.model(&format!("minecraft:{key}"))?;
                    Some((key.to_owned(), Arc::new(model)))
                })
                .collect(),
            texture: "entity/player/wide/steve".into(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: false },
            sounds: [
                "entity.player.hurt",
                "entity.player.hurt",
                "entity.player.death",
            ],
            step_sound: None,
            food: Arc::from([]),
            tempt: Arc::from([]),
        })
    })
}
fn wrap(d: f32) -> f32 {
    let d = d % 360.0;
    if d >= 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}
impl Game<'_> {
    /// `key.togglePerspective`.
    pub(super) fn cycle_camera(&mut self) {
        self.camera_type = self.camera_type.cycle();
    }
    /// `Camera.update` (`alignWithEntity` and `getMaxZoom`): the
    /// interpolated eye, or a point up to the camera distance behind (or in
    /// front of) it, pulled in where visual shapes are in the way. Returns the
    /// position, yaw and pitch to render from.
    pub(super) fn camera_view(&mut self, partial: f32) -> Option<(glam::DVec3, f32, f32)> {
        let (scale, distance) = {
            let p = self.player.as_mut()?;
            (
                p.attribute("minecraft:scale").unwrap_or(1.0) as f32,
                p.attribute("minecraft:camera_distance").unwrap_or(4.0),
            )
        };
        let p = self.player.as_ref()?;
        let sleeping = self.sleeping.map(|head| {
            let state = self.world.block(head[0], head[1], head[2]);
            let props = self
                .pack
                .assets
                .registry
                .state(state)
                .map(|s| &s.properties);
            // BedBlock.getBedOrientation as Direction.toYRot.
            props.and_then(|props| match props.get("facing").map(String::as_str) {
                Some("south") => Some(0.0),
                Some("west") => Some(90.0),
                Some("north") => Some(180.0),
                Some("east") => Some(270.0),
                _ => None,
            })
        });
        // Camera.setup: the living mount's scale and camera distance
        // attributes (the happy ghast's is 8, the default 4).
        let mount = self
            .riding
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .map(|m| {
                let attribute = |name: &str, default: f64| {
                    m.kind.attributes.get(name).copied().unwrap_or(default)
                };
                (
                    attribute("minecraft:scale", 1.0) as f32,
                    attribute("minecraft:camera_distance", 4.0),
                )
            });
        // AbstractBoat.positionRider turns a passenger by the boat's
        // deltaRotation after its yRotO was saved, and LocalPlayer.getViewYRot
        // interpolates a passenger's yaw: the boat's turn is spread over the
        // frames of a tick.
        let ride_turn = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map_or(0.0, |b| b.delta_rotation);
        let entity = CameraEntity {
            position: [p.position.x, p.position.y, p.position.z],
            previous: [
                self.previous_position.x,
                self.previous_position.y,
                self.previous_position.z,
            ],
            // Entity.turn moves the old rotation with the new: the mouse
            // turns the view every frame without interpolation.
            x_rot: self.pitch,
            y_rot: self.yaw,
            x_rot_o: self.pitch,
            y_rot_o: self.yaw - ride_turn,
            eye_height: p.dimensions.eye_height,
            living: Some(LivingCamera {
                scale,
                camera_distance: distance,
                sleeping,
                dead: None,
            }),
            mount,
        };
        let mut view = self.view;
        let kind = match self.camera_type {
            CameraType::FirstPerson => horizon_client::camera::CameraType::FirstPerson,
            CameraType::ThirdPersonBack => horizon_client::camera::CameraType::ThirdPersonBack,
            CameraType::ThirdPersonFront => horizon_client::camera::CameraType::ThirdPersonFront,
        };
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: p.position.y,
                descending: p.shift_key_down,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        );
        let mut rays = VisualRays {
            scene: SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            },
            facts: self.res.facts(),
            context,
        };
        view.align_with_entity(&entity, partial, kind, |from, to| {
            let hit = clip_world(
                &mut rays,
                Vec3::new(from[0], from[1], from[2]),
                Vec3::new(to[0], to[1], to[2]),
                RayBlockMode::Visual,
                RayFluidMode::None,
                4096,
            )
            .ok()?;
            (!hit.miss).then_some([hit.hit.location.x, hit.hit.location.y, hit.hit.location.z])
        });
        self.view = view;
        Some((
            glam::DVec3::from_array(view.position),
            view.y_rot,
            view.x_rot,
        ))
    }
    /// Keep the player's model in step with the player (`LivingEntity.tick`
    /// body turning, `WalkAnimationState`, the swing and the pose).
    pub(super) fn tick_player_model(&mut self) {
        let scale = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:scale").ok())
            .unwrap_or(1.0) as f32;
        let Some(p) = self.player.as_ref() else {
            self.player_model = None;
            return;
        };
        if self.player_model.is_none() {
            let Some(kind) = self.res.player_type().clone() else {
                return;
            };
            let mut m = Mob::new(0, kind, p.position, self.yaw);
            m.head_pitch = self.pitch;
            self.player_model = Some(m);
        }
        let crouching = p.pose == Pose::Crouching;
        let position = p.position;
        let (fall_flying, fly_ticks, velocity) = (p.fall_flying, p.fall_fly_ticks, p.velocity);
        let spinning = p.auto_spin_attack_ticks > 0;
        let using = self.using.map(|(_, ticks)| ticks as i32 + 1).unwrap_or(0);
        let using_off_hand = self
            .using
            .is_some_and(|(slot, _)| slot == super::shield::OFFHAND_SLOT);
        let blocking = self
            .using
            .and_then(|(slot, _)| self.stack_in(slot))
            .is_some_and(|stack| self.blocks_attacks(stack).is_some());
        let charged = self.inventory[self.selected]
            .as_ref()
            .is_some_and(|s| s.data().is_some_and(|d| !d.charged.is_empty()));
        let charge_duration = self.inventory[self.selected]
            .as_ref()
            .map_or(25, |s| Self::charge_ticks(s).max(1)) as i32;
        let attack = (self.swing.old_animation, self.swing.animation);
        let swinging = self.swing.swinging();
        // currentSwing.hand().asArm(mainArm).
        let swing_right_arm = swinging.then(|| {
            (self.swing.attack_hand() == horizon_client::first_person::Hand::Main)
                != self.settings.main_hand_left
        });
        // LivingEntity.isVisuallySwimming and Entity.isInWater.
        let visually_swimming =
            p.pose == Pose::Swimming || (!p.fall_flying && p.pose == Pose::FallFlying);
        let in_water = p.fluids.was_touching_water;
        let (yaw, pitch) = (self.yaw, self.pitch);
        let main = self.inventory[self.selected].clone();
        let off = self.offhand.as_ref().map(|s| s.item.clone());
        let armor = self.armor.clone();
        let riding = self.riding.is_some();
        let hurt = p.health.hurt_animation_time;
        let avatar = self.avatar;
        let previous_position = self.previous_position;
        // LivingEntity.getBedOrientation: the model lies in the bed as
        // another player's does (`villager::sleep_pose`).
        let bed = self.sleeping.map_or(0, |head| self.bed_facing(head));
        let Some(m) = self.player_model.as_mut() else {
            return;
        };
        m.extra.ints[1] = bed;
        m.extra.model_scale = scale;
        // The crouch flag is still last tick's here.
        let (from, at) =
            player_model_positions(previous_position, position, m.extra.crouching, crouching);
        m.previous = from;
        m.position = at;
        let (dx, dz) = (at.x - m.previous.x, at.z - m.previous.z);
        let walked = player_walk_distance(m.previous, at);
        m.previous_yaw = m.yaw;
        m.previous_body_yaw = m.body_yaw;
        m.previous_head_yaw = m.head_yaw;
        m.previous_head_pitch = m.head_pitch;
        m.yaw = yaw;
        m.head_yaw = yaw;
        m.head_pitch = pitch;
        let mut target = m.body_yaw;
        if dx * dx + dz * dz > 0.002_500_000_2 {
            // LivingEntity.tick: walking backwards the body faces away from
            // the walk direction, not along it.
            let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
            let facing = (wrap(yaw) - walk).abs();
            target = if 95.0 < facing && facing < 265.0 {
                walk - 180.0
            } else {
                walk
            };
        }
        if swinging {
            target = yaw;
        }
        // tickHeadTurn: ease toward the target, within 50 degrees of the head.
        m.body_yaw += wrap(target - m.body_yaw) * 0.3;
        let diff = wrap(yaw - m.body_yaw);
        if diff.abs() > 50.0 {
            m.body_yaw += diff - diff.signum() * 50.0;
        }
        // WalkAnimationState.update(min(distance * 4, 1), 0.4).
        m.previous_limb_speed = m.limb_speed;
        m.limb_speed += ((walked * 4.0).min(1.0) - m.limb_speed) * 0.4;
        m.limb_position += m.limb_speed;
        m.age = m.age.wrapping_add(1);
        m.anim.attack_anim_o = attack.0;
        m.anim.attack_anim = attack.1;
        m.extra.set_main_hand_stack(main);
        m.extra.off_hand = off;
        m.extra.head = self.armor[0].as_ref().map(|s| s.item.clone());
        m.extra.chest = self.armor[1].as_ref().map(|s| s.item.clone());
        m.extra.legs = self.armor[2].as_ref().map(|s| s.item.clone());
        m.extra.feet = self.armor[3].as_ref().map(|s| s.item.clone());
        // The worn stacks, for the elytra's glint (`WingsLayer`).
        m.extra.armor = armor;
        m.extra.crouching = crouching;
        m.extra.passenger = riding;
        m.velocity = velocity;
        m.extra.fall_flying = fall_flying;
        m.extra.fall_fly_ticks = fly_ticks;
        m.extra.auto_spin_attack = spinning;
        m.extra.main_hand_left = self.settings.main_hand_left;
        m.extra.hidden_model_parts = !self.settings.file.model_customisation() & 0x7F;
        m.extra.using_item = using;
        m.extra.using_off_hand = using_off_hand;
        m.extra.blocking = blocking;
        m.extra.charged = charged;
        m.extra.ints[5] = charge_duration;
        m.extra.elytra.tick(fall_flying, crouching, velocity);
        m.extra.swim.tick(visually_swimming, in_water);
        // The local player's ClientAvatarState, its cloak in the model's frame
        // (the model stands 0.125 lower while crouching).
        m.extra.avatar = avatar;
        if crouching {
            m.extra.avatar.cloak[1] -= 0.125;
            m.extra.avatar.cloak_o[1] -= 0.125;
        }
        m.extra.swing_right_arm = swing_right_arm;
        m.extra.swing_stab = self.swing.current.is_some_and(|c| c.animation.kind == 2);
        m.hurt_time = hurt;
    }
    /// `Entity.turn` for the player, by the mouse, a pad, the arrow keys or an
    /// agent's `turn`.
    /// The model's old pitch is the player's `xRotO`, which the turn moves
    /// with `xRot` (`turn_rotation`).
    pub(super) fn turn_player(&mut self, xo: f64, yo: f64) {
        let pitch_o = self.player_model.as_ref().map(|m| m.previous_head_pitch);
        let rotation = turn_rotation(self.yaw, self.pitch, pitch_o, xo, yo);
        self.yaw = rotation.y_rot;
        self.pitch = rotation.x_rot;
        if let Some(m) = self.player_model.as_mut() {
            m.previous_head_pitch = rotation.x_rot_o;
            m.head_pitch = rotation.x_rot;
        }
        // Entity.turn ends in AbstractBoat.onPassengerTurned: clampRotation
        // keeps the view within 105 degrees of the boat at every turn, not
        // only at the tick, so it never swings past and back.
        if let Some(b) = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
        {
            let delta = (self.yaw - b.yaw + 180.0).rem_euclid(360.0) - 180.0;
            self.yaw += delta.clamp(-105.0, 105.0) - delta;
        }
    }
    /// `Entity.setOldRot` after a server sets the player's rotation
    /// (`handleRotatePlayer`): the model's head snaps to it instead of easing
    /// in over the next tick.
    pub(super) fn snap_player_model_pitch(&mut self) {
        let pitch = self.pitch;
        if let Some(m) = self.player_model.as_mut() {
            m.previous_head_pitch = pitch;
            m.head_pitch = pitch;
        }
    }
    /// `LevelExtractor`'s skip of the camera entity: the player's own body
    /// is drawn unless the camera is in first person, or while it sleeps.
    pub(super) fn own_body_in_view(&self) -> bool {
        !self.camera_type.first_person() || self.sleeping.is_some()
    }
    /// Whether the player's own model shows in the world view (its body or,
    /// while invisible, only its armour and held items).
    pub(super) fn player_model_shown(&self) -> bool {
        // A shader pack's shadow pass draws the player in first person too (shadowPlayer).
        (self.own_body_in_view() || self.pack_shadow_casters.1)
            && !self.spectator()
            && self
                .player
                .as_ref()
                .is_some_and(|p| !p.health.dead_or_dying())
    }
    /// The player's own team: a server's client is in it by its profile name.
    fn own_team(&self) -> Option<&horizon_client::scoreboard::Team> {
        match &self.remote {
            Some(r) => self.score.board.players_team(&r.profile_name),
            None => self.player_team(),
        }
    }
    /// Whether the player's own body is drawn (`body_visible`).
    pub(super) fn own_body_visible(&self) -> bool {
        body_visible(&self.potions.effects, self.own_team())
    }
    /// The player's model for the entity pass in third person.
    pub(super) fn player_model_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        if !self.player_model_shown() {
            return;
        }
        let Some(m) = self.player_model.as_ref() else {
            return;
        };
        let light = self.world.light(
            m.position.x.floor() as i32,
            (m.position.y + 1.62).floor() as i32,
            m.position.z.floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let start = out.len();
        let body = self.own_body_visible();
        let render = if body {
            horizon_client::mobs::render::render_with
        } else {
            horizon_client::mobs::render::render_layers
        };
        render(
            m,
            alpha as f32,
            camera,
            &lookup,
            packed,
            self.res.mob_data(),
            &self.player_armor_layers(),
            out,
        );
        let chest = self.armor[1].as_ref().map(|s| &*s.item);
        // CapeLayer: no cape on an invisible player (`isInvisible`, whoever looks).
        if self.potions.effects.get("invisibility").is_none() {
            self.player_cape_geometry(m, chest, camera, alpha as f32, packed, out);
        }
        self.player_elytra_geometry(m, chest, camera, alpha as f32, packed, out);
        self.spin_attack_geometry(m, camera, alpha as f32, packed, out);
        self.held_item_geometry(m, camera, alpha, packed, true, out);
        // CustomHeadLayer: a pumpkin, skull or other head item.
        self.head_item_geometry(m, camera, alpha, packed, true, out);
        // LivingEntity.isCurrentlyGlowing: the Glowing effect.
        if self.potions.effects.get("glowing").is_some() {
            let color = super::glowing::team_color(self.own_team());
            // LivingEntityRenderer.getRenderType: an invisible glowing body
            // still draws its outline.
            if !body {
                let mut silhouette = Vec::new();
                horizon_client::mobs::render::render(
                    m,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self.res.mob_data(),
                    &mut silhouette,
                );
                self.mark_outline(&silhouette, color);
            }
            self.mark_outline(&out[start..], color);
        }
    }
    /// `extractEntityInInventoryFollowsMouse`: the player in the box
    /// `[x0, x1) x [y0, y1)`, `size` pixels per block, turned toward the
    /// cursor; drawn with a depth buffer and back-face culling.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_player_preview(
        &self,
        batch: &mut HudBatch,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        size: f32,
        offset_y: f32,
    ) {
        let Some(m) = self.player_model.as_ref() else {
            return;
        };
        if self
            .player
            .as_ref()
            .is_some_and(|p| p.health.dead_or_dying())
        {
            return;
        }
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let x_angle = ((cx - self.cursor.0) / 40.0).atan();
        let y_angle = ((cy - self.cursor.1) / 40.0).atan();
        let mut m = m.clone();
        // `extractEntityInInventory` divides the size by the entity's scale: the
        // preview is the same size whatever the player's scale.
        m.extra.model_scale = 0.0;
        // The render state: still, facing out, the head toward the cursor.
        m.previous = m.position;
        m.body_yaw = 180.0 + x_angle * 20.0;
        m.previous_body_yaw = m.body_yaw;
        m.head_yaw = m.body_yaw + x_angle * 20.0;
        m.previous_head_yaw = m.head_yaw;
        m.head_pitch = -y_angle * 20.0;
        m.previous_head_pitch = m.head_pitch;
        m.position.y += 0.125 * f64::from(u8::from(m.extra.crouching));
        m.previous = m.position;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        // The same render state as in the world: an invisible body is not drawn.
        let render = if self.own_body_visible() {
            horizon_client::mobs::render::render_with
        } else {
            horizon_client::mobs::render::render_layers
        };
        render(
            &m,
            1.0,
            camera,
            &lookup,
            240 | 240 << 8,
            self.res.mob_data(),
            &self.player_armor_layers(),
            &mut verts,
        );
        let chest = self.armor[1].as_ref().map(|s| &*s.item);
        if self.potions.effects.get("invisibility").is_none() {
            self.player_cape_geometry(&m, chest, camera, 1.0, 240 | 240 << 8, &mut verts);
        }
        self.player_elytra_geometry(&m, chest, camera, 1.0, 240 | 240 << 8, &mut verts);
        self.held_item_geometry(&m, camera, 1.0, 240 | 240 << 8, false, &mut verts);
        self.head_item_geometry(&m, camera, 1.0, 240 | 240 << 8, false, &mut verts);
        let height = if m.extra.crouching { 1.5 } else { 1.8 };
        // PictureInPictureRenderer: the box centre, (size, size, -size);
        // then the translation and Rz(pi) * Rx(y_angle * 20 degrees).
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, height / 2.0 + offset_y, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x((y_angle * 20.0).to_radians());
        let fire = self
            .player
            .as_ref()
            .filter(|p| p.fire_ticks > 0 && !p.spectator)
            .map(|p| (p.dimensions.width, p.dimensions.height));
        let quads = player_preview_quads(verts, gui, fire, &atlas.sprites);
        let start = batch.vertices.len();
        for (_, p, uv, color) in quads {
            batch.quad_3d(p, uv, color);
        }
        batch.depth_since(start, [x0, y0, x1, y1]);
    }
    /// The survival inventory's player box (`InventoryScreen`).
    pub(super) fn draw_inventory_player(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        if self.screen == Screen::Inventory {
            self.draw_player_preview(
                batch,
                ox + 26.0,
                oy + 8.0,
                ox + 75.0,
                oy + 78.0,
                30.0,
                0.0625,
            );
        }
    }
}

impl Game<'_> {
    /// `WingsLayer` for a player wearing `chest`.
    pub(super) fn player_elytra_geometry(
        &self,
        m: &Mob,
        chest: Option<&str>,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        self.wings_geometry(
            chest,
            m.cape.as_deref(),
            false,
            horizon_client::mobs::render::base_transform(m, partial, camera),
            m.extra.elytra.angles(partial),
            m.extra.crouching,
            light,
            self.wings_overlay(
                m.extra.armor[1]
                    .as_ref()
                    .filter(|s| chest == Some(&*s.item)),
            ),
            out,
        );
    }
    /// `WingsLayer` draws through `EquipmentLayerRenderer.renderLayers`: the
    /// armour glint when the worn elytra `hasFoil()`.
    pub(super) fn wings_overlay(&self, stack: Option<&horizon_client::items::ItemStack>) -> u32 {
        let item_glint = |item: &str| {
            let props = &self.res.items().enchantments.props;
            props.get(item).is_some_and(|p| p.glint)
        };
        match stack {
            Some(s) if s.has_foil(item_glint(&s.item)) => horizon_client::mobs::render::ARMOR_GLINT,
            _ => 0,
        }
    }
    /// `WingsLayer.submit` for any wearer: the wings of `chest`'s equipment
    /// asset on `base` (the wearer's model transform), `baby` for the baby
    /// elytra model. `cape` is a player's own cape texture, which has its
    /// elytra (`WingsLayer.getPlayerElytraTexture`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn wings_geometry(
        &self,
        chest: Option<&str>,
        cape: Option<&str>,
        baby: bool,
        base: glam::Mat4,
        angles: [f32; 3],
        crouching: bool,
        light: u32,
        overlay: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let Some(asset) = chest
            .and_then(|item| self.items().get(item))
            .and_then(|i| i.equip_asset.as_ref())
        else {
            return;
        };
        let assets = &self.armor_stand_assets;
        let Some((texture, _)) = assets.equipment.get(asset).and_then(|k| k.get("wings")) else {
            return;
        };
        let sprites = &self.gui.atlas.sprites;
        let rect = cape.and_then(|c| sprites.get(c));
        let (Some(model), Some(rect)) = (
            assets
                .player
                .get(if baby { "elytra_baby" } else { "elytra" }),
            rect.or_else(|| sprites.get(texture)),
        ) else {
            return;
        };
        let base = base * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.125));
        horizon_client::mobs::render_model(
            model,
            base,
            &|part| {
                let mut p = part.pose;
                if part.name == "left_wing" || part.name == "right_wing" {
                    let invert = if part.name == "left_wing" { 1.0 } else { -1.0 };
                    p[1] = if crouching { 3.0 } else { 0.0 };
                    p[3] = angles[0];
                    p[4] = invert * angles[1];
                    p[5] = invert * angles[2];
                }
                p
            },
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            overlay,
            out,
        );
    }
    /// `CapeLayer` for a player wearing `chest`: no cape under an elytra's
    /// wings, and moved out over a chestplate.
    pub(super) fn player_cape_geometry(
        &self,
        m: &Mob,
        chest: Option<&str>,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let Some(rect) = m
            .cape
            .as_deref()
            .and_then(|c| self.gui.atlas.sprites.get(c))
        else {
            return;
        };
        if horizon_client::mobs::trader::is_invisible(m) {
            return;
        }
        let layers = chest
            .and_then(|item| self.res.items().get(item))
            .and_then(|i| i.equip_asset.as_ref())
            .and_then(|asset| self.armor_stand_assets.equipment.get(asset));
        if layers.is_some_and(|k| k.contains_key("wings")) {
            return;
        }
        let mut base = horizon_client::mobs::render::base_transform(m, partial, camera);
        if layers.is_some_and(|k| k.contains_key("humanoid")) {
            base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.053125, 0.06875));
        }
        horizon_client::mobs::render::cape(
            m,
            partial,
            base,
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            self.res.mob_data(),
            out,
        );
    }
    /// `SpinAttackEffectLayer`: Riptide's two boxes turning around a spinning
    /// player (`SpinAttackEffectModel.setupAnim`).
    pub(super) fn spin_attack_geometry(
        &self,
        m: &Mob,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        if !m.extra.auto_spin_attack {
            return;
        }
        let (Some(model), Some(rect)) = (
            self.res.spin_attack().as_ref(),
            self.gui.atlas.sprites.get("entity/trident/trident_riptide"),
        ) else {
            return;
        };
        let age = m.age as f32 + partial;
        horizon_client::mobs::render_model(
            model,
            horizon_client::mobs::render::base_transform(m, partial, camera),
            &|part| {
                let mut p = part.pose;
                if let Some(i) = part
                    .name
                    .strip_prefix("box")
                    .and_then(|i| i.parse::<u8>().ok())
                {
                    let angle = age * -(45.0 + f32::from(i + 1) * 5.0);
                    p[4] = wrap(angle).to_radians();
                }
                p
            },
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            0,
            out,
        );
    }
}

fn player_preview_quads(
    mut verts: Vec<horizon_client::mesher::Vertex>,
    gui: glam::Mat4,
    fire: Option<(f32, f32)>,
    sprites: &std::collections::HashMap<String, horizon_client::hud::Rect>,
) -> Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> {
    let flame_start = verts.len();
    if let Some((width, height)) = fire {
        horizon_client::fire_render::flames(
            [0.0; 3],
            width,
            height,
            glam::DVec3::ZERO,
            0.0,
            sprites,
            0xff,
            &mut verts,
        );
    }
    let mut quads: Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> = Vec::new();
    for (index, q) in verts.as_chunks::<4>().0.iter().enumerate() {
        let p: [glam::Vec3; 4] =
            std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        // The winding gives inward normals: faces with n.z >= 0 point away.
        if n.z >= 0.0 {
            continue;
        }
        let n = n.normalize_or_zero();
        // Lighting.Entry.ENTITY_IN_UI, roughly: lit from the front and above.
        let shade = if index * 4 >= flame_start {
            1.0
        } else {
            (0.55 + 0.45 * -n.z - 0.15 * n.y).clamp(0.4, 1.0)
        };
        let c = q[0].color;
        let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
        let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
        let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
        quads.push((
            depth,
            p.map(|v| v.to_array()),
            std::array::from_fn(|i| q[i].uv),
            color,
        ));
    }
    // Keep far-to-near order for translucent faces; opaque overlap uses depth.
    quads.sort_by(|a, b| a.0.total_cmp(&b.0));
    quads
}

/// `Entity.turn` adds the turn to `xRotO` as well as to `xRot`, so the local
/// player's `getXRot(partialTick)` follows the mouse every frame instead of
/// easing in over a tick (#3844), while a change the tick made itself (the
/// pitch a sleeper lies at) still eases in. `pitch_o` is the model's old
/// pitch; without a model the old pitch is the pitch.
fn turn_rotation(yaw: f32, pitch: f32, pitch_o: Option<f32>, xo: f64, yo: f64) -> Rotation {
    let mut rotation = Rotation {
        x_rot: pitch,
        y_rot: yaw,
        x_rot_o: pitch_o.unwrap_or(pitch),
        y_rot_o: yaw,
    };
    rotation.turn(xo, yo);
    rotation
}

/// The player's model's old and new positions for a tick: the player's old
/// (`Entity.xo`, `yo`, `zo`) and new positions, each 0.125 lower while
/// crouching. The model is the player itself in vanilla, so
/// `EntityRenderer.extractRenderState` lerps it from the same old position
/// `Camera.setup` does, however far it moved: at Levitation 255 (12.75 blocks
/// a tick) the body keeps up with the camera. A teleport or respawn resets
/// the old position, and the model jumps with it.
fn player_model_positions(
    previous_position: Vec3,
    position: Vec3,
    was_crouching: bool,
    crouching: bool,
) -> (Vec3, Vec3) {
    let drop =
        |p: Vec3, crouching: bool| Vec3::new(p.x, p.y - if crouching { 0.125 } else { 0.0 }, p.z);
    (
        drop(previous_position, was_crouching),
        drop(position, crouching),
    )
}

/// The distance `LivingEntity.calculateEntityAnimation` walks the player's
/// model for a move from `from` to `to`. `omnidirectionalAirMover()` is false
/// for every player (`Entity`'s, not overridden), so only the horizontal
/// distance counts, flying and gliding included: rising or sinking straight
/// up or down leaves the legs and arms still.
fn player_walk_distance(from: Vec3, to: Vec3) -> f32 {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    (dx * dx + dz * dz).sqrt() as f32
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn the_player_model_rises_with_the_camera_at_levitation_255() {
        // LivingEntity.travel at Levitation 255: 0.05 * 255 blocks a tick.
        let from = Vec3::new(0.5, 80.0, 0.5);
        let to = Vec3::new(0.5, 92.75, 0.5);
        // The camera lerps from the old position: so does the body.
        assert_eq!(
            player_model_positions(from, to, false, false),
            (from, to),
            "the body jumps instead of rising (#3616)"
        );
        // Any distance, crouching on both ends.
        let far = Vec3::new(40.5, 80.0, 0.5);
        assert_eq!(
            player_model_positions(from, far, true, true),
            (Vec3::new(0.5, 79.875, 0.5), Vec3::new(40.5, 79.875, 0.5)),
        );
        // A teleport resets the old position to the new one: no sweep.
        assert_eq!(player_model_positions(to, to, false, false), (to, to));
    }
    #[test]
    fn flying_straight_up_does_not_walk_the_player_model() {
        let from = Vec3::new(0.5, 80.0, 0.5);
        // Creative flight straight up (#3302) and straight down.
        assert_eq!(player_walk_distance(from, Vec3::new(0.5, 80.375, 0.5)), 0.0);
        assert_eq!(player_walk_distance(from, Vec3::new(0.5, 78.5, 0.5)), 0.0);
        // Moving sideways while rising walks only the horizontal part.
        let d = player_walk_distance(from, Vec3::new(0.8, 81.0, 0.9));
        assert!((d - 0.5).abs() < 1e-6, "{d}");
    }
    /// #3844: `getXRot(partialTick)` after `Entity.turn`, as the model's
    /// renderer lerps it, with turns before and after the tick.
    #[test]
    fn own_head_pitch_follows_the_mouse_between_ticks() {
        // (pitch, the model's previous head pitch, its head pitch), as
        // `turn_player` and `tick_player_model` move them.
        let turn = |(pitch, previous, _): (f32, f32, f32), degrees: f32| {
            let r = turn_rotation(0.0, pitch, Some(previous), 0.0, f64::from(degrees / 0.15));
            (r.x_rot, r.x_rot_o, r.x_rot)
        };
        let tick = |(pitch, _, head): (f32, f32, f32)| (pitch, head, pitch);
        let x_rot =
            |(_, previous, head): (f32, f32, f32), alpha: f32| previous + (head - previous) * alpha;
        let at = |state, pitch: f32| {
            for alpha in [0.0, 0.5, 1.0] {
                let x = x_rot(state, alpha);
                assert!((x - pitch).abs() < 1e-3, "partial tick {alpha}: {x}");
            }
        };
        // The arrow keys turn before the frame's ticks: the tick then copies
        // ends that are already in step.
        let still = (10.0, 10.0, 10.0);
        let state = tick(turn(still, 30.0));
        at(state, 40.0);
        // The mouse turns after the tick: the head is there at once.
        let state = turn(state, -30.0);
        at(state, 10.0);
        // The tick laid the head down from 30 to 0: the turn moves both
        // ends, and the tick's change still eases in.
        let state = turn((0.0, 30.0, 0.0), 10.0);
        assert!((x_rot(state, 0.5) - 25.0).abs() < 1e-3, "{state:?}");
        // xRotO is clamped as xRot is.
        let r = turn_rotation(0.0, 60.0, Some(80.0), 0.0, 200.0);
        assert_eq!((r.x_rot_o, r.x_rot), (90.0, 90.0));
        // Without a model the old pitch is the pitch.
        let r = turn_rotation(0.0, 10.0, None, 0.0, 100.0);
        assert_eq!(r.x_rot_o, r.x_rot);
    }
    #[test]
    fn preview_keeps_crossing_faces_depth_and_clips_without_dropping_faces() {
        use horizon_client::{
            hud::{Blend, HudBatch},
            mesher::Vertex,
        };
        let face = |z: [f32; 4]| {
            [[-2.0, -1.0], [-2.0, 1.0], [2.0, 1.0], [2.0, -1.0]]
                .into_iter()
                .zip(z)
                .map(|([x, y], z)| Vertex {
                    pos: [x, y, z],
                    uv: [0.0; 2],
                    layer: 0,
                    color: u32::MAX,
                    light: 0,
                    normal: [0; 4],

                    glint_uv: [0.0, 0.0],
                })
        };
        // A shield and arm can intersect: neither average depth orders both ends.
        let verts = face([1.0, 1.0, -1.0, -1.0]).chain(face([0.0; 4])).collect();
        let quads = player_preview_quads(verts, glam::Mat4::IDENTITY, None, &Default::default());
        assert_eq!(quads.len(), 2);
        let mut batch = HudBatch::new([320, 240]);
        for (_, p, uv, color) in quads {
            batch.quad_3d(p, uv, color);
        }
        // Its centre is outside this scissor; the visible part must still be drawn.
        batch.depth_since(0, [1.0, 0.0, 2.0, 1.0]);
        assert_eq!(batch.vertices.len(), 12);
        let shield = &batch.vertices[..6];
        let arm = &batch.vertices[6..];
        assert!(shield[0].depth < arm[0].depth);
        assert!(shield[2].depth > arm[2].depth);
        assert_eq!(batch.runs(), vec![(0..12, Blend::Depth([1, 0, 1, 1]))]);
    }

    #[test]
    #[ignore = "requires prepared model data in HORIZON_TEST_RESOURCES"]
    fn player_render_transform_uses_vanilla_avatar_scale() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let mut m = Mob::new(1, player_type(&root).unwrap(), Vec3::ZERO, 0.0);
        for (id, expected) in [("minecraft:player", 0.9375), ("minecraft:zombie", 1.0)] {
            Arc::make_mut(&mut m.kind).id = id.into();
            let transform =
                horizon_client::mobs::render::base_transform_at(&m, 1.0, glam::Mat4::IDENTITY);
            for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
                assert!((transform.transform_vector3(axis).length() - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn an_invisible_player_hides_its_own_body() {
        use horizon_client::{effects::EffectInstance, scoreboard::Scoreboard};
        // The effect as a server's UpdateMobEffect or a drunk potion adds it.
        let id = horizon_client::brewing::effect("minecraft:invisibility")
            .unwrap()
            .id;
        let mut effects = horizon_client::effects::PlayerEffects::default();
        assert!(body_visible(&effects, None));
        effects.add(EffectInstance::new(id, 3600, 0));
        assert!(!body_visible(&effects, None), "no team: the body is hidden");
        // Entity.isInvisibleTo: a team that sees friendly invisibles sees it.
        let mut board = Scoreboard::default();
        let team = board.add_team("blue");
        assert!(body_visible(&effects, Some(team)));
        team.see_friendly_invisibles = false;
        assert!(!body_visible(&effects, Some(team)));
    }

    /// LivingEntityRenderer.submit: an invisible body is not drawn, its
    /// HumanoidArmorLayer is.
    #[test]
    fn an_invisible_body_keeps_its_armour() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        let Some(kind) = player_type(&root) else {
            return;
        };
        let m = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
        let armour = [(&*kind.model, [0.5, 0.5, 0.75, 0.75], u32::MAX, 0)];
        let atlas = |_: &str| Some([0.0, 0.0, 0.25, 0.25]);
        let data = horizon_client::mobs::MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            pois: Default::default(),
        };
        let draw = |body: bool| {
            let mut out = Vec::new();
            let render = if body {
                horizon_client::mobs::render::render_with
            } else {
                horizon_client::mobs::render::render_layers
            };
            render(
                &m,
                1.0,
                glam::DVec3::ZERO,
                &atlas,
                0xF0F0,
                &data,
                &armour,
                &mut out,
            );
            out
        };
        let (shown, hidden) = (draw(true), draw(false));
        assert!(shown.iter().any(|v| v.uv[0] <= 0.25), "the body draws");
        assert!(!hidden.is_empty(), "the armour still draws");
        assert!(hidden.iter().all(|v| v.uv[0] >= 0.5), "only the armour");
        assert_eq!(hidden.len() * 2, shown.len());
    }

    #[test]
    fn burning_inventory_preview_keeps_visible_fullbright_flames() {
        let rect = |u0| horizon_client::hud::Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.1,
            v1: 0.1,
            w: 16,
            h: 16,
        };
        let sprites = std::collections::HashMap::from([
            ("block/fire_0".into(), rect(0.0)),
            ("block/fire_1".into(), rect(0.5)),
        ]);
        for pitch in [-30.0f32, 0.0, 30.0] {
            let gui = glam::Mat4::from_scale(glam::Vec3::new(30.0, 30.0, -30.0))
                * glam::Mat4::from_rotation_z(std::f32::consts::PI)
                * glam::Mat4::from_rotation_x(pitch.to_radians());
            assert!(player_preview_quads(Vec::new(), gui, None, &sprites).is_empty());
            let quads = player_preview_quads(Vec::new(), gui, Some((0.6, 1.8)), &sprites);
            assert!(
                !quads.is_empty(),
                "inventory projection must not cull all flames"
            );
            assert!(quads.iter().all(|q| q.3 == 0xffff_ffff));
            assert!(quads.iter().any(|q| q.2[0][0] <= 0.1));
            assert!(quads.iter().any(|q| q.2[0][0] >= 0.5));
            assert!(quads.windows(2).all(|q| q[0].0 <= q[1].0));
        }
    }
}
