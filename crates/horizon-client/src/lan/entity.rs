//! Entity tracking (ChunkMap.TrackedEntity / ServerEntity): the game describes
//! every entity it simulates as an `EntityState` each tick. A level's
//! `Entities` remember what its clients were last told about each entity,
//! once for all of them as vanilla's ServerEntity does, and produce the
//! movement, motion, metadata and equipment packets that bring them up to
//! date at the entity type's update interval (`tracking`); each client's
//! `Tracker` is the set of entities it has been paired with.
use super::tracking;
use horizon_protocol::{
    buf::pack_degrees,
    entity_data::{Value, serializer},
    item::Stack,
    packets::play::{
        self, AddEntity, AttributeSnapshot, EntityPositionSync, MoveEntity, MoveEntityPos,
        MoveEntityPosRot, MoveEntityRot, PositionPath, RotateHead, SetEntityData, SetEntityLink,
        SetEntityMotion, SetEquipment, SetPassengers, UpdateAttributes, VecDelta,
    },
};
use std::sync::Arc;

/// The attribute instances of `now` that differ from what `sent` held (an
/// instance is never removed: one that disappears is left as it was sent).
pub fn changed_attributes(
    now: &[AttributeSnapshot],
    sent: &[AttributeSnapshot],
) -> Vec<AttributeSnapshot> {
    now.iter()
        .filter(|a| {
            sent.iter()
                .find(|b| b.attribute == a.attribute)
                .is_none_or(|b| b != *a)
        })
        .cloned()
        .collect()
}

/// What the game says about one entity this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityState {
    pub id: i32,
    pub uuid: u128,
    /// Registry id of the entity type (`minecraft:zombie`).
    pub kind: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub velocity: [f64; 3],
    /// ClientboundAddEntityPacket.data (item frame direction, block state ...).
    pub data: i32,
    /// SynchedEntityData entries that differ from the defaults.
    pub meta: Vec<(u8, i32, Value)>,
    /// The defaults of entries `meta` leaves out while they are at them,
    /// sent when one goes back to its default (a calf grown up, a mob healed
    /// back to 1, wool grown back white) beyond Entity's own below.
    pub defaults: Vec<(u8, i32, Value)>,
    /// EquipmentSlot ordinals (0 main hand, 1 off hand, 2 feet, 3 legs,
    /// 4 chest, 5 head, 6 body, 7 saddle) and their stacks.
    pub equipment: Vec<(u8, Option<Stack>)>,
    /// A living entity's client-syncable attribute instances
    /// (`AttributeMap.getSyncableAttributes`), by registry id.
    pub attributes: Vec<AttributeSnapshot>,
    /// `Entity.getRequiresPrecisePosition` (primed TNT and falling blocks
    /// touching a block): moves are sent as full position syncs.
    pub precise: bool,
    /// `Entity.needsSync`: send its changes this tick, due or not.
    pub needs_sync: bool,
    /// `Leashable.getLeashHolder`: the entity holding its lead.
    pub leash: Option<i32>,
    /// `ServerPlayer.isSpectator`: a player in spectator mode, whom only the
    /// clients of other spectators track (`ServerPlayer.broadcastToPlayer`).
    pub spectator: bool,
    /// `ServerPlayer.getCamera() != this`: a spectator watching through
    /// another entity is shown to nobody (`ServerPlayer.broadcastToPlayer`).
    pub watching: bool,
}

impl EntityState {
    /// The entity `id` of type `kind` as a fresh state has it: at the origin,
    /// at rest, with no data, equipment or attributes. Keeps the allocations
    /// of the strings and vectors it held.
    pub fn reset(&mut self, id: i32, uuid: u128, kind: &str) {
        self.id = id;
        self.uuid = uuid;
        self.kind.clear();
        self.kind.push_str(kind);
        self.position = [0.0; 3];
        (self.yaw, self.pitch, self.head_yaw) = (0.0, 0.0, 0.0);
        self.on_ground = false;
        self.velocity = [0.0; 3];
        self.data = 0;
        self.meta.clear();
        self.defaults.clear();
        self.equipment.clear();
        self.attributes.clear();
        self.precise = false;
        self.needs_sync = false;
        self.leash = None;
        self.spectator = false;
        self.watching = false;
    }
}

/// The states a level describes its entities with, kept from tick to tick:
/// each tick's are written over the last tick's in place, so the strings and
/// vectors in them are reused instead of being made and freed again for every
/// entity on every tick.
#[derive(Default)]
pub struct States {
    list: Vec<EntityState>,
    len: usize,
}

impl States {
    /// Start a new tick's states.
    pub fn clear(&mut self) {
        self.len = 0;
    }
    /// The next entity's state, reset (`EntityState::reset`).
    pub fn add(&mut self, id: i32, uuid: u128, kind: &str) -> &mut EntityState {
        if self.len == self.list.len() {
            self.list.push(EntityState::default());
        }
        let e = &mut self.list[self.len];
        self.len += 1;
        e.reset(id, uuid, kind);
        e
    }
    pub fn push(&mut self, e: EntityState) {
        match self.list.get_mut(self.len) {
            Some(slot) => *slot = e,
            None => self.list.push(e),
        }
        self.len += 1;
    }
    pub fn as_slice(&self) -> &[EntityState] {
        &self.list[..self.len]
    }
    pub fn as_mut_slice(&mut self) -> &mut [EntityState] {
        &mut self.list[..self.len]
    }
}

impl Extend<EntityState> for States {
    fn extend<I: IntoIterator<Item = EntityState>>(&mut self, states: I) {
        for e in states {
            self.push(e);
        }
    }
}

/// `AbstractArrow`: every update sends its position and rotation.
fn is_arrow(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident"
    )
}

/// `AbstractHurtingProjectile.accelerationPower` (0.1, none for wind
/// charges), sent with each velocity update.
fn acceleration_power(kind: &str) -> Option<f64> {
    match kind {
        "minecraft:fireball"
        | "minecraft:small_fireball"
        | "minecraft:dragon_fireball"
        | "minecraft:wither_skull" => Some(0.1),
        "minecraft:wind_charge" | "minecraft:breeze_wind_charge" => Some(0.0),
        _ => None,
    }
}

/// Who rides what this tick (`Entity.getPassengers`, `isPassenger`).
#[derive(Default)]
pub struct Riding {
    /// Each vehicle's passengers, in order.
    pub passengers: crate::fast_hash::FxHashMap<i32, Vec<i32>>,
    /// Each passenger's vehicle.
    pub vehicle: crate::fast_hash::FxHashMap<i32, i32>,
}

impl Riding {
    pub fn clear(&mut self) {
        self.passengers.clear();
        self.vehicle.clear();
    }
    /// `passenger` rides `vehicle`, after the ones already riding it.
    pub fn add(&mut self, vehicle: i32, passenger: i32) {
        self.passengers.entry(vehicle).or_default().push(passenger);
        self.vehicle.insert(passenger, vehicle);
    }
    pub fn passengers(&self, id: i32) -> &[i32] {
        self.passengers.get(&id).map_or(&[], Vec::as_slice)
    }
    fn set_passengers(&self, vehicle: i32) -> play::Clientbound {
        play::Clientbound::SetPassengers(SetPassengers {
            vehicle,
            passengers: self.passengers(vehicle).to_vec(),
        })
    }
}

