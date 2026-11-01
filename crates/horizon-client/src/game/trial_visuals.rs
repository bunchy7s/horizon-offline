//! The client side of trial spawners and vaults: `LevelEventHandler` 3011 to
//! 3021, `TrialSpawner.tickClient` (the state's particles, the spinning mob,
//! the ambient sound) and `VaultBlockEntity.Client.tick` (idle smoke and
//! flames, the connection particles towards the player, the ambient sound).
//! Every draw comes from the client level's random.
use super::Game;
use horizon_client::{
    containers::BlockEntity,
    mobs::trial_spawner::TrialState,
    particles::{ParticleOptions, ParticleType as T},
};
use horizon_core::random::{LegacyRandom, RandomSource};

type Pos = (i32, i32, i32);

/// `Mth.nextDouble`.
fn next_double(random: &mut LegacyRandom, min: f64, max: f64) -> f64 {
    if min >= max {
        min
    } else {
        random.next_double() * (max - min) + min
    }
}
/// `Vec3.offsetRandom`.
fn offset_random(v: [f64; 3], random: &mut LegacyRandom, amount: f32) -> [f64; 3] {
    v.map(|c| c + f64::from((random.next_float() - 0.5) * amount))
}
/// The pitch of `playLocalSound`'s block event sounds.
fn event_pitch(random: &mut LegacyRandom) -> f32 {
    (random.next_float() - random.next_float()) * 0.2 + 1.0
}
fn center(pos: [i32; 3]) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}
fn flame(ominous: bool) -> T {
    if ominous {
        T::SoulFireFlame
    } else {
        T::SmallFlame
    }
}

