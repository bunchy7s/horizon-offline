//! What changes inside packets between an older release and the pinned one, after their ids are renumbered:
//! registry ids (block states, items, entity types, ...) and the packets whose fields changed. A packet not
//! named here passes byte for byte.
//!
//! Most packets that carry registry ids are decoded with the older version's ids renumbered as they are read
//! ([`ids::renumbering`]) and encoded again for Horizon; ids in plain fields are renumbered between the two.
//! A packet whose layout changed is read in the older layout by hand here.
use super::{
    ids::{self, Ids, Way},
    session::Session,
};
use crate::{
    Result, State,
    buf::{Codec, Reader, Writer},
    chunk::{Paletted, Section, Strategy},
    ids as packet_ids,
    packets::{configuration, play},
};

impl Ids {
    /// The table of a registry tags are sent for, by its name; registries the protocol syncs from the
    /// server's own data (biomes, enchantments, ...) are numbered by that data and alike on both sides.
    pub fn by_registry(&self, registry: &str) -> ids::Remap {
        match registry {
            "minecraft:block" => self.block,
            "minecraft:item" => self.item,
            "minecraft:entity_type" => self.entity_type,
            "minecraft:fluid" => self.fluid,
            "minecraft:game_event" => self.game_event,
            "minecraft:particle_type" => self.particle_type,
            "minecraft:sound_event" => self.sound_event,
            "minecraft:mob_effect" => self.mob_effect,
            "minecraft:attribute" => self.attribute,
            "minecraft:block_entity_type" => self.block_entity_type,
            "minecraft:menu" => self.menu,
            "minecraft:villager_type" => self.villager_type,
            "minecraft:villager_profession" => self.villager_profession,
            "minecraft:data_component_type" => self.data_component_type,
            _ => ids::Remap::Same,
        }
    }
}