/// What a level's clients were last told about one entity (ServerEntity's
/// fields), shared by every client that tracks it.
struct ServerEntity {
    /// What the mods hear of it when a client starts or stops tracking it.
    tracked: Arc<Tracked>,
    /// The registry id of its type (None: a type the clients don't know,
    /// never paired).
    kind: Option<i32>,
    /// The type's tracking range in chunks, update interval (0: never),
    /// whether velocity is tracked, whether it is an `AbstractArrow` and its
    /// `AbstractHurtingProjectile.accelerationPower`.
    range: i32,
    interval: u32,
    track_deltas: bool,
    arrow: bool,
    power: Option<f64>,
    /// VecDeltaCodec's base: the last position sent.
    base: [f64; 3],
    /// lastSentYRot / lastSentXRot / lastSentYHeadRot.
    rotation: (i8, i8),
    head: i8,
    /// lastSentMovement.
    velocity: [f64; 3],
    /// trackedDataValues: the entity data the clients hold.
    meta: Vec<(u8, i32, Value)>,
    equipment: Vec<(u8, Option<Stack>)>,
    /// The attribute instances the clients hold.
    attributes: Vec<AttributeSnapshot>,
    was_on_ground: bool,
    tick: u32,
    teleport_delay: u32,
    /// lastPassengers, and wasRiding: a passenger's position isn't synced,
    /// and the first update after it dismounts is a full position sync.
    passengers: Vec<i32>,
    was_riding: bool,
    /// The lead's holder the clients were told of.
    leash: Option<i32>,
    /// The level tick it was last seen in, its place in that tick's list, and
    /// whether it was added in it (its first update is the next tick's).
    seen: u32,
    index: usize,
    added: bool,
    /// TrackedEntity.lastSectionPos: its pairs are checked again when it
    /// moves to another section.
    section: [i32; 3],
    /// Whether it was a spectator last tick: its pairs are checked again when
    /// that changes, as `ServerPlayer.broadcastToPlayer` answers otherwise.
    spectator: bool,
    /// Likewise for a spectator who watches through a camera.
    watching: bool,
}

/// SectionPos.of(entity).
fn section_of(position: [f64; 3]) -> [i32; 3] {
    position.map(|v| (v.floor() as i32) >> 4)
}

/// VecDeltaCodec.encode.
fn encode(v: f64) -> i64 {
    (v * 4096.0).round() as i64
}

impl ServerEntity {
    /// `new ServerEntity`: the entity as it is when it joins the level.
    fn new(e: &EntityState, kind: Option<i32>, riding: &Riding) -> Self {
        let (range, interval, track_deltas) = tracking::settings(&e.kind);
        Self {
            tracked: Arc::new(Tracked {
                uuid: e.uuid,
                kind: Arc::from(e.kind.as_str()),
            }),
            kind,
            range: i32::from(range),
            interval,
            track_deltas,
            arrow: is_arrow(&e.kind),
            power: acceleration_power(&e.kind),
            base: e.position,
            rotation: (pack_degrees(e.pitch), pack_degrees(e.yaw)),
            head: pack_degrees(e.head_yaw),
            velocity: e.velocity,
            meta: e.meta.clone(),
            equipment: e.equipment.clone(),
            attributes: e.attributes.clone(),
            was_on_ground: e.on_ground,
            tick: 0,
            teleport_delay: 0,
            passengers: riding.passengers(e.id).to_vec(),
            was_riding: false,
            leash: e.leash,
            seen: 0,
            index: 0,
            added: true,
            section: section_of(e.position),
            spectator: e.spectator,
            watching: e.watching,
        }
    }

    /// ServerEntity.sendPairingData: the packets that introduce the entity
    /// to a client that starts tracking it.
    fn pairing(
        &self,
        e: &EntityState,
        kind: i32,
        riding: &Riding,
        out: &mut Vec<play::Clientbound>,
    ) {
        out.push(play::Clientbound::AddEntity(AddEntity {
            id: e.id,
            uuid: e.uuid,
            kind,
            position: e.position,
            velocity: e.velocity,
            pitch: pack_degrees(e.pitch),
            yaw: pack_degrees(e.yaw),
            head_yaw: pack_degrees(e.head_yaw),
            data: e.data,
        }));
        if !self.meta.is_empty() {
            out.push(play::Clientbound::SetEntityData(SetEntityData {
                id: e.id,
                values: self.meta.clone(),
            }));
        }
        // getSyncableAttributes: the instances as they are now.
        if !e.attributes.is_empty() {
            out.push(play::Clientbound::UpdateAttributes(UpdateAttributes {
                id: e.id,
                values: e.attributes.clone(),
            }));
        }
        if !self.equipment.is_empty() {
            out.push(play::Clientbound::SetEquipment(SetEquipment {
                id: e.id,
                slots: self.equipment.clone(),
            }));
        }
        if !riding.passengers(e.id).is_empty() {
            out.push(riding.set_passengers(e.id));
        }
        if let Some(&vehicle) = riding.vehicle.get(&e.id) {
            out.push(riding.set_passengers(vehicle));
        }
        // A leashed entity: who holds its lead.
        if let Some(dest) = e.leash {
            out.push(play::Clientbound::SetEntityLink(SetEntityLink {
                source: e.id,
                dest,
            }));
        }
    }

    /// Changes since the last update (ServerEntity.sendChanges): movement and
    /// rotation every update interval (positions at least every 60 ticks, a
    /// full position sync every 400 or when the entity lands or leaves the
    /// ground), velocity for types that track it, changed data at once.
    fn send_changes(&mut self, e: &EntityState, riding: &Riding, out: &mut Vec<play::Clientbound>) {
        let passengers = riding.passengers(e.id);
        if passengers != self.passengers.as_slice() {
            out.push(riding.set_passengers(e.id));
            self.passengers.clear();
            self.passengers.extend_from_slice(passengers);
        }
        // Leashable.setLeashedTo / dropLeash: sendToTrackingPlayers at once.
        if e.leash != self.leash {
            out.push(play::Clientbound::SetEntityLink(SetEntityLink {
                source: e.id,
                dest: e.leash.unwrap_or(0),
            }));
            self.leash = e.leash;
        }
        // SynchedEntityData.isDirty: once for every client.
        let dirty = e.meta != self.meta;
        let due = self.interval != 0 && self.tick.is_multiple_of(self.interval);
        if due || dirty || e.needs_sync {
            let rotation = (pack_degrees(e.pitch), pack_degrees(e.yaw));
            let turned = |a: i8, b: i8| (i32::from(a) - i32::from(b)).abs() >= 1;
            let rotate = self.arrow
                || turned(rotation.1, self.rotation.1)
                || turned(rotation.0, self.rotation.0);
            if riding.vehicle.contains_key(&e.id) {
                // A passenger: its rotation alone (its vehicle places it).
                if rotate {
                    out.push(play::Clientbound::MoveEntityRot(MoveEntityRot(
                        MoveEntity {
                            id: e.id,
                            delta: None,
                            rotation: Some((rotation.1, rotation.0)),
                            on_ground: e.on_ground,
                        },
                    )));
                    self.rotation = rotation;
                }
                self.base = e.position;
                self.was_riding = true;
                self.send_dirty(e, dirty, out);
                self.send_head(e, out);
                self.tick += 1;
                self.send_equipment(e, out);
                return;
            }
            self.teleport_delay += 1;
            let d: f64 = (0..3).map(|k| (e.position[k] - self.base[k]).powi(2)).sum();
            let send_position = self.arrow || d >= 7.629_394_5e-6 || self.tick.is_multiple_of(60);
            let sync = EntityPositionSync {
                id: e.id,
                path: PositionPath::Linear(e.position),
                yaw: e.yaw,
                pitch: e.pitch,
                on_ground: e.on_ground,
            };
            // ServerEntity.createMovePacket.
            let packet = if e.precise
                || self.teleport_delay > 400
                || self.was_riding
                || self.was_on_ground != e.on_ground
            {
                self.was_on_ground = e.on_ground;
                self.teleport_delay = 0;
                self.was_riding = false;
                Some(play::Clientbound::EntityPositionSync(sync))
            } else if send_position {
                let delta = [0, 1, 2].map(|k| encode(e.position[k]) - encode(self.base[k]));
                if delta.iter().any(|v| i16::try_from(*v).is_err()) {
                    Some(play::Clientbound::EntityPositionSync(sync))
                } else {
                    let m = MoveEntity {
                        id: e.id,
                        delta: Some(VecDelta::Linear(delta.map(|v| v as i16))),
                        rotation: rotate.then_some((rotation.1, rotation.0)),
                        on_ground: e.on_ground,
                    };
                    Some(if rotate {
                        play::Clientbound::MoveEntityPosRot(MoveEntityPosRot(m))
                    } else {
                        play::Clientbound::MoveEntityPos(MoveEntityPos(m))
                    })
                }
            } else if rotate {
                Some(play::Clientbound::MoveEntityRot(MoveEntityRot(
                    MoveEntity {
                        id: e.id,
                        delta: None,
                        rotation: Some((rotation.1, rotation.0)),
                        on_ground: e.on_ground,
                    },
                )))
            } else {
                None
            };
            if self.track_deltas || e.needs_sync {
                let diff: f64 = (0..3)
                    .map(|k| (e.velocity[k] - self.velocity[k]).powi(2))
                    .sum();
                let still = e.velocity.iter().all(|v| *v == 0.0);
                if diff > 1.0e-7 || diff > 0.0 && still {
                    self.velocity = e.velocity;
                    let motion = play::Clientbound::SetEntityMotion(SetEntityMotion {
                        id: e.id,
                        velocity: e.velocity,
                    });
                    match self.power {
                        // A bundle: the motion and the projectile's power.
                        Some(power) => out.extend([
                            play::Clientbound::BundleDelimiter(horizon_protocol::packets::Empty),
                            motion,
                            play::Clientbound::ProjectilePower(play::ProjectilePower {
                                id: e.id,
                                power,
                            }),
                            play::Clientbound::BundleDelimiter(horizon_protocol::packets::Empty),
                        ]),
                        None => out.push(motion),
                    }
                }
            }
            if let Some(packet) = packet {
                let (position, rotated) = match &packet {
                    play::Clientbound::EntityPositionSync(_)
                    | play::Clientbound::MoveEntityPosRot(_) => (true, true),
                    play::Clientbound::MoveEntityPos(_) => (true, false),
                    _ => (false, true),
                };
                out.push(packet);
                if position {
                    self.base = e.position;
                }
                if rotated {
                    self.rotation = rotation;
                }
            }
            self.send_dirty(e, dirty, out);
            self.send_head(e, out);
        }
        self.tick += 1;
        self.send_equipment(e, out);
    }

