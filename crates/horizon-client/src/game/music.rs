//! `Minecraft.getSituationalMusic` and `getMusicVolume` for the music manager,
//! the local player's ambient sound handlers and the sound engine tick.
use super::Game;
use horizon_client::{
    blocks::Fluid,
    music::{BackgroundMusic, Music, MusicFrequency, Situation},
    sound_engine::{SoundEngine, SoundInstance, Sounds, TickContext},
};

impl Game<'_> {
    /// `Minecraft.tick` -> `MusicManager.tick`, once per client tick.
    pub(super) fn tick_music(&mut self) {
        // Splitscreen: one music, P1's.
        if self.options.split_guest {
            return;
        }
        let (music, volume) = self.situational_music();
        // Options.musicFrequency's listener: MusicManager.setMinutesBetweenSongs.
        let frequency = MusicFrequency::parse(&self.settings.file.name("musicFrequency"));
        if frequency != self.music.frequency {
            self.music.set_frequency(frequency, music.as_ref());
        }
        // LevelLoadingScreen: no new track before the player is in the world.
        let loading = self.title.is_none() && self.player.is_none() && self.win.is_none();
        self.music
            .tick(music.as_ref(), volume, loading, &mut self.audio);
        // Options.musicToast's listener: ToastManager.setMusicToastDisplayState.
        let state = self.settings.file.int("musicToast").clamp(0, 2) as u8;
        if state != self.music_toast_state {
            self.music_toast_state = state;
            self.set_music_toast_state(state);
        }
        // MusicManager's showNowPlayingToast and hideNowPlayingToast calls.
        let multiplier = f64::from(self.settings.notification_display_time);
        for show in std::mem::take(&mut self.music.toast_requests) {
            if let Some(toast) = &mut self.now_playing {
                if show {
                    toast.show(multiplier);
                } else {
                    toast.hide();
                }
            }
        }
    }
    /// `Options.getFinalSoundSourceVolume(SoundSource.MUSIC)`.
    fn final_music_volume(&self) -> f32 {
        self.settings.master_volume * self.settings.volumes[1]
    }
    /// `ToastManager.setMusicToastDisplayState`: Pause and Pause and Toast make
    /// the toast (the second one shows it at once while music is audible),
    /// Never drops it.
    fn set_music_toast_state(&mut self, state: u8) {
        self.now_playing = match state {
            0 => None,
            _ => {
                let mut toast = horizon_client::toasts::NowPlayingToast::default();
                if state == 2 && self.final_music_volume() > 0.0 {
                    toast.show(f64::from(self.settings.notification_display_time));
                }
                Some(toast)
            }
        };
    }
    /// `NowPlayingToast.getNowPlayingString` of the song the music manager
    /// plays: its file's short language key, slashes as dots.
    fn now_playing_song(&self) -> Option<String> {
        let id = self.music.current_instance()?;
        let location = self
            .audio
            .engine
            .instance(id)?
            .sound
            .as_ref()?
            .location
            .as_str();
        let (namespace, path) = location.split_once(':').unwrap_or(("minecraft", location));
        // Identifier.toShortLanguageKey.
        let key = if namespace == "minecraft" {
            path.to_owned()
        } else {
            format!("{namespace}.{path}")
        }
        .replace('/', ".");
        Some(horizon_client::lang::get_or(&key, &key))
    }
    /// `ToastManager.update` (it plays no toast sound for this toast) and
    /// `extractRenderState` for the Now Playing toast, which slides in at the
    /// top left over every screen but the pause screen, and
    /// `PauseScreen.extractRenderState` with its `tick`, which draws the song
    /// name on the pause menu under the Pause and Pause and Toast settings
    /// while music is audible. `hidden` (F1) keeps the toast
    /// timing out without drawing it.
    pub(super) fn draw_music_toast(
        &mut self,
        batch: &mut horizon_client::hud::HudBatch,
        hidden: bool,
    ) {
        let song = self.now_playing_song();
        let Some(toast) = self.now_playing.as_mut() else {
            return;
        };
        toast.update(song.is_some(), &mut self.music_notes);
        let Some(song) = song else {
            return;
        };
        if self.screen == super::Screen::Paused {
            if self.pause_menu && self.final_music_volume() > 0.0 {
                self.music_notes.tick();
                horizon_client::toasts::draw_now_playing(
                    &self.gui.atlas,
                    batch,
                    0.0,
                    &song,
                    self.music_notes.color,
                );
            }
        } else if !hidden
            && self.music_toast_state == 2
            && let Some(toast) = &self.now_playing
        {
            toast.draw(&self.gui.atlas, batch, &song, &self.music_notes);
        }
    }
    /// `Minecraft.tick` -> `SoundManager.tick(pause)`, after the music, with
    /// `pauseAllExcept(MUSIC, UI)` when the game pauses and `resume` when it
    /// carries on.
    pub(super) fn tick_sound_engine(&mut self) {
        if self.pause != self.sound_paused {
            if self.pause {
                self.audio.pause();
            } else {
                self.audio.resume();
            }
            self.sound_paused = self.pause;
        }
        let (vehicle, carts) = self.start_cart_sounds();
        let mobs = self.start_mob_sounds();
        let ridden = self.start_riding_sound();
        let (player_position, player_speed_sqr, player_fall_flying) =
            self.player.as_ref().map_or(([0.0; 3], 0.0, false), |p| {
                let at = p.position;
                (
                    [at.x, at.y, at.z],
                    p.velocity.length_squared(),
                    p.fall_flying,
                )
            });
        // LocalPlayer.onSyncedDataUpdated: starting to glide starts the
        // elytra's flying sound.
        if player_fall_flying && !self.was_fall_flying {
            let instance = SoundInstance::elytra(player_position, SoundEngine::unseeded());
            self.audio.play_instance(instance, false);
        }
        self.was_fall_flying = player_fall_flying;
        let ctx = TickContext {
            player_underwater: self.ambience.was_underwater(),
            player_removed: self.player.is_none(),
            vehicle,
            carts,
            mobs,
            frozen: !self.clock.runs_normally(),
            player_position,
            player_speed_sqr,
            player_fall_flying,
            ridden,
        };
        self.audio.tick(self.pause, &ctx);
    }
    /// `Minecraft.setLevel` -> `updateLevelInEngines`: `SoundManager.stop()`.
    /// The music manager keeps its stopped track and, on its next tick, waits
    /// the level's music delay before the next one.
    pub(super) fn stop_sounds_for_new_level(&mut self) {
        self.ambience.clear(&mut self.audio);
        self.audio.stop_all();
    }
    /// `getSituationalMusic` and `getMusicVolume`.
    pub(super) fn situational_music(&self) -> (Option<Music>, f32) {
        let empty = BackgroundMusic::default();
        // The title screen has no player (and no level for the camera probe).
        // A server's client has its player from handleLogin on, before the
        // first teleport makes it here: the camera is still at the origin.
        let eye = if self.title.is_some() {
            None
        } else {
            self.eye_position().or_else(|| {
                self.remote
                    .as_ref()
                    .is_some_and(|r| r.logged_in)
                    .then_some([0.0; 3])
            })
        };
        let (background, probe_volume) = match eye {
            Some(eye) => {
                let (dimension, biome) = (self.dim.id(), self.biome_at(eye));
                let tables = self.res.music();
                (
                    tables.background(dimension, biome),
                    tables.volume(dimension, biome),
                )
            }
            None => (empty, 1.0),
        };
        let situation = Situation {
            // WinScreen.getBackgroundMusic.
            screen_music: self
                .win
                .is_some()
                .then(horizon_client::music::Music::credits),
            in_world: eye.is_some(),
            in_end: self.dim == super::Dim::End,
            boss_music: self.boss_music(),
            background: &background,
            probe_volume,
            creative: self.creative,
            underwater: self.ambience.was_underwater(),
        };
        (situation.music(), situation.volume())
    }
    /// `BiomeManager.getNoiseBiomeAtPosition`: the quart cell holding a position.
    fn biome_at(&self, p: [f64; 3]) -> &str {
        let (x, y, z) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        self.world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or("minecraft:plains", |c| {
                world_biomes!(self).list[c.biome(x, y, z) as usize]
                    .name
                    .as_str()
            })
    }
    /// `isEyeInFluid(WATER)`: the eye inside the water of its block column.
    pub(super) fn eye_in_water(&self, x: f64, eye_y: f64, z: f64) -> bool {
        let (bx, by, bz) = (x.floor() as i32, eye_y.floor() as i32, z.floor() as i32);
        let f = self.res.facts().get(self.world.block(bx, by, bz));
        if f.fluid != Fluid::Water {
            return false;
        }
        let above = self.res.facts().get(self.world.block(bx, by + 1, bz));
        let height = if above.fluid == Fluid::Water {
            1.0
        } else {
            f64::from(f.fluid_level) / 9.0
        };
        eye_y <= f64::from(by) + height
    }
}

