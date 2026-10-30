//! Play state packets: world, entities, player state, inventory and chat.
use crate::{
    Error, Result,
    buf::{
        Bool, Byte, BytesN, ChunkPos, Codec, Double, Float, Ident, Int, List, Long, LpVec3, Nbt,
        Opt, OptVarInt, Pos, Reader, SectionPos, Short, Str, StrN, UByte, Uuid, VarInt, VarLong,
        Vec3d, Writer,
    },
    chunk::{ChunkData, LightData},
    entity_data::{EntityData, Particle, Value},
    item::{Component, DelimitedSlot, Hashed, HashedSlot, Slot, Stack},
    packet, packet_enum,
    packets::{
        Empty,
        configuration::{
            ClientInformation, CookieRequest, CookieResponse, CustomClickAction, CustomPayload,
            Disconnect, KeepAlive, Ping, ResourcePackPop, ResourcePackPush, ResourcePackResponse,
            ServerLinks, StoreCookie, Transfer, UpdateTags,
        },
        login::Property,
    },
};
use horizon_world::nbt::Tag;

// ---- shared structures -------------------------------------------------

packet!(
    /// GlobalPos: a dimension and a block position.
    GlobalPos { dimension: String => Ident, pos: [i32; 3] => Pos }
);
packet!(
    /// CommonPlayerSpawnInfo.
    SpawnInfo {
        dimension_type: i32 => VarInt,
        dimension: String => Ident,
        seed: i64 => Long,
        game_type: i32 => VarInt,
        previous_game_type: Option<i32> => OptVarInt,
        debug: bool => Bool,
        flat: bool => Bool,
        last_death: Option<GlobalPos> => Opt<GlobalPos>,
        portal_cooldown: i32 => VarInt,
        sea_level: i32 => VarInt,
    }
);

/// Relative flags (Relative.pack): X, Y, Z, Y_ROT, X_ROT, DELTA_X/Y/Z, ROTATE_DELTA.
pub mod relative {
    pub const X: i32 = 1;
    pub const Y: i32 = 2;
    pub const Z: i32 = 4;
    pub const Y_ROT: i32 = 8;
    pub const X_ROT: i32 = 16;
    pub const DELTA_X: i32 = 32;
    pub const DELTA_Y: i32 = 64;
    pub const DELTA_Z: i32 = 128;
    pub const ROTATE_DELTA: i32 = 256;
}

// ---- clientbound -------------------------------------------------------

packet!(AddEntity {
    id: i32 => VarInt,
    uuid: u128 => Uuid,
    kind: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    velocity: [f64; 3] => LpVec3,
    pitch: i8 => Byte,
    yaw: i8 => Byte,
    head_yaw: i8 => Byte,
    data: i32 => VarInt,
});
packet!(Animate { id: i32 => VarInt, action: u8 => UByte });
packet!(BlockChangedAck { sequence: i32 => VarInt });
packet!(BlockDestruction { id: i32 => VarInt, pos: [i32; 3] => Pos, progress: u8 => UByte });
packet!(BlockEntityData { pos: [i32; 3] => Pos, kind: i32 => VarInt, tag: Tag => Nbt });
packet!(BlockUpdate { pos: [i32; 3] => Pos, state: i32 => VarInt });
packet!(ChangeDifficulty { difficulty: i32 => VarInt, locked: bool => Bool });
// ResourceKey<GameRule<?>> is an identifier, followed by its string value.
packet!(GameRuleEntry { name: String => Ident, value: String => Str });
packet!(GameRuleValues { entries: Vec<GameRuleEntry> => List<GameRuleEntry> });
packet!(ChunkBatchFinished { size: i32 => VarInt });
packet!(ContainerClose { container: i32 => VarInt });
packet!(ContainerSetContent {
    container: i32 => VarInt,
    state: i32 => VarInt,
    items: Vec<Option<Stack>> => List<Slot>,
    carried: Option<Stack> => Slot,
});
/// ItemCost.STREAM_CODEC: item, count, then the exact component predicate
/// (a list of typed component values, with no patch removal count).
#[derive(Clone, Debug, PartialEq)]
pub struct NetCost {
    pub item: i32,
    pub count: i32,
    pub components: Vec<(i32, Component)>,
}
impl From<Stack> for NetCost {
    fn from(stack: Stack) -> Self {
        Self {
            item: stack.item,
            count: stack.count,
            components: stack.added,
        }
    }
}
impl From<NetCost> for Stack {
    fn from(cost: NetCost) -> Self {
        Self {
            item: cost.item,
            count: cost.count,
            added: cost.components,
            removed: Vec::new(),
            wire: Default::default(),
        }
    }
}
/// MerchantOffer.STREAM_CODEC.
#[derive(Clone, Debug, PartialEq)]
pub struct NetOffer {
    pub cost_a: NetCost,
    pub result: Stack,
    pub cost_b: Option<NetCost>,
    pub out_of_stock: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}
