//! Beehives and bee nests in the game (`crate::beehive` holds their logic):
//! their server tick, bees going in, and the bees let out, angry, when a
//! player breaks the hive or fire reaches it.
use super::Game;
use horizon_client::{
    Result,
    beehive::{self, Beehive, BeehiveExt, HiveHost, Release},
    containers::BlockEntity,
    items::ItemStack,
    mobs::{Mob, Who, bee},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{nbt::Tag, registry::BlockStateId};

struct GameHive<'g, 'a> {
    game: &'g mut Game<'a>,
}
impl HiveHost for GameHive<'_, '_> {
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.game.world.random
    }
    fn bees_stay_in_hive(&self, _at: [i32; 3]) -> bool {
        bee::bees_stay_in_hive(&self.game.mob_world_base(&[]))
    }
    fn blocked(&self, at: [i32; 3]) -> bool {
        !self
            .game
            .mob_world_base(&[])
            .block_collision_boxes(at[0], at[1], at[2])
            .is_empty()
    }
    fn set_honey_level(&mut self, at: [i32; 3], level: i32) {
        let state = self.game.world.block(at[0], at[1], at[2]);
        if let Some(state) = self.game.hive_with_honey(state, level) {
            let _ = self.game.level_set(at, state);
        }
    }
    fn load(&mut self, tag: &Tag) -> Option<Mob> {
        let kind = horizon_client::entity_nbt::kind(tag)?;
        if !self
            .game
            .res
            .mob_data()
            .entity_has("minecraft:beehive_inhabitors", &kind)
        {
            return None;
        }
        let id = self.game.load_mob(tag.clone(), None)?;
        let i = self.game.mobs.iter().position(|m| m.id == id)?;
        Some(self.game.mobs.remove(i))
    }
    fn add(&mut self, m: Mob) -> bool {
        self.game.mobs.push(m);
        true
    }
    fn sound(&mut self, name: &'static str, at: [f64; 3]) {
        self.game
            .audio
            .play(&format!("minecraft:{name}"), Some(at), 1.0, 1.0);
    }
}

