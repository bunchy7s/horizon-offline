//! Saving and loading the game state that lives outside chunk blocks:
//! entities (`entities/*.mca`, per chunk like vanilla's `EntityStorage`) and
//! scheduled block/fluid ticks (chunk `block_ticks`/`fluid_ticks`).
use super::{FallingBlock, Game};
use horizon_client::{
    Result,
    chunk::Chunk,
    entity_nbt,
    mobs::Who,
    save::{ChunkExtras, random_uuid},
};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::RandomSource,
    ticks::ChunkTicks,
};
use horizon_world::attachments::{ATTACHMENTS_KEY, attachment_compound};
use horizon_world::{level::MovingPiston, nbt::Tag, registry::BlockStateId};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// What Horizon remembers about loaded entities to save them faithfully.
#[derive(Default)]
pub struct EntityMemory {
    /// UUIDs of mobs and item entities by Horizon entity ID.
    pub(super) uuids: HashMap<u32, [i32; 4]>,
    /// The compound each mob or item entity was loaded with.
    pub(super) raw: HashMap<u32, Tag>,
    /// Entities of types Horizon does not simulate, kept per chunk and
    /// written back unchanged.
    unknown: HashMap<(i32, i32), Vec<Tag>>,
    /// Entities without a Horizon entity ID (orbs, arrows, falling blocks, crystals,
    /// the dragon): the UUID and compound they were loaded with, found again by
    /// type and position (an entity that has moved since loading is saved afresh).
    anon: HashMap<(String, [u64; 3]), (Vec3, Option<[i32; 4]>, Tag)>,
    /// UUIDs of the vehicles `restore_saved_vehicle` brought in from the
    /// player's file: their chunk copies must not spawn a second one. Cleared
    /// with the rest when a world closes.
    restored: HashSet<[i32; 4]>,
    /// Entities a native mod kept out of the level (`native_entity_join_level`),
    /// counted so `/summon` can fail as vanilla's refused add does.
    pub(super) joins_canceled: u32,
}

/// The key that marks a compound queued with the chunk loads as a fresh add
/// (`PersistentEntitySectionManager.addNewEntity`: `/summon`, a portal
/// arrival), not an entity loaded from disk. Taken off before the compound
/// is kept or saved.
const FRESH_ADD: &str = "horizon:fresh_add";

/// Queue `tag` with the chunk loads as a fresh add.
pub(super) fn mark_fresh(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.insert(FRESH_ADD.into(), Tag::Byte(1));
    }
}

/// The key that marks a compound queued with the chunk loads as an entity
/// moving within its level (an End gateway: `Entity.teleport` takes
/// `teleportSameDimension`, which moves the entity without removing it), so
/// it doesn't join the level again. Taken off like [`FRESH_ADD`].
const SAME_LEVEL_MOVE: &str = "horizon:same_level_move";

/// Queue `tag` with the chunk loads as an entity moving within its level.
pub(super) fn mark_moved(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.insert(SAME_LEVEL_MOVE.into(), Tag::Byte(1));
    }
}

/// Whether `tag` was queued as a move within its level; the mark comes off.
fn take_moved(tag: &mut Tag) -> bool {
    match tag {
        Tag::Compound(c) => c.remove(&SAME_LEVEL_MOVE.into()).is_some(),
        _ => false,
    }
}

/// Whether `tag` was queued as a fresh add; the mark comes off.
fn take_fresh(tag: &mut Tag) -> bool {
    match tag {
        Tag::Compound(c) => c.remove(&FRESH_ADD.into()).is_some(),
        _ => false,
    }
}

fn anon_key(kind: &str, p: Vec3) -> (String, [u64; 3]) {
    (
        kind.to_owned(),
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()],
    )
}
/// The loaded compound underneath the fields Horizon models: what the entity was
/// loaded with, with every key of `modelled` replacing its loaded value.
pub(super) fn with_raw(modelled: Tag, raw: Option<&Tag>) -> Tag {
    let (Some(Tag::Compound(raw)), Tag::Compound(modelled)) = (raw, &modelled) else {
        return modelled;
    };
    let mut out = raw.clone();
    for (k, v) in modelled {
        // Defaults Horizon invents for fields it does not simulate give way to the save.
        let invented = matches!(
            k.to_string_lossy().as_str(),
            "Fire" | "Air" | "Invulnerable" | "PortalCooldown" | "HasBeenShot"
        );
        if !(invented && out.contains_key(k)) {
            out.insert(k.clone(), v.clone());
        }
    }
    Tag::Compound(out)
}

fn read_attachments(
    tag: &Tag,
    uuid: [i32; 4],
    maps: &mut HashMap<[i32; 4], horizon_world::attachments::Attachments>,
) {
    if let Ok(Some(data)) = attachment_compound(tag.get(ATTACHMENTS_KEY)) {
        maps.entry(uuid)
            .or_insert_with(|| horizon_world::attachments::Attachments::from_nbt(data.clone()));
    }
}

/// Overlay live attachment maps on entity tags, including passenger trees.
pub(super) fn write_attachments(
    tag: &mut Tag,
    maps: &HashMap<[i32; 4], horizon_world::attachments::Attachments>,
) {
    let uuid = entity_nbt::uuid(tag);
    if let Tag::Compound(root) = tag {
        if let Some(map) = uuid.and_then(|id| maps.get(&id))
            && attachment_compound(root.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            let data = map.to_nbt();
            if data.is_empty() {
                root.remove(&ATTACHMENTS_KEY.into());
            } else {
                root.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
            }
        }
        if let Some(Tag::List(passengers)) = root.get_mut(&"Passengers".into()) {
            for passenger in passengers {
                write_attachments(passenger, maps);
            }
        }
    }
}

fn painting_center(p: &horizon_client::entities::Painting) -> Vec3 {
    let (lo, hi) = p.bounds();
    Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    )
}
fn block_entity_pos(tag: &Tag) -> [i32; 3] {
    [tag.get("x"), tag.get("y"), tag.get("z")]
        .map(|v| horizon_client::save::number(v).unwrap_or(0) as i32)
}
fn chunk_of(p: Vec3) -> (i32, i32) {
    ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
}