/// A clientbound packet, renumbered to the pinned `id`, as the pinned version lays it out: the pinned id
/// it now has (an older packet can become a different one) and its new payload, `None` when the payload
/// stays as it is. `None` altogether drops the packet.
pub fn clientbound(
    session: &Session,
    state: State,
    id: i32,
    payload: &[u8],
) -> Result<Option<(i32, Option<Vec<u8>>)>> {
    let Some(ids) = session.version.ids() else {
        return Ok(Some((id, None)));
    };
    use packet_ids::play::clientbound as cb;
    let out = match (state, id) {
        (State::Configuration, packet_ids::configuration::clientbound::UPDATE_TAGS) => {
            let mut p = configuration::Clientbound::decode(id, payload)?;
            if let configuration::Clientbound::UpdateTags(t) = &mut p {
                tags(ids, &mut t.registries);
            }
            strip_id(p.encode())
        }
        (State::Play, cb::UPDATE_TAGS) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::UpdateTags(t) = &mut p {
                tags(ids, &mut t.registries);
            }
            strip_id(p.encode())
        }
        (State::Play, cb::LOGIN) => login(payload)?,
        (State::Play, cb::RESPAWN) => respawn(payload)?,
        (State::Play, cb::LEVEL_CHUNK_WITH_LIGHT) => chunk(ids, payload)?,
        (State::Play, cb::LIGHT_UPDATE) => light_update(payload)?,
        (State::Play, cb::UPDATE_ADVANCEMENTS) => advancements(ids, payload)?,
        (State::Play, cb::LEVEL_PARTICLES) => particles(ids, payload)?,
        (State::Play, cb::EXPLODE) => {
            // 26.3 adds whether the blast's sound plays, last; 26.2 always played it.
            let mut body = payload.to_vec();
            body.push(1);
            redecode(ids, id, &body)?
        }
        (State::Play, cb::MOVE_ENTITY_POS) => move_entity(payload, true, false)?,
        (State::Play, cb::MOVE_ENTITY_POS_ROT) => move_entity(payload, true, true)?,
        (State::Play, cb::MOVE_ENTITY_ROT) => move_entity(payload, false, true)?,
        (State::Play, cb::ENTITY_POSITION_SYNC) => position_sync(payload)?,
        (State::Play, cb::ANIMATE) => {
            return animate(payload).map(|(id, body)| Some((id, Some(body))));
        }
        (State::Play, cb::ADD_ENTITY) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::AddEntity(e) = &mut p {
                let falling_block = ids.entity_type.down(packet_ids_entity::FALLING_BLOCK);
                if Some(e.kind) == falling_block {
                    e.data = ids.block_state.up(e.data).unwrap_or(0);
                }
                e.kind = ids.entity_type.up(e.kind).unwrap_or(e.kind);
            }
            strip_id(p.encode())
        }
        (State::Play, cb::BLOCK_ENTITY_DATA) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::BlockEntityData(b) = &mut p {
                b.kind = ids.block_entity_type.up(b.kind).unwrap_or(b.kind);
            }
            strip_id(p.encode())
        }
        (State::Play, cb::BLOCK_UPDATE) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::BlockUpdate(b) = &mut p {
                b.state = ids.block_state.up(b.state).unwrap_or(0);
            }
            strip_id(p.encode())
        }
        (State::Play, cb::SECTION_BLOCKS_UPDATE) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::SectionBlocksUpdate(s) = &mut p {
                for change in &mut s.changes {
                    let state = ids.block_state.up((*change >> 12) as i32).unwrap_or(0);
                    *change = i64::from(state) << 12 | (*change & 0xfff);
                }
            }
            strip_id(p.encode())
        }
        (State::Play, cb::LEVEL_EVENT) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::LevelEvent(e) = &mut p {
                // LevelEvent.PARTICLES_DESTROY_BLOCK and the brush's BLOCK_FINISHED_BRUSHING carry a state.
                if matches!(e.kind, 2001 | 3008) {
                    e.data = ids.block_state.up(e.data).unwrap_or(0);
                }
            }
            strip_id(p.encode())
        }
        (State::Play, cb::UPDATE_ATTRIBUTES) => {
            let mut p = decode_play(ids, id, payload)?;
            if let play::Clientbound::UpdateAttributes(a) = &mut p {
                // An attribute the pinned version lacks can't be: 26.3 only added some.
                for v in &mut a.values {
                    v.attribute = ids.attribute.up(v.attribute).unwrap_or(v.attribute);
                }
            }
            strip_id(p.encode())
        }
        (
            State::Play,
            cb::SET_ENTITY_DATA
            | cb::CONTAINER_SET_CONTENT
            | cb::CONTAINER_SET_SLOT
            | cb::SET_CURSOR_ITEM
            | cb::SET_PLAYER_INVENTORY
            | cb::SET_EQUIPMENT
            | cb::MERCHANT_OFFERS
            | cb::SOUND
            | cb::SOUND_ENTITY
            | cb::RECIPE_BOOK_ADD
            | cb::PLACE_GHOST_RECIPE,
        ) => redecode(ids, id, payload)?,
        _ => return Ok(Some((id, None))),
    };
    Ok(Some((id, Some(out))))
}

/// Pinned entity type ids the translation looks for.
mod packet_ids_entity {
    /// `minecraft:falling_block` in 26.3.
    pub const FALLING_BLOCK: i32 = 52;
}

/// A serverbound packet with Horizon's `id`, as the older server numbers and lays it out: its id there and
/// the payload. `None`: nothing is sent.
pub fn serverbound(
    session: &Session,
    state: State,
    id: i32,
    payload: &[u8],
) -> Result<Option<(i32, Vec<u8>)>> {
    let renumbered = |id: i32, payload: Vec<u8>| {
        Ok(session
            .version
            .serverbound_id(state, id)
            .map(|mapped| (mapped, payload)))
    };
    let Some(ids) = session.version.ids() else {
        return renumbered(id, payload.to_vec());
    };
    use packet_ids::play::serverbound as sb;
    match (state, id) {
        // 26.3 also states where the client was teleported to; 26.2 only the teleport's id.
        (State::Play, sb::ACCEPT_TELEPORTATION) => {
            let mut r = Reader::new(payload);
            let mut w = Writer::new();
            w.varint(r.varint()?);
            renumbered(id, w.data)
        }
        // 26.3 inserted CHANGE_DESTROY_DIRECTION at 1; 26.2 has no such action.
        (State::Play, sb::PLAYER_ACTION) => {
            let mut r = Reader::new(payload);
            let action = r.varint()?;
            let action = match action {
                0 => 0,
                1 => return Ok(None),
                n => n - 1,
            };
            let mut w = Writer::new();
            w.varint(action);
            w.bytes(r.rest());
            renumbered(id, w.data)
        }
        // 26.3's punch took over 26.2's swing of the main hand.
        (State::Play, sb::PUNCH) => {
            let mut w = Writer::new();
            w.varint(0);
            Ok(Some((V26_2_SWING, w.data)))
        }
        (State::Play, sb::SET_CREATIVE_MODE_SLOT | sb::CONTAINER_CLICK) => {
            let p = play::Serverbound::decode(id, payload)?;
            let payload = {
                let _down = ids::renumbering(ids, Way::Down);
                strip_id(p.encode())
            };
            renumbered(id, payload)
        }
        _ => renumbered(id, payload.to_vec()),
    }
}

