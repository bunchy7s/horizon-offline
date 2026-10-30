//! What villagers do with each other and with the player: gossip and
//! reputation, sharing food and wares, picking up dropped items, making
//! babies, calling iron golems and the prices they ask.
use super::Game;
use horizon_client::{
    gossip::{self, GossipType},
    items::ItemStack,
    mobs::{Mob, Species, spawning, villager, villager_brain},
};
use horizon_core::geometry::Vec3;

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
impl Game<'_> {
    fn villager_index(&self, id: u32) -> Option<usize> {
        self.mobs
            .iter()
            .position(|m| m.id == id && m.species() == Species::Villager)
    }
    fn item_max_stack(&self, item: &str) -> u32 {
        self.items().get(item).map_or(64, |i| i.max_stack)
    }
    /// `WorkAtComposter.useWorkstation`: at a composter the farmer makes
    /// bread, then composts seeds.
    pub(super) fn villager_work_at_composter(&mut self, id: u32, pos: [i32; 3]) {
        let Some(i) = self.villager_index(id) else {
            return;
        };
        if self.composter_level(pos).is_none() {
            return;
        }
        self.villager_make_bread(i);
        self.villager_compost_items(i, pos);
    }
    /// `WorkAtComposter.makeBread`: up to three loaves from nine wheat while
    /// the villager holds at most 36 bread; what does not fit drops.
    fn villager_make_bread(&mut self, i: usize) {
        let max_stack = self.item_max_stack("minecraft:bread");
        let e = villager::ext_mut(&mut self.mobs[i]);
        if e.count_item("minecraft:bread") > 36 {
            return;
        }
        let loaves = (e.count_item("minecraft:wheat") / 3).min(3);
        if loaves == 0 {
            return;
        }
        e.remove_item("minecraft:wheat", loaves * 3);
        let left = e.add_item(ItemStack::new("minecraft:bread", loaves), max_stack);
        if let Some(rest) = left {
            // spawnAtLocation(level, stack, 0.5F)
            let p = self.mobs[i].position;
            self.spawn_item(rest, [p.x, p.y + 0.5, p.z], [0.0, 0.2, 0.0], 10);
        }
    }
    /// `BehaviorUtils.throwItem`: toss a stack from a villager's hand toward
    /// a point.
    fn villager_throw(&mut self, from: Vec3, eye: f64, to: Vec3, stack: ItemStack) {
        let at = [from.x, eye - 0.3, from.z];
        let d = Vec3::new(to.x - from.x, to.y - from.y, to.z - from.z).normalize();
        self.spawn_item(stack, at, [d.x * 0.3, d.y * 0.3, d.z * 0.3], 10);
    }

    /// `TradeWithVillager.tick` for one pair: gossip (at most once in 1200
    /// ticks for both), then surplus food and wanted wares tossed over.
    pub(super) fn villager_meet(&mut self, a: u32, b: u32) {
        let (Some(ia), Some(ib)) = (self.villager_index(a), self.villager_index(b)) else {
            return;
        };
        let now = self.world.game_time;
        let items = world_items!(self);
        let meeting = villager::meet(&mut self.mobs, ia, ib, now, |item| {
            items.get(item).map_or(64, |i| i.max_stack)
        });
        for stack in meeting.thrown {
            self.villager_throw(meeting.from, meeting.eye, meeting.to, stack);
        }
        if meeting.gossiped {
            self.villager_golem_check(a, 5);
        }
    }

    /// `Villager.spawnGolemIfNeeded`: when enough villagers nearby that
    /// slept recently and saw no golem agree, an iron golem appears.
    pub(super) fn villager_golem_check(&mut self, id: u32, needed: usize) {
        let Some(i) = self.villager_index(id) else {
            return;
        };
        let now = self.world.game_time;
        let Some(near) = villager::golem_summoners(&self.mobs, i, now, needed) else {
            return;
        };
        if self.try_spawn_iron_golem(block_of(self.mobs[i].position)) {
            for j in near {
                villager_brain::golem_detected(&mut self.mobs[j], now);
            }
        }
    }
    /// `SpawnUtil.trySpawnMob(IRON_GOLEM, ...)`: a golem near `start`, with
    /// the ambient sound `trySpawnMob` plays.
    fn try_spawn_iron_golem(&mut self, start: [i32; 3]) -> bool {
        let Some(kind) = self.res.mob_types().get("minecraft:iron_golem").cloned() else {
            return false;
        };
        let id = self.next_entity + 1;
        let mut random = self.world.random.clone();
        let golem = villager::summon_golem(
            &self.mob_world_base(&[]),
            start,
            &mut random,
            |random, at, yaw| {
                let mut m = Mob::new(id, kind.clone(), at, yaw);
                villager::finalize_summoned_golem(&mut m, random);
                m
            },
        );
        self.world.random = random;
        let Some(golem) = golem else {
            return false;
        };
        self.next_entity = id;
        let sound = golem.kind.sounds[0];
        if !sound.is_empty() {
            let at = golem.center();
            self.audio
                .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
        }
        self.mobs.push(golem);
        true
    }

    /// `VillagerMakeLove.tryToGiveBirth`: both eat, a free bed within 48
    /// blocks is taken for the child, who is a baby villager of one of
    /// three villager types.
    pub(super) fn villager_birth(
        &mut self,
        parent: u32,
        partner: u32,
        child: Option<([i32; 3], Option<u8>)>,
    ) {
        let (Some(ia), Some(ib)) = (self.villager_index(parent), self.villager_index(partner))
        else {
            return;
        };
        villager::ext_mut(&mut self.mobs[ib]).eat_and_digest();
        let from = self.mobs[ia].position;
        let Some((bed, variant)) = child else {
            // VillagerMakeLove: entity event 13 for the partner, then the parent.
            for i in [ib, ia] {
                let id = self.mobs[i].id;
                self.mob_event_particles(id, 13);
            }
            return;
        };
        let variant = variant.unwrap_or_else(|| {
            let facts = self.biome_facts(
                from.x.floor() as i32,
                from.y.floor() as i32,
                from.z.floor() as i32,
            );
            spawning::villager_type(&facts)
        });
        self.mobs[ib].extra.age = 6000;
        let Some(id) =
            self.spawn_mob_for("minecraft:villager", [from.x, from.y, from.z], true, true)
        else {
            return;
        };
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            child.extra.variant = variant;
            child.extra.variant2 = villager::NONE;
            child.extra.age = -24000;
            child.extra.bed = Some(bed);
        }
        // VillagerMakeLove.giveBirth: entity event 12 for the child.
        self.mob_event_particles(id, 12);
    }
    /// `Level.broadcastEntityEvent` of an event whose `handleEntityEvent`
    /// draws particles about the mob (villager 12, 13 and 14, taming 6 and 7,
    /// love 18): the guests get the byte, and this game draws it as its own
    /// client would.
    pub(super) fn mob_event_particles(&mut self, id: u32, event: i8) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        self.lan_entity_event(id as i32, event, at);
        self.entity_event_particles(id, event);
    }
    /// `handleEntityEvent(event)`'s particles about mob `id`
    /// (`Villager`, `TamableAnimal`, `AbstractHorse`, `Animal`, `Allay`).
    pub(super) fn entity_event_particles(&mut self, id: u32, event: i8) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        if let Some((kind, burst)) =
            horizon_client::particles::entity_event_burst(&m.kind.id, event as u8)
        {
            self.particles_around_mob(id, kind, burst);
        }
    }
    /// `particles::around_self` about mob `id`.
    pub(super) fn particles_around_mob(
        &mut self,
        id: u32,
        kind: &'static str,
        burst: horizon_client::particles::Burst,
    ) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let feet = [m.position.x, m.position.y, m.position.z];
        let size = [f64::from(m.width()), f64::from(m.height())];
        self.particles_around(kind, feet, size, burst);
    }
    /// `particles::around_self` on the client: the particle engine's random
    /// stands in for the client entity's, so no server draw moves.
    pub(super) fn particles_around(
        &mut self,
        kind: &'static str,
        feet: [f64; 3],
        size: [f64; 2],
        burst: horizon_client::particles::Burst,
    ) {
        let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) else {
            return;
        };
        let options = horizon_client::particles::ParticleOptions::simple(ty);
        let mut spawned = Vec::new();
        horizon_client::particles::around_self(
            self.particles.random(),
            feet,
            size[0],
            size[1],
            burst,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }

    /// `NearestItemSensor.doTick` for the villagers whose sensor is due: the
    /// nearest dropped item within 32 blocks that `Villager.wantsToPickUp`
    /// accepts and the villager sees, or none.
    pub(super) fn villager_item_sense(&mut self) {
        if self.dropped.is_empty() {
            for m in &mut self.mobs {
                if m.species() == Species::Villager && villager_brain::take_item_scan(m) {
                    villager_brain::set_wanted_item(m, None);
                }
            }
            return;
        }
        let picks_up = self
            .res
            .items()
            .item_tags
            .get("minecraft:villager_picks_up")
            .cloned();
        let mut due = Vec::new();
        for (i, m) in self.mobs.iter_mut().enumerate() {
            if m.species() == Species::Villager && villager_brain::take_item_scan(m) {
                due.push(i);
            }
        }
        if due.is_empty() {
            return;
        }
        let w = self.mob_world_base(&[]);
        let mut found = Vec::new();
        for i in due {
            let m = &self.mobs[i];
            let area = m.bounds().inflate(Vec3::new(32.0, 16.0, 32.0));
            let requested = villager::requested_items(m);
            let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
            let mut best: Option<(f64, Vec3)> = None;
            for e in &self.dropped {
                let d = e.position.distance_squared(m.position);
                if best.is_some_and(|(b, _)| d >= b)
                    || !e.bounds().intersects(area)
                    || d >= 32.0 * 32.0
                {
                    continue;
                }
                let item = &e.stack.item;
                let wants = villager::food_points(item) > 0
                    || picks_up.as_ref().is_some_and(|t| t.contains(&**item))
                    || requested.contains(&&**item);
                let fits =
                    villager::ext(m).is_none_or(|x| x.can_add(&e.stack, self.item_max_stack(item)));
                // ItemEntity's eye height is 0.2125.
                let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
                if wants && fits && w.line_of_sight(eye, at) {
                    best = Some((d, e.position));
                }
            }
            found.push((i, best.map(|(_, p)| [p.x, p.y, p.z])));
        }
        for (i, at) in found {
            villager_brain::set_wanted_item(&mut self.mobs[i], at);
        }
    }
    /// `Mob.aiStep` item pickup for villagers (`InventoryCarrier`): food,
    /// seeds and what their profession wants, from within a block.
    pub(super) fn villager_pickups(&mut self) {
        if self.dropped.is_empty() || !self.ui.rules.bool("mob_griefing") {
            return;
        }
        let picks_up = self
            .res
            .items()
            .item_tags
            .get("minecraft:villager_picks_up")
            .cloned();
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            let (pos, half) = (m.position, f64::from(m.width()) / 2.0 + 1.0);
            let top = f64::from(m.height());
            let requested = villager::requested_items(m);
            let mut j = 0;
            while j < self.dropped.len() {
                let e = &self.dropped[j];
                let near = (e.position.x - pos.x).abs() <= half
                    && (e.position.z - pos.z).abs() <= half
                    && e.position.y >= pos.y - 0.25
                    && e.position.y <= pos.y + top;
                let item = e.stack.item.clone();
                let wants = villager::food_points(&item) > 0
                    || picks_up.as_ref().is_some_and(|t| t.contains(&*item))
                    || requested.contains(&&*item);
                if !near || e.pickup_delay > 0 || !wants {
                    j += 1;
                    continue;
                }
                let max = self.item_max_stack(&item);
                let stack = self.dropped[j].stack.clone();
                if !villager::ext_mut(&mut self.mobs[i]).can_add(&stack, max) {
                    j += 1;
                    continue;
                }
                let before = self.dropped[j].clone();
                let rest = villager::ext_mut(&mut self.mobs[i]).add_item(stack, max);
                if rest.as_ref().is_none_or(|r| r.count < before.stack.count) {
                    let id = self.mobs[i].id;
                    self.add_pickup(
                        super::pickups::Taken::Item(Box::new(before)),
                        super::pickups::Collector::Mob(id),
                    );
                }
                match rest {
                    None => {
                        self.dropped.remove(j);
                    }
                    Some(rest) => {
                        self.dropped[j].stack.count = rest.count;
                        j += 1;
                    }
                }
            }
        }
    }

    /// The UUID of the player who last hurt a mob (`last_hurt_by_player`:
    /// None for the host, else the guest's connection): the reputation a
    /// villager keeps is that player's, the one trading looks up.
    fn player_uuid_of(&self, who: Option<u32>) -> Option<[i32; 4]> {
        let uuid = match who {
            None => return self.level.player_uuid,
            // While that guest acts its own UUID is swapped in.
            Some(conn) if self.lan_acting == Some(conn) => self.lan_acting_uuid?,
            Some(conn) => self.lan.as_ref()?.hub.guest(conn)?.uuid,
        };
        Some(horizon_client::lan::playerdata::uuid_ints(uuid))
    }
    /// `Villager.setLastHurtByMob` and `ServerLevel.onReputationEvent`: a hit
    /// from a player counts against that player with the villager that was hit.
    pub(super) fn villager_reputation_tick(&mut self) {
        let mut angry = Vec::new();
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Villager
                || m.ai.last_hurt_by != Some(horizon_client::mobs::Who::Player)
            {
                continue;
            }
            let time = m.ai.last_hurt_by_time;
            let alive = m.alive();
            let at = m.id;
            if villager::ext(m).is_some_and(|e| e.hurt_seen == time) {
                continue;
            }
            let hurt_by = self.last_hurt_by_player.get(&at).copied().flatten();
            let Some(uuid) = self.player_uuid_of(hurt_by) else {
                continue;
            };
            let e = villager::ext_mut(&mut self.mobs[i]);
            e.hurt_seen = time;
            e.gossips.add(
                uuid,
                GossipType::MinorNegative,
                gossip::REPUTATION_CHANGE_PER_EVENT,
            );
            if alive {
                angry.push(at);
            }
        }
        // Villager.setLastHurtByMob: entity event 13.
        for id in angry {
            self.mob_event_particles(id, 13);
        }
    }
    /// `Villager.tellWitnessesThatIWasMurdered`: the villagers that saw a
    /// player kill one remember it as a major wrong.
    /// `killer` is the guest connection that last hurt it (None: the host).
    pub(super) fn villager_murdered(&mut self, victim: &Mob, by_player: bool, killer: Option<u32>) {
        if victim.species() != Species::Villager || !by_player {
            return;
        }
        let Some(uuid) = self.player_uuid_of(killer) else {
            return;
        };
        let eye = Vec3::new(victim.position.x, victim.eye_y(), victim.position.z);
        let witnesses: Vec<usize> = {
            let w = self.mob_world_base(&[]);
            self.mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.id != victim.id
                        && m.species() == Species::Villager
                        && m.alive()
                        && (m.position.x - victim.position.x).abs() <= 16.0
                        && (m.position.y - victim.position.y).abs() <= 16.0
                        && (m.position.z - victim.position.z).abs() <= 16.0
                        && w.line_of_sight(eye, Vec3::new(m.position.x, m.eye_y(), m.position.z))
                })
                .map(|(i, _)| i)
                .collect()
        };
        for i in witnesses {
            villager::ext_mut(&mut self.mobs[i]).gossips.add(
                uuid,
                GossipType::MajorNegative,
                gossip::REPUTATION_CHANGE_PER_EVENT,
            );
        }
    }
    /// `ReputationEventType.TRADE`: each trade earns a little trust.
    pub(super) fn villager_traded(&mut self, id: u32) {
        if let Some(uuid) = self.level.player_uuid {
            self.villager_traded_by(id, uuid);
        }
    }
    /// The TRADE reputation event from the player `uuid` (a guest's too).
    pub(super) fn villager_traded_by(&mut self, id: u32, uuid: [i32; 4]) {
        let Some(i) = self.villager_index(id) else {
            return;
        };
        villager::ext_mut(&mut self.mobs[i]).gossips.add(
            uuid,
            GossipType::Trading,
            gossip::REPUTATION_CHANGE_PER_TRADE,
        );
        // Villager.customServerAiStep after a trade: entity event 14.
        self.mob_event_particles(id, 14);
    }
    /// `Villager.getPlayerReputation` of the local player.
    pub(super) fn villager_reputation(&self, id: u32) -> i32 {
        let Some(uuid) = self.level.player_uuid else {
            return 0;
        };
        self.villager_reputation_of(id, uuid)
    }
    /// `Villager.getPlayerReputation` of the player `uuid`.
    pub(super) fn villager_reputation_of(&self, id: u32, uuid: [i32; 4]) -> i32 {
        self.villager_index(id)
            .and_then(|i| villager::ext(&self.mobs[i]))
            .map_or(0, |e| e.gossips.reputation(uuid))
    }
    /// The Hero of the Village amplifier of the player, if any.
    pub(super) fn hero_amplifier(&self) -> Option<i32> {
        self.potions.effects.amplifier("hero_of_the_village")
    }
    /// `AbstractBedBlock.kickVillagerOutOfBed`: the first sleeping villager
    /// whose box touches the bed's head block wakes up
    /// (`Villager.stopSleeping`). Whether one did.
    pub(super) fn kick_villager_out_of_bed(&mut self, head: [i32; 3]) -> bool {
        let min = Vec3::new(f64::from(head[0]), f64::from(head[1]), f64::from(head[2]));
        let max = Vec3::new(min.x + 1.0, min.y + 1.0, min.z + 1.0);
        let Some(i) = self.mobs.iter().position(|m| {
            m.species() == Species::Villager
                && villager::sleeping(m)
                && m.bounds().intersects_corners(min, max)
        }) else {
            return false;
        };
        let id = self.mobs[i].id;
        self.wake_villager(id);
        true
    }
    /// `LivingEntity.stopSleeping` for the villager `id`, outside its own
    /// tick: it stands up and its bed is no longer occupied.
    pub(super) fn wake_villager(&mut self, id: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let mut mobs = std::mem::take(&mut self.mobs);
        let mut events = Vec::new();
        villager::stop_sleeping(&mut mobs[i], &self.mob_world_base(&[]), &mut events);
        villager_brain::woken(&mut mobs[i], self.world.game_time);
        self.mobs = mobs;
        if let Err(e) = self.handle_mob_events(events) {
            log::warn!("waking villager {id}: {e}");
        }
    }
}
