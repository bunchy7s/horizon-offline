//! Maps (`EmptyMapItem`, `MapItem`, `MapItemSavedData`, `MapRenderer`):
//! creating a map centred on the player's grid cell, revealing it while held
//! (`MapItem.update`: dominant map colour per pixel, height and water-depth
//! shading, the dithered edge), the player marker, `data/minecraft/maps/<id>.dat` and
//! `maps/last_id.dat`, the 128x128 texture in an atlas slot, the two-handed
//! first-person view and maps in item frames.
use super::{Game, lan::LanHost};
use horizon_client::{
    blocks::{Fluid, MAP_COLORS},
    hud::{MAP_SLOTS, Rect},
    items::ItemStack,
    mesher::Vertex,
};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

/// What a decoration is filed under in `MapItemSavedData.decorations` (a
/// `LinkedHashMap<String, MapDecoration>`): a player's marker by their
/// connection, the banner at a position (`MapBanner.getId`), a framed map's
/// frame by its entity (`getFrameKey`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Key {
    Player(u32),
    /// An item's `minecraft:map_decorations` (an explorer map's target), or a
    /// decoration a server sent (`addClientSideDecorations`).
    Item,
    Banner([i32; 3]),
    Frame(i32),
}
/// A decoration (`MapDecoration`): sprite, map position (-128..127, half
/// pixels), rotation (0..15) and name, filed under its key.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Decoration {
    pub sprite: &'static str,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
    pub name: Option<String>,
    pub key: Key,
}
/// A map as a native mod sees it when `native_map_decorations` is asked.
// Native mods read it; nothing in Horizon does.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct NativeMapInfo<'a> {
    /// The map's id (`map_<id>`).
    pub id: i32,
    /// The map's centre block `[x, z]`.
    pub center: [i32; 2],
    /// `MapItemSavedData.scale`: a map pixel is `1 << scale` blocks.
    pub scale: u8,
    /// The dimension the map shows (`minecraft:overworld`).
    pub dimension: &'a str,
    pub locked: bool,
    /// Whether it is drawn in an item frame, not held or on the cartography screen.
    pub in_frame: bool,
}
/// An icon a native mod puts on a map: the same place and pose as vanilla's
/// decorations (`MapDecoration`).
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMapDecoration {
    /// An atlas sprite: vanilla's `map/decorations/<name>`, or a content
    /// mod's texture. An unknown sprite is left out.
    pub sprite: String,
    /// The position on the map in half map pixels, -128 (left, top) to 127.
    /// For the world position `(wx, wz)`: `(wx - center_x) / (1 << scale) * 2`,
    /// and the same for z, floored and clamped.
    pub x: i8,
    pub y: i8,
    /// 0 to 15, in 22.5 degree steps clockwise (only the low four bits count).
    pub rot: u8,
    /// Whether it shows in an item frame and on the cartography screen
    /// (`MapDecorationType.showOnItemFrame`); a held map shows all.
    pub shown_on_frame: bool,
}

