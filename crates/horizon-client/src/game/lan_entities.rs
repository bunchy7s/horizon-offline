//! The entities beyond mobs and items that a server tracks to its clients
//! (`ChunkMap.TrackedEntity`, `ServerEntity`): boats, minecarts, armor
//! stands, item frames, paintings and the entities Horizon keeps without
//! simulating them, each as vanilla pairs it (`getAddEntityPacket` and the
//! synched data that differs from the defaults), and who rides what.
use super::Game;
use horizon_client::lan::{
    entity::{EntityState, Riding, States},
    hub::Hub,
    playerdata::ints_uuid,
};
use horizon_protocol::entity_data::{Value, serializer};
use horizon_world::nbt::Tag;

fn state(id: u32, kind: &str, position: [f64; 3], yaw: f32, on_ground: bool) -> EntityState {
    EntityState {
        id: id as i32,
        uuid: super::lan::entity_uuid(id),
        kind: kind.to_owned(),
        position,
        yaw,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground,
        velocity: [0.0; 3],
        data: 0,
        meta: Vec::new(),
        defaults: Vec::new(),
        equipment: Vec::new(),
        attributes: Vec::new(),
        precise: false,
        needs_sync: false,
        leash: None,
    }
}

/// `VehicleEntity`'s DATA_ID_HURT, DATA_ID_HURTDIR and DATA_ID_DAMAGE
/// (entries 8 to 10) when they differ from their defaults (0, 1, 0).
fn vehicle_meta(hurt_time: i32, hurt_dir: i32, damage: f32) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    if hurt_time != 0 {
        meta.push((8, serializer::INT, Value::Int(hurt_time)));
    }
    if hurt_dir != 1 {
        meta.push((9, serializer::INT, Value::Int(hurt_dir)));
    }
    if damage != 0.0 {
        meta.push((10, serializer::FLOAT, Value::Float(damage)));
    }
    meta
}

/// An armor stand's pose (`ArmorStandPose`), each part that differs from the
/// default, as DATA_HEAD_POSE to DATA_RIGHT_LEG_POSE (entries 16 to 21).
fn stand_poses(a: &horizon_client::entities::ArmorStand) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    for (i, (rotation, default)) in a
        .pose
        .iter()
        .zip(horizon_client::entities::ArmorStand::DEFAULT_POSE)
        .enumerate()
    {
        if *rotation != default {
            meta.push((
                16 + i as u8,
                serializer::ROTATIONS,
                Value::Floats(rotation.to_vec()),
            ));
        }
    }
    meta
}