    /// ServerEntity.sendDirtyEntityData: the changed entity data, then the
    /// attribute instances that changed since they were last sent
    /// (`AttributeMap.getAttributesToSync`).
    fn send_dirty(&mut self, e: &EntityState, dirty: bool, out: &mut Vec<play::Clientbound>) {
        if dirty {
            let mut changed: Vec<_> = e
                .meta
                .iter()
                .filter(|m| !self.meta.contains(m))
                .cloned()
                .collect();
            // A value back at its default is left out of the state, but the
            // clients still hold the old one: Entity's shared flags and pose go
            // back to 0 (a player standing up again, a mob no longer on fire),
            // a sleeper's bed to none, an item frame's item to empty and its
            // rotation to 0 (ItemFrame.DATA_ITEM and DATA_ROTATION).
            let frame = e.kind.ends_with("item_frame");
            for (index, kind, _) in &self.meta {
                let default = match (*index, *kind) {
                    (0, serializer::BYTE) => Value::Byte(0),
                    (6, serializer::POSE) => Value::Id(0),
                    (14, serializer::OPTIONAL_BLOCK_POS) => Value::OptPos(None),
                    (9, serializer::ITEM_STACK) if frame => Value::Item(None),
                    (10, serializer::INT) if frame => Value::Int(0),
                    _ => match e.defaults.iter().find(|d| (d.0, d.1) == (*index, *kind)) {
                        Some(d) => d.2.clone(),
                        None => continue,
                    },
                };
                if !e.meta.iter().any(|m| m.0 == *index) {
                    changed.push((*index, *kind, default));
                }
            }
            if !changed.is_empty() {
                out.push(play::Clientbound::SetEntityData(SetEntityData {
                    id: e.id,
                    values: changed,
                }));
            }
            self.meta.clone_from(&e.meta);
        }
        if e.attributes != self.attributes {
            let changed = changed_attributes(&e.attributes, &self.attributes);
            if !changed.is_empty() {
                out.push(play::Clientbound::UpdateAttributes(UpdateAttributes {
                    id: e.id,
                    values: changed,
                }));
            }
            self.attributes.clone_from(&e.attributes);
        }
    }

    fn send_head(&mut self, e: &EntityState, out: &mut Vec<play::Clientbound>) {
        let head = pack_degrees(e.head_yaw);
        if (i32::from(head) - i32::from(self.head)).abs() >= 1 {
            out.push(play::Clientbound::RotateHead(RotateHead {
                id: e.id,
                head_yaw: head,
            }));
            self.head = head;
        }
    }

    /// LivingEntity.detectEquipmentUpdates: changed slots every tick.
    fn send_equipment(&mut self, e: &EntityState, out: &mut Vec<play::Clientbound>) {
        if e.equipment != self.equipment {
            // Cleared slots are sent as empty stacks.
            let mut slots: Vec<(u8, Option<Stack>)> = e
                .equipment
                .iter()
                .filter(|m| !self.equipment.contains(m))
                .cloned()
                .collect();
            for (slot, _) in &self.equipment {
                if !e.equipment.iter().any(|(k, _)| k == slot) {
                    slots.push((*slot, None));
                }
            }
            if !slots.is_empty() {
                out.push(play::Clientbound::SetEquipment(SetEquipment {
                    id: e.id,
                    slots,
                }));
            }
            self.equipment.clone_from(&e.equipment);
        }
    }
}

/// An entity a client tracks, as the mods hear of it.
#[derive(Debug)]
pub struct Tracked {
    pub uuid: u128,
    /// Its type's registry id (`minecraft:zombie`).
    pub kind: Arc<str>,
}

/// A client started or stopped tracking an entity (`ServerEntity.addPairing`
/// and `removePairing`).
#[derive(Debug)]
pub struct TrackEvent {
    pub start: bool,
    pub entity: i32,
    pub tracked: Arc<Tracked>,
}

/// The entities one client tracks (`TrackedEntity.seenBy`, from the
/// client's side), and what started and stopped since the game last took
/// the events.
#[derive(Default)]
pub struct Tracker {
    seen: crate::fast_hash::FxHashMap<i32, Arc<Tracked>>,
    events: Vec<TrackEvent>,
}

impl Tracker {
    pub fn is_tracking(&self, id: i32) -> bool {
        self.seen.contains_key(&id)
    }
    pub fn ids(&self) -> Vec<i32> {
        self.seen.keys().copied().collect()
    }
    /// Stop tracking everything without a packet: the caller removes them
    /// from the client.
    pub fn clear(&mut self) {
        let events = &mut self.events;
        events.extend(self.seen.drain().map(|(entity, tracked)| TrackEvent {
            start: false,
            entity,
            tracked,
        }));
    }
    /// Stop tracking `id` without a packet: the caller removes it from the
    /// client, and the next tick pairs it again if it is still in range.
    pub fn forget(&mut self, id: i32) -> bool {
        let Some(tracked) = self.seen.remove(&id) else {
            return false;
        };
        self.events.push(TrackEvent {
            start: false,
            entity: id,
            tracked,
        });
        true
    }
    pub fn len(&self) -> usize {
        self.seen.len()
    }
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
    /// The starts and stops since the last call, in order.
    pub fn take_events(&mut self) -> Vec<TrackEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Who a level's entities are tracked by this tick: a client's position,
/// view distance and chunks.
pub struct Viewer<'a> {
    pub entity_id: i32,
    /// Its player is in spectator mode: it tracks the other spectators too.
    pub spectator: bool,
    /// It accepted a move since the last tick (ChunkMap.move): every pair
    /// of it is checked again.
    pub moved: bool,
    pub position: [f64; 3],
    pub view_distance: i32,
    pub chunks: &'a crate::fast_hash::FxHashSet<(i32, i32)>,
    pub tracker: &'a mut Tracker,
    /// What the tick sends it, in order.
    pub out: &'a mut Vec<Vec<u8>>,
    /// Where the entities it stops tracking were last sent, when asked (the prediction's replay).
    pub gone: Option<&'a mut Vec<[f64; 3]>>,
}