pub struct Offers;
impl Codec<Vec<NetOffer>> for Offers {
    fn read(r: &mut Reader) -> Result<Vec<NetOffer>> {
        fn cost(r: &mut Reader) -> Result<NetCost> {
            let item = r.varint()?;
            let count = r.varint()?;
            let n = r.count(r.remaining())?;
            let mut components = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                let kind = r.varint()?;
                components.push((kind, Component::read(kind, r)?));
            }
            Ok(NetCost {
                item,
                count,
                components,
            })
        }
        let n = r.count(r.remaining())?;
        let mut out = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let cost_a = cost(r)?;
            let result = Slot::read(r)?.ok_or_else(|| Error::new("empty merchant result"))?;
            let cost_b = if r.bool()? { Some(cost(r)?) } else { None };
            out.push(NetOffer {
                cost_a,
                result,
                cost_b,
                out_of_stock: r.bool()?,
                uses: r.i32()?,
                max_uses: r.i32()?,
                xp: r.i32()?,
                special_price: r.i32()?,
                price_multiplier: r.f32()?,
                demand: r.i32()?,
            });
        }
        Ok(out)
    }
    fn write(v: &Vec<NetOffer>, w: &mut Writer) {
        fn cost(c: &NetCost, w: &mut Writer) {
            w.varint(c.item);
            w.varint(c.count);
            w.varint(c.components.len() as i32);
            for (kind, value) in &c.components {
                w.varint(*kind);
                value.write(*kind, w);
            }
        }
        w.varint(v.len() as i32);
        for o in v {
            cost(&o.cost_a, w);
            Slot::write(&Some(o.result.clone()), w);
            w.bool(o.cost_b.is_some());
            if let Some(b) = &o.cost_b {
                cost(b, w);
            }
            w.bool(o.out_of_stock);
            w.i32(o.uses);
            w.i32(o.max_uses);
            w.i32(o.xp);
            w.i32(o.special_price);
            w.f32(o.price_multiplier);
            w.i32(o.demand);
        }
    }
}
packet!(
    /// ClientboundMerchantOffersPacket.
    MerchantOffers {
        container: i32 => VarInt,
        offers: Vec<NetOffer> => Offers,
        level: i32 => VarInt,
        xp: i32 => VarInt,
        show_progress: bool => Bool,
        can_restock: bool => Bool,
    }
);
packet!(ContainerSetData { container: i32 => VarInt, id: i16 => Short, value: i16 => Short });
// ClientboundCooldownPacket: the cooldown group (an identifier) and the ticks
// its entry lasts (0 ends it).
packet!(Cooldown { group: String => Ident, duration: i32 => VarInt });
packet!(ContainerSetSlot { container: i32 => VarInt, state: i32 => VarInt, slot: i16 => Short, item: Option<Stack> => Slot });
/// A holder reference: registry id + 1, or 0 for an inline value (unsupported).
pub struct HolderId;
impl Codec<i32> for HolderId {
    fn read(r: &mut Reader) -> Result<i32> {
        match r.varint()? {
            0 => Err(Error::new("inline registry values are not supported")),
            id => Ok(id - 1),
        }
    }
    fn write(v: &i32, w: &mut Writer) {
        w.varint(v + 1)
    }
}
/// `Holder<Dialog>` as Dialog.STREAM_CODEC sends it: an entry of the
/// `minecraft:dialog` registry the server sent in configuration, or the
/// dialog itself.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogHolder {
    Registry(i32),
    Direct(Tag),
}
impl Codec<DialogHolder> for DialogHolder {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(match r.varint()? {
            0 => Self::Direct(r.nbt()?),
            id => Self::Registry(id - 1),
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match v {
            Self::Registry(id) => w.varint(id + 1),
            Self::Direct(tag) => {
                w.varint(0);
                w.nbt(tag);
            }
        }
    }
}
packet!(
    /// ClientboundShowDialogPacket.
    ShowDialog { dialog: DialogHolder => DialogHolder }
);
packet!(
    /// ChatType.Bound.
    ChatTypeBound { chat_type: i32 => HolderId, name: Tag => Nbt, target: Option<Tag> => Opt<Nbt> }
);
packet!(DisguisedChat { message: Tag => Nbt, chat_type: ChatTypeBound => ChatTypeBound });
packet!(EntityEvent { id: i32 => Int, event: i8 => Byte });
/// PositionPath: Linear(end position) or Stepped(list of (position, tick offset)).
#[derive(Clone, Debug, PartialEq)]
pub enum PositionPath {
    Linear([f64; 3]),
    Stepped(Vec<([f64; 3], i32)>),
}
impl PositionPath {
    pub fn end(&self) -> [f64; 3] {
        match self {
            Self::Linear(p) => *p,
            Self::Stepped(steps) => steps.last().map_or([0.0; 3], |s| s.0),
        }
    }
}
impl Codec<PositionPath> for PositionPath {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(match r.varint()? {
            1 => {
                let n = r.count(r.remaining() / 25)?;
                Self::Stepped(
                    (0..n)
                        .map(|_| Ok((Vec3d::read(r)?, r.varint()?)))
                        .collect::<Result<_>>()?,
                )
            }
            _ => Self::Linear(Vec3d::read(r)?),
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match v {
            Self::Linear(p) => {
                w.varint(0);
                Vec3d::write(p, w);
            }
            Self::Stepped(steps) => {
                w.varint(1);
                w.varint(steps.len() as i32);
                for (p, t) in steps {
                    Vec3d::write(p, w);
                    w.varint(*t);
                }
            }
        }
    }
}
packet!(EntityPositionSync { id: i32 => VarInt, path: PositionPath => PositionPath, yaw: f32 => Float, pitch: f32 => Float, on_ground: bool => Bool });
packet!(ForgetLevelChunk { pos: [i32; 2] => ChunkPos });
packet!(GameEvent { event: u8 => UByte, param: f32 => Float });
packet!(HurtAnimation { id: i32 => VarInt, yaw: f32 => Float });
packet!(
    /// ClientboundDamageEventPacket: the hurt entity, the damage type's
    /// registry id, the causing and the direct entity (none: -1 on the wire's
    /// id + 1) and the source's position.
    DamageEvent {
        id: i32 => VarInt,
        source_type: i32 => VarInt,
        cause: Option<i32> => OptVarInt,
        direct: Option<i32> => OptVarInt,
        position: Option<[f64; 3]> => Opt<Vec3d>,
    }
);
packet!(LevelChunkWithLight { x: i32 => Int, z: i32 => Int, chunk: ChunkData => ChunkData, light: LightData => LightData });
packet!(LevelEvent { kind: i32 => Int, pos: [i32; 3] => Pos, data: i32 => Int, global: bool => Bool });
packet!(LightUpdate { x: i32 => VarInt, z: i32 => VarInt, light: LightData => LightData });
packet!(Login {
    player_id: i32 => Int,
    hardcore: bool => Bool,
    levels: Vec<String> => List<Ident>,
    max_players: i32 => VarInt,
    chunk_radius: i32 => VarInt,
    simulation_distance: i32 => VarInt,
    reduced_debug_info: bool => Bool,
    show_death_screen: bool => Bool,
    limited_crafting: bool => Bool,
    spawn: SpawnInfo => SpawnInfo,
    online_mode: bool => Bool,
    enforces_secure_chat: bool => Bool,
});
packet!(
/// MapDecoration: a registry holder, its map position and rotation, and an
/// optional name.
MapDecoration {
    kind: i32 => VarInt,
    x: i8 => Byte,
    y: i8 => Byte,
    rot: i8 => Byte,
    name: Option<Tag> => Opt<Nbt>,
}
);

/// MapItemSavedData.MapPatch: an empty patch is a zero width.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPatch {
    pub start_x: i32,
    pub start_y: i32,
    pub width: i32,
    pub height: i32,
    pub colors: Vec<u8>,
}
pub struct MapPatchOpt;
impl Codec<Option<MapPatch>> for MapPatchOpt {
    fn read(r: &mut Reader) -> Result<Option<MapPatch>> {
        let width = i32::from(r.u8()?);
        if width == 0 {
            return Ok(None);
        }
        let height = i32::from(r.u8()?);
        let start_x = i32::from(r.u8()?);
        let start_y = i32::from(r.u8()?);
        let n = r.count(r.remaining())?;
        let colors = r.bytes(n)?.to_vec();
        Ok(Some(MapPatch {
            start_x,
            start_y,
            width,
            height,
            colors,
        }))
    }
    fn write(patch: &Option<MapPatch>, w: &mut Writer) {
        let Some(patch) = patch else {
            w.u8(0);
            return;
        };
        w.u8(patch.width as u8);
        w.u8(patch.height as u8);
        w.u8(patch.start_x as u8);
        w.u8(patch.start_y as u8);
        w.varint(patch.colors.len() as i32);
        w.bytes(&patch.colors);
    }
}

packet!(MapItemData {
    map_id: i32 => VarInt,
    scale: i8 => Byte,
    locked: bool => Bool,
    decorations: Option<Vec<MapDecoration>> => Opt<List<MapDecoration>>,
    patch: Option<MapPatch> => MapPatchOpt,
});
/// VecDelta: per-axis deltas in 1/4096 blocks, linear or in timed steps.
#[derive(Clone, Debug, PartialEq)]
pub enum VecDelta {
    Linear([i16; 3]),
    Stepped(Vec<([i16; 3], i32)>),
}
impl VecDelta {
    /// VecDeltaCodec.decode against a base position: zero deltas keep the
    /// base coordinate exactly, others add to its 1/4096 quantization.
    pub fn apply(base: [f64; 3], delta: [i16; 3]) -> [f64; 3] {
        let mut out = base;
        for i in 0..3 {
            if delta[i] != 0 {
                out[i] = ((base[i] * 4096.0).round() as i64 + i64::from(delta[i])) as f64 / 4096.0;
            }
        }
        out
    }
    /// The final position after all steps.
    pub fn end(&self, base: [f64; 3]) -> [f64; 3] {
        match self {
            Self::Linear(d) => Self::apply(base, *d),
            Self::Stepped(steps) => steps.iter().fold(base, |p, (d, _)| Self::apply(p, *d)),
        }
    }
    fn read(r: &mut Reader, steps: usize) -> Result<Self> {
        if steps == 0 {
            return Ok(Self::Linear([r.i16()?, r.i16()?, r.i16()?]));
        }
        if steps > r.remaining() / 7 {
            return Err(Error::new("VecDelta with too many steps"));
        }
        Ok(Self::Stepped(
            (0..steps)
                .map(|_| {
                    let ticks = r.varint()?;
                    Ok(([r.i16()?, r.i16()?, r.i16()?], ticks))
                })
                .collect::<Result<_>>()?,
        ))
    }
    fn steps(&self) -> usize {
        match self {
            Self::Linear(_) => 0,
            Self::Stepped(s) => s.len(),
        }
    }
    fn write(&self, w: &mut Writer) {
        match self {
            Self::Linear(d) => d.iter().for_each(|v| w.i16(*v)),
            Self::Stepped(steps) => {
                for (d, ticks) in steps {
                    w.varint(*ticks);
                    d.iter().for_each(|v| w.i16(*v));
                }
            }
        }
    }
}
/// ClientboundMoveEntityPacket.Pos / PosRot / Rot.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntity {
    pub id: i32,
    pub delta: Option<VecDelta>,
    pub rotation: Option<(i8, i8)>,
    pub on_ground: bool,
}
macro_rules! move_entity {
    ($name:ident, pos: $pos:expr, rot: $rot:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub MoveEntity);
        impl Codec<$name> for $name {
            fn read(r: &mut Reader) -> Result<$name> {
                Self::read_inner(r).map($name)
            }
            fn write(v: &$name, w: &mut Writer) {
                Self::write_inner(&v.0, w)
            }
        }
        impl $name {
            fn read_inner(r: &mut Reader) -> Result<MoveEntity> {
                let id = r.varint()?;
                if !$pos {
                    let on_ground = r.bool()?;
                    let rotation = Some((r.i8()?, r.i8()?));
                    return Ok(MoveEntity {
                        id,
                        delta: None,
                        rotation,
                        on_ground,
                    });
                }
                let properties = r.varint()?;
                let delta = Some(VecDelta::read(r, (properties as u32 >> 1) as usize)?);
                let rotation = if $rot { Some((r.i8()?, r.i8()?)) } else { None };
                Ok(MoveEntity {
                    id,
                    delta,
                    rotation,
                    on_ground: properties & 1 != 0,
                })
            }
            fn write_inner(v: &MoveEntity, w: &mut Writer) {
                w.varint(v.id);
                if !$pos {
                    w.bool(v.on_ground);
                } else {
                    let delta = v.delta.clone().unwrap_or(VecDelta::Linear([0; 3]));
                    w.varint(i32::from(v.on_ground) | (delta.steps() as i32) << 1);
                    delta.write(w);
                }
                if let Some((y, x)) = v.rotation.filter(|_| $rot) {
                    w.i8(y);
                    w.i8(x);
                }
            }
        }
    };
}
move_entity!(MoveEntityPos, pos: true, rot: false);
move_entity!(MoveEntityPosRot, pos: true, rot: true);
move_entity!(MoveEntityRot, pos: false, rot: true);
packet!(OpenScreen { container: i32 => VarInt, menu: i32 => VarInt, title: Tag => Nbt });
// ClientboundMountScreenOpenPacket: a horse's or llama's inventory, with its
// chest's columns (`getInventoryColumns`).
packet!(MountScreenOpen { container: i32 => VarInt, columns: i32 => VarInt, entity: i32 => Int });
packet!(
    /// ClientboundOpenSignEditorPacket: the sign whose side the player edits.
    OpenSignEditor { pos: [i32; 3] => Pos, front: bool => SignSide }
);
packet!(
    /// ClientboundOpenBookPacket: the hand (InteractionHand, 0 main, 1 off)
    /// whose written book the player reads.
    OpenBook { hand: i32 => VarInt }
);
packet!(PlayerAbilities { flags: i8 => Byte, flying_speed: f32 => Float, walking_speed: f32 => Float });
/// MessageSignature.Packed: an index into the last-seen cache or a full signature.
pub struct PackedSignature;
impl Codec<(i32, Option<Vec<u8>>)> for PackedSignature {
    fn read(r: &mut Reader) -> Result<(i32, Option<Vec<u8>>)> {
        let id = r.varint()? - 1;
        Ok(if id == -1 {
            (-1, Some(r.bytes(256)?.to_vec()))
        } else {
            (id, None)
        })
    }
    fn write(v: &(i32, Option<Vec<u8>>), w: &mut Writer) {
        w.varint(v.0 + 1);
        if let Some(sig) = &v.1 {
            w.bytes(sig);
        }
    }
}
/// MessageSignature: 256 raw bytes.
pub struct Signature;
impl Codec<Vec<u8>> for Signature {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        Ok(r.bytes(256)?.to_vec())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.bytes(v)
    }
}
/// FilterMask: 0 pass-through, 1 fully filtered, 2 partially (with a BitSet).
pub struct FilterMask;
impl Codec<(i32, Vec<i64>)> for FilterMask {
    fn read(r: &mut Reader) -> Result<(i32, Vec<i64>)> {
        let kind = r.varint()?;
        Ok((
            kind,
            if kind == 2 {
                r.long_array()?
            } else {
                Vec::new()
            },
        ))
    }
    fn write(v: &(i32, Vec<i64>), w: &mut Writer) {
        w.varint(v.0);
        if v.0 == 2 {
            w.long_array(&v.1);
        }
    }
}
packet!(PlayerChat {
    global_index: i32 => VarInt,
    sender: u128 => Uuid,
    index: i32 => VarInt,
    signature: Option<Vec<u8>> => Opt<Signature>,
    content: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    last_seen: Vec<(i32, Option<Vec<u8>>)> => List<PackedSignature>,
    unsigned_content: Option<Tag> => Opt<Nbt>,
    filter: (i32, Vec<i64>) => FilterMask,
    chat_type: ChatTypeBound => ChatTypeBound,
});
packet!(PlayerCombatKill { id: i32 => VarInt, message: Tag => Nbt });
packet!(PlayerInfoRemove { uuids: Vec<u128> => List<Uuid> });
/// One ClientboundPlayerInfoUpdatePacket entry; fields are set for the
/// actions in the packet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInfo {
    pub uuid: u128,
    pub name: Option<String>,
    pub properties: Vec<Property>,
    /// RemoteChatSession.Data: session id, key expiry, key, key signature.
    pub chat_session: Option<(u128, i64, Vec<u8>, Vec<u8>)>,
    pub game_mode: Option<i32>,
    pub listed: Option<bool>,
    pub latency: Option<i32>,
    pub display_name: Option<Option<Tag>>,
    pub list_order: Option<i32>,
    pub show_hat: Option<bool>,
}
/// Actions bits: ADD_PLAYER, INITIALIZE_CHAT, UPDATE_GAME_MODE, UPDATE_LISTED,
/// UPDATE_LATENCY, UPDATE_DISPLAY_NAME, UPDATE_LIST_ORDER, UPDATE_HAT.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfoUpdate {
    pub actions: u8,
    pub entries: Vec<PlayerInfo>,
}
impl Codec<PlayerInfoUpdate> for PlayerInfoUpdate {
    fn read(r: &mut Reader) -> Result<Self> {
        let actions = r.u8()?;
        let n = r.count(r.remaining())?;
        let mut entries = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let mut e = PlayerInfo {
                uuid: r.uuid()?,
                ..PlayerInfo::default()
            };
            if actions & 1 != 0 {
                e.name = Some(r.string(16)?);
                let count = r.count(16)?;
                e.properties = (0..count)
                    .map(|_| Property::read(r))
                    .collect::<Result<_>>()?;
            }
            if actions & 2 != 0 && r.bool()? {
                e.chat_session =
                    Some((r.uuid()?, r.i64()?, r.byte_array(512)?, r.byte_array(4096)?));
            }
            if actions & 4 != 0 {
                e.game_mode = Some(r.varint()?);
            }
            if actions & 8 != 0 {
                e.listed = Some(r.bool()?);
            }
            if actions & 16 != 0 {
                e.latency = Some(r.varint()?);
            }
            if actions & 32 != 0 {
                e.display_name = Some(if r.bool()? { Some(r.nbt()?) } else { None });
            }
            if actions & 64 != 0 {
                e.list_order = Some(r.varint()?);
            }
            if actions & 128 != 0 {
                e.show_hat = Some(r.bool()?);
            }
            entries.push(e);
        }
        Ok(Self { actions, entries })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.u8(v.actions);
        w.varint(v.entries.len() as i32);
        for e in &v.entries {
            w.uuid(e.uuid);
            if v.actions & 1 != 0 {
                w.string(e.name.as_deref().unwrap_or(""));
                w.varint(e.properties.len() as i32);
                e.properties.iter().for_each(|p| Property::write(p, w));
            }
            if v.actions & 2 != 0 {
                w.bool(e.chat_session.is_some());
                if let Some((id, expires, key, sig)) = &e.chat_session {
                    w.uuid(*id);
                    w.i64(*expires);
                    w.byte_array(key);
                    w.byte_array(sig);
                }
            }
            if v.actions & 4 != 0 {
                w.varint(e.game_mode.unwrap_or(0));
            }
            if v.actions & 8 != 0 {
                w.bool(e.listed.unwrap_or(false));
            }
            if v.actions & 16 != 0 {
                w.varint(e.latency.unwrap_or(0));
            }
            if v.actions & 32 != 0 {
                let name = e.display_name.clone().flatten();
                w.bool(name.is_some());
                if let Some(t) = &name {
                    w.nbt(t);
                }
            }
            if v.actions & 64 != 0 {
                w.varint(e.list_order.unwrap_or(0));
            }
            if v.actions & 128 != 0 {
                w.bool(e.show_hat.unwrap_or(false));
            }
        }
    }
}
packet!(PlayerPosition {
    teleport: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    delta: [f64; 3] => Vec3d,
    yaw: f32 => Float,
    pitch: f32 => Float,
    relatives: i32 => Int,
});
packet!(
    /// ClientboundProjectilePowerPacket: an AbstractHurtingProjectile's accelerationPower.
    ProjectilePower { id: i32 => VarInt, power: f64 => Double }
);
packet!(
    /// ClientboundPlayerLookAtPacket's entity target: its id and the anchor
    /// (`EntityAnchorArgument.Anchor`: 0 feet, 1 eyes) to look at on it.
    LookAtEntity { id: i32 => VarInt, anchor: i32 => VarInt }
);
packet!(
    /// ClientboundPlayerLookAtPacket: turn this player's `from` anchor towards
    /// a point, or towards an entity when the client knows it.
    PlayerLookAt {
        from: i32 => VarInt,
        target: [f64; 3] => Vec3d,
        entity: Option<LookAtEntity> => Opt<LookAtEntity>,
    }
);
packet!(PlayerRotation { yaw: f32 => Float, relative_yaw: bool => Bool, pitch: f32 => Float, relative_pitch: bool => Bool });
packet!(RemoveEntities { ids: Vec<i32> => List<VarInt> });
packet!(Respawn { spawn: SpawnInfo => SpawnInfo, keep: i8 => Byte });
packet!(RotateHead { id: i32 => VarInt, head_yaw: i8 => Byte });
packet!(
    /// Each change is `state << 12 | x << 8 | z << 4 | y` within the section.
    SectionBlocksUpdate { section: [i32; 3] => SectionPos, changes: Vec<i64> => List<VarLong> }
);
packet!(Text { text: Tag => Nbt });
packet!(SetChunkCacheCenter { x: i32 => VarInt, z: i32 => VarInt });
packet!(
    /// ClientboundTickingStatePacket: the tick rate and whether the level is frozen.
    TickingState { rate: f32 => Float, frozen: bool => Bool }
);
packet!(
    /// ClientboundTickingStepPacket: the ticks a frozen level runs (`/tick step`).
    TickingStep { steps: i32 => VarInt }
);
packet!(Distance { distance: i32 => VarInt });
packet!(SetCursorItem { item: Option<Stack> => Slot });
packet!(SetDefaultSpawnPosition { pos: GlobalPos => GlobalPos, yaw: f32 => Float, pitch: f32 => Float });
packet!(SetEntityData { id: i32 => VarInt, values: Vec<(u8, i32, Value)> => EntityData });
packet!(SetEntityMotion { id: i32 => VarInt, velocity: [f64; 3] => LpVec3 });
packet!(
    /// ClientboundSetEntityLinkPacket: the entity holding a leashed entity's
    /// lead, 0 when none (Leashable.setDelayedLeashHolderId).
    SetEntityLink { source: i32 => Int, dest: i32 => Int }
);
packet!(
    /// ClientboundSetPassengersPacket: a vehicle and every entity riding it.
    SetPassengers { vehicle: i32 => VarInt, passengers: Vec<i32> => List<VarInt> }
);
packet!(
    /// ClientboundMoveVehiclePacket: the server puts a vehicle its rider
    /// controls back where it was (PositionAndRotation: x, y, z, yRot, xRot).
    MoveVehicle { position: [f64; 3] => Vec3d, yaw: f32 => Float, pitch: f32 => Float }
);
/// ClientboundSetEquipmentPacket: (EquipmentSlot ordinal, stack) pairs.
pub struct Equipment;
impl Codec<Vec<(u8, Option<Stack>)>> for Equipment {
    fn read(r: &mut Reader) -> Result<Vec<(u8, Option<Stack>)>> {
        let mut out = Vec::new();
        loop {
            let slot = r.u8()?;
            out.push((slot & 127, Slot::read(r)?));
            if slot & 128 == 0 {
                return Ok(out);
            }
        }
    }
    fn write(v: &Vec<(u8, Option<Stack>)>, w: &mut Writer) {
        for (i, (slot, stack)) in v.iter().enumerate() {
            w.u8(if i + 1 < v.len() { slot | 128 } else { *slot });
            Slot::write(stack, w);
        }
    }
}
packet!(SetEquipment { id: i32 => VarInt, slots: Vec<(u8, Option<Stack>)> => Equipment });
packet!(SetExperience { progress: f32 => Float, level: i32 => VarInt, total: i32 => VarInt });
packet!(SetHealth { health: f32 => Float, food: i32 => VarInt, saturation: f32 => Float });
packet!(SetHeldSlot { slot: i32 => VarInt });
packet!(SetPlayerInventory { slot: i32 => VarInt, item: Option<Stack> => Slot });
packet!(
    /// ClockNetworkState for one world clock (registry id).
    ClockState { clock: i32 => VarInt, total_ticks: i64 => VarLong, partial_tick: f32 => Float, rate: f32 => Float }
);
packet!(SetTime { game_time: i64 => Long, clocks: Vec<ClockState> => List<ClockState> });
packet!(TitlesAnimation { fade_in: i32 => Int, stay: i32 => Int, fade_out: i32 => Int });
/// SoundEvent holder: a registry id, or an inline location with optional range.
pub struct SoundHolder;
impl Codec<(i32, Option<(String, Option<f32>)>)> for SoundHolder {
    fn read(r: &mut Reader) -> Result<(i32, Option<(String, Option<f32>)>)> {
        let id = r.varint()?;
        if id != 0 {
            return Ok((id - 1, None));
        }
        let location = r.string(32767)?;
        let range = if r.bool()? { Some(r.f32()?) } else { None };
        Ok((-1, Some((location, range))))
    }
    fn write(v: &(i32, Option<(String, Option<f32>)>), w: &mut Writer) {
        match &v.1 {
            None => w.varint(v.0 + 1),
            Some((location, range)) => {
                w.varint(0);
                w.string(location);
                <Opt<Float> as Codec<Option<f32>>>::write(range, w);
            }
        }
    }
}
packet!(Sound {
    sound: (i32, Option<(String, Option<f32>)>) => SoundHolder,
    source: i32 => VarInt,
    x: i32 => Int,
    y: i32 => Int,
    z: i32 => Int,
    volume: f32 => Float,
    pitch: f32 => Float,
    seed: i64 => Long,
});
packet!(SystemChat { content: Tag => Nbt, overlay: bool => Bool });
packet!(TakeItemEntity { item: i32 => VarInt, player: i32 => VarInt, amount: i32 => VarInt });
packet!(TeleportEntity {
    id: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    delta: [f64; 3] => Vec3d,
    yaw: f32 => Float,
    pitch: f32 => Float,
    relatives: i32 => Int,
    on_ground: bool => Bool,
});

