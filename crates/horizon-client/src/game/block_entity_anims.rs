//! The client side of animated block entities (`horizon_client::block_entity_anim`):
//! their client tickers (`Level.tickBlockEntities` on the client level) and
//! the block events that drive them (`ClientboundBlockEventPacket` reaching
//! `BlockEntity.triggerEvent`). The renderer reads the resulting state through
//! [`Game::block_anim`] and [`Game::chest_lid`].
use super::Game;
use horizon_client::{
    block_entity_anim::{
        BellEntity, ConduitServer, ConduitTarget, EndGateway, Shulker, ShulkerStatus, Ticker,
        closer_to_center_than,
    },
    block_entity_render::BlockAnim,
    collisions::{SceneCollisions, no_collision_empty},
    containers::BlockEntity,
    effects::EffectInstance,
    mobs::{Damage, DamageKind},
    particles::{ParticleOptions, ParticleType},
    save::number,
    skins,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::nbt::Tag;

/// The handle of the local player among a bell's gathered entities.
const PLAYER: u64 = u64::MAX;

impl Game<'_> {
    /// `BlockEntity.triggerEvent(a, b)` on the client copy of the block entity at `pos`.
    pub(super) fn block_entity_event(&mut self, pos: [i32; 3], a: i32, b: i32) {
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        let time = self.world.game_time;
        // PotentSulfurBlock.triggerEvent.
        if self.potent_sulfur_eruption_event((pos[0], pos[1], pos[2])) {
            return;
        }
        match self.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                self.be_anims
                    .chests
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b);
            }
            Some(Ticker::Shulker { .. }) => {
                self.be_anims
                    .shulkers
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b);
            }
            Some(Ticker::Bell) => {
                let gathered = self.bell_gather(pos);
                let rung =
                    self.be_anims
                        .bells
                        .entry(pos)
                        .or_default()
                        .trigger_event(a, b, time, || gathered);
                // The server's copy tells the living entities within 32 blocks.
                if rung && self.remote.is_none() {
                    self.bell_heard(pos, time);
                }
            }
            Some(Ticker::Gateway) => {
                let initial = self.gateway_loaded(pos);
                self.be_anims
                    .gateways
                    .entry(pos)
                    .or_insert(initial)
                    .trigger_event(a, b);
            }
            Some(Ticker::Pot) => {
                self.be_anims
                    .pots
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time);
            }
            _ => {}
        }
    }
    /// A player opened (`open`) or closed the container at `key`: the opener
    /// count block event (`ContainerOpenersCounter` / `ShulkerBoxBlockEntity`
    /// `startOpen`/`stopOpen`) for chests, ender chests and shulker boxes,
    /// on both halves of a double chest (`CompoundContainer`). True when the
    /// count went from 0 to 1 or back to 0: the only moments the container
    /// plays its open or close sound.
    pub(super) fn container_opener_event(&mut self, key: (i32, i32, i32), open: bool) -> bool {
        // Spectators never count as openers.
        !self.spectator() && self.container_openers_changed(key, open)
    }
    /// `container_opener_event` for a player who is not a spectator (a guest).
    pub(super) fn container_openers_changed(&mut self, key: (i32, i32, i32), open: bool) -> bool {
        let pos = [key.0, key.1, key.2];
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        match self.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                let edge = self.lid_openers_changed(key, open);
                if let Some(other) = self.chest_other_half(key) {
                    self.lid_openers_changed(other, open);
                }
                edge
            }
            _ if self
                .res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]))
                .name
                .as_ref()
                == "minecraft:barrel" =>
            {
                self.barrel_openers_changed(key, open)
            }
            Some(Ticker::Shulker { .. }) => {
                let shulker = self.be_anims.shulkers.entry(pos).or_default();
                let (count, edge) = if open {
                    shulker.start_open()
                } else {
                    shulker.stop_open()
                };
                self.block_entity_event(pos, 1, count);
                self.lan_block_event(pos, 1, count);
                edge
            }
            _ => false,
        }
    }
    /// `ContainerOpenersCounter.incrementOpeners` / `decrementOpeners` of the
    /// chest at `key` (the host and the guests are counted), then
    /// `ChestBlockEntity.signalOpenCount`: block event 1 with the count, here
    /// and for the guests. True when the count crossed zero.
    fn lid_openers_changed(&mut self, key: (i32, i32, i32), open: bool) -> bool {
        let (before, count) = self.count_opener(key, open);
        let pos = [key.0, key.1, key.2];
        self.block_entity_event(pos, 1, count);
        self.lan_block_event(pos, 1, count);
        // TrappedChestBlockEntity.signalOpenCount: its signal is the count
        // (`TrappedChestBlock.getSignal`), so the blocks around it and
        // below it hear of a change.
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if before != count && self.res.facts().get(state).name.as_ref() == "minecraft:trapped_chest"
        {
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                let block = b.block(state);
                b.update_neighbors_at(level, ticks, p, block);
                b.update_neighbors_at(level, ticks, p.offset(0, -1, 0), block);
            });
        }
        (before == 0) != (count == 0)
    }
    /// `BarrelBlockEntity.openersCounter`: the first opener opens the barrel
    /// (`open=true`), the last one to leave closes it (`updateBlockState`).
    /// True when the count crossed zero.
    fn barrel_openers_changed(&mut self, key: (i32, i32, i32), open: bool) -> bool {
        let (before, count) = self.count_opener(key, open);
        let edge = (before == 0) != (count == 0);
        if edge {
            self.set_bool_prop(
                key,
                "open",
                count > 0,
                horizon_world::level::flags::UPDATE_ALL,
            );
        }
        edge
    }
    /// One opener more (`open`) or less at `key`: the count before and after
    /// (`ContainerOpenersCounter.incrementOpeners` / `decrementOpeners`).
    fn count_opener(&mut self, key: (i32, i32, i32), open: bool) -> (i32, i32) {
        let count = self.lid_openers.entry(key).or_default();
        let before = *count;
        *count = (before + if open { 1 } else { -1 }).max(0);
        let after = *count;
        if after == 0 {
            self.lid_openers.remove(&key);
        }
        (before, after)
    }
    /// `BellBlockEntity.updateEntities`: the living entities whose bounding
    /// box meets the bell's block box inflated by 48.
    fn bell_gather(&self, pos: [i32; 3]) -> Vec<u64> {
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos[0]) - 48.0,
                f64::from(pos[1]) - 48.0,
                f64::from(pos[2]) - 48.0,
            ),
            Vec3::new(
                f64::from(pos[0]) + 49.0,
                f64::from(pos[1]) + 49.0,
                f64::from(pos[2]) + 49.0,
            ),
        );
        let mut out = Vec::new();
        if self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            out.push(PLAYER);
        }
        for m in &self.mobs {
            if !m.removed && m.bounds().intersects(area) {
                out.push(u64::from(m.id));
            }
        }
        out
    }
    /// `BellBlockEntity.updateEntities`: every gathered entity alive within 32
    /// blocks of the bell's centre gets `HEARD_BELL_TIME` (only villagers
    /// have a brain that reads it).
    fn bell_heard(&mut self, pos: [i32; 3], now: i64) {
        let nearby = self
            .be_anims
            .bells
            .get(&pos)
            .and_then(|b| b.nearby.clone())
            .unwrap_or_default();
        for m in &mut self.mobs {
            if m.alive()
                && !m.removed
                && nearby.contains(&u64::from(m.id))
                && closer_to_center_than(pos, [m.position.x, m.position.y, m.position.z], 32.0)
            {
                horizon_client::mobs::villager_brain::heard_bell(m, now);
            }
        }
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box opens only when its lid has
    /// room to rise (`level.noCollision` of the half-open lid box).
    pub(super) fn shulker_can_open(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        let Some(Ticker::Shulker { facing }) = self.be_anims.ticker(state) else {
            return true;
        };
        let closed = self
            .be_anims
            .shulkers
            .get(&pos)
            .is_none_or(|s| s.status == ShulkerStatus::Closed);
        if !closed {
            return true;
        }
        let b = Shulker::lid_box(pos, facing);
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        no_collision_empty(
            &mut scene,
            Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])),
        )
    }
    /// The client tickers of the loaded animated block entities, once per
    /// client tick (`ClientLevel.tickBlockEntities`).
    pub(super) fn tick_block_entity_anims(&mut self) {
        // A server has no window, but its gateways and conduits still run
        // their server tickers; a client of one only animates what it is sent.
        if self.remote.is_none() {
            self.tick_conduit_servers();
        }
        let Some(r) = &self.renderer else {
            if self.remote.is_none() {
                self.tick_gateway_servers();
            }
            return;
        };
        // Sorted by position: the order they tick in, and a quick lookup.
        // Read again only when the drawn block entities changed.
        let revision = r.block_entity_revision();
        if self.be_anims.present_revision != Some(revision) {
            let mut present: Vec<([i32; 3], Ticker)> = r
                .block_entities()
                .filter_map(|(p, s)| Some((*p, self.be_anims.ticker(*s)?)))
                .collect();
            present.sort_unstable_by_key(|(p, _)| *p);
            self.be_anims.retain_present(&present);
            self.be_anims.present = present;
            self.be_anims.present_revision = Some(revision);
        }
        let positions = std::mem::take(&mut self.be_anims.present);
        let players: Vec<[f64; 3]> = match &self.player {
            Some(p) if !self.spectator() => vec![[p.position.x, p.position.y, p.position.z]],
            _ => Vec::new(),
        };
        let raiders = self
            .res
            .stations()
            .enchantments
            .entity_tags
            .get("minecraft:raiders")
            .cloned()
            .unwrap_or_default();
        let frames = self
            .res
            .items()
            .block_tags
            .get("minecraft:conduit_effect_block")
            .cloned()
            .unwrap_or_default();
        for &(pos, ticker) in &positions {
            match ticker {
                Ticker::Lid => self.be_anims.chests.entry(pos).or_default().tick(),
                Ticker::Book => {
                    let anims = &mut self.be_anims;
                    anims
                        .books
                        .entry(pos)
                        .or_default()
                        .tick(pos, &players, &mut anims.book_random);
                }
                Ticker::Shulker { facing } => {
                    // The lid's push of entities in its way (moveCollidedEntities)
                    // and the neighbour updates are left to the level.
                    self.be_anims
                        .shulkers
                        .entry(pos)
                        .or_default()
                        .tick(pos, facing);
                }
                Ticker::Skull { powered } => {
                    self.be_anims.skulls.entry(pos).or_default().tick(powered);
                }
                Ticker::Bell => {
                    let player = self.player_view().map(|p| BellEntity {
                        position: [p.position.x, p.position.y, p.position.z],
                        alive: p.alive,
                        raider: false,
                    });
                    let mobs = &self.mobs;
                    let entity = |h: u64| {
                        if h == PLAYER {
                            return player;
                        }
                        let m = mobs.iter().find(|m| u64::from(m.id) == h)?;
                        Some(BellEntity {
                            position: [m.position.x, m.position.y, m.position.z],
                            alive: m.alive() && !m.removed,
                            raider: raiders.contains(&m.kind.id),
                        })
                    };
                    let tick = self
                        .be_anims
                        .bells
                        .entry(pos)
                        .or_default()
                        .tick(pos, entity);
                    let centre = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]) + 0.5,
                        f64::from(pos[2]) + 0.5,
                    ];
                    // On a server its own copy does this and sends the sound.
                    if tick.resonate && self.remote.is_none() {
                        // The server's copy plays the resonance for everyone.
                        self.audio
                            .play("minecraft:block.bell.resonate", Some(centre), 1.0, 1.0);
                    }
                    if let Some((particles, glow)) = tick.ended {
                        for (p, color) in particles {
                            // ColorParticleOption.create(ENTITY_EFFECT, color): ARGB.
                            let c = color as u32;
                            let channel = |shift: u32| ((c >> shift) & 255) as f32 / 255.0;
                            let options = ParticleOptions {
                                color: [channel(16), channel(8), channel(0), channel(24)],
                                ..ParticleOptions::simple(ParticleType::EntityEffect)
                            };
                            self.particles.add(&options, p, [0.0; 3]);
                        }
                        // makeRaidersGlow (the server's effect).
                        for m in self.mobs.iter_mut().filter(|_| self.remote.is_none()) {
                            if glow.contains(&u64::from(m.id)) {
                                m.add_effect("minecraft:glowing", 60, 0);
                            }
                        }
                    }
                }
                Ticker::Conduit => {
                    let target = self.conduit_target(pos);
                    let world = &self.world;
                    let facts = self.res.facts();
                    let registry = self.pack.assets.registry;
                    let is_water = |p: [i32; 3]| {
                        facts.get(world.block(p[0], p[1], p[2])).fluid
                            == horizon_client::blocks::Fluid::Water
                    };
                    let is_frame = |p: [i32; 3]| {
                        registry
                            .state(world.block(p[0], p[1], p[2]))
                            .is_some_and(|s| frames.contains(&s.name))
                    };
                    let time = world.game_time;
                    let anims = &mut self.be_anims;
                    let particles = anims.conduits.entry(pos).or_default().client_tick(
                        pos,
                        time,
                        is_water,
                        is_frame,
                        target,
                        self.particles.random(),
                    );
                    let nautilus = ParticleOptions::simple(ParticleType::Nautilus);
                    for (p, v) in particles {
                        self.particles.add(&nautilus, p, v);
                    }
                }
                Ticker::Pot => {}
                Ticker::Gateway => {
                    // The server copy's portalTick runs first; its block event
                    // reaches the client copy before the client ticks it.
                    let initial = self.gateway_loaded(pos);
                    let anims = &mut self.be_anims;
                    let server = anims.gateway_servers.entry(pos).or_insert(initial);
                    // A server sends its copy's block event instead.
                    let triggered = self.remote.is_none() && server.portal_tick();
                    let client = anims.gateways.entry(pos).or_insert(initial);
                    if triggered {
                        client.trigger_event(1, 0);
                    }
                    client.tick();
                }
            }
        }
        self.be_anims.present = positions;
    }
    /// `ConduitBlockEntity.serverTick` of every conduit in the chunks that
    /// tick blocks, with or without a renderer. The conduits are looked for
    /// when the shape is checked, every 40 ticks (they have no block entity
    /// of their own here).
    fn tick_conduit_servers(&mut self) {
        if self.world.game_time % 40 == 0 {
            self.find_conduits();
        }
        let mut positions: Vec<[i32; 3]> = self.be_anims.conduit_servers.keys().copied().collect();
        positions.sort_unstable();
        for pos in positions {
            self.tick_conduit_server(pos);
        }
    }
    /// Sets `be_anims.conduit_servers` to the conduits in the
    /// chunks that tick blocks, keeping the state of those already known.
    fn find_conduits(&mut self) {
        let levels = self.tickets.levels();
        let kinds = &self.be_anims.kinds;
        let is_conduit = |id: u32| matches!(kinds.get(id as usize), Some(Some(Ticker::Conduit)));
        let mut found = Vec::new();
        for (&(cx, cz), chunk) in &self.world.chunks {
            if !levels.block_ticking_at(cx * 16, cz * 16) {
                continue;
            }
            for (si, section) in chunk.sections.iter().enumerate() {
                if !section.blocks.maybe_has(is_conduit) {
                    continue;
                }
                for i in 0..4096 {
                    if is_conduit(section.get(i)) {
                        found.push([
                            cx * 16 + (i & 15) as i32,
                            horizon_client::chunk::MIN_Y + si as i32 * 16 + (i >> 8) as i32,
                            cz * 16 + ((i >> 4) & 15) as i32,
                        ]);
                    }
                }
            }
        }
        let mut old = std::mem::take(&mut self.be_anims.conduit_servers);
        for pos in found {
            let known = old.remove(&pos).unwrap_or_default();
            self.be_anims.conduit_servers.insert(pos, known);
        }
    }
    /// `ConduitBlockEntity.serverTick` of the conduit at `pos`, in a chunk
    /// that ticks blocks: the shape check every 40 ticks with its sounds, the
    /// power for players in range, the hurt for an enemy, then the ambient
    /// sounds.
    fn tick_conduit_server(&mut self, pos: [i32; 3]) {
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        if self.be_anims.ticker(state) != Some(Ticker::Conduit) {
            // Broken since the chunks were searched.
            self.be_anims.conduit_servers.remove(&pos);
            return;
        }
        if !self.tickets.levels().block_ticking_at(pos[0], pos[2])
            || !self.border.contains(f64::from(pos[0]), f64::from(pos[2]))
        {
            return;
        }
        let time = self.world.game_time;
        let mut conduit = self
            .be_anims
            .conduit_servers
            .remove(&pos)
            .unwrap_or_default();
        let check = {
            let world = &self.world;
            let facts = self.res.facts();
            let registry = self.pack.assets.registry;
            let frames = self
                .res
                .items()
                .block_tags
                .get("minecraft:conduit_effect_block");
            // Level.isWaterAt: the fluid at the position is water.
            let is_water = |p: [i32; 3]| {
                facts.get(world.block(p[0], p[1], p[2])).fluid
                    == horizon_client::blocks::Fluid::Water
            };
            let is_frame = |p: [i32; 3]| {
                registry
                    .state(world.block(p[0], p[1], p[2]))
                    .is_some_and(|s| frames.is_some_and(|f| f.contains(&s.name)))
            };
            conduit.check(pos, time, is_water, is_frame)
        };
        let centre = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(check) = check {
            if let Some(active) = check.toggled {
                let sound = if active {
                    "minecraft:block.conduit.activate"
                } else {
                    "minecraft:block.conduit.deactivate"
                };
                self.audio.play(sound, Some(centre), 1.0, 1.0);
            }
            if let Some(frames) = check.frames {
                self.conduit_apply_effects(pos, frames);
                self.conduit_attack(&mut conduit, pos, frames >= 42);
            }
        }
        let (ambient, short) = conduit.ambient(time, &mut self.world.random);
        if ambient {
            self.audio
                .play("minecraft:block.conduit.ambient", Some(centre), 1.0, 1.0);
        }
        if short {
            self.audio.play(
                "minecraft:block.conduit.ambient_short",
                Some(centre),
                1.0,
                1.0,
            );
        }
        self.be_anims.conduit_servers.insert(pos, conduit);
    }
    /// `ConduitBlockEntity.applyEffects`: conduit power for every player
    /// in water or rain within `frames / 7 * 16` blocks.
    fn conduit_apply_effects(&mut self, pos: [i32; 3], frames: usize) {
        let range = (frames / 7 * 16) as i64;
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().filter(|p| !p.health.dead_or_dying()) else {
                return;
            };
            if g.spectator() {
                return;
            }
            let at = p.position;
            let in_water = p.fluids.was_touching_water;
            let top = at.y + f64::from(p.dimensions.height);
            // BlockPos.closerThan: the squared distance under range squared.
            let d = [at.x, at.y, at.z]
                .map(|v| v.floor() as i64)
                .iter()
                .zip(pos)
                .map(|(a, b)| (a - i64::from(b)).pow(2))
                .sum::<i64>();
            if d >= range * range || !(in_water || g.in_rain(at, top)) {
                return;
            }
            let power = EffectInstance {
                ambient: true,
                ..EffectInstance::new("conduit_power", 260, 0)
            };
            g.potions.effects.add(power);
        });
    }
    /// `Entity.isInRain`: rain at the block position or at the top of the box.
    pub(super) fn in_rain(&mut self, at: Vec3, top: f64) -> bool {
        let p = BlockPos::new(
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let up = BlockPos::new(p.x, top.floor() as i32, p.z);
        // A server's client has no live biome features to ask: it answers from
        // the chunks and the weather it was sent.
        if self.remote.is_some() {
            return self.raining_at(p) || self.raining_at(up);
        }
        self.with_level(|b, level, _| b.is_raining_at(level, p) || b.is_raining_at(level, up))
    }
    /// `ConduitBlockEntity.updateAndAttackTarget`: while the conduit hunts
    /// (42 frame blocks) its target stays until it dies or leaves the 8
    /// blocks, else a random enemy in water or rain is chosen; the target
    /// takes 4 magic damage.
    fn conduit_attack(&mut self, conduit: &mut ConduitServer, pos: [i32; 3], hunting: bool) {
        conduit.target = match conduit.target {
            _ if !hunting => None,
            None => self.conduit_new_target(pos),
            Some(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .filter(|m| {
                    let at = [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i64);
                    m.alive()
                        && at
                            .iter()
                            .zip(pos)
                            .map(|(a, b)| (a - i64::from(b)).pow(2))
                            .sum::<i64>()
                            < 64
                })
                .map(|m| m.id),
        };
        let Some(m) = conduit
            .target
            .and_then(|id| self.mobs.iter_mut().find(|m| m.id == id))
        else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        m.damage(4.0, Damage::of(DamageKind::Magic));
        self.audio
            .play("minecraft:block.conduit.attack.target", Some(at), 1.0, 1.0);
    }
    /// `ConduitBlockEntity.selectNewTarget`: one of the enemies in water or
    /// rain whose box meets the conduit's block inflated by 8.
    fn conduit_new_target(&mut self, pos: [i32; 3]) -> Option<u32> {
        let [x, y, z] = pos.map(f64::from);
        let area = Aabb::new(
            Vec3::new(x - 8.0, y - 8.0, z - 8.0),
            Vec3::new(x + 9.0, y + 9.0, z + 9.0),
        );
        let near: Vec<(u32, bool, Vec3, f64)> = self
            .mobs
            .iter()
            .filter(|m| m.hostile() && !m.removed && m.bounds().intersects(area))
            .map(|m| {
                let top = m.position.y + f64::from(m.height());
                (m.id, m.fluids.in_water, m.position, top)
            })
            .collect();
        let candidates: Vec<u32> = near
            .into_iter()
            .filter(|&(_, in_water, at, top)| in_water || self.in_rain(at, top))
            .map(|(id, ..)| id)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let pick = self
            .world
            .random
            .next_int_bound(candidates.len() as i32)
            .unwrap_or(0);
        Some(candidates[pick as usize])
    }
    /// The conduit's `destroyTarget` as the client draws it.
    fn conduit_target(&self, pos: [i32; 3]) -> Option<ConduitTarget> {
        let id = self.be_anims.conduit_servers.get(&pos)?.target?;
        let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        Some(ConduitTarget {
            position: [m.position.x, m.position.y, m.position.z],
            eye_y: m.eye_y(),
            width: m.width(),
            height: m.height(),
        })
    }
    /// `TheEndGatewayBlockEntity.portalTick` (the server ticker) of the
    /// gateways a server keeps a copy of, in block-ticking chunks
    /// (`Level.tickBlockEntities`): the cooldown an entry started runs out,
    /// and every 2400 ticks `triggerCooldown` sends its block event.
    fn tick_gateway_servers(&mut self) {
        let levels = self.tickets.levels();
        let mut positions: Vec<[i32; 3]> = self
            .be_anims
            .gateway_servers
            .keys()
            .copied()
            .filter(|p| levels.block_ticking_at(p[0], p[2]))
            .collect();
        positions.sort_unstable();
        for pos in positions {
            let triggered = self
                .be_anims
                .gateway_servers
                .get_mut(&pos)
                .is_some_and(EndGateway::portal_tick);
            if triggered {
                self.block_entity_event(pos, 1, 0);
            }
        }
    }
    /// `EndGatewayBlock.entityInside` on the server: whether the gateway at
    /// `pos` let the entity in (`triggerCooldown`, with its block event).
    pub(super) fn gateway_enter(&mut self, pos: [i32; 3]) -> bool {
        let initial = self.gateway_loaded(pos);
        let entered = self
            .be_anims
            .gateway_servers
            .entry(pos)
            .or_insert(initial)
            .enter();
        if entered {
            self.block_entity_event(pos, 1, 0);
        }
        entered
    }
    /// The End gateway at `pos` as its chunk was loaded: `Age` from the saved
    /// block entity (`getUpdateTag`), or a new gateway.
    fn gateway_loaded(&self, pos: [i32; 3]) -> EndGateway {
        let age = self.saved_tag(pos).and_then(|t| number(t.get("Age")));
        EndGateway {
            age: age.unwrap_or(0),
            teleport_cooldown: 0,
        }
    }
    /// The block entity at `pos` Horizon keeps as loaded, if any.
    fn saved_tag(&self, pos: [i32; 3]) -> Option<&Tag> {
        self.world
            .chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .and_then(|c| c.saved.as_deref())
            .and_then(|saved| {
                saved.block_entities.iter().find(|t| {
                    [t.get("x"), t.get("y"), t.get("z")]
                        .map(number)
                        .eq(&pos.map(|v| Some(i64::from(v))))
                })
            })
    }
    /// The renderer's view of the block entity at `pos`, with a decorated
    /// pot's saved `sherds` (back, left, right, front).
    pub(super) fn block_anim(&self, pos: [i32; 3], partial: f32) -> BlockAnim {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let sherds = if &*self.res.facts().get(state).name == "minecraft:decorated_pot" {
            let sherds = self.saved_tag(pos).and_then(|t| t.get("sherds"));
            ["back", "left", "right", "front"].map(|side| match sherds.and_then(|s| s.get(side)) {
                // An item id, or the item stack the processed templates hold.
                Some(Tag::String(id)) => Some(id.to_string_lossy()),
                Some(stack) => {
                    horizon_client::save::item_from_nbt(stack).map(|s| s.item.to_string())
                }
                None => None,
            })
        } else {
            Default::default()
        };
        self.be_anims
            .block_anim(pos, self.world.game_time, partial, sherds)
    }
    /// `ChestLidController.getOpenness(partial)` at `pos`.
    pub(super) fn chest_lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.be_anims.lid(pos, partial)
    }
    /// `SkullBlockRenderer.getPlayerSkin`: the skin sprite of the profile of
    /// the player head at `pos`. The skin download is started here; while it
    /// is in flight (or when it cannot be got) the profile's default skin
    /// stands in. A head with no profile answers None: `SKIN_BY_TYPE`.
    pub(super) fn skull_skin_at(&self, pos: [i32; 3]) -> Option<String> {
        let Some(BlockEntity::Skull(skull)) =
            self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return None;
        };
        let skin = skull.skin.as_ref()?;
        // The texture a packed or resolved `textures` property fetches: the
        // download of a static profile, or the one the services answer for a
        // `Dynamic` (name- or id-only) profile.
        let download = skin.download.clone().or_else(|| {
            let key = skin.dynamic.as_ref()?;
            let skins::Resolve::Done(Some(profile)) = self.skin_profiles.borrow_mut().state(key)
            else {
                return None;
            };
            let (value, signature) = profile.textures()?;
            let url = skins::unpack(&value, signature, &skins::TEXTURE_URIS).skin?;
            let location = skins::skin_location(&skins::texture_hash(&url)?);
            Some((url, location))
        });
        if let Some((url, location)) = download {
            let mut downloads = self.skin_downloads.borrow_mut();
            if self.gui.atlas.sprites.contains_key(&location) {
                let _ = downloads.state(skins::Kind::Skin, &url);
                downloads.drawn(&location);
                return Some(location);
            }
            // Its slot was evicted: offer the image again.
            if downloads.state(skins::Kind::Skin, &url) == skins::Download::Ready {
                downloads.again(&location);
            }
        }
        Some(skin.fallback.to_owned())
    }
}