/// One entity as this tick's range checks read it: where it is
/// horizontally, its id, its tracking range in blocks (scaled by
/// `entity-broadcast-range-percentage`) and the chunk it is in (an index
/// into the tick's chunks).
#[derive(Clone, Copy)]
struct Probe {
    x: f64,
    z: f64,
    id: i32,
    range: i32,
    chunk: u32,
    spectator: bool,
    watching: bool,
}

/// `ServerPlayer.broadcastToPlayer` (`Entity`'s is `true`): to a spectator,
/// a player is shown only while its camera is itself; to anyone else, a
/// spectator is hidden.
fn broadcast_to_player(viewer_spectator: bool, p: &Probe) -> bool {
    if viewer_spectator {
        !p.watching
    } else {
        !p.spectator
    }
}

/// TrackedEntity.updatePlayer: within the type's range (capped by the view
/// distance) horizontally, shown to the client (`Entity.broadcastToPlayer`:
/// `ServerPlayer`'s hides a spectator from players who are not spectators;
/// a guest's camera is always its own player, so a spectating client sees
/// every player) and in a chunk the client has. `held` remembers for
/// each of the tick's chunks whether the client has it (0: not looked up
/// yet, 1: no, 2: yes), so each chunk is looked up once per client.
fn sees(v: &Viewer, p: &Probe, chunks: &[(i32, i32)], held: &mut [u8]) -> bool {
    let range = f64::from(p.range.min(v.view_distance * 16));
    let (dx, dz) = (v.position[0] - p.x, v.position[2] - p.z);
    dx * dx + dz * dz <= range * range && broadcast_to_player(v.spectator, p) && {
        let held = &mut held[p.chunk as usize];
        if *held == 0 {
            *held = 1 + u8::from(v.chunks.contains(&chunks[p.chunk as usize]));
        }
        *held == 2
    }
}

/// One level's tracked entities (ChunkMap.entityMap).
#[derive(Default)]
pub struct Entities {
    /// `entity-broadcast-range-percentage` (None: 100), which scales every
    /// type's tracking range (MinecraftServer.getScaledTrackingDistance).
    pub range_percent: Option<i32>,
    states: crate::fast_hash::FxHashMap<i32, ServerEntity>,
    /// The level tick, stamped on every entity still in the level.
    tick: u32,
    /// This tick's entities as the range checks read them, the chunks they
    /// are in (and each one's index), whether the client being checked has
    /// each chunk, and what each client makes of each entity ([`SEES`],
    /// [`TRACKS`]), kept between ticks.
    probes: Vec<Probe>,
    chunks: Vec<(i32, i32)>,
    chunk_index: crate::fast_hash::FxHashMap<(i32, i32), u32>,
    /// The entities in each of `chunks` (indices into `probes`): a client
    /// that moved checks only the chunks within its view distance.
    chunk_members: Vec<Vec<u32>>,
    held: Vec<u8>,
    flags: Vec<Vec<u8>>,
    /// Whether a client starts tracking each entity this tick.
    pairs: Vec<bool>,
    /// The entities that appeared or moved to another section this tick
    /// (their pairs with every client are checked again), in order and as a
    /// flag per entity.
    moved: Vec<usize>,
    is_moved: Vec<bool>,
    /// One entity's packets and their bodies, kept between entities.
    packets: Vec<play::Clientbound>,
    bodies: Vec<Vec<u8>>,
}

/// The client sees the entity this tick, and already tracked it.
const SEES: u8 = 1;
const TRACKS: u8 = 2;

fn encode_all(packets: &[play::Clientbound], bodies: &mut Vec<Vec<u8>>) {
    bodies.clear();
    bodies.extend(packets.iter().map(play::Clientbound::encode));
}