packet!(BlockEvent { pos: [i32; 3] => Pos, b0: u8 => UByte, b1: u8 => UByte, block: i32 => VarInt });
packet!(
    /// ClientboundInitializeBorderPacket.
    InitializeBorder {
        center_x: f64 => Double,
        center_z: f64 => Double,
        old_size: f64 => Double,
        new_size: f64 => Double,
        lerp_time: i64 => VarLong,
        absolute_max_size: i32 => VarInt,
        warning_blocks: i32 => VarInt,
        warning_time: i32 => VarInt,
    }
);
packet!(TabList { header: Tag => Nbt, footer: Tag => Nbt });
packet!(
    /// ClientboundSetBorderCenterPacket.
    SetBorderCenter { x: f64 => Double, z: f64 => Double }
);
packet!(
    /// ClientboundSetBorderLerpSizePacket.
    SetBorderLerpSize { old_size: f64 => Double, new_size: f64 => Double, lerp_time: i64 => VarLong }
);
packet!(
    /// ClientboundSetBorderSizePacket.
    SetBorderSize { size: f64 => Double }
);
packet!(
    /// ClientboundSetBorderWarningDelayPacket (ticks).
    SetBorderWarningDelay { delay: i32 => VarInt }
);
packet!(
    /// ClientboundSetBorderWarningDistancePacket.
    SetBorderWarningDistance { blocks: i32 => VarInt }
);
packet!(
    /// ClientboundSetDisplayObjectivePacket: `DisplaySlot` id (0 list,
    /// 1 sidebar, 2 below name, 3.. team sidebars) and the objective ("" clears).
    SetDisplayObjective { slot: i32 => VarInt, objective: String => Str }
);
packet!(
    /// ClientboundResetScorePacket: one objective, or all of them when absent.
    ResetScore { owner: String => Str, objective: Option<String> => Opt<Str> }
);
/// `NumberFormatTypes.STREAM_CODEC`, by `number_format_type` registry id.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberFormat {
    /// 0: `minecraft:blank`.
    Blank,
    /// 1: `minecraft:styled`, a style compound.
    Styled(Tag),
    /// 2: `minecraft:fixed`, a component.
    Fixed(Tag),
}
impl Codec<NumberFormat> for NumberFormat {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(match r.varint()? {
            0 => NumberFormat::Blank,
            1 => NumberFormat::Styled(r.nbt()?),
            2 => NumberFormat::Fixed(r.nbt()?),
            other => return Err(crate::Error::new(format!("unknown number format {other}"))),
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match v {
            NumberFormat::Blank => w.varint(0),
            NumberFormat::Styled(t) => {
                w.varint(1);
                w.nbt(t);
            }
            NumberFormat::Fixed(t) => {
                w.varint(2);
                w.nbt(t);
            }
        }
    }
}
packet!(
    /// ClientboundSetScorePacket.
    SetScore {
        owner: String => Str,
        objective: String => Str,
        score: i32 => VarInt,
        display: Option<Tag> => Opt<Nbt>,
        number_format: Option<NumberFormat> => Opt<NumberFormat>,
    }
);
/// ClientboundSetObjectivePacket: method 0 adds, 1 removes, 2 changes;
/// adds and changes carry the display name, render type (0 integer,
/// 1 hearts) and number format.
#[derive(Clone, Debug, PartialEq)]
pub struct SetObjective {
    pub name: String,
    pub method: i8,
    pub display: Option<(Tag, i32, Option<NumberFormat>)>,
}
impl Codec<SetObjective> for SetObjective {
    fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string(32767)?;
        let method = r.i8()?;
        let display = if method == 0 || method == 2 {
            Some((r.nbt()?, r.varint()?, <Opt<NumberFormat>>::read(r)?))
        } else {
            None
        };
        Ok(Self {
            name,
            method,
            display,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.string(&v.name);
        w.i8(v.method);
        if v.method == 0 || v.method == 2 {
            let (name, render, format) = v.display.clone().unwrap_or((Tag::End, 0, None));
            w.nbt(&name);
            w.varint(render);
            <Opt<NumberFormat>>::write(&format, w);
        }
    }
}
/// ClientboundLevelParticlesPacket: a particle, whether it ignores the
/// client's particle setting (`overrideLimiter`) and its distance limit
/// (`alwaysShow`), where, the spread and the speed on each axis, how many
/// (0: one, moving by speed times spread) and the randomization type
/// (0 default: gaussian spread and speed).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelParticles {
    pub particle: Particle,
    pub override_limiter: bool,
    pub always_show: bool,
    pub pos: [f64; 3],
    pub dist: [f32; 3],
    pub max_speed: [f32; 3],
    pub count: i32,
    pub randomization: i32,
}
impl Codec<LevelParticles> for LevelParticles {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            particle: Particle::read(r)?,
            override_limiter: r.bool()?,
            always_show: r.bool()?,
            pos: Vec3d::read(r)?,
            dist: [r.f32()?, r.f32()?, r.f32()?],
            max_speed: [r.f32()?, r.f32()?, r.f32()?],
            count: r.varint()?,
            randomization: r.varint()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        v.particle.write(w);
        w.bool(v.override_limiter);
        w.bool(v.always_show);
        Vec3d::write(&v.pos, w);
        for f in v.dist.iter().chain(&v.max_speed) {
            w.f32(*f);
        }
        w.varint(v.count);
        w.varint(v.randomization);
    }
}
/// `ExplosionParticleInfo` in a `WeightedList`: a particle the blast
/// scatters, its scaling and speed, then its weight.
#[derive(Clone, Debug, PartialEq)]
pub struct ExplosionParticle {
    pub particle: Particle,
    pub scaling: f32,
    pub speed: f32,
    pub weight: i32,
}
/// ClientboundExplodePacket: the centre, radius and blocks destroyed, the
/// push the receiving player takes, the particle at the centre, the sound,
/// the particles the blast scatters, and whether the sound plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Explode {
    pub center: [f64; 3],
    pub radius: f32,
    pub block_count: i32,
    pub knockback: Option<[f64; 3]>,
    pub particle: Particle,
    pub sound: (i32, Option<(String, Option<f32>)>),
    pub block_particles: Vec<ExplosionParticle>,
    pub play_sound: bool,
}
impl Codec<Explode> for Explode {
    fn read(r: &mut Reader) -> Result<Self> {
        let center = Vec3d::read(r)?;
        let radius = r.f32()?;
        let block_count = r.i32()?;
        let knockback = <Opt<Vec3d>>::read(r)?;
        let particle = Particle::read(r)?;
        let sound = SoundHolder::read(r)?;
        let n = r.varint()?;
        let mut block_particles = Vec::new();
        for _ in 0..n {
            block_particles.push(ExplosionParticle {
                particle: Particle::read(r)?,
                scaling: r.f32()?,
                speed: r.f32()?,
                weight: r.varint()?,
            });
        }
        Ok(Self {
            center,
            radius,
            block_count,
            knockback,
            particle,
            sound,
            block_particles,
            play_sound: r.bool()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        Vec3d::write(&v.center, w);
        w.f32(v.radius);
        w.i32(v.block_count);
        <Opt<Vec3d>>::write(&v.knockback, w);
        v.particle.write(w);
        SoundHolder::write(&v.sound, w);
        w.varint(v.block_particles.len() as i32);
        for b in &v.block_particles {
            b.particle.write(w);
            w.f32(b.scaling);
            w.f32(b.speed);
            w.varint(b.weight);
        }
        w.bool(v.play_sound);
    }
}
/// `ClientboundSetPlayerTeamPacket.Parameters`.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamParameters {
    pub display_name: Tag,
    pub prefix: Tag,
    pub suffix: Tag,
    /// `Team.Visibility` id.
    pub name_tag_visibility: i32,
    /// `Team.CollisionRule` id.
    pub collision_rule: i32,
    /// `TeamColor` id (0 black .. 15 white), absent for no colour.
    pub color: Option<i32>,
    /// 1: friendly fire, 2: see friendly invisibles.
    pub options: i8,
}
/// ClientboundSetPlayerTeamPacket: method 0 adds (parameters and players),
/// 1 removes, 2 changes (parameters), 3 joins and 4 leaves (players).
#[derive(Clone, Debug, PartialEq)]
pub struct SetPlayerTeam {
    pub name: String,
    pub method: i8,
    pub parameters: Option<TeamParameters>,
    pub players: Vec<String>,
}
impl Codec<SetPlayerTeam> for SetPlayerTeam {
    fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string(32767)?;
        let method = r.i8()?;
        let parameters = if method == 0 || method == 2 {
            Some(TeamParameters {
                display_name: r.nbt()?,
                prefix: r.nbt()?,
                suffix: r.nbt()?,
                name_tag_visibility: r.varint()?,
                collision_rule: r.varint()?,
                color: <Opt<VarInt>>::read(r)?,
                options: r.i8()?,
            })
        } else {
            None
        };
        let players = if matches!(method, 0 | 3 | 4) {
            <List<Str>>::read(r)?
        } else {
            Vec::new()
        };
        Ok(Self {
            name,
            method,
            parameters,
            players,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.string(&v.name);
        w.i8(v.method);
        if v.method == 0 || v.method == 2 {
            let p = v.parameters.clone().unwrap_or(TeamParameters {
                display_name: Tag::End,
                prefix: Tag::End,
                suffix: Tag::End,
                name_tag_visibility: 0,
                collision_rule: 0,
                color: None,
                options: 0,
            });
            w.nbt(&p.display_name);
            w.nbt(&p.prefix);
            w.nbt(&p.suffix);
            w.varint(p.name_tag_visibility);
            w.varint(p.collision_rule);
            <Opt<VarInt>>::write(&p.color, w);
            w.i8(p.options);
        }
        if matches!(v.method, 0 | 3 | 4) {
            <List<Str>>::write(&v.players, w);
        }
    }
}
packet!(ServerData { motd: Tag => Nbt, icon: Option<Vec<u8>> => Opt<crate::buf::Bytes> });
packet!(
    /// ClientboundUpdateMobEffectPacket; flags: 1 ambient, 2 visible, 4 icon, 8 blend.
    UpdateMobEffect { id: i32 => VarInt, effect: i32 => VarInt, amplifier: i32 => VarInt, duration: i32 => VarInt, flags: i8 => Byte }
);
packet!(RemoveMobEffect { id: i32 => VarInt, effect: i32 => VarInt });
packet!(
    /// AttributeModifier.STREAM_CODEC: id, amount and operation (0 add_value,
    /// 1 add_multiplied_base, 2 add_multiplied_total).
    AttributeModifier { id: String => Ident, amount: f64 => Double, operation: i32 => VarInt }
);
packet!(
    /// ClientboundUpdateAttributesPacket.AttributeSnapshot: the attribute's
    /// registry id, its base value and every modifier.
    AttributeSnapshot {
        attribute: i32 => VarInt,
        base: f64 => Double,
        modifiers: Vec<AttributeModifier> => List<AttributeModifier>,
    }
);
packet!(
    /// ClientboundUpdateAttributesPacket: an entity's synced attribute instances.
    UpdateAttributes { id: i32 => VarInt, values: Vec<AttributeSnapshot> => List<AttributeSnapshot> }
);
packet!(
    /// ClientboundSwingAnimationPacket: hand (0 main, 1 off) and the swing
    /// type (0 none, 1 whack, 2 stab) with its duration in ticks.
    SwingAnimation { id: i32 => VarInt, hand: i32 => VarInt, kind: i32 => VarInt, duration: i32 => VarInt }
);
packet!(SoundEntity {
    sound: (i32, Option<(String, Option<f32>)>) => SoundHolder,
    source: i32 => VarInt,
    id: i32 => VarInt,
    volume: f32 => Float,
    pitch: f32 => Float,
    seed: i64 => Long,
});

/// ClientboundStopSoundPacket: bit 0 selects a source, bit 1 an event id.
#[derive(Clone, Debug, PartialEq)]
pub struct StopSound {
    pub source: Option<i32>,
    pub sound: Option<String>,
}
impl Codec<StopSound> for StopSound {
    fn read(r: &mut Reader) -> crate::Result<Self> {
        let flags = r.u8()?;
        let source = if flags & 1 != 0 {
            let value = r.varint()?;
            if !(0..11).contains(&value) {
                return Err(crate::Error::new("invalid sound source"));
            }
            Some(value)
        } else {
            None
        };
        let sound = if flags & 2 != 0 {
            Some(r.string(32767)?)
        } else {
            None
        };
        Ok(Self { source, sound })
    }
    fn write(p: &Self, w: &mut Writer) {
        w.u8(u8::from(p.source.is_some()) | (u8::from(p.sound.is_some()) << 1));
        if let Some(source) = p.source {
            w.varint(source);
        }
        if let Some(sound) = &p.sound {
            w.string(sound);
        }
    }
}
/// One entry of ClientboundCommandsPacket: a literal or an argument node.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandNode {
    /// 0 root, 1 literal, 2 argument.
    pub kind: u8,
    pub executable: bool,
    pub restricted: bool,
    pub children: Vec<i32>,
    pub redirect: Option<i32>,
    pub name: String,
    /// Argument parser registry id and its raw property bytes.
    pub parser: Option<(i32, Vec<u8>)>,
    /// Custom suggestion provider (`minecraft:ask_server`).
    pub suggestions: Option<String>,
}
/// The `minecraft:command_argument_type` registry of 26.3 in id order
/// (ArgumentTypeInfos.bootstrap).
pub const ARGUMENT_TYPES: &[&str] = &[
    "brigadier:bool",
    "brigadier:float",
    "brigadier:double",
    "brigadier:integer",
    "brigadier:long",
    "brigadier:string",
    "minecraft:entity",
    "minecraft:game_profile",
    "minecraft:block_pos",
    "minecraft:column_pos",
    "minecraft:vec3",
    "minecraft:vec2",
    "minecraft:block_state",
    "minecraft:block_predicate",
    "minecraft:item_stack",
    "minecraft:item_predicate",
    "minecraft:team_color",
    "minecraft:hex_color",
    "minecraft:component",
    "minecraft:style",
    "minecraft:message",
    "minecraft:nbt_compound_tag",
    "minecraft:nbt_tag",
    "minecraft:nbt_path",
    "minecraft:objective",
    "minecraft:objective_criteria",
    "minecraft:operation",
    "minecraft:particle",
    "minecraft:angle",
    "minecraft:rotation",
    "minecraft:scoreboard_slot",
    "minecraft:score_holder",
    "minecraft:swizzle",
    "minecraft:team",
    "minecraft:item_slot",
    "minecraft:item_slots",
    "minecraft:resource_location",
    "minecraft:function",
    "minecraft:entity_anchor",
    "minecraft:int_range",
    "minecraft:float_range",
    "minecraft:dimension",
    "minecraft:gamemode",
    "minecraft:time",
    "minecraft:resource_or_tag",
    "minecraft:resource_or_tag_key",
    "minecraft:resource",
    "minecraft:resource_key",
    "minecraft:resource_selector",
    "minecraft:template_mirror",
    "minecraft:template_rotation",
    "minecraft:heightmap",
    "minecraft:loot_table",
    "minecraft:loot_predicate",
    "minecraft:loot_modifier",
    "minecraft:context_float_provider",
    "minecraft:context_int_provider",
    "minecraft:slot_source",
    "minecraft:dialog",
    "minecraft:feature",
    "minecraft:swing_animation",
    "minecraft:uuid",
];

/// The properties of an argument parser (`ArgumentTypeInfo.Template`) as
/// one of the 26.3 parsers decoded from the network.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgumentProperties {
    None,
    Integer {
        min: Option<i32>,
        max: Option<i32>,
    },
    Long {
        min: Option<i64>,
        max: Option<i64>,
    },
    Float {
        min: Option<f32>,
        max: Option<f32>,
    },
    Double {
        min: Option<f64>,
        max: Option<f64>,
    },
    /// StringArgumentType.StringType: 0 word, 1 phrase, 2 greedy.
    String(i32),
    Entity {
        single: bool,
        players_only: bool,
    },
    ScoreHolder {
        multiple: bool,
    },
    Time {
        min: i32,
    },
    /// The resource arguments' registry.
    Registry(String),
}