impl Game<'_> {
    /// Who rides what (`Entity.getPassengers`): mobs on mobs, and the
    /// host in its boat or minecart.
    pub(super) fn lan_riding(
        &self,
        host_entity: Option<i32>,
        guests: &[(u32, i32)],
        riding: &mut Riding,
    ) {
        riding.clear();
        // The server's players in their boats and minecarts.
        for &(vehicle, guest) in guests {
            riding.add(vehicle as i32, guest);
        }
        for m in self.mobs.iter().filter(|m| !m.removed) {
            if let Some(vehicle) = m.extra.mount {
                riding.add(vehicle as i32, m.id as i32);
            }
        }
        if let (Some(host), Some(vehicle)) = (host_entity, self.riding) {
            riding.add(vehicle as i32, host);
        }
    }

    /// Boats, minecarts, armor stands, item frames, paintings and the
    /// entities kept unsimulated, as guests track them.
    pub(super) fn lan_object_states(
        &self,
        host_entity: Option<i32>,
        hub: &mut Hub,
        out: &mut States,
    ) {
        for b in &self.boats {
            let p = b.position;
            let mut e = state(b.id, &b.entity_type(), [p.x, p.y, p.z], b.yaw, b.on_ground);
            e.velocity = [b.velocity.x, b.velocity.y, b.velocity.z];
            e.meta = vehicle_meta(b.hurt_time, b.hurt_dir, b.damage);
            // AbstractBoat.DATA_ID_PADDLE_LEFT and DATA_ID_PADDLE_RIGHT.
            for (k, paddling) in b.paddling.iter().enumerate() {
                if *paddling {
                    e.meta
                        .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            out.push(e);
        }
        for c in &self.carts {
            let p = c.position;
            let mut e = state(c.id, c.kind.id(), [p.x, p.y, p.z], c.yaw, c.on_ground());
            e.velocity = [c.velocity.x, c.velocity.y, c.velocity.z];
            e.meta = vehicle_meta(c.hurt_time, c.hurt_dir, c.damage);
            match c.kind {
                // MinecartFurnace.DATA_ID_FUEL.
                super::minecarts::CartKind::Furnace if c.fuel > 0 => {
                    e.meta.push((13, serializer::BOOLEAN, Value::Bool(true)));
                }
                // MinecartCommandBlock.DATA_ID_COMMAND_NAME.
                super::minecarts::CartKind::CommandBlock => {
                    if let Some(Tag::String(command)) =
                        self.persist.raw.get(&c.id).and_then(|t| t.get("Command"))
                    {
                        let command = command.to_string_lossy();
                        if !command.is_empty() {
                            e.meta
                                .push((13, serializer::STRING, Value::String(command)));
                        }
                    }
                }
                _ => {}
            }
            out.push(e);
        }
        for a in &self.armor_stands {
            let p = a.position;
            let mut e = state(
                a.id,
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                a.yaw,
                a.on_ground,
            );
            e.head_yaw = a.yaw;
            e.velocity = [a.velocity.x, a.velocity.y, a.velocity.z];
            let mut flags = 0i8;
            if a.fire_ticks > 0 {
                flags |= 1;
            }
            if a.invisible {
                flags |= 0x20;
            }
            if flags != 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(flags)));
            }
            // LivingEntity.DATA_HEALTH_ID: 1 until the stand's max health is set.
            e.meta.push((9, serializer::FLOAT, Value::Float(a.health)));
            // ArmorStand.DATA_CLIENT_FLAGS: small 1, arms 4, no base plate 8, marker 16.
            let flags = i8::from(a.small)
                | i8::from(a.show_arms) << 2
                | i8::from(a.no_base_plate) << 3
                | i8::from(a.marker) << 4;
            if flags != 0 {
                e.meta.push((15, serializer::BYTE, Value::Byte(flags)));
            }
            e.meta.extend(stand_poses(a));
            // Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE: a hologram's text.
            if let Some(name) = &a.custom_name {
                e.meta.push((
                    2,
                    serializer::OPTIONAL_COMPONENT,
                    Value::OptText(Some(horizon_client::scoreboard::json_to_tag(
                        &name.component(),
                    ))),
                ));
                if a.name_visible {
                    e.meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            // A name taken away or hidden again (`/data`) is sent as none / false.
            e.defaults
                .push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
            e.defaults
                .push((3, serializer::BOOLEAN, Value::Bool(false)));
            // A pose put back to its default (`/data`) is sent as that default.
            e.defaults.push((15, serializer::BYTE, Value::Byte(0)));
            for (i, default) in horizon_client::entities::ArmorStand::DEFAULT_POSE
                .iter()
                .enumerate()
            {
                e.defaults.push((
                    16 + i as u8,
                    serializer::ROTATIONS,
                    Value::Floats(default.to_vec()),
                ));
            }
            // Stored main hand, off hand, feet, legs, chest, head: the wire's slots 0 to 5.
            for (slot, item) in a.equipment.iter().enumerate() {
                if item.is_some() {
                    e.equipment.push((slot as u8, hub.stack(item)));
                }
            }
            e.attributes = self.stand_attributes(a.id, &a.equipment);
            out.push(e);
        }
        for f in &self.item_frames {
            // ItemFrame.getAddEntityPacket: the block it hangs in and its direction.
            let mut e = state(f.id, f.kind(), f.pos.map(f64::from), 0.0, false);
            e.pitch = match f.facing {
                0 => 90.0,
                1 => -90.0,
                _ => 0.0,
            };
            e.yaw = match f.facing {
                2 => 180.0,
                4 => 90.0,
                5 => 270.0,
                _ => 0.0,
            };
            e.data = i32::from(f.facing);
            if f.invisible {
                e.meta.push((0, serializer::BYTE, Value::Byte(0x20)));
            }
            // HangingEntity.DATA_DIRECTION, south by default.
            if f.facing != 3 {
                e.meta
                    .push((8, serializer::DIRECTION, Value::Id(i32::from(f.facing))));
            }
            if f.item.is_some() {
                e.meta
                    .push((9, serializer::ITEM_STACK, Value::Item(hub.stack(&f.item))));
            }
            if f.rotation != 0 {
                e.meta
                    .push((10, serializer::INT, Value::Int(i32::from(f.rotation))));
            }
            out.push(e);
        }
        for (p, &id) in &self.knot_ids {
            // LeashFenceKnotEntity.getAddEntityPacket: the fence's block.
            out.push(state(
                id,
                "minecraft:leash_knot",
                p.map(f64::from),
                0.0,
                false,
            ));
        }
        for p in &self.paintings {
            let mut e = state(p.id, "minecraft:painting", p.pos.map(f64::from), 0.0, false);
            // Direction.get2DDataValue (south 0, west 1, north 2, east 3) to its
            // 3D data value and yaw.
            let facing = [3, 4, 2, 5][usize::from(p.facing & 3)];
            e.yaw = f32::from(p.facing & 3) * 90.0;
            e.data = facing;
            if facing != 3 {
                e.meta.push((8, serializer::DIRECTION, Value::Id(facing)));
            }
            // Painting.DATA_PAINTING_VARIANT_ID, a registry holder (id + 1),
            // set unless it is the registry's first variant (VariantUtils.getAny).
            if let Some(id) = hub.registry_id("minecraft:painting_variant", &p.variant)
                && id != 0
            {
                e.meta
                    .push((9, serializer::PAINTING_VARIANT, Value::Id(id + 1)));
            }
            out.push(e);
        }
        for c in &self.cushions {
            let p = c.position;
            let mut e = state(c.id, "minecraft:cushion", [p.x, p.y, p.z], c.yaw, false);
            // Cushion.DATA_COLOR, set unless it is the default white.
            if c.color != 0 {
                e.meta
                    .push((8, serializer::DYE_COLOR, Value::Id(i32::from(c.color))));
            }
            out.push(e);
        }
        self.lan_projectile_states(host_entity, hub, out);
        for o in &self.others {
            let p = o.position;
            let mut e = state(o.id, &o.kind, [p.x, p.y, p.z], o.yaw, false);
            e.pitch = o.pitch;
            e.head_yaw = o.yaw;
            out.push(e);
        }
    }
}

