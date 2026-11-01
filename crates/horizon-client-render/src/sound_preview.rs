//! `SoundPreviewHandler`: the sound a category volume slider plays while it
//! moves on the title screen's options (`Options.createSoundSliderOptionInstance`
//! calls `preview` when there is no level).
use crate::settings::SOUND_SOURCES;
use crate::sound_engine::{InstanceId, SoundEngine, SoundInstance, Sounds};

/// The sound `SoundPreviewHandler.preview` plays for a `SoundSource` index,
/// or `None` (`SoundEvents.EMPTY`) for master, music and voice.
pub fn preview_event(source: usize) -> Option<&'static str> {
    Some(match *SOUND_SOURCES.get(source)? {
        "record" => "minecraft:block.note_block.guitar",
        "weather" => "minecraft:entity.lightning_bolt.thunder",
        "block" => "minecraft:block.grass.place",
        "hostile" => "minecraft:entity.zombie.ambient",
        // SoundEvents.COW_SOUNDS.get(CowSoundVariants.SoundSet.CLASSIC).ambientSound()
        "neutral" => "minecraft:entity.cow.ambient",
        "player" => "minecraft:entity.generic.eat",
        "ambient" => "minecraft:ambient.cave",
        "ui" => "minecraft:ui.button.click",
        _ => return None,
    })
}

/// `SoundPreviewHandler`'s `activePreview` and `previousCategory`.
#[derive(Debug, Default)]
pub struct SoundPreview {
    active: Option<InstanceId>,
    previous: Option<usize>,
}

impl SoundPreview {
    /// `SoundPreviewHandler.preview(soundManager, category, volume)`: stop the
    /// preview of another category, then, unless the last preview still
    /// plays, play this category's sound (`SimpleSoundInstance.forUI` at
    /// pitch 1 and the slider's volume).
    pub fn preview(&mut self, sounds: &mut impl Sounds, source: usize, volume: f32) {
        self.stop_other_category_preview(sounds, source);
        if !self.can_play_sound(sounds) {
            return;
        }
        let Some(event) = preview_event(source) else {
            return;
        };
        if let Some(old) = self.active.take() {
            sounds.release(old);
        }
        let instance = SoundInstance::for_ui(event, 1.0, volume, SoundEngine::unseeded());
        self.active = Some(sounds.play_instance(instance, true).0);
    }

    /// The preview playing now, if any.
    pub fn active(&self) -> Option<InstanceId> {
        self.active
    }

    /// `stopOtherCategoryPreview`.
    fn stop_other_category_preview(&mut self, sounds: &mut impl Sounds, source: usize) {
        if self.previous != Some(source) {
            self.previous = Some(source);
            if let Some(active) = self.active {
                sounds.stop_instance(active);
            }
        }
    }

    /// `canPlaySound`: no preview yet, or the last one has finished.
    fn can_play_sound(&self, sounds: &mut impl Sounds) -> bool {
        self.active.is_none_or(|active| !sounds.is_active(active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_engine::{Bound, TickContext, UI, test_backend::Null};
    use crate::sound_events::SoundRegistry;

    fn engine() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"entity.zombie.ambient": {"sounds": ["mob/zombie/say1"]},
                 "entity.cow.ambient": {"sounds": ["mob/cow/say1"]},
                 "ui.button.click": {"sounds": ["random/click"]}}"#,
            &|_, _| true,
        );
        SoundEngine::new(registry)
    }

    fn source(name: &str) -> usize {
        SOUND_SOURCES.iter().position(|s| *s == name).unwrap()
    }

    #[test]
    fn each_category_previews_its_vanilla_sound() {
        let expected = [
            ("master", None),
            ("music", None),
            ("record", Some("minecraft:block.note_block.guitar")),
            ("weather", Some("minecraft:entity.lightning_bolt.thunder")),
            ("block", Some("minecraft:block.grass.place")),
            ("hostile", Some("minecraft:entity.zombie.ambient")),
            ("neutral", Some("minecraft:entity.cow.ambient")),
            ("player", Some("minecraft:entity.generic.eat")),
            ("ambient", Some("minecraft:ambient.cave")),
            ("voice", None),
            ("ui", Some("minecraft:ui.button.click")),
        ];
        for (name, event) in expected {
            assert_eq!(preview_event(source(name)), event, "{name}");
        }
    }

    fn slide(
        preview: &mut SoundPreview,
        engine: &mut SoundEngine,
        backend: &mut Null,
        name: &str,
        volume: f32,
    ) {
        preview.preview(&mut Bound { engine, backend }, source(name), volume);
    }

    /// Ticks enough for the engine to drop finished or stopped channels.
    fn settle(engine: &mut SoundEngine, backend: &mut Null) {
        for _ in 0..40 {
            engine.tick(false, &TickContext::default(), backend);
        }
    }

    #[test]
    fn a_moving_slider_plays_one_preview_at_a_time_and_switching_stops_it() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut preview = SoundPreview::default();
        slide(&mut preview, &mut engine, &mut backend, "hostile", 0.4);
        let first = preview
            .active()
            .expect("the hostile slider plays a preview");
        let instance = engine.instance(first).unwrap();
        assert_eq!(instance.identifier, "minecraft:entity.zombie.ambient");
        assert_eq!(instance.source, UI);
        assert_eq!((instance.volume, instance.pitch), (0.4, 1.0));
        assert!(instance.relative);
        assert!(engine.is_active(first));
        // canPlaySound: still playing, so dragging further does not restart it.
        slide(&mut preview, &mut engine, &mut backend, "hostile", 0.6);
        assert_eq!(preview.active(), Some(first));
        assert_eq!(engine.active_instances(), vec![first]);
        // Once it has finished, the next move plays it again.
        for stopped in backend.stopped.values_mut() {
            *stopped = true;
        }
        settle(&mut engine, &mut backend);
        assert!(!engine.is_active(first));
        slide(&mut preview, &mut engine, &mut backend, "hostile", 0.6);
        let second = preview.active().unwrap();
        assert_ne!(second, first);
        assert!(engine.is_active(second));
        // Another category stops the playing preview (SoundManager.stop); the
        // engine lets its channel go on the next ticks, then that category's
        // own sound plays.
        slide(&mut preview, &mut engine, &mut backend, "neutral", 1.0);
        assert!(backend.stopped.values().all(|stopped| *stopped));
        settle(&mut engine, &mut backend);
        assert!(engine.active_instances().is_empty());
        slide(&mut preview, &mut engine, &mut backend, "neutral", 1.0);
        let third = preview.active().unwrap();
        assert_eq!(
            engine.instance(third).unwrap().identifier,
            "minecraft:entity.cow.ambient"
        );
        // Master has no sound but still stops the other category's preview.
        slide(&mut preview, &mut engine, &mut backend, "master", 1.0);
        settle(&mut engine, &mut backend);
        assert!(engine.active_instances().is_empty());
        slide(&mut preview, &mut engine, &mut backend, "master", 1.0);
        assert!(engine.active_instances().is_empty());
    }
}