impl ArgumentProperties {
    /// ArgumentTypeInfo.deserializeFromNetwork for parser `id`.
    pub fn read(r: &mut Reader, id: i32) -> Result<Self> {
        let name = usize::try_from(id)
            .ok()
            .and_then(|i| ARGUMENT_TYPES.get(i))
            .ok_or_else(|| crate::Error::new(format!("unknown argument type {id}")))?;
        let flagged = |r: &mut Reader| -> Result<(bool, bool)> {
            let f = r.u8()?;
            Ok((f & 1 != 0, f & 2 != 0))
        };
        Ok(match *name {
            "brigadier:integer" => {
                let (min, max) = flagged(r)?;
                let min = if min { Some(r.i32()?) } else { None };
                let max = if max { Some(r.i32()?) } else { None };
                Self::Integer { min, max }
            }
            "brigadier:long" => {
                let (min, max) = flagged(r)?;
                let min = if min { Some(r.i64()?) } else { None };
                let max = if max { Some(r.i64()?) } else { None };
                Self::Long { min, max }
            }
            "brigadier:float" => {
                let (min, max) = flagged(r)?;
                let min = if min { Some(r.f32()?) } else { None };
                let max = if max { Some(r.f32()?) } else { None };
                Self::Float { min, max }
            }
            "brigadier:double" => {
                let (min, max) = flagged(r)?;
                let min = if min { Some(r.f64()?) } else { None };
                let max = if max { Some(r.f64()?) } else { None };
                Self::Double { min, max }
            }
            "brigadier:string" => Self::String(r.varint()?),
            "minecraft:entity" => {
                let f = r.u8()?;
                Self::Entity {
                    single: f & 1 != 0,
                    players_only: f & 2 != 0,
                }
            }
            "minecraft:score_holder" => Self::ScoreHolder {
                multiple: r.u8()? & 1 != 0,
            },
            "minecraft:time" => Self::Time { min: r.i32()? },
            "minecraft:resource"
            | "minecraft:resource_key"
            | "minecraft:resource_or_tag"
            | "minecraft:resource_or_tag_key"
            | "minecraft:resource_selector" => Self::Registry(r.string(32767)?),
            _ => Self::None,
        })
    }
    /// Decodes a node's raw `parser` bytes.
    pub fn decode(id: i32, bytes: &[u8]) -> Result<Self> {
        Self::read(&mut Reader::new(bytes), id)
    }
}

/// Reads past an argument node's parser properties.
fn read_argument_properties(r: &mut Reader, id: i32) -> Result<()> {
    ArgumentProperties::read(r, id).map(|_| ())
}

/// The command tree: nodes and the index of the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Commands {
    pub nodes: Vec<CommandNode>,
    pub root: i32,
}
impl Codec<Commands> for Commands {
    fn read(r: &mut Reader) -> Result<Self> {
        let n = r.count(r.remaining())?;
        let mut nodes = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let flags = r.u8()?;
            let count = r.count(r.remaining())?;
            let children = (0..count).map(|_| r.varint()).collect::<Result<_>>()?;
            let redirect = if flags & 8 != 0 {
                Some(r.varint()?)
            } else {
                None
            };
            let kind = flags & 3;
            let (mut name, mut parser, mut suggestions) = (String::new(), None, None);
            if kind >= 1 {
                name = r.string(32767)?;
            }
            if kind == 2 {
                let id = r.varint()?;
                let start = r.position();
                read_argument_properties(r, id)?;
                parser = Some((id, r.since(start).to_vec()));
                if flags & 16 != 0 {
                    suggestions = Some(r.string(32767)?);
                }
            }
            nodes.push(CommandNode {
                kind,
                executable: flags & 4 != 0,
                restricted: flags & 32 != 0,
                children,
                redirect,
                name,
                parser,
                suggestions,
            });
        }
        Ok(Self {
            nodes,
            root: r.varint()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.nodes.len() as i32);
        for n in &v.nodes {
            let mut flags = n.kind & 3;
            if n.executable {
                flags |= 4;
            }
            if n.redirect.is_some() {
                flags |= 8;
            }
            if n.suggestions.is_some() {
                flags |= 16;
            }
            if n.restricted {
                flags |= 32;
            }
            w.u8(flags);
            w.varint(n.children.len() as i32);
            n.children.iter().for_each(|c| w.varint(*c));
            if let Some(r) = n.redirect {
                w.varint(r);
            }
            if n.kind >= 1 {
                w.string(&n.name);
            }
            if n.kind == 2 {
                let (id, props) = n.parser.clone().unwrap_or((5, Vec::new()));
                w.varint(id);
                w.bytes(&props);
                if let Some(s) = &n.suggestions {
                    w.string(s);
                }
            }
        }
        w.varint(v.root);
    }
}

/// `DisplayInfo` of an advancement as sent.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementDisplay {
    pub title: Tag,
    pub description: Tag,
    pub icon: crate::item::Stack,
    /// `AdvancementType` ordinal: task, challenge, goal.
    pub frame: i32,
    pub show_toast: bool,
    pub hidden: bool,
    pub background: Option<String>,
}
/// `ClientboundUpdateAdvancementsPacket.PositionedAdvancement`.
#[derive(Clone, Debug, PartialEq)]
pub struct Advancement {
    pub id: String,
    pub parent: Option<String>,
    pub display: Option<AdvancementDisplay>,
    pub requirements: Vec<Vec<String>>,
    pub telemetry: bool,
    pub x: f32,
    pub y: f32,
}
/// `ClientboundUpdateAdvancementsPacket`: whether it replaces what the
/// client has, the advancements added and removed, the progress of each
/// changed one (criterion, when obtained in epoch milliseconds), and whether
/// to show toasts.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateAdvancements {
    pub reset: bool,
    pub added: Vec<Advancement>,
    pub removed: Vec<String>,
    pub progress: Vec<(String, Vec<(String, Option<i64>)>)>,
    pub show: bool,
}
impl Codec<UpdateAdvancements> for UpdateAdvancements {
    fn read(r: &mut Reader) -> Result<Self> {
        let reset = r.bool()?;
        let n = r.count(r.remaining())?;
        let mut added = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let id = r.string(32767)?;
            let parent = if r.bool()? {
                Some(r.string(32767)?)
            } else {
                None
            };
            let display = if r.bool()? {
                let title = r.nbt()?;
                let description = r.nbt()?;
                let icon = crate::item::Template::read(r)?;
                let frame = r.varint()?;
                let flags = r.i32()?;
                let background = if flags & 1 != 0 {
                    Some(r.string(32767)?)
                } else {
                    None
                };
                Some(AdvancementDisplay {
                    title,
                    description,
                    icon,
                    frame,
                    show_toast: flags & 2 != 0,
                    hidden: flags & 4 != 0,
                    background,
                })
            } else {
                None
            };
            let groups = r.count(r.remaining())?;
            let mut requirements = Vec::with_capacity(groups.min(256));
            for _ in 0..groups {
                let k = r.count(r.remaining())?;
                requirements.push((0..k).map(|_| r.string(32767)).collect::<Result<_>>()?);
            }
            let telemetry = r.bool()?;
            let (x, y) = (r.f32()?, r.f32()?);
            added.push(Advancement {
                id,
                parent,
                display,
                requirements,
                telemetry,
                x,
                y,
            });
        }
        let n = r.count(r.remaining())?;
        let removed = (0..n).map(|_| r.string(32767)).collect::<Result<_>>()?;
        let n = r.count(r.remaining())?;
        let mut progress = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let id = r.string(32767)?;
            let k = r.count(r.remaining())?;
            let mut criteria = Vec::with_capacity(k.min(256));
            for _ in 0..k {
                let name = r.string(32767)?;
                let at = if r.bool()? { Some(r.i64()?) } else { None };
                criteria.push((name, at));
            }
            progress.push((id, criteria));
        }
        let show = r.bool()?;
        Ok(Self {
            reset,
            added,
            removed,
            progress,
            show,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.bool(v.reset);
        w.varint(v.added.len() as i32);
        for a in &v.added {
            w.string(&a.id);
            w.bool(a.parent.is_some());
            if let Some(p) = &a.parent {
                w.string(p);
            }
            w.bool(a.display.is_some());
            if let Some(d) = &a.display {
                w.nbt(&d.title);
                w.nbt(&d.description);
                crate::item::Template::write(&d.icon, w);
                w.varint(d.frame);
                w.i32(
                    i32::from(d.background.is_some())
                        | i32::from(d.show_toast) << 1
                        | i32::from(d.hidden) << 2,
                );
                if let Some(b) = &d.background {
                    w.string(b);
                }
            }
            w.varint(a.requirements.len() as i32);
            for g in &a.requirements {
                w.varint(g.len() as i32);
                for c in g {
                    w.string(c);
                }
            }
            w.bool(a.telemetry);
            w.f32(a.x);
            w.f32(a.y);
        }
        w.varint(v.removed.len() as i32);
        for id in &v.removed {
            w.string(id);
        }
        w.varint(v.progress.len() as i32);
        for (id, criteria) in &v.progress {
            w.string(id);
            w.varint(criteria.len() as i32);
            for (name, at) in criteria {
                w.string(name);
                w.bool(at.is_some());
                if let Some(at) = at {
                    w.i64(*at);
                }
            }
        }
        w.bool(v.show);
    }
}
packet!(
    /// ClientboundAwardStatsPacket: (stat type, value, count) by network id.
    AwardStats { stats: Vec<(i32, i32, i32)> => StatList }
);
/// `Object2IntMap<Stat<?>>`: each stat's type and value ids, then its count.
pub struct StatList;
impl Codec<Vec<(i32, i32, i32)>> for StatList {
    fn read(r: &mut Reader) -> Result<Vec<(i32, i32, i32)>> {
        let n = r.count(r.remaining())?;
        (0..n)
            .map(|_| Ok((r.varint()?, r.varint()?, r.varint()?)))
            .collect()
    }
    fn write(v: &Vec<(i32, i32, i32)>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for &(a, b, c) in v {
            w.varint(a);
            w.varint(b);
            w.varint(c);
        }
    }
}