impl Game<'_> {
    /// The local player's ambient sound handlers (LocalPlayer.tick).
    pub(super) fn tick_ambience(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let feet = [p.position.x, p.position.y, p.position.z];
        let eye_y = feet[1] + f64::from(p.dimensions.eye_height);
        let sounds = self.res.ambient().get(self.dim.id(), self.biome_at(feet));
        let underwater = self.eye_in_water(feet[0], eye_y, feet[2]);
        let world = &self.world;
        let light = |b: [i32; 3]| {
            let l = world.light(b[0], b[1], b[2]);
            (l >> 4, l & 15)
        };
        let (half, height) = (
            f64::from(p.dimensions.width / 2.0),
            f64::from(p.dimensions.height),
        );
        let registry = &self.pack.assets.registry;
        let facts = self.res.facts();
        let bubble_column = horizon_client::ambience::bubble_column_in(
            [
                feet[0] - half,
                feet[1],
                feet[2] - half,
                feet[0] + half,
                feet[1] + height,
                feet[2] + half,
            ],
            |b| {
                let state = world.block(b[0], b[1], b[2]);
                (&*facts.get(state).name == "minecraft:bubble_column").then(|| {
                    registry
                        .state(state)
                        .and_then(|s| s.properties.get("drag"))
                        .is_some_and(|d| d == "true")
                })
            },
        );
        let ctx = horizon_client::ambience::AmbientContext {
            sounds: &sounds,
            position: feet,
            eye_y,
            underwater,
            spectator: self.spectator(),
            light: &light,
            bubble_column,
        };
        self.ambience.tick(&ctx, &mut self.audio);
    }
}
