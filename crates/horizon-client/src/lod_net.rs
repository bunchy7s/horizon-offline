//! Far terrain from a server (`horizon:lod`): a Horizon client playing on
//! another machine's world has neither the seed nor the generator its far
//! terrain samples, so the server samples the tiles for it, up to the distance
//! its owner allows (`horizon-lod-distance`). Only a dedicated server can offer
//! it for now: an open-to-LAN world has no setting for that distance yet, so
//! its `LanConfig::lod_distance` stays 0.
//!
//! The server offers it with an [`Offer`] payload in the configuration phase,
//! to Horizon clients only (a vanilla client never hears of it). The client
//! then asks for one far tile at a time ([`Request`]: 16 x 16 columns, as
//! `lod::Lod` samples them) and the server answers each with the tile's
//! columns, or a refusal. The offer carries the server's limits, which the
//! client keeps to: requests in flight and tiles a second. The server checks
//! them again, with the tile's distance from the player, and samples on one
//! background thread for every player in turn (`server`), so far terrain
//! never takes more than that thread's time from the ticks.
//!
//! It is off unless the owner sets the key: far tiles show the generator's
//! terrain far beyond view distance, which helps crack a world's seed.
use crate::lod::{Column, Span, Spans, Surface};
use horizon_protocol::{
    Error, Result,
    buf::{Reader, Writer},
};
use horizon_world::registry::BlockStateId;
use std::{collections::HashMap, time::Instant};

mod client;
mod server;
pub use client::{Inbox, RemoteSource};
pub use server::{Delivery, Job, Service, Verdict};

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "horizon:lod";
/// The server.properties key (Horizon's own): the farthest a player's far
/// terrain may reach, in chunks; 0 turns serving it off.
pub const PROPERTY: &str = "horizon-lod-distance";
/// [`PROPERTY`] when the file doesn't set it: off.
pub const DEFAULT_DISTANCE: u32 = 0;
/// The protocol's version, first in an [`Offer`]: a client serves itself
/// from a server that speaks its own only. Requests and replies follow it.
pub const VERSION: u32 = 1;
/// The most [`PROPERTY`] may be (the client's own LOD Distance cap).
pub const MAX_DISTANCE: u32 = 1024;
/// Requests a player may have waiting at the server at once.
pub const IN_FLIGHT: u32 = 8;
/// Tiles a second a player may ask for, on average.
pub const PER_SECOND: u32 = 64;
/// Columns on a tile's side (`lod::CELLS`).
pub const SIDE: usize = 16;
/// How much farther than the allowed distance a tile may be, in chunks: the
/// player moved between the client's request and the server's check.
const SLACK: u32 = 8;
/// Biomes one tile may name at most.
const MAX_BIOMES: usize = 256;
/// A biome name's length at most.
const MAX_BIOME_NAME: usize = 256;
/// Spans one tile may hold at most, all its columns together (a mean of 64
/// a column): a bigger tile is refused, so what the client decodes and
/// meshes stays small.
pub const MAX_SPANS: usize = 16 * 1024;
/// A reply's size at most, in bytes.
pub const MAX_REPLY: usize = 512 * 1024;
/// The lowest and highest a span or surface may reach (a dimension's limits).
const HEIGHTS: std::ops::RangeInclusive<i32> = -2048..=4096;

/// [`PROPERTY`]'s value: [`DEFAULT_DISTANCE`] when absent or unreadable,
/// at most [`MAX_DISTANCE`].
pub fn distance_property(value: Option<&str>) -> u32 {
    value
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_DISTANCE)
        .min(MAX_DISTANCE)
}

/// What the server serves, as it tells a Horizon client while it configures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    /// Chunks from the player at most.
    pub distance: u32,
    /// Requests waiting at once at most.
    pub in_flight: u32,
    /// Tiles a second at most.
    pub per_second: u32,
}
impl Offer {
    pub fn new(distance: u32) -> Self {
        Self {
            distance: distance.min(MAX_DISTANCE),
            in_flight: IN_FLIGHT,
            per_second: PER_SECOND,
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(VERSION as i32);
        w.varint(self.distance as i32);
        w.varint(self.in_flight as i32);
        w.varint(self.per_second as i32);
        w.data
    }
    /// An offer, None when it serves nothing or speaks another version.
    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let mut next = || r.varint().ok().and_then(|v| u32::try_from(v).ok());
        if next()? != VERSION {
            return None;
        }
        let offer = Self {
            distance: next()?.min(MAX_DISTANCE),
            in_flight: next()?.min(64),
            per_second: next()?,
        };
        (offer.distance > 0 && offer.in_flight > 0 && offer.per_second > 0).then_some(offer)
    }
}

