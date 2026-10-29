//! Vanilla's `SoundEngine` bookkeeping, separated from the mixer: sound
//! instances resolved from the registry with their own random, the channel
//! pools (`Library.CountingChannelPool`), `calculateVolume`/`calculatePitch`,
//! the channel parameters a play sets (gain, pitch, linear attenuation, looping,
//! position, relative), the subtitle notifications, tickable instances,
//! delayed and manually looped sounds, the 20-tick minimum channel lifetime,
//! `isActive`, category gains and stopping by sound or source.
//!
//! The executor is modelled as running every channel task immediately (the
//! sound thread keeps up between ticks); a channel's own end is reported by the
//! backend through `stopped`.
use crate::sound_events::{INTENTIONALLY_EMPTY, Sentinel, Sound, SoundRegistry};
use horizon_core::random::LegacyRandom;
use std::collections::HashMap;

/// `SoundSource` indices (vanilla order).
pub const MASTER: usize = 0;
pub const MUSIC: usize = 1;
pub const RECORDS: usize = 2;
pub const NEUTRAL: usize = 6;
pub const PLAYERS: usize = 7;
pub const AMBIENT: usize = 8;
pub const UI: usize = 10;
/// The pool sizes `Library.init` derives from OpenAL Soft's default 255 mono
/// sources: `clamp((int) sqrt(255), 2, 8)` streaming, the rest static.
pub const STATIC_CHANNELS: usize = 247;
pub const STREAMING_CHANNELS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayResult {
    Started,
    StartedSilently,
    NotStarted,
}
/// `SoundInstance.Attenuation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attenuation {
    None,
    Linear,
}
/// The tickable instance kinds Horizon plays (`TickableSoundInstance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ticking {
    /// Not tickable.
    No,
    /// `BiomeAmbientSoundsHandler.LoopSoundInstance`.
    BiomeLoop { fade: i32, direction: i32 },
    /// `UnderLiquidAmbientSoundInstance`.
    UnderLiquidLoop { fade: i32 },
    /// `UnderLiquidSubSound`.
    UnderLiquidSub,
    /// `MinecartSoundInstance` following a minecart.
    Minecart { cart: u32 },
    /// `RidingMinecartSoundInstance` for the player riding a minecart.
    RidingMinecart { cart: u32, underwater: bool },
    /// `BeeFlyingSoundInstance`, or `BeeAggressiveSoundInstance` when
    /// `aggressive`.
    Bee { bee: u32, aggressive: bool },
    /// `GuardianAttackSoundInstance`.
    GuardianAttack { guardian: u32 },
    /// `ElytraOnPlayerSoundInstance`: ticks since it started.
    Elytra { time: i32 },
}
/// What a minecart's sound instances read about it.
#[derive(Clone, Copy, Debug)]
pub struct CartSound {
    pub id: u32,
    pub position: [f64; 3],
    /// `getDeltaMovement().horizontalDistance()`.
    pub speed: f64,
}
/// What a bee's or a guardian's sound instance reads about it.
#[derive(Clone, Copy, Debug)]
pub struct MobSound {
    pub id: u32,
    pub position: [f64; 3],
    /// `getDeltaMovement().horizontalDistance()`.
    pub speed: f64,
    pub baby: bool,
    /// `Bee.isAngry`.
    pub angry: bool,
    /// `isSilent`.
    pub silent: bool,
    /// `Guardian.getAttackAnimationScale(0)`.
    pub attack_scale: f32,
}
/// What tickable instances read about the player and the entities they follow.
#[derive(Clone, Debug, Default)]
pub struct TickContext {
    pub player_underwater: bool,
    pub player_removed: bool,
    /// The minecart the player rides.
    pub vehicle: Option<u32>,
    /// The minecarts in the level (a missing one has been removed).
    pub carts: Vec<CartSound>,
    /// The bees and guardians in the level (a missing one has been removed).
    pub mobs: Vec<MobSound>,
    /// `!tickRateManager().runsNormally()`.
    pub frozen: bool,
    /// The player's position.
    pub player_position: [f64; 3],
    /// `getDeltaMovement().lengthSqr()` of the player.
    pub player_speed_sqr: f64,
    /// `isFallFlying()` of the player.
    pub player_fall_flying: bool,
}
/// `AbstractSoundInstance` and its subclasses' fields.
#[derive(Clone, Debug)]
pub struct SoundInstance {
    pub identifier: String,
    pub source: usize,
    pub volume: f32,
    pub pitch: f32,
    pub position: [f64; 3],
    pub looping: bool,
    pub delay: i32,
    pub attenuation: Attenuation,
    pub relative: bool,
    pub random: LegacyRandom,
    pub can_start_silent: bool,
    pub ticking: Ticking,
    /// `AbstractTickableSoundInstance.stopped`.
    pub stopped: bool,
    /// `sound` after `getOrResolve`.
    pub sound: Option<Sound>,
    /// `soundEvent != null` after `getOrResolve`, and its subtitle.
    pub event: Option<Option<String>>,
}
impl SoundInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identifier: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        random: LegacyRandom,
        looping: bool,
        delay: i32,
        attenuation: Attenuation,
        position: [f64; 3],
        relative: bool,
    ) -> Self {
        Self {
            identifier: identifier.to_owned(),
            source,
            volume,
            pitch,
            position,
            looping,
            delay,
            attenuation,
            relative,
            random,
            can_start_silent: false,
            ticking: Ticking::No,
            stopped: false,
            sound: None,
            event: None,
        }
    }
    /// `new SimpleSoundInstance(sound, source, volume, pitch, random, x, y, z)`.
    pub fn at(
        id: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        random: LegacyRandom,
        at: [f64; 3],
    ) -> Self {
        Self::new(
            id,
            source,
            volume,
            pitch,
            random,
            false,
            0,
            Attenuation::Linear,
            at,
            false,
        )
    }
    /// `SimpleSoundInstance.forUI(sound, pitch, volume)`.
    pub fn for_ui(id: &str, pitch: f32, volume: f32, random: LegacyRandom) -> Self {
        Self::new(
            id,
            UI,
            volume,
            pitch,
            random,
            false,
            0,
            Attenuation::None,
            [0.0; 3],
            true,
        )
    }
    /// `SimpleSoundInstance.forMusic`.
    pub fn for_music(id: &str, random: LegacyRandom) -> Self {
        Self::new(
            id,
            MUSIC,
            1.0,
            1.0,
            random,
            false,
            0,
            Attenuation::None,
            [0.0; 3],
            true,
        )
    }
    /// `SimpleSoundInstance.forJukeboxSong`.
    pub fn for_jukebox_song(id: &str, at: [f64; 3], random: LegacyRandom) -> Self {
        Self::new(
            id,
            RECORDS,
            4.0,
            1.0,
            random,
            false,
            0,
            Attenuation::Linear,
            at,
            false,
        )
    }
    /// `SimpleSoundInstance.forLocalAmbience`.
    pub fn for_local_ambience(id: &str, pitch: f32, volume: f32, random: LegacyRandom) -> Self {
        Self::new(
            id,
            AMBIENT,
            volume,
            pitch,
            random,
            false,
            0,
            Attenuation::None,
            [0.0; 3],
            true,
        )
    }
    /// `SimpleSoundInstance.forAmbientAddition`.
    pub fn for_ambient_addition(id: &str, random: LegacyRandom) -> Self {
        Self::for_local_ambience(id, 1.0, 1.0, random)
    }
    /// `SimpleSoundInstance.forAmbientMood`.
    pub fn for_ambient_mood(id: &str, random: LegacyRandom, at: [f64; 3]) -> Self {
        Self::new(
            id,
            AMBIENT,
            1.0,
            1.0,
            random,
            false,
            0,
            Attenuation::Linear,
            at,
            false,
        )
    }
    /// A relative looping ambient instance (the loop sound constructors).
    fn ambient_loop(id: &str, random: LegacyRandom, looping: bool, ticking: Ticking) -> Self {
        let mut s = Self::new(
            id,
            AMBIENT,
            1.0,
            1.0,
            random,
            looping,
            0,
            Attenuation::Linear,
            [0.0; 3],
            true,
        );
        s.ticking = ticking;
        s
    }
    /// `new BiomeAmbientSoundsHandler.LoopSoundInstance(event)`.
    pub fn biome_loop(id: &str, random: LegacyRandom) -> Self {
        Self::ambient_loop(
            id,
            random,
            true,
            Ticking::BiomeLoop {
                fade: 0,
                direction: 0,
            },
        )
    }
    /// `UnderLiquidAmbientSoundInstance.underwater(player)`.
    pub fn under_liquid_loop(id: &str, random: LegacyRandom) -> Self {
        Self::ambient_loop(id, random, true, Ticking::UnderLiquidLoop { fade: 0 })
    }
    /// `new UnderLiquidSubSound(player, event)`.
    pub fn under_liquid_sub(id: &str, random: LegacyRandom) -> Self {
        Self::ambient_loop(id, random, false, Ticking::UnderLiquidSub)
    }
    /// `new MinecartSoundInstance(minecart)`: silent until the cart rolls.
    pub fn minecart(cart: &CartSound, random: LegacyRandom) -> Self {
        let [x, y, z] = cart.position;
        let mut s = Self::new(
            "minecraft:entity.minecart.riding",
            NEUTRAL,
            0.0,
            1.0,
            random,
            true,
            0,
            Attenuation::Linear,
            [
                f64::from(x as f32),
                f64::from(y as f32),
                f64::from(z as f32),
            ],
            false,
        );
        s.can_start_silent = true;
        s.ticking = Ticking::Minecart { cart: cart.id };
        s
    }
    /// `new RidingMinecartSoundInstance(player, minecart, underwater, event, 0, 0.75, 1)`.
    pub fn riding_minecart(cart: &CartSound, underwater: bool, random: LegacyRandom) -> Self {
        let id = if underwater {
            "minecraft:entity.minecart.inside.underwater"
        } else {
            "minecraft:entity.minecart.inside"
        };
        let mut s = Self::new(
            id,
            NEUTRAL,
            0.0,
            1.0,
            random,
            true,
            0,
            Attenuation::None,
            cart.position,
            false,
        );
        s.can_start_silent = true;
        s.ticking = Ticking::RidingMinecart {
            cart: cart.id,
            underwater,
        };
        s
    }
    /// `new BeeFlyingSoundInstance(bee)` (or `BeeAggressiveSoundInstance`):
    /// silent until the bee moves.
    pub fn bee(bee: &MobSound, aggressive: bool, random: LegacyRandom) -> Self {
        let id = if aggressive {
            "minecraft:entity.bee.loop_aggressive"
        } else {
            "minecraft:entity.bee.loop"
        };
        let [x, y, z] = bee.position;
        let mut s = Self::new(
            id,
            NEUTRAL,
            0.0,
            1.0,
            random,
            true,
            0,
            Attenuation::Linear,
            [
                f64::from(x as f32),
                f64::from(y as f32),
                f64::from(z as f32),
            ],
            false,
        );
        s.can_start_silent = true;
        s.ticking = Ticking::Bee {
            bee: bee.id,
            aggressive,
        };
        s
    }
    /// `new GuardianAttackSoundInstance(guardian)`: the guardian's own volume
    /// and pitch until its first tick, heard everywhere.
    pub fn guardian_attack(guardian: &MobSound, random: LegacyRandom) -> Self {
        let id = "minecraft:entity.guardian.attack";
        let mut s = Self::new(
            id,
            crate::settings::sound_source(id),
            1.0,
            1.0,
            random,
            true,
            0,
            Attenuation::None,
            guardian.position,
            false,
        );
        s.ticking = Ticking::GuardianAttack {
            guardian: guardian.id,
        };
        s
    }
    /// `new ElytraOnPlayerSoundInstance(player)`: looping at volume 0.1 until
    /// its first tick.
    pub fn elytra(position: [f64; 3], random: LegacyRandom) -> Self {
        let mut s = Self::new(
            "minecraft:item.elytra.flying",
            PLAYERS,
            0.1,
            1.0,
            random,
            true,
            0,
            Attenuation::Linear,
            position,
            false,
        );
        s.ticking = Ticking::Elytra { time: 0 };
        s
    }
    pub fn tickable(&self) -> bool {
        self.ticking != Ticking::No
    }
    /// `AbstractSoundInstance.getVolume` (after `getOrResolve`).
    pub fn get_volume(&mut self) -> f32 {
        let sample = match &self.sound {
            Some(s) => s.volume.sample(&mut self.random),
            None => 1.0,
        };
        self.volume * sample
    }
    /// `AbstractSoundInstance.getPitch`.
    pub fn get_pitch(&mut self) -> f32 {
        let sample = match &self.sound {
            Some(s) => s.pitch.sample(&mut self.random),
            None => 1.0,
        };
        self.pitch * sample
    }
    /// `AbstractTickableSoundInstance.stop`.
    fn stop(&mut self) {
        self.stopped = true;
        self.looping = false;
    }
    /// `TickableSoundInstance.tick`.
    pub fn tick(&mut self, ctx: &TickContext) {
        match &mut self.ticking {
            Ticking::No => {}
            Ticking::BiomeLoop { fade, direction } => {
                let stop = *fade < 0;
                *fade += *direction;
                self.volume = (*fade as f32 / 40.0).clamp(0.0, 1.0);
                if stop {
                    self.stop();
                }
            }
            Ticking::UnderLiquidLoop { fade } => {
                if !ctx.player_removed && *fade >= 0 {
                    if ctx.player_underwater {
                        *fade += 1;
                    } else {
                        *fade -= 2;
                    }
                    *fade = (*fade).min(40);
                    self.volume = 0.0f32.max((*fade as f32 / 40.0).min(1.0));
                } else {
                    self.stop();
                }
            }
            Ticking::UnderLiquidSub => {
                if ctx.player_removed || !ctx.player_underwater {
                    self.stop();
                }
            }
            Ticking::Minecart { cart } => {
                let id = *cart;
                let Some(cart) = ctx.carts.iter().find(|c| c.id == id) else {
                    self.stop();
                    return;
                };
                let [x, y, z] = cart.position;
                self.position = [
                    f64::from(x as f32),
                    f64::from(y as f32),
                    f64::from(z as f32),
                ];
                // Its own `pitch` field shadows the instance's, which stays 1.
                let speed = cart.speed as f32;
                self.volume = if speed >= 0.01 && !ctx.frozen {
                    speed.clamp(0.0, 0.5) * 0.7
                } else {
                    0.0
                };
            }
            Ticking::Bee { bee, aggressive } => {
                let (id, aggressive) = (*bee, *aggressive);
                // canPlaySound, then shouldSwitchSounds: the other kind of
                // instance is started in this one's place.
                let Some(bee) = ctx
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .filter(|m| !m.silent && m.angry == aggressive)
                else {
                    self.stop();
                    return;
                };
                let [x, y, z] = bee.position;
                self.position = [
                    f64::from(x as f32),
                    f64::from(y as f32),
                    f64::from(z as f32),
                ];
                let (min, max) = if bee.baby { (1.1, 1.5) } else { (0.7, 1.1) };
                let speed = bee.speed as f32;
                if speed >= 0.01 {
                    // Mth.lerp(Mth.clamp(speed, min, max), min, max).
                    self.pitch = min + speed.clamp(min, max) * (max - min);
                    self.volume = speed.clamp(0.0, 0.5) * 1.2;
                } else {
                    self.pitch = 0.0;
                    self.volume = 0.0;
                }
            }
            Ticking::Elytra { time } => {
                *time += 1;
                let time = *time;
                if ctx.player_removed || (time > 20 && !ctx.player_fall_flying) {
                    self.stop();
                    return;
                }
                let [x, y, z] = ctx.player_position;
                self.position = [
                    f64::from(x as f32),
                    f64::from(y as f32),
                    f64::from(z as f32),
                ];
                let speed = ctx.player_speed_sqr as f32;
                self.volume = if speed >= 1.0e-7 {
                    (speed / 4.0).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // Silent for a second, then a second's fade in.
                if time < 20 {
                    self.volume = 0.0;
                } else if time < 40 {
                    self.volume *= (time - 20) as f32 / 20.0;
                }
                self.pitch = if self.volume > 0.8 {
                    1.0 + (self.volume - 0.8)
                } else {
                    1.0
                };
            }
            Ticking::GuardianAttack { guardian } => {
                let id = *guardian;
                // The client's guardian never has a target of its own, so
                // only its removal or silence ends the loop.
                let Some(guardian) = ctx.mobs.iter().find(|m| m.id == id && !m.silent) else {
                    self.stop();
                    return;
                };
                let [x, y, z] = guardian.position;
                self.position = [
                    f64::from(x as f32),
                    f64::from(y as f32),
                    f64::from(z as f32),
                ];
                let scale = guardian.attack_scale;
                self.volume = 0.0 + 1.0 * scale * scale;
                self.pitch = 0.7 + 0.5 * scale;
            }
            Ticking::RidingMinecart { cart, underwater } => {
                let (id, underwater) = (*cart, *underwater);
                let found = ctx.carts.iter().find(|c| c.id == id);
                let Some(cart) = found.filter(|c| ctx.vehicle == Some(c.id)) else {
                    self.stop();
                    return;
                };
                if underwater != ctx.player_underwater {
                    self.volume = 0.0;
                } else {
                    // Mth.clampedLerp(speed, 0, 0.75), amplified by 1.
                    let speed = cart.speed as f32;
                    self.volume = if speed >= 0.01 {
                        if speed > 1.0 { 0.75 } else { speed * 0.75 }
                    } else {
                        0.0
                    };
                    self.position = cart.position;
                }
            }
        }
    }
    /// `LoopSoundInstance.fadeOut`.
    pub fn fade_out(&mut self) {
        if let Ticking::BiomeLoop { fade, direction } = &mut self.ticking {
            *fade = (*fade).min(40);
            *direction = -1;
        }
    }
    /// `LoopSoundInstance.fadeIn`.
    pub fn fade_in(&mut self) {
        if let Ticking::BiomeLoop { fade, direction } = &mut self.ticking {
            *fade = 0.max(*fade);
            *direction = 1;
        }
    }
}
/// A sound instance handle.
pub type InstanceId = u64;
/// A backend channel (an OpenAL source).
pub type ChannelId = u64;
/// The OpenAL side: `Channel` operations, all applied immediately.
pub trait Backend {
    /// `Channel.create`.
    fn create(&mut self, streaming: bool) -> Option<ChannelId>;
    /// `Channel.destroy`.
    fn destroy(&mut self, channel: ChannelId);
    fn set_pitch(&mut self, channel: ChannelId, pitch: f32);
    fn set_volume(&mut self, channel: ChannelId, volume: f32);
    /// `Some(max distance)` for `linearAttenuation`, None for `disableAttenuation`.
    fn set_attenuation(&mut self, channel: ChannelId, linear: Option<f32>);
    fn set_looping(&mut self, channel: ChannelId, looping: bool);
    fn set_position(&mut self, channel: ChannelId, position: [f64; 3]);
    fn set_relative(&mut self, channel: ChannelId, relative: bool);
    /// Attach the sound's buffer or stream (when loaded) and play it.
    fn start(&mut self, channel: ChannelId, sound: &Sound, looping: bool);
    fn stop(&mut self, channel: ChannelId);
    fn pause(&mut self, channel: ChannelId);
    fn unpause(&mut self, channel: ChannelId);
    /// `Channel.stopped`.
    fn stopped(&mut self, channel: ChannelId) -> bool;
}
/// A `SoundEventListener.onPlaySound` call.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub identifier: String,
    pub subtitle: Option<String>,
    pub position: [f64; 3],
    pub relative: bool,
    pub looping: bool,
    pub range: f32,
}
/// `ChannelAccess.ChannelHandle`.
#[derive(Clone, Copy, Debug)]
struct Handle {
    channel: Option<ChannelId>,
    streaming: bool,
    stopped: bool,
}
pub struct SoundEngine {
    pub registry: SoundRegistry,
    /// `loaded`: the library opened.
    pub loaded: bool,
    tick_count: i32,
    instances: HashMap<InstanceId, SoundInstance>,
    /// Instances their owner still refers to.
    retained: HashMap<InstanceId, ()>,
    next_instance: InstanceId,
    /// `instanceToChannel` (insertion order).
    instance_to_channel: Vec<(InstanceId, u64)>,
    handles: HashMap<u64, Handle>,
    next_handle: u64,
    /// `instanceBySource`.
    instance_by_source: Vec<(usize, InstanceId)>,
    gain_by_source: [f32; 11],
    ticking: Vec<InstanceId>,
    /// `queuedSounds` (insertion order).
    queued: Vec<(InstanceId, i32)>,
    delete_time: HashMap<InstanceId, i32>,
    queued_tickable: Vec<InstanceId>,
    used: [usize; 2],
    pub limits: [usize; 2],
    /// `Options.getSoundSourceVolume` per source (index 0 is master).
    pub source_volumes: [f32; 11],
    /// A `SoundEventListener` is registered.
    pub listening: bool,
    pub notifications: Vec<Notification>,
}
fn handle_channel(h: &Handle) -> Option<ChannelId> {
    h.channel
}
impl SoundEngine {
    pub fn new(registry: SoundRegistry) -> Self {
        Self {
            registry,
            loaded: true,
            tick_count: 0,
            instances: HashMap::new(),
            retained: HashMap::new(),
            next_instance: 1,
            instance_to_channel: Vec::new(),
            handles: HashMap::new(),
            next_handle: 1,
            instance_by_source: Vec::new(),
            gain_by_source: [1.0; 11],
            ticking: Vec::new(),
            queued: Vec::new(),
            delete_time: HashMap::new(),
            queued_tickable: Vec::new(),
            used: [0, 0],
            limits: [STATIC_CHANNELS, STREAMING_CHANNELS],
            source_volumes: [1.0; 11],
            listening: false,
            notifications: Vec::new(),
        }
    }
    /// Hand an instance to the engine. A retained instance stays until
    /// `release`; others go once the engine no longer refers to them.
    pub fn add(&mut self, instance: SoundInstance, retain: bool) -> InstanceId {
        let id = self.next_instance;
        self.next_instance += 1;
        self.instances.insert(id, instance);
        if retain {
            self.retained.insert(id, ());
        }
        id
    }
    /// The owner drops its reference.
    pub fn release(&mut self, id: InstanceId) {
        self.retained.remove(&id);
        self.collect(id);
    }
    fn referenced(&self, id: InstanceId) -> bool {
        self.retained.contains_key(&id)
            || self.instance_to_channel.iter().any(|(i, _)| *i == id)
            || self.queued.iter().any(|(i, _)| *i == id)
            || self.queued_tickable.contains(&id)
            || self.ticking.contains(&id)
    }
    fn collect(&mut self, id: InstanceId) {
        if !self.referenced(id) {
            self.instances.remove(&id);
        }
    }
    /// Table sizes, for soak tests watching what the engine holds.
    pub fn census(&self) -> serde_json::Value {
        // The sounds holding channels, most first: what a soak test hears.
        let mut events: HashMap<&str, usize> = HashMap::new();
        for (id, _) in &self.instance_to_channel {
            if let Some(i) = self.instances.get(id) {
                *events.entry(i.identifier.as_str()).or_default() += 1;
            }
        }
        let mut events: Vec<(&str, usize)> = events.into_iter().collect();
        events.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        events.truncate(12);
        serde_json::json!({
            "events": events,
            "instances": self.instances.len(),
            "playing": self.instance_to_channel.len(),
            "handles": self.handles.len(),
            "ticking": self.ticking.len(),
            "queued": self.queued.len(),
            "used": self.used,
        })
    }
    pub fn instance(&self, id: InstanceId) -> Option<&SoundInstance> {
        self.instances.get(&id)
    }
    pub fn instance_mut(&mut self, id: InstanceId) -> Option<&mut SoundInstance> {
        self.instances.get_mut(&id)
    }
    pub fn tick_count(&self) -> i32 {
        self.tick_count
    }
    /// `Options.getFinalSoundSourceVolume`.
    pub fn final_source_volume(&self, source: usize) -> f32 {
        if source == MASTER {
            self.source_volumes[MASTER]
        } else {
            self.source_volumes[source] * self.source_volumes[MASTER]
        }
    }
    /// `calculateVolume(volume, source)`.
    pub fn calculate_volume(&self, volume: f32, source: usize) -> f32 {
        volume.clamp(0.0, 1.0)
            * self.final_source_volume(source).clamp(0.0, 1.0)
            * self.gain_by_source[source]
    }
    /// `calculatePitch`.
    pub fn calculate_pitch(pitch: f32) -> f32 {
        pitch.clamp(0.5, 2.0)
    }
    fn channel_of(&self, id: InstanceId) -> Option<u64> {
        self.instance_to_channel
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, h)| *h)
    }
    /// `refreshCategoryVolume`.
    pub fn refresh_category_volume(&mut self, source: usize, backend: &mut impl Backend) {
        if !self.loaded {
            return;
        }
        for (id, handle) in self.instance_to_channel.clone() {
            let Some(instance) = self.instances.get_mut(&id) else {
                continue;
            };
            if source == instance.source || source == MASTER {
                let (v, s) = (instance.get_volume(), instance.source);
                let volume = self.calculate_volume(v, s);
                if let Some(c) = self.handles.get(&handle).and_then(handle_channel) {
                    backend.set_volume(c, volume);
                }
            }
        }
    }
    /// `updateCategoryVolume`.
    pub fn update_category_volume(&mut self, source: usize, gain: f32, backend: &mut impl Backend) {
        self.gain_by_source[source] = gain.clamp(0.0, 1.0);
        self.refresh_category_volume(source, backend);
    }
    pub fn category_gain(&self, source: usize) -> f32 {
        self.gain_by_source[source]
    }
    /// `stop(SoundInstance)`.
    pub fn stop(&mut self, id: InstanceId, backend: &mut impl Backend) {
        if !self.loaded {
            return;
        }
        if let Some(handle) = self.channel_of(id)
            && let Some(c) = self.handles.get(&handle).and_then(handle_channel)
        {
            backend.stop(c);
        }
    }
    /// `ChannelHandle.release`.
    fn release_handle(&mut self, handle: u64, backend: &mut impl Backend) {
        if let Some(h) = self.handles.get_mut(&handle) {
            h.stopped = true;
            if let Some(c) = h.channel.take() {
                self.used[usize::from(h.streaming)] -= 1;
                backend.destroy(c);
            }
        }
    }
    /// `stopAll`.
    pub fn stop_all(&mut self, backend: &mut impl Backend) {
        if !self.loaded {
            return;
        }
        let ids: Vec<InstanceId> = self.instances.keys().copied().collect();
        self.instance_to_channel.clear();
        let handles: Vec<u64> = self.handles.keys().copied().collect();
        for h in handles {
            self.release_handle(h, backend);
        }
        self.handles.clear();
        self.queued.clear();
        self.ticking.clear();
        self.instance_by_source.clear();
        self.delete_time.clear();
        self.queued_tickable.clear();
        self.gain_by_source = [1.0; 11];
        for id in ids {
            self.collect(id);
        }
    }
    /// `stop(Identifier sound, SoundSource source)`.
    pub fn stop_matching(
        &mut self,
        sound: Option<&str>,
        source: Option<usize>,
        backend: &mut impl Backend,
    ) {
        let matches = |engine: &Self, id: InstanceId| {
            sound.is_none_or(|s| engine.instances.get(&id).is_some_and(|i| i.identifier == s))
        };
        if let Some(source) = source {
            let ids: Vec<InstanceId> = self
                .instance_by_source
                .iter()
                .filter(|(s, _)| *s == source)
                .map(|(_, i)| *i)
                .collect();
            for id in ids {
                if matches(self, id) {
                    self.stop(id, backend);
                }
            }
        } else if sound.is_none() {
            self.stop_all(backend);
        } else {
            let ids: Vec<InstanceId> = self.instance_to_channel.iter().map(|(i, _)| *i).collect();
            for id in ids {
                if matches(self, id) {
                    self.stop(id, backend);
                }
            }
        }
    }
    /// `getRange`.
    fn range(relative: bool, attenuation: Attenuation, distance: f32) -> f32 {
        if !relative && attenuation != Attenuation::None {
            distance
        } else {
            f32::INFINITY
        }
    }
    fn notify(&mut self, id: InstanceId, subtitle: Option<String>, distance: f32) {
        let Some(i) = self.instances.get(&id) else {
            return;
        };
        self.notifications.push(Notification {
            identifier: i.identifier.clone(),
            subtitle,
            position: i.position,
            relative: i.relative,
            looping: i.looping,
            range: Self::range(i.relative, i.attenuation, distance),
        });
    }
    /// `getOrResolve`.
    pub fn resolve(&mut self, id: InstanceId) {
        let Some(instance) = self.instances.get_mut(&id) else {
            return;
        };
        if instance.identifier == INTENTIONALLY_EMPTY {
            instance.sound = Some(Sound::intentionally_empty());
            instance.event = Some(None);
            return;
        }
        match self.registry.get(&instance.identifier) {
            None => {
                instance.event = None;
                instance.sound = Some(Sound::empty());
            }
            Some(event) => {
                instance.event = Some(event.subtitle.clone());
                instance.sound = self
                    .registry
                    .sound(&instance.identifier, &mut instance.random);
            }
        }
    }
    /// `play(SoundInstance)`.
    pub fn play(&mut self, id: InstanceId, backend: &mut impl Backend) -> PlayResult {
        let result = self.play_inner(id, backend);
        self.collect(id);
        result
    }
    /// Add and play an instance.
    pub fn play_new(
        &mut self,
        instance: SoundInstance,
        retain: bool,
        backend: &mut impl Backend,
    ) -> (InstanceId, PlayResult) {
        let id = self.add(instance, retain);
        (id, self.play(id, backend))
    }
    fn play_inner(&mut self, id: InstanceId, backend: &mut impl Backend) -> PlayResult {
        if !self.loaded || !self.instances.contains_key(&id) {
            return PlayResult::NotStarted;
        }
        self.resolve(id);
        let instance = &self.instances[&id];
        let Some(subtitle) = instance.event.clone() else {
            return PlayResult::NotStarted;
        };
        let sound = instance.sound.clone().unwrap_or_else(Sound::empty);
        if sound.sentinel != Sentinel::None {
            return PlayResult::NotStarted;
        }
        let instance = self.instances.get_mut(&id).expect("present");
        let instance_volume = instance.get_volume();
        let attenuation_distance = sound.attenuation_distance(instance_volume);
        if self.listening {
            self.notify(id, subtitle, attenuation_distance);
        }
        let instance = &self.instances[&id];
        let source = instance.source;
        let volume = self.calculate_volume(instance_volume, source);
        let mut silently = false;
        if volume == 0.0 {
            if !instance.can_start_silent && source != MUSIC {
                return PlayResult::NotStarted;
            }
            silently = true;
        }
        let position = instance.position;
        let looping = instance.looping && instance.delay <= 0;
        let streaming = sound.stream;
        let pool = usize::from(streaming);
        let channel = if self.used[pool] >= self.limits[pool] {
            None
        } else {
            backend.create(streaming)
        };
        let Some(channel) = channel else {
            return PlayResult::NotStarted;
        };
        self.used[pool] += 1;
        let handle = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(
            handle,
            Handle {
                channel: Some(channel),
                streaming,
                stopped: false,
            },
        );
        self.delete_time.insert(id, self.tick_count + 20);
        if let Some(entry) = self.instance_to_channel.iter_mut().find(|(i, _)| *i == id) {
            entry.1 = handle;
        } else {
            self.instance_to_channel.push((id, handle));
        }
        if !self.instance_by_source.contains(&(source, id)) {
            self.instance_by_source.push((source, id));
        }
        let instance = self.instances.get_mut(&id).expect("present");
        let pitch = Self::calculate_pitch(instance.get_pitch());
        let (attenuation, relative, tickable) =
            (instance.attenuation, instance.relative, instance.tickable());
        backend.set_pitch(channel, pitch);
        backend.set_volume(channel, volume);
        backend.set_attenuation(
            channel,
            (attenuation == Attenuation::Linear).then_some(attenuation_distance),
        );
        backend.set_looping(channel, looping && !streaming);
        backend.set_position(channel, position);
        backend.set_relative(channel, relative);
        backend.start(channel, &sound, looping);
        if tickable {
            self.ticking.push(id);
        }
        if silently {
            PlayResult::StartedSilently
        } else {
            PlayResult::Started
        }
    }
    /// `queueTickingSound`.
    pub fn queue_ticking(&mut self, id: InstanceId) {
        self.queued_tickable.push(id);
    }
    /// `playDelayed`.
    pub fn play_delayed(&mut self, id: InstanceId, delay: i32) {
        let at = self.tick_count + delay;
        if let Some(entry) = self.queued.iter_mut().find(|(i, _)| *i == id) {
            entry.1 = at;
        } else {
            self.queued.push((id, at));
        }
    }
    /// `isActive`.
    pub fn is_active(&self, id: InstanceId) -> bool {
        self.loaded && self.channel_of(id).is_some()
    }
    /// `tick(paused)`, then `ChannelAccess.scheduleTick`.
    pub fn tick(&mut self, paused: bool, ctx: &TickContext, backend: &mut impl Backend) {
        if !paused {
            self.tick_in_game(ctx, backend);
        } else {
            self.tick_music_when_paused();
        }
        self.schedule_tick(backend);
    }
    fn schedule_tick(&mut self, backend: &mut impl Backend) {
        let live: Vec<(u64, ChannelId)> = self
            .handles
            .iter()
            .filter_map(|(h, handle)| handle.channel.map(|c| (*h, c)))
            .collect();
        for (h, c) in live {
            if backend.stopped(c) {
                self.release_handle(h, backend);
            }
        }
        // Released handles nobody maps any more.
        let mapped: Vec<u64> = self.instance_to_channel.iter().map(|(_, h)| *h).collect();
        self.handles
            .retain(|h, handle| handle.channel.is_some() || mapped.contains(h));
    }
    fn handle_stopped(&self, handle: u64) -> bool {
        self.handles.get(&handle).is_none_or(|h| h.stopped)
    }
    fn tick_in_game(&mut self, ctx: &TickContext, backend: &mut impl Backend) {
        self.tick_count += 1;
        for id in std::mem::take(&mut self.queued_tickable) {
            self.play(id, backend);
        }
        for id in self.ticking.clone() {
            let Some(instance) = self.instances.get_mut(&id) else {
                continue;
            };
            instance.tick(ctx);
            if instance.stopped {
                self.stop(id, backend);
                continue;
            }
            let (v, source) = (instance.get_volume(), instance.source);
            let pitch = Self::calculate_pitch(instance.get_pitch());
            let position = instance.position;
            let volume = self.calculate_volume(v, source);
            let Some(handle) = self.channel_of(id) else {
                continue;
            };
            if let Some(c) = self.handles.get(&handle).and_then(handle_channel) {
                backend.set_volume(c, volume);
                backend.set_pitch(c, pitch);
                backend.set_position(c, position);
            }
            let instance = self.instances.get_mut(&id).expect("present");
            if instance.looping
                && self.tick_count % 10 == 0
                && volume > 0.0
                && self.listening
                && let (Some(sound), Some(subtitle)) =
                    (instance.sound.clone(), instance.event.clone())
            {
                let distance = sound.attenuation_distance(instance.get_volume());
                self.notify(id, subtitle, distance);
            }
        }
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.instance_to_channel.len() {
            let (id, handle) = self.instance_to_channel[index];
            if self.handle_stopped(handle)
                && self.delete_time.get(&id).copied().unwrap_or(0) <= self.tick_count
            {
                if let Some(i) = self.instances.get(&id)
                    && i.looping
                    && i.delay > 0
                {
                    let at = self.tick_count + i.delay;
                    if let Some(entry) = self.queued.iter_mut().find(|(q, _)| *q == id) {
                        entry.1 = at;
                    } else {
                        self.queued.push((id, at));
                    }
                }
                self.instance_to_channel.remove(index);
                self.handles.remove(&handle);
                self.delete_time.remove(&id);
                let source = self.instances.get(&id).map(|i| i.source);
                self.instance_by_source
                    .retain(|e| Some(e.0) != source || e.1 != id);
                if let Some(p) = self.ticking.iter().position(|t| *t == id) {
                    self.ticking.remove(p);
                }
                removed.push(id);
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.queued.len() {
            let (id, at) = self.queued[index];
            if self.tick_count >= at {
                self.queued.remove(index);
                if let Some(i) = self.instances.get_mut(&id)
                    && i.tickable()
                {
                    i.tick(ctx);
                }
                self.play(id, backend);
            } else {
                index += 1;
            }
        }
        for id in removed {
            self.collect(id);
        }
    }
    fn tick_music_when_paused(&mut self) {
        let mut index = 0;
        while index < self.instance_to_channel.len() {
            let (id, handle) = self.instance_to_channel[index];
            let music = self.instances.get(&id).is_some_and(|i| i.source == MUSIC);
            if music && self.handle_stopped(handle) {
                self.instance_to_channel.remove(index);
                self.handles.remove(&handle);
                self.delete_time.remove(&id);
                self.instance_by_source
                    .retain(|e| e.0 != MUSIC || e.1 != id);
                self.collect(id);
            } else {
                index += 1;
            }
        }
    }
    /// `pauseAllExcept`.
    pub fn pause_all_except(&mut self, sources: &[usize], backend: &mut impl Backend) {
        if !self.loaded {
            return;
        }
        for (id, handle) in self.instance_to_channel.clone() {
            let source = self.instances.get(&id).map_or(MASTER, |i| i.source);
            if !sources.contains(&source)
                && let Some(c) = self.handles.get(&handle).and_then(handle_channel)
            {
                backend.pause(c);
            }
        }
    }
    /// `resume`.
    pub fn resume(&mut self, backend: &mut impl Backend) {
        if !self.loaded {
            return;
        }
        let channels: Vec<ChannelId> = self.handles.values().filter_map(handle_channel).collect();
        for c in channels {
            backend.unpause(c);
        }
    }
    /// Channels in use (static, streaming).
    pub fn used_channels(&self) -> [usize; 2] {
        self.used
    }
    /// The live channel of an instance.
    pub fn channel(&self, id: InstanceId) -> Option<ChannelId> {
        self.channel_of(id)
            .and_then(|h| self.handles.get(&h))
            .and_then(handle_channel)
    }
    /// Instances with a channel, in insertion order.
    pub fn active_instances(&self) -> Vec<InstanceId> {
        self.instance_to_channel.iter().map(|(i, _)| *i).collect()
    }
    /// A fresh unseeded random (`RandomSource.create()`).
    pub fn unseeded() -> LegacyRandom {
        LegacyRandom::new(unique_seed())
    }
}
/// `RandomSupport.generateUniqueSeed`.
pub fn unique_seed() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static UNIQUIFIER: AtomicI64 = AtomicI64::new(8_682_522_807_148_012);
    let mut current = UNIQUIFIER.load(Ordering::Relaxed);
    let next = loop {
        let next = current.wrapping_mul(1_181_783_497_276_652_981);
        match UNIQUIFIER.compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break next,
            Err(v) => current = v,
        }
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64);
    next ^ nanos
}
/// The `SoundManager` calls game code makes.
pub trait Sounds {
    fn engine(&mut self) -> &mut SoundEngine;
    /// `SoundManager.play` of a new instance (`retain`: the caller keeps it).
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult);
    /// `SoundManager.play` of an instance the caller holds.
    fn play_id(&mut self, id: InstanceId) -> PlayResult;
    /// `SoundManager.stop(SoundInstance)`.
    fn stop_instance(&mut self, id: InstanceId);
    /// `SoundManager.isActive`.
    fn is_active(&mut self, id: InstanceId) -> bool {
        self.engine().is_active(id)
    }
    /// `SoundManager.updateCategoryVolume`.
    fn update_category_volume(&mut self, source: usize, gain: f32);
    /// The caller drops its instance.
    fn release(&mut self, id: InstanceId) {
        self.engine().release(id);
    }
}
/// An engine with its backend.
pub struct Bound<'a, B: Backend> {
    pub engine: &'a mut SoundEngine,
    pub backend: &'a mut B,
}
impl<B: Backend> Sounds for Bound<'_, B> {
    fn engine(&mut self) -> &mut SoundEngine {
        self.engine
    }
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult) {
        self.engine.play_new(instance, retain, self.backend)
    }
    fn play_id(&mut self, id: InstanceId) -> PlayResult {
        self.engine.play(id, self.backend)
    }
    fn stop_instance(&mut self, id: InstanceId) {
        self.engine.stop(id, self.backend);
    }
    fn update_category_volume(&mut self, source: usize, gain: f32) {
        self.engine
            .update_category_volume(source, gain, self.backend);
    }
}
#[cfg(any(test, feature = "test-support"))]
pub mod test_backend {
    use super::{Backend, ChannelId};
    use crate::sound_events::Sound;
    use std::collections::HashMap;
    /// Channels that play until stopped.
    #[derive(Default)]
    pub struct Null {
        next: ChannelId,
        pub stopped: HashMap<ChannelId, bool>,
        pub volume: HashMap<ChannelId, f32>,
    }
    impl Backend for Null {
        fn create(&mut self, _streaming: bool) -> Option<ChannelId> {
            self.next += 1;
            self.stopped.insert(self.next, false);
            Some(self.next)
        }
        fn destroy(&mut self, c: ChannelId) {
            self.stopped.remove(&c);
        }
        fn set_pitch(&mut self, _: ChannelId, _: f32) {}
        fn set_volume(&mut self, c: ChannelId, v: f32) {
            self.volume.insert(c, v);
        }
        fn set_attenuation(&mut self, _: ChannelId, _: Option<f32>) {}
        fn set_looping(&mut self, _: ChannelId, _: bool) {}
        fn set_position(&mut self, _: ChannelId, _: [f64; 3]) {}
        fn set_relative(&mut self, _: ChannelId, _: bool) {}
        fn start(&mut self, _: ChannelId, _: &Sound, _: bool) {}
        fn stop(&mut self, c: ChannelId) {
            self.stopped.insert(c, true);
        }
        fn pause(&mut self, _: ChannelId) {}
        fn unpause(&mut self, _: ChannelId) {}
        fn stopped(&mut self, c: ChannelId) -> bool {
            self.stopped.get(&c).copied().unwrap_or(true)
        }
    }
}