/// 26.2's serverbound `swing` (play), which 26.3 replaced with `punch`.
const V26_2_SWING: i32 = 63;

/// A play packet decoded with the older version's ids renumbered as they are read.
fn decode_play(ids: &'static Ids, id: i32, payload: &[u8]) -> Result<play::Clientbound> {
    let _up = ids::renumbering(ids, Way::Up);
    play::Clientbound::decode(id, payload)
}

/// The packet's payload once its registry ids are renumbered by decoding it.
fn redecode(ids: &'static Ids, id: i32, payload: &[u8]) -> Result<Vec<u8>> {
    let p = decode_play(ids, id, payload)?;
    Ok(strip_id(p.encode()))
}

fn strip_id(body: Vec<u8>) -> Vec<u8> {
    let mut r = Reader::new(&body);
    match r.varint() {
        Ok(_) => r.rest().to_vec(),
        Err(_) => body,
    }
}

fn tags(ids: &Ids, registries: &mut [configuration::RegistryTags]) {
    for registry in registries {
        let remap = ids.by_registry(&registry.registry);
        for tag in &mut registry.tags {
            tag.ids = tag.ids.iter().filter_map(|&id| remap.up(id)).collect();
        }
    }
}

/// 26.2's CommonPlayerSpawnInfo, read into Horizon's: the game types went from bytes (-1 for no previous
/// one) to VarInts (an optional one as id + 1).
fn spawn_info(r: &mut Reader) -> Result<play::SpawnInfo> {
    let dimension_type = r.varint()?;
    let dimension = r.string(32767)?;
    let seed = r.i64()?;
    let game_type = i32::from(r.i8()?);
    let previous = r.i8()?;
    let debug = r.bool()?;
    let flat = r.bool()?;
    let last_death = <crate::buf::Opt<play::GlobalPos>>::read(r)?;
    Ok(play::SpawnInfo {
        dimension_type,
        dimension,
        seed,
        game_type,
        previous_game_type: (previous >= 0).then_some(i32::from(previous)),
        debug,
        flat,
        last_death,
        portal_cooldown: r.varint()?,
        sea_level: r.varint()?,
    })
}

fn login(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.i32(r.i32()?);
    w.bool(r.bool()?);
    let n = r.count(r.remaining())?;
    w.varint(n as i32);
    for _ in 0..n {
        w.string(&r.string(32767)?);
    }
    for _ in 0..3 {
        w.varint(r.varint()?);
    }
    for _ in 0..3 {
        w.bool(r.bool()?);
    }
    play::SpawnInfo::write(&spawn_info(&mut r)?, &mut w);
    w.bytes(r.rest());
    Ok(w.data)
}

fn respawn(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    play::SpawnInfo::write(&spawn_info(&mut r)?, &mut w);
    w.bytes(r.rest());
    Ok(w.data)
}

/// The column's sections with their block states renumbered and packed for the pinned palette width (a
/// section on the older global palette uses fewer bits per entry), and its light masks as 26.3 sends them.
/// Biomes are the server's own registry and stay as they are.
fn chunk(ids: &'static Ids, payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let x = r.i32()?;
    let z = r.i32()?;
    let heightmaps = crate::chunk::Heightmaps::read(&mut r)?;
    let buffer = r.byte_array(r.remaining())?;
    let mut block_entities = <crate::buf::List<crate::chunk::BlockEntityInfo>>::read(&mut r)?;
    // A block entity type the pinned version dropped (26.1's bed) goes; the block still draws.
    block_entities.retain_mut(|b| match ids.block_entity_type.up(b.kind) {
        Some(kind) => {
            b.kind = kind;
            true
        }
        None => false,
    });
    let light = light(&mut r)?;
    let (old_bits, new_bits) = ids.state_bits();
    let mut sections = Reader::new(&buffer);
    let mut out = Vec::new();
    while sections.remaining() > 0 {
        let mut s = Section::read(&mut sections, old_bits, 0)?;
        s.states = renumber_states(ids, &s.states, new_bits);
        out.push(s);
    }
    let mut w = Writer::new();
    play::LevelChunkWithLight::write(
        &play::LevelChunkWithLight {
            x,
            z,
            chunk: crate::chunk::ChunkData {
                heightmaps,
                buffer: crate::chunk::ChunkData::from_sections(&out),
                block_entities,
            },
            light,
        },
        &mut w,
    );
    Ok(w.data)
}