impl Game<'_> {
    /// Saved ticks and the save time for a chunk about to be written.
    pub(super) fn attach_save_data(&self, chunk: &mut Chunk) {
        let pos = ChunkPos::new(chunk.x, chunk.z);
        let time = self.world.game_time;
        let mut extras: ChunkExtras = chunk.saved.as_deref().cloned().unwrap_or_default();
        extras.block_entity_attachments = self
            .world
            .block_entities
            .chunk_attachments(chunk.x, chunk.z);
        // A chunk the scheduler hasn't registered yet (one loaded and
        // unloaded again before the next tick) still holds its saved ticks:
        // `register_chunk_ticks` empties that list once it takes them.
        if let Some(t) = self.block_ticks.blocks.chunk(pos) {
            extras.block_ticks = t.pack(time);
        }
        if let Some(t) = self.block_ticks.fluids.chunk(pos) {
            extras.fluid_ticks = t.pack(time);
        }
        extras.last_update = time;
        // PistonMovingBlockEntity.saveAdditional: a block caught mid-move is
        // saved with its chunk (#3310).
        for (p, piston) in &self.block_ticks.moving_pistons {
            if (p.x >> 4, p.z >> 4) != (chunk.x, chunk.z) {
                continue;
            }
            extras
                .block_entities
                .retain(|tag| block_entity_pos(tag) != [p.x, p.y, p.z]);
            extras
                .block_entities
                .push(self.moving_piston_to_nbt(*p, piston));
        }
        // TheEndGatewayBlockEntity saves its Age (the server copy's).
        for tag in &mut extras.block_entities {
            let pos = [tag.get("x"), tag.get("y"), tag.get("z")]
                .map(|v| horizon_client::save::number(v).unwrap_or(0) as i32);
            if let Some(g) = self.be_anims.gateway_servers.get(&pos)
                && let Some(age) = tag.get_mut("Age")
            {
                *age = horizon_world::nbt::Tag::Long(g.age);
            }
        }
        if let Some(t) = self.world.inhabited.get(&(chunk.x, chunk.z)) {
            extras
                .root
                .insert("InhabitedTime".into(), horizon_world::nbt::Tag::Long(*t));
        }
        chunk.saved = Some(Arc::new(extras));
    }
    /// Register a newly resident chunk with the tick scheduler, with the
    /// block and fluid ticks it was saved with (`LevelChunk.registerTickContainerInLevel`
    /// at FULL). They are unpacked when the chunk starts ticking
    /// (`start_ticking_chunks`).
    pub(super) fn register_chunk_ticks(&mut self, key: (i32, i32)) {
        let pos = ChunkPos::new(key.0, key.1);
        let saved = self.world.chunks.get(&key).and_then(|c| c.saved.clone());
        // ChunkAccess.inhabitedTime as saved.
        if let Some(Tag::Long(t)) = saved
            .as_deref()
            .and_then(|s| s.root.get(&"InhabitedTime".into()))
        {
            self.world.inhabited.insert(key, *t);
        }
        let (blocks, fluids) = saved.as_deref().map_or((Vec::new(), Vec::new()), |s| {
            (s.block_ticks.clone(), s.fluid_ticks.clone())
        });
        let loaded = !blocks.is_empty() || !fluids.is_empty();
        if self
            .block_ticks
            .add_chunk(
                pos,
                ChunkTicks::from_saved(blocks),
                ChunkTicks::from_saved(fluids),
            )
            .is_ok()
            && loaded
        {
            self.tick_chunks.wait(key);
        }
        // The saved list is consumed: later saves take the scheduler's ticks.
        if loaded && let Some(c) = self.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                let s = Arc::make_mut(s);
                s.block_ticks.clear();
                s.fluid_ticks.clear();
            }
        }
        // PistonMovingBlockEntity.loadAdditional: blocks saved mid-move carry
        // on moving; later saves take them from the level (#3310).
        let pistons: Vec<_> = saved.as_deref().map_or_else(Vec::new, |s| {
            s.block_entities
                .iter()
                .filter_map(|tag| self.moving_piston_from_nbt(tag))
                .collect()
        });
        if pistons.is_empty() {
            return;
        }
        for &(pos, piston) in &pistons {
            self.block_ticks.load_moving_piston(pos, piston);
        }
        if let Some(c) = self.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                Arc::make_mut(s).block_entities.retain(|tag| {
                    let at = block_entity_pos(tag);
                    !pistons.iter().any(|(p, _)| [p.x, p.y, p.z] == at)
                });
            }
        }
    }
    /// `PistonMovingBlockEntity.saveAdditional`.
    fn moving_piston_to_nbt(&self, pos: BlockPos, piston: &MovingPiston) -> Tag {
        let flag = |b: bool| Tag::Byte(i8::from(b));
        Tag::compound([
            ("id", Tag::String("minecraft:piston".into())),
            ("x", Tag::Int(pos.x)),
            ("y", Tag::Int(pos.y)),
            ("z", Tag::Int(pos.z)),
            ("blockState", self.state_to_nbt(piston.moved_state)),
            // Direction.LEGACY_ID_CODEC: the 3D data value as a byte.
            ("facing", Tag::Byte(piston.direction as i8)),
            ("progress", Tag::Float(piston.progress_o.to_bits())),
            ("extending", flag(piston.extending)),
            ("source", flag(piston.source)),
        ])
    }
    /// `PistonMovingBlockEntity.loadAdditional`, for a `minecraft:piston`
    /// entity whose block is still a moving piston.
    fn moving_piston_from_nbt(&self, tag: &Tag) -> Option<(BlockPos, MovingPiston)> {
        use horizon_client::save::{float, number, text};
        if text(tag.get("id")).as_deref() != Some("minecraft:piston") {
            return None;
        }
        let [x, y, z] = block_entity_pos(tag);
        let here = self.world.block(x, y, z);
        if &*self.res.facts().get(here).name != "minecraft:moving_piston" {
            return None;
        }
        let moved_state = tag
            .get("blockState")
            .and_then(|t| self.state_from_nbt(t))
            .or_else(|| self.pack.assets.registry.default_state("minecraft:air"))?;
        let direction = usize::try_from(number(tag.get("facing")).unwrap_or(0))
            .ok()
            .and_then(|i| Direction::ALL.get(i).copied())
            .unwrap_or(Direction::Down);
        let progress = float(tag.get("progress")).unwrap_or(0.0) as f32;
        let flag = |k: &str| number(tag.get(k)).unwrap_or(0) != 0;
        Some((
            BlockPos::new(x, y, z),
            MovingPiston {
                moved_state,
                direction,
                extending: flag("extending"),
                source: flag("source"),
                progress,
                progress_o: progress,
                last_ticked: 0,
                death_ticks: 0,
                id: 0,
            },
        ))
    }
    /// Spawn the entities of chunks that finished loading.
    pub(super) fn spawn_loaded_entities(&mut self) {
        let loaded = std::mem::take(&mut self.world.loaded_entities);
        let player_uuid = self.level.player_uuid;
        for (key, tags) in loaded {
            for mut tag in tags {
                // PersistentEntitySectionManager.addEntity's `loaded`.
                let from_disk = !take_fresh(&mut tag);
                // An End gateway's traveller never left the level: nothing joins.
                let moved = take_moved(&mut tag);
                let kind = entity_nbt::kind(&tag).unwrap_or_default();
                let uuid = entity_nbt::uuid(&tag);
                if uuid.is_some_and(|u| self.restored_vehicle(u).is_some()) {
                    continue;
                }
                if (matches!(
                    kind.as_str(),
                    "minecraft:experience_orb"
                        | "minecraft:arrow"
                        | "minecraft:fireball"
                        | "minecraft:small_fireball"
                        | "minecraft:wind_charge"
                        | "minecraft:breeze_wind_charge"
                        | "minecraft:snowball"
                        | "minecraft:egg"
                        | "minecraft:ender_pearl"
                        | "minecraft:falling_block"
                        | "minecraft:firework_rocket"
                        | "minecraft:trident"
                        | "minecraft:end_crystal"
                        | "minecraft:ender_dragon"
                ) || horizon_client::projectiles::thrown_type(&kind).is_some())
                    && let Some(at) = entity_nbt::position(&tag)
                {
                    self.persist
                        .anon
                        .insert(anon_key(&kind, at), (at, uuid, tag.clone()));
                }
                match kind.as_str() {
                    "minecraft:item" => {
                        self.next_entity += 1;
                        let bob = self.world.random.next_float() * std::f32::consts::PI * 2.0;
                        if let Some(e) =
                            entity_nbt::item_entity_from_nbt(&tag, self.next_entity, bob)
                        {
                            let (id, p) = (e.id, e.position);
                            self.remember(e.id, uuid, tag);
                            self.dropped.push(e);
                            if !moved
                                && !self.native_entity_joins(
                                    id,
                                    "minecraft:item",
                                    [p.x, p.y, p.z],
                                    from_disk,
                                )
                            {
                                self.dropped.pop();
                                self.forget_entity(id);
                            }
                        }
                    }
                    "minecraft:experience_orb" => {
                        for mut orb in entity_nbt::orbs_from_nbt(&tag) {
                            self.next_entity += 1;
                            orb.id = self.next_entity;
                            let (id, p) = (orb.id, orb.position);
                            // An orb that came through a portal keeps its cooldown.
                            self.restore_portal_cooldown(id, &tag);
                            self.orbs.push(orb);
                            self.remember_tags(id, &tag);
                            let kind = "minecraft:experience_orb";
                            if !moved
                                && !self.native_entity_joins(id, kind, [p.x, p.y, p.z], from_disk)
                            {
                                self.orbs.pop();
                            }
                        }
                    }
                    "minecraft:arrow" => {
                        if let Some((mut p, in_block)) = entity_nbt::arrow_from_nbt(
                            &tag,
                            player_uuid,
                            super::dispenser::tipped_effect,
                        ) {
                            if let Some(a) = p.arrow.as_mut() {
                                a.last_state = in_block.and_then(|t| self.state_from_nbt(&t));
                            }
                            self.claim_owner(&mut p);
                            // An arrow that came through a portal keeps its
                            // cooldown: it does not go straight back through.
                            self.next_entity += 1;
                            p.id = self.next_entity;
                            self.restore_portal_cooldown(p.id, &tag);
                            self.remember_tags(p.id, &tag);
                            self.add_projectile(p);
                        }
                    }
                    "minecraft:fireball"
                    | "minecraft:small_fireball"
                    | "minecraft:wind_charge"
                    | "minecraft:breeze_wind_charge" => {
                        if let Some(mut p) = entity_nbt::fireball_from_nbt(&tag, player_uuid) {
                            self.claim_owner(&mut p);
                            // A projectile that came through a portal keeps its cooldown.
                            self.next_entity += 1;
                            p.id = self.next_entity;
                            self.restore_portal_cooldown(p.id, &tag);
                            self.add_projectile(p);
                            self.remember_last_projectile_tags(&tag);
                        }
                    }
                    id if entity_nbt::throwable_kind(id).is_some() => {
                        if let Some(mut p) = entity_nbt::throwable_from_nbt(&tag, player_uuid) {
                            self.claim_owner(&mut p);
                            // A projectile that came through a portal keeps its cooldown.
                            self.next_entity += 1;
                            p.id = self.next_entity;
                            self.restore_portal_cooldown(p.id, &tag);
                            self.add_projectile(p);
                            self.remember_last_projectile_tags(&tag);
                        }
                    }
                    "minecraft:falling_block" => {
                        if let Some((state, position, velocity, age, fall_distance)) =
                            entity_nbt::falling_block_from_nbt(&tag)
                            && let Some(state) = self.state_from_nbt(&state)
                        {
                            let (no_gravity, drop_item) = entity_nbt::falling_block_flags(&tag);
                            self.next_entity += 1;
                            // A summoned one has no UUID yet: `Entity` makes a random one.
                            let uuid = uuid.unwrap_or_else(|| self.uuid_for(self.next_entity));
                            self.falling.push(FallingBlock {
                                id: self.next_entity,
                                state,
                                // DATA_START_POS is not persisted by vanilla.
                                start_pos: horizon_core::position::BlockPos::default(),
                                position,
                                previous: position,
                                velocity,
                                age,
                                no_gravity,
                                portal: horizon_client::nether_portal::PortalState {
                                    cooldown: horizon_client::save::number(
                                        tag.get("PortalCooldown"),
                                    )
                                    .unwrap_or(0)
                                        as i32,
                                    process: None,
                                },
                                drop_item,
                                fall_distance,
                                stuck: Vec3::ZERO,
                                damage: horizon_client::falling_block::Damage::from_nbt(
                                    &tag,
                                    self.res.facts().get(state).name.ends_with("anvil"),
                                ),
                                saved: Some(Box::new((uuid, tag.clone()))),
                            });
                        }
                    }
                    "minecraft:trident" => {
                        if let Some(n) = entity_nbt::trident_from_nbt(&tag) {
                            self.tridents
                                .push(super::trident::ThrownTrident::from_nbt(n));
                        }
                    }
                    "minecraft:firework_rocket" => {
                        if let Some(r) = entity_nbt::firework_rocket_from_nbt(&tag, player_uuid) {
                            self.next_entity += 1;
                            let id = self.next_entity;
                            let mut rocket = super::fireworks::Rocket::from_nbt(r);
                            rocket.id = id;
                            self.rockets.push(rocket);
                            self.remember(id, uuid, tag);
                        }
                    }
                    // PrimedTnt.readAdditionalSaveData: its fuse (80 when absent) and motion.
                    // Its Passengers ride it (EntityType.loadEntityRecursive forces them on).
                    "minecraft:tnt" => {
                        let mut tag = tag;
                        let riders = self.take_mob_passengers(&mut tag);
                        if let Some(position) = entity_nbt::position(&tag) {
                            self.next_entity += 1;
                            let id = self.next_entity;
                            let fuse = match tag.get("fuse") {
                                Some(Tag::Short(f)) => i32::from(*f),
                                _ => 80,
                            };
                            self.tnts.push(super::tnt::PrimedTnt {
                                no_gravity: entity_nbt::flag(&tag, "NoGravity"),
                                id,
                                position,
                                previous: position,
                                velocity: entity_nbt::motion(&tag),
                                fuse,
                                on_ground: matches!(tag.get("OnGround"), Some(Tag::Byte(1))),
                                owner: None,
                                guest: None,
                            });
                            // `Entity.load`: the `PortalCooldown` of one that just came through.
                            self.remember(id, uuid, tag);
                            for rider in riders {
                                self.load_mob(rider, Some(id));
                            }
                        }
                    }
                    k if super::boats::any_boat_wood(k)
                        .is_some_and(|(wood, chest)| self.boat_models.has(wood, chest)) =>
                    {
                        if let Some((position, velocity, yaw)) = entity_nbt::boat_from_nbt(&tag) {
                            let mut tag = tag;
                            let riders = self.take_mob_passengers(&mut tag);
                            self.next_entity += 1;
                            let id = self.next_entity;
                            let (wood, chest) =
                                super::boats::any_boat_wood(k).unwrap_or(("oak", false));
                            let mut b = super::boats::Boat::new(
                                id,
                                std::sync::Arc::from(wood),
                                position,
                                yaw,
                            );
                            b.no_gravity = entity_nbt::flag(&tag, "NoGravity");
                            if chest {
                                b = b.with_chest();
                                b.items = horizon_client::save::decode_items(
                                    tag.get("Items"),
                                    super::boats::CHEST_SLOTS,
                                );
                            }
                            b.velocity = velocity;
                            // AbstractBoat.readAdditionalSaveData: the lead's holder.
                            b.leash_pending =
                                horizon_client::mobs::leash::pending_from_nbt(tag.get("leash"));
                            self.remember(id, uuid, tag);
                            self.boats.push(b);
                            for rider in riders {
                                self.load_mob(rider, Some(id));
                            }
                        }
                    }
                    k if super::minecarts::CartKind::from_id(k).is_some() => {
                        let kind = super::minecarts::CartKind::from_id(k).unwrap();
                        let mut tag = tag;
                        let riders = self.take_mob_passengers(&mut tag);
                        let id = self.next_entity + 1;
                        if let Some(mut c) = super::minecarts::Minecart::from_nbt(&tag, id, kind) {
                            self.roll_cart_loot(&mut c, &tag);
                            self.next_entity = id;
                            self.remember(id, uuid, tag);
                            self.carts.push(c);
                            for rider in riders {
                                self.load_mob(rider, Some(id));
                            }
                        }
                    }
                    "minecraft:ominous_item_spawner" => self.load_ominous_item_spawner(tag),
                    "minecraft:ender_dragon" => {
                        let seed = self.world.random.next_long();
                        if self.dragon.is_none()
                            && let Some(mut d) = super::dragon::Dragon::from_nbt(&tag, seed)
                        {
                            // EnderDragon's constructor takes its id and its
                            // parts' from the entity counter.
                            d.id = self.next_entity + 1;
                            self.next_entity += 9;
                            self.remember_tags(d.id, &tag);
                            self.dragon = Some(d);
                        }
                    }
                    "minecraft:end_crystal" => {
                        let time = self.world.random.next_int_bound(100_000).unwrap_or(0);
                        if let Some(mut c) = super::dragon::Crystal::from_nbt(&tag, time) {
                            // EndCrystal's constructor takes an id from the
                            // entity counter.
                            self.next_entity += 1;
                            c.id = self.next_entity;
                            self.remember_tags(c.id, &tag);
                            self.crystals.push(c);
                        }
                    }
                    "minecraft:armor_stand" => {
                        self.next_entity += 1;
                        if let Some(a) = entity_nbt::armor_stand_from_nbt(&tag, self.next_entity) {
                            self.remember(a.id, uuid, tag);
                            self.armor_stands.push(a);
                        }
                    }
                    "minecraft:item_frame" | "minecraft:glow_item_frame" => {
                        self.next_entity += 1;
                        if let Some(f) = entity_nbt::item_frame_from_nbt(&tag, self.next_entity) {
                            self.remember(f.id, uuid, tag);
                            self.item_frames.push(f);
                        }
                    }
                    "minecraft:cushion" => {
                        self.next_entity += 1;
                        if let Some(c) = entity_nbt::cushion_from_nbt(&tag, self.next_entity) {
                            self.remember(c.id, uuid, tag);
                            self.cushions.push(c);
                        }
                    }
                    "minecraft:painting" => {
                        if let Some((pos, facing, variant)) = entity_nbt::painting_from_nbt(&tag)
                            && let Some(v) = self.painting_variants.get(&variant).cloned()
                        {
                            self.next_entity += 1;
                            let id = self.next_entity;
                            self.remember(id, uuid, tag);
                            self.paintings.push(horizon_client::entities::Painting {
                                id,
                                pos,
                                facing,
                                variant: v.id,
                                width: v.width,
                                height: v.height,
                                check_interval: 0,
                            });
                        } else {
                            self.persist.unknown.entry(key).or_default().push(tag);
                        }
                    }
                    _ if self.res.mob_types().contains_key(&kind) => {
                        self.load_mob(tag, None);
                    }
                    _ if !kind.is_empty() && entity_nbt::position(&tag).is_some() => {
                        self.load_other(tag, &kind);
                    }
                    _ => self.persist.unknown.entry(key).or_default().push(tag),
                }
            }
        }
    }
    /// A saved mob and, under `Passengers`, the mobs riding it.
    pub(super) fn load_mob(&mut self, tag: Tag, mount: Option<u32>) -> Option<u32> {
        let uuid = entity_nbt::uuid(&tag);
        if let Some(id) = uuid.and_then(|u| self.restored_vehicle(u)) {
            return Some(id);
        }
        self.next_entity += 1;
        // Structure mobs that generation left to finish: saves from before
        // they were finalized at placement, or a placement that could not.
        let mut tag = tag;
        let mut finalize = false;
        if let Tag::Compound(c) = &mut tag {
            finalize = c.remove(&"horizon:finalize".into()).is_some();
        }
        let riders = self.take_mob_passengers(&mut tag);
        let mut m = entity_nbt::mob_from_nbt(
            &tag,
            self.res.mob_types(),
            self.next_entity,
            Some(self.pack.assets.registry),
        )?;
        if horizon_client::mobs::camel::is_camel(m.species()) {
            horizon_client::mobs::camel::finish_load(&mut m, self.world.game_time);
        }
        // `NeutralMob.readPersistentAngerSaveData`: the anger runs until its
        // end time, and the mob is angry at the target again, through
        // `Mob.setTarget` -> `asValidTarget`: a player the golem cannot attack
        // (`IronGolem.canAttack`) comes back as no target.
        m.extra.anger = horizon_client::mobs::anger_left(m.extra.anger_end, self.world.game_time);
        if m.extra.anger > 0 && m.extra.angry_at.is_some() {
            m.ai.target = (!m.built_by_player()).then_some(horizon_client::mobs::Who::Player);
        }
        m.extra.mount = mount;
        m.extra.passenger = mount.is_some();
        if finalize {
            let facts = self.biome_facts(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let at = [m.position.x, m.position.y, m.position.z];
            let difficulty = self.local_difficulty(at);
            horizon_client::mobs::spawning::finalize(&mut m, &facts, &difficulty);
            // Cat.finalizeSpawn: black in #cats_spawn_as_black.
            if m.species() == horizon_client::mobs::Species::Cat {
                let p = [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ];
                let black = self.structure_tag("minecraft:cats_spawn_as_black");
                if self.structures_at(p).iter().any(|s| black.contains(s)) {
                    m.extra.variant = 10;
                    m.persistent = true;
                }
            }
        }
        let id = m.id;
        self.remember(id, uuid, tag);
        self.mobs.push(m);
        if finalize {
            self.enchant_spawn_equipment(id);
            self.halloween_spawn_head(id);
        }
        for rider in riders {
            if let Some(r) = self.load_mob(rider, Some(id))
                && let Some(v) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                v.extra.rider = Some(r);
            }
        }
        Some(id)
    }
    /// The mobs under an entity's `Passengers`, taken out of it; the
    /// passengers Horizon doesn't simulate stay.
    fn take_mob_passengers(&self, tag: &mut Tag) -> Vec<Tag> {
        let mut riders = Vec::new();
        if let Tag::Compound(c) = tag
            && let Some(Tag::List(list)) = c.remove(&"Passengers".into())
        {
            let mut unknown = Vec::new();
            for p in list {
                let known =
                    entity_nbt::kind(&p).is_some_and(|k| self.res.mob_types().contains_key(&k));
                if known {
                    riders.push(p);
                } else {
                    unknown.push(p);
                }
            }
            if !unknown.is_empty() {
                c.insert("Passengers".into(), Tag::List(unknown));
            }
        }
        riders
    }
    /// The mobs riding entity `id` under `tag`'s `Passengers`, after the
    /// passengers it kept from loading.
    fn put_mob_passengers(&mut self, tag: &mut Tag, id: u32) {
        let riders: Vec<usize> = (0..self.mobs.len())
            .filter(|&j| self.mobs[j].extra.mount == Some(id) && self.mobs[j].alive())
            .collect();
        let riders: Vec<Tag> = riders.into_iter().map(|j| self.mob_tree_nbt(j)).collect();
        add_passengers(tag, riders);
    }
    /// An entity of a type Horizon does not simulate, kept as its compound.
    fn load_other(&mut self, mut tag: Tag, kind: &str) -> Option<u32> {
        strip_finalize(&mut tag);
        let position = entity_nbt::position(&tag)?;
        let rotation = match tag.get("Rotation") {
            Some(Tag::List(l)) => {
                let f = |i: usize| match l.get(i) {
                    Some(Tag::Float(v)) => f32::from_bits(*v),
                    _ => 0.0,
                };
                [f(0), f(1)]
            }
            _ => [0.0, 0.0],
        };
        self.next_entity += 1;
        let id = self.next_entity;
        let uuid = entity_nbt::uuid(&tag);
        self.remember(id, uuid, tag);
        self.others.push(super::commands::OtherEntity {
            id,
            kind: kind.to_owned(),
            position,
            yaw: rotation[0],
            pitch: rotation[1],
        });
        Some(id)
    }
    /// The compound of an entity Horizon does not simulate, with its current
    /// position, rotation and UUID.
    pub(super) fn other_to_nbt(&self, o: &super::commands::OtherEntity, uuid: [i32; 4]) -> Tag {
        let mut tag = self
            .persist
            .raw
            .get(&o.id)
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Default::default()));
        if let Tag::Compound(c) = &mut tag {
            c.insert("id".into(), Tag::String(o.kind.as_str().into()));
            c.insert(
                "Pos".into(),
                Tag::List(
                    [o.position.x, o.position.y, o.position.z]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            c.insert(
                "Rotation".into(),
                Tag::List(vec![
                    Tag::Float(o.yaw.to_bits()),
                    Tag::Float(o.pitch.to_bits()),
                ]),
            );
            c.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        }
        tag
    }
    /// The root of a stack of riders (`Entity.getRootVehicle`).
    fn mob_root(&self, mut i: usize) -> usize {
        for _ in 0..8 {
            let Some(v) = self.mobs[i].extra.mount else {
                break;
            };
            let Some(j) = self.mobs.iter().position(|m| m.id == v) else {
                break;
            };
            i = j;
        }
        i
    }
    /// `Entity.getVehicle` of entity `id`: the mob, boat or minecart a mob
    /// rides (only a mob rides another entity here).
    fn vehicle_of(&self, id: u32) -> Option<u32> {
        let v = self.mobs.iter().find(|m| m.id == id)?.extra.mount?;
        let there = self.mobs.iter().any(|m| m.id == v)
            || self.boats.iter().any(|b| b.id == v)
            || self.carts.iter().any(|c| c.id == v);
        there.then_some(v)
    }
    /// The root vehicle `root` with its passengers (`Entity.save`).
    fn vehicle_tree_nbt(&mut self, root: u32) -> Option<Tag> {
        if let Some(i) = self.mobs.iter().position(|m| m.id == root) {
            return Some(self.mob_tree_nbt(i));
        }
        if self.boats.iter().any(|b| b.id == root) {
            let uuid = self.uuid_for(root);
            let b = self.boats.iter().find(|b| b.id == root)?;
            let mut tag = entity_nbt::boat_to_nbt(
                &b.entity_type(),
                b.position,
                b.velocity,
                b.yaw,
                uuid,
                self.persist.raw.get(&root),
                b.chest.then_some(&b.items[..]),
            );
            self.put_mob_passengers(&mut tag, root);
            return Some(tag);
        }
        if !self.carts.iter().any(|c| c.id == root) {
            return None;
        }
        let uuid = self.uuid_for(root);
        let c = self.carts.iter().find(|c| c.id == root)?;
        let mut tag = c.to_nbt(uuid, self.persist.raw.get(&root));
        self.put_mob_passengers(&mut tag, root);
        Some(tag)
    }
    /// `ServerPlayer.saveParentVehicle`: a player riding `vehicle` saves
    /// RootVehicle {Attach: that mount's UUID, Entity: the root vehicle with
    /// its passengers}, only while it is the root's one player passenger
    /// (`Entity.hasExactlyOnePlayerPassenger`); `others` are the vehicles the
    /// other players in the level ride. With the root and the mount's UUID.
    pub(super) fn parent_vehicle_tag(
        &mut self,
        vehicle: Option<u32>,
        others: &[u32],
    ) -> Option<(Tag, u32, [i32; 4])> {
        let vehicle = vehicle?;
        let root = parent_vehicle_root(vehicle, others, |id| self.vehicle_of(id))?;
        let mut entity = self.vehicle_tree_nbt(root)?;
        write_attachments(&mut entity, &self.entity_attachments);
        let attach = self.uuid_for(vehicle);
        Some((root_vehicle_tag(attach, entity), root, attach))
    }
    /// `PlayerList.remove`: the root vehicle and its passengers leave the
    /// world with the player (`setRemoved(UNLOADED_WITH_PLAYER)`).
    pub(super) fn remove_vehicle_tree(&mut self, root: u32) {
        // The mobs `put_mob_passengers` saves under each vehicle of the tree.
        let mounts: Vec<(u32, Option<u32>)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (m.id, m.extra.mount))
            .collect();
        let gone = vehicle_tree(root, &mounts);
        self.mobs.retain(|m| !gone.contains(&m.id));
        for &id in &gone {
            self.release_vehicle_container(id);
        }
        self.boats.retain(|b| !gone.contains(&b.id));
        self.carts.retain(|c| !gone.contains(&c.id));
        for g in gone {
            self.forget_entity(g);
        }
    }
    /// The vehicle with this UUID, among the ones in the world.
    pub(super) fn vehicle_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        let id = *self
            .persist
            .uuids
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| id)?;
        let there = self.mobs.iter().any(|m| m.id == id && m.alive())
            || self.boats.iter().any(|b| b.id == id)
            || self.carts.iter().any(|c| c.id == id);
        there.then_some(id)
    }
    /// A vehicle already in the world that was restored from the player's file
    /// during this load (not any entity whose UUID is remembered).
    fn restored_vehicle(&self, uuid: [i32; 4]) -> Option<u32> {
        if !self.persist.restored.contains(&uuid) {
            return None;
        }
        self.vehicle_by_uuid(uuid)
    }
    /// Vanilla stores the root entity in the player's file, outside chunk entities.
    /// Keep the raw wrapper until attachment succeeds, including unsupported trees.
    pub(super) fn restore_saved_vehicle(
        &mut self,
        uuid: [i32; 4],
        raw: Option<&Tag>,
    ) -> Option<u32> {
        if let Some(id) = self.vehicle_by_uuid(uuid) {
            return Some(id);
        }
        let tag = raw?.get("RootVehicle")?.get("Entity")?.clone();
        let root_uuid = entity_nbt::uuid(&tag)?;
        let kind = entity_nbt::kind(&tag)?;
        if !self.res.mob_types().contains_key(&kind)
            && super::boats::any_boat_wood(&kind).is_none()
            && super::minecarts::CartKind::from_id(&kind).is_none()
        {
            return None;
        }
        if self.vehicle_by_uuid(root_uuid).is_none() {
            let at = entity_nbt::position(&tag)?;
            let key = (
                (at.x.floor() as i32).div_euclid(16),
                (at.z.floor() as i32).div_euclid(16),
            );
            self.persist.restored.insert(root_uuid);
            self.world.loaded_entities.push((key, vec![tag]));
            self.spawn_loaded_entities();
        }
        self.persist.restored.insert(uuid);
        self.vehicle_by_uuid(uuid)
    }
    /// `ServerPlayer.loadAndSpawnParentVehicle`: the ride the player's file
    /// names, once the vehicle is in the world.
    pub(super) fn restore_root_vehicle(&mut self) {
        if self.serving.is_some()
            || self.options.dedicated.is_some()
            || self.riding.is_some()
            || self.player.is_none()
        {
            return;
        }
        let Some(uuid) = self.pending_vehicle else {
            return;
        };
        let raw = self.level.player_extra.clone();
        let Some(id) = self.restore_saved_vehicle(uuid, raw.as_ref()) else {
            return;
        };
        if let Some(Tag::Compound(p)) = &mut self.level.player_extra {
            p.remove(&"RootVehicle".into());
        }
        self.pending_vehicle = None;
        self.riding = Some(id);
    }
    /// A mob's entity tag with the mobs riding it under `Passengers`.
    pub(super) fn mob_tree_nbt(&mut self, i: usize) -> Tag {
        let id = self.mobs[i].id;
        let uuid = self.uuid_for(id);
        // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
        // absolute game time, so it goes on while the chunk is unloaded, and
        // the target is the entity the mob is angry at.
        let anger = self.mobs[i].extra.anger;
        self.mobs[i].extra.anger_end = if anger > 0 {
            self.world.game_time + i64::from(anger)
        } else {
            -1
        };
        self.mobs[i].extra.angry_at = if anger > 0 {
            // A wolf keeps its anger's target itself, also once the target
            // is gone (`tame::update_persistent_anger`).
            if self.mobs[i].species() == horizon_client::mobs::Species::Wolf {
                self.mobs[i].extra.angry_at
            } else {
                self.mobs[i].ai.target
            }
        } else {
            None
        };
        let mut tag = entity_nbt::mob_to_nbt(
            &self.mobs[i],
            uuid,
            self.persist.raw.get(&id),
            Some(self.pack.assets.registry),
        );
        if horizon_client::mobs::camel::is_camel(self.mobs[i].species())
            && let Tag::Compound(c) = &mut tag
        {
            let tick =
                horizon_client::mobs::camel::synced_pose_tick(&self.mobs[i], self.world.game_time);
            c.insert("LastPoseTick".into(), Tag::Long(tick));
        }
        self.write_leash(&mut tag, i);
        self.put_mob_passengers(&mut tag, id);
        tag
    }
    /// Forget an entity that left the world (its UUID and loaded compound).
    pub(super) fn forget_entity(&mut self, id: u32) {
        if let Some(uuid) = self.persist.uuids.remove(&id) {
            self.entity_attachments.remove(&uuid);
        }
        self.persist.raw.remove(&id);
    }
    /// Detach a travelling entity without discarding its in-memory attachments.
    pub(super) fn detach_entity(&mut self, id: u32) {
        self.persist.uuids.remove(&id);
        self.persist.raw.remove(&id);
    }
    /// The compound an entity without a Horizon entity ID was loaded with
    /// (the crystals and the dragon), found again by type and position.
    pub(super) fn persist_anon(&self, kind: &str, p: Vec3) -> (Option<[i32; 4]>, Option<&Tag>) {
        self.persist
            .anon
            .get(&anon_key(kind, p))
            .map_or((None, None), |(_, uuid, raw)| (*uuid, Some(raw)))
    }
    /// The compound an entity was loaded with (and commands have edited).
    pub(super) fn persist_raw(&self, id: u32) -> Option<&Tag> {
        self.persist.raw.get(&id)
    }
    /// A loaded chest minecart with a loot table and no items: the table
    /// rolled now, as generated containers are (Horizon does not unpack on
    /// first open), and kept to save it again while unchanged.
    fn roll_cart_loot(&self, cart: &mut super::minecarts::Minecart, tag: &Tag) {
        let Some(table) = horizon_client::save::text(tag.get("LootTable")) else {
            return;
        };
        if cart.items.is_empty() || cart.items.iter().any(Option::is_some) {
            return;
        }
        let loot = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        );
        let p = cart.position;
        let pos = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let biome = self.biome_name_at(p).unwrap_or_default();
        let seed = horizon_client::structure_blocks::loot_seed(tag);
        cart.loot = Some(horizon_client::structure_blocks::roll_container(
            &mut cart.items,
            loot.as_deref(),
            table,
            seed,
            pos,
            &biome,
        ));
    }
    /// An entity Horizon keeps no compound for (orbs, projectiles, the
    /// crystals and the dragon) still keeps its `Tags`, by entity id, so
    /// `@e[tag=]`, `/tag` and `/data get` see them.
    pub(super) fn remember_tags(&mut self, id: u32, tag: &Tag) {
        if let Some(tags @ Tag::List(_)) = tag.get("Tags") {
            let mut map = horizon_world::nbt::Compound::new();
            map.insert("Tags".into(), tags.clone());
            self.persist.raw.insert(id, Tag::Compound(map));
        }
    }
    /// The `Tags` of the projectile `add_projectile` just pushed.
    fn remember_last_projectile_tags(&mut self, tag: &Tag) {
        if let Some(id) = self.creatures.projectiles.last().map(|p| p.id) {
            self.remember_tags(id, tag);
        }
    }
    fn remember(&mut self, id: u32, uuid: Option<[i32; 4]>, tag: Tag) {
        self.restore_portal_cooldown(id, &tag);
        if let Some(u) = uuid {
            self.persist.uuids.insert(id, u);
            read_attachments(&tag, u, &mut self.entity_attachments);
        }
        self.persist.raw.insert(id, tag);
    }
    /// The mob that was loaded or saved with a UUID.
    pub(super) fn mob_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.persist
            .uuids
            .iter()
            .find(|(id, u)| **u == uuid && self.mobs.iter().any(|m| m.id == **id))
            .map(|(id, _)| *id)
    }
    /// A mob's UUID, made up for new ones.
    pub(super) fn mob_uuid(&mut self, id: u32) -> [i32; 4] {
        self.uuid_for(id)
    }
    pub(super) fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        *self.persist.uuids.entry(id).or_insert_with(random_uuid)
    }
    /// `BlockState.CODEC`: a block name or `{id, properties}` (or legacy `{Name, Properties}`).
    pub(super) fn state_from_nbt(&self, tag: &Tag) -> Option<BlockStateId> {
        entity_nbt::state_from_nbt(self.pack.assets.registry, tag)
    }
    pub(in crate::game) fn state_to_nbt(&self, state: BlockStateId) -> Tag {
        entity_nbt::state_to_nbt(self.pack.assets.registry, state)
    }
    /// The entity NBT of every simulated entity in `keys`, plus the unknown
    /// entities kept for them. With `remove`, the entities leave the world
    /// (their chunks unload).
    pub(super) fn entity_chunks(
        &mut self,
        keys: &HashSet<(i32, i32)>,
        remove: bool,
    ) -> Vec<((i32, i32), Vec<Tag>)> {
        let mut out: HashMap<(i32, i32), Vec<Tag>> =
            keys.iter().map(|k| (*k, Vec::new())).collect();
        // A rider is saved inside its root vehicle, in the root's chunk; a
        // boat's passengers inside the boat.
        let mobs: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| {
                self.mob_root(i) == i
                    && self.mobs[i].extra.mount.is_none_or(|v| {
                        self.boats.iter().all(|b| b.id != v) && self.carts.iter().all(|c| c.id != v)
                    })
                    && keys.contains(&chunk_of(self.mobs[i].position))
                    && self.mobs[i].alive()
                    && !self.mobs[i].removed
            })
            .collect();
        for i in mobs {
            let tag = self.mob_tree_nbt(i);
            out.entry(chunk_of(self.mobs[i].position))
                .or_default()
                .push(tag);
        }
        let items: Vec<usize> = (0..self.dropped.len())
            .filter(|&i| keys.contains(&chunk_of(self.dropped[i].position)))
            .collect();
        for i in items {
            let id = self.dropped[i].id;
            let uuid = self.uuid_for(id);
            let tag =
                entity_nbt::item_entity_to_nbt(&self.dropped[i], uuid, self.persist.raw.get(&id));
            out.entry(chunk_of(self.dropped[i].position))
                .or_default()
                .push(tag);
        }
        let anon = |persist: &EntityMemory, kind: &str, at: Vec3| {
            persist
                .anon
                .get(&anon_key(kind, at))
                .map(|(_, uuid, raw)| (uuid.unwrap_or_else(random_uuid), Some(raw.clone())))
                .unwrap_or_else(|| (random_uuid(), None))
        };
        for o in &self.orbs {
            if keys.contains(&chunk_of(o.position)) {
                let (uuid, raw) = anon(&self.persist, "minecraft:experience_orb", o.position);
                out.entry(chunk_of(o.position)).or_default().push(with_raw(
                    with_raw(entity_nbt::orb_to_nbt(o, uuid), raw.as_ref()),
                    self.persist.raw.get(&o.id),
                ));
            }
        }
        for p in &self.creatures.projectiles {
            if !keys.contains(&chunk_of(p.position)) || !entity_nbt::saved_with_chunk(p) {
                continue;
            }
            let owner = p.owner_uuid;
            let (uuid, raw) = anon(&self.persist, p.kind.id(), p.position);
            let in_block = p
                .arrow
                .as_ref()
                .and_then(|a| a.last_state)
                .map(|s| self.state_to_nbt(s));
            let tag = entity_nbt::arrow_to_nbt(p, uuid, owner, in_block, raw.as_ref())
                .or_else(|| entity_nbt::fireball_to_nbt(p, uuid, owner, raw.as_ref()))
                .or_else(|| entity_nbt::throwable_to_nbt(p, uuid, owner, raw.as_ref()));
            if let Some(tag) = tag {
                let tag = with_raw(tag, self.persist.raw.get(&p.id));
                out.entry(chunk_of(p.position)).or_default().push(tag);
            }
        }
        for f in &self.falling {
            if keys.contains(&chunk_of(f.position)) {
                let (uuid, raw) = match &f.saved {
                    Some(saved) => (saved.0, Some(saved.1.clone())),
                    None => anon(&self.persist, "minecraft:falling_block", f.position),
                };
                let tag = entity_nbt::falling_block_to_nbt(
                    self.state_to_nbt(f.state),
                    f.position,
                    f.velocity,
                    f.age,
                    f.drop_item,
                    f.damage,
                    f.fall_distance,
                    uuid,
                );
                let mut tag = with_raw(tag, raw.as_ref());
                if let Tag::Compound(fields) = &mut tag {
                    fields.insert("PortalCooldown".into(), Tag::Int(f.portal.cooldown));
                }
                out.entry(chunk_of(f.position)).or_default().push(tag);
            }
        }
        for i in 0..self.rockets.len() {
            if keys.contains(&chunk_of(self.rockets[i].position)) {
                let (id, at) = (self.rockets[i].id, self.rockets[i].position);
                let uuid = self.uuid_for(id);
                let tag =
                    self.rockets[i].to_nbt(uuid, self.level.player_uuid, self.persist.raw.get(&id));
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for t in &self.tridents {
            if keys.contains(&chunk_of(t.position)) {
                let (uuid, raw) = anon(&self.persist, "minecraft:trident", t.position);
                out.entry(chunk_of(t.position))
                    .or_default()
                    .push(t.to_nbt(uuid, raw.as_ref()));
            }
        }
        for i in 0..self.tnts.len() {
            let t = &self.tnts[i];
            if keys.contains(&chunk_of(t.position)) {
                let (id, at) = (t.id, t.position);
                let uuid = self.uuid_for(id);
                let t = &self.tnts[i];
                let tag = entity_nbt::tnt_to_nbt(
                    t.position,
                    t.velocity,
                    t.on_ground,
                    t.fuse,
                    uuid,
                    self.persist.raw.get(&id),
                );
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        let boats: Vec<usize> = (0..self.boats.len())
            .filter(|&i| keys.contains(&chunk_of(self.boats[i].position)))
            .collect();
        for i in boats {
            let b = &self.boats[i];
            let (id, at) = (b.id, b.position);
            let uuid = self
                .persist
                .uuids
                .get(&id)
                .copied()
                .unwrap_or_else(random_uuid);
            let mut tag = entity_nbt::boat_to_nbt(
                &b.entity_type(),
                b.position,
                b.velocity,
                b.yaw,
                uuid,
                self.persist.raw.get(&id),
                b.chest.then_some(&b.items[..]),
            );
            self.put_mob_passengers(&mut tag, id);
            // AbstractBoat.addAdditionalSaveData: the lead's holder.
            self.write_boat_leash(&mut tag, i);
            out.entry(chunk_of(at)).or_default().push(tag);
        }
        if let Some(d) = &self.dragon
            && keys.contains(&chunk_of(d.position))
        {
            let (uuid, raw) = anon(&self.persist, "minecraft:ender_dragon", d.position);
            out.entry(chunk_of(d.position))
                .or_default()
                .push(d.to_nbt(uuid, raw.as_ref()));
        }
        for c in &self.crystals {
            if keys.contains(&chunk_of(c.position)) {
                let (uuid, raw) = anon(&self.persist, "minecraft:end_crystal", c.position);
                out.entry(chunk_of(c.position))
                    .or_default()
                    .push(c.to_nbt(uuid, raw.as_ref()));
            }
        }
        for s in &self.ominous_spawners {
            let at = Vec3::new(s.position[0], s.position[1], s.position[2]);
            if keys.contains(&chunk_of(at)) {
                let tag = entity_nbt::ominous_item_spawner_to_nbt(
                    &s.item,
                    at,
                    s.spawn_item_after,
                    s.uuid,
                    s.raw.as_deref(),
                );
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        let carts: Vec<usize> = (0..self.carts.len())
            .filter(|&i| keys.contains(&chunk_of(self.carts[i].position)))
            .collect();
        for i in carts {
            let id = self.carts[i].id;
            let at = self.carts[i].position;
            let uuid = self
                .persist
                .uuids
                .get(&id)
                .copied()
                .unwrap_or_else(random_uuid);
            let mut tag = self.carts[i].to_nbt(uuid, self.persist.raw.get(&id));
            self.put_mob_passengers(&mut tag, id);
            out.entry(chunk_of(at)).or_default().push(tag);
        }
        for a in &self.armor_stands {
            if keys.contains(&chunk_of(a.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&a.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::armor_stand_to_nbt(a, uuid, self.persist.raw.get(&a.id));
                out.entry(chunk_of(a.position)).or_default().push(tag);
            }
        }
        for f in &self.item_frames {
            let (lo, hi) = f.bounds();
            let at = Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            );
            if keys.contains(&chunk_of(at)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&f.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::item_frame_to_nbt(f, uuid, self.persist.raw.get(&f.id));
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for c in &self.cushions {
            if keys.contains(&chunk_of(c.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&c.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::cushion_to_nbt(c, uuid, self.persist.raw.get(&c.id));
                out.entry(chunk_of(c.position)).or_default().push(tag);
            }
        }
        for p in &self.paintings {
            let at = painting_center(p);
            if keys.contains(&chunk_of(at)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&p.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::painting_to_nbt(p, uuid, self.persist.raw.get(&p.id));
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for o in &self.others {
            if keys.contains(&chunk_of(o.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&o.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = self.other_to_nbt(o, uuid);
                out.entry(chunk_of(o.position)).or_default().push(tag);
            }
        }
        for key in keys {
            let unknown = if remove {
                self.persist.unknown.remove(key)
            } else {
                self.persist.unknown.get(key).cloned()
            };
            if let Some(list) = unknown {
                out.entry(*key).or_default().extend(list);
            }
        }
        for tags in out.values_mut() {
            for tag in tags {
                write_attachments(tag, &self.entity_attachments);
            }
        }
        if remove {
            let gone = |p: Vec3| keys.contains(&chunk_of(p));
            let mut removed: Vec<u32> = Vec::new();
            let riding = self.riding;
            self.boats.retain(|b| {
                let keep = !gone(b.position) || Some(b.id) == riding;
                if !keep {
                    removed.push(b.id);
                }
                keep
            });
            self.crystals.retain(|c| !gone(c.position));
            self.ominous_spawners
                .retain(|s| !gone(Vec3::new(s.position[0], s.position[1], s.position[2])));
            if self.dragon.as_ref().is_some_and(|d| gone(d.position)) {
                self.dragon = None;
            }
            self.carts.retain(|c| {
                let keep = !gone(c.position) || Some(c.id) == riding;
                if !keep {
                    removed.push(c.id);
                }
                keep
            });
            self.others.retain(|o| {
                let keep = !gone(o.position);
                if !keep {
                    removed.push(o.id);
                }
                keep
            });
            self.armor_stands.retain(|a| {
                let keep = !gone(a.position);
                if !keep {
                    removed.push(a.id);
                }
                keep
            });
            self.item_frames.retain(|f| {
                let (lo, hi) = f.bounds();
                let keep = !gone(Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ));
                if !keep {
                    removed.push(f.id);
                }
                keep
            });
            self.paintings.retain(|p| {
                let keep = !gone(painting_center(p));
                if !keep {
                    removed.push(p.id);
                }
                keep
            });
            self.cushions.retain(|c| {
                let keep = !gone(c.position);
                if !keep {
                    removed.push(c.id);
                }
                keep
            });
            // Riders leave with their root vehicle, a boat's with the boat.
            let leaving: HashSet<u32> = (0..self.mobs.len())
                .filter(|&i| {
                    let r = self.mob_root(i);
                    match self.mobs[r].extra.mount {
                        Some(v) if removed.contains(&v) => true,
                        Some(v) if self.boats.iter().any(|b| b.id == v) => false,
                        _ => gone(self.mobs[r].position),
                    }
                })
                .map(|i| self.mobs[i].id)
                .collect();
            self.mobs.retain(|m| {
                let keep = !leaving.contains(&m.id);
                if !keep {
                    removed.push(m.id);
                }
                keep
            });
            let (items_before, orbs_before) = (self.dropped_ids(), self.orb_ids());
            self.dropped.retain(|e| {
                let keep = !gone(e.position);
                if !keep {
                    removed.push(e.id);
                }
                keep
            });
            self.orbs.retain(|o| !gone(o.position));
            // Unloaded with their chunk: PersistentEntitySectionManager.stopTracking.
            self.native_items_left_since(&items_before);
            self.native_orbs_left_since(&orbs_before);
            // the ones saved with the chunk go with it (a trident too, unsaved).
            self.creatures.projectiles.retain(|p| {
                let leaves = entity_nbt::saved_with_chunk(p) || p.arrow.is_some();
                !leaves || !gone(p.position)
            });
            self.falling.retain(|f| !gone(f.position));
            self.rockets.retain(|r| !gone(r.position));
            self.tridents.retain(|t| !gone(t.position));
            self.tnts.retain(|t| !gone(t.position));
            self.persist
                .anon
                .retain(|_, (at, _, _)| !keys.contains(&chunk_of(*at)));
            for id in removed {
                self.forget_entity(id);
            }
        }
        out.into_iter().collect()
    }
    /// Write the entities of every resident chunk (an autosave or quit).
    /// Without `wait` (the autosave) they are written on the region writer
    /// meanwhile: `PersistentEntitySectionManager.autoSave` stores them
    /// without `saveAll`'s flush.
    pub(super) fn save_entities(&mut self, wait: bool) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, false);
        self.prune_entity_memory();
        if wait {
            self.save.save_entities(chunks)
        } else {
            self.save.save_entities_later(chunks)
        }
    }
    pub(super) fn prune_entity_memory(&mut self) {
        // Forget entities that no longer exist.
        let alive: HashSet<u32> = self
            .mobs
            .iter()
            .map(|m| m.id)
            .chain(self.dropped.iter().map(|e| e.id))
            .chain(self.boats.iter().map(|e| e.id))
            .chain(self.carts.iter().map(|e| e.id))
            .chain(self.armor_stands.iter().map(|e| e.id))
            .chain(self.cushions.iter().map(|e| e.id))
            .chain(self.item_frames.iter().map(|e| e.id))
            .chain(self.paintings.iter().map(|e| e.id))
            .chain(self.others.iter().map(|e| e.id))
            .chain(self.orbs.iter().map(|e| e.id))
            .chain(self.creatures.projectiles.iter().map(|e| e.id))
            .chain(self.crystals.iter().map(|e| e.id))
            .chain(self.dragon.iter().map(|e| e.id))
            .chain(self.tnts.iter().map(|e| e.id))
            .collect();
        for (id, uuid) in &self.persist.uuids {
            if !alive.contains(id) {
                self.entity_attachments.remove(uuid);
            }
        }
        self.persist.uuids.retain(|id, _| alive.contains(id));
        self.persist.raw.retain(|id, _| alive.contains(id));
    }
    /// A loaded projectile's `Owner`: a connected guest, else a mob by UUID.
    fn claim_owner(&self, p: &mut horizon_client::projectiles::Projectile) {
        let Some(uuid) = p.owner_uuid else {
            return;
        };
        let guest = self.lan.as_ref().and_then(|host| {
            let ints = horizon_client::lan::playerdata::uuid_ints;
            host.hub
                .guests
                .iter()
                .find(|g| ints(g.uuid) == uuid)
                .map(|g| g.conn)
        });
        if guest.is_some() {
            (p.owner, p.guest) = (Some(Who::Player), guest);
        } else if p.owner.is_none() {
            p.owner = self.mob_by_uuid(uuid).map(Who::Mob);
        }
    }
    /// Save and remove the entities of chunks that unloaded.
    pub(super) fn unload_entities(&mut self, unloaded: &[Arc<Chunk>]) {
        if unloaded.is_empty() {
            return;
        }
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = unloaded.iter().map(|c| (c.x, c.z)).collect();
        let chunks = self.entity_chunks(&keys, true);
        if let Err(e) = self.save.save_entities_later(chunks) {
            log::warn!("Horizon: failed to save entities: {e}");
        }
    }
    /// Leaving a dimension: its entities are saved with it.
    pub(super) fn save_dimension_entities(&mut self) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, true);
        self.persist = EntityMemory::default();
        self.save.save_entities(chunks)
    }
}
/// Only a mob is finalized; another entity drops the request.
fn strip_finalize(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.remove(&"horizon:finalize".into());
    }
}
/// `riders` under `tag`'s `Passengers`, after the passengers it kept from
/// loading (`Entity.saveWithoutId`).
fn add_passengers(tag: &mut Tag, riders: Vec<Tag>) {
    if let Tag::Compound(c) = tag {
        let mut list = match c.get(&"Passengers".into()) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        list.extend(riders);
        if list.is_empty() {
            c.remove(&"Passengers".into());
        } else {
            c.insert("Passengers".into(), Tag::List(list));
        }
    }
}
/// `Entity.getRootVehicle`: up the vehicles `vehicle_of` gives, from `id`.
fn root_vehicle(id: u32, vehicle_of: impl Fn(u32) -> Option<u32>) -> u32 {
    let mut root = id;
    // A bound against a loop of mounts.
    for _ in 0..16 {
        match vehicle_of(root) {
            Some(v) if v != root => root = v,
            _ => break,
        }
    }
    root
}
/// `ServerPlayer.saveParentVehicle`'s root: that of `vehicle`, while no
/// other player rides its tree (`Entity.hasExactlyOnePlayerPassenger`);
/// `others` are the vehicles the other players ride.
fn parent_vehicle_root(
    vehicle: u32,
    others: &[u32],
    vehicle_of: impl Fn(u32) -> Option<u32>,
) -> Option<u32> {
    let root = root_vehicle(vehicle, &vehicle_of);
    let shared = others.iter().any(|&o| root_vehicle(o, &vehicle_of) == root);
    (!shared).then_some(root)
}
/// RootVehicle {Attach: the mount's UUID, Entity: the root's tree}.
fn root_vehicle_tag(attach: [i32; 4], entity: Tag) -> Tag {
    Tag::compound([
        ("Attach", Tag::IntArray(attach.to_vec())),
        ("Entity", entity),
    ])
}
/// `Entity.getPassengersAndSelf` of `root`, from each rider's mount.
fn vehicle_tree(root: u32, mounts: &[(u32, Option<u32>)]) -> Vec<u32> {
    let mut tree = vec![root];
    let mut k = 0;
    while k < tree.len() {
        let v = tree[k];
        let riders: Vec<u32> = mounts
            .iter()
            .filter(|(id, m)| *m == Some(v) && !tree.contains(id))
            .map(|(id, _)| *id)
            .collect();
        tree.extend(riders);
        k += 1;
    }
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNT: [i32; 4] = [1, 2, 3, 4];

    fn entity(kind: &str) -> Tag {
        Tag::compound([("id", Tag::String(kind.into()))])
    }
    fn passengers(tag: &Tag) -> Vec<String> {
        match tag.get("Passengers") {
            Some(Tag::List(l)) => l.iter().filter_map(entity_nbt::kind).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn entity_attachment_boundary_includes_passengers() {
        use horizon_world::attachments::{
            AttachmentCodec, AttachmentError, AttachmentType, Attachments,
        };
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
        };
        let root_id = [1, 2, 3, 4];
        let child_id = [5, 6, 7, 8];
        let mut data = Attachments::default();
        data.set(&key, 29).unwrap();
        let maps = [(child_id, data)].into();
        let mut root = Tag::compound([
            ("UUID", Tag::IntArray(root_id.to_vec())),
            (
                "Passengers",
                Tag::List(vec![Tag::compound([(
                    "UUID",
                    Tag::IntArray(child_id.to_vec()),
                )])]),
            ),
        ]);
        write_attachments(&mut root, &maps);
        let Some(Tag::List(children)) = root.get("Passengers") else {
            panic!("passenger missing");
        };
        let mut loaded = HashMap::new();
        read_attachments(&children[0], child_id, &mut loaded);
        assert_eq!(loaded.get_mut(&child_id).unwrap().get(&key), Ok(29));
    }

    #[test]
    fn a_lone_rider_saves_the_horse() {
        let horse = 7;
        assert_eq!(parent_vehicle_root(horse, &[], |_| None), Some(horse));
        let tag = root_vehicle_tag(MOUNT, entity("minecraft:horse"));
        assert_eq!(tag.get("Attach"), Some(&Tag::IntArray(MOUNT.to_vec())));
        let saved = tag.get("Entity").and_then(entity_nbt::kind);
        assert_eq!(saved.as_deref(), Some("minecraft:horse"));
        assert_eq!(vehicle_tree(horse, &[]), vec![horse]);
    }

    #[test]
    fn two_players_on_one_boat_save_nothing() {
        let boat = 3;
        assert_eq!(parent_vehicle_root(boat, &[boat], |_| None), None);
        // A player on a mob in the same boat shares its tree too.
        let pig = 4;
        let of = |id| (id == pig).then_some(boat);
        assert_eq!(parent_vehicle_root(boat, &[pig], of), None);
        assert_eq!(parent_vehicle_root(boat, &[9], of), Some(boat));
    }

    #[test]
    fn a_mob_in_a_boat_rides_under_the_boat() {
        let (boat, pig) = (3, 4);
        let of = |id| (id == pig).then_some(boat);
        assert_eq!(parent_vehicle_root(pig, &[], of), Some(boat));
        assert_eq!(vehicle_tree(boat, &[(pig, Some(boat))]), vec![boat, pig]);
        let mut tag = entity("minecraft:oak_boat");
        add_passengers(&mut tag, vec![entity("minecraft:pig")]);
        assert_eq!(passengers(&tag), ["minecraft:pig"]);
    }

    #[test]
    fn a_cart_saves_and_takes_its_mob() {
        let (cart, zombie, chicken) = (5, 6, 8);
        // A chicken on a zombie in the cart: the cart is the root.
        let mounts = [(zombie, Some(cart)), (chicken, Some(zombie)), (9, None)];
        let of = |id| mounts.iter().find(|m| m.0 == id).and_then(|m| m.1);
        assert_eq!(parent_vehicle_root(chicken, &[], of), Some(cart));
        assert_eq!(vehicle_tree(cart, &mounts), vec![cart, zombie, chicken]);
        let mut tag = entity("minecraft:minecart");
        add_passengers(&mut tag, vec![entity("minecraft:zombie")]);
        assert_eq!(passengers(&tag), ["minecraft:zombie"]);
        // No rider leaves no empty list.
        let mut tag = entity("minecraft:minecart");
        add_passengers(&mut tag, Vec::new());
        assert!(tag.get("Passengers").is_none());
    }

    /// A portal arrival or a `/summon` is told as a fresh add, and the mark
    /// never reaches the entity's kept or saved compound.
    #[test]
    fn a_fresh_add_is_told_apart_from_a_load_and_unmarked() {
        let mut loaded = Tag::Compound(Default::default());
        assert!(!take_fresh(&mut loaded));
        let mut fresh = Tag::Compound(Default::default());
        mark_fresh(&mut fresh);
        assert!(take_fresh(&mut fresh));
        assert!(!take_fresh(&mut fresh));
        assert_eq!(fresh, Tag::Compound(Default::default()));
    }
}