/// `ClientboundBossEventPacket` (`boss_event`).
#[derive(Clone, Debug, PartialEq)]
pub enum BossAction {
    Add {
        name: Tag,
        progress: f32,
        /// `BossBarColor` ordinal.
        color: i32,
        /// `BossBarOverlay` ordinal.
        overlay: i32,
        darken: bool,
        music: bool,
        fog: bool,
    },
    Remove,
    UpdateProgress {
        progress: f32,
    },
    UpdateName {
        name: Tag,
    },
    UpdateStyle {
        color: i32,
        overlay: i32,
    },
    UpdateProperties {
        darken: bool,
        music: bool,
        fog: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct BossEvent {
    pub id: u128,
    pub action: BossAction,
}
fn boss_flags(darken: bool, music: bool, fog: bool) -> u8 {
    u8::from(darken) | (u8::from(music) << 1) | (u8::from(fog) << 2)
}
fn split_flags(flags: u8) -> (bool, bool, bool) {
    (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0)
}
impl Codec<BossEvent> for BossEvent {
    fn read(r: &mut Reader) -> Result<BossEvent> {
        let id = r.uuid()?;
        let action = match r.varint()? {
            0 => {
                let name = r.nbt()?;
                let progress = r.f32()?;
                let color = r.varint()?;
                let overlay = r.varint()?;
                let (darken, music, fog) = split_flags(r.u8()?);
                BossAction::Add {
                    name,
                    progress,
                    color,
                    overlay,
                    darken,
                    music,
                    fog,
                }
            }
            1 => BossAction::Remove,
            2 => BossAction::UpdateProgress { progress: r.f32()? },
            3 => BossAction::UpdateName { name: r.nbt()? },
            4 => BossAction::UpdateStyle {
                color: r.varint()?,
                overlay: r.varint()?,
            },
            5 => {
                let (darken, music, fog) = split_flags(r.u8()?);
                BossAction::UpdateProperties { darken, music, fog }
            }
            op => return Err(Error::new(format!("unknown boss event operation {op}"))),
        };
        Ok(BossEvent { id, action })
    }
    fn write(v: &BossEvent, w: &mut Writer) {
        w.uuid(v.id);
        match &v.action {
            BossAction::Add {
                name,
                progress,
                color,
                overlay,
                darken,
                music,
                fog,
            } => {
                w.varint(0);
                w.nbt(name);
                w.f32(*progress);
                w.varint(*color);
                w.varint(*overlay);
                w.u8(boss_flags(*darken, *music, *fog));
            }
            BossAction::Remove => w.varint(1),
            BossAction::UpdateProgress { progress } => {
                w.varint(2);
                w.f32(*progress);
            }
            BossAction::UpdateName { name } => {
                w.varint(3);
                w.nbt(name);
            }
            BossAction::UpdateStyle { color, overlay } => {
                w.varint(4);
                w.varint(*color);
                w.varint(*overlay);
            }
            BossAction::UpdateProperties { darken, music, fog } => {
                w.varint(5);
                w.u8(boss_flags(*darken, *music, *fog));
            }
        }
    }
}

packet_enum!(Clientbound, crate::ids::play::clientbound {
    BundleDelimiter(Empty) = BUNDLE_DELIMITER,
    AddEntity(AddEntity) = ADD_ENTITY,
    Animate(Animate) = ANIMATE,
    AwardStats(AwardStats) = AWARD_STATS,
    BlockChangedAck(BlockChangedAck) = BLOCK_CHANGED_ACK,
    BlockDestruction(BlockDestruction) = BLOCK_DESTRUCTION,
    BlockEntityData(BlockEntityData) = BLOCK_ENTITY_DATA,
    BlockEvent(BlockEvent) = BLOCK_EVENT,
    BlockUpdate(BlockUpdate) = BLOCK_UPDATE,
    BossEvent(BossEvent) = BOSS_EVENT,
    ChangeDifficulty(ChangeDifficulty) = CHANGE_DIFFICULTY,
    ChunkBatchFinished(ChunkBatchFinished) = CHUNK_BATCH_FINISHED,
    ChunkBatchStart(Empty) = CHUNK_BATCH_START,
    ClearTitles(Bool1) = CLEAR_TITLES,
    Commands(Commands) = COMMANDS,
    RecipeBookAdd(super::recipes::RecipeBookAdd) = RECIPE_BOOK_ADD,
    RecipeBookRemove(super::recipes::RecipeBookRemove) = RECIPE_BOOK_REMOVE,
    RecipeBookSettings(super::recipes::RecipeBookSettings) = RECIPE_BOOK_SETTINGS,
    PlaceGhostRecipe(super::recipes::PlaceGhostRecipe) = PLACE_GHOST_RECIPE,
    ContainerClose(ContainerClose) = CONTAINER_CLOSE,
    ContainerSetContent(ContainerSetContent) = CONTAINER_SET_CONTENT,
    ContainerSetData(ContainerSetData) = CONTAINER_SET_DATA,
    ContainerSetSlot(ContainerSetSlot) = CONTAINER_SET_SLOT,
    CookieRequest(CookieRequest) = COOKIE_REQUEST,
    Cooldown(Cooldown) = COOLDOWN,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    Disconnect(Disconnect) = DISCONNECT,
    DisguisedChat(DisguisedChat) = DISGUISED_CHAT,
    EntityEvent(EntityEvent) = ENTITY_EVENT,
    EntityPositionSync(EntityPositionSync) = ENTITY_POSITION_SYNC,
    ForgetLevelChunk(ForgetLevelChunk) = FORGET_LEVEL_CHUNK,
    GameEvent(GameEvent) = GAME_EVENT,
    GameRuleValues(GameRuleValues) = GAME_RULE_VALUES,
    HurtAnimation(HurtAnimation) = HURT_ANIMATION,
    DamageEvent(DamageEvent) = DAMAGE_EVENT,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    InitializeBorder(InitializeBorder) = INITIALIZE_BORDER,
    LevelChunkWithLight(LevelChunkWithLight) = LEVEL_CHUNK_WITH_LIGHT,
    LevelEvent(LevelEvent) = LEVEL_EVENT,
    LightUpdate(LightUpdate) = LIGHT_UPDATE,
    Login(Login) = LOGIN,
    MapItemData(MapItemData) = MAP_ITEM_DATA,
    MoveEntityPos(MoveEntityPos) = MOVE_ENTITY_POS,
    MoveEntityPosRot(MoveEntityPosRot) = MOVE_ENTITY_POS_ROT,
    MoveEntityRot(MoveEntityRot) = MOVE_ENTITY_ROT,
    MerchantOffers(MerchantOffers) = MERCHANT_OFFERS,
    MountScreenOpen(MountScreenOpen) = MOUNT_SCREEN_OPEN,
    OpenScreen(OpenScreen) = OPEN_SCREEN,
    OpenBook(OpenBook) = OPEN_BOOK,
    OpenSignEditor(OpenSignEditor) = OPEN_SIGN_EDITOR,
    Ping(Ping) = PING,
    PongResponse(crate::packets::status::Ping) = PONG_RESPONSE,
    PlayerAbilities(PlayerAbilities) = PLAYER_ABILITIES,
    PlayerChat(PlayerChat) = PLAYER_CHAT,
    PlayerCombatKill(PlayerCombatKill) = PLAYER_COMBAT_KILL,
    PlayerInfoRemove(PlayerInfoRemove) = PLAYER_INFO_REMOVE,
    PlayerInfoUpdate(PlayerInfoUpdate) = PLAYER_INFO_UPDATE,
    PlayerLookAt(PlayerLookAt) = PLAYER_LOOK_AT,
    PlayerPosition(PlayerPosition) = PLAYER_POSITION,
    PlayerRotation(PlayerRotation) = PLAYER_ROTATION,
    RemoveEntities(RemoveEntities) = REMOVE_ENTITIES,
    RemoveMobEffect(RemoveMobEffect) = REMOVE_MOB_EFFECT,
    ResetScore(ResetScore) = RESET_SCORE,
    ResourcePackPop(ResourcePackPop) = RESOURCE_PACK_POP,
    ResourcePackPush(ResourcePackPush) = RESOURCE_PACK_PUSH,
    Respawn(Respawn) = RESPAWN,
    RotateHead(RotateHead) = ROTATE_HEAD,
    SectionBlocksUpdate(SectionBlocksUpdate) = SECTION_BLOCKS_UPDATE,
    ServerData(ServerData) = SERVER_DATA,
    SetActionBarText(Text) = SET_ACTION_BAR_TEXT,
    SetBorderCenter(SetBorderCenter) = SET_BORDER_CENTER,
    SetBorderLerpSize(SetBorderLerpSize) = SET_BORDER_LERP_SIZE,
    SetBorderSize(SetBorderSize) = SET_BORDER_SIZE,
    SetBorderWarningDelay(SetBorderWarningDelay) = SET_BORDER_WARNING_DELAY,
    SetBorderWarningDistance(SetBorderWarningDistance) = SET_BORDER_WARNING_DISTANCE,
    SetChunkCacheCenter(SetChunkCacheCenter) = SET_CHUNK_CACHE_CENTER,
    SetChunkCacheRadius(Distance) = SET_CHUNK_CACHE_RADIUS,
    SetCursorItem(SetCursorItem) = SET_CURSOR_ITEM,
    SetDefaultSpawnPosition(SetDefaultSpawnPosition) = SET_DEFAULT_SPAWN_POSITION,
    SetDisplayObjective(SetDisplayObjective) = SET_DISPLAY_OBJECTIVE,
    SetEntityData(SetEntityData) = SET_ENTITY_DATA,
    SetEntityMotion(SetEntityMotion) = SET_ENTITY_MOTION,
    SetEntityLink(SetEntityLink) = SET_ENTITY_LINK,
    SetEquipment(SetEquipment) = SET_EQUIPMENT,
    SetExperience(SetExperience) = SET_EXPERIENCE,
    SetHealth(SetHealth) = SET_HEALTH,
    SetHeldSlot(SetHeldSlot) = SET_HELD_SLOT,
    SetObjective(SetObjective) = SET_OBJECTIVE,
    SetPassengers(SetPassengers) = SET_PASSENGERS,
    MoveVehicle(MoveVehicle) = MOVE_VEHICLE,
    ProjectilePower(ProjectilePower) = PROJECTILE_POWER,
    SetPlayerInventory(SetPlayerInventory) = SET_PLAYER_INVENTORY,
    SetPlayerTeam(SetPlayerTeam) = SET_PLAYER_TEAM,
    SetScore(SetScore) = SET_SCORE,
    SetSimulationDistance(Distance) = SET_SIMULATION_DISTANCE,
    SetSubtitleText(Text) = SET_SUBTITLE_TEXT,
    SetTime(SetTime) = SET_TIME,
    SetTitleText(Text) = SET_TITLE_TEXT,
    SetTitlesAnimation(TitlesAnimation) = SET_TITLES_ANIMATION,
    Sound(Sound) = SOUND,
    SoundEntity(SoundEntity) = SOUND_ENTITY,
    StopSound(StopSound) = STOP_SOUND,
    StartConfiguration(Empty) = START_CONFIGURATION,
    StoreCookie(StoreCookie) = STORE_COOKIE,
    // A common packet: the same `configuration::ServerLinks` the
    // configuration phase sends.
    ServerLinks(ServerLinks) = SERVER_LINKS,
    SwingAnimation(SwingAnimation) = SWING_ANIMATION,
    SystemChat(SystemChat) = SYSTEM_CHAT,
    TabList(TabList) = TAB_LIST,
    TakeItemEntity(TakeItemEntity) = TAKE_ITEM_ENTITY,
    TeleportEntity(TeleportEntity) = TELEPORT_ENTITY,
    TickingState(TickingState) = TICKING_STATE,
    TickingStep(TickingStep) = TICKING_STEP,
    Transfer(Transfer) = TRANSFER,
    UpdateAdvancements(UpdateAdvancements) = UPDATE_ADVANCEMENTS,
    UpdateAttributes(UpdateAttributes) = UPDATE_ATTRIBUTES,
    LevelParticles(LevelParticles) = LEVEL_PARTICLES,
    Explode(Explode) = EXPLODE,
    UpdateMobEffect(UpdateMobEffect) = UPDATE_MOB_EFFECT,
    UpdateTags(UpdateTags) = UPDATE_TAGS,
    TrackedWaypoint(TrackedWaypoint) = WAYPOINT,
    ClearDialog(Empty) = CLEAR_DIALOG,
    ShowDialog(ShowDialog) = SHOW_DIALOG,
});
packet!(
    /// ClientboundClearTitlesPacket: whether to also reset the timings.
    Bool1 { value: bool => Bool }
);

/// ClientboundTrackedWaypointPacket.Operation, by ordinal.
pub mod waypoint_operation {
    pub const TRACK: i32 = 0;
    pub const UNTRACK: i32 = 1;
    pub const UPDATE: i32 = 2;
}

/// TrackedWaypoint's identifier: `Either<UUID, String>`.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointId {
    Uuid(u128),
    Name(String),
}

/// What a TrackedWaypoint points at (its `Type` and contents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaypointTarget {
    /// `TrackedWaypoint.empty`: an untracked waypoint.
    Empty,
    /// A block position (`Vec3iWaypoint`).
    Block([i32; 3]),
    /// A chunk (`ChunkWaypoint`).
    Chunk([i32; 2]),
    /// An angle in radians (`AzimuthWaypoint`).
    Azimuth(f32),
}

/// ClientboundTrackedWaypointPacket: the locator bar's waypoints.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedWaypoint {
    /// `waypoint_operation`.
    pub operation: i32,
    pub id: WaypointId,
    /// Waypoint.Icon: the style (a `waypoint_style_asset` key) and an RGB colour.
    pub style: String,
    pub color: Option<i32>,
    pub target: WaypointTarget,
}

impl Codec<TrackedWaypoint> for TrackedWaypoint {
    fn read(r: &mut Reader) -> Result<TrackedWaypoint> {
        let operation = r.varint()?;
        if !(0..=2).contains(&operation) {
            return Err(Error::new(format!("waypoint operation {operation}")));
        }
        let id = if r.bool()? {
            WaypointId::Uuid(r.uuid()?)
        } else {
            WaypointId::Name(r.string(32767)?)
        };
        let style = Ident::read(r)?;
        // ByteBufCodecs.RGB_COLOR: three bytes, read back as an opaque ARGB colour.
        let color = if r.bool()? {
            let [red, green, blue] = [r.u8()?, r.u8()?, r.u8()?].map(i32::from);
            Some(-16_777_216 | red << 16 | green << 8 | blue)
        } else {
            None
        };
        let target = match r.varint()? {
            0 => WaypointTarget::Empty,
            1 => WaypointTarget::Block([r.varint()?, r.varint()?, r.varint()?]),
            2 => WaypointTarget::Chunk([r.varint()?, r.varint()?]),
            3 => WaypointTarget::Azimuth(r.f32()?),
            other => return Err(Error::new(format!("waypoint type {other}"))),
        };
        Ok(TrackedWaypoint {
            operation,
            id,
            style,
            color,
            target,
        })
    }
    fn write(v: &TrackedWaypoint, w: &mut Writer) {
        w.varint(v.operation);
        match &v.id {
            WaypointId::Uuid(uuid) => {
                w.bool(true);
                w.uuid(*uuid);
            }
            WaypointId::Name(name) => {
                w.bool(false);
                w.string(name);
            }
        }
        w.string(&v.style);
        w.bool(v.color.is_some());
        if let Some(color) = v.color {
            for shift in [16, 8, 0] {
                w.u8((color >> shift) as u8);
            }
        }
        match v.target {
            WaypointTarget::Empty => w.varint(0),
            WaypointTarget::Block([x, y, z]) => {
                w.varint(1);
                w.varint(x);
                w.varint(y);
                w.varint(z);
            }
            WaypointTarget::Chunk([x, z]) => {
                w.varint(2);
                w.varint(x);
                w.varint(z);
            }
            WaypointTarget::Azimuth(angle) => {
                w.varint(3);
                w.f32(angle);
            }
        }
    }
}

// ---- serverbound -------------------------------------------------------