/// `MapBanner`: a banner a map marks, as `banners` saves it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MapBanner {
    pub pos: [i32; 3],
    /// `DyeColor.getName` of the banner's base colour.
    pub color: String,
    pub name: Option<String>,
}
impl MapBanner {
    /// `MapBanner.getDecoration`: the marker of the banner's colour.
    fn sprite(&self) -> &'static str {
        decoration_sprite(&format!("banner_{}", self.color)).unwrap_or(BANNER_WHITE)
    }
    /// `MapBanner.CODEC`.
    fn to_nbt(&self) -> Tag {
        let mut entries = vec![
            ("pos", Tag::IntArray(self.pos.to_vec())),
            ("color", Tag::String(self.color.as_str().into())),
        ];
        if let Some(name) = &self.name {
            entries.push(("name", Tag::String(name.as_str().into())));
        }
        Tag::compound(entries)
    }
    fn from_nbt(tag: &Tag) -> Option<Self> {
        // DyeColor.CODEC is read leniently: white when missing or unknown.
        let color = horizon_client::save::text(tag.get("color"))
            .filter(|c| decoration_sprite(&format!("banner_{c}")).is_some())
            .unwrap_or_else(|| "white".to_owned());
        Some(Self {
            pos: block_pos(tag.get("pos"))?,
            color,
            name: horizon_client::save::text(tag.get("name")),
        })
    }
}
/// `MapFrame`: an item frame a map hangs in, as `frames` saves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MapFrame {
    pub pos: [i32; 3],
    /// The frame's `Direction.get2DDataValue` times 90.
    pub rotation: i32,
    pub entity_id: i32,
}
impl MapFrame {
    /// `MapFrame.CODEC`.
    fn to_nbt(self) -> Tag {
        Tag::compound([
            ("pos", Tag::IntArray(self.pos.to_vec())),
            ("rotation", Tag::Int(self.rotation)),
            ("entity_id", Tag::Int(self.entity_id)),
        ])
    }
    fn from_nbt(tag: &Tag) -> Option<Self> {
        let int = |k: &str| horizon_client::save::number(tag.get(k)).map(|v| v as i32);
        Some(Self {
            pos: block_pos(tag.get("pos"))?,
            rotation: int("rotation")?,
            entity_id: int("entity_id")?,
        })
    }
}
/// `BlockPos.CODEC`: three ints, as an int array or a list.
fn block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        Tag::List(l) if l.len() == 3 => {
            let int = |t: &Tag| match t {
                Tag::Int(v) => Some(*v),
                _ => None,
            };
            Some([int(&l[0])?, int(&l[1])?, int(&l[2])?])
        }
        _ => None,
    }
}
/// The item frame a map hangs in, as `tickCarriedBy` reads it
/// (`placedInFrame`): the block it is in, its `Direction.get3DDataValue`
/// (down 0, up 1, north 2, south 3, west 4, east 5) and its entity id.
#[derive(Clone, Copy)]
struct Framed {
    pos: [i32; 3],
    facing: u8,
    id: i32,
}
/// `Direction.get2DDataValue` of a 3D data value: south 0, west 1, north 2,
/// east 3, and -1 for up and down.
fn direction_2d(facing: u8) -> i32 {
    match facing {
        3 => 0,
        4 => 1,
        2 => 2,
        5 => 3,
        _ => -1,
    }
}
/// The host among a map's holders (the guests are their connections).
const HOST: u32 = u32::MAX;
/// `MapDecorationTypes` the markers use: the player's arrow, the arrows that
/// replace it past the map's edge, the item frame's, the white banner's (the
/// fallback colour).
const PLAYER: &str = "map/decorations/player";
const FRAME: &str = "map/decorations/frame";
const BANNER_WHITE: &str = "map/decorations/white_banner";
const PLAYER_OFF_MAP: &str = "map/decorations/player_off_map";
const PLAYER_OFF_LIMITS: &str = "map/decorations/player_off_limits";
/// A player carrying a map, or seeing it framed
/// (`MapItemSavedData.HoldingPlayer`): the column sweep's step, and what
/// changed since the last `ClientboundMapItemDataPacket` it was sent.
struct Holder {
    who: u32,
    /// The last tick it carried the map, and saw it framed.
    carried: i64,
    framed: i64,
    step: i32,
    /// The changed pixels' box (min x, min y, max x, max y).
    dirty: Option<[i32; 4]>,
    decorations: bool,
    tick: u32,
    /// Wore map invisibility gear on its last tick
    /// (`MapItemSavedData.hasMapInvisibilityItemEquipped`).
    invisible: bool,
}
impl Holder {
    fn new(who: u32) -> Self {
        Self {
            who,
            carried: 0,
            framed: 0,
            step: 0,
            dirty: Some([0, 0, 127, 127]),
            decorations: true,
            tick: 0,
            invisible: false,
        }
    }
    /// Still carrying the map, or seeing it framed (frames tick every 10).
    fn holds(&self, now: i64) -> bool {
        self.carried + 1 >= now || self.framed + 10 >= now
    }
}
pub(super) struct MapData {
    pub center: [i32; 2],
    pub scale: u8,
    pub dimension: String,
    pub locked: bool,
    pub tracking: bool,
    pub unlimited: bool,
    pub colors: Vec<u8>,
    pub decorations: Vec<Decoration>,
    /// `bannerMarkers` and `frameMarkers` (`banners`, `frames` when saved).
    pub banners: Vec<MapBanner>,
    frames: Vec<MapFrame>,
    holders: Vec<Holder>,
    /// Changed since the last save / texture upload.
    dirty: bool,
    texture_dirty: bool,
}
impl MapData {
    /// `MapItemSavedData.createFresh`: centred on the scale's grid cell.
    pub(super) fn fresh(x: f64, z: f64, scale: u8, dimension: &str) -> Self {
        let size = 128 * (1 << scale);
        let cell = |v: f64| (v.floor() as i32 + 64).div_euclid(size) * size + size / 2 - 64;
        Self {
            center: [cell(x), cell(z)],
            scale,
            dimension: dimension.to_owned(),
            locked: false,
            tracking: true,
            unlimited: false,
            colors: vec![0; 128 * 128],
            decorations: Vec::new(),
            banners: Vec::new(),
            frames: Vec::new(),
            holders: Vec::new(),
            dirty: true,
            texture_dirty: true,
        }
    }
    fn update_color(&mut self, x: i32, y: i32, color: u8) -> bool {
        let i = (x + y * 128) as usize;
        if self.colors[i] == color {
            return false;
        }
        self.colors[i] = color;
        self.dirty = true;
        self.texture_dirty = true;
        // setColorsDirty: every holder's next packet carries it.
        for h in &mut self.holders {
            h.dirty = Some(match h.dirty {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
        true
    }
    /// `tickCarriedBy`'s holder for `who`, added when new.
    fn holder(&mut self, who: u32) -> &mut Holder {
        let i = match self.holders.iter().position(|h| h.who == who) {
            Some(i) => i,
            None => {
                self.holders.push(Holder::new(who));
                self.holders.len() - 1
            }
        };
        &mut self.holders[i]
    }
    /// The holders that no longer carry the map nor see it framed go, and
    /// their markers with them (`tickCarriedBy`'s removeDecoration).
    fn drop_holders(&mut self, now: i64) {
        self.holders.retain(|h| h.holds(now));
        let holders = &self.holders;
        let before = self.decorations.len();
        self.decorations.retain(|d| match d.key {
            Key::Player(w) => holders.iter().any(|h| h.who == w),
            _ => true,
        });
        if self.decorations.len() != before {
            self.decorations_changed();
        }
    }
    /// setDecorationsDirty.
    fn decorations_changed(&mut self) {
        for h in &mut self.holders {
            h.decorations = true;
        }
    }
    /// `MapItemSavedData.removeDecoration`: whatever `key` names goes, and
    /// every holder is told the decorations changed, found or not.
    fn remove_decoration(&mut self, key: Key) {
        self.decorations.retain(|d| d.key != key);
        self.decorations_changed();
    }
    /// `MapItemSavedData.addDecoration` with `calculateDecorationLocationAndType`
    /// and `calculateRotation`: `sprite` at the world position `x_pos`,
    /// `z_pos` under `key`, in place when the key has a decoration so that the
    /// order (the drawing order) stays. A position off the map leaves it out
    /// (a player's turns into an off-map arrow, then nothing past 320 without
    /// unlimited tracking) and removes what the key had. `game_time` is the
    /// level's: without a level the Nether's spinning rotation is not used.
    #[allow(clippy::too_many_arguments)]
    fn add_decoration(
        &mut self,
        sprite: &'static str,
        game_time: Option<i64>,
        key: Key,
        x_pos: f64,
        z_pos: f64,
        y_rot: f64,
        name: Option<String>,
    ) {
        let scaling = (1 << self.scale) as f32;
        let dx = (x_pos - f64::from(self.center[0])) as f32 / scaling;
        let dz = (z_pos - f64::from(self.center[1])) as f32 / scaling;
        let inside = (-63.0..=63.0).contains(&dx) && (-63.0..=63.0).contains(&dz);
        let shown = if sprite != PLAYER {
            (inside || self.unlimited).then_some(sprite)
        } else if inside {
            Some(sprite)
        } else if dx.abs() < 320.0 && dz.abs() < 320.0 {
            Some(PLAYER_OFF_MAP)
        } else {
            self.unlimited.then_some(PLAYER_OFF_LIMITS)
        };
        let Some(sprite) = shown else {
            self.remove_decoration(key);
            return;
        };
        let spinning = self.dimension == "minecraft:the_nether" && game_time.is_some();
        let decoration = Decoration {
            sprite,
            x: clamp_map_coordinate(dx),
            y: clamp_map_coordinate(dz),
            rot: decoration_rotation(spinning, game_time.unwrap_or(0), y_rot),
            name,
            key,
        };
        match self.decorations.iter().position(|d| d.key == key) {
            Some(i) if self.decorations[i] == decoration => {}
            Some(i) => {
                self.decorations[i] = decoration;
                self.decorations_changed();
            }
            None => {
                self.decorations.push(decoration);
                self.decorations_changed();
            }
        }
    }
    /// `MapItemSavedData.tickCarriedBy`'s check on every holder but `who`, the
    /// player ticking: one wearing map invisibility gear has no marker.
    fn hide_invisible(&mut self, who: u32) {
        let hidden: Vec<u32> = self
            .holders
            .iter()
            .filter(|h| h.who != who && h.invisible)
            .map(|h| h.who)
            .collect();
        for w in hidden {
            self.remove_decoration(Key::Player(w));
        }
    }
    /// `MapItemSavedData.tickCarriedBy` with `placedInFrame`, for `who`, a
    /// player in the level of `frame`, the item frame holding the map.
    /// `carries` is whether their inventory holds the map (`mapMatcher`),
    /// `invisible` whether they wear map invisibility gear. Without a carried
    /// copy the player has no marker; the frame gets its own, which replaces
    /// the marker of another frame entity remembered at the same block.
    fn tick_framed(&mut self, who: u32, now: i64, carries: bool, invisible: bool, frame: Framed) {
        let h = self.holder(who);
        h.framed = now;
        h.invisible = invisible;
        if !carries {
            self.remove_decoration(Key::Player(who));
        }
        self.hide_invisible(who);
        if !self.tracking {
            return;
        }
        let pos = frame.pos;
        if let Some(old) = self.frames.iter().find(|f| f.pos == pos).copied()
            && old.entity_id != frame.id
        {
            self.remove_decoration(Key::Frame(old.entity_id));
        }
        let rotation = direction_2d(frame.facing) * 90;
        let (x, z) = (f64::from(pos[0]), f64::from(pos[2]));
        let key = Key::Frame(frame.id);
        self.add_decoration(FRAME, Some(now), key, x, z, f64::from(rotation), None);
        let marker = MapFrame {
            pos,
            rotation,
            entity_id: frame.id,
        };
        match self.frames.iter().position(|f| f.pos == pos) {
            Some(i) if self.frames[i] == marker => {}
            Some(i) => {
                self.frames[i] = marker;
                self.dirty = true;
            }
            None => {
                self.frames.push(marker);
                self.dirty = true;
            }
        }
    }
    /// `MapItemSavedData.removedFromFrame`: the frame entity's marker goes,
    /// and the frame remembered at its block.
    fn removed_from_frame(&mut self, pos: [i32; 3], entity_id: i32) {
        self.remove_decoration(Key::Frame(entity_id));
        self.frames.retain(|f| f.pos != pos);
        self.dirty = true;
    }
    /// `MapItemSavedData.isTrackedCountOverLimit`: more than `limit`
    /// decorations count (`MapDecorationType.trackCount`).
    fn tracked_over_limit(&self, limit: usize) -> bool {
        self.decorations
            .iter()
            .filter(|d| tracks_count(d.sprite))
            .count()
            > limit
    }
    /// `MapItemSavedData.toggleBanner`: a banner inside the map that has no
    /// marker gets one (unless the map tracks too many), one whose marker is
    /// there as it stands loses it, and a changed banner's marker is renewed.
    /// `current` is the banner at `pos` (`MapBanner.fromWorld`); true when
    /// the markers changed.
    fn toggle_banner(&mut self, pos: [i32; 3], current: Option<MapBanner>, game_time: i64) -> bool {
        let (x_pos, z_pos) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
        let scale = f64::from(1 << self.scale);
        let xd = (x_pos - f64::from(self.center[0])) / scale;
        let yd = (z_pos - f64::from(self.center[1])) / scale;
        if !(xd >= -63.0 && yd >= -63.0 && xd <= 63.0 && yd <= 63.0) {
            return false;
        }
        let Some(banner) = current else {
            return false;
        };
        if let Some(i) = self.banners.iter().position(|b| *b == banner) {
            self.banners.remove(i);
            self.remove_decoration(Key::Banner(pos));
            self.dirty = true;
            return true;
        }
        if self.tracked_over_limit(256) {
            return false;
        }
        self.banners.retain(|b| b.pos != pos);
        self.banners.push(banner.clone());
        let sprite = banner.sprite();
        self.add_decoration(
            sprite,
            Some(game_time),
            Key::Banner(pos),
            x_pos,
            z_pos,
            180.0,
            banner.name,
        );
        self.dirty = true;
        true
    }
    /// `MapItemSavedData.checkBanners`: a banner marker in the column `x`,
    /// `z` whose banner is gone or changed (`current` reads
    /// `MapBanner.fromWorld` at a position) is dropped with its decoration.
    fn check_banners(&mut self, x: i32, z: i32, current: impl Fn([i32; 3]) -> Option<MapBanner>) {
        let stale: Vec<MapBanner> = self
            .banners
            .iter()
            .filter(|b| b.pos[0] == x && b.pos[2] == z && current(b.pos).as_ref() != Some(*b))
            .cloned()
            .collect();
        for banner in stale {
            self.banners.retain(|b| *b != banner);
            self.remove_decoration(Key::Banner(banner.pos));
            self.dirty = true;
        }
    }
    /// `HoldingPlayer.nextUpdatePacket` for `who`: the changed pixels, and
    /// the decorations (at most every 5th call while they changed).
    fn update_packet(
        &mut self,
        id: i32,
        who: u32,
        kind: impl Fn(&str) -> Option<i32>,
    ) -> Option<horizon_protocol::packets::play::MapItemData> {
        use horizon_protocol::packets::play::{MapDecoration, MapItemData, MapPatch};
        let h = self.holders.iter_mut().find(|h| h.who == who)?;
        let patch = h.dirty.take().map(|[x0, y0, x1, y1]| {
            let (width, height) = (x1 + 1 - x0, y1 + 1 - y0);
            let mut colors = Vec::with_capacity((width * height) as usize);
            for y in 0..height {
                for x in 0..width {
                    colors.push(self.colors[(x0 + x + (y0 + y) * 128) as usize]);
                }
            }
            MapPatch {
                start_x: x0,
                start_y: y0,
                width,
                height,
                colors,
            }
        });
        let decorations = if h.decorations && {
            let due = h.tick % 5 == 0;
            h.tick += 1;
            due
        } {
            h.decorations = false;
            Some(
                self.decorations
                    .iter()
                    .filter_map(|d| {
                        Some(MapDecoration {
                            kind: kind(decoration_type(d.sprite)?)?,
                            x: d.x,
                            y: d.y,
                            rot: d.rot as i8,
                            name: d.name.as_ref().map(|n| Tag::String(n.as_str().into())),
                        })
                    })
                    .collect(),
            )
        } else {
            None
        };
        (patch.is_some() || decorations.is_some()).then_some(MapItemData {
            map_id: id,
            scale: self.scale as i8,
            locked: self.locked,
            decorations,
            patch,
        })
    }
    fn to_nbt(&self) -> Tag {
        let flag = |b: bool| Tag::Byte(i8::from(b));
        Tag::compound([
            (
                "data",
                Tag::compound([
                    ("dimension", Tag::String(self.dimension.as_str().into())),
                    ("xCenter", Tag::Int(self.center[0])),
                    ("zCenter", Tag::Int(self.center[1])),
                    ("scale", Tag::Byte(self.scale as i8)),
                    ("locked", flag(self.locked)),
                    ("trackingPosition", flag(self.tracking)),
                    ("unlimitedTracking", flag(self.unlimited)),
                    ("colors", Tag::ByteArray(self.colors.clone())),
                    (
                        "banners",
                        Tag::List(self.banners.iter().map(MapBanner::to_nbt).collect()),
                    ),
                    (
                        "frames",
                        Tag::List(self.frames.iter().copied().map(MapFrame::to_nbt).collect()),
                    ),
                ]),
            ),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ])
    }
    fn from_nbt(tag: &Tag) -> Option<Self> {
        let d = tag.get("data")?;
        let int = |k: &str| match d.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let dimension = match d.get("dimension") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => "minecraft:overworld".to_owned(),
        };
        let mut colors = match d.get("colors") {
            Some(Tag::ByteArray(c)) => c.clone(),
            _ => Vec::new(),
        };
        colors.resize(128 * 128, 0);
        let list = |k: &str| match d.get(k) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        let mut map = Self {
            center: [int("xCenter")?, int("zCenter")?],
            scale: int("scale").unwrap_or(0).clamp(0, 4) as u8,
            dimension,
            locked: int("locked").unwrap_or(0) != 0,
            tracking: int("trackingPosition").unwrap_or(1) != 0,
            unlimited: int("unlimitedTracking").unwrap_or(0) != 0,
            colors,
            decorations: Vec::new(),
            banners: list("banners")
                .iter()
                .filter_map(MapBanner::from_nbt)
                .collect(),
            frames: list("frames")
                .iter()
                .filter_map(MapFrame::from_nbt)
                .collect(),
            holders: Vec::new(),
            dirty: false,
            texture_dirty: true,
        };
        // The constructor adds each marker without a level: no Nether spin.
        for b in map.banners.clone() {
            let (x, z) = (f64::from(b.pos[0]), f64::from(b.pos[2]));
            map.add_decoration(b.sprite(), None, Key::Banner(b.pos), x, z, 180.0, b.name);
        }
        for f in map.frames.clone() {
            let (x, z) = (f64::from(f.pos[0]), f64::from(f.pos[2]));
            let rotation = f64::from(f.rotation);
            map.add_decoration(FRAME, None, Key::Frame(f.entity_id), x, z, rotation, None);
        }
        Some(map)
    }
}
pub(super) struct Maps {
    pub data: HashMap<i32, MapData>,
    /// The highest map id handed out (`maps/last_id.dat`), loaded on first use.
    last_id: Option<i32>,
    /// The atlas slots holding map textures.
    pub slots: super::atlas_slots::AtlasSlots<i32>,
    missing: std::collections::HashSet<i32>,
}
impl Default for Maps {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            last_id: None,
            slots: super::atlas_slots::AtlasSlots::new(MAP_SLOTS),
            missing: Default::default(),
        }
    }
}
/// `MapColor.getColorFromPackedId` as RGBA bytes (NONE is transparent).
fn packed_rgba(packed: u8) -> [u8; 4] {
    let id = usize::from(packed >> 2);
    if id == 0 || id >= MAP_COLORS.len() {
        return [0; 4];
    }
    let modifier = [180u32, 220, 255, 135][usize::from(packed & 3)];
    let col = MAP_COLORS[id];
    let ch = |s: u32| ((col >> s & 255) * modifier / 255) as u8;
    [ch(16), ch(8), ch(0), 255]
}
/// `get(DataComponents.MAP_ID)`: the map id any stack carries. Filled maps
/// (explorer maps included) carry one; a command can give it to any item.
pub(super) fn map_id(stack: &ItemStack) -> Option<i32> {
    stack.data().and_then(|d| d.map_id)
}
/// `MapDecorationTypes` Horizon draws: (type, sprite).
const DECORATION_TYPES: [(&str, &str); 40] = [
    ("player", "map/decorations/player"),
    ("frame", "map/decorations/frame"),
    ("red_marker", "map/decorations/red_marker"),
    ("blue_marker", "map/decorations/blue_marker"),
    ("target_x", "map/decorations/target_x"),
    ("target_point", "map/decorations/target_point"),
    ("player_off_map", "map/decorations/player_off_map"),
    ("player_off_limits", "map/decorations/player_off_limits"),
    ("mansion", "map/decorations/woodland_mansion"),
    ("monument", "map/decorations/ocean_monument"),
    ("banner_white", "map/decorations/white_banner"),
    ("banner_orange", "map/decorations/orange_banner"),
    ("banner_magenta", "map/decorations/magenta_banner"),
    ("banner_light_blue", "map/decorations/light_blue_banner"),
    ("banner_yellow", "map/decorations/yellow_banner"),
    ("banner_lime", "map/decorations/lime_banner"),
    ("banner_pink", "map/decorations/pink_banner"),
    ("banner_gray", "map/decorations/gray_banner"),
    ("banner_light_gray", "map/decorations/light_gray_banner"),
    ("banner_cyan", "map/decorations/cyan_banner"),
    ("banner_purple", "map/decorations/purple_banner"),
    ("banner_blue", "map/decorations/blue_banner"),
    ("banner_brown", "map/decorations/brown_banner"),
    ("banner_green", "map/decorations/green_banner"),
    ("banner_red", "map/decorations/red_banner"),
    ("banner_black", "map/decorations/black_banner"),
    ("red_x", "map/decorations/red_x"),
    ("village_desert", "map/decorations/desert_village"),
    ("village_plains", "map/decorations/plains_village"),
    ("village_savanna", "map/decorations/savanna_village"),
    ("village_snowy", "map/decorations/snowy_village"),
    ("village_taiga", "map/decorations/taiga_village"),
    ("jungle_temple", "map/decorations/jungle_temple"),
    ("swamp_hut", "map/decorations/swamp_hut"),
    ("trial_chambers", "map/decorations/trial_chambers"),
    ("abandoned_camp", "map/decorations/abandoned_camp"),
    ("ancient_city", "map/decorations/ancient_city"),
    ("desert_pyramid", "map/decorations/desert_pyramid"),
    ("mineshaft", "map/decorations/mineshaft"),
    ("ocean_ruin_warm", "map/decorations/warm_ocean_ruins"),
];
/// `MapDecorationType.showOnItemFrame` is false for the player arrows and the two markers.
/// `MapRenderer.render`'s pose for decoration `d` in map pixels (0..128,
/// y down) at depth `z`: its quad is (-1, -1) to (1, 1) under it.
pub(super) fn decoration_pose(d: &Decoration, z: f32) -> glam::Mat4 {
    decoration_pose_at(d.x, d.y, d.rot, z)
}
/// [`decoration_pose`] for a decoration at `x`, `y` (half map pixels) turned `rot`.
pub(super) fn decoration_pose_at(x: i8, y: i8, rot: u8, z: f32) -> glam::Mat4 {
    glam::Mat4::from_translation(glam::Vec3::new(
        f32::from(x) / 2.0 + 64.0,
        f32::from(y) / 2.0 + 64.0,
        z,
    )) * glam::Mat4::from_rotation_z((f32::from(rot) * 360.0 / 16.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(4.0))
        * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.125, 0.0))
}
pub(super) fn shown_on_frame(sprite: &str) -> bool {
    !matches!(
        decoration_type(sprite),
        Some("player" | "red_marker" | "blue_marker" | "player_off_map" | "player_off_limits")
    )
}
/// `MapDecorationType.trackCount`: the players' arrows, the markers, the
/// frames and the banners count toward a map's limit, the structures do not.
fn tracks_count(sprite: &str) -> bool {
    decoration_type(sprite).is_some_and(|t| {
        t.starts_with("banner_")
            || matches!(
                t,
                "player"
                    | "frame"
                    | "red_marker"
                    | "blue_marker"
                    | "player_off_map"
                    | "player_off_limits"
            )
    })
}
/// The sprite of a decoration type that a `minecraft:map_decorations`
/// entry or a server's packet names.
fn decoration_sprite(id: &str) -> Option<&'static str> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    DECORATION_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sprite)| *sprite)
}
/// The type a decoration's sprite stands for, as its registry names it.
fn decoration_type(sprite: &str) -> Option<&'static str> {
    DECORATION_TYPES
        .iter()
        .find(|(_, s)| *s == sprite)
        .map(|(name, _)| *name)
}
/// `minecraft:map_decorations` of a map item: (sprite, x, z, rotation), in
/// the component's order.
fn static_decorations(stack: &ItemStack) -> Vec<(&'static str, f64, f64, f32)> {
    let Some(Tag::Compound(entries)) = stack.data().and_then(|d| {
        d.other
            .iter()
            .find(|(k, _)| k == "minecraft:map_decorations")
            .map(|(_, t)| t)
    }) else {
        return Vec::new();
    };
    let number = |t: Option<&Tag>| match t {
        Some(Tag::Double(b)) => Some(f64::from_bits(*b)),
        Some(Tag::Float(b)) => Some(f64::from(f32::from_bits(*b))),
        Some(Tag::Int(v)) => Some(f64::from(*v)),
        _ => None,
    };
    entries
        .values()
        .filter_map(|e| {
            let sprite = match e.get("type") {
                Some(Tag::String(s)) => decoration_sprite(&s.to_string_lossy())?,
                _ => return None,
            };
            Some((
                sprite,
                number(e.get("x"))?,
                number(e.get("z"))?,
                number(e.get("rotation")).unwrap_or(0.0) as f32,
            ))
        })
        .collect()
}
/// `MapItemSavedData.addTargetDecoration`: the `"+"` entry of
/// `minecraft:map_decorations`, rotation 180.
fn target_decoration(decoration: &str, at: [i32; 3]) -> Tag {
    Tag::compound([(
        "+",
        Tag::compound([
            ("type", Tag::String(decoration.into())),
            ("x", Tag::Double(f64::from(at[0]).to_bits())),
            ("z", Tag::Double(f64::from(at[2]).to_bits())),
            ("rotation", Tag::Float(180.0f32.to_bits())),
        ]),
    )])
}
/// `MapItemSavedData.clampMapCoordinate`.
fn clamp_map_coordinate(d: f32) -> i8 {
    if d <= -63.0 {
        -128
    } else if d >= 63.0 {
        127
    } else {
        (d * 2.0 + 0.5) as i8
    }
}
/// `MapItemSavedData.calculateRotation`: spinning in the Nether.
fn decoration_rotation(nether: bool, game_time: i64, y_rot: f64) -> u8 {
    if nether {
        let s = (game_time / 10) as i32;
        (s.wrapping_mul(s)
            .wrapping_mul(34_187_121)
            .wrapping_add(s.wrapping_mul(121))
            >> 15
            & 15) as u8
    } else {
        let adjusted = if y_rot < 0.0 {
            y_rot - 8.0
        } else {
            y_rot + 8.0
        };
        ((adjusted * 16.0 / 360.0) as i32 & 15) as u8
    }
}
impl Game<'_> {
    fn dimension_id(&self) -> &'static str {
        match self.dim {
            super::Dim::Nether => "minecraft:the_nether",
            super::Dim::End => "minecraft:the_end",
            _ => "minecraft:overworld",
        }
    }
    fn map_path(&self, name: &str) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/maps").join(name)
    }
    /// The map's data, loaded from `data/minecraft/maps/<id>.dat` on first use.
    pub(super) fn map(&mut self, id: i32) -> Option<&mut MapData> {
        if !self.maps.data.contains_key(&id) && !self.maps.missing.contains(&id) {
            let loaded = horizon_client::save::read_dat(&self.map_path(&format!("{id}.dat")))
                .and_then(|t| MapData::from_nbt(&t));
            match loaded {
                Some(m) => {
                    self.maps.data.insert(id, m);
                }
                None => {
                    self.maps.missing.insert(id);
                }
            }
        }
        self.maps.data.get_mut(&id)
    }
    /// `ClientboundMapItemDataPacket.handle`: the server's colours and
    /// decorations for a map (one carried or framed on a server).
    pub(super) fn remote_map_item_data(
        &mut self,
        p: &horizon_protocol::packets::play::MapItemData,
    ) {
        let decorations = p.decorations.as_ref().map(|list| {
            let types = self
                .remote
                .as_ref()
                .and_then(|r| r.builtin.get("minecraft:map_decoration_type"));
            list.iter()
                .filter_map(|d| {
                    Some(Decoration {
                        sprite: decoration_sprite(types?.name(d.kind)?)?,
                        x: d.x,
                        y: d.y,
                        rot: (d.rot & 15) as u8,
                        name: d
                            .name
                            .as_ref()
                            .map(|t| horizon_client::entity_nbt::name_text(t).plain()),
                        key: Key::Item,
                    })
                })
                .collect()
        });
        let dimension = self.dimension_id();
        let map = self
            .maps
            .data
            .entry(p.map_id)
            .or_insert_with(|| MapData::fresh(0.0, 0.0, p.scale.max(0) as u8, dimension));
        map.locked = p.locked;
        if let Some(decorations) = decorations {
            map.decorations = decorations;
        }
        let Some(patch) = &p.patch else {
            map.dirty = false;
            return;
        };
        for x in 0..patch.width {
            for y in 0..patch.height {
                let Some(&color) = patch.colors.get((x + y * patch.width) as usize) else {
                    continue;
                };
                let (px, py) = (patch.start_x + x, patch.start_y + y);
                if (0..128).contains(&px) && (0..128).contains(&py) {
                    map.update_color(px, py, color);
                }
            }
        }
        // The server owns a remote map: only the texture has to change.
        map.dirty = false;
    }
    fn next_map_id(&mut self) -> i32 {
        let last = *self.maps.last_id.get_or_insert_with(|| {
            horizon_client::save::read_dat(&self.world_root.join("data/minecraft/maps/last_id.dat"))
                .and_then(|t| match t.get("data")?.get("map")? {
                    Tag::Int(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(-1)
        });
        self.maps.last_id = Some(last + 1);
        last + 1
    }
    /// The id the next new map gets, without taking it.
    pub(super) fn peek_map_id(&mut self) -> i32 {
        let id = self.next_map_id();
        self.maps.last_id = Some(id - 1);
        id
    }
    /// Maps made by exploration map trades: (id, target, zoom, decoration).
    pub(super) fn register_exploration_maps(&mut self, made: Vec<(i32, [i32; 3], u8, String)>) {
        let dimension = self.dimension_id().to_owned();
        for (id, at, zoom, _) in made {
            // MapItem.applyNewSavedData: tracking, unlimited tracking.
            let mut data = MapData::fresh(f64::from(at[0]), f64::from(at[2]), zoom, &dimension);
            data.unlimited = true;
            self.maps.data.insert(id, data);
            let last = self.maps.last_id.unwrap_or(-1).max(id);
            self.maps.last_id = Some(last);
            self.render_biome_preview(id);
        }
    }
    /// `ExplorationMapFunction.run` for an item a generated chest still holds
    /// unsearched (`structure_blocks::take_pending_map`): the nearest
    /// structure of the tag around `origin`, a new map centred on it with its
    /// biome preview and target decoration. False when nothing is found: the
    /// loot table's `filtered` map_id check would have discarded the item.
    pub(super) fn resolve_pending_map(&mut self, item: &mut ItemStack, origin: [f64; 3]) -> bool {
        let Some(map) = horizon_client::structure_blocks::take_pending_map(item) else {
            return true;
        };
        let names = self.structure_tag(&map.destination);
        let from = origin.map(|v| v.floor() as i32);
        let found = self
            .end
            .locator()
            .filter(|_| !names.is_empty())
            .and_then(|t| {
                t.locate_for_map(&names, from, map.search_radius, map.skip_existing_chunks)
            });
        let Some((_, at)) = found else {
            return false;
        };
        let mut data = MapData::fresh(
            f64::from(at[0]),
            f64::from(at[2]),
            map.zoom,
            self.dimension_id(),
        );
        data.unlimited = true;
        let id = self.new_map_id(data);
        self.render_biome_preview(id);
        let d = item.data_mut();
        d.map_id = Some(id);
        d.other.retain(|(k, _)| k != "minecraft:map_decorations");
        d.other.push((
            "minecraft:map_decorations".into(),
            target_decoration(&map.decoration, at),
        ));
        true
    }
    /// The unsearched exploration maps of a container being opened
    /// (`RandomizableContainer.unpackLootTable` runs the function then).
    pub(super) fn resolve_container_maps(&mut self, key: (i32, i32, i32), origin: [f64; 3]) {
        use horizon_client::containers::BlockEntity;
        let Some(BlockEntity::Chest(slots)) = self.world.block_entities.get(&key) else {
            return;
        };
        let pending: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.as_ref()
                    .is_some_and(horizon_client::structure_blocks::has_pending_map)
            })
            .map(|(i, _)| i)
            .collect();
        for i in pending {
            let Some(BlockEntity::Chest(slots)) = self.world.block_entities.get_mut(&key) else {
                return;
            };
            let Some(mut item) = slots[i].take() else {
                continue;
            };
            let keep = self.resolve_pending_map(&mut item, origin);
            if let Some(BlockEntity::Chest(slots)) = self.world.block_entities.get_mut(&key) {
                slots[i] = keep.then_some(item);
            }
        }
    }
    /// `MapItem.renderBiomePreviewMap`: the map's biomes at sea level, water
    /// outlines (`#water_on_map_outlines`) drawn in orange and brown.
    pub(super) fn render_biome_preview(&mut self, id: i32) {
        let Some(terrain) = self.end.locator() else {
            return;
        };
        let dim = self.dimension_id();
        let zoom = self.streaming.biome_zoom;
        let watery_biomes: std::collections::HashSet<String> = self
            .tag_entries("worldgen/biome", "minecraft:water_on_map_outlines")
            .into_iter()
            .collect();
        let Some(m) = self.maps.data.get(&id) else {
            return;
        };
        if m.dimension != dim {
            return;
        }
        let scale = 1i32 << m.scale;
        let start_x = m.center[0] / scale - 64;
        let start_z = m.center[1] / scale - 64;
        let sea_level = 63;
        let quarts: Vec<[i32; 3]> = (0..128)
            .flat_map(|row| {
                (0..128).map(move |column| {
                    zoom.quart_at_block(horizon_core::position::BlockPos::new(
                        (start_x + column) * scale,
                        sea_level,
                        (start_z + row) * scale,
                    ))
                })
            })
            .collect();
        let watery: Vec<bool> = terrain
            .noise_biomes(&quarts)
            .into_iter()
            .map(|b| b.is_some_and(|b| watery_biomes.contains(&b)))
            .collect();
        let is_watery = |x: i32, z: i32| watery[(z * 128 + x) as usize];
        // MapColor ids and MapColor.Brightness ids.
        const ORANGE: u8 = 15;
        const BROWN: u8 = 26;
        const LOW: u8 = 0;
        const NORMAL: u8 = 1;
        const HIGH: u8 = 2;
        const LOWEST: u8 = 3;
        let Some(m) = self.maps.data.get_mut(&id) else {
            return;
        };
        for mx in 1..127 {
            for mz in 1..127 {
                let mut water_count = 0;
                for dx in -1..2 {
                    for dz in -1..2 {
                        if (dx != 0 || dz != 0) && is_watery(mx + dx, mz + dz) {
                            water_count += 1;
                        }
                    }
                }
                let mut brightness = LOWEST;
                let mut color = 0;
                if is_watery(mx, mz) {
                    color = ORANGE;
                    if water_count > 7 && mz % 2 == 0 {
                        let sin = horizon_core::geometry::trig::sin(f64::from(mz as f32 + 0.0));
                        match (mx + (sin * 7.0) as i32) / 8 % 5 {
                            0 | 4 => brightness = LOW,
                            1 | 3 => brightness = NORMAL,
                            2 => brightness = HIGH,
                            _ => {}
                        }
                    } else if water_count > 7 {
                        color = 0;
                    } else if water_count > 5 {
                        brightness = NORMAL;
                    } else if water_count > 1 {
                        brightness = LOW;
                    }
                } else if water_count > 0 {
                    color = BROWN;
                    brightness = if water_count > 3 { NORMAL } else { LOWEST };
                }
                if color != 0 {
                    m.update_color(mx, mz, color << 2 | brightness);
                }
            }
        }
    }
    /// Registers `data` under a free map id.
    pub(super) fn new_map_id(&mut self, data: MapData) -> i32 {
        let id = self.next_map_id();
        self.maps.data.insert(id, data);
        id
    }
    /// `EmptyMapItem.use`: a new map centred around the player.
    pub(super) fn use_empty_map(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:map" {
            return false;
        }
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return false;
        };
        let id = self.next_map_id();
        let data = MapData::fresh(p.x, p.z, 0, self.dimension_id());
        self.maps.data.insert(id, data);
        let mut filled = ItemStack::new("minecraft:filled_map", 1);
        filled.data_mut().map_id = Some(id);
        if self.creative {
            self.give_or_drop_stack(filled);
        } else if stack.count == 1 {
            *self.held_mut() = Some(filled);
        } else {
            self.consume_held();
            self.give_or_drop_stack(filled);
        }
        let at = [p.x, p.y, p.z];
        self.audio.play(
            "minecraft:ui.cartography_table.take_result",
            Some(at),
            1.0,
            1.0,
        );
        self.swing_arm();
        true
    }
    /// `ItemFrame.removeFramedMap`: the map `frame` holds forgets the frame.
    pub(super) fn remove_framed_map(&mut self, frame: &horizon_client::entities::ItemFrame) {
        let Some(id) = frame.item.as_ref().and_then(map_id) else {
            return;
        };
        if let Some(m) = self.map(id) {
            m.removed_from_frame(frame.pos, frame.id as i32);
        }
    }
    /// `MapItem.useOn`: a map used on a banner toggles that banner's marker
    /// on the map; when the map's data says no (`toggleBanner` fails) the use
    /// ends without a swing, and nothing else is done with the map either way.
    pub(super) fn use_map_on_banner(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        let Some(id) = map_id(stack) else {
            return false;
        };
        // ItemStack.useOn passes in adventure mode (its `can_place_on`
        // exception is not ported).
        if !self.may_build() {
            return false;
        }
        let [x, y, z] = hit.pos;
        let block = self.res.facts().get(self.world.block(x, y, z)).name.clone();
        if !self.block_in_tag("minecraft:banners", &block) {
            return false;
        }
        let current = self.map_banner_at(hit.pos);
        let game_time = self.world.game_time;
        let failed = self
            .map(id)
            .is_some_and(|m| !m.toggle_banner(hit.pos, current, game_time));
        if failed {
            return true;
        }
        self.trigger_block("minecraft:item_used_on_block", hit.pos, stack);
        self.stats.add("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `CompassItem.useOn`: a compass used on a lodestone locks onto it (a
    /// single one in survival changes in place, else one is split off).
    pub(super) fn use_lodestone_compass(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:compass" {
            return false;
        }
        let [x, y, z] = hit.pos;
        if self
            .res
            .facts()
            .get(self.world.block(x, y, z))
            .name
            .as_ref()
            != "minecraft:lodestone"
        {
            return false;
        }
        let at = [x, y, z].map(|v| f64::from(v) + 0.5);
        self.trigger_block("minecraft:item_used_on_block", hit.pos, stack);
        self.audio
            .play("minecraft:item.lodestone_compass.lock", Some(at), 1.0, 1.0);
        let target = Tag::compound([(
            "target",
            Tag::compound([
                ("dimension", Tag::String(self.dimension_id().into())),
                ("pos", Tag::IntArray(vec![x, y, z])),
            ]),
        )]);
        if !self.creative && stack.count == 1 {
            if let Some(held) = self.held_mut() {
                held.data_mut()
                    .set_other("minecraft:lodestone_tracker", Some(target));
            }
        } else {
            let mut locked = stack.clone();
            locked.count = 1;
            locked
                .data_mut()
                .set_other("minecraft:lodestone_tracker", Some(target));
            self.consume_held();
            self.give_or_drop_stack(locked);
        }
        self.stats.add("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `CompassItem.inventoryTick` / `LodestoneTracker.tick`: a compass whose
    /// lodestone is gone loses its target. A target in an unloaded chunk waits.
    pub(super) fn tick_lodestone_compasses(&mut self) {
        let dim = self.dimension_id();
        let gone = |game: &Self, stack: &ItemStack| -> bool {
            let Some(tracker) = stack
                .data()
                .and_then(|d| d.other("minecraft:lodestone_tracker"))
            else {
                return false;
            };
            let (Some(target), true) = (
                tracker.get("target"),
                horizon_client::save::number(tracker.get("tracked")).is_none_or(|v| v != 0),
            ) else {
                return false;
            };
            let Some(Tag::IntArray(pos)) = target.get("pos") else {
                return false;
            };
            let [x, y, z] = [pos[0], pos[1], pos[2]];
            horizon_client::save::text(target.get("dimension")).as_deref() == Some(dim)
                && game.world.chunks.contains_key(&(x >> 4, z >> 4))
                && game
                    .res
                    .facts()
                    .get(game.world.block(x, y, z))
                    .name
                    .as_ref()
                    != "minecraft:lodestone"
        };
        for slot in (0..self.inventory.len()).chain([super::shield::OFFHAND_SLOT]) {
            if self.stack_in(slot).is_some_and(|s| gone(self, s))
                && let Some(s) = self.stack_in_mut(slot)
            {
                s.data_mut().set_other(
                    "minecraft:lodestone_tracker",
                    Some(Tag::Compound(Default::default())),
                );
            }
        }
    }
    fn give_or_drop_stack(&mut self, stack: ItemStack) {
        if let Some(rest) = self.give(stack)
            && let Some(eye) = self.eye_position()
        {
            self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
        }
    }
    /// `MapItemSavedData.hasMapInvisibilityItemEquipped`: a piece of
    /// `#map_invisibility_equipment` (a carved pumpkin) in an armor slot; the
    /// hands do not count.
    fn map_invisibility_worn(&self, worn: &[Option<ItemStack>]) -> bool {
        let tag = self
            .res
            .items()
            .item_tags
            .get("minecraft:map_invisibility_equipment");
        worn.iter()
            .flatten()
            .any(|s| tag.is_some_and(|t| t.contains(&*s.item)))
    }
    /// Held maps update; every carried map tracks the player marker.
    pub(super) fn tick_maps(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        // A generated exploration map that reached the inventory unsearched
        // (moved out of its chest by a hopper): searched from here.
        for i in 0..self.inventory.len() {
            if self.inventory[i]
                .as_ref()
                .is_some_and(horizon_client::structure_blocks::has_pending_map)
                && let Some(mut item) = self.inventory[i].take()
            {
                let keep = self.resolve_pending_map(&mut item, [p.x, p.y, p.z]);
                self.inventory[i] = keep.then_some(item);
            }
        }
        let held: Vec<i32> = [Some(self.selected), Some(super::shield::OFFHAND_SLOT)]
            .into_iter()
            .flatten()
            .filter_map(|i| self.stack_in(i).and_then(map_id))
            .collect();
        let carried: Vec<(i32, Vec<(&'static str, f64, f64, f32)>)> = self
            .inventory
            .iter()
            .chain(std::iter::once(&self.offhand))
            .flatten()
            .filter_map(|s| Some((map_id(s)?, static_decorations(s))))
            .collect();
        let (yaw, dim) = (self.yaw, self.dimension_id());
        let (nether, game_time) = (self.dim == super::Dim::Nether, self.world.game_time);
        let invisible = self.map_invisibility_worn(&self.armor);
        // The guest whose turn it is, else the host.
        let me = self.lan_acting.unwrap_or(HOST);
        for (id, targets) in carried {
            let Some(m) = self.map(id) else { continue };
            let h = m.holder(me);
            h.carried = game_time;
            h.invisible = invisible;
            m.drop_holders(game_time);
            // tickCarriedBy: the player marker while tracking in this dimension.
            if m.tracking && m.dimension == dim {
                m.add_decoration(
                    PLAYER,
                    Some(game_time),
                    Key::Player(me),
                    p.x,
                    p.z,
                    f64::from(yaw),
                    None,
                );
            } else if m.decorations.iter().any(|d| d.key == Key::Player(me)) {
                m.remove_decoration(Key::Player(me));
            }
            m.hide_invisible(me);
            // The item's MAP_DECORATIONS (an explorer map's target), added
            // after the players as addDecoration would.
            let s = (1 << m.scale) as f32;
            let mut items: Vec<Decoration> = Vec::new();
            for (sprite, x, z, rotation) in targets {
                let dx = (x - f64::from(m.center[0])) as f32 / s;
                let dz = (z - f64::from(m.center[1])) as f32 / s;
                let inside = (-63.0..=63.0).contains(&dx) && (-63.0..=63.0).contains(&dz);
                if !inside && !m.unlimited {
                    continue;
                }
                let d = Decoration {
                    sprite,
                    x: clamp_map_coordinate(dx),
                    y: clamp_map_coordinate(dz),
                    rot: decoration_rotation(nether, game_time, f64::from(rotation)),
                    name: None,
                    key: Key::Item,
                };
                if !items.contains(&d) {
                    items.push(d);
                }
            }
            if !m
                .decorations
                .iter()
                .filter(|d| d.key == Key::Item)
                .eq(items.iter())
            {
                m.decorations.retain(|d| d.key != Key::Item);
                m.decorations.extend(items);
                m.decorations_changed();
            }
        }
        for id in held {
            self.update_map(id);
        }
    }
    /// The maps the player carries, for `status`: how many of their pixels
    /// are drawn, and their decorations.
    pub(super) fn maps_status(&self) -> Vec<serde_json::Value> {
        self.inventory
            .iter()
            .chain(std::iter::once(&self.offhand))
            .flatten()
            .filter_map(map_id)
            .filter_map(|id| {
                let m = self.maps.data.get(&id)?;
                Some(serde_json::json!({
                    "id": id,
                    "pixels": m.colors.iter().filter(|&&c| c != 0).count(),
                    "decorations": m.decorations.iter().map(|d| d.sprite).collect::<Vec<_>>(),
                }))
            })
            .collect()
    }
    /// `ServerPlayer.synchronizeSpecialItemUpdates` for each map the guest
    /// carries, and every 10 ticks (`ServerEntity.sendChanges`) for each map
    /// framed in the level: a `ClientboundMapItemDataPacket` when something
    /// changed since the guest's last one.
    pub(super) fn lan_map_updates(&mut self, host: &LanHost, conn: u32) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let mut ids: Vec<i32> = g
            .inventory
            .iter()
            .chain(std::iter::once(&g.offhand))
            .flatten()
            .filter_map(map_id)
            .collect();
        let now = self.world.game_time;
        if now % 10 == 0 && host.hub.here(g) {
            let framed: Vec<(i32, std::sync::Arc<str>, Framed)> = self
                .item_frames
                .iter()
                // ServerEntity.sendChanges: only a framed MapItem (a filled
                // map), not any stack that carries a map id.
                .filter_map(|f| {
                    let item = f.item.as_ref()?;
                    if &*item.item != "minecraft:filled_map" {
                        return None;
                    }
                    let frame = Framed {
                        pos: f.pos,
                        facing: f.facing,
                        id: f.id as i32,
                    };
                    Some((map_id(item)?, item.item.clone(), frame))
                })
                .collect();
            let invisible = self.map_invisibility_worn(&g.armor);
            for (id, item, frame) in framed {
                // tickCarriedBy's mapMatcher: the same item with the same map id.
                let carries = g
                    .inventory
                    .iter()
                    .chain(std::iter::once(&g.offhand))
                    .flatten()
                    .any(|s| s.item == item && map_id(s) == Some(id));
                if let Some(m) = self.map(id) {
                    m.tick_framed(conn, now, carries, invisible, frame);
                    ids.push(id);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let kind = |name: &str| {
            host.hub.registry_id(
                "minecraft:map_decoration_type",
                &format!("minecraft:{name}"),
            )
        };
        for id in ids {
            if let Some(p) = self
                .maps
                .data
                .get_mut(&id)
                .and_then(|m| m.update_packet(id, conn, kind))
            {
                g.send(&horizon_protocol::packets::play::Clientbound::MapItemData(
                    p,
                ));
            }
        }
    }
    /// `MapItemSavedData.checkBanners` on map `id` for the column `x`, `z`
    /// that `MapItem.update` is reading.
    fn check_banners(&mut self, id: i32, x: i32, z: i32) {
        let (world, res) = (&self.world, &self.res);
        let Some(m) = self.maps.data.get_mut(&id) else {
            return;
        };
        if m.banners.is_empty() {
            return;
        }
        m.check_banners(x, z, |pos| {
            let state = world.block(pos[0], pos[1], pos[2]);
            super::banners::map_banner(
                pos,
                world.block_entities.get(&(pos[0], pos[1], pos[2])),
                &res.facts().get(state).name,
            )
        });
    }
    /// `MapItem.update` for the player holding map `id`.
    fn update_map(&mut self, id: i32) {
        let dim = self.dimension_id();
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let nether = self.dim == super::Dim::Nether;
        let Some(m) = self.maps.data.get_mut(&id) else {
            return;
        };
        if m.locked || m.dimension != dim {
            return;
        }
        let scale = 1i32 << m.scale;
        let [cx, cz] = m.center;
        let px = (p.x - f64::from(cx)).floor() as i32 / scale + 64;
        let pz = (p.z - f64::from(cz)).floor() as i32 / scale + 64;
        let mut radius = 128 / scale;
        if nether {
            radius /= 2;
        }
        let h = m.holder(self.lan_acting.unwrap_or(HOST));
        h.step += 1;
        let step = h.step;
        let mut changes: Vec<(i32, i32, u8)> = Vec::new();
        let mut consecutive = false;
        for img_x in px - radius + 1..px + radius {
            if (img_x & 15) != (step & 15) && !consecutive {
                continue;
            }
            consecutive = false;
            let mut previous_height = 0.0f64;
            for img_y in pz - radius - 1..pz + radius {
                if !(0..128).contains(&img_x) || !(-1..128).contains(&img_y) {
                    continue;
                }
                let d2 = (img_x - px).pow(2) + (img_y - pz).pow(2);
                let dither_black = d2 > (radius - 2) * (radius - 2);
                let min_x = (cx / scale + img_x - 64) * scale;
                let min_z = (cz / scale + img_y - 64) * scale;
                if !self.world.chunks.contains_key(&(min_x >> 4, min_z >> 4)) {
                    continue;
                }
                let mut counts: Vec<(u8, u32)> = Vec::new();
                let mut add = |c: u8, n: u32| match counts.iter_mut().find(|e| e.0 == c) {
                    Some(e) => e.1 += n,
                    None => counts.push((c, n)),
                };
                let mut water_depth = 0i32;
                let mut height = 0.0f64;
                if nether {
                    let mut noise = min_x.wrapping_add(min_z.wrapping_mul(231_871));
                    noise = noise
                        .wrapping_mul(noise)
                        .wrapping_mul(31_287_121)
                        .wrapping_add(noise.wrapping_mul(11));
                    // Dirt (10) or stone (11) noise under a ceiling.
                    if noise >> 20 & 1 == 0 {
                        add(10, 10);
                    } else {
                        add(11, 100);
                    }
                    height = 100.0;
                } else {
                    for dx in 0..scale {
                        for dz in 0..scale {
                            let (x, z) = (min_x + dx, min_z + dz);
                            let mut y = self.world_surface(x, z);
                            let min_y = self.world.bottom_y();
                            let mut color;
                            if y <= min_y {
                                color = 11;
                            } else {
                                loop {
                                    y -= 1;
                                    color =
                                        self.res.facts().get(self.world.block(x, y, z)).map_color;
                                    if color != 0 || y <= min_y {
                                        break;
                                    }
                                }
                                let f = self.res.facts().get(self.world.block(x, y, z));
                                if y > min_y && f.fluid != Fluid::None {
                                    let mut solid = y - 1;
                                    loop {
                                        let below =
                                            self.res.facts().get(self.world.block(x, solid, z));
                                        solid -= 1;
                                        water_depth += 1;
                                        if solid <= min_y || below.fluid == Fluid::None {
                                            break;
                                        }
                                    }
                                    // getCorrectStateForFluidBlock: the fluid unless sturdy on top.
                                    if f.sturdy_full >> 1 & 1 == 0 {
                                        color = if f.fluid == Fluid::Lava { 4 } else { 12 };
                                    }
                                }
                            }
                            self.check_banners(id, x, z);
                            height += f64::from(y) / f64::from(scale * scale);
                            add(color, 1);
                        }
                    }
                }
                water_depth /= scale * scale;
                // Multisets.copyHighestCountFirst: the most common, first seen on ties.
                let best = counts
                    .iter()
                    .fold(None::<(u8, u32)>, |acc, &(c, n)| match acc {
                        Some((_, bn)) if bn >= n => acc,
                        _ => Some((c, n)),
                    })
                    .map_or(0, |e| e.0);
                let parity = f64::from((img_x + img_y) & 1);
                let brightness = if best == 12 {
                    let diff = f64::from(water_depth) * 0.1 + parity * 0.2;
                    if diff < 0.5 {
                        2
                    } else if diff > 0.9 {
                        0
                    } else {
                        1
                    }
                } else {
                    let diff = (height - previous_height) * 4.0 / f64::from(scale + 4)
                        + (parity - 0.5) * 0.4;
                    if diff > 0.6 {
                        2
                    } else if diff < -0.6 {
                        0
                    } else {
                        1
                    }
                };
                previous_height = height;
                if img_y >= 0 && d2 < radius * radius && (!dither_black || (img_x + img_y) & 1 != 0)
                {
                    changes.push((img_x, img_y, best << 2 | brightness));
                }
            }
            if let Some(m) = self.maps.data.get_mut(&id) {
                for (x, y, c) in changes.drain(..) {
                    consecutive |= m.update_color(x, y, c);
                }
            }
        }
    }
    /// Writes changed maps and the id counter (`MapItemSavedData`, `MapIndex`).
    pub(super) fn save_maps(&mut self) {
        for (id, m) in &mut self.maps.data {
            if m.dirty {
                let path = self
                    .world_root
                    .join("data/minecraft/maps")
                    .join(format!("{id}.dat"));
                if horizon_client::save::write_dat(&path, m.to_nbt()).is_ok() {
                    m.dirty = false;
                }
            }
        }
        if let Some(last) = self.maps.last_id {
            let tag = Tag::compound([
                ("data", Tag::compound([("map", Tag::Int(last))])),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]);
            if let Err(e) = horizon_client::save::write_dat(
                &self.world_root.join("data/minecraft/maps/last_id.dat"),
                tag,
            ) {
                log::warn!("Horizon: the next map id is not saved: {e}");
            }
        }
    }
    /// The atlas slot showing map `id`, its texture uploaded when it changed.
    pub(super) fn map_texture(&mut self, id: i32) -> Option<Rect> {
        self.map(id)?;
        let slot = match self.maps.slots.show(&id) {
            Some(slot) => slot,
            None => {
                let (slot, _) = self.maps.slots.claim(id);
                if let Some(m) = self.maps.data.get_mut(&id) {
                    m.texture_dirty = true;
                }
                slot
            }
        };
        let rect = self
            .gui
            .atlas
            .sprites
            .get(&format!("horizon/map{slot}"))
            .copied()?;
        let m = self.maps.data.get_mut(&id)?;
        if m.texture_dirty {
            m.texture_dirty = false;
            let rgba: Vec<u8> = m.colors.iter().flat_map(|&c| packed_rgba(c)).collect();
            let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
            self.map_patches.push(horizon_client::font::Patch {
                x: (rect.u0 * w).round() as u32,
                y: (rect.v0 * h).round() as u32,
                width: 128,
                height: 128,
                rgba,
            });
        }
        Some(rect)
    }
    /// `MapRenderer.render`: the map texture and its decorations under `m`
    /// (map pixels 0..128, y down), drawn at depth offsets toward the viewer.
    /// `show_only_frame` (a map in an item frame) leaves out the decorations that
    /// do not show on frames.
    pub(super) fn map_vertices(
        &mut self,
        id: i32,
        show_only_frame: bool,
        m: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let Some(rect) = self.map_texture(id) else {
            return;
        };
        let quad = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], out: &mut Vec<Vertex>| {
            for (c, uv) in corners.into_iter().zip(uv) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,

                    glint_uv: [0.0, 0.0],
                });
            }
        };
        let z = -0.01;
        quad(
            [
                [0.0, 128.0, z],
                [128.0, 128.0, z],
                [128.0, 0.0, z],
                [0.0, 0.0, z],
            ],
            [
                [rect.u0, rect.v1],
                [rect.u1, rect.v1],
                [rect.u1, rect.v0],
                [rect.u0, rect.v0],
            ],
            out,
        );
        let decorations = self
            .maps
            .data
            .get(&id)
            .map(|m| m.decorations.clone())
            .unwrap_or_default();
        let mut count = 0;
        for d in &decorations {
            if show_only_frame && !shown_on_frame(d.sprite) {
                continue;
            }
            let Some(s) = self.gui.atlas.sprites.get(d.sprite).copied() else {
                continue;
            };
            let dm = decoration_pose(d, -0.02 - count as f32 * 0.001);
            let p = |x: f32, y: f32| dm.transform_point3(glam::Vec3::new(x, y, 0.0)).to_array();
            let mut q = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4]| {
                for (c, uv) in corners.into_iter().zip(uv) {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                        uv,
                        layer: 0,
                        color: 0xFFFF_FFFF,
                        light,

                        glint_uv: [0.0, 0.0],
                    });
                }
            };
            q(
                [p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0), p(-1.0, -1.0)],
                [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]],
            );
            count += 1;
        }
        // A native mod's own icons, over the vanilla ones.
        for d in self.mod_map_decorations(id, show_only_frame) {
            let Some(s) = self.gui.atlas.sprites.get(d.sprite.as_str()).copied() else {
                continue;
            };
            let dm = decoration_pose_at(d.x, d.y, d.rot, -0.02 - count as f32 * 0.001);
            let p = |x: f32, y: f32| dm.transform_point3(glam::Vec3::new(x, y, 0.0)).to_array();
            let corners = [p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0), p(-1.0, -1.0)];
            let uvs = [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]];
            for (c, uv) in corners.into_iter().zip(uvs) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,
                    glint_uv: [0.0, 0.0],
                });
            }
            count += 1;
        }
    }
    /// The icons `native_map_decorations` adds to map `id`, as it is drawn
    /// (`frame`: in an item frame or on a screen, which show only what
    /// `shown_on_frame` allows).
    pub(super) fn mod_map_decorations(&self, id: i32, frame: bool) -> Vec<NativeMapDecoration> {
        let Some(map) = self.maps.data.get(&id) else {
            return Vec::new();
        };
        let info = NativeMapInfo {
            id,
            center: map.center,
            scale: map.scale,
            dimension: &map.dimension,
            locked: map.locked,
            in_frame: frame,
        };
        let mut decorations = Vec::new();
        self.native_map_decorations(&info, &mut decorations);
        decorations
            .into_iter()
            .filter(|d| !frame || d.shown_on_frame)
            .map(|d| NativeMapDecoration {
                rot: d.rot & 15,
                ..d
            })
            .collect()
    }
    /// `renderTwoHandedMap`: map `id` held in both hands, in view space (the
    /// arms under it: `first_person::map_arm_poses`).
    pub(super) fn two_handed_map_geometry(
        &mut self,
        to_world: glam::Mat4,
        attack: f32,
        inverse_height: f32,
        id: i32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use horizon_core::geometry::trig::Mth;
        let deg = |d: f32| d.to_radians();
        let base = to_world
            * horizon_client::first_person::two_handed_map_base(self.pitch, inverse_height, attack);
        let swing = (attack.sqrt() * std::f32::consts::PI).msin();
        let m = base
            * glam::Mat4::from_rotation_x(deg(swing * 20.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(2.0));
        self.held_map(m, id, light, out);
    }
    /// `renderMap`: map `id` at pose `m`, its background (with a 7-pixel
    /// border) then the map.
    pub(super) fn held_map(&mut self, m: glam::Mat4, id: i32, light: u32, out: &mut Vec<Vertex>) {
        let deg = |d: f32| d.to_radians();
        let m = m
            * glam::Mat4::from_rotation_y(deg(180.0))
            * glam::Mat4::from_rotation_z(deg(180.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.38))
            * glam::Mat4::from_translation(glam::Vec3::new(-0.5, -0.5, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.0078125));
        let has_data = self.map(id).is_some();
        let background = if has_data {
            "map/map_background_checkerboard"
        } else {
            "map/map_background"
        };
        if let Some(r) = self.gui.atlas.sprites.get(background).copied() {
            for (c, uv) in [
                ([-7.0, 135.0, 0.0], [r.u0, r.v1]),
                ([135.0, 135.0, 0.0], [r.u1, r.v1]),
                ([135.0, -7.0, 0.0], [r.u1, r.v0]),
                ([-7.0, -7.0, 0.0], [r.u0, r.v0]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if has_data {
            self.map_vertices(id, false, m, light, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_icon_sits_where_a_vanilla_decoration_would() {
        // Half pixels from the map's centre: x 20 is 10 map pixels right of it.
        let vanilla = Decoration {
            sprite: "map/decorations/frame",
            x: 20,
            y: -30,
            rot: 4,
            name: None,
            key: Key::Item,
        };
        assert_eq!(
            decoration_pose(&vanilla, -0.02),
            decoration_pose_at(20, -30, 4, -0.02)
        );
        let at = decoration_pose_at(20, -30, 0, 0.0).transform_point3(glam::Vec3::ZERO);
        assert!(
            (at.x - 74.0).abs() < 0.6 && (at.y - 49.0).abs() < 0.6,
            "{at:?}"
        );
    }

    #[test]
    fn every_target_decoration_type_has_its_sprite() {
        // MapDecorationTypes: (name, asset).
        for (name, asset) in [
            ("mansion", "woodland_mansion"),
            ("monument", "ocean_monument"),
            ("red_x", "red_x"),
            ("village_desert", "desert_village"),
            ("village_plains", "plains_village"),
            ("village_savanna", "savanna_village"),
            ("village_snowy", "snowy_village"),
            ("village_taiga", "taiga_village"),
            ("jungle_temple", "jungle_temple"),
            ("swamp_hut", "swamp_hut"),
            ("trial_chambers", "trial_chambers"),
            ("abandoned_camp", "abandoned_camp"),
            ("ancient_city", "ancient_city"),
            ("desert_pyramid", "desert_pyramid"),
            ("mineshaft", "mineshaft"),
            ("ocean_ruin_warm", "warm_ocean_ruins"),
        ] {
            assert_eq!(
                decoration_sprite(&format!("minecraft:{name}")),
                Some(format!("map/decorations/{asset}").as_str()),
                "{name}"
            );
        }
        assert_eq!(decoration_sprite("minecraft:nothing"), None);
    }

    #[test]
    fn target_decorations_read_back_with_vanilla_rotation() {
        let mut stack = ItemStack::new("minecraft:buried_treasure_map", 1);
        stack.data_mut().map_id = Some(3);
        stack.data_mut().other.push((
            "minecraft:map_decorations".into(),
            target_decoration("minecraft:red_x", [120, 40, -300]),
        ));
        assert_eq!(map_id(&stack), Some(3));
        let found = static_decorations(&stack);
        assert_eq!(found, vec![("map/decorations/red_x", 120.0, -300.0, 180.0)]);
        // calculateRotation(180): (byte) (188 * 16 / 360) = 8.
        assert_eq!(decoration_rotation(false, 0, 180.0), 8);
        assert_eq!(decoration_rotation(false, 0, -90.0), 12);
        assert_eq!(clamp_map_coordinate(-70.0), -128);
        assert_eq!(clamp_map_coordinate(10.25), 21);
    }
}
