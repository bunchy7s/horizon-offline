//! Jukeboxes (`JukeboxBlock`, `JukeboxBlockEntity`, `JukeboxSongPlayer`):
//! inserting a `jukebox_playable` disc, ejecting it, the song with its
//! "Now Playing" overlay, note particles every second, the redstone
//! signal while playing, the comparator output, and saving.
use super::Game;
use horizon_client::{
    containers::{BlockEntity, Jukebox},
    first_person::Hand,
    items::ItemStack,
    mobs::Species,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::level::flags as level_flags;

impl Game<'_> {
    fn jukebox(&self, pos: [i32; 3]) -> Option<&Jukebox> {
        match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Jukebox(j)) => Some(j),
            _ => None,
        }
    }
    fn set_has_record(&mut self, pos: [i32; 3], has: bool) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let registry = self.pack.assets.registry;
        let Some(mut s) = registry.state(state).cloned() else {
            return;
        };
        s.properties.insert(
            "has_record".into(),
            if has { "true" } else { "false" }.into(),
        );
        if let Some(new) = registry.id(&s) {
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, p, new, level_flags::UPDATE_CLIENTS)
            });
        }
    }
    /// `onSongChanged`: neighbours (the signal) and comparators.
    fn jukebox_changed(&mut self, pos: [i32; 3]) {
        let p = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, p, block);
        });
        self.container_changed((pos[0], pos[1], pos[2]));
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }
    /// `JukeboxBlock.useItemOn` / `useWithoutItem` on the jukebox at `pos`.
    pub(super) fn use_jukebox(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        if &*self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            != "minecraft:jukebox"
        {
            return false;
        }
        if self.jukebox(pos).is_some_and(|j| j.record.is_some()) {
            // TRY_WITH_EMPTY_HAND: only the main hand goes on to useWithoutItem.
            if self.hand != Hand::Main {
                return false;
            }
            self.eject_record(pos);
            return true;
        }
        let Some(stack) = held else { return false };
        let Some(song) = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.jukebox_song.clone())
        else {
            return false;
        };
        // JukeboxPlayable.tryInsertIntoJukebox
        let record = ItemStack {
            count: 1,
            ..stack.clone()
        };
        if !self.creative {
            self.consume_held();
        }
        self.stats.custom("play_record", 1);
        self.world.block_entities.insert(
            (pos[0], pos[1], pos[2]),
            BlockEntity::Jukebox(Jukebox {
                record: Some(record),
                ticks: 0,
                playing: true,
            }),
        );
        self.set_has_record(pos, true);
        self.start_song(pos, &song);
        self.jukebox_changed(pos);
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        true
    }
    /// `JukeboxBlock.useItemOn` on this client's level for the main hand:
    /// whether the jukebox at `pos` takes the click (`has_record` pops the
    /// disc out, or a `jukebox_playable` item goes in). The server does the
    /// rest, see `use_jukebox`.
    pub(super) fn jukebox_takes(&self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:jukebox" {
            return false;
        }
        let has_record = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("has_record"))
            .is_some_and(|v| v == "true");
        has_record
            || held.is_some_and(|stack| {
                self.res
                    .items()
                    .get(&stack.item)
                    .is_some_and(|i| i.jukebox_song.is_some())
            })
    }
    /// Level event 1010: the song from the jukebox, and the "Now Playing" overlay.
    fn start_song(&mut self, pos: [i32; 3], song: &str) {
        self.stop_sound(pos);
        let Some(s) = self.res.items().songs.get(song).cloned() else {
            return;
        };
        // LevelRenderer.playJukeboxSong: SimpleSoundInstance.forJukeboxSong.
        let id = self.audio.play_jukebox_song(&s.sound, pos);
        self.jukebox_sounds.insert((pos[0], pos[1], pos[2]), id);
        let description =
            horizon_client::component::to_text(&s.description, &mut |k| Text::literal(k));
        self.titles
            .set_overlay(Text::translate("record.nowPlaying", &[description]), true);
        self.update_song_volumes();
        self.notify_record_playing(pos, true);
        if self.audio.capture.is_some() {
            self.lan_songs.push((pos, Some(song.to_owned())));
        }
    }
    /// `LevelEventHandler` 1010 on a client of a server: `playJukeboxSong`
    /// for the song the event's data names in the server's `jukebox_song`
    /// registry (`Registry.get(int).ifPresent`: an unknown id does nothing).
    /// The server's jukebox plays it; this client only hears it.
    pub(super) fn jukebox_song_event(&mut self, pos: [i32; 3], data: i32) {
        if let Some(song) = jukebox_song_name(self.remote.as_ref().map(|r| &*r.registries), data) {
            self.start_song(pos, &song);
        }
    }
    /// `LevelEventHandler` 1011: `stopJukeboxSongAndNotifyNearby`.
    pub(super) fn jukebox_stop_event(&mut self, pos: [i32; 3]) {
        self.stop_sound(pos);
        self.notify_record_playing(pos, false);
    }
    fn stop_sound(&mut self, pos: [i32; 3]) {
        if let Some(id) = self.jukebox_sounds.remove(&(pos[0], pos[1], pos[2])) {
            self.audio.stop_and_release(id);
        }
    }
    /// Level event 1011: the song ends, here and for the guests.
    fn stop_song(&mut self, pos: [i32; 3]) {
        self.stop_sound(pos);
        self.notify_record_playing(pos, false);
        if self.audio.capture.is_some() {
            self.lan_songs.push((pos, None));
        }
    }
    /// `LevelEventHandler.notifyNearbyEntities` (level events 1010 and 1011):
    /// parrots within three blocks of the jukebox start or stop dancing.
    pub(super) fn notify_record_playing(&mut self, pos: [i32; 3], playing: bool) {
        let min = Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
        let area = Aabb::new(min, min.add_vector(Vec3::new(1.0, 1.0, 1.0)))
            .inflate(Vec3::new(3.0, 3.0, 3.0));
        for m in self
            .mobs
            .iter_mut()
            .filter(|m| m.species() == Species::Parrot && m.bounds().intersects(area))
        {
            m.extra.party_jukebox = playing.then_some(pos);
        }
    }
    /// `popOutTheItem`: the disc above the jukebox, and the song stops.
    fn eject_record(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let record = match self.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Jukebox(j)) => {
                j.playing = false;
                j.ticks = 0;
                j.record.take()
            }
            _ => None,
        };
        self.stop_song(pos);
        if let Some(record) = record {
            // atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random, 0.7);
            // `new ItemEntity` moves it with its own random.
            let at = horizon_world::level::removal::jukebox_pop_position(
                &mut self.world.random,
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            );
            self.spawn_item(record, at, super::dispenser::new_item_motion(), 10);
        }
        self.set_has_record(pos, false);
        self.jukebox_changed(pos);
    }
    /// A broken jukebox stops its song (its disc drops with the contents).
    pub(super) fn jukebox_removed(&mut self, pos: [i32; 3]) {
        self.stop_song(pos);
    }
    /// `JukeboxSongPlayer.tick` of the jukebox at `key`, while it plays.
    pub(super) fn tick_jukebox(&mut self, key: (i32, i32, i32)) {
        let playing = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Jukebox(j)) if j.playing => j.record.as_ref().and_then(|r| {
                let song = self.res.items().get(&r.item)?.jukebox_song.clone()?;
                Some((song, j.ticks))
            }),
            _ => None,
        };
        let Some((song, ticks)) = playing else {
            return;
        };
        let pos = [key.0, key.1, key.2];
        let Some(s) = self.res.items().songs.get(&song).cloned() else {
            return;
        };
        if s.finished(ticks) {
            if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key) {
                j.playing = false;
                j.ticks = 0;
            }
            self.stop_song(pos);
            self.jukebox_changed(pos);
            return;
        }
        if !self.jukebox_sounds.contains_key(&key) && ticks == 0 {
            self.start_song(pos, &song);
        }
        if ticks % 20 == 0 {
            let color = f64::from(self.world.random.next_int_bound(4).unwrap_or(0)) / 24.0;
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 1.2,
                f64::from(pos[2]) + 0.5,
            ];
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Note),
                at,
                [color, 0.0, 0.0],
            );
        }
        if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key) {
            j.ticks += 1;
        }
    }
    /// Let go of songs that ended or whose jukebox stopped.
    pub(super) fn update_song_volumes(&mut self) {
        let sounds: Vec<_> = self
            .jukebox_sounds
            .iter()
            .map(|(k, id)| (*k, *id))
            .collect();
        for (k, id) in sounds {
            // A server's client has no playing jukebox of its own: its songs
            // stop with level event 1011, or when they end.
            let playing = matches!(self.world.block_entities.get(&k), Some(BlockEntity::Jukebox(j)) if j.playing);
            if !keeps_song(
                self.audio.engine.is_active(id),
                self.remote.is_some(),
                playing,
            ) {
                self.audio.stop_and_release(id);
                self.jukebox_sounds.remove(&k);
            }
        }
    }
}
/// The song a level event 1010's data names: its id in the `jukebox_song`
/// registry the server sent (`Registry.get(int)`). Only a server's client
/// has one; a level that plays its own jukeboxes starts their songs itself.
fn jukebox_song_name(
    registries: Option<&horizon_protocol::registry::Dynamic>,
    id: i32,
) -> Option<String> {
    registries?
        .name("minecraft:jukebox_song", id)
        .map(str::to_owned)
}
/// Whether a jukebox song keeps playing: until it ends, and, in a level that
/// plays its own jukeboxes, while that jukebox plays. A server's client
/// stops it at level event 1011 (`LevelRenderer.playingJukeboxSongs`).
fn keeps_song(active: bool, server_client: bool, jukebox_playing: bool) -> bool {
    active && (server_client || jukebox_playing)
}
/// The jukebox's redstone signal (15 while a song plays) and comparator output.
pub(super) fn jukebox_signals(
    world: &horizon_client::world::World,
    items: &horizon_client::items::Items,
    pos: BlockPos,
) -> Option<(i32, i32)> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Jukebox(j)) => {
            let comparator = j
                .record
                .as_ref()
                .and_then(|r| items.get(&r.item)?.jukebox_song.as_ref())
                .and_then(|s| items.songs.get(s))
                .map_or(0, |s| s.comparator);
            Some((if j.playing { 15 } else { 0 }, comparator))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{jukebox_song_name, keeps_song};
    use horizon_protocol::registry::Dynamic;

    /// On a server's client the song comes from level event 1010, whose data
    /// is the song's id in the server's `jukebox_song` registry (#3173), and
    /// it plays on although this client's jukebox isn't playing.
    #[test]
    fn a_server_clients_jukebox_song_comes_from_level_event_1010() {
        let mut registries = Dynamic::default();
        registries.registries.insert(
            "minecraft:jukebox_song".into(),
            ["minecraft:11", "minecraft:13", "minecraft:cat"]
                .map(|n| (n.to_owned(), None))
                .to_vec(),
        );
        assert_eq!(
            jukebox_song_name(Some(&registries), 2).as_deref(),
            Some("minecraft:cat")
        );
        assert_eq!(jukebox_song_name(Some(&registries), 3), None);
        assert_eq!(jukebox_song_name(Some(&registries), -1), None);
        assert_eq!(jukebox_song_name(None, 2), None);
        // A server's client: on until the song ends (or level event 1011).
        assert!(keeps_song(true, true, false));
        assert!(!keeps_song(false, true, false));
        // A level that plays its own jukebox: also until the jukebox stops.
        assert!(keeps_song(true, false, true));
        assert!(!keeps_song(true, false, false));
    }
}