packet!(AcceptTeleportation { id: i32 => VarInt, x: f64 => Double, y: f64 => Double, z: f64 => Double, yaw: f32 => Float, pitch: f32 => Float });
packet!(Attack { id: i32 => VarInt });
packet!(TeleportToEntity { uuid: u128 => Uuid });
packet!(
    /// GameType ids: 0 survival, 1 creative, 2 adventure, 3 spectator.
    ChangeGameMode { mode: i32 => VarInt }
);
packet!(LastSeenUpdate { offset: i32 => VarInt, acknowledged: [u8; 3] => Fixed3, checksum: i8 => Byte });
/// LastSeenMessages.Update's fixed 20-bit BitSet (3 bytes).
pub struct Fixed3;
impl Codec<[u8; 3]> for Fixed3 {
    fn read(r: &mut Reader) -> Result<[u8; 3]> {
        Ok(r.bytes(3)?.try_into().expect("3 bytes"))
    }
    fn write(v: &[u8; 3], w: &mut Writer) {
        w.bytes(v)
    }
}
packet!(Chat {
    message: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    signature: Option<Vec<u8>> => Opt<Signature>,
    last_seen: LastSeenUpdate => LastSeenUpdate,
});
packet!(ChatCommand { command: String => Str });
packet!(ChatAck { offset: i32 => VarInt });
/// ArgumentSignatures: each signed argument's name (at most 16 chars) and
/// signature, at most 8.
pub struct ArgumentSignatures;
impl Codec<Vec<(String, Vec<u8>)>> for ArgumentSignatures {
    fn read(r: &mut Reader) -> Result<Vec<(String, Vec<u8>)>> {
        let n = r.count(8)?;
        (0..n)
            .map(|_| Ok((r.string(16)?, r.bytes(256)?.to_vec())))
            .collect()
    }
    fn write(v: &Vec<(String, Vec<u8>)>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for (name, signature) in v {
            w.string(name);
            w.bytes(signature);
        }
    }
}
packet!(ChatCommandSigned {
    command: String => Str,
    timestamp: i64 => Long,
    salt: i64 => Long,
    arguments: Vec<(String, Vec<u8>)> => ArgumentSignatures,
    last_seen: LastSeenUpdate => LastSeenUpdate,
});
packet!(
    /// RemoteChatSession.Data: the session id, then ProfilePublicKey.Data
    /// (expiry in epoch milliseconds, X.509 key, the services' signature).
    ChatSessionUpdate {
        session: u128 => Uuid,
        expires_at: i64 => Long,
        key: Vec<u8> => BytesN<512>,
        key_signature: Vec<u8> => BytesN<4096>,
    }
);
packet!(ChangeDifficultyRequest { difficulty: i32 => VarInt });
packet!(LockDifficulty { locked: bool => Bool });
// ServerboundPlayerAbilitiesPacket: only whether the player flies (2).
packet!(PlayerAbilitiesRequest { flags: i8 => Byte });
packet!(SetGameRule { entries: Vec<GameRuleEntry> => List<GameRuleEntry> });
packet!(ChunkBatchReceived { chunks_per_tick: f32 => Float });
packet!(
    /// ServerboundClientCommandPacket.Action: 0 respawn, 1 request stats, 2 request game rules.
    ClientCommand { action: i32 => VarInt }
);
/// Changed slots: (slot, hashed stack) pairs.
pub struct ChangedSlots;
impl Codec<Vec<(i16, Option<Hashed>)>> for ChangedSlots {
    fn read(r: &mut Reader) -> Result<Vec<(i16, Option<Hashed>)>> {
        let n = r.count(128)?;
        (0..n)
            .map(|_| Ok((r.i16()?, HashedSlot::read(r)?)))
            .collect()
    }
    fn write(v: &Vec<(i16, Option<Hashed>)>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for (slot, h) in v {
            w.i16(*slot);
            HashedSlot::write(h, w);
        }
    }
}
packet!(
    /// ContainerInput: 0 pickup, 1 quick move, 2 swap, 3 clone, 4 throw, 5 quick craft, 6 pickup all.
    ContainerClick {
        container: i32 => VarInt,
        state: i32 => VarInt,
        slot: i16 => Short,
        button: i8 => Byte,
        input: i32 => VarInt,
        changed: Vec<(i16, Option<Hashed>)> => ChangedSlots,
        carried: Option<Hashed> => HashedSlot,
    }
);
packet!(SelectTrade { item: i32 => VarInt });
packet!(
    /// ServerboundContainerButtonClickPacket: `AbstractContainerMenu.clickMenuButton`
    /// (the loom's patterns, the stonecutter's recipes, the enchanting table's offers).
    ContainerButtonClick { container: i32 => VarInt, button: i32 => VarInt }
);
packet!(
    /// ServerboundContainerSlotStateChangedPacket: the crafter's slot toggle
    /// (`CrafterBlockEntity.setSlotState`).
    ContainerSlotStateChanged { slot: i32 => VarInt, container: i32 => VarInt, enabled: bool => Bool }
);
packet!(
    /// ServerboundRenameItemPacket: `AnvilMenu.setItemName`.
    RenameItem { name: String => Str }
);
packet!(
    /// ServerboundSignUpdatePacket: the four lines the player wrote on a side.
    SignUpdate { pos: [i32; 3] => Pos, lines: [String; 4] => SignLines, front: bool => SignSide }
);
packet!(
    /// ServerboundSetCommandBlockPacket: the command block's command, its
    /// mode (`CommandBlockEntity.Mode` ordinal) and the track output (1),
    /// conditional (2) and automatic (4) flags.
    SetCommandBlock { pos: [i32; 3] => Pos, command: String => Str, mode: i32 => VarInt, flags: i8 => Byte }
);
packet!(
    /// ServerboundEditBookPacket: the book's slot (a hotbar slot, or 40 for
    /// the off hand), its pages and, to sign it, its title.
    EditBook {
        slot: i32 => VarInt,
        pages: Vec<String> => BookPages,
        title: Option<String> => Opt<StrN<32>>,
    }
);
/// `stringUtf8(1024)` in `list(100)`: a book's pages.
pub struct BookPages;
impl Codec<Vec<String>> for BookPages {
    fn read(r: &mut Reader) -> Result<Vec<String>> {
        let n = r.varint()?;
        if !(0..=100).contains(&n) {
            return Err(Error::new(format!("{n} book pages, at most 100")));
        }
        (0..n).map(|_| r.string(1024)).collect()
    }
    fn write(v: &Vec<String>, w: &mut Writer) {
        List::<StrN<1024>>::write(v, w);
    }
}
/// SignTextSlot.STREAM_CODEC: VarInt 0 for the back, 1 for the front (any
/// other id reads as the back, ByIdMap's ZERO strategy).
pub struct SignSide;
impl Codec<bool> for SignSide {
    fn read(r: &mut Reader) -> Result<bool> {
        Ok(r.varint()? == 1)
    }
    fn write(v: &bool, w: &mut Writer) {
        w.varint(i32::from(*v));
    }
}
/// `stringUtf8(384)` in `fixedSizeList(4)`: four lines, no count.
pub struct SignLines;
impl Codec<[String; 4]> for SignLines {
    fn read(r: &mut Reader) -> Result<[String; 4]> {
        Ok([
            r.string(384)?,
            r.string(384)?,
            r.string(384)?,
            r.string(384)?,
        ])
    }
    fn write(v: &[String; 4], w: &mut Writer) {
        for line in v {
            w.string(line);
        }
    }
}
packet!(
    /// ServerboundSetBeaconPacket: `BeaconMenu.updateEffects`, each power an
    /// optional `mob_effect` id.
    SetBeacon { primary: Option<i32> => Opt<VarInt>, secondary: Option<i32> => Opt<VarInt> }
);
packet!(Interact { id: i32 => VarInt, hand: i32 => VarInt, location: [f64; 3] => LpVec3, secondary: bool => Bool });
packet!(
    /// ServerboundMovePlayerPacket flags: 1 on ground, 2 horizontal collision.
    MovePlayerPos { x: f64 => Double, y: f64 => Double, z: f64 => Double, flags: u8 => UByte }
);
packet!(MovePlayerPosRot { x: f64 => Double, y: f64 => Double, z: f64 => Double, yaw: f32 => Float, pitch: f32 => Float, flags: u8 => UByte });
packet!(MovePlayerRot { yaw: f32 => Float, pitch: f32 => Float, flags: u8 => UByte });
packet!(MovePlayerStatus { flags: u8 => UByte });
packet!(
    /// ServerboundMoveVehiclePacket: where the rider moved the vehicle it
    /// controls (PositionAndRotation, then onGround).
    MoveVehicleRequest { position: [f64; 3] => Vec3d, yaw: f32 => Float, pitch: f32 => Float, on_ground: bool => Bool }
);
packet!(
    /// ServerboundPaddleBoatPacket: the boat's left and right paddles.
    PaddleBoat { left: bool => Bool, right: bool => Bool }
);
packet!(PickItemFromBlock { pos: [i32; 3] => Pos, include_data: bool => Bool });
packet!(PickItemFromEntity { id: i32 => VarInt, include_data: bool => Bool });
packet!(
    /// ServerboundPlayerActionPacket.Action: 0 start destroy, 1 change destroy
    /// direction, 2 abort, 3 stop destroy, 4 drop all, 5 drop item, 6 release
    /// use item, 7 swap offhand, 8 stab. Direction is the 3D data value.
    PlayerAction { action: i32 => VarInt, pos: [i32; 3] => Pos, direction: u8 => UByte, sequence: i32 => VarInt }
);
packet!(
    /// ServerboundPlayerCommandPacket.Action: 0 stop sleeping, 1 start sprinting,
    /// 2 stop sprinting, 3/4 riding jump, 5 open inventory, 6 start fall flying.
    PlayerCommand { id: i32 => VarInt, action: i32 => VarInt, data: i32 => VarInt }
);
packet!(
    /// Input flags: forward 1, backward 2, left 4, right 8, jump 16, shift 32, sprint 64.
    PlayerInput { flags: u8 => UByte }
);
packet!(
    /// ServerboundSelectBundleItemPacket: the menu slot of a bundle and the
    /// index of its selected item (-1: none).
    SelectBundleItem { slot: i32 => VarInt, index: i32 => VarInt }
);
packet!(SetCarriedItem { slot: i16 => Short });
packet!(SetCreativeModeSlot { slot: i16 => Short, item: Option<Stack> => DelimitedSlot });
packet!(UseItem { hand: i32 => VarInt, sequence: i32 => VarInt, yaw: f32 => Float, pitch: f32 => Float });
packet!(UseItemOn {
    hand: i32 => VarInt,
    pos: [i32; 3] => Pos,
    face: i32 => VarInt,
    cursor: [f32; 3] => Cursor,
    inside: bool => Bool,
    world_border: bool => Bool,
    sequence: i32 => VarInt,
});
/// The click location relative to the block (three floats).
pub struct Cursor;
impl Codec<[f32; 3]> for Cursor {
    fn read(r: &mut Reader) -> Result<[f32; 3]> {
        Ok([r.f32()?, r.f32()?, r.f32()?])
    }
    fn write(v: &[f32; 3], w: &mut Writer) {
        v.iter().for_each(|f| w.f32(*f));
    }
}

