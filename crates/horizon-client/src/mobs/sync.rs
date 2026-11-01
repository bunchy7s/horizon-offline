//! A server's mobs as a client animates them. The server thread, a LAN host
//! and a dedicated server run each mob's animation state (`ai::tick`); a
//! client of a server only gets synched data and entity events, and runs the
//! animation steps vanilla runs on the client (`Entity.onSyncedDataUpdated`,
//! `handleEntityEvent` and the per tick animation states). The server's half
//! is `game/lan.rs` (`mob_meta`, with `pose_id` here for index 6) and
//! `MobEvent::EntityEvent`; the client's half is `game/remote_mob_data.rs`,
//! `game/remote.rs` (`C::EntityEvent`) and `client_tick` here.
//!
//! To animate another species for a client:
//! - send its synched data in `mob_meta` and read it in
//!   `remote_mob_data::apply`;
//! - send its poses in `pose_id` and read them in `set_pose`;
//! - broadcast its entity events with `MobEvent::EntityEvent` and handle
//!   them in `entity_event`;
//! - run its client side animation in `client_tick`, sharing the function
//!   with the server's `ai_step` where the server computes the same.
use super::{
    Mob, Species, allay, armadillo, axolotl, breeze, camel, copper_golem, creaking, equine, frog,
    goat, golem, guardian, piglin, ravager, shulker, sniffer, tame, undead, villager, warden,
};

// `Pose.id` of the poses mobs take.
pub const STANDING: u8 = 0;
pub const SLEEPING: u8 = 2;
pub const CROUCHING: u8 = 5;
pub const LONG_JUMPING: u8 = 6;
pub const CROAKING: u8 = 8;
pub const USING_TONGUE: u8 = 9;
pub const SITTING: u8 = 10;
pub const ROARING: u8 = 11;
pub const SNIFFING: u8 = 12;
pub const EMERGING: u8 = 13;
pub const DIGGING: u8 = 14;
pub const SLIDING: u8 = 15;
pub const SHOOTING: u8 = 16;
pub const INHALING: u8 = 17;

/// `Entity.DATA_POSE` (index 6) of a mob of a server: the pose its species
/// keeps in its own fields.
pub fn pose_id(m: &Mob) -> u8 {
    match m.species() {
        Species::Warden => warden::pose_id(m),
        Species::Breeze => breeze::pose_id(m),
        Species::Camel | Species::CamelHusk if camel::sitting(m) => SITTING,
        Species::Frog | Species::Cat | Species::Ocelot | Species::Goat => m.extra.pose,
        // LivingEntity.startSleeping: the pose, with the bed in index 14.
        Species::Villager if villager::sleeping(m) => SLEEPING,
        _ => STANDING,
    }
}

/// `Entity.onSyncedDataUpdated(DATA_POSE)` on a client.
pub fn set_pose(m: &mut Mob, id: u8) {
    let old = std::mem::replace(&mut m.extra.pose, id);
    match m.species() {
        Species::Frog if old != id => frog::pose_changed(m),
        Species::Warden => warden::set_pose_id(m, id),
        Species::Breeze => breeze::set_pose_id(m, id),
        _ => {}
    }
}

/// The per tick animation steps of a client's mobs (after `m.age` moved on),
/// for the species whose server runs them in `ai_step`. `game_time` is the
/// client level's.
pub fn client_tick(m: &mut Mob, game_time: i64) {
    match m.species() {
        Species::Warden => warden::client_tick(m),
        Species::Sniffer => sniffer::animate(m),
        Species::Breeze => breeze::ai_step(m),
        Species::Camel | Species::CamelHusk => camel::client_tick(m, game_time),
        Species::Shulker => shulker::client_tick(m),
        Species::Ravager => ravager::client_tick(m),
        Species::Hoglin | Species::Zoglin => piglin::client_tick(m),
        Species::IronGolem => golem::client_tick(m),
        Species::Creaking => creaking::animate(m),
        Species::Allay => {
            m.anim.vo = m.anim.v;
            allay::animate(m);
        }
        Species::Axolotl => axolotl::tick_animation(m),
        Species::Guardian | Species::ElderGuardian => {
            m.anim.vo = m.anim.v;
            guardian::tail_and_spikes(m);
        }
        // The head spin's sound is the server's.
        Species::CopperGolem if !m.no_ai => {
            copper_golem::animate(m);
        }
        Species::Armadillo => armadillo::tick(m),
        Species::Frog => frog::tick(m),
        Species::Goat => goat::tick(m),
        // Sheep.aiStep on a client: the eating head's clock.
        Species::Sheep if m.extra.anim_ticks > 0 => m.extra.anim_ticks -= 1,
        s if equine::grazes(s) => equine::client_tick(m),
        Species::Drowned => undead::drowned_client_tick(m),
        // Wolf.tick (alive only): the begging head tilt follows the synched flag.
        Species::Wolf if m.alive() => tame::interest_tick(m),
        _ => {}
    }
}

/// `handleEntityEvent` of the mob types that start animations by an event
/// (`ClientboundEntityEventPacket` for a mob of a client).
pub fn entity_event(m: &mut Mob, event: u8) {
    if m.species() == Species::Armadillo {
        armadillo::entity_event(m, event);
    }
    if m.species() == Species::Goat {
        goat::entity_event(m, event);
    }
    let age = m.age as f32;
    match m.species() {
        Species::Warden => match event {
            // Attack: the roar ends, the swing starts.
            4 => {
                m.anim.states[warden::ROAR].stop();
                m.anim.states[warden::ATTACK].start(age);
            }
            // The tendrils twitch.
            61 => m.anim.v[warden::TENDRIL] = 10.0,
            // The sonic boom.
            62 => m.anim.states[warden::SONIC_BOOM].start(age),
            _ => {}
        },
        Species::Ravager => ravager::entity_event(m, event),
        Species::Hoglin | Species::Zoglin => piglin::client_event(m, event),
        Species::IronGolem => golem::client_event(m, event),
        Species::Creaking => creaking::entity_event(m, event),
        Species::Wolf => tame::entity_event(m, event),
        // Sheep.handleEntityEvent 10: the head goes down to eat grass.
        Species::Sheep if event == 10 => m.extra.anim_ticks = 40,
        _ => {}
    }
}
