//! Situational music: `MusicManager.tick` choosing tracks from
//! `Minecraft.getSituationalMusic`, with the `minecraft:audio/background_music`
//! and `minecraft:audio/music_volume` environment attributes of dimension
//! types and biomes (a biome value overrides its dimension's).
use crate::sound_engine::{InstanceId, MUSIC, PlayResult, SoundEngine, SoundInstance, Sounds};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// `Music`: a sound event and the delay range (ticks) before it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Music {
    pub sound: String,
    pub min_delay: i32,
    pub max_delay: i32,
    pub replace_current_music: bool,
}
impl Music {
    pub fn new(sound: &str, min_delay: i32, max_delay: i32, replace_current_music: bool) -> Self {
        Self {
            sound: sound.to_owned(),
            min_delay,
            max_delay,
            replace_current_music,
        }
    }
    /// `Musics.MENU`.
    pub fn menu() -> Self {
        Self::new("minecraft:music.menu", 20, 600, true)
    }
    /// `Musics.CREDITS`.
    pub fn credits() -> Self {
        Self::new("minecraft:music.credits", 0, 0, true)
    }
    /// `Musics.END_BOSS`.
    pub fn end_boss() -> Self {
        Self::new("minecraft:music.dragon", 0, 0, true)
    }
    /// `Musics.CREATIVE`.
    pub fn creative() -> Self {
        Self::new("minecraft:music.creative", 12000, 24000, false)
    }
    /// `Musics.END`.
    pub fn end() -> Self {
        Self::new("minecraft:music.end", 6000, 24000, true)
    }
    /// `Musics.UNDER_WATER`.
    pub fn under_water() -> Self {
        Self::new("minecraft:music.under_water", 12000, 24000, false)
    }
    /// `Musics.GAME`.
    pub fn game() -> Self {
        Self::new("minecraft:music.game", 12000, 24000, false)
    }
    /// `Music.CODEC`.
    fn parse(v: &Value) -> Option<Self> {
        let sound = v["sound"].as_str()?;
        let int = |k: &str| v[k].as_i64().unwrap_or(0) as i32;
        Some(Self::new(
            sound,
            int("min_delay"),
            int("max_delay"),
            v["replace_current_music"].as_bool().unwrap_or(false),
        ))
    }
}
/// `BackgroundMusic`: the attribute value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackgroundMusic {
    pub default: Option<Music>,
    pub creative: Option<Music>,
    pub underwater: Option<Music>,
}
impl BackgroundMusic {
    fn parse(v: &Value) -> Self {
        Self {
            default: Music::parse(&v["default"]),
            creative: Music::parse(&v["creative"]),
            underwater: Music::parse(&v["underwater"]),
        }
    }
    /// `BackgroundMusic.select`.
    pub fn select(&self, creative: bool, underwater: bool) -> Option<&Music> {
        if underwater && self.underwater.is_some() {
            self.underwater.as_ref()
        } else if creative && self.creative.is_some() {
            self.creative.as_ref()
        } else {
            self.default.as_ref()
        }
    }
}
#[derive(Clone, Debug, Default)]
struct Layer {
    music: Option<BackgroundMusic>,
    volume: Option<f32>,
}
impl Layer {
    fn parse(json: &Value) -> Self {
        let attributes = &json["attributes"];
        let music = &attributes["minecraft:audio/background_music"];
        Self {
            music: (!music.is_null()).then(|| BackgroundMusic::parse(music)),
            volume: attributes["minecraft:audio/music_volume"]
                .as_f64()
                .map(|v| v.clamp(0.0, 1.0) as f32),
        }
    }
}
/// What `Minecraft.getSituationalMusic` and `getMusicVolume` read.
#[derive(Clone, Debug)]
pub struct Situation<'a> {
    /// `Screen.getBackgroundMusic` of the open screen.
    pub screen_music: Option<Music>,
    /// `Minecraft.player != null`.
    pub in_world: bool,
    /// The player's level is the End.
    pub in_end: bool,
    /// `BossHealthOverlay.shouldPlayMusic`: a boss bar asks for music.
    pub boss_music: bool,
    /// `BACKGROUND_MUSIC` at the camera.
    pub background: &'a BackgroundMusic,
    /// `MUSIC_VOLUME` at the camera (1 without a level).
    pub probe_volume: f32,
    /// `abilities.instabuild && abilities.mayfly`.
    pub creative: bool,
    /// `LocalPlayer.isUnderWater`.
    pub underwater: bool,
}
impl Situation<'_> {
    /// `Minecraft.getSituationalMusic`.
    pub fn music(&self) -> Option<Music> {
        if let Some(music) = &self.screen_music {
            return Some(music.clone());
        }
        if !self.in_world {
            return Some(Music::menu());
        }
        if self.in_end && self.boss_music {
            return Some(Music::end_boss());
        }
        self.background
            .select(self.creative, self.underwater)
            .cloned()
    }
    /// `Minecraft.getMusicVolume`.
    pub fn volume(&self) -> f32 {
        if self.screen_music.is_some() {
            1.0
        } else {
            self.probe_volume
        }
    }
}
/// Music attributes by dimension type and biome.
#[derive(Clone, Debug, Default)]
pub struct MusicTables {
    dimensions: HashMap<String, Layer>,
    biomes: HashMap<String, Layer>,
}
impl MusicTables {
    pub fn load(root: &Path) -> Self {
        let read = |dir: &str| -> HashMap<String, Layer> {
            let Ok(entries) = fs::read_dir(root.join("data/minecraft").join(dir)) else {
                return HashMap::new();
            };
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_stem()?.to_str()?.to_owned();
                    let json: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                    Some((format!("minecraft:{name}"), Layer::parse(&json)))
                })
                .collect()
        };
        Self {
            dimensions: read("dimension_type"),
            biomes: read("worldgen/biome"),
        }
    }
    /// `EnvironmentAttributes.BACKGROUND_MUSIC` at a position.
    pub fn background(&self, dimension: &str, biome: &str) -> BackgroundMusic {
        self.biomes
            .get(biome)
            .and_then(|l| l.music.clone())
            .or_else(|| self.dimensions.get(dimension).and_then(|l| l.music.clone()))
            .unwrap_or_default()
    }
    /// `EnvironmentAttributes.MUSIC_VOLUME` at a position (default 1).
    pub fn volume(&self, dimension: &str, biome: &str) -> f32 {
        self.biomes
            .get(biome)
            .and_then(|l| l.volume)
            .or_else(|| self.dimensions.get(dimension).and_then(|l| l.volume))
            .unwrap_or(1.0)
    }
}
/// `MusicManager.MusicFrequency` (the `musicFrequency` option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MusicFrequency {
    #[default]
    Default,
    Frequent,
    Constant,
}
impl MusicFrequency {
    pub fn parse(name: &str) -> Self {
        match name {
            "FREQUENT" => Self::Frequent,
            "CONSTANT" => Self::Constant,
            _ => Self::Default,
        }
    }
    fn max_frequency(self) -> i32 {
        match self {
            Self::Default => 20 * 1200,
            Self::Frequent => 10 * 1200,
            Self::Constant => 0,
        }
    }
    fn next_song_delay(self, music: Option<&Music>, random: &mut LegacyRandom) -> i32 {
        let Some(music) = music else {
            return self.max_frequency();
        };
        if self == Self::Constant {
            return 100;
        }
        let min = music.min_delay.min(self.max_frequency());
        let max = music.max_delay.min(self.max_frequency());
        next_int(random, min, max)
    }
}
/// `Mth.nextInt(random, min, max)`: inclusive.
fn next_int(random: &mut LegacyRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}
/// `MusicManager`.
pub struct MusicManager {
    random: LegacyRandom,
    /// `currentMusic`: the instance the manager holds.
    current: Option<InstanceId>,
    current_gain: f32,
    next_song_delay: i32,
    toast_shown: bool,
    pub frequency: MusicFrequency,
    /// `ToastManager.showNowPlayingToast` (true) / `hideNowPlayingToast`
    /// (false) calls since the last drain.
    pub toast_requests: Vec<bool>,
}
impl Default for MusicManager {
    fn default() -> Self {
        // RandomSource.create(): seeded from the clock, not the world.
        Self::new(crate::sound_engine::unique_seed())
    }
}
impl MusicManager {
    pub fn new(seed: i64) -> Self {
        Self {
            random: LegacyRandom::new(seed),
            current: None,
            current_gain: 1.0,
            next_song_delay: 100,
            toast_shown: false,
            frequency: MusicFrequency::Default,
            toast_requests: Vec::new(),
        }
    }
    /// The sound event of the current track.
    pub fn current<'a>(&self, sounds: &'a SoundEngine) -> Option<&'a str> {
        self.current
            .and_then(|id| sounds.instance(id))
            .map(|i| i.identifier.as_str())
    }
    pub fn current_instance(&self) -> Option<InstanceId> {
        self.current
    }
    pub fn next_song_delay(&self) -> i32 {
        self.next_song_delay
    }
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
    pub fn toast_shown(&self) -> bool {
        self.toast_shown
    }
    fn set_current(&mut self, id: Option<InstanceId>, sounds: &mut impl Sounds) {
        if let Some(old) = self.current
            && Some(old) != id
        {
            sounds.release(old);
        }
        self.current = id;
    }
    /// `MusicManager.tick` with `Minecraft.getMusicVolume` and
    /// `getSituationalMusic`; `loading` is a `LevelLoadingScreen`.
    pub fn tick(
        &mut self,
        music: Option<&Music>,
        volume: f32,
        loading: bool,
        sounds: &mut impl Sounds,
    ) {
        if self.current.is_some()
            && self.current_gain != volume
            && !self.fade(volume, music, sounds)
        {
            return;
        }
        let Some(music) = music else {
            self.next_song_delay = self.next_song_delay.max(100);
            return;
        };
        if let Some(current) = self.current {
            // canReplace
            let playing = sounds
                .engine()
                .instance(current)
                .map(|i| i.identifier.clone());
            if music.replace_current_music && playing.as_deref() != Some(music.sound.as_str()) {
                sounds.stop_instance(current);
                self.next_song_delay = next_int(&mut self.random, 0, music.min_delay / 2);
            }
            if !sounds.is_active(current) {
                self.set_current(None, sounds);
                let delay = self
                    .frequency
                    .next_song_delay(Some(music), &mut self.random);
                self.next_song_delay = self.next_song_delay.min(delay);
            }
        }
        self.next_song_delay = self.next_song_delay.min(music.max_delay);
        if self.current.is_none() && !loading {
            self.next_song_delay -= 1;
            if self.next_song_delay <= 0 {
                self.start_playing(music, sounds);
            }
        }
    }
    /// `MusicManager.startPlaying`.
    pub fn start_playing(&mut self, music: &Music, sounds: &mut impl Sounds) {
        let instance = SoundInstance::for_music(&music.sound, SoundEngine::unseeded());
        let (id, result) = sounds.play_instance(instance, true);
        self.set_current(Some(id), sounds);
        match result {
            PlayResult::Started => {
                self.toast_requests.push(true);
                self.toast_shown = true;
            }
            PlayResult::StartedSilently => self.toast_shown = false,
            PlayResult::NotStarted => {}
        }
        self.next_song_delay = i32::MAX;
    }
    /// `MusicManager.showNowPlayingToastIfNeeded`.
    pub fn show_now_playing_toast_if_needed(&mut self) {
        if !self.toast_shown {
            self.toast_requests.push(true);
            self.toast_shown = true;
        }
    }
    /// `MusicManager.stopPlaying(Music)`.
    pub fn stop_playing_music(
        &mut self,
        music: &Music,
        situational: Option<&Music>,
        sounds: &mut impl Sounds,
    ) {
        if self.is_playing_music(music, sounds.engine()) {
            self.stop_playing(situational, sounds);
        }
    }
    /// `MusicManager.stopPlaying()`.
    pub fn stop_playing(&mut self, situational: Option<&Music>, sounds: &mut impl Sounds) {
        if let Some(current) = self.current {
            sounds.stop_instance(current);
            self.set_current(None, sounds);
            self.toast_requests.push(false);
        }
        self.next_song_delay = self
            .frequency
            .next_song_delay(situational, &mut self.random)
            + 100;
    }
    /// `MusicManager.isPlayingMusic`.
    pub fn is_playing_music(&self, music: &Music, sounds: &SoundEngine) -> bool {
        self.current(sounds) == Some(music.sound.as_str())
    }
    /// `MusicManager.setMinutesBetweenSongs`.
    pub fn set_frequency(&mut self, frequency: MusicFrequency, situational: Option<&Music>) {
        self.frequency = frequency;
        self.next_song_delay = self
            .frequency
            .next_song_delay(situational, &mut self.random);
    }
    /// `MusicManager.fadePlaying`: false once the track faded out and stopped.
    fn fade(&mut self, volume: f32, situational: Option<&Music>, sounds: &mut impl Sounds) -> bool {
        if self.current.is_none() {
            return false;
        }
        if self.current_gain == volume {
            return true;
        }
        if self.current_gain < volume {
            self.current_gain += self.current_gain.clamp(5.0E-4, 0.005);
            if self.current_gain > volume {
                self.current_gain = volume;
            }
        } else {
            self.current_gain = 0.03 * volume + 0.97 * self.current_gain;
            if (self.current_gain - volume).abs() < 1.0E-4 || self.current_gain < volume {
                self.current_gain = volume;
            }
        }
        self.current_gain = self.current_gain.clamp(0.0, 1.0);
        if self.current_gain <= 1.0E-4 {
            self.stop_playing(situational, sounds);
            false
        } else {
            sounds.update_category_volume(MUSIC, self.current_gain);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_engine::{Bound, test_backend::Null};
    use crate::sound_events::SoundRegistry;

    fn engine() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"music.menu": {"sounds": [{"name": "m", "stream": true}]},
                 "music.game": {"sounds": [{"name": "g", "stream": true}]},
                 "music.end": {"sounds": [{"name": "e", "stream": true}]}}"#,
            &|_, _| true,
        );
        SoundEngine::new(registry)
    }

    #[test]
    fn menu_music_starts_within_its_delay_and_game_music_replaces_nothing() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(1);
        let menu = Music::menu();
        let tick =
            |m: &mut MusicManager, music: &Music, engine: &mut SoundEngine, backend: &mut Null| {
                m.tick(Some(music), 1.0, false, &mut Bound { engine, backend });
            };
        // nextSongDelay starts at 100, capped to MENU's max delay 600: starts on tick 100.
        for _ in 0..99 {
            tick(&mut m, &menu, &mut engine, &mut backend);
        }
        assert!(m.current(&engine).is_none());
        tick(&mut m, &menu, &mut engine, &mut backend);
        assert_eq!(m.current(&engine), Some("minecraft:music.menu"));
        // Game music does not replace the current track.
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        tick(&mut m, &game, &mut engine, &mut backend);
        assert_eq!(m.current(&engine), Some("minecraft:music.menu"));
    }

    #[test]
    fn a_new_level_stops_menu_music_and_waits_for_game_music() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(2);
        let menu = Music::menu();
        m.start_playing(
            &menu,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        // Minecraft.setLevel -> SoundManager.stop(); the next tick has a player.
        engine.stop_all(&mut backend);
        let game = Music::game();
        m.tick(
            Some(&game),
            1.0,
            true,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        assert!(m.current_instance().is_none());
        assert!((12000..=24000).contains(&m.next_song_delay()));
    }

    #[test]
    fn zero_music_volume_fades_out_and_stops() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(3);
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        m.start_playing(
            &game,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        let mut ticks = 0;
        while m.current_instance().is_some() && ticks < 1000 {
            m.tick(
                Some(&game),
                0.0,
                false,
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            );
            ticks += 1;
        }
        // 0.97^n < 1e-4 after ~303 ticks.
        assert!((290..=310).contains(&ticks), "{ticks}");
    }

    #[test]
    fn select_prefers_underwater_then_creative() {
        let bg = BackgroundMusic {
            default: Some(Music::new("a", 0, 0, false)),
            creative: Some(Music::new("b", 0, 0, false)),
            underwater: Some(Music::new("c", 0, 0, false)),
        };
        assert_eq!(bg.select(true, true).unwrap().sound, "c");
        assert_eq!(bg.select(true, false).unwrap().sound, "b");
        assert_eq!(bg.select(false, false).unwrap().sound, "a");
    }
}
