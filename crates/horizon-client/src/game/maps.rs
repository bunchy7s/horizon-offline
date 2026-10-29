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

/// A decoration: sprite, map position (-128..127, half pixels) and rotation
/// (0..15), and the player it marks (an item's own decorations mark none).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Decoration {
    pub sprite: &'static str,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
    pub holder: Option<u32>,
}
/// The host among a map's holders (the guests are their connections).
const HOST: u32 = u32::MAX;
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
    /// `MapBanner` and `MapFrame` entries as saved (`banners`, `frames`), kept
    /// so a save does not drop them.
    pub banners: Vec<Tag>,
    frames: Vec<Tag>,
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
        self.decorations
            .retain(|d| d.holder.is_none_or(|w| holders.iter().any(|h| h.who == w)));
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
    /// `addDecoration` / `removeDecoration` for `holder`'s marker: in place,
    /// so that the order (the drawing order) stays.
    fn set_marker(&mut self, holder: u32, marker: Option<Decoration>) {
        let i = self
            .decorations
            .iter()
            .position(|d| d.holder == Some(holder));
        let changed = match (i, marker) {
            (Some(i), Some(d)) if self.decorations[i] == d => false,
            (Some(i), Some(d)) => {
                self.decorations[i] = d;
                true
            }
            (Some(i), None) => {
                self.decorations.remove(i);
                true
            }
            (None, Some(d)) => {
                self.decorations.push(d);
                true
            }
            (None, None) => false,
        };
        if changed {
            self.decorations_changed();
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
                            name: None,
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
                    ("banners", Tag::List(self.banners.clone())),
                    ("frames", Tag::List(self.frames.clone())),
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
        Some(Self {
            center: [int("xCenter")?, int("zCenter")?],
            scale: int("scale").unwrap_or(0).clamp(0, 4) as u8,
            dimension,
            locked: int("locked").unwrap_or(0) != 0,
            tracking: int("trackingPosition").unwrap_or(1) != 0,
            unlimited: int("unlimitedTracking").unwrap_or(0) != 0,
            colors,
            decorations: Vec::new(),
            banners: list("banners"),
            frames: list("frames"),
            holders: Vec::new(),
            dirty: false,
            texture_dirty: true,
        })
    }
}
#[derive(Default)]
pub(super) struct Maps {
    pub data: HashMap<i32, MapData>,
    /// The highest map id handed out (`maps/last_id.dat`), loaded on first use.
    last_id: Option<i32>,
    /// (map id, atlas slot), most recently shown last.
    slots: Vec<(i32, usize)>,
    missing: std::collections::HashSet<i32>,
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
/// `has(DataComponents.MAP_ID)`: filled maps and the explorer map items
/// (`buried_treasure_map`, `woodland_mansion_map`, ..., all `MapItem`s).
pub(super) fn map_id(stack: &ItemStack) -> Option<i32> {
    stack.data().and_then(|d| d.map_id)
}
/// `MapDecorationTypes` Horizon draws: (type, sprite).
const DECORATION_TYPES: [(&str, &str); 24] = [
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
fn shown_on_frame(sprite: &str) -> bool {
    !matches!(
        decoration_type(sprite),
        Some("player" | "red_marker" | "blue_marker" | "player_off_map" | "player_off_limits")
    )
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
                        holder: None,
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
            .and_then(|t| t.locate(&names, from, map.search_radius));
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
        // The guest whose turn it is, else the host.
        let me = self.lan_acting.unwrap_or(HOST);
        for (id, targets) in carried {
            let Some(m) = self.map(id) else { continue };
            m.holder(me).carried = game_time;
            m.drop_holders(game_time);
            // tickCarriedBy: the player marker while tracking in this dimension.
            let mut marker = None;
            if m.tracking && m.dimension == dim {
                let s = f64::from(1 << m.scale);
                let dx = ((p.x - f64::from(m.center[0])) / s) as f32;
                let dz = ((p.z - f64::from(m.center[1])) / s) as f32;
                let inside = (-63.0..=63.0).contains(&dx) && (-63.0..=63.0).contains(&dz);
                let clamp = |d: f32| {
                    if d <= -63.0 {
                        -128
                    } else if d >= 63.0 {
                        127
                    } else {
                        (d * 2.0 + 0.5) as i8
                    }
                };
                // calculateRotation: the arrow spins in the Nether.
                let rot = decoration_rotation(nether, game_time, f64::from(yaw));
                let sprite = if inside {
                    Some("map/decorations/player")
                } else if dx.abs() < 320.0 && dz.abs() < 320.0 {
                    Some("map/decorations/player_off_map")
                } else if m.unlimited {
                    Some("map/decorations/player_off_limits")
                } else {
                    None
                };
                marker = sprite.map(|sprite| Decoration {
                    sprite,
                    x: clamp(dx),
                    y: clamp(dz),
                    rot,
                    holder: Some(me),
                });
            }
            m.set_marker(me, marker);
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
                    holder: None,
                };
                if !items.contains(&d) {
                    items.push(d);
                }
            }
            if !m
                .decorations
                .iter()
                .filter(|d| d.holder.is_none())
                .eq(items.iter())
            {
                m.decorations.retain(|d| d.holder.is_some());
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
            let framed: Vec<i32> = self
                .item_frames
                .iter()
                .filter_map(|f| f.item.as_ref().and_then(map_id))
                .collect();
            for id in framed {
                if let Some(m) = self.map(id) {
                    m.holder(conn).framed = now;
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
                            let min_y = super::MIN_Y;
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
            let _ = horizon_client::save::write_dat(
                &self.world_root.join("data/minecraft/maps/last_id.dat"),
                tag,
            );
        }
    }
    /// The atlas slot showing map `id`, its texture uploaded when it changed.
    pub(super) fn map_texture(&mut self, id: i32) -> Option<Rect> {
        self.map(id)?;
        let slots = &mut self.maps.slots;
        if let Some(i) = slots.iter().position(|e| e.0 == id) {
            let e = slots.remove(i);
            slots.push(e);
        } else {
            // A free slot, else the least recently shown one.
            let slot = if slots.len() < MAP_SLOTS {
                slots.len()
            } else {
                slots.remove(0).1
            };
            slots.push((id, slot));
            if let Some(m) = self.maps.data.get_mut(&id) {
                m.texture_dirty = true;
            }
        }
        let slot = self.maps.slots.last()?.1;
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
            let dm = glam::Mat4::from_translation(glam::Vec3::new(
                f32::from(d.x) / 2.0 + 64.0,
                f32::from(d.y) / 2.0 + 64.0,
                -0.02 - count as f32 * 0.001,
            )) * glam::Mat4::from_rotation_z(
                (f32::from(d.rot) * 360.0 / 16.0).to_radians(),
            ) * glam::Mat4::from_scale(glam::Vec3::splat(4.0))
                * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.125, 0.0));
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