/// 26.2's ClientboundLightUpdatePacketData: its four masks are `BitSet.toLongArray` (a VarInt count of
/// longs); 26.3 sends `toByteArray`, the same bits as little-endian bytes without trailing zeros.
fn light(r: &mut Reader) -> Result<crate::chunk::LightData> {
    let mut mask = || -> Result<Vec<u8>> {
        let longs = r.long_array()?;
        let mut bytes: Vec<u8> = longs.iter().flat_map(|l| l.to_le_bytes()).collect();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        bytes.truncate(len);
        Ok(bytes)
    };
    let sky_mask = mask()?;
    let block_mask = mask()?;
    let empty_sky_mask = mask()?;
    let empty_block_mask = mask()?;
    Ok(crate::chunk::LightData {
        sky_mask,
        block_mask,
        empty_sky_mask,
        empty_block_mask,
        sky: crate::chunk::Layers::read(r)?,
        block: crate::chunk::Layers::read(r)?,
    })
}

fn light_update(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let x = r.varint()?;
    let z = r.varint()?;
    let light = light(&mut r)?;
    let mut w = Writer::new();
    play::LightUpdate::write(&play::LightUpdate { x, z, light }, &mut w);
    Ok(w.data)
}

fn renumber_states(ids: &Ids, states: &Paletted, new_bits: u8) -> Paletted {
    let up = |id: i32| ids.block_state.up(id).unwrap_or(0);
    if !states.palette.is_empty() {
        // A palette: renumbering its entries is enough.
        let mut out = states.clone();
        out.palette.iter_mut().for_each(|p| *p = up(*p));
        return out;
    }
    let values: Vec<i32> = (0..crate::chunk::BLOCKS)
        .map(|i| up(states.get(i)))
        .collect();
    Paletted::from_values(&values, Strategy::Blocks, new_bits)
}

/// 26.2 keeps an advancement's tab position in its display (two floats after the background); 26.3 sends
/// it after the whole advancement, displayed or not. What follows the added advancements is unchanged.
fn advancements(ids: &'static Ids, payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.bool(r.bool()?);
    let n = r.count(r.remaining())?;
    w.varint(n as i32);
    for _ in 0..n {
        w.string(&r.string(32767)?);
        let parent = r.bool()?;
        w.bool(parent);
        if parent {
            w.string(&r.string(32767)?);
        }
        let mut position = (0.0, 0.0);
        let display = r.bool()?;
        w.bool(display);
        if display {
            w.nbt(&r.nbt()?);
            w.nbt(&r.nbt()?);
            let icon = {
                let _up = ids::renumbering(ids, Way::Up);
                crate::item::Template::read(&mut r)?
            };
            crate::item::Template::write(&icon, &mut w);
            w.varint(r.varint()?);
            let flags = r.i32()?;
            w.i32(flags);
            if flags & 1 != 0 {
                w.string(&r.string(32767)?);
            }
            position = (r.f32()?, r.f32()?);
        }
        let groups = r.count(r.remaining())?;
        w.varint(groups as i32);
        for _ in 0..groups {
            let k = r.count(r.remaining())?;
            w.varint(k as i32);
            for _ in 0..k {
                w.string(&r.string(32767)?);
            }
        }
        w.bool(r.bool()?);
        w.f32(position.0);
        w.f32(position.1);
    }
    w.bytes(r.rest());
    Ok(w.data)
}