packet_enum!(Serverbound, crate::ids::play::serverbound {
    ChangeDifficulty(ChangeDifficultyRequest) = CHANGE_DIFFICULTY,
    LockDifficulty(LockDifficulty) = LOCK_DIFFICULTY,
    SetGameRule(SetGameRule) = SET_GAME_RULE,
    AcceptTeleportation(AcceptTeleportation) = ACCEPT_TELEPORTATION,
    Attack(Attack) = ATTACK,
    TeleportToEntity(TeleportToEntity) = TELEPORT_TO_ENTITY,
    ChangeGameMode(ChangeGameMode) = CHANGE_GAME_MODE,
    Chat(Chat) = CHAT,
    ChatAck(ChatAck) = CHAT_ACK,
    ChatCommand(ChatCommand) = CHAT_COMMAND,
    ChatCommandSigned(ChatCommandSigned) = CHAT_COMMAND_SIGNED,
    ChatSessionUpdate(ChatSessionUpdate) = CHAT_SESSION_UPDATE,
    PlaceRecipe(super::recipes::PlaceRecipe) = PLACE_RECIPE,
    RecipeBookChangeSettings(super::recipes::RecipeBookChangeSettings) = RECIPE_BOOK_CHANGE_SETTINGS,
    ChunkBatchReceived(ChunkBatchReceived) = CHUNK_BATCH_RECEIVED,
    ClientCommand(ClientCommand) = CLIENT_COMMAND,
    ClientInformation(ClientInformation) = CLIENT_INFORMATION,
    ClientTickEnd(Empty) = CLIENT_TICK_END,
    ConfigurationAcknowledged(Empty) = CONFIGURATION_ACKNOWLEDGED,
    ContainerButtonClick(ContainerButtonClick) = CONTAINER_BUTTON_CLICK,
    ContainerClick(ContainerClick) = CONTAINER_CLICK,
    ContainerClose(ContainerClose) = CONTAINER_CLOSE,
    CookieResponse(CookieResponse) = COOKIE_RESPONSE,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    CustomClickAction(CustomClickAction) = CUSTOM_CLICK_ACTION,
    EditBook(EditBook) = EDIT_BOOK,
    Interact(Interact) = INTERACT,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    MovePlayerPos(MovePlayerPos) = MOVE_PLAYER_POS,
    MovePlayerPosRot(MovePlayerPosRot) = MOVE_PLAYER_POS_ROT,
    MovePlayerRot(MovePlayerRot) = MOVE_PLAYER_ROT,
    MovePlayerStatus(MovePlayerStatus) = MOVE_PLAYER_STATUS_ONLY,
    MoveVehicle(MoveVehicleRequest) = MOVE_VEHICLE,
    PaddleBoat(PaddleBoat) = PADDLE_BOAT,
    PickItemFromBlock(PickItemFromBlock) = PICK_ITEM_FROM_BLOCK,
    PickItemFromEntity(PickItemFromEntity) = PICK_ITEM_FROM_ENTITY,
    PingRequest(crate::packets::status::Ping) = PING_REQUEST,
    PlayerAbilities(PlayerAbilitiesRequest) = PLAYER_ABILITIES,
    PlayerAction(PlayerAction) = PLAYER_ACTION,
    PlayerCommand(PlayerCommand) = PLAYER_COMMAND,
    PlayerInput(PlayerInput) = PLAYER_INPUT,
    PlayerLoaded(Empty) = PLAYER_LOADED,
    Pong(Ping) = PONG,
    RenameItem(RenameItem) = RENAME_ITEM,
    ContainerSlotStateChanged(ContainerSlotStateChanged) = CONTAINER_SLOT_STATE_CHANGED,
    Punch(Empty) = PUNCH,
    ResourcePack(ResourcePackResponse) = RESOURCE_PACK,
    SelectBundleItem(SelectBundleItem) = BUNDLE_ITEM_SELECTED,
    SelectTrade(SelectTrade) = SELECT_TRADE,
    SetBeacon(SetBeacon) = SET_BEACON,
    SetCarriedItem(SetCarriedItem) = SET_CARRIED_ITEM,
    SetCommandBlock(SetCommandBlock) = SET_COMMAND_BLOCK,
    SetCreativeModeSlot(SetCreativeModeSlot) = SET_CREATIVE_MODE_SLOT,
    SignUpdate(SignUpdate) = SIGN_UPDATE,
    UseItem(UseItem) = USE_ITEM,
    UseItemOn(UseItemOn) = USE_ITEM_ON,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::BlockEntityInfo, packets::round_trip, text::literal};

    #[test]
    fn merchant_cost_predicate_is_a_typed_list_without_patch_counts() {
        // ItemCost.STREAM_CODEC: item=7, count=1, one typed component,
        // potion_contents=53, potion present (water=0), no color/effects/name.
        // Result: count=1, item=8, empty component patch. Secondary cost:
        // item=9, count=2, empty predicate, followed by the offer fields.
        let mut bytes = vec![1, 7, 1, 1, 53, 1, 0, 0, 0, 0, 1, 8, 0, 0, 1, 9, 2, 0, 0];
        bytes.extend_from_slice(&[0; 4]); // uses
        bytes.extend_from_slice(&2i32.to_be_bytes()); // max uses
        bytes.extend_from_slice(&[0; 16]); // xp, special price, multiplier, demand
        let mut r = Reader::new(&bytes);
        let offers = Offers::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!(
            offers[0].cost_a.components,
            vec![(
                53,
                Component::Potion {
                    potion: Some(0),
                    color: None,
                    rest: Vec::new()
                }
            )]
        );
        assert!(offers[0].cost_b.as_ref().unwrap().components.is_empty());
        let mut w = Writer::new();
        Offers::write(&offers, &mut w);
        assert_eq!(w.data, bytes);
        for len in 0..bytes.len() {
            assert!(Offers::read(&mut Reader::new(&bytes[..len])).is_err());
        }
    }

    #[test]
    fn serverbound_abilities_carry_only_flying() {
        // ServerboundPlayerAbilitiesPacket.write: one byte, 2 when flying.
        let flying = Serverbound::PlayerAbilities(PlayerAbilitiesRequest { flags: 2 });
        assert_eq!(
            flying.encode(),
            [crate::ids::play::serverbound::PLAYER_ABILITIES as u8, 2]
        );
        round_trip(flying, Serverbound::encode, Serverbound::decode);
    }

    #[test]
    fn world_options_packets_match_vanilla_stream_codecs() {
        // mcsrc.dev 26.3: difficulty is a VarInt, locking is a Boolean,
        // rule entries are ResourceKey identifiers followed by UTF-8 values.
        let difficulty = Serverbound::ChangeDifficulty(ChangeDifficultyRequest { difficulty: 3 });
        assert_eq!(&difficulty.encode()[1..], &[3]);
        round_trip(difficulty, Serverbound::encode, Serverbound::decode);
        let lock = Serverbound::LockDifficulty(LockDifficulty { locked: true });
        assert_eq!(&lock.encode()[1..], &[1]);
        round_trip(lock, Serverbound::encode, Serverbound::decode);
        let payload = b"\x01\x18minecraft:keep_inventory\x04true";
        let entry = GameRuleEntry {
            name: "minecraft:keep_inventory".into(),
            value: "true".into(),
        };
        let set = Serverbound::SetGameRule(SetGameRule {
            entries: vec![entry.clone()],
        });
        assert_eq!(&set.encode()[1..], payload);
        assert_eq!(
            Serverbound::decode(crate::ids::play::serverbound::SET_GAME_RULE, payload).unwrap(),
            set
        );
        let values = Clientbound::GameRuleValues(GameRuleValues {
            entries: vec![entry],
        });
        assert_eq!(&values.encode()[1..], payload);
        assert_eq!(
            Clientbound::decode(crate::ids::play::clientbound::GAME_RULE_VALUES, payload).unwrap(),
            values
        );
        assert!(
            Serverbound::decode(
                crate::ids::play::serverbound::SET_GAME_RULE,
                &payload[..payload.len() - 1]
            )
            .is_err()
        );
    }

    fn spawn() -> SpawnInfo {
        SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: -123,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: Some(GlobalPos {
                dimension: "minecraft:the_nether".into(),
                pos: [1, 2, 3],
            }),
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    #[test]
    fn tracked_waypoint_bytes() {
        // TRACK, a UUID, the default style, red (#FF5555), a block position.
        let track = Clientbound::TrackedWaypoint(TrackedWaypoint {
            operation: waypoint_operation::TRACK,
            id: WaypointId::Uuid(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10),
            style: "minecraft:default".into(),
            color: Some(0xFFFF_5555_u32 as i32),
            target: WaypointTarget::Block([1, -2, 300]),
        });
        let mut expected = vec![141, 1, 0, 1];
        expected.extend(1..=16u8);
        expected.push(17);
        expected.extend(b"minecraft:default");
        expected.extend([
            1, 0xFF, 0x55, 0x55, 1, 1, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 0xAC, 0x02,
        ]);
        assert_eq!(track.encode(), expected);
        assert_eq!(Clientbound::decode(141, &expected[2..]).unwrap(), track);
        for target in [
            WaypointTarget::Empty,
            WaypointTarget::Chunk([-3, 4]),
            WaypointTarget::Azimuth(-1.25),
        ] {
            let packet = Clientbound::TrackedWaypoint(TrackedWaypoint {
                operation: waypoint_operation::UPDATE,
                id: WaypointId::Name("spawn".into()),
                style: "minecraft:bowtie".into(),
                color: None,
                target,
            });
            let bytes = packet.encode();
            assert_eq!(Clientbound::decode(141, &bytes[2..]).unwrap(), packet);
        }
    }

    #[test]
    fn clientbound_round_trips() {
        let packets = vec![
            Clientbound::BundleDelimiter(Empty),
            Clientbound::PongResponse(crate::packets::status::Ping { time: 1_234_567 }),
            Clientbound::SetBorderCenter(SetBorderCenter { x: 1.5, z: -2.0 }),
            Clientbound::SetBorderLerpSize(SetBorderLerpSize {
                old_size: 10.0,
                new_size: 200.0,
                lerp_time: 1200,
            }),
            Clientbound::SetBorderSize(SetBorderSize { size: 30.0 }),
            Clientbound::SetBorderWarningDelay(SetBorderWarningDelay { delay: 300 }),
            Clientbound::SetBorderWarningDistance(SetBorderWarningDistance { blocks: 5 }),
            Clientbound::SetDisplayObjective(SetDisplayObjective {
                slot: 0,
                objective: "kills".into(),
            }),
            Clientbound::ResetScore(ResetScore {
                owner: "Steve".into(),
                objective: Some("kills".into()),
            }),
            Clientbound::SetScore(SetScore {
                owner: "Steve".into(),
                objective: "kills".into(),
                score: -3,
                display: Some(Tag::String("S".into())),
                number_format: Some(NumberFormat::Styled(Tag::compound([(
                    "color",
                    Tag::String("red".into()),
                )]))),
            }),
            Clientbound::SetObjective(SetObjective {
                name: "kills".into(),
                method: 0,
                display: Some((Tag::String("Kills".into()), 1, Some(NumberFormat::Blank))),
            }),
            Clientbound::SetObjective(SetObjective {
                name: "kills".into(),
                method: 1,
                display: None,
            }),
            Clientbound::SetPlayerTeam(SetPlayerTeam {
                name: "red".into(),
                method: 0,
                parameters: Some(TeamParameters {
                    display_name: Tag::String("Red".into()),
                    prefix: Tag::String("[R] ".into()),
                    suffix: Tag::String("".into()),
                    name_tag_visibility: 0,
                    collision_rule: 0,
                    color: Some(12),
                    options: 3,
                }),
                players: vec!["Steve".into(), "Alex".into()],
            }),
            Clientbound::SetPlayerTeam(SetPlayerTeam {
                name: "red".into(),
                method: 4,
                parameters: None,
                players: vec!["Alex".into()],
            }),
            Clientbound::AddEntity(AddEntity {
                id: 5,
                uuid: 9,
                kind: 100,
                position: [1.5, 64.0, -3.25],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 64,
                head_yaw: 64,
                data: 0,
            }),
            Clientbound::BlockUpdate(BlockUpdate {
                pos: [-5, -60, 7],
                state: 1,
            }),
            Clientbound::ContainerSetContent(ContainerSetContent {
                container: 0,
                state: 3,
                items: vec![None, Some(Stack::new(1, 64))],
                carried: None,
            }),
            Clientbound::ContainerSetSlot(ContainerSetSlot {
                container: 0,
                state: 4,
                slot: 36,
                item: Some(Stack::new(2, 1)),
            }),
            Clientbound::Disconnect(Disconnect {
                reason: literal("kicked"),
            }),
            Clientbound::EntityPositionSync(EntityPositionSync {
                id: 1,
                path: PositionPath::Stepped(vec![([1.0, 2.0, 3.0], 0), ([1.5, 2.0, 3.0], 2)]),
                yaw: 1.0,
                pitch: 2.0,
                on_ground: true,
            }),
            Clientbound::ForgetLevelChunk(ForgetLevelChunk { pos: [-1, 2] }),
            Clientbound::GameEvent(GameEvent {
                event: 13,
                param: 0.0,
            }),
            Clientbound::LevelChunkWithLight(LevelChunkWithLight {
                x: 3,
                z: -4,
                chunk: ChunkData {
                    heightmaps: vec![(4, vec![0; 37])],
                    buffer: vec![0, 0, 0, 0, 0, 0, 0, 0],
                    block_entities: vec![BlockEntityInfo {
                        packed_xz: 1,
                        y: 70,
                        kind: 3,
                        tag: Tag::End,
                    }],
                },
                light: LightData {
                    sky_mask: vec![1],
                    block_mask: vec![],
                    empty_sky_mask: vec![],
                    empty_block_mask: vec![2],
                    sky: vec![vec![0xFF; 2048]],
                    block: vec![],
                },
            }),
            Clientbound::Login(Login {
                player_id: 42,
                hardcore: false,
                levels: vec!["minecraft:overworld".into(), "minecraft:the_nether".into()],
                max_players: 20,
                chunk_radius: 10,
                simulation_distance: 10,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: spawn(),
                online_mode: false,
                enforces_secure_chat: false,
            }),
            Clientbound::MoveEntityPos(MoveEntityPos(MoveEntity {
                id: 3,
                delta: Some(VecDelta::Linear([4096, 0, -4096])),
                rotation: None,
                on_ground: true,
            })),
            Clientbound::MoveEntityPosRot(MoveEntityPosRot(MoveEntity {
                id: 3,
                delta: Some(VecDelta::Stepped(vec![([1, 2, 3], 1), ([0, 0, 1], 2)])),
                rotation: Some((10, -10)),
                on_ground: false,
            })),
            Clientbound::MoveEntityRot(MoveEntityRot(MoveEntity {
                id: 3,
                delta: None,
                rotation: Some((1, 2)),
                on_ground: true,
            })),
            Clientbound::OpenScreen(OpenScreen {
                container: 1,
                menu: 2,
                title: literal("Chest"),
            }),
            Clientbound::MountScreenOpen(MountScreenOpen {
                container: 4,
                columns: 5,
                entity: 300,
            }),
            Clientbound::MerchantOffers(MerchantOffers {
                container: 3,
                offers: vec![NetOffer {
                    cost_a: Stack::new(5, 24).into(),
                    result: Stack::new(7, 1),
                    cost_b: Some(Stack::new(9, 1).into()),
                    out_of_stock: false,
                    uses: 2,
                    max_uses: 16,
                    xp: 2,
                    special_price: -3,
                    price_multiplier: 0.05,
                    demand: 4,
                }],
                level: 2,
                xp: 12,
                show_progress: true,
                can_restock: true,
            }),
            Clientbound::PlayerChat(PlayerChat {
                global_index: 0,
                sender: 5,
                index: 0,
                signature: None,
                content: "hello".into(),
                timestamp: 1,
                salt: 2,
                last_seen: vec![(3, None), (-1, Some(vec![7; 256]))],
                unsigned_content: None,
                filter: (0, vec![]),
                chat_type: ChatTypeBound {
                    chat_type: 0,
                    name: literal("Steve"),
                    target: None,
                },
            }),
            Clientbound::PlayerInfoUpdate(PlayerInfoUpdate {
                actions: 0b0001_1101,
                entries: vec![PlayerInfo {
                    uuid: 1,
                    name: Some("Steve".into()),
                    game_mode: Some(1),
                    listed: Some(true),
                    latency: Some(12),
                    ..PlayerInfo::default()
                }],
            }),
            Clientbound::PlayerPosition(PlayerPosition {
                teleport: 1,
                position: [0.5, 70.0, 0.5],
                delta: [0.0; 3],
                yaw: 90.0,
                pitch: 0.0,
                relatives: relative::DELTA_X,
            }),
            Clientbound::Respawn(Respawn {
                spawn: spawn(),
                keep: 3,
            }),
            Clientbound::SectionBlocksUpdate(SectionBlocksUpdate {
                section: [1, -4, 2],
                changes: vec![(9 << 12) | 0x123],
            }),
            Clientbound::SetEntityData(SetEntityData {
                id: 2,
                values: vec![(8, 7, Value::Item(Some(Stack::new(5, 3))))],
            }),
            Clientbound::SetEntityMotion(SetEntityMotion {
                id: 2,
                velocity: [0.0; 3],
            }),
            Clientbound::SetPassengers(SetPassengers {
                vehicle: 2,
                passengers: vec![3, 300],
            }),
            Clientbound::SetEntityLink(SetEntityLink { source: 2, dest: 0 }),
            Clientbound::LevelParticles(LevelParticles {
                particle: Particle {
                    id: 21,
                    data: Some(crate::entity_data::ParticleData::ColorScale(0xFF0000, 1.5)),
                },
                override_limiter: true,
                always_show: false,
                pos: [0.5, 101.0, 3.0],
                dist: [0.2, 0.0, 0.2],
                max_speed: [0.0; 3],
                count: 60,
                randomization: 0,
            }),
            Clientbound::Explode(Explode {
                center: [1.0, 2.0, 3.0],
                radius: 4.0,
                block_count: 12,
                knockback: Some([0.5, 0.1, 0.0]),
                particle: Particle { id: 29, data: None },
                sound: (-1, Some(("minecraft:entity.generic.explode".into(), None))),
                block_particles: vec![ExplosionParticle {
                    particle: Particle { id: 69, data: None },
                    scaling: 0.5,
                    speed: 1.0,
                    weight: 1,
                }],
                play_sound: true,
            }),
            Clientbound::ProjectilePower(ProjectilePower { id: 4, power: 0.1 }),
            Clientbound::SetEquipment(SetEquipment {
                id: 2,
                slots: vec![(0, None), (5, Some(Stack::new(1, 1)))],
            }),
            Clientbound::UpdateAttributes(UpdateAttributes {
                id: 2,
                values: vec![AttributeSnapshot {
                    attribute: 26,
                    base: 0.23,
                    modifiers: vec![AttributeModifier {
                        id: "minecraft:baby".into(),
                        amount: 0.5,
                        operation: 1,
                    }],
                }],
            }),
            Clientbound::DamageEvent(DamageEvent {
                id: 7,
                source_type: 31,
                cause: Some(0),
                direct: None,
                position: Some([1.5, -2.0, 3.25]),
            }),
            Clientbound::SetHealth(SetHealth {
                health: 17.5,
                food: 18,
                saturation: 3.0,
            }),
            Clientbound::SetTime(SetTime {
                game_time: 100,
                clocks: vec![ClockState {
                    clock: 0,
                    total_ticks: 6000,
                    partial_tick: 0.0,
                    rate: 1.0,
                }],
            }),
            Clientbound::Sound(Sound {
                sound: (12, None),
                source: 4,
                x: 8,
                y: 9,
                z: 10,
                volume: 1.0,
                pitch: 1.0,
                seed: 3,
            }),
            Clientbound::Sound(Sound {
                sound: (-1, Some(("minecraft:x".into(), Some(16.0)))),
                source: 0,
                x: 0,
                y: 0,
                z: 0,
                volume: 1.0,
                pitch: 1.0,
                seed: 0,
            }),
            Clientbound::SystemChat(SystemChat {
                content: literal("hi"),
                overlay: false,
            }),
            Clientbound::TeleportEntity(TeleportEntity {
                id: 1,
                position: [1.0; 3],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: 0,
                on_ground: true,
            }),
            Clientbound::TickingState(TickingState {
                rate: 40.0,
                frozen: true,
            }),
            Clientbound::TickingStep(TickingStep { steps: 7 }),
            Clientbound::AwardStats(AwardStats {
                stats: vec![(8, 3, 120), (0, 1, 2)],
            }),
            Clientbound::UpdateAdvancements(UpdateAdvancements {
                reset: true,
                added: vec![Advancement {
                    id: "minecraft:story/root".into(),
                    parent: None,
                    display: Some(AdvancementDisplay {
                        title: literal("Minecraft"),
                        description: literal("The heart and story of the game"),
                        icon: crate::item::Stack::new(42, 1),
                        frame: 0,
                        show_toast: false,
                        hidden: false,
                        background: Some("minecraft:block/stone".into()),
                    }),
                    requirements: vec![vec!["crafting_table".into()]],
                    telemetry: false,
                    x: 0.0,
                    y: 1.5,
                }],
                removed: vec!["minecraft:story/gone".into()],
                progress: vec![(
                    "minecraft:story/root".into(),
                    vec![("crafting_table".into(), Some(1_792_216_800_000))],
                )],
                show: true,
            }),
        ];
        for p in packets {
            round_trip(p, Clientbound::encode, Clientbound::decode);
        }
    }

    #[test]
    fn serverbound_round_trips() {
        let packets = vec![
            Serverbound::PingRequest(crate::packets::status::Ping { time: -7 }),
            Serverbound::AcceptTeleportation(AcceptTeleportation {
                id: 1,
                x: 0.5,
                y: 64.0,
                z: 0.5,
                yaw: 0.0,
                pitch: 0.0,
            }),
            Serverbound::Chat(Chat {
                message: "hello".into(),
                timestamp: 1,
                salt: 0,
                signature: None,
                last_seen: LastSeenUpdate {
                    offset: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                },
            }),
            Serverbound::ChatCommand(ChatCommand {
                command: "time set day".into(),
            }),
            Serverbound::ChatAck(ChatAck { offset: 3 }),
            Serverbound::ChatCommandSigned(ChatCommandSigned {
                command: "say hi".into(),
                timestamp: 1_700_000_000_000,
                salt: -5,
                arguments: vec![("message".into(), vec![9; 256])],
                last_seen: LastSeenUpdate {
                    offset: 1,
                    acknowledged: [1, 0, 8],
                    checksum: -3,
                },
            }),
            Serverbound::ChatSessionUpdate(ChatSessionUpdate {
                session: 0x1234,
                expires_at: 1_800_000_000_000,
                key: vec![0x30; 294],
                key_signature: vec![7; 512],
            }),
            Serverbound::ChunkBatchReceived(ChunkBatchReceived {
                chunks_per_tick: 9.5,
            }),
            Serverbound::ClientCommand(ClientCommand { action: 0 }),
            Serverbound::ClientTickEnd(Empty),
            Serverbound::ContainerClick(ContainerClick {
                container: 0,
                state: 5,
                slot: 36,
                button: 0,
                input: 0,
                changed: vec![(36, None)],
                carried: Some(Hashed::of(&Stack::new(1, 64))),
            }),
            Serverbound::MovePlayerPosRot(MovePlayerPosRot {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                yaw: 4.0,
                pitch: 5.0,
                flags: 1,
            }),
            Serverbound::MovePlayerStatus(MovePlayerStatus { flags: 3 }),
            Serverbound::PlayerAction(PlayerAction {
                action: 0,
                pos: [1, 2, 3],
                direction: 1,
                sequence: 7,
            }),
            Serverbound::PlayerInput(PlayerInput { flags: 0b0101_0001 }),
            Serverbound::PlayerLoaded(Empty),
            Serverbound::Punch(Empty),
            Serverbound::SetCarriedItem(SetCarriedItem { slot: 4 }),
            Serverbound::UseItemOn(UseItemOn {
                hand: 0,
                pos: [1, 64, 1],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence: 8,
            }),
            Serverbound::Interact(Interact {
                id: 9,
                hand: 0,
                location: [0.25, 1.0, 0.0],
                secondary: false,
            }),
        ];
        for p in packets {
            if let Serverbound::Interact(_) = &p {
                // LpVec3 is lossy; check bytes instead.
                let body = p.encode();
                let mut r = Reader::new(&body);
                let id = r.varint().unwrap();
                assert!(matches!(
                    Serverbound::decode(id, r.rest()).unwrap(),
                    Serverbound::Interact(Interact { id: 9, .. })
                ));
                continue;
            }
            round_trip(p, Serverbound::encode, Serverbound::decode);
        }
        round_trip(
            Serverbound::SelectTrade(SelectTrade { item: 3 }),
            Serverbound::encode,
            Serverbound::decode,
        );
        // CONTAINER_ID and the button, both VarInts.
        let button = Serverbound::ContainerButtonClick(ContainerButtonClick {
            container: 3,
            button: 200,
        });
        assert_eq!(&button.encode()[1..], &[3, 0xC8, 1]);
        round_trip(button, Serverbound::encode, Serverbound::decode);
        // The name, a UTF-8 string with its VarInt length.
        let rename = Serverbound::RenameItem(RenameItem {
            name: "Blade".into(),
        });
        assert_eq!(&rename.encode()[1..], &[5, b'B', b'l', b'a', b'd', b'e']);
        round_trip(rename, Serverbound::encode, Serverbound::decode);
        // The slot, the container id (VarInts), then the new state.
        let toggle = Serverbound::ContainerSlotStateChanged(ContainerSlotStateChanged {
            slot: 4,
            container: 2,
            enabled: false,
        });
        assert_eq!(&toggle.encode()[1..], &[4, 2, 0]);
        round_trip(toggle, Serverbound::encode, Serverbound::decode);
        // Two optional mob effect ids: a presence byte, then the VarInt.
        let beacon = Serverbound::SetBeacon(SetBeacon {
            primary: Some(2),
            secondary: None,
        });
        assert_eq!(&beacon.encode()[1..], &[1, 2, 0]);
        round_trip(beacon, Serverbound::encode, Serverbound::decode);
    }

    #[test]
    fn a_cooldown_packet_carries_its_group_and_ticks() {
        let packet = Clientbound::Cooldown(Cooldown {
            group: "minecraft:ender_pearl".into(),
            duration: 20,
        });
        round_trip(packet, Clientbound::encode, Clientbound::decode);
        // The group is an identifier, the ticks a VarInt; a duration of 0
        // removes the entry.
        let body = Clientbound::encode(&Clientbound::Cooldown(Cooldown {
            group: "minecraft:ender_pearl".into(),
            duration: 0,
        }));
        assert_eq!(body[0], crate::ids::play::clientbound::COOLDOWN as u8);
        assert_eq!(&body[1..3], [21, b'm']);
        assert_eq!(body[body.len() - 1], 0);
    }

    #[test]
    fn change_game_mode_uses_the_game_type_varint() {
        for mode in 0..=3 {
            let packet = Serverbound::ChangeGameMode(ChangeGameMode { mode });
            assert_eq!(packet.encode(), vec![5, mode as u8]);
            round_trip(packet, Serverbound::encode, Serverbound::decode);
        }
    }

    #[test]
    fn move_entity_deltas_decode_like_vec_delta_codec() {
        let base = [10.0, 64.0, -3.5];
        assert_eq!(
            VecDelta::Linear([4096, 0, -2048]).end(base),
            [11.0, 64.0, -4.0]
        );
        // Zero deltas keep unquantized base coordinates exactly.
        assert_eq!(
            VecDelta::Linear([0, 0, 0]).end([0.123_456_789, 1.0, 2.0]),
            [0.123_456_789, 1.0, 2.0]
        );
        assert_eq!(
            VecDelta::Stepped(vec![([4096, 0, 0], 1), ([4096, 0, 0], 2)]).end(base),
            [12.0, 64.0, -3.5]
        );
    }

    #[test]
    fn player_look_at_round_trips() {
        let at_point = PlayerLookAt {
            from: 1,
            target: [1.5, 64.0, -2.25],
            entity: None,
        };
        let at_entity = PlayerLookAt {
            from: 0,
            target: [0.0, 70.0, 0.0],
            entity: Some(LookAtEntity { id: 42, anchor: 1 }),
        };
        for p in [at_point, at_entity] {
            let bytes = Clientbound::PlayerLookAt(p.clone()).encode();
            // 1 id byte, 1 anchor, 3 doubles, the flag; the entity adds 2 VarInts.
            let len = 1 + 1 + 24 + 1 + if p.entity.is_some() { 2 } else { 0 };
            assert_eq!(bytes.len(), len);
            assert_eq!(
                Clientbound::decode(crate::ids::play::clientbound::PLAYER_LOOK_AT, &bytes[1..])
                    .unwrap(),
                Clientbound::PlayerLookAt(p)
            );
        }
    }

    #[test]
    fn unknown_packets_keep_their_payload() {
        let p =
            Clientbound::decode(crate::ids::play::clientbound::DEBUG_SAMPLE, &[1, 2, 3]).unwrap();
        assert_eq!(
            p,
            Clientbound::Unknown(crate::ids::play::clientbound::DEBUG_SAMPLE, vec![1, 2, 3])
        );
        assert_eq!(p.encode()[1..], [1, 2, 3]);
    }

    #[test]
    fn update_attributes_encode_like_vanilla() {
        let p = Clientbound::UpdateAttributes(UpdateAttributes {
            id: 5,
            values: vec![AttributeSnapshot {
                attribute: 1,
                base: 0.0,
                modifiers: vec![AttributeModifier {
                    id: "minecraft:armor.helmet".into(),
                    amount: 3.0,
                    operation: 0,
                }],
            }],
        });
        let body = p.encode();
        // The packet id (134) is a two-byte VarInt.
        let mut expected = vec![134, 1, 5, 1, 1];
        expected.extend(0.0f64.to_be_bytes());
        expected.extend([1, 22]);
        expected.extend(b"minecraft:armor.helmet");
        expected.extend(3.0f64.to_be_bytes());
        expected.push(0);
        assert_eq!(body, expected);
        assert_eq!(crate::ids::play::clientbound::UPDATE_ATTRIBUTES, 134);
        assert_eq!(Clientbound::decode(134, &body[2..]).unwrap(), p);
    }

    #[test]
    fn map_item_data_carries_a_server_patch() {
        let patch = MapPatch {
            start_x: 3,
            start_y: 4,
            width: 2,
            height: 1,
            colors: vec![1, 2],
        };
        let mut w = Writer::new();
        MapPatchOpt::write(&Some(patch.clone()), &mut w);
        assert_eq!(w.data, [2, 1, 3, 4, 2, 1, 2]);
        let p = Clientbound::MapItemData(MapItemData {
            map_id: 2146483647,
            scale: 1,
            locked: true,
            decorations: None,
            patch: Some(patch),
        });
        round_trip(p, Clientbound::encode, Clientbound::decode);
    }

    #[test]
    fn sign_packets_match_vanilla_stream_codecs() {
        // ClientboundOpenSignEditorPacket: BlockPos, then SignTextSlot as a VarInt (FRONT is 1).
        let open = Clientbound::OpenSignEditor(OpenSignEditor {
            pos: [1, 2, 3],
            front: true,
        });
        let body = open.encode();
        assert_eq!(
            body[1..9],
            crate::buf::pack_block_pos([1, 2, 3]).to_be_bytes()
        );
        assert_eq!(body[9..], [1]);
        round_trip(open, Clientbound::encode, Clientbound::decode);
        // ServerboundSignUpdatePacket: BlockPos, four strings with no count, then the slot.
        let update = Serverbound::SignUpdate(SignUpdate {
            pos: [-4, 70, 9],
            lines: ["Hi".into(), String::new(), "x".into(), String::new()],
            front: false,
        });
        let body = update.encode();
        assert_eq!(body[9..], [2, b'H', b'i', 0, 1, b'x', 0, 0]);
        round_trip(update, Serverbound::encode, Serverbound::decode);
    }

    #[test]
    fn book_packets_match_vanilla_stream_codecs() {
        // ClientboundOpenBookPacket: InteractionHand as a VarInt.
        let open = Clientbound::OpenBook(OpenBook { hand: 1 });
        assert_eq!(open.encode()[1..], [1]);
        round_trip(open, Clientbound::encode, Clientbound::decode);
        // ServerboundEditBookPacket: the slot, the pages (a counted list), then an optional title.
        let sign = Serverbound::EditBook(EditBook {
            slot: 40,
            pages: vec!["a".into(), String::new()],
            title: Some("T".into()),
        });
        assert_eq!(sign.encode()[1..], [40, 2, 1, b'a', 0, 1, 1, b'T']);
        round_trip(sign, Serverbound::encode, Serverbound::decode);
        let mut w = Writer::new();
        w.varint(101);
        assert!(BookPages::read(&mut Reader::new(&w.data)).is_err());
    }

    #[test]
    fn boss_event_round_trips() {
        let add = Clientbound::BossEvent(BossEvent {
            id: 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210,
            action: BossAction::Add {
                name: Tag::String("Wither".into()),
                progress: 0.5,
                color: 5,
                overlay: 0,
                darken: true,
                music: false,
                fog: true,
            },
        });
        let body = add.encode();
        // Packet id 9, then the uuid, operation 0, and the property byte
        // (darken 1, fog 4).
        assert_eq!(body[0], 9);
        assert_eq!(*body.last().unwrap(), 5);
        round_trip(add, Clientbound::encode, Clientbound::decode);
        round_trip(
            Clientbound::BossEvent(BossEvent {
                id: 7,
                action: BossAction::UpdateProgress { progress: 0.25 },
            }),
            Clientbound::encode,
            Clientbound::decode,
        );
        round_trip(
            Clientbound::BossEvent(BossEvent {
                id: 7,
                action: BossAction::Remove,
            }),
            Clientbound::encode,
            Clientbound::decode,
        );
    }

    /// A registry entry is its id + 1, the dialog itself a 0 then its NBT.
    #[test]
    fn dialogs_round_trip() {
        let notice = Tag::compound([("type", Tag::String("minecraft:notice".into()))]);
        for dialog in [DialogHolder::Registry(3), DialogHolder::Direct(notice)] {
            crate::packets::round_trip(
                Clientbound::ShowDialog(ShowDialog { dialog }),
                Clientbound::encode,
                Clientbound::decode,
            );
        }
        let mut w = Writer::new();
        DialogHolder::write(&DialogHolder::Registry(3), &mut w);
        assert_eq!(w.data, [4]);
        crate::packets::round_trip(
            Serverbound::CustomClickAction(CustomClickAction {
                id: "minecraft:custom".into(),
                payload: None,
            }),
            Serverbound::encode,
            Serverbound::decode,
        );
    }
}