impl Game<'_> {
    /// `ClientboundAddEntityPacket(Entity, ServerEntity, int)`: the entity's
    /// own UUID (`Entity.getUUID`), the one `/data`, saves and UUID
    /// selectors use, so a client can name it back (the crosshair UUID that
    /// `ClientSuggestionProvider.getSelectedEntities` suggests).
    pub(super) fn lan_entity_uuids(&mut self, states: &mut States) {
        for e in states.as_mut_slice() {
            e.uuid = ints_uuid(self.uuid_for(e.id as u32));
        }
    }

    /// Gives the entities Horizon keeps without an id one (`Entity.id`, from
    /// the same counter as the rest) before they are first tracked.
    pub(super) fn lan_assign_ids(&mut self) {
        // LeashFenceKnotEntity: one per fence post with a lead on it
        // (`notifyLeasheeRemoved` discards it once the last lead is gone).
        let knots: Vec<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .filter_map(|m| match m.extra.leash {
                Some(horizon_client::mobs::leash::Holder::Knot(p)) => Some(p),
                _ => None,
            })
            .collect();
        self.knot_ids.retain(|p, _| knots.contains(p));
        for p in knots {
            self.knot_ids.entry(p).or_insert_with(|| {
                self.next_entity += 1;
                self.next_entity
            });
        }
        let next = &mut self.next_entity;
        let mut give = |id: &mut u32| {
            if *id == 0 {
                *next += 1;
                *id = *next;
            }
        };
        self.tnts.iter_mut().for_each(|t| give(&mut t.id));
        self.falling.iter_mut().for_each(|f| give(&mut f.id));
        self.creatures
            .projectiles
            .iter_mut()
            .for_each(|p| give(&mut p.id));
        self.tridents.iter_mut().for_each(|t| give(&mut t.id));
        self.rockets.iter_mut().for_each(|r| give(&mut r.id));
        self.crystals.iter_mut().for_each(|c| give(&mut c.id));
        self.bolts.iter_mut().for_each(|b| give(&mut b.id));
        self.potions.thrown_ids().for_each(&mut give);
        self.end.eye_ids().for_each(&mut give);
        self.breath_clouds.iter_mut().for_each(|b| give(&mut b.id));
        self.dragon_fireballs
            .iter_mut()
            .for_each(|b| give(&mut b.id));
    }

    /// Primed TNT, falling blocks, projectiles, tridents, firework rockets and
    /// end crystals, as guests track them.
    fn lan_projectile_states(&self, host_entity: Option<i32>, hub: &mut Hub, out: &mut States) {
        let owner = |who: Option<horizon_client::mobs::Who>| match who {
            Some(horizon_client::mobs::Who::Mob(id)) => id as i32,
            Some(horizon_client::mobs::Who::Player) => host_entity.unwrap_or(0),
            None => 0,
        };
        let at = |p: horizon_core::geometry::Vec3| [p.x, p.y, p.z];
        for f in &self.fangs.list {
            out.push(state(f.id, "minecraft:evoker_fangs", f.pos, f.yaw, false));
        }
        // LightningBolt: each client draws and flashes its own.
        for b in self.bolts.iter().filter(|b| b.id != 0) {
            out.push(state(b.id, "minecraft:lightning_bolt", b.at, 0.0, false));
        }
        for t in self.tnts.iter().filter(|t| t.id != 0) {
            let mut e = state(t.id, "minecraft:tnt", at(t.position), 0.0, t.on_ground);
            e.velocity = at(t.velocity);
            // PrimedTnt.DATA_FUSE_ID, 80 by default; requiresPrecisePosition
            // while it touches a block.
            if t.fuse != 80 {
                e.meta.push((8, serializer::INT, Value::Int(t.fuse)));
            }
            e.precise = t.on_ground;
            out.push(e);
        }
        for f in self.falling.iter().filter(|f| f.id != 0) {
            let mut e = state(f.id, "minecraft:falling_block", at(f.position), 0.0, false);
            e.velocity = at(f.velocity);
            // getAddEntityPacket: the block state; DATA_START_POS.
            e.data = f.state.0 as i32;
            if f.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            let start = [f.start_pos.x, f.start_pos.y, f.start_pos.z];
            if start != [0; 3] {
                e.meta.push((8, serializer::BLOCK_POS, Value::Pos(start)));
            }
            out.push(e);
        }
        for p in self.creatures.projectiles.iter().filter(|p| p.id != 0) {
            use horizon_client::projectiles::ProjectileKind as K;
            let kind = p.kind.id();
            let mut e = state(p.id, kind, at(p.position), p.yaw, false);
            e.pitch = p.pitch;
            e.velocity = at(p.velocity);
            e.needs_sync = p.needs_sync;
            // Projectile.getAddEntityPacket: the owner's id.
            e.data = match p.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(p.owner),
            };
            match &p.kind {
                K::Arrow { critical, .. } => {
                    let a = p.arrow.as_deref();
                    e.meta.extend(arrow_meta(
                        *critical,
                        a.is_some_and(|a| a.no_physics),
                        a.map_or(0, |a| a.pierce),
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    ));
                }
                // ThrownTrident.ID_LOYALTY (none for a mob's) and ID_FOIL, on
                // AbstractArrow's flags.
                K::Trident => {
                    let a = p.arrow.as_deref();
                    e.meta.extend(arrow_meta(
                        false,
                        a.is_some_and(|a| a.no_physics),
                        0,
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    ));
                    if a.is_some_and(|a| a.pickup_item.has_any_enchantments()) {
                        e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
                    }
                }
                // WitherSkull.DATA_DANGEROUS.
                K::WitherSkull { dangerous: true } => {
                    e.meta.push((8, serializer::BOOLEAN, Value::Bool(true)));
                }
                _ => {}
            }
            out.push(e);
        }
        // Thrown potions and bottles: ThrowableItemProjectile.DATA_ITEM_STACK
        // (the potion's colour), which a plain bottle's type already has.
        for (id, kind, stack, position, velocity) in self.potions.thrown_tracked() {
            let mut e = state(id, kind, at(position), 0.0, false);
            e.velocity = at(velocity);
            if stack.extra.is_some() || stack.count != 1 {
                e.meta.push((
                    8,
                    serializer::ITEM_STACK,
                    Value::Item(hub.stack(&Some(stack.clone()))),
                ));
            }
            out.push(e);
        }
        // EyeOfEnder: its item is the plain eye (DATA_ITEM_STACK's default).
        for (id, position, velocity) in self.end.eyes_tracked() {
            let mut e = state(id, "minecraft:eye_of_ender", position, 0.0, false);
            e.velocity = velocity;
            out.push(e);
        }
        for t in self.tridents.iter().filter(|t| t.id != 0) {
            let mut e = state(t.id, "minecraft:trident", at(t.position), t.yaw, false);
            e.pitch = t.pitch;
            e.velocity = at(t.velocity);
            // Projectile.getAddEntityPacket: the owner's id, the guest's that
            // threw it.
            e.data = match t.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(Some(horizon_client::mobs::Who::Player)),
            };
            e.meta = arrow_meta(false, t.no_physics, 0, t.in_ground);
            // ThrownTrident.ID_LOYALTY and ID_FOIL.
            if t.loyalty != 0 {
                e.meta
                    .push((11, serializer::BYTE, Value::Byte(t.loyalty as i8)));
            }
            if t.stack.has_any_enchantments() {
                e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
            }
            out.push(e);
        }
        for r in self.rockets.iter().filter(|r| r.id != 0) {
            let mut e = state(
                r.id,
                "minecraft:firework_rocket",
                at(r.position),
                0.0,
                false,
            );
            e.velocity = at(r.velocity);
            // DATA_ID_FIREWORKS_ITEM once set from a stack, DATA_ATTACHED_TO_TARGET,
            // DATA_SHOT_AT_ANGLE.
            let plain = &*r.item.item == "minecraft:firework_rocket"
                && r.item.count == 1
                && r.item.extra.is_none();
            if !plain {
                e.meta.push((
                    8,
                    serializer::ITEM_STACK,
                    Value::Item(hub.stack(&Some(r.item.clone()))),
                ));
            }
            let boosted = match r.guest {
                Some(conn) => hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .map(|g| g.entity_id),
                None => host_entity,
            };
            if r.attached
                && let Some(player) = boosted
            {
                e.meta
                    .push((9, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(player + 1)));
            }
            if r.angled {
                e.meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
            }
            out.push(e);
        }
        for c in self.crystals.iter().filter(|c| c.id != 0) {
            let mut e = state(c.id, "minecraft:end_crystal", at(c.position), 0.0, false);
            // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM (true by default).
            if c.beam.is_some() {
                e.meta
                    .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(c.beam)));
            }
            if !c.show_bottom {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(false)));
            }
            out.push(e);
        }
        // The dragon's breath and lingering potion clouds (AreaEffectCloud's DATA_RADIUS,
        // DATA_WAITING for its 20-tick wait, and DATA_PARTICLE): the
        // client draws them from these (`tick_remote_clouds`).
        // The hub is out of `self.lan` meanwhile, so it is passed in.
        let breath = Self::hub_particle(
            hub,
            &horizon_client::particles::ParticleOptions::simple(
                horizon_client::particles::ParticleType::DragonBreath,
            ),
            None,
        );
        for b in self.dragon_fireballs.iter().filter(|b| b.id != 0) {
            let p = b.position;
            let mut e = state(b.id, "minecraft:dragon_fireball", at(p), 0.0, false);
            e.velocity = [b.velocity.x, b.velocity.y, b.velocity.z];
            out.push(e);
        }
        for b in self.breath_clouds.iter().filter(|b| b.id != 0) {
            let mut e = state(b.id, "minecraft:area_effect_cloud", b.at, 0.0, false);
            e.meta.push((8, serializer::FLOAT, Value::Float(b.radius)));
            if b.waiting() {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(true)));
            }
            let particle = match &b.potion {
                Some(p) => self.network_particle(&p.particle(), None),
                None => breath.clone(),
            };
            if let Some(p) = &particle {
                e.meta
                    .push((10, serializer::PARTICLE, Value::Particle(p.clone())));
            }
            e.defaults
                .push((9, serializer::BOOLEAN, Value::Bool(false)));
            out.push(e);
        }
        out.extend(self.lan_dragon_state());
    }
}

/// `AbstractArrow`'s ID_FLAGS (critical 1, no physics 2), PIERCE_LEVEL and
/// IN_GROUND, when they differ from the defaults.
fn arrow_meta(
    critical: bool,
    no_physics: bool,
    pierce: u8,
    in_ground: bool,
) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    let flags = i8::from(critical) | i8::from(no_physics) << 1;
    if flags != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(flags)));
    }
    if pierce != 0 {
        meta.push((9, serializer::BYTE, Value::Byte(pierce as i8)));
    }
    if in_ground {
        meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armor_stand_poses_send_only_what_differs() {
        let mut stand = horizon_client::entities::ArmorStand::new(1, Default::default(), 0.0);
        stand.pose[0] = [10.0, 20.0, 0.0];
        let meta = stand_poses(&stand);
        assert_eq!(
            meta,
            vec![(
                16,
                serializer::ROTATIONS,
                Value::Floats(vec![10.0, 20.0, 0.0])
            )]
        );
        let plain = horizon_client::entities::ArmorStand::new(1, Default::default(), 0.0);
        assert!(stand_poses(&plain).is_empty());
    }
}