/// 26.2: flags, position, spread, one max speed, an Int count, then the particle. 26.3 sends the particle
/// first, a speed per axis, a VarInt count and how the spread is drawn (0, gaussian, as 26.2 did).
fn particles(ids: &'static Ids, payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let override_limiter = r.bool()?;
    let always_show = r.bool()?;
    let pos = [r.f64()?, r.f64()?, r.f64()?];
    let dist = [r.f32()?, r.f32()?, r.f32()?];
    let speed = r.f32()?;
    let count = r.i32()?;
    let particle = {
        let _up = ids::renumbering(ids, Way::Up);
        crate::entity_data::Particle::read(&mut r)?
    };
    let mut w = Writer::new();
    play::LevelParticles::write(
        &play::LevelParticles {
            particle,
            override_limiter,
            always_show,
            pos,
            dist,
            max_speed: [speed; 3],
            count,
            randomization: 0,
        },
        &mut w,
    );
    Ok(w.data)
}

/// 26.2 sends a move as three Short deltas (1/4096 of a block) then on-ground, and a turn as the two angles
/// then on-ground; 26.3 packs on-ground with the delta's step count, and sends it before the angles.
fn move_entity(payload: &[u8], pos: bool, rot: bool) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let id = r.varint()?;
    let delta = if pos {
        Some(play::VecDelta::Linear([r.i16()?, r.i16()?, r.i16()?]))
    } else {
        None
    };
    let rotation = if rot { Some((r.i8()?, r.i8()?)) } else { None };
    let on_ground = r.bool()?;
    let m = play::MoveEntity {
        id,
        delta,
        rotation,
        on_ground,
    };
    let mut w = Writer::new();
    match (pos, rot) {
        (true, false) => play::MoveEntityPos::write(&play::MoveEntityPos(m), &mut w),
        (true, true) => play::MoveEntityPosRot::write(&play::MoveEntityPosRot(m), &mut w),
        _ => play::MoveEntityRot::write(&play::MoveEntityRot(m), &mut w),
    }
    Ok(w.data)
}

/// 26.2: the entity, PositionMoveRotation (position, motion, the two angles), on-ground. 26.3 sends where it
/// goes as a path, then the angles; the motion is no longer sent.
fn position_sync(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let id = r.varint()?;
    let position = [r.f64()?, r.f64()?, r.f64()?];
    let _motion = [r.f64()?, r.f64()?, r.f64()?];
    let yaw = r.f32()?;
    let pitch = r.f32()?;
    let on_ground = r.bool()?;
    let mut w = Writer::new();
    play::EntityPositionSync::write(
        &play::EntityPositionSync {
            id,
            path: play::PositionPath::Linear(position),
            yaw,
            pitch,
            on_ground,
        },
        &mut w,
    );
    Ok(w.data)
}