/// One far tile: [`SIDE`] x [`SIDE`] columns from block (`x`, `z`), each
/// `stride` blocks wide, as `LodSampler::grid` samples them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u32,
    pub x: i32,
    pub z: i32,
    pub stride: i32,
    /// Reduced far detail (`LodSampler::set_approximate`).
    pub approximate: bool,
}
impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(self.id as i32);
        w.i32(self.x);
        w.i32(self.z);
        w.varint(self.stride);
        w.bool(self.approximate);
        w.data
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let request = Self {
            id: r.varint()? as u32,
            x: r.i32()?,
            z: r.i32()?,
            stride: r.varint()?,
            approximate: r.bool()?,
        };
        if r.remaining() > 0 {
            return Err(Error::new("far terrain request: trailing bytes"));
        }
        Ok(request)
    }
    /// Whether the server serves it to a player at `position` when it allows
    /// `distance` chunks: a whole tile (aligned, a power-of-two stride no
    /// coarser than the client's coarsest tile at that distance) whose
    /// nearest point is within the distance.
    pub fn allowed(&self, distance: u32, position: [f64; 3]) -> bool {
        let coarsest = distance.next_power_of_two().min(MAX_DISTANCE) as i32;
        if distance == 0
            || self.stride < 1
            || self.stride > coarsest
            || self.stride.count_ones() != 1
        {
            return false;
        }
        let size = self.stride * SIDE as i32;
        if self.x.rem_euclid(size) != 0 || self.z.rem_euclid(size) != 0 {
            return false;
        }
        let gap = |c: f64, lo: i32| {
            (f64::from(lo) - c)
                .max(c - f64::from(lo) - f64::from(size))
                .max(0.0)
        };
        let (dx, dz) = (gap(position[0], self.x), gap(position[2], self.z));
        let reach = f64::from((distance + SLACK) * 16);
        dx * dx + dz * dz <= reach * reach
    }
}

/// A tile's columns as the server sends them: the request's id, then 0 and
/// the columns' biome names and columns, or 1 for a refusal. A tile the
/// client would refuse (past [`MAX_SPANS`] or [`MAX_REPLY`]) is refused.
pub fn encode_reply(id: u32, columns: Option<&[Column]>, biome_names: &[String]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id as i32);
    let spans: usize = columns.map_or(0, |c| c.iter().map(|c| c.spans.len()).sum());
    let Some(columns) = columns.filter(|_| spans <= MAX_SPANS) else {
        w.u8(1);
        return w.data;
    };
    w.u8(0);
    let mut palette: Vec<u16> = Vec::new();
    let mut local = |biome: u16| -> i32 {
        let at = palette.iter().position(|&b| b == biome).unwrap_or_else(|| {
            palette.push(biome);
            palette.len() - 1
        });
        at as i32
    };
    let mut body = Writer::new();
    body.varint(columns.len() as i32);
    for column in columns {
        match &column.surface {
            Some(s) => {
                body.bool(true);
                body.i32(s.height);
                body.varint(s.state.0 as i32);
                match s.water {
                    Some((height, state)) => {
                        body.bool(true);
                        body.i32(height);
                        body.varint(state.0 as i32);
                    }
                    None => body.bool(false),
                }
                body.varint(local(s.biome));
            }
            None => body.bool(false),
        }
        body.varint(column.spans.len() as i32);
        for span in &column.spans {
            body.i16(span.min);
            body.i16(span.max);
            body.varint(span.state.0 as i32);
            body.varint(local(span.biome));
            body.u16(span.light);
        }
    }
    if palette.len() > MAX_BIOMES {
        return refusal(id);
    }
    w.varint(palette.len() as i32);
    for biome in palette {
        w.string(
            biome_names
                .get(usize::from(biome))
                .filter(|name| name.len() <= MAX_BIOME_NAME)
                .map_or("minecraft:plains", String::as_str),
        );
    }
    w.bytes(&body.data);
    if w.data.len() > MAX_REPLY {
        return refusal(id);
    }
    w.data
}

/// A refusal's payload: the tile isn't served.
pub fn refusal(id: u32) -> Vec<u8> {
    encode_reply(id, None, &[])
}

/// The id a reply answers.
pub fn reply_id(data: &[u8]) -> Option<u32> {
    Reader::new(data).varint().ok().map(|id| id as u32)
}

