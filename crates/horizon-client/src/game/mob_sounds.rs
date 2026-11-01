//! The looping sound instances of mobs: a bee's buzz
//! (`ClientPacketListener.postAddEntitySoundInstance`) and a guardian's
//! attack hum (entity event 21), and the riding sound of a happy ghast or nautilus. The
//! sound engine ticks them with what `start_mob_sounds` and
//! `start_riding_sound` read off the mobs.
use super::Game;
use horizon_client::{
    mobs::{Species, bee, guardian, sniffer},
    sound_engine::{MobSound, RiddenSound, SoundEngine, SoundInstance, Sounds},
};

impl Game<'_> {
    /// `LocalPlayer.startRiding` on a happy ghast or an `AbstractNautilus`:
    /// its riding sound (a minecart's are `start_cart_sounds`'). Then what the
    /// instance reads about the mob the player rides.
    pub(super) fn start_riding_sound(&mut self) -> Option<RiddenSound> {
        let ridden = self
            .riding
            .and_then(|id| self.mobs.iter().find(|m| m.id == id && !m.removed))
            .map(|m| {
                let species = m.species();
                let sound = RiddenSound {
                    id: m.id,
                    // Vec3.length.
                    speed: m.velocity.length(),
                    silent: m.silent,
                    // Entity.isUnderWater: wasEyeInWater && isInWater.
                    underwater: m.was_eye_in_water && m.fluids.in_water,
                };
                (species, sound)
            });
        let started = ridden
            .filter(|r| {
                matches!(
                    r.0,
                    Species::HappyGhast | Species::Nautilus | Species::ZombieNautilus
                )
            })
            .map(|r| r.1);
        if let Some(sound) = started
            && self.riding_sound != Some(sound.id)
            && !sound.silent
        {
            let nautilus = matches!(
                ridden.map(|r| r.0),
                Some(Species::Nautilus | Species::ZombieNautilus)
            );
            let instance = if nautilus {
                SoundInstance::riding_nautilus(&sound, SoundEngine::unseeded())
            } else {
                SoundInstance::riding_happy_ghast(&sound, SoundEngine::unseeded())
            };
            self.audio.play_instance(instance, false);
        }
        self.riding_sound = started.map(|g| g.id);
        ridden.map(|r| r.1)
    }
    /// Start the buzz of each bee that has none (or whose anger changed, which
    /// swaps the flying loop for the aggressive one and back), forget the mobs
    /// that are gone, and read what the instances follow.
    pub(super) fn start_mob_sounds(&mut self) -> Vec<MobSound> {
        let game_time = self.world.game_time;
        let mobs: Vec<MobSound> = self
            .mobs
            .iter()
            .filter(|m| {
                !m.removed
                    && matches!(
                        m.species(),
                        Species::Bee
                            | Species::Guardian
                            | Species::ElderGuardian
                            | Species::Sniffer
                    )
            })
            .map(|m| MobSound {
                id: m.id,
                position: [m.position.x, m.position.y, m.position.z],
                // Vec3.horizontalDistance.
                speed: m.velocity.x.hypot(m.velocity.z),
                baby: m.is_baby(),
                // NeutralMob.isAngry: the anger end time is ahead.
                angry: m.species() == Species::Bee
                    && bee::get(m).is_some_and(|b| b.anger_end > 0 && b.anger_end - game_time > 0),
                silent: m.silent,
                attack_scale: guardian::attack_scale(m),
                // SnifferSoundInstance.tick.
                sniffer_digging: m.species() == Species::Sniffer
                    && m.ai.target.is_none()
                    && sniffer::can_play_digging_sound(m),
            })
            .collect();
        self.guardian_sounds
            .retain(|id| mobs.iter().any(|m| m.id == *id));
        self.bee_sounds
            .retain(|id, _| mobs.iter().any(|m| m.id == *id));
        for sound in &mobs {
            let is_bee = self
                .mobs
                .iter()
                .any(|m| m.id == sound.id && m.species() == Species::Bee);
            if is_bee && self.bee_sounds.get(&sound.id) != Some(&sound.angry) {
                self.bee_sounds.insert(sound.id, sound.angry);
                let instance = SoundInstance::bee(sound, sound.angry, SoundEngine::unseeded());
                self.audio.play_instance(instance, false);
            }
        }
        mobs
    }

    /// `ClientPacketListener.handleEntityEvent` 21: a guardian's attack
    /// hum starts (one loop per guardian; the loops of its earlier beams are
    /// the same loop, as each one follows the guardian's charge).
    pub(super) fn start_guardian_sound(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| {
            m.id == id && matches!(m.species(), Species::Guardian | Species::ElderGuardian)
        }) else {
            return;
        };
        if !self.guardian_sounds.insert(id) {
            return;
        }
        let sound = MobSound {
            id,
            position: [m.position.x, m.position.y, m.position.z],
            speed: 0.0,
            baby: false,
            angry: false,
            silent: m.silent,
            attack_scale: guardian::attack_scale(m),
            sniffer_digging: false,
        };
        let instance = SoundInstance::guardian_attack(&sound, SoundEngine::unseeded());
        self.audio.play_instance(instance, false);
    }
}