/// 26.2's animate swings hands too (0 main, 3 off), which 26.3 sends as `swing_animation`; the others were
/// renumbered (wake up 2 -> 0, critical hits 4, 5 -> 1, 2).
fn animate(payload: &[u8]) -> Result<(i32, Vec<u8>)> {
    let mut r = Reader::new(payload);
    let id = r.varint()?;
    let action = r.u8()?;
    let mut w = Writer::new();
    if action == 0 || action == 3 {
        // A plain swing: SwingAnimationType WHACK over the default 6 ticks.
        play::SwingAnimation::write(
            &play::SwingAnimation {
                id,
                hand: i32::from(action == 3),
                kind: 1,
                duration: 6,
            },
            &mut w,
        );
        return Ok((packet_ids::play::clientbound::SWING_ANIMATION, w.data));
    }
    let action = match action {
        2 => 0,
        4 => 1,
        5 => 2,
        other => other,
    };
    w.varint(id);
    w.u8(action);
    Ok((packet_ids::play::clientbound::ANIMATE, w.data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> &'static Ids {
        super::super::ServerVersion::V26_2.ids().unwrap()
    }

    #[test]
    fn a_26_2_chunk_section_on_the_global_palette_is_repacked_wider() {
        let ids = ids();
        // Every block a different state from 0..4096: past 8 bits, so the global palette at 15 bits.
        let values: Vec<i32> = (0..4096).map(|i| 13000 + i).collect();
        let old = Paletted::from_values(&values, Strategy::Blocks, 15);
        assert_eq!(old.bits, 15);
        let new = renumber_states(ids, &old, 16);
        assert_eq!(new.bits, 16);
        // oak_slab's first state, 13330 in 26.2, is 15177 in 26.3.
        assert_eq!(new.get(330), 15177);
        assert_eq!(new.get(0), ids.block_state.up(13000).unwrap());
    }

    #[test]
    fn a_26_2_move_reads_as_26_3s() {
        let mut w = Writer::new();
        w.varint(7);
        w.i16(4096);
        w.i16(0);
        w.i16(-2048);
        w.bool(true);
        let body = move_entity(&w.data, true, false).unwrap();
        let p = play::Clientbound::decode(packet_ids::play::clientbound::MOVE_ENTITY_POS, &body)
            .unwrap();
        assert_eq!(
            p,
            play::Clientbound::MoveEntityPos(play::MoveEntityPos(play::MoveEntity {
                id: 7,
                delta: Some(play::VecDelta::Linear([4096, 0, -2048])),
                rotation: None,
                on_ground: true,
            }))
        );
        let mut w = Writer::new();
        w.varint(7);
        w.i8(10);
        w.i8(-20);
        w.bool(false);
        let body = move_entity(&w.data, false, true).unwrap();
        let p = play::Clientbound::decode(packet_ids::play::clientbound::MOVE_ENTITY_ROT, &body)
            .unwrap();
        assert_eq!(
            p,
            play::Clientbound::MoveEntityRot(play::MoveEntityRot(play::MoveEntity {
                id: 7,
                delta: None,
                rotation: Some((10, -20)),
                on_ground: false,
            }))
        );
    }

    #[test]
    fn a_26_2_swing_becomes_a_swing_animation() {
        let (id, body) = animate(&[5, 3]).unwrap();
        assert_eq!(id, packet_ids::play::clientbound::SWING_ANIMATION);
        let p = play::Clientbound::decode(id, &body).unwrap();
        assert_eq!(
            p,
            play::Clientbound::SwingAnimation(play::SwingAnimation {
                id: 5,
                hand: 1,
                kind: 1,
                duration: 6
            })
        );
        assert_eq!(
            animate(&[5, 4]).unwrap(),
            (packet_ids::play::clientbound::ANIMATE, vec![5, 1])
        );
    }

    #[test]
    fn falling_blocks_are_known_in_both_numberings() {
        let ids = ids();
        assert_eq!(
            ids.entity_type
                .down(packet_ids_entity::FALLING_BLOCK)
                .and_then(|o| ids.entity_type.up(o)),
            Some(packet_ids_entity::FALLING_BLOCK)
        );
    }

    #[test]
    fn a_26_2_light_mask_of_longs_becomes_26_3s_bytes() {
        let mut w = Writer::new();
        w.varint(3);
        w.varint(-2);
        // Sky mask: sections 0 and 2 (one long, 0b101); block mask: section 9 (bit 9); both empty masks none.
        for mask in [&[0b101_i64][..], &[1 << 9], &[], &[]] {
            w.varint(mask.len() as i32);
            mask.iter().for_each(|&l| w.i64(l));
        }
        w.varint(0);
        w.varint(0);
        let body = light_update(&w.data).unwrap();
        let p =
            play::Clientbound::decode(packet_ids::play::clientbound::LIGHT_UPDATE, &body).unwrap();
        let play::Clientbound::LightUpdate(l) = p else {
            panic!("{p:?}");
        };
        assert_eq!((l.x, l.z), (3, -2));
        assert_eq!(l.light.sky_mask, vec![0b101]);
        assert_eq!(l.light.block_mask, vec![0, 0b10]);
        assert!(l.light.empty_sky_mask.is_empty());
    }

    #[test]
    fn a_26_3_destroy_direction_change_is_not_sent_to_26_2() {
        let session = Session::new(super::super::ServerVersion::V26_2);
        let action = |n: i32| {
            let mut w = Writer::new();
            w.varint(n);
            w.i64(0);
            w.u8(1);
            serverbound(
                &session,
                State::Play,
                packet_ids::play::serverbound::PLAYER_ACTION,
                &w.data,
            )
            .unwrap()
            .map(|(_, payload)| payload[0])
        };
        assert_eq!(action(0), Some(0)); // start destroying
        assert_eq!(action(1), None); // change_destroy_direction, new in 26.3
        assert_eq!(action(3), Some(2)); // stop destroying: 3 in 26.3, 2 in 26.2
    }
}