impl Entities {
    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// One tick of ChunkMap.tick for the entities in the level now and the
    /// clients in it: each client first loses the entities that left its
    /// range or the level (one ClientboundRemoveEntitiesPacket), then each
    /// entity's changes are made once and go to every client tracking it,
    /// and the clients that start tracking it are paired with it. A client
    /// never tracks its own entity. `kind` names a type's registry id.
    pub fn tick(
        &mut self,
        entities: &[&EntityState],
        riding: &Riding,
        viewers: &mut [Viewer],
        mut kind: impl FnMut(&str) -> Option<i32>,
    ) {
        self.tick = self.tick.wrapping_add(1);
        let now = self.tick;
        self.probes.clear();
        self.chunks.clear();
        self.chunk_index.clear();
        for members in &mut self.chunk_members {
            members.clear();
        }
        self.moved.clear();
        self.is_moved.clear();
        self.is_moved.resize(entities.len(), false);
        for (index, e) in entities.iter().enumerate() {
            let mut added = false;
            let s = self.states.entry(e.id).or_insert_with(|| {
                added = true;
                ServerEntity::new(e, kind(&e.kind), riding)
            });
            s.seen = now;
            s.index = index;
            // ChunkMap.addEntity, ChunkMap.tick's section change, and a
            // player going in or out of spectator mode (which ChunkMap.move
            // catches at the player's next move, a second later at most):
            // updatePlayers.
            let section = section_of(e.position);
            if added
                || section != s.section
                || e.spectator != s.spectator
                || e.watching != s.watching
            {
                s.section = section;
                s.spectator = e.spectator;
                s.watching = e.watching;
                self.moved.push(index);
                self.is_moved[index] = true;
            }
            let at = (
                (e.position[0].floor() as i32) >> 4,
                (e.position[2].floor() as i32) >> 4,
            );
            let next = self.chunks.len() as u32;
            let chunk = *self.chunk_index.entry(at).or_insert(next);
            if chunk == next {
                self.chunks.push(at);
                if self.chunk_members.len() == chunk as usize {
                    self.chunk_members.push(Vec::new());
                }
            }
            self.chunk_members[chunk as usize].push(index as u32);
            self.probes.push(Probe {
                x: e.position[0],
                z: e.position[2],
                id: e.id,
                range: self
                    .range_percent
                    .map_or(s.range * 16, |p| p * (s.range * 16) / 100),
                chunk,
                spectator: e.spectator,
                watching: e.watching,
            });
        }
        // TrackedEntity.getEffectiveRange: a vehicle is tracked as far as its
        // widest indirect passenger, so a rider never leaves before its mount.
        for (&vehicle, passengers) in &riding.passengers {
            let Some(own) = self.states.get(&vehicle).filter(|s| s.seen == now) else {
                continue;
            };
            let (index, mut range) = (own.index, own.range);
            let mut pending = passengers.clone();
            while let Some(id) = pending.pop() {
                if let Some(s) = self.states.get(&id).filter(|s| s.seen == now) {
                    range = range.max(s.range);
                }
                pending.extend_from_slice(riding.passengers(id));
            }
            self.probes[index].range = self
                .range_percent
                .map_or(range * 16, |p| p * (range * 16) / 100);
        }
        self.flags.resize_with(viewers.len(), Vec::new);
        self.pairs.clear();
        self.pairs.resize(entities.len(), false);
        for (v, flags) in viewers.iter_mut().zip(&mut self.flags) {
            flags.clear();
            self.held.clear();
            self.held.resize(self.chunks.len(), 0);
            let (chunks, held, pairs) = (&self.chunks, &mut self.held, &mut self.pairs);
            let members = &self.chunk_members;
            let mut kept = 0;
            // A client that moved (ChunkMap.move), or whose own entity
            // appeared or changed section (ChunkMap.tick's movedPlayers), has
            // every pair checked again.
            let all = v.moved
                || self
                    .states
                    .get(&v.entity_id)
                    .is_none_or(|s| s.seen != now || self.is_moved[s.index]);
            if all {
                // `sees` caps the range at its view distance, so only the
                // entities in chunks that far from its own can be seen; the
                // rest stay 0, as checking each of them gave.
                flags.resize(self.probes.len(), 0);
                let reach = v.view_distance + 1;
                let (vx, vz) = (
                    (v.position[0].floor() as i32) >> 4,
                    (v.position[2].floor() as i32) >> 4,
                );
                for (&(cx, cz), members) in chunks.iter().zip(members) {
                    if (cx - vx).abs() > reach || (cz - vz).abs() > reach {
                        continue;
                    }
                    for &index in members {
                        let index = index as usize;
                        let p = &self.probes[index];
                        if p.id == v.entity_id || !sees(v, p, chunks, held) {
                            continue;
                        }
                        if v.tracker.seen.contains_key(&p.id) {
                            flags[index] = SEES | TRACKS;
                            kept += 1;
                        } else {
                            flags[index] = SEES;
                            pairs[index] = true;
                        }
                    }
                }
            } else if v.tracker.seen.len() * 4 > self.probes.len() {
                // Otherwise its pairs stay as they were, but for the entities
                // that appeared or changed section; a client tracking much
                // of the level goes through it in order.
                let is_moved = &self.is_moved;
                flags.extend(self.probes.iter().zip(pairs.iter_mut()).zip(is_moved).map(
                    |((p, pairs), &moved)| {
                        let tracks = v.tracker.seen.contains_key(&p.id);
                        if p.id == v.entity_id || (moved && !sees(v, p, chunks, held)) {
                            0
                        } else if tracks {
                            kept += 1;
                            SEES | TRACKS
                        } else if moved {
                            *pairs = true;
                            SEES
                        } else {
                            0
                        }
                    },
                ));
            } else {
                // One tracking little of it, from what it tracks.
                flags.resize(self.probes.len(), 0);
                for id in v.tracker.seen.keys() {
                    if let Some(s) = self.states.get(id).filter(|s| s.seen == now) {
                        flags[s.index] = SEES | TRACKS;
                        kept += 1;
                    }
                }
                for &index in &self.moved {
                    let p = &self.probes[index];
                    let tracks = flags[index] & TRACKS != 0;
                    if p.id != v.entity_id && sees(v, p, chunks, held) {
                        if !tracks {
                            flags[index] = SEES;
                            pairs[index] = true;
                        }
                    } else if tracks {
                        flags[index] = 0;
                        kept -= 1;
                    }
                }
            }
            // Usually the client still sees all it tracks.
            let mut removed = Vec::new();
            if kept != v.tracker.seen.len() {
                let states = &self.states;
                let Tracker { seen, events } = &mut *v.tracker;
                seen.retain(|id, tracked| match states.get(id) {
                    Some(s) if s.seen == now && flags[s.index] & TRACKS != 0 => true,
                    other => {
                        if let (Some(gone), Some(s)) = (v.gone.as_deref_mut(), other) {
                            gone.push(s.base);
                        }
                        removed.push(*id);
                        events.push(TrackEvent {
                            start: false,
                            entity: *id,
                            tracked: Arc::clone(tracked),
                        });
                        false
                    }
                });
            }
            if !removed.is_empty() {
                v.out.push(
                    play::Clientbound::RemoveEntities(play::RemoveEntities { ids: removed })
                        .encode(),
                );
            }
        }
        for (index, e) in entities.iter().enumerate() {
            let Some(s) = self.states.get_mut(&e.id) else {
                continue;
            };
            self.packets.clear();
            if !s.added {
                s.send_changes(e, riding, &mut self.packets);
            }
            s.added = false;
            // Nothing to send: no changes, and nobody starts tracking it.
            if self.packets.is_empty() && !self.pairs[index] {
                continue;
            }
            let mut encoded = false;
            let mut pairing: Option<Vec<Vec<u8>>> = None;
            for (v, flags) in viewers.iter_mut().zip(&self.flags) {
                let f = flags[index];
                if f & TRACKS != 0 {
                    if self.packets.is_empty() {
                        continue;
                    }
                    if !encoded {
                        encode_all(&self.packets, &mut self.bodies);
                        encoded = true;
                    }
                    v.out.extend(self.bodies.iter().cloned());
                } else if f & SEES != 0
                    && let Some(kind) = s.kind
                {
                    let bodies = pairing.get_or_insert_with(|| {
                        let mut packets = Vec::new();
                        s.pairing(e, kind, riding, &mut packets);
                        packets.iter().map(play::Clientbound::encode).collect()
                    });
                    v.out.extend(bodies.iter().cloned());
                    if v.tracker
                        .seen
                        .insert(e.id, Arc::clone(&s.tracked))
                        .is_none()
                    {
                        v.tracker.events.push(TrackEvent {
                            start: true,
                            entity: e.id,
                            tracked: Arc::clone(&s.tracked),
                        });
                    }
                }
            }
        }
        self.states.retain(|_, s| s.seen == now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f64) -> EntityState {
        EntityState {
            id: 7,
            uuid: 1,
            kind: "minecraft:zombie".into(),
            position: [x, 64.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: true,
            velocity: [0.0; 3],
            data: 0,
            meta: Vec::new(),
            defaults: Vec::new(),
            equipment: Vec::new(),
            attributes: Vec::new(),
            precise: false,
            needs_sync: false,
            leash: None,
            spectator: false,
            watching: false,
        }
    }

    #[test]
    fn states_are_written_over_last_ticks_in_place() {
        let mut states = States::default();
        let e = states.add(1, 1, "minecraft:zombie");
        e.position = [1.0, 2.0, 3.0];
        e.meta.push((9, serializer::FLOAT, Value::Float(20.0)));
        e.equipment.push((0, None));
        let (kind, meta) = (e.kind.as_ptr(), e.meta.as_ptr());
        states.push(state(2.0));
        states.clear();
        let e = states.add(3, 3, "minecraft:husk");
        // Nothing of the last tick's state is left, but its allocations are.
        let fresh = EntityState {
            id: 3,
            uuid: 3,
            kind: "minecraft:husk".into(),
            ..EntityState::default()
        };
        assert_eq!(*e, fresh);
        assert_eq!((e.kind.as_ptr(), e.meta.as_ptr()), (kind, meta));
        assert_eq!(states.as_slice(), std::slice::from_ref(&fresh));
        states.extend([state(5.0), state(6.0)]);
        assert_eq!(states.as_slice(), [fresh, state(5.0), state(6.0)]);
    }

    /// The entity as added this tick, then its updates (one a tick).
    fn added(e: &EntityState) -> ServerEntity {
        ServerEntity::new(e, Some(100), &Riding::default())
    }
    fn changes(s: &mut ServerEntity, e: &EntityState) -> Vec<play::Clientbound> {
        let mut out = Vec::new();
        s.send_changes(e, &Riding::default(), &mut out);
        out
    }

    #[test]
    fn furnace_cart_sends_unpowered_metadata_when_fuel_runs_out() {
        let mut powered = state(0.0);
        powered.kind = "minecraft:furnace_minecart".into();
        powered
            .defaults
            .push((13, serializer::BOOLEAN, Value::Bool(false)));
        powered
            .meta
            .push((13, serializer::BOOLEAN, Value::Bool(true)));
        let mut tracker = added(&powered);
        let mut empty = powered.clone();
        empty.meta.clear();
        let mut packets = Vec::new();
        tracker.send_dirty(&empty, true, &mut packets);
        assert!(packets.iter().any(|packet| matches!(packet,
            play::Clientbound::SetEntityData(data)
                if data.values.contains(&(13, serializer::BOOLEAN, Value::Bool(false)))
        )));
    }

    #[test]
    fn moves_go_out_every_update_interval() {
        let mut s = added(&state(0.0));
        let mut spawn = Vec::new();
        s.pairing(&state(0.0), 100, &Riding::default(), &mut spawn);
        assert!(matches!(spawn[..], [play::Clientbound::AddEntity(_)]));
        // Tick 0 is due and a multiple of 60: the position goes out unchanged.
        assert!(matches!(
            changes(&mut s, &state(0.0))[..],
            [play::Clientbound::MoveEntityPos(_)]
        ));
        // A zombie updates every third tick.
        assert!(changes(&mut s, &state(0.5)).is_empty());
        assert!(changes(&mut s, &state(0.5)).is_empty());
        assert!(matches!(
            changes(&mut s, &state(0.5))[..],
            [play::Clientbound::MoveEntityPos(_)]
        ));
        for _ in 0..2 {
            changes(&mut s, &state(40.0));
        }
        let jump = changes(&mut s, &state(40.0));
        assert!(matches!(jump[0], play::Clientbound::EntityPositionSync(_)));
    }

    #[test]
    fn a_knocked_back_mob_is_sent_at_once() {
        let mut s = added(&state(0.0));
        changes(&mut s, &state(0.0));
        // Not due (tick 1 of 3), but LivingEntity.knockback set needsSync:
        // its move and its new motion go out now.
        let mut knocked = state(0.3);
        knocked.velocity = [0.2, 0.3, 0.0];
        knocked.needs_sync = true;
        let out = changes(&mut s, &knocked);
        assert!(
            matches!(
                out[..],
                [
                    play::Clientbound::SetEntityMotion(_),
                    play::Clientbound::MoveEntityPos(_)
                ]
            ),
            "{out:?}"
        );
        // Then every third tick again.
        assert!(changes(&mut s, &state(0.5)).is_empty());
        assert!(matches!(
            changes(&mut s, &state(0.6))[..],
            [
                play::Clientbound::SetEntityMotion(_),
                play::Clientbound::MoveEntityPos(_)
            ]
        ));
    }

    #[test]
    fn fireballs_bundle_their_motion_with_their_power() {
        let mut e = state(0.0);
        e.kind = "minecraft:fireball".into();
        e.on_ground = false;
        let mut s = added(&e);
        e.velocity = [0.0, 0.0, 0.5];
        let out = changes(&mut s, &e);
        assert!(matches!(
            out[..],
            [
                play::Clientbound::BundleDelimiter(_),
                play::Clientbound::SetEntityMotion(_),
                play::Clientbound::ProjectilePower(play::ProjectilePower { power, .. }),
                play::Clientbound::BundleDelimiter(_),
                play::Clientbound::MoveEntityPos(_),
            ] if power == 0.1
        ));
    }

    #[test]
    fn arrows_send_position_and_rotation_every_update() {
        let mut e = state(0.0);
        e.kind = "minecraft:arrow".into();
        let mut s = added(&e);
        assert!(matches!(
            changes(&mut s, &e)[..],
            [play::Clientbound::MoveEntityPosRot(_)]
        ));
    }

    #[test]
    fn precise_entities_sync_their_position() {
        let mut e = state(0.0);
        e.kind = "minecraft:tnt".into();
        e.precise = true;
        let mut s = added(&e);
        assert!(matches!(
            changes(&mut s, &e)[..],
            [play::Clientbound::EntityPositionSync(_)]
        ));
    }

    #[test]
    fn landing_sends_a_position_sync() {
        let mut e = state(0.0);
        let mut s = added(&e);
        changes(&mut s, &e);
        e.on_ground = false;
        changes(&mut s, &e);
        changes(&mut s, &e);
        assert!(matches!(
            changes(&mut s, &e)[..],
            [play::Clientbound::EntityPositionSync(_)]
        ));
    }

    #[test]
    fn attributes_pair_whole_then_send_the_changed_instances_when_due() {
        let speed = |base| AttributeSnapshot {
            attribute: 26,
            base,
            modifiers: Vec::new(),
        };
        let armor = AttributeSnapshot {
            attribute: 1,
            base: 0.0,
            modifiers: Vec::new(),
        };
        let mut e = state(0.0);
        e.attributes = vec![speed(0.23), armor];
        let mut s = added(&e);
        let mut spawn = Vec::new();
        s.pairing(&e, 100, &Riding::default(), &mut spawn);
        let Some(play::Clientbound::UpdateAttributes(p)) = spawn.get(1) else {
            panic!("expected the attributes after the add");
        };
        assert_eq!(p.values, e.attributes);
        changes(&mut s, &e);
        e.attributes[0] = speed(0.3);
        // Not due (a zombie's interval is 3): sendDirtyEntityData waits for the update.
        assert!(changes(&mut s, &e).is_empty());
        assert!(changes(&mut s, &e).is_empty());
        let out = changes(&mut s, &e);
        let Some(play::Clientbound::UpdateAttributes(p)) = out
            .iter()
            .find(|p| matches!(p, play::Clientbound::UpdateAttributes(_)))
        else {
            panic!("expected the changed attribute");
        };
        assert_eq!(p.values, vec![speed(0.3)]);
    }

    #[test]
    fn cleared_flags_and_pose_go_back_to_their_default() {
        let mut e = state(0.0);
        let mut s = added(&e);
        changes(&mut s, &e);
        // A crouching player: the crouch flag and the crouching pose.
        e.meta = vec![
            (0, serializer::BYTE, Value::Byte(2)),
            (6, serializer::POSE, Value::Id(5)),
        ];
        changes(&mut s, &e);
        e.meta.clear();
        let data = changes(&mut s, &e)
            .into_iter()
            .find_map(|p| match p {
                play::Clientbound::SetEntityData(d) => Some(d),
                _ => None,
            })
            .expect("standing up is sent");
        assert_eq!(
            data.values,
            vec![
                (0, serializer::BYTE, Value::Byte(0)),
                (6, serializer::POSE, Value::Id(0)),
            ]
        );
        assert!(changes(&mut s, &e).is_empty());
    }

    #[test]
    fn an_emptied_item_frame_sends_its_empty_item_and_rotation() {
        let mut e = state(0.0);
        e.kind = "minecraft:item_frame".into();
        let mut s = added(&e);
        changes(&mut s, &e);
        e.meta = vec![
            (
                9,
                serializer::ITEM_STACK,
                Value::Item(Some(Stack::new(5, 1))),
            ),
            (10, serializer::INT, Value::Int(3)),
        ];
        changes(&mut s, &e);
        // Punched out (ItemFrame.removeFramedMap, setItem(EMPTY)): both back to the default.
        e.meta.clear();
        let data = changes(&mut s, &e)
            .into_iter()
            .find_map(|p| match p {
                play::Clientbound::SetEntityData(d) => Some(d),
                _ => None,
            })
            .expect("the empty frame is sent");
        assert_eq!(
            data.values,
            vec![
                (9, serializer::ITEM_STACK, Value::Item(None)),
                (10, serializer::INT, Value::Int(0)),
            ]
        );
    }

    #[test]
    fn a_cleared_crystal_beam_is_sent() {
        let mut e = state(0.0);
        e.kind = "minecraft:end_crystal".into();
        e.defaults = vec![(8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None))];
        e.meta = vec![(
            8,
            serializer::OPTIONAL_BLOCK_POS,
            Value::OptPos(Some([0, 128, 0])),
        )];
        let mut s = added(&e);
        e.meta.clear();
        let mut packets = Vec::new();
        s.send_dirty(&e, true, &mut packets);
        let sent: Vec<_> = packets
            .iter()
            .filter_map(|p| match p {
                play::Clientbound::SetEntityData(d) => Some(d.values.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            sent,
            vec![(8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None))]
        );
    }

    #[test]
    fn a_value_back_at_a_listed_default_is_sent() {
        let mut e = state(0.0);
        e.defaults = vec![(16, serializer::BOOLEAN, Value::Bool(false))];
        // A calf: paired with its baby flag, then grown up.
        e.meta = vec![(16, serializer::BOOLEAN, Value::Bool(true))];
        let mut s = added(&e);
        changes(&mut s, &e);
        e.meta.clear();
        let data = changes(&mut s, &e)
            .into_iter()
            .find_map(|p| match p {
                play::Clientbound::SetEntityData(d) => Some(d),
                _ => None,
            })
            .expect("growing up is sent");
        assert_eq!(
            data.values,
            vec![(16, serializer::BOOLEAN, Value::Bool(false))]
        );
        assert!(changes(&mut s, &e).is_empty());
    }

    #[test]
    fn metadata_and_equipment_send_only_the_changes() {
        let mut e = state(0.0);
        let mut s = added(&e);
        changes(&mut s, &e);
        e.meta.push((0, 0, Value::Byte(1)));
        e.equipment.push((0, Some(Stack::new(5, 1))));
        let out = changes(&mut s, &e);
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEntityData(_)))
        );
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEquipment(_)))
        );
        assert!(changes(&mut s, &e).is_empty());
        e.equipment.clear();
        let out = changes(&mut s, &e);
        let Some(play::Clientbound::SetEquipment(eq)) = out.first() else {
            panic!("expected equipment");
        };
        assert_eq!(eq.slots, vec![(0, None)]);
    }

    #[test]
    fn a_lead_tied_and_untied_reaches_the_clients() {
        let link = |out: &[play::Clientbound]| {
            out.iter().find_map(|p| match p {
                play::Clientbound::SetEntityLink(l) => Some((l.source, l.dest)),
                _ => None,
            })
        };
        let mut e = state(0.0);
        let mut s = added(&e);
        changes(&mut s, &e);
        e.leash = Some(7);
        assert_eq!(link(&changes(&mut s, &e)), Some((e.id, 7)));
        assert_eq!(link(&changes(&mut s, &e)), None);
        e.leash = None;
        assert_eq!(link(&changes(&mut s, &e)), Some((e.id, 0)));
    }

    /// Two clients at the origin; the chunks they hold.
    struct Clients {
        chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
        trackers: [Tracker; 2],
        out: [Vec<Vec<u8>>; 2],
        positions: [[f64; 3]; 2],
        moved: [bool; 2],
        spectators: [bool; 2],
        /// The first client's entities gone, where they were last sent.
        gone: Vec<[f64; 3]>,
    }

    impl Clients {
        fn new() -> Self {
            Self {
                chunks: (-4..4).flat_map(|x| (-4..4).map(move |z| (x, z))).collect(),
                trackers: Default::default(),
                out: Default::default(),
                positions: [[0.0, 64.0, 0.0]; 2],
                moved: [false; 2],
                spectators: [false; 2],
                gone: Vec::new(),
            }
        }
        fn tick(&mut self, level: &mut Entities, entities: &[&EntityState]) {
            for out in &mut self.out {
                out.clear();
            }
            let [t0, t1] = &mut self.trackers;
            let [o0, o1] = &mut self.out;
            let mut viewers = [
                Viewer {
                    entity_id: 1,
                    spectator: self.spectators[0],
                    moved: self.moved[0],
                    position: self.positions[0],
                    view_distance: 10,
                    chunks: &self.chunks,
                    tracker: t0,
                    out: o0,
                    gone: Some(&mut self.gone),
                },
                Viewer {
                    entity_id: 2,
                    spectator: self.spectators[1],
                    moved: self.moved[1],
                    position: self.positions[1],
                    view_distance: 10,
                    chunks: &self.chunks,
                    tracker: t1,
                    out: o1,
                    gone: None,
                },
            ];
            level.tick(entities, &Riding::default(), &mut viewers, |_| Some(100));
        }
    }

    fn body(p: play::Clientbound) -> Vec<u8> {
        p.encode()
    }

    /// ChunkMap checks a pair again only when one side moves: a zombie
    /// walking into range within its section isn't paired with a client
    /// standing still until the client moves, and one walking out stays
    /// tracked until it changes section.
    #[test]
    fn pairs_are_checked_again_when_a_side_moves() {
        let mut level = Entities::default();
        let mut clients = Clients::new();
        clients.positions[0] = [-108.0, 64.0, 0.0];
        let mut me = state(-108.0);
        (me.id, me.kind) = (1, "minecraft:player".into());
        // 128.5 blocks away: past a zombie's 8 chunks.
        let tracks = |clients: &Clients| clients.trackers[0].is_tracking(7);
        clients.tick(&mut level, &[&me, &state(20.5)]);
        assert!(!tracks(&clients));
        // 127.5 blocks away, in the same section: not checked again.
        clients.tick(&mut level, &[&me, &state(19.5)]);
        assert!(!tracks(&clients));
        // The client's move checks every pair of it.
        clients.moved[0] = true;
        clients.tick(&mut level, &[&me, &state(19.5)]);
        assert!(tracks(&clients));
        clients.moved[0] = false;
        clients.tick(&mut level, &[&me, &state(20.5)]);
        assert!(tracks(&clients));
        // Into the next section, out of range: no longer tracked.
        clients.tick(&mut level, &[&me, &state(40.0)]);
        assert!(!tracks(&clients));
        // The other client (at the origin, its entity not in the level)
        // checks every pair each tick.
        assert!(clients.trackers[1].is_tracking(7));
    }

    #[test]
    fn every_client_gets_the_same_changes_and_late_ones_are_paired() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        // The second client is out of range at first.
        c.positions[1] = [500.0, 64.0, 0.0];
        let mut e = state(0.0);
        c.tick(&mut level, &[&e]);
        assert!(c.trackers[0].is_tracking(7) && !c.trackers[1].is_tracking(7));
        assert!(c.out[1].is_empty());
        // Tick 0's forced position goes to the client tracking it.
        c.tick(&mut level, &[&e]);
        assert_eq!(c.out[0].len(), 1);
        // The second client comes in range: paired with the entity as it is,
        // then sent the same changes as the first.
        c.positions[1] = [0.0, 64.0, 0.0];
        e.position[0] = 1.0;
        c.tick(&mut level, &[&e]);
        assert!(c.trackers[1].is_tracking(7));
        assert!(c.out[0].is_empty());
        assert_eq!(c.out[1].len(), 1);
        c.tick(&mut level, &[&e]);
        e.position[0] = 2.0;
        c.tick(&mut level, &[&e]);
        assert!(!c.out[0].is_empty());
        assert_eq!(c.out[0], c.out[1]);
    }

    #[test]
    fn clients_lose_what_leaves_range_or_the_level_and_never_track_themselves() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        let mut me = state(0.0);
        me.id = 1;
        me.kind = "minecraft:player".into();
        let mut e = state(0.0);
        c.tick(&mut level, &[&me, &e]);
        assert!(!c.trackers[0].is_tracking(1) && c.trackers[1].is_tracking(1));
        assert!(c.trackers[0].is_tracking(7));
        // Out of the zombie's 8 chunks: the client last had it where it started.
        let start = e.position;
        e.position[0] = 200.0;
        c.tick(&mut level, &[&me, &e]);
        assert!(!c.trackers[0].is_tracking(7));
        assert_eq!(c.gone, [start]);
        let removed = body(play::Clientbound::RemoveEntities(play::RemoveEntities {
            ids: vec![7],
        }));
        assert_eq!(c.out[0].first(), Some(&removed));
        // Gone from the level: the other client loses it too, and so does
        // the level.
        e.position[0] = 0.0;
        c.tick(&mut level, &[&me, &e]);
        assert!(c.trackers[0].is_tracking(7));
        c.tick(&mut level, &[&me]);
        assert!(!c.trackers[0].is_tracking(7) && !c.trackers[1].is_tracking(7));
        assert_eq!(c.out[0].first(), Some(&removed));
        assert_eq!(level.len(), 1);
    }

    #[test]
    fn entities_in_chunks_a_client_lacks_wait_for_them() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        // Two zombies in range in chunk (4, 0), which the clients don't have
        // yet, and one in chunk (0, 0).
        let mut far = state(70.0);
        let mut near = state(66.0);
        near.id = 8;
        let mut home = state(1.0);
        home.id = 9;
        c.tick(&mut level, &[&far, &near, &home]);
        for t in &c.trackers {
            assert!(!t.is_tracking(7) && !t.is_tracking(8) && t.is_tracking(9));
        }
        // The chunk arrives: both are paired, in order.
        c.chunks.insert((4, 0));
        c.tick(&mut level, &[&far, &near, &home]);
        for t in &c.trackers {
            assert!(t.is_tracking(7) && t.is_tracking(8) && t.is_tracking(9));
        }
        // One walks back into chunk (3, 0) and the other out of range.
        far.position[0] = 200.0;
        near.position[0] = 60.0;
        c.tick(&mut level, &[&far, &near, &home]);
        for t in &c.trackers {
            assert!(!t.is_tracking(7) && t.is_tracking(8) && t.is_tracking(9));
        }
    }

    /// ServerPlayer.broadcastToPlayer: a spectator is shown only to other
    /// spectators, who see everyone. Going in or out of spectator mode checks
    /// its pairs again, though nobody moved.
    #[test]
    fn spectators_are_tracked_only_by_spectators() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        let player = |id: i32| {
            let mut p = state(0.0);
            (p.id, p.uuid, p.kind) = (id, id as u128, "minecraft:player".into());
            p
        };
        let (mut one, mut two) = (player(1), player(2));
        // A third player, already spectating, that neither client plays.
        let mut three = player(3);
        three.spectator = true;
        c.tick(&mut level, &[&one, &two, &three]);
        c.tick(&mut level, &[&one, &two, &three]);
        assert!(c.trackers[0].is_tracking(2) && c.trackers[1].is_tracking(1));
        assert!(!c.trackers[0].is_tracking(3) && !c.trackers[1].is_tracking(3));
        // The second player starts spectating: the first client loses it,
        // and it sees the first player and the other spectator.
        (two.spectator, c.spectators[1]) = (true, true);
        c.tick(&mut level, &[&one, &two, &three]);
        assert!(!c.trackers[0].is_tracking(2));
        let removed = body(play::Clientbound::RemoveEntities(play::RemoveEntities {
            ids: vec![2],
        }));
        assert_eq!(c.out[0].first(), Some(&removed));
        assert!(c.trackers[1].is_tracking(1) && c.trackers[1].is_tracking(3));
        // Its armour changes go to nobody who doesn't track it.
        two.equipment.push((5, Some(Stack::new(5, 1))));
        c.tick(&mut level, &[&one, &two, &three]);
        assert!(c.out[0].is_empty());
        // The first player starts spectating too: both spectators show up.
        (one.spectator, c.spectators[0]) = (true, true);
        c.tick(&mut level, &[&one, &two, &three]);
        assert!(c.trackers[0].is_tracking(2) && c.trackers[0].is_tracking(3));
        // Both back in survival: they still see each other, and lose the
        // third player.
        (one.spectator, c.spectators[0]) = (false, false);
        (two.spectator, c.spectators[1]) = (false, false);
        c.tick(&mut level, &[&one, &two, &three]);
        assert!(c.trackers[0].is_tracking(2) && c.trackers[1].is_tracking(1));
        assert!(!c.trackers[0].is_tracking(3) && !c.trackers[1].is_tracking(3));
    }

    /// ServerPlayer.broadcastToPlayer for a spectator: a spectator watching
    /// through a camera is hidden from the other spectators too.
    #[test]
    fn a_spectator_with_a_camera_is_hidden_from_spectators() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        let player = |id: i32| {
            let mut p = state(0.0);
            (p.id, p.uuid, p.kind) = (id, id as u128, "minecraft:player".into());
            p
        };
        let (mut one, mut two) = (player(1), player(2));
        (one.spectator, two.spectator) = (true, true);
        c.spectators = [true, true];
        c.tick(&mut level, &[&one, &two]);
        c.tick(&mut level, &[&one, &two]);
        assert!(c.trackers[0].is_tracking(2) && c.trackers[1].is_tracking(1));
        // The second spectator takes a camera: the first loses sight of it.
        two.watching = true;
        c.tick(&mut level, &[&one, &two]);
        assert!(!c.trackers[0].is_tracking(2) && c.trackers[1].is_tracking(1));
        // Back to its own view: it shows up again.
        two.watching = false;
        c.tick(&mut level, &[&one, &two]);
        assert!(c.trackers[0].is_tracking(2));
    }

    #[test]
    fn pairing_and_unpairing_are_kept_for_the_game() {
        let mut level = Entities::default();
        let mut c = Clients::new();
        let zombie = state(0.0);
        c.tick(&mut level, &[&zombie]);
        c.tick(&mut level, &[&zombie]);
        // Both clients were paired with it, once.
        for t in &mut c.trackers {
            let events = t.take_events();
            assert_eq!(events.len(), 1, "{events:?}");
            assert!(events[0].start && events[0].entity == 7);
            assert_eq!(&*events[0].tracked.kind, "minecraft:zombie");
            assert_eq!(events[0].tracked.uuid, 1);
            assert!(t.take_events().is_empty());
        }
        // It leaves the level: both clients lose it.
        c.tick(&mut level, &[]);
        for t in &mut c.trackers {
            let events = t.take_events();
            assert!(
                matches!(&events[..], [e] if !e.start && e.entity == 7),
                "{events:?}"
            );
        }
        // Back, then forgotten and cleared without a tick.
        c.tick(&mut level, &[&zombie]);
        c.tick(&mut level, &[&zombie]);
        c.trackers[0].take_events();
        c.trackers[1].take_events();
        assert!(c.trackers[0].forget(7));
        c.trackers[1].clear();
        for t in &mut c.trackers {
            let events = t.take_events();
            assert!(
                matches!(&events[..], [e] if !e.start && e.entity == 7),
                "{events:?}"
            );
        }
    }

    #[test]
    fn a_forgotten_entity_is_paired_again_at_the_next_tick() {
        // A respawned player: the hub removes it from the other clients and
        // forgets it, and the next tick pairs it again as a new entity.
        let mut level = Entities::default();
        let mut c = Clients::new();
        let mut player = state(0.0);
        player.id = 1;
        player.kind = "minecraft:player".into();
        c.tick(&mut level, &[&player]);
        c.tick(&mut level, &[&player]);
        assert!(c.trackers[1].forget(1));
        assert!(!c.trackers[1].forget(1));
        c.tick(&mut level, &[&player]);
        assert!(c.trackers[1].is_tracking(1));
        let paired = c.out[1]
            .first()
            .and_then(|p| play::Clientbound::decode(i32::from(p[0]), &p[1..]).ok());
        assert!(
            matches!(paired, Some(play::Clientbound::AddEntity(ref a)) if a.id == 1),
            "{paired:?}"
        );
    }
}