impl Game<'_> {
    fn particle(&mut self, ty: T, p: [f64; 3], v: [f64; 3]) {
        self.particles.add(&ParticleOptions::simple(ty), p, v);
    }
    fn local_sound(&mut self, name: &str, pos: [i32; 3], volume: f32, pitch: f32) {
        self.audio.play(
            &format!("minecraft:{name}"),
            Some(center(pos)),
            volume,
            pitch,
        );
    }
    /// `LevelEventHandler.levelEvent` 3011 to 3021.
    pub(super) fn trial_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) {
        // TrialSpawner.FlameParticle.decode.
        let spawn_flame = if data == 1 {
            T::SoulFireFlame
        } else {
            T::Flame
        };
        let sound = |s: &mut Self, name: &str, volume: f32| {
            let pitch = event_pitch(s.particles.level_random());
            s.local_sound(name, pos, volume, pitch);
        };
        match id {
            3011 => self.trial_spawn_particles(pos, spawn_flame),
            3012 => {
                sound(self, "block.trial_spawner.spawn_mob", 1.0);
                self.trial_spawn_particles(pos, spawn_flame);
            }
            3013 | 3019 => {
                sound(self, "block.trial_spawner.detect_player", 1.0);
                let ty = if id == 3013 {
                    T::TrialSpawnerDetection
                } else {
                    T::TrialSpawnerDetectionOminous
                };
                self.detect_player_particles(pos, data, ty);
            }
            3014 => {
                sound(self, "block.trial_spawner.eject_item", 1.0);
                self.eject_item_particles(pos);
            }
            3015 => {
                if matches!(
                    self.world.block_entities.get(&(pos[0], pos[1], pos[2])),
                    Some(BlockEntity::Vault(_))
                ) {
                    self.vault_activation_particles(pos, data != 0);
                    sound(self, "block.vault.activate", 1.0);
                }
            }
            3016 => {
                // emitDeactivationParticles.
                let ty = flame(data != 0);
                for _ in 0..20 {
                    let random = self.particles.level_random();
                    let p = [0.4, 0.4, 0.4].map(|lo| next_double(random, lo, 0.6));
                    let v = [0; 3].map(|_| random.next_gaussian() * 0.02);
                    let at = std::array::from_fn(|i| f64::from(pos[i]) + p[i]);
                    self.particle(ty, at, v);
                }
                sound(self, "block.vault.deactivate", 1.0);
            }
            3017 => self.eject_item_particles(pos),
            3018 => {
                for _ in 0..10 {
                    let random = self.particles.level_random();
                    let v = [0; 3].map(|_| random.next_gaussian() * 0.02);
                    let at = pos.map(|c| f64::from(c) + random.next_double());
                    self.particle(T::Poof, at, v);
                }
                sound(self, "block.cobweb.place", 1.0);
            }
            3020 => {
                sound(
                    self,
                    "block.trial_spawner.ominous_activate",
                    if data == 0 { 0.3 } else { 1.0 },
                );
                self.detect_player_particles(pos, 0, T::TrialSpawnerDetectionOminous);
                // addBecomeOminousParticles.
                for _ in 0..20 {
                    let random = self.particles.level_random();
                    let at = pos.map(|c| f64::from(c) + 0.5 + (random.next_double() - 0.5) * 2.0);
                    let v = [0; 3].map(|_| random.next_gaussian() * 0.02);
                    self.particle(T::TrialOmen, at, v);
                    self.particle(T::SoulFireFlame, at, v);
                }
            }
            3021 => {
                sound(self, "block.trial_spawner.spawn_item", 1.0);
                self.trial_spawn_particles(pos, spawn_flame);
            }
            _ => {}
        }
    }
    /// `TrialSpawner.addSpawnParticles`.
    fn trial_spawn_particles(&mut self, pos: [i32; 3], ty: T) {
        for _ in 0..20 {
            let random = self.particles.level_random();
            let at = pos.map(|c| f64::from(c) + 0.5 + (random.next_double() - 0.5) * 2.0);
            self.particle(T::Smoke, at, [0.0; 3]);
            self.particle(ty, at, [0.0; 3]);
        }
    }
    /// `TrialSpawner.addDetectPlayerParticles`.
    fn detect_player_particles(&mut self, pos: [i32; 3], data: i32, ty: T) {
        for _ in 0..30 + data.min(10) * 5 {
            let random = self.particles.level_random();
            let x = f64::from(2.0 * random.next_float() - 1.0) * 0.65;
            let z = f64::from(2.0 * random.next_float() - 1.0) * 0.65;
            let y = f64::from(pos[1]) + 0.1 + f64::from(random.next_float()) * 0.8;
            let at = [f64::from(pos[0]) + 0.5 + x, y, f64::from(pos[2]) + 0.5 + z];
            self.particle(ty, at, [0.0; 3]);
        }
    }
    /// `TrialSpawner.addEjectItemParticles`.
    fn eject_item_particles(&mut self, pos: [i32; 3]) {
        for _ in 0..20 {
            let random = self.particles.level_random();
            let at = pos.map(|c| f64::from(c) + 0.4 + random.next_double() * 0.2);
            let v = [0; 3].map(|_| random.next_gaussian() * 0.02);
            self.particle(T::SmallFlame, at, [v[0], v[1], v[2] * 0.25]);
            self.particle(T::Smoke, at, v);
        }
    }
    /// `TrialSpawner.tickClient`: the state's particles, the spin of the
    /// mob shown inside and the ambient sound.
    pub(super) fn client_tick_trial_spawner(&mut self, key: Pos) {
        let Some((state, ominous)) = self.trial_spawner_block(key) else {
            return;
        };
        let pos = [key.0, key.1, key.2];
        let time = self.world.game_time;
        let c = center(pos);
        let random = self.particles.level_random();
        // TrialSpawnerState.ParticleEmission.
        let mut emitted: Vec<(T, [f64; 3])> = Vec::new();
        match state {
            TrialState::WaitingForPlayers
            | TrialState::WaitingForRewardEjection
            | TrialState::EjectingReward => {
                if random.next_int_bound(2) == Ok(0) {
                    emitted.push((flame(ominous), offset_random(c, random, 0.9)));
                }
            }
            TrialState::Active => {
                let at = offset_random(c, random, 1.0);
                let ty = if ominous { T::SoulFireFlame } else { T::Flame };
                emitted.extend([(T::Smoke, at), (ty, at)]);
            }
            TrialState::Cooldown => {
                let at = offset_random(c, random, 0.9);
                if random.next_int_bound(3) == Ok(0) {
                    emitted.push((T::Smoke, at));
                }
                if time % 20 == 0 {
                    let top = [c[0], c[1] + 0.5, c[2]];
                    let count = random.next_int_bound(4).unwrap_or(0) + 20;
                    emitted.extend((0..count).map(|_| (T::Smoke, top)));
                }
            }
            TrialState::Inactive => {}
        }
        let ambient = state.capable_of_spawning() && random.next_float() <= 0.02;
        let ambient =
            ambient.then(|| (random.next_float() * 0.25 + 0.75, random.next_float() + 0.5));
        for (ty, at) in emitted {
            self.particle(ty, at, [0.0; 3]);
        }
        if let Some(BlockEntity::TrialSpawner(s)) = self.world.block_entities.get_mut(&key)
            && let Some(speed) = state.spinning_mob_speed()
        {
            let delay = (s.next_mob_spawns_at - time).max(0) as f64;
            s.o_spin = s.spin;
            s.spin = (s.spin + speed / (delay + 200.0)) % 360.0;
        }
        if let Some((volume, pitch)) = ambient {
            let name = if ominous {
                "block.trial_spawner.ambient_ominous"
            } else {
                "block.trial_spawner.ambient"
            };
            self.local_sound(name, pos, volume, pitch);
        }
        self.update_spawner_display(key);
    }
    /// `VaultBlockEntity.Client.tick`.
    pub(super) fn client_tick_vault(&mut self, key: Pos) {
        let Some((mut vault, _, ominous)) = self.take_vault(key) else {
            return;
        };
        vault.update_display_item_spin();
        let shows = vault.display_item.is_some();
        self.world
            .block_entities
            .put(key, BlockEntity::Vault(vault));
        let pos = [key.0, key.1, key.2];
        if self.world.game_time % 20 == 0 {
            self.vault_connection_particles(pos);
        }
        // emitIdleParticles.
        let random = self.particles.level_random();
        if random.next_float() <= 0.5 {
            let at = cage_point(pos, random);
            self.particle(T::Smoke, at, [0.0; 3]);
            if shows {
                self.particle(flame(ominous), at, [0.0; 3]);
            }
        }
        // playIdleSounds.
        if shows {
            let random = self.particles.level_random();
            if random.next_float() <= 0.02 {
                let volume = random.next_float() * 0.25 + 0.75;
                let pitch = random.next_float() + 0.5;
                self.local_sound("block.vault.ambient", pos, volume, pitch);
            }
        }
    }
    /// `VaultBlockEntity.Client.emitActivationParticles`.
    fn vault_activation_particles(&mut self, pos: [i32; 3], ominous: bool) {
        self.vault_connection_particles(pos);
        for _ in 0..20 {
            let at = cage_point(pos, self.particles.level_random());
            self.particle(T::Smoke, at, [0.0; 3]);
            self.particle(flame(ominous), at, [0.0; 3]);
        }
    }
    /// `emitConnectionParticlesForNearbyPlayers`: from the keyhole towards
    /// each connected player within the particles' range.
    fn vault_connection_particles(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Vault(vault)) = self.world.block_entities.get(&key) else {
            return;
        };
        let me = self.level.player_uuid.unwrap_or_default();
        let range = vault.connected_particles_range;
        // VaultSharedData.getConnectedPlayers, each through getPlayerByUUID: the
        // host and the guests, with their position and height.
        let mut players = Vec::new();
        for uuid in &vault.connected_players {
            if *uuid == me {
                if let Some(p) = self.player.as_ref() {
                    let at = [p.position.x, p.position.y, p.position.z];
                    players.push((at, p.dimensions.height));
                }
            } else if let Some(g) = self.lan_present().into_iter().find(|g| g.uuid == *uuid) {
                players.push((g.position, g.height));
            }
        }
        if players.is_empty() {
            return;
        }
        let facing = self
            .pack
            .assets
            .registry
            .state(self.world.block(pos[0], pos[1], pos[2]))
            .and_then(|b| b.properties.get("facing").cloned());
        let step = match facing.as_deref() {
            Some("south") => [0.0, 1.0],
            Some("west") => [-1.0, 0.0],
            Some("east") => [1.0, 0.0],
            _ => [0.0, -1.0],
        };
        // keyholePos: the bottom centre, 1.75 up and half a block out the front.
        let keyhole = [
            f64::from(pos[0]) + 0.5 + step[0] * 0.5,
            f64::from(pos[1]) + 1.75,
            f64::from(pos[2]) + 0.5 + step[1] * 0.5,
        ];
        for (at, height) in players {
            // isWithinConnectionRange: player.blockPosition().distSqr(vault) in range.
            let block = at.map(|v| v.floor() as i32);
            let d = [0, 1, 2].map(|i| f64::from(block[i] - pos[i]));
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > range * range {
                continue;
            }
            let target = [at[0], at[1] + f64::from(height / 2.0), at[2]];
            let direction: [f64; 3] = std::array::from_fn(|i| target[i] - keyhole[i]);
            let random = self.particles.level_random();
            let count = random.next_int_bound(4).unwrap_or(0) + 2;
            for _ in 0..count {
                let v = offset_random(direction, self.particles.level_random(), 1.0);
                self.particle(T::VaultConnection, keyhole, v);
            }
        }
    }
}

/// `randomPosInsideCage`.
fn cage_point(pos: [i32; 3], random: &mut LegacyRandom) -> [f64; 3] {
    let x = next_double(random, 0.1, 0.9);
    let y = next_double(random, 0.25, 0.75);
    let z = next_double(random, 0.1, 0.9);
    [
        f64::from(pos[0]) + x,
        f64::from(pos[1]) + y,
        f64::from(pos[2]) + z,
    ]
}