/// A reply's columns (None: refused), biomes by this client's indices
/// (unknown ones as the first), block states below `states`. A reply a
/// Horizon server doesn't send is an error before it costs memory: too big,
/// too many spans, spans out of order or overlapping, heights out of range.
pub fn decode_reply(
    data: &[u8],
    biomes: &HashMap<String, u16>,
    states: u32,
) -> Result<Option<Vec<Column>>> {
    if data.len() > MAX_REPLY {
        return Err(Error::new("far terrain: reply too big"));
    }
    let mut r = Reader::new(data);
    r.varint()?;
    if r.u8()? != 0 {
        return Ok(None);
    }
    let count = r.varint()?;
    if !(0..=MAX_BIOMES as i32).contains(&count) {
        return Err(Error::new("far terrain: too many biomes"));
    }
    let palette: Vec<u16> = (0..count)
        .map(|_| Ok(biomes.get(&r.string(MAX_BIOME_NAME)?).copied().unwrap_or(0)))
        .collect::<Result<_>>()?;
    let biome = |r: &mut Reader| -> Result<u16> {
        palette
            .get(r.varint()? as usize)
            .copied()
            .ok_or_else(|| Error::new("far terrain: biome out of the palette"))
    };
    let state = |r: &mut Reader| -> Result<BlockStateId> {
        let id = r.varint()? as u32;
        if id >= states {
            return Err(Error::new("far terrain: unknown block state"));
        }
        Ok(BlockStateId(id))
    };
    let columns = r.varint()?;
    if columns != (SIDE * SIDE) as i32 {
        return Err(Error::new("far terrain: not a whole tile"));
    }
    let height = |r: &mut Reader| -> Result<i32> {
        let y = r.i32()?;
        if !HEIGHTS.contains(&y) {
            return Err(Error::new("far terrain: height out of range"));
        }
        Ok(y)
    };
    let mut out = Vec::with_capacity(SIDE * SIDE);
    let mut total = 0;
    for _ in 0..columns {
        let surface = if r.bool()? {
            let height_ = height(&mut r)?;
            let top = state(&mut r)?;
            let water = if r.bool()? {
                Some((height(&mut r)?, state(&mut r)?))
            } else {
                None
            };
            Some(Surface {
                height: height_,
                state: top,
                water,
                biome: biome(&mut r)?,
            })
        } else {
            None
        };
        let count =
            usize::try_from(r.varint()?).map_err(|_| Error::new("far terrain: too many spans"))?;
        total += count;
        if total > MAX_SPANS {
            return Err(Error::new("far terrain: too many spans"));
        }
        let mut spans = Spans::with_capacity(count);
        for _ in 0..count {
            let span = Span {
                min: r.i16()?,
                max: r.i16()?,
                state: state(&mut r)?,
                biome: biome(&mut r)?,
                light: r.u16()?,
            };
            let floor = spans.last().map_or(*HEIGHTS.start(), |s| i32::from(s.max));
            if i32::from(span.min) < floor
                || span.min >= span.max
                || !HEIGHTS.contains(&i32::from(span.max))
            {
                return Err(Error::new("far terrain: spans out of order"));
            }
            spans.push(span);
        }
        out.push(Column { surface, spans });
    }
    Ok(Some(out))
}

/// A token bucket: `rate` a second, `burst` at once.
#[derive(Clone, Debug)]
pub struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    at: Instant,
}
impl Bucket {
    pub fn new(rate: f64, burst: f64, now: Instant) -> Self {
        Self {
            tokens: burst,
            rate,
            burst,
            at: now,
        }
    }
    /// Takes a token when there is one.
    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile() -> Vec<Column> {
        (0..SIDE * SIDE)
            .map(|i| Column {
                surface: (i % 3 != 0).then_some(Surface {
                    height: 60 + i as i32 % 7,
                    state: BlockStateId(9),
                    water: (i % 5 == 0).then_some((62, BlockStateId(86))),
                    biome: (i % 4) as u16,
                }),
                spans: Spans::from(vec![Span {
                    min: -64,
                    max: 60,
                    state: BlockStateId(1),
                    biome: (i % 2) as u16,
                    light: 240,
                }]),
            })
            .collect()
    }

    #[test]
    fn a_tile_crosses_by_biome_name() {
        let server: Vec<String> = ["a", "b", "c", "d"].map(String::from).into();
        let data = encode_reply(7, Some(&tile()), &server);
        assert_eq!(reply_id(&data), Some(7));
        // The client lists the same biomes in another order.
        let client: HashMap<String, u16> = [("d", 0), ("c", 1), ("b", 2), ("a", 3)]
            .map(|(n, i)| (n.to_owned(), i))
            .into();
        let got = decode_reply(&data, &client, 100).unwrap().unwrap();
        let back = |b: u16| 3 - b;
        for (sent, got) in tile().iter().zip(&got) {
            assert_eq!(
                sent.surface.map(|s| s.height),
                got.surface.map(|s| s.height)
            );
            assert_eq!(sent.surface.map(|s| s.water), got.surface.map(|s| s.water));
            assert_eq!(
                sent.surface.map(|s| s.biome),
                got.surface.map(|s| back(s.biome))
            );
            assert_eq!(sent.spans[0].biome, back(got.spans[0].biome));
            assert_eq!(sent.spans[0].state, got.spans[0].state);
        }
        // States the client doesn't have are refused, not drawn.
        assert!(decode_reply(&data, &client, 50).is_err());
        assert_eq!(decode_reply(&refusal(3), &client, 100).unwrap(), None);
    }