impl Game<'_> {
    fn hive_with_honey(&self, state: BlockStateId, level: i32) -> Option<BlockStateId> {
        let registry = self.pack.assets.registry;
        let mut s = registry.state(state)?.clone();
        s.properties
            .insert("honey_level".to_owned(), level.to_string());
        registry.resolve(&s).ok()
    }
    /// The hive's honey level and front, from its block state.
    pub(super) fn hive_state(&self, at: [i32; 3]) -> Option<(Option<i32>, [i32; 2])> {
        let state = self.world.block(at[0], at[1], at[2]);
        let name = &self.res.facts().get(state).name;
        if !self
            .res
            .items()
            .block_tags
            .get("minecraft:beehives")
            .is_some_and(|t| t.contains(&**name))
        {
            return None;
        }
        let props = &self.pack.assets.registry.state(state)?.properties;
        let honey = props.get("honey_level").and_then(|v| v.parse().ok());
        let facing = beehive::facing_step(props.get("facing").map(String::as_str));
        Some((honey, facing))
    }
    /// `BeehiveBlockEntity.serverTick` of the hive at `key`, after an
    /// emergency exit if fire is beside it (`BeehiveBlock.updateShape`).
    pub(super) fn tick_beehive(&mut self, key: (i32, i32, i32)) {
        let at = [key.0, key.1, key.2];
        let Some((honey, facing)) = self.hive_state(at) else {
            return;
        };
        let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.take(&key) else {
            return;
        };
        let fire = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ]
        .iter()
        .any(|d| {
            let state = self.world.block(at[0] + d[0], at[1] + d[1], at[2] + d[2]);
            &*self.res.facts().get(state).name == "minecraft:fire"
        });
        let mut host = GameHive { game: self };
        if fire && !hive.bees.is_empty() {
            hive.release_all(at, facing, honey, Release::Emergency, &mut host);
        }
        hive.server_tick(at, facing, honey, &mut host);
        self.world
            .block_entities
            .put(key, BlockEntity::Beehive(hive));
    }
    /// `BeehiveBlockEntity.addOccupant` for a bee its `BeeEnterHiveGoal`
    /// sent in: off its lead, its tag stored, the bee gone.
    pub(super) fn bee_enter_hive(&mut self, mob: u32, at: [i32; 3]) {
        let key = (at[0], at[1], at[2]);
        let Some(i) = self
            .mobs
            .iter()
            .position(|m| m.id == mob && m.alive() && !m.removed)
        else {
            return;
        };
        if !matches!(self.world.block_entities.get(&key), Some(BlockEntity::Beehive(h)) if !h.full())
        {
            return;
        }
        if self.mobs[i].extra.leash.is_some() {
            self.break_leash(mob, true, false);
            self.mobs[i].extra.leash = None;
        }
        self.mobs[i].extra.mount = None;
        let tag = self.mob_tree_nbt(i);
        let flower = bee::get(&self.mobs[i]).and_then(|b| b.flower);
        let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.take(&key) else {
            return;
        };
        let stored = hive.add_occupant(at, tag, flower, &mut GameHive { game: self });
        self.world
            .block_entities
            .put(key, BlockEntity::Beehive(hive));
        if stored && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
            m.removed = true;
        }
    }
    /// `BeehiveBlockEntity.emptyAllLivingFromHive` with no player about.
    pub(super) fn empty_hive(&mut self, at: [i32; 3], facing: [i32; 2], status: Release) {
        let key = (at[0], at[1], at[2]);
        let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.take(&key) else {
            return;
        };
        hive.release_all(at, facing, None, status, &mut GameHive { game: self });
        self.world
            .block_entities
            .put(key, BlockEntity::Beehive(hive));
    }
    /// `BeehiveBlock.playerDestroy` without silk touch: the bees come out
    /// angry at the player unless smoke calms them, then
    /// `angerNearbyBees`.
    pub(super) fn hive_broken_by_player(
        &mut self,
        at: [i32; 3],
        mut hive: Beehive,
        honey: Option<i32>,
        facing: [i32; 2],
    ) {
        let released = hive.release_all(
            at,
            facing,
            honey,
            Release::Emergency,
            &mut GameHive { game: self },
        );
        let sedated = beehive::smokey(&self.mob_world_base(&[]), at);
        if let Some(p) = self.player_view().map(|p| p.position) {
            for m in self.mobs.iter_mut().filter(|m| released.contains(&m.id)) {
                if m.distance_sqr(p) <= 16.0 {
                    if sedated {
                        bee::set_stay_out_of_hive_countdown(m);
                    } else {
                        m.ai.target = Some(Who::Player);
                    }
                }
            }
        }
        self.anger_nearby_bees(at);
    }
    /// `BeehiveBlock.useItemOn` with `held` in the acting hand: shears take
    /// the honeycombs and a glass bottle a honey bottle from a full hive
    /// (honey level 5), then the honey level resets and, unless a campfire
    /// smokes the hive, the bees come out angry. True when it took the click.
    pub(super) fn use_beehive(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> Result<bool> {
        let Some(held) = held else { return Ok(false) };
        let shears = &*held.item == "minecraft:shears";
        if !shears && &*held.item != "minecraft:glass_bottle" {
            return Ok(false);
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !matches!(
            &*self.res.facts().get(state).name,
            "minecraft:beehive" | "minecraft:bee_nest"
        ) {
            return Ok(false);
        }
        let prop = |key: &str| {
            self.pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get(key).cloned())
        };
        if prop("honey_level").and_then(|v| v.parse::<i32>().ok()) != Some(5) {
            return Ok(false);
        }
        let facing = beehive::facing_step(prop("facing").as_deref());
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if shears {
            // dropHoneycomb (harvest/beehive), the shear sound, hurtAndBreak.
            for stack in self.sequence_loot(
                "minecraft:harvest/beehive",
                "minecraft:harvest/beehive",
                Some(held),
            ) {
                self.pop_resource(pos, stack);
            }
            self.audio
                .play("minecraft:block.beehive.shear", at, 1.0, 1.0);
            self.damage_held(1);
            self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        } else {
            // shrink(1), then the honey bottle in the hand, the inventory or
            // on the ground.
            self.audio.play("minecraft:item.bottle.fill", at, 1.0, 1.0);
            let cell = self.held_mut();
            if let Some(s) = cell.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *cell = None;
                }
            }
            let bottle = ItemStack::new("minecraft:honey_bottle", 1);
            if self.held().is_none() {
                *self.held_mut() = Some(bottle);
            } else if let Some(rest) = self.give(bottle)
                && let Some(eye) = self.eye_position()
            {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
            self.post_block_game_event("minecraft:fluid_pickup", pos, self.sculk_actor(), None);
        }
        self.stats.add("used", &held.item, 1);
        let smokey = beehive::smokey(&self.mob_world_base(&[]), pos);
        let key = (pos[0], pos[1], pos[2]);
        if !smokey
            && matches!(self.world.block_entities.get(&key), Some(BlockEntity::Beehive(h)) if !h.bees.is_empty())
        {
            self.anger_nearby_bees(pos);
        }
        // resetHoneyLevel.
        if let Some(reset) = self.with_props(state, &[("honey_level", "0")]) {
            self.level_set(pos, reset)?;
        }
        if !smokey
            && let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.take(&key)
        {
            // emptyAllLivingFromHive: the bees within 4 blocks of the player
            // go for them.
            let released = hive.release_all(
                pos,
                facing,
                None,
                Release::Emergency,
                &mut GameHive { game: self },
            );
            self.world
                .block_entities
                .put(key, BlockEntity::Beehive(hive));
            if let Some(p) = self.player_view().map(|p| p.position) {
                for m in self.mobs.iter_mut().filter(|m| released.contains(&m.id)) {
                    if m.distance_sqr(p) <= 16.0 {
                        m.ai.target = Some(Who::Player);
                    }
                }
            }
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, held);
        Ok(true)
    }
    /// `BeehiveBlock.angerNearbyBees`: bees within 8 blocks (6 up and down)
    /// without a target go for a random player in the same box, the window's
    /// own or a guest (`None` is the window's player).
    pub(super) fn anger_nearby_bees(&mut self, at: [i32; 3]) {
        let min = Vec3::new(
            f64::from(at[0]) - 8.0,
            f64::from(at[1]) - 6.0,
            f64::from(at[2]) - 8.0,
        );
        let area = Aabb::new(min, Vec3::new(min.x + 17.0, min.y + 13.0, min.z + 17.0));
        let mut players: Vec<Option<u32>> = Vec::new();
        if self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            players.push(None);
        }
        if let Some(host) = self.lan.as_ref() {
            players.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
                    .filter(|g| super::lan_mobs::guest_bounds(g).intersects(area))
                    .map(|g| Some(g.conn)),
            );
        }
        if players.is_empty() {
            return;
        }
        let mut bees: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| {
                let m = &self.mobs[i];
                m.species() == horizon_client::mobs::Species::Bee
                    && !m.removed
                    && m.bounds().intersects(area)
            })
            .collect();
        bees.retain(|&i| self.mobs[i].ai.target.is_none());
        for i in bees {
            // Util.getRandom over the players.
            let pick = self
                .world
                .random
                .next_int_bound(players.len() as i32)
                .unwrap_or(0) as usize;
            self.mobs[i].ai.target = Some(Who::Player);
            // The player a mob sees is its guest binding, else the window's.
            if let Some(host) = self.lan.as_mut() {
                match players[pick] {
                    Some(conn) => host.mob_players.insert(self.mobs[i].id, conn),
                    None => host.mob_players.remove(&self.mobs[i].id),
                };
            }
        }
    }
}
