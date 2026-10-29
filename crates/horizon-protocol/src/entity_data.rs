//! SynchedEntityData values (ClientboundSetEntityDataPacket): an entry index,
//! the EntityDataSerializers id, and the serializer's value, ending with 0xFF.
//! Serializers Horizon cannot decode end the list early; the entries before
//! them are kept.
use crate::{
    Result,
    buf::{Codec, Reader, Writer},
    item::{Slot, Stack, Template},
};
use horizon_world::nbt::Tag;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(i8),
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    Text(Tag),
    OptText(Option<Tag>),
    Item(Option<Stack>),
    Bool(bool),
    Floats(Vec<f32>),
    Pos([i32; 3]),
    OptPos(Option<[i32; 3]>),
    OptUuid(Option<u128>),
    /// Any VarInt-valued serializer (direction, block state, pose, variants ...).
    Id(i32),
    /// Several VarInts (VillagerData).
    Ids(Vec<i32>),
    /// A particle choice: the `minecraft:particle_type` id plus its data.
    /// Rendered nowhere (the client spawns no world particles yet); decoded
    /// so the rest of an entry list survives a particle slot.
    Particle(Particle),
    /// A particle list (PARTICLES: a VarInt count, then each particle).
    Particles(Vec<Particle>),
    /// A `ResolvableProfile` (a mannequin's DATA_PROFILE).
    Profile(Profile),
}
/// `ResolvableProfile.STREAM_CODEC`: the GameProfile a mannequin or player
/// head resolves — either a full profile (id, name and properties on the
/// wire) or a partial one (a name and/or id to look up) — followed by a
/// `PlayerSkin.Patch` of texture overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub name: Option<String>,
    pub uuid: Option<u128>,
    /// `PropertyMap`: its `textures` property carries the skin.
    pub properties: Vec<crate::packets::login::Property>,
    /// `PlayerSkin.Patch`: texture asset ids overriding the profile's.
    pub body: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    /// `PlayerModelType`: true is the slim (4px arms) model.
    pub slim: Option<bool>,
}
/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`.
fn read_properties(r: &mut Reader) -> Result<Vec<crate::packets::login::Property>> {
    use crate::buf::Codec;
    let n = r.count(16)?;
    (0..n)
        .map(|_| crate::packets::login::Property::read(r))
        .collect()
}
fn write_properties(p: &[crate::packets::login::Property], w: &mut Writer) {
    use crate::buf::Codec;
    w.varint(p.len() as i32);
    p.iter()
        .for_each(|p| crate::packets::login::Property::write(p, w));
}
fn read_profile(r: &mut Reader) -> Result<Profile> {
    let mut p = Profile::default();
    // ByteBufCodecs.either: true is the full GameProfile, false a Partial.
    if r.bool()? {
        p.uuid = Some(r.uuid()?);
        p.name = Some(r.string(16)?);
        p.properties = read_properties(r)?;
    } else {
        if r.bool()? {
            p.name = Some(r.string(16)?);
        }
        if r.bool()? {
            p.uuid = Some(r.uuid()?);
        }
        p.properties = read_properties(r)?;
    }
    // PlayerSkin.Patch: three optional texture ids, then the model type.
    if r.bool()? {
        p.body = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.cape = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.elytra = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.slim = Some(r.bool()?);
    }
    Ok(p)
}
fn write_profile(p: &Profile, w: &mut Writer) {
    let full = p.uuid.is_some() && p.name.is_some();
    w.bool(full);
    if full {
        w.uuid(p.uuid.unwrap_or(0));
        w.string(p.name.as_deref().unwrap_or(""));
    } else {
        w.bool(p.name.is_some());
        if let Some(name) = &p.name {
            w.string(name);
        }
        w.bool(p.uuid.is_some());
        if let Some(uuid) = p.uuid {
            w.uuid(uuid);
        }
    }
    write_properties(&p.properties, w);
    for texture in [&p.body, &p.cape, &p.elytra] {
        w.bool(texture.is_some());
        if let Some(texture) = texture {
            w.string(texture);
        }
    }
    w.bool(p.slim.is_some());
    if let Some(slim) = p.slim {
        w.bool(slim);
    }
}
/// One `ParticleTypes.STREAM_CODEC` value: the `minecraft:particle_type` id,
/// then the data its type carries (26.3 registry order).
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub id: i32,
    pub data: Option<ParticleData>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleData {
    /// Block state id (block, block_marker, falling_dust, dust_pillar,
    /// block_crumble).
    State(i32),
    /// ARGB color (entity_effect, tinted_leaves, flash).
    Color(i32),
    /// Dust: color then scale.
    ColorScale(i32, f32),
    /// Dust color transition: from color, to color, scale.
    ColorScale2(i32, i32, f32),
    /// Spell (effect, instant_effect): color then power.
    ColorPower(i32, f32),
    /// A bare float (dragon_breath power, sculk_charge roll).
    Scalar(f32),
    /// Delay in ticks (shriek).
    Delay(i32),
    /// Trail: target (3 doubles), ARGB color, duration.
    Trail([f64; 3], i32, i32),
    /// An item (ItemStackTemplate: item, count, patch).
    Item(Stack),
    /// Geyser and geyser_plume: water blocks.
    Geyser(i32),
    /// Geyser_base and geyser_poof: water blocks, then the burst impulse.
    GeyserBase(i32, f32),
    /// Vibration: where it travels to, then its arrival time in ticks.
    Vibration { source: PositionSource, ticks: i32 },
}
/// `PositionSource.STREAM_CODEC`: a `minecraft:position_source_type` id
/// (block 0, entity 1), then that type's data.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionSource {
    Block([i32; 3]),
    Entity { id: i32, y_offset: f32 },
}
impl Particle {
    pub fn read(r: &mut Reader) -> Result<Particle> {
        let id = r.varint()?;
        // `minecraft:particle_type` registry order as of 26.3.
        let data = match id {
            1 | 2 | 36 | 121 | 125 => Some(ParticleData::State(r.varint()?)),
            28 | 46 | 52 => Some(ParticleData::Color(r.i32()?)),
            21 => {
                let color = r.i32()?;
                Some(ParticleData::ColorScale(color, r.f32()?))
            }
            22 => {
                let from = r.i32()?;
                let to = r.i32()?;
                Some(ParticleData::ColorScale2(from, to, r.f32()?))
            }
            23 | 56 => {
                let color = r.i32()?;
                Some(ParticleData::ColorPower(color, r.f32()?))
            }
            15 | 48 => Some(ParticleData::Scalar(r.f32()?)),
            115 => Some(ParticleData::Delay(r.varint()?)),
            59 => {
                let at = [r.f64()?, r.f64()?, r.f64()?];
                let color = r.i32()?;
                Some(ParticleData::Trail(at, color, r.varint()?))
            }
            57 => Some(ParticleData::Item(Template::read(r)?)),
            58 => {
                let source = match r.varint()? {
                    0 => PositionSource::Block(crate::buf::Pos::read(r)?),
                    1 => {
                        let id = r.varint()?;
                        PositionSource::Entity {
                            id,
                            y_offset: r.f32()?,
                        }
                    }
                    kind => return Err(crate::Error::new(format!("position source type {kind}"))),
                };
                Some(ParticleData::Vibration {
                    source,
                    ticks: r.varint()?,
                })
            }
            7 | 10 => Some(ParticleData::Geyser(r.i32()?)),
            8 | 9 => {
                let water = r.i32()?;
                Some(ParticleData::GeyserBase(water, r.f32()?))
            }
            // Every other registered particle carries no data.
            _ => None,
        };
        Ok(Particle { id, data })
    }
    pub fn write(&self, w: &mut Writer) {
        w.varint(self.id);
        let Some(data) = &self.data else { return };
        match data {
            ParticleData::State(s) | ParticleData::Delay(s) => w.varint(*s),
            ParticleData::Color(c) | ParticleData::Geyser(c) => w.i32(*c),
            ParticleData::ColorScale(c, f)
            | ParticleData::ColorPower(c, f)
            | ParticleData::GeyserBase(c, f) => {
                w.i32(*c);
                w.f32(*f);
            }
            ParticleData::ColorScale2(a, b, f) => {
                w.i32(*a);
                w.i32(*b);
                w.f32(*f);
            }
            ParticleData::Scalar(f) => w.f32(*f),
            ParticleData::Trail(at, c, d) => {
                at.iter().for_each(|v| w.f64(*v));
                w.i32(*c);
                w.varint(*d);
            }
            ParticleData::Item(s) => Template::write(s, w),
            ParticleData::Vibration { source, ticks } => {
                match source {
                    PositionSource::Block(pos) => {
                        w.varint(0);
                        crate::buf::Pos::write(pos, w);
                    }
                    PositionSource::Entity { id, y_offset } => {
                        w.varint(1);
                        w.varint(*id);
                        w.f32(*y_offset);
                    }
                }
                w.varint(*ticks);
            }
        }
    }
}
/// Serializer ids in EntityDataSerializers registration order.
pub mod serializer {
    pub const BYTE: i32 = 0;
    pub const INT: i32 = 1;
    pub const LONG: i32 = 2;
    pub const FLOAT: i32 = 3;
    pub const STRING: i32 = 4;
    pub const COMPONENT: i32 = 5;
    pub const OPTIONAL_COMPONENT: i32 = 6;
    pub const ITEM_STACK: i32 = 7;
    pub const BOOLEAN: i32 = 8;
    pub const ROTATIONS: i32 = 9;
    pub const BLOCK_POS: i32 = 10;
    pub const OPTIONAL_BLOCK_POS: i32 = 11;
    pub const DIRECTION: i32 = 12;
    pub const OPTIONAL_LIVING_ENTITY_REFERENCE: i32 = 13;
    pub const BLOCK_STATE: i32 = 14;
    pub const OPTIONAL_BLOCK_STATE: i32 = 15;
    pub const PARTICLE: i32 = 16;
    pub const PARTICLES: i32 = 17;
    pub const VILLAGER_DATA: i32 = 18;
    pub const OPTIONAL_UNSIGNED_INT: i32 = 19;
    pub const POSE: i32 = 20;
    pub const CAT_VARIANT: i32 = 21;
    pub const CAT_SOUND_VARIANT: i32 = 22;
    pub const COW_VARIANT: i32 = 23;
    pub const COW_SOUND_VARIANT: i32 = 24;
    pub const WOLF_VARIANT: i32 = 25;
    pub const WOLF_SOUND_VARIANT: i32 = 26;
    pub const FROG_VARIANT: i32 = 27;
    pub const PIG_VARIANT: i32 = 28;
    pub const PIG_SOUND_VARIANT: i32 = 29;
    pub const CHICKEN_VARIANT: i32 = 30;
    pub const CHICKEN_SOUND_VARIANT: i32 = 31;
    pub const ZOMBIE_NAUTILUS_VARIANT: i32 = 32;
    pub const OPTIONAL_GLOBAL_POS: i32 = 33;
    pub const PAINTING_VARIANT: i32 = 34;
    pub const SNIFFER_STATE: i32 = 35;
    pub const ARMADILLO_STATE: i32 = 36;
    pub const COPPER_GOLEM_STATE: i32 = 37;
    pub const WEATHERING_COPPER_STATE: i32 = 38;
    pub const VECTOR3: i32 = 39;
    pub const QUATERNION: i32 = 40;
    pub const RESOLVABLE_PROFILE: i32 = 41;
    pub const HUMANOID_ARM: i32 = 42;
    pub const DYE_COLOR: i32 = 43;
}
fn read_value(kind: i32, r: &mut Reader) -> Result<Option<Value>> {
    use serializer::*;
    Ok(Some(match kind {
        BYTE => Value::Byte(r.i8()?),
        INT => Value::Int(r.varint()?),
        LONG => Value::Long(r.varlong()?),
        FLOAT => Value::Float(r.f32()?),
        STRING => Value::String(r.string(32767)?),
        COMPONENT => Value::Text(r.nbt()?),
        OPTIONAL_COMPONENT => Value::OptText(if r.bool()? { Some(r.nbt()?) } else { None }),
        ITEM_STACK => Value::Item(Slot::read(r)?),
        BOOLEAN => Value::Bool(r.bool()?),
        ROTATIONS | VECTOR3 => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?]),
        QUATERNION => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        BLOCK_POS => Value::Pos(crate::buf::Pos::read(r)?),
        OPTIONAL_BLOCK_POS => Value::OptPos(if r.bool()? {
            Some(crate::buf::Pos::read(r)?)
        } else {
            None
        }),
        OPTIONAL_LIVING_ENTITY_REFERENCE => {
            Value::OptUuid(if r.bool()? { Some(r.uuid()?) } else { None })
        }
        // VillagerData: type, profession and level.
        VILLAGER_DATA => Value::Ids(vec![r.varint()?, r.varint()?, r.varint()?]),
        // VarInt-encoded: direction, block states, optional ints, pose, the
        // holder-registry variants (cat to zombie nautilus, painting) and the
        // enum states (sniffer to weathering copper), arm, dye.
        DIRECTION
        | BLOCK_STATE
        | OPTIONAL_BLOCK_STATE
        | OPTIONAL_UNSIGNED_INT
        | POSE
        | CAT_VARIANT..=ZOMBIE_NAUTILUS_VARIANT
        | PAINTING_VARIANT..=WEATHERING_COPPER_STATE
        | HUMANOID_ARM
        | DYE_COLOR => Value::Id(r.varint()?),
        PARTICLE => Value::Particle(Particle::read(r)?),
        RESOLVABLE_PROFILE => Value::Profile(read_profile(r)?),
        PARTICLES => {
            let n = r.count(65536)?;
            let mut list = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                list.push(Particle::read(r)?);
            }
            Value::Particles(list)
        }
        _ => return Ok(None),
    }))
}
fn write_value(kind: i32, v: &Value, w: &mut Writer) {
    match v {
        Value::Byte(b) => w.i8(*b),
        Value::Int(i) | Value::Id(i) => w.varint(*i),
        Value::Ids(ids) => ids.iter().for_each(|i| w.varint(*i)),
        Value::Long(l) => w.varlong(*l),
        Value::Float(f) => w.f32(*f),
        Value::String(s) => w.string(s),
        Value::Text(t) => w.nbt(t),
        Value::OptText(t) => {
            w.bool(t.is_some());
            if let Some(t) = t {
                w.nbt(t);
            }
        }
        Value::Item(s) => Slot::write(s, w),
        Value::Bool(b) => w.bool(*b),
        Value::Floats(f) => f.iter().for_each(|f| w.f32(*f)),
        Value::Pos(p) => crate::buf::Pos::write(p, w),
        Value::OptPos(p) => {
            w.bool(p.is_some());
            if let Some(p) = p {
                crate::buf::Pos::write(p, w);
            }
        }
        Value::OptUuid(u) => {
            w.bool(u.is_some());
            if let Some(u) = u {
                w.uuid(*u);
            }
        }
        Value::Particle(p) => p.write(w),
        Value::Particles(list) => {
            w.varint(list.len() as i32);
            list.iter().for_each(|p| p.write(w));
        }
        Value::Profile(p) => write_profile(p, w),
    }
    let _ = kind;
}

/// (index, serializer id, value) entries.
pub struct EntityData;
impl Codec<Vec<(u8, i32, Value)>> for EntityData {
    fn read(r: &mut Reader) -> Result<Vec<(u8, i32, Value)>> {
        let mut out = Vec::new();
        loop {
            let index = r.u8()?;
            if index == 0xFF {
                return Ok(out);
            }
            let kind = r.varint()?;
            match read_value(kind, r)? {
                Some(v) => out.push((index, kind, v)),
                None => {
                    r.rest();
                    return Ok(out);
                }
            }
        }
    }
    fn write(v: &Vec<(u8, i32, Value)>, w: &mut Writer) {
        for (index, kind, value) in v {
            w.u8(*index);
            w.varint(*kind);
            write_value(*kind, value, w);
        }
        w.u8(0xFF);
    }
}

/// ItemEntity.DATA_ITEM (after the eight Entity data entries).
pub const ITEM_ENTITY_ITEM: u8 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_stops_at_unknown_serializers() {
        let entries = vec![
            (0u8, serializer::BYTE, Value::Byte(0x02)),
            (1, serializer::INT, Value::Int(300)),
            (2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)),
            (6, serializer::POSE, Value::Id(5)),
            (
                8,
                serializer::ITEM_STACK,
                Value::Item(Some(Stack::new(3, 12))),
            ),
        ];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(*w.data.last().unwrap(), 0xFF);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
        // Serializer 200 is unknown: the list stops there, earlier entries kept.
        let mut bytes = vec![0, 0, 7, 9, 200, 1, 2, 3];
        bytes.push(0xFF);
        let read = EntityData::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(read, vec![(0, 0, Value::Byte(7))]);
        // A block particle decodes fully and round-trips.
        let entries = vec![(
            10u8,
            serializer::PARTICLE,
            Value::Particle(Particle {
                id: 1,
                data: Some(ParticleData::State(2)),
            }),
        )];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
    }

    /// Every particle data shape (26.3 codecs) round-trips, and a PARTICLES
    /// list is a VarInt count then each particle, so the entry after it
    /// still decodes.
    #[test]
    fn particles_round_trip() {
        let all = [
            (1, Some(ParticleData::State(2))),
            (7, Some(ParticleData::Geyser(5))),
            (8, Some(ParticleData::GeyserBase(3, 0.5))),
            (15, Some(ParticleData::Scalar(1.0))),
            (21, Some(ParticleData::ColorScale(0xff0000, 1.5))),
            (22, Some(ParticleData::ColorScale2(1, 2, 0.75))),
            (23, Some(ParticleData::ColorPower(-1, 2.0))),
            (28, Some(ParticleData::Color(0x7f00ff00))),
            (
                57,
                Some(ParticleData::Item(Stack {
                    item: 42,
                    count: 3,
                    added: Vec::new(),
                    removed: vec![7],
                    wire: Default::default(),
                })),
            ),
            (
                58,
                Some(ParticleData::Vibration {
                    source: PositionSource::Block([-12, 64, 300]),
                    ticks: 20,
                }),
            ),
            (
                58,
                Some(ParticleData::Vibration {
                    source: PositionSource::Entity {
                        id: 99,
                        y_offset: 1.25,
                    },
                    ticks: 8,
                }),
            ),
            (59, Some(ParticleData::Trail([1.5, -2.0, 3.25], 9, 40))),
            (115, Some(ParticleData::Delay(6))),
            (0, None),
        ]
        .map(|(id, data)| Particle { id, data });
        let mut entries: Vec<_> = all
            .iter()
            .map(|p| (10u8, serializer::PARTICLE, Value::Particle(p.clone())))
            .collect();
        entries.push((11, serializer::PARTICLES, Value::Particles(all.to_vec())));
        entries.push((12, serializer::BYTE, Value::Byte(4)));
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
        // Wire shape: an empty list is one 0 byte, then the next entry.
        let bytes = [11, serializer::PARTICLES as u8, 0, 12, 0, 4, 0xFF];
        assert_eq!(
            EntityData::read(&mut Reader::new(&bytes)).unwrap(),
            vec![
                (11, serializer::PARTICLES, Value::Particles(Vec::new())),
                (12, serializer::BYTE, Value::Byte(4)),
            ]
        );
    }

    /// RESOLVABLE_PROFILE (a mannequin's DATA_PROFILE): both ResolvableProfile
    /// contents round-trip, and an entry after it still decodes.
    #[test]
    fn resolvable_profile_round_trips() {
        use crate::packets::login::Property;
        let full = Profile {
            name: Some("Gommunity".into()),
            uuid: Some(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF),
            properties: vec![Property {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            }],
            body: Some("minecraft:entity/player/wide/steve".into()),
            cape: None,
            elytra: None,
            slim: Some(false),
        };
        let partial = Profile {
            name: Some("Gommunity".into()),
            ..Profile::default()
        };
        let mut w = Writer::new();
        EntityData::write(
            &vec![
                (
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(full.clone()),
                ),
                (
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(partial.clone()),
                ),
                (18, serializer::BOOLEAN, Value::Bool(true)),
            ],
            &mut w,
        );
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            vec![
                (17, serializer::RESOLVABLE_PROFILE, Value::Profile(full)),
                (17, serializer::RESOLVABLE_PROFILE, Value::Profile(partial)),
                (18, serializer::BOOLEAN, Value::Bool(true)),
            ]
        );
    }
}