    #[test]
    fn hostile_replies_are_refused_before_they_cost_memory() {
        let names: Vec<String> = ["a"].map(String::from).into();
        let client: HashMap<String, u16> = [("a".to_owned(), 0)].into();
        let span = |min, max| Span {
            min,
            max,
            state: BlockStateId(1),
            biome: 0,
            light: 0,
        };
        let with = |spans: Vec<Span>| {
            let mut tile = vec![Column::default(); SIDE * SIDE];
            tile[0].spans = spans.into();
            encode_reply(1, Some(&tile), &names)
        };
        assert!(decode_reply(&with(vec![span(0, 4), span(4, 9)]), &client, 9).is_ok());
        // Overlapping, out of order, empty or inverted spans.
        for spans in [
            vec![span(0, 4), span(2, 9)],
            vec![span(4, 9), span(0, 4)],
            vec![span(3, 3)],
            vec![span(-4000, 0)],
        ] {
            assert!(decode_reply(&with(spans), &client, 9).is_err());
        }
        // A tile past the span cap is refused by the server and by the client.
        let many: Vec<Span> = (0..=MAX_SPANS as i16 / 4)
            .map(|i| span(i - 2048, i - 2047))
            .collect();
        let mut tile = vec![Column::default(); SIDE * SIDE];
        for column in &mut tile[..4] {
            column.spans = many.clone().into();
        }
        assert_eq!(
            decode_reply(&with(Vec::new()), &client, 9)
                .unwrap()
                .unwrap()
                .len(),
            256
        );
        assert_eq!(
            decode_reply(&encode_reply(2, Some(&tile), &names), &client, 9).unwrap(),
            None
        );
        let mut forged = Writer::new();
        forged.varint(2);
        forged.u8(0);
        forged.varint(1);
        forged.string("a");
        forged.varint(256);
        for i in 0..256 {
            forged.bool(false);
            forged.varint(if i < 4 { many.len() as i32 } else { 0 });
            for s in many.iter().take(if i < 4 { many.len() } else { 0 }) {
                forged.i16(s.min);
                forged.i16(s.max);
                forged.varint(1);
                forged.varint(0);
                forged.u16(0);
            }
        }
        let err = decode_reply(&forged.data, &client, 9).unwrap_err();
        assert!(err.to_string().contains("too many spans"), "{err}");
        assert!(decode_reply(&vec![0; MAX_REPLY + 1], &client, 9).is_err());
    }

    #[test]
    fn requests_and_offers_round_trip() {
        let r = Request {
            id: 300,
            x: -512,
            z: 4096,
            stride: 2,
            approximate: true,
        };
        assert_eq!(Request::decode(&r.encode()).unwrap(), r);
        let offer = Offer::new(96);
        assert_eq!(Offer::decode(&offer.encode()), Some(offer));
        assert_eq!(Offer::decode(&Offer::new(0).encode()), None);
        // Another version's offer is ignored.
        let mut other = offer.encode();
        other[0] = VERSION as u8 + 1;
        assert_eq!(Offer::decode(&other), None);
        // Off unless the owner sets it.
        assert_eq!(distance_property(None), 0);
        assert_eq!(distance_property(Some(" 64 ")), 64);
        assert_eq!(distance_property(Some("5000")), MAX_DISTANCE);
        assert_eq!(distance_property(Some("far")), 0);
    }

    #[test]
    fn only_tiles_within_the_distance_are_served() {
        let at = [8.0, 70.0, 8.0];
        let tile = |x: i32, z: i32, stride: i32| Request {
            id: 0,
            x,
            z,
            stride,
            approximate: false,
        };
        // 64 chunks, 8 more for movement: 1152 blocks.
        assert!(tile(1136, 0, 1).allowed(64, at));
        assert!(!tile(1168, 0, 1).allowed(64, at));
        assert!(!tile(1136, 1136, 1).allowed(64, at));
        // The coarsest tile 64 chunks plan: 64 blocks a cell.
        assert!(tile(0, 0, 64).allowed(64, at));
        assert!(!tile(0, 0, 128).allowed(64, at));
        // Whole tiles only, power-of-two strides.
        assert!(!tile(8, 0, 1).allowed(64, at));
        assert!(!tile(0, 0, 3).allowed(64, at));
        assert!(!tile(0, 0, 0).allowed(64, at));
        assert!(!tile(0, 0, -2).allowed(64, at));
        assert!(!tile(0, 0, 1).allowed(0, at));
    }
}
