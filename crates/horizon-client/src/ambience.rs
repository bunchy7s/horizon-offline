//! The player's ambient sound handlers: `BiomeAmbientSoundsHandler` (biome
//! loops with 40-tick cross-fades, random additions, cave mood) driven by the
//! `minecraft:audio/ambient_sounds` attribute, `UnderwaterAmbientSoundHandler`
//! and `LocalPlayer.updateIsUnderwater` (enter/exit sounds and the
//! `UnderLiquidAmbientSoundInstance` loop), played through the sound engine.
use crate::sound_engine::{AMBIENT, InstanceId, PLAYERS, SoundEngine, SoundInstance, Sounds};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

/// `AmbientMoodSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct Mood {
    pub sound: String,
    pub tick_delay: i32,
    pub block_search_extent: i32,
    pub offset: f64,
}
/// `AmbientSounds`: the attribute value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientSounds {
    pub r#loop: Option<String>,
    pub mood: Option<Mood>,
    /// (sound, tick chance)
    pub additions: Vec<(String, f64)>,
}
impl AmbientSounds {
    fn parse(v: &Value) -> Self {
        let mood = &v["mood"];
        let addition =
            |a: &Value| Some((a["sound"].as_str()?.to_owned(), a["tick_chance"].as_f64()?));
        Self {
            r#loop: v["loop"].as_str().map(str::to_owned),
            mood: mood["sound"].as_str().map(|sound| Mood {
                sound: sound.to_owned(),
                tick_delay: mood["tick_delay"].as_i64().unwrap_or(6000) as i32,
                block_search_extent: mood["block_search_extent"].as_i64().unwrap_or(8) as i32,
                offset: mood["offset"].as_f64().unwrap_or(2.0),
            }),
            // A single addition or a list of them.
            additions: match &v["additions"] {
                Value::Array(list) => list.iter().filter_map(addition).collect(),
                single => addition(single).into_iter().collect(),
            },
        }
    }
}
/// Ambient sound attributes by dimension type and biome.
#[derive(Clone, Debug, Default)]
pub struct AmbientTables {
    dimensions: HashMap<String, AmbientSounds>,
    biomes: HashMap<String, AmbientSounds>,
}
impl AmbientTables {
    pub fn load(root: &Path) -> Self {
        let read = |dir: &str| -> HashMap<String, AmbientSounds> {
            let Ok(folder) =
                horizon_world::json_folder::read(&root.join("data/minecraft").join(dir))
            else {
                return HashMap::new();
            };
            folder
                .files
                .iter()
                .filter_map(|file| {
                    let json: Value = serde_json::from_slice(file.bytes().ok()?).ok()?;
                    let v = &json["attributes"]["minecraft:audio/ambient_sounds"];
                    (!v.is_null())
                        .then(|| (format!("minecraft:{}", file.name), AmbientSounds::parse(v)))
                })
                .collect()
        };
        Self {
            dimensions: read("dimension_type"),
            biomes: read("worldgen/biome"),
        }
    }
    /// `EnvironmentAttributes.AMBIENT_SOUNDS` at a position.
    pub fn get(&self, dimension: &str, biome: &str) -> AmbientSounds {
        self.biomes
            .get(biome)
            .or_else(|| self.dimensions.get(dimension))
            .cloned()
            .unwrap_or_default()
    }
}
/// What the handlers read about the player and level each tick.
pub struct AmbientContext<'a> {
    /// `AMBIENT_SOUNDS` at the player's position.
    pub sounds: &'a AmbientSounds,
    /// The player's position (feet) and eye height.
    pub position: [f64; 3],
    pub eye_y: f64,
    /// `isEyeInFluid(WATER)` this tick (`Entity.updateIsUnderwater`).
    pub underwater: bool,
    pub spectator: bool,
    /// `Level.getBrightness` (sky, block) at a block.
    pub light: &'a dyn Fn([i32; 3]) -> (u8, u8),
    /// The bubble column in the player's box (its `drag` property), from
    /// `bubble_column_in`.
    pub bubble_column: Option<bool>,
}
/// `BubbleColumnAmbientSoundHandler`'s lookup: the first bubble column
/// (x fastest, then y, then z) in the player's bounding box inflated by
/// (0, -0.4F, 0) and deflated by 1.0E-6, as `drag_down`. `state` gives the
/// bubble column at a block (None for anything else).
pub fn bubble_column_in(bb: [f64; 6], state: impl Fn([i32; 3]) -> Option<bool>) -> Option<bool> {
    // AABB keeps min <= max on every construction.
    let make = |a: [f64; 6]| {
        [
            a[0].min(a[3]),
            a[1].min(a[4]),
            a[2].min(a[5]),
            a[0].max(a[3]),
            a[1].max(a[4]),
            a[2].max(a[5]),
        ]
    };
    let inflate = |b: [f64; 6], x: f64, y: f64, z: f64| {
        make([b[0] - x, b[1] - y, b[2] - z, b[3] + x, b[4] + y, b[5] + z])
    };
    let b = inflate(make(bb), 0.0, f64::from(-0.4f32), 0.0);
    let b = inflate(b, -1.0E-6, -1.0E-6, -1.0E-6);
    let (x0, y0, z0) = (
        b[0].floor() as i32,
        b[1].floor() as i32,
        b[2].floor() as i32,
    );
    let (x1, y1, z1) = (
        b[3].floor() as i32,
        b[4].floor() as i32,
        b[5].floor() as i32,
    );
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(drag) = state([x, y, z]) {
                    return Some(drag);
                }
            }
        }
    }
    None
}
/// The local player's ambient sound handlers.
pub struct Ambience {
    /// The client level's random.
    pub random: LegacyRandom,
    /// `BiomeAmbientSoundsHandler.loopSounds` (insertion order).
    loops: Vec<(String, InstanceId)>,
    previous_loop: Option<String>,
    pub moodiness: f32,
    /// `UnderwaterAmbientSoundHandler.tickDelay`.
    underwater_delay: i32,
    /// `LocalPlayer.wasUnderwater`.
    was_underwater: bool,
    /// `BubbleColumnAmbientSoundHandler.wasInBubbleColumn` / `firstTick`.
    was_in_bubble_column: bool,
    first_tick: bool,
}
impl Default for Ambience {
    fn default() -> Self {
        Self::new(crate::sound_engine::unique_seed())
    }
}
impl Ambience {
    pub fn new(seed: i64) -> Self {
        Self {
            random: LegacyRandom::new(seed),
            loops: Vec::new(),
            previous_loop: None,
            moodiness: 0.0,
            underwater_delay: 0,
            was_underwater: false,
            was_in_bubble_column: false,
            first_tick: true,
        }
    }
    /// `LocalPlayer.isUnderWater`.
    pub fn was_underwater(&self) -> bool {
        self.was_underwater
    }
    /// `BiomeAmbientSoundsHandler.loopSounds`.
    pub fn loops(&self) -> &[(String, InstanceId)] {
        &self.loops
    }
    /// `UnderwaterAmbientSoundHandler.tickDelay`.
    pub fn underwater_delay(&self) -> i32 {
        self.underwater_delay
    }
    /// A new player (leaving or changing worlds): fresh handlers.
    pub fn clear(&mut self, sounds: &mut impl Sounds) {
        for (_, id) in self.loops.drain(..) {
            sounds.release(id);
        }
        self.previous_loop = None;
        self.moodiness = 0.0;
        self.underwater_delay = 0;
        self.was_underwater = false;
        self.was_in_bubble_column = false;
        self.first_tick = true;
    }
    /// `ClientLevel.playLocalSound(x, y, z, sound, source, 1, 1, false)`.
    fn play_local(&mut self, event: &str, source: usize, at: [f64; 3], sounds: &mut impl Sounds) {
        let seed = self.random.next_long();
        let instance = SoundInstance::at(event, source, 1.0, 1.0, LegacyRandom::new(seed), at);
        sounds.play_instance(instance, false);
    }
    /// `LocalPlayer.updateIsUnderwater`, then the `ambientSoundHandlers`
    /// (underwater, bubble column, biome) as `LocalPlayer.tick` runs them.
    pub fn tick(&mut self, ctx: &AmbientContext, sounds: &mut impl Sounds) {
        let old = self.was_underwater;
        self.was_underwater = ctx.underwater;
        if !ctx.spectator {
            if !old && ctx.underwater {
                self.play_local(
                    "minecraft:ambient.underwater.enter",
                    AMBIENT,
                    ctx.position,
                    sounds,
                );
                let instance = SoundInstance::under_liquid_loop(
                    "minecraft:ambient.underwater.loop",
                    SoundEngine::unseeded(),
                );
                sounds.play_instance(instance, false);
            }
            if old && !ctx.underwater {
                self.play_local(
                    "minecraft:ambient.underwater.exit",
                    AMBIENT,
                    ctx.position,
                    sounds,
                );
            }
        }
        self.tick_underwater(sounds);
        self.tick_bubble_column(ctx, sounds);
        self.tick_biome(ctx, sounds);
    }
    /// `BubbleColumnAmbientSoundHandler.tick`: entering a column plays its
    /// inside sound (`LocalPlayer.playSound`, on PLAYERS).
    fn tick_bubble_column(&mut self, ctx: &AmbientContext, sounds: &mut impl Sounds) {
        if let Some(drag_down) = ctx.bubble_column {
            if !self.was_in_bubble_column && !self.first_tick && !ctx.spectator {
                let event = if drag_down {
                    "minecraft:block.bubble_column.whirlpool_inside"
                } else {
                    "minecraft:block.bubble_column.upwards_inside"
                };
                self.play_local(event, PLAYERS, ctx.position, sounds);
            }
            self.was_in_bubble_column = true;
        } else {
            self.was_in_bubble_column = false;
        }
        self.first_tick = false;
    }
    /// `UnderwaterAmbientSoundHandler.tick`.
    fn tick_underwater(&mut self, sounds: &mut impl Sounds) {
        self.underwater_delay -= 1;
        if self.underwater_delay <= 0 && self.was_underwater {
            let rand = self.random.next_float();
            let sound = if rand < 1.0E-4 {
                Some("minecraft:ambient.underwater.loop.additions.ultra_rare")
            } else if rand < 0.001 {
                Some("minecraft:ambient.underwater.loop.additions.rare")
            } else if rand < 0.01 {
                Some("minecraft:ambient.underwater.loop.additions")
            } else {
                None
            };
            if let Some(sound) = sound {
                self.underwater_delay = 0;
                let instance = SoundInstance::under_liquid_sub(sound, SoundEngine::unseeded());
                sounds.play_instance(instance, false);
            }
        }
    }
    /// `BiomeAmbientSoundsHandler.tick`.
    fn tick_biome(&mut self, ctx: &AmbientContext, sounds: &mut impl Sounds) {
        let mut kept = Vec::new();
        for (event, id) in std::mem::take(&mut self.loops) {
            if sounds.engine().instance(id).is_some_and(|i| i.stopped) {
                sounds.release(id);
            } else {
                kept.push((event, id));
            }
        }
        self.loops = kept;
        let current = ctx
            .sounds
            .r#loop
            .as_deref()
            .map(crate::sound_events::normalize);
        if current != self.previous_loop {
            self.previous_loop = current.clone();
            for (_, id) in &self.loops {
                if let Some(i) = sounds.engine().instance_mut(*id) {
                    i.fade_out();
                }
            }
            if let Some(event) = current {
                let id = match self.loops.iter().find(|(e, _)| *e == event) {
                    Some((_, id)) => *id,
                    None => {
                        let instance = SoundInstance::biome_loop(&event, SoundEngine::unseeded());
                        let id = sounds.engine().add(instance, true);
                        sounds.play_id(id);
                        self.loops.push((event, id));
                        id
                    }
                };
                if let Some(i) = sounds.engine().instance_mut(id) {
                    i.fade_in();
                }
            }
        }
        for (sound, chance) in &ctx.sounds.additions {
            if self.random.next_double() < *chance {
                let event = crate::sound_events::normalize(sound);
                let instance = SoundInstance::for_ambient_addition(&event, SoundEngine::unseeded());
                sounds.play_instance(instance, false);
            }
        }
        let Some(mood) = &ctx.sounds.mood else { return };
        let extent = mood.block_search_extent;
        let span = extent * 2 + 1;
        let x = ctx.position[0] + f64::from(self.random.next_int_bound(span).unwrap_or(0))
            - f64::from(extent);
        let y = ctx.eye_y + f64::from(self.random.next_int_bound(span).unwrap_or(0))
            - f64::from(extent);
        let z = ctx.position[2] + f64::from(self.random.next_int_bound(span).unwrap_or(0))
            - f64::from(extent);
        let sample = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
        let (sky, block) = (ctx.light)(sample);
        if sky > 0 {
            self.moodiness -= f32::from(sky) / 15.0 * 0.001;
        } else {
            self.moodiness -= (i32::from(block) - 1) as f32 / mood.tick_delay as f32;
        }
        if self.moodiness >= 1.0 {
            let sample_x = f64::from(sample[0]) + 0.5;
            let sample_y = f64::from(sample[1]) + 0.5;
            let sample_z = f64::from(sample[2]) + 0.5;
            let dx = sample_x - ctx.position[0];
            let dy = sample_y - ctx.eye_y;
            let dz = sample_z - ctx.position[2];
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            let source = distance + mood.offset;
            let at = [
                ctx.position[0] + dx / distance * source,
                ctx.eye_y + dy / distance * source,
                ctx.position[2] + dz / distance * source,
            ];
            // The mood sound draws from the level random itself.
            let event = crate::sound_events::normalize(&mood.sound);
            let instance = SoundInstance::for_ambient_mood(&event, self.random.clone(), at);
            let id = sounds.engine().add(instance, true);
            sounds.play_id(id);
            if let Some(i) = sounds.engine().instance(id) {
                self.random = i.random.clone();
            }
            sounds.release(id);
            self.moodiness = 0.0;
        } else {
            self.moodiness = self.moodiness.max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_engine::{Bound, TickContext, test_backend::Null};
    use crate::sound_events::SoundRegistry;

    fn engine() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"crimson": {"sounds": ["c"]}, "warped": {"sounds": ["w"]},
                 "ambient.cave": {"sounds": ["a", "b"]},
                 "ambient.underwater.loop": {"sounds": ["u"]},
                 "ambient.underwater.enter": {"sounds": ["e"]},
                 "ambient.underwater.exit": {"sounds": ["x"]}}"#,
            &|_, _| true,
        );
        SoundEngine::new(registry)
    }
    fn ctx<'a>(
        sounds: &'a AmbientSounds,
        underwater: bool,
        light: &'a dyn Fn([i32; 3]) -> (u8, u8),
    ) -> AmbientContext<'a> {
        AmbientContext {
            sounds,
            position: [0.5, 10.0, 0.5],
            eye_y: 11.62,
            underwater,
            spectator: false,
            light,
            bubble_column: None,
        }
    }

    #[test]
    fn loops_cross_fade_over_forty_ticks() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut a = Ambience::new(1);
        let lit = |_: [i32; 3]| (15u8, 0u8);
        let crimson = AmbientSounds {
            r#loop: Some("minecraft:crimson".into()),
            ..Default::default()
        };
        let warped = AmbientSounds {
            r#loop: Some("minecraft:warped".into()),
            ..Default::default()
        };
        let tick =
            |a: &mut Ambience, s: &AmbientSounds, engine: &mut SoundEngine, backend: &mut Null| {
                a.tick(
                    &ctx(s, false, &lit),
                    &mut Bound {
                        engine: &mut *engine,
                        backend: &mut *backend,
                    },
                );
                engine.tick(false, &TickContext::default(), backend);
            };
        for _ in 0..40 {
            tick(&mut a, &crimson, &mut engine, &mut backend);
        }
        for _ in 0..20 {
            tick(&mut a, &warped, &mut engine, &mut backend);
        }
        let volumes: Vec<f32> = a
            .loops
            .iter()
            .map(|(_, id)| engine.instance(*id).unwrap().volume)
            .collect();
        assert_eq!(volumes, [0.5, 0.5]);
        for _ in 0..30 {
            tick(&mut a, &warped, &mut engine, &mut backend);
        }
        assert_eq!(a.loops.len(), 1);
    }

    #[test]
    fn mood_builds_in_the_dark_and_plays_a_cave_sound() {
        let (mut engine, mut backend) = (engine(), Null::default());
        engine.listening = true;
        let mut a = Ambience::new(2);
        let cave = |_: [i32; 3]| (0u8, 0u8);
        let sounds = AmbientSounds {
            mood: Some(Mood {
                sound: "minecraft:ambient.cave".into(),
                tick_delay: 6000,
                block_search_extent: 8,
                offset: 2.0,
            }),
            ..Default::default()
        };
        let mut ticks = 0;
        while engine.notifications.is_empty() && ticks < 7000 {
            a.tick(
                &ctx(&sounds, false, &cave),
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            );
            ticks += 1;
        }
        assert!((5990..=6010).contains(&ticks), "{ticks}");
        assert_eq!(engine.notifications[0].identifier, "minecraft:ambient.cave");
        assert_eq!(a.moodiness, 0.0);
    }
}
