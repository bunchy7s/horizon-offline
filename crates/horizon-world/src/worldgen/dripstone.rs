//! `minecraft:speleothem_cluster` and `minecraft:large_dripstone`
//! (SpeleothemClusterFeature, LargeDripstoneFeature and SpeleothemUtils).
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    int_range, number_text, optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
    tree::{TreeDecoder, TreeResult, kind, read, scan_column, unit_float, write},
    tree_blocks::{self as tb, TreeBlocks},
    value_provider::{FloatProvider, IntProvider},
};
use crate::{heightmap::HeightmapType, registry::BlockStateId};
use horizon_core::{geometry::trig, random::RandomSource};
use std::{collections::BTreeSet, sync::Arc};

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}
/// Column.scan result: optional floor and ceiling edge heights.
#[derive(Clone, Copy, Debug)]
struct Column {
    floor: Option<i32>,
    ceiling: Option<i32>,
}
impl Column {
    fn height(&self) -> Option<i32> {
        match (self.floor, self.ceiling) {
            (Some(floor), Some(ceiling)) => Some(ceiling - floor - 1),
            _ => None,
        }
    }
}

/// Shared speleothem block classification.
#[derive(Clone, Debug)]
struct Speleothem {
    base: BlockStateId,
    pointed: BlockStateId,
    replaceable: Arc<BTreeSet<BlockStateId>>,
    lava: BlockStateId,
    blocks: Arc<TreeBlocks>,
}
impl Speleothem {
    fn new(
        bindings: &FeatureBlockBindings<'_>,
        base: BlockStateId,
        pointed: BlockStateId,
        replaceable: Arc<BTreeSet<BlockStateId>>,
    ) -> Result<Self, DecodeError> {
        let registry = bindings.registry();
        let default = |state: BlockStateId| {
            registry
                .default_state(&registry.state(state).unwrap().name)
                .ok_or(DecodeError::Field("speleothem block"))
        };
        Ok(Self {
            base: default(base)?,
            pointed: default(pointed)?,
            replaceable,
            lava: registry
                .default_state("minecraft:lava")
                .ok_or(DecodeError::Field("lava"))?,
            blocks: bindings.tree_blocks()?,
        })
    }
    fn empty_or_water(&self, s: BlockStateId) -> bool {
        self.blocks.is_air(s) || self.blocks.has(s, tb::WATER_BLOCK)
    }
    fn empty_water_or_lava(&self, s: BlockStateId) -> bool {
        self.empty_or_water(s) || self.blocks.same_block(s, self.lava)
    }
    fn is_base(&self, s: BlockStateId) -> bool {
        self.blocks.same_block(s, self.base) || self.replaceable.contains(&s)
    }
    fn scan<W: FeatureWorld>(
        &self,
        world: &mut W,
        pos: [i32; 3],
        range: i32,
        edge: impl Fn(&Self, BlockStateId) -> bool,
    ) -> TreeResult<Option<Column>, W::Error> {
        if !self.empty_or_water(read(world, pos)?) {
            return Ok(None);
        }
        // Column.scan: up to range - 1 empty blocks from pos, then the edge.
        let mut scan = |step: i32| -> TreeResult<Option<i32>, W::Error> {
            let y = scan_column(world, pos, step, range - 1, |s| self.empty_or_water(s))?;
            let p = [pos[0], y, pos[2]];
            Ok(edge(self, read(world, p)?).then_some(y))
        };
        let ceiling = scan(1)?;
        let floor = scan(-1)?;
        Ok(Some(Column { floor, ceiling }))
    }
    fn pointed_state(&self, up: bool, thickness: &str) -> Option<BlockStateId> {
        let state = self.blocks.with(
            self.pointed,
            "vertical_direction",
            if up { "up" } else { "down" },
        )?;
        self.blocks.with(state, "thickness", thickness)
    }
    /// SpeleothemUtils.growSpeleothem.
    #[allow(clippy::too_many_arguments)]
    fn grow<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        start: [i32; 3],
        up: bool,
        height: i32,
        merged: bool,
    ) -> TreeResult<(), W::Error> {
        let step = if up { 1 } else { -1 };
        if !self.is_base(read(world, tb::offset(start, 0, -step, 0))?) {
            return Ok(());
        }
        let mut sequence = Vec::new();
        if height >= 3 {
            sequence.push("base");
            sequence.extend(std::iter::repeat_n("middle", (height - 3) as usize));
        }
        if height >= 2 {
            sequence.push("frustum");
        }
        if height >= 1 {
            sequence.push(if merged { "tip_merge" } else { "tip" });
        }
        let mut p = start;
        for thickness in sequence {
            let mut state = self
                .pointed_state(up, thickness)
                .ok_or(FeatureExecutionError::InvalidState)?;
            let water = self.blocks.has(read(world, p)?, tb::WATER_FLUID);
            state = self.blocks.waterlogged(state, water);
            write(world, p, state, 2)?;
            p[1] = p[1].wrapping_add(step);
        }
        Ok(())
    }
    fn place_base_if_possible<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if self.replaceable.contains(&read(world, p)?) {
            write(world, p, self.base, 2)?;
            return Ok(true);
        }
        Ok(false)
    }
}

fn float_provider(
    raw: &serde_json::value::RawValue,
    low: f32,
    high: f32,
) -> Result<FloatProvider, DecodeError> {
    let value = FloatProvider::from_json(raw.get())?;
    let [min, max] = value.bounds();
    if min < low || max > high {
        return Err(DecodeError::Field("float provider bounds"));
    }
    Ok(value)
}
fn float_range(
    raw: &serde_json::value::RawValue,
    name: &'static str,
    low: f32,
    high: f32,
) -> Result<f32, DecodeError> {
    let value: f32 = number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field(name))?;
    if !(low..=high).contains(&value) {
        return Err(DecodeError::Field(name));
    }
    Ok(value)
}
fn block_state(
    bindings: &FeatureBlockBindings<'_>,
    raw: &serde_json::value::RawValue,
) -> Result<BlockStateId, DecodeError> {
    bindings
        .codec
        .decode(raw.get(), false)
        .map_err(|e| DecodeError::Registry(e.to_string()))
}
fn clamped_lerp(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}
fn clamped_lerp_f32(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

#[derive(Clone, Debug)]
pub struct SpeleothemClusterFeature {
    speleothem: Speleothem,
    search_range: i32,
    height: IntProvider,
    radius: IntProvider,
    max_height_diff: i32,
    height_deviation: i32,
    layer_thickness: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    chance_at_max_distance: f32,
    max_distance_from_edge: i32,
    max_distance_from_center: i32,
}
impl SpeleothemClusterFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:speleothem_cluster" {
            return Err(DecodeError::Field("speleothem cluster type"));
        }
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        let int = |name: &'static str, low: i32, high: i32| {
            int_range(required(&object, name)?, name, low, high)
        };
        Ok(Self {
            speleothem: Speleothem::new(
                bindings,
                block_state(bindings, required(&object, "base_block")?)?,
                block_state(bindings, required(&object, "pointed_block")?)?,
                decoder.block_set(required(&object, "replaceable_blocks")?)?,
            )?,
            search_range: int("floor_to_ceiling_search_range", 1, 512)?,
            height: decoder.int_provider(required(&object, "height")?, 1, 128)?,
            radius: decoder.int_provider(required(&object, "radius")?, 1, 128)?,
            max_height_diff: int("max_stalagmite_stalactite_height_diff", 0, 64)?,
            height_deviation: int("height_deviation", 1, 64)?,
            layer_thickness: decoder.int_provider(
                required(&object, "speleothem_block_layer_thickness")?,
                0,
                128,
            )?,
            density: float_provider(required(&object, "density")?, 0.0, 2.0)?,
            wetness: float_provider(required(&object, "wetness")?, 0.0, 2.0)?,
            chance_at_max_distance: unit_float(
                required(&object, "chance_of_speleothem_at_max_distance_from_center")?,
                "chance_of_speleothem_at_max_distance_from_center",
            )?,
            max_distance_from_edge: int(
                "max_distance_from_edge_affecting_chance_of_speleothem",
                1,
                64,
            )?,
            max_distance_from_center: int("max_distance_from_center_affecting_height_bias", 1, 64)?,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if !self.speleothem.empty_or_water(read(world, origin)?) {
            return Ok(false);
        }
        let height = self.height.sample(random)?;
        let wetness = self.wetness.sample(random);
        let density = self.density.sample(random);
        let x_radius = self.radius.sample(random)?;
        let z_radius = self.radius.sample(random)?;
        for dx in -x_radius..=x_radius {
            for dz in -z_radius..=z_radius {
                let distance = (x_radius - dx.abs()).min(z_radius - dz.abs());
                let chance = f64::from(clamped_lerp_f32(
                    (distance as f32 - 0.0) / (self.max_distance_from_edge as f32 - 0.0),
                    self.chance_at_max_distance,
                    1.0,
                ));
                let p = tb::offset(origin, dx, 0, dz);
                self.column(world, random, p, dx, dz, wetness, chance, height, density)?;
            }
        }
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    fn column<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        pos: [i32; 3],
        dx: i32,
        dz: i32,
        chance_of_water: f32,
        chance: f64,
        cluster_height: i32,
        density: f32,
    ) -> TreeResult<(), W::Error> {
        let s = &self.speleothem;
        let Some(base_column) =
            s.scan(world, pos, self.search_range, |s, st| !s.empty_or_water(st))?
        else {
            return Ok(());
        };
        let ceiling = base_column.ceiling;
        let base_floor = base_column.floor;
        if ceiling.is_none() && base_floor.is_none() {
            return Ok(());
        }
        let want_pool = random.next_float() < chance_of_water;
        let column = match base_floor {
            Some(floor_y)
                if want_pool && self.can_place_pool(world, [pos[0], floor_y, pos[2]])? =>
            {
                let water = s
                    .blocks
                    .default_state("minecraft:water")
                    .ok_or(FeatureExecutionError::InvalidState)?;
                write(world, [pos[0], floor_y, pos[2]], water, 2)?;
                Column {
                    floor: Some(floor_y - 1),
                    ceiling,
                }
            }
            _ => base_column,
        };
        let floor = column.floor;
        let want_stalactite = random.next_double() < chance;
        let stalactite_height = match ceiling {
            Some(c)
                if want_stalactite
                    && !s
                        .blocks
                        .same_block(read(world, [pos[0], c, pos[2]])?, s.lava) =>
            {
                let thickness = self.layer_thickness.sample(random)?;
                self.replace_with_base(world, [pos[0], c, pos[2]], thickness, 1)?;
                let max = match floor {
                    Some(f) => cluster_height.min(c - f),
                    None => cluster_height,
                };
                self.speleothem_height(random, dx, dz, density, max)?
            }
            _ => 0,
        };
        let want_stalagmite = random.next_double() < chance;
        let stalagmite_height = match floor {
            Some(f)
                if want_stalagmite
                    && !s
                        .blocks
                        .same_block(read(world, [pos[0], f, pos[2]])?, s.lava) =>
            {
                let thickness = self.layer_thickness.sample(random)?;
                self.replace_with_base(world, [pos[0], f, pos[2]], thickness, -1)?;
                if ceiling.is_some() {
                    let diff = self.max_height_diff;
                    0.max(stalactite_height + random.next_int_bound(diff * 2 + 1)? - diff)
                } else {
                    self.speleothem_height(random, dx, dz, density, cluster_height)?
                }
            }
            _ => 0,
        };
        let (actual_stalactite, actual_stalagmite) = match (ceiling, floor) {
            (Some(c), Some(f)) if c - stalactite_height <= f + stalagmite_height => {
                let lowest_bottom = (c - stalactite_height).max(f + 1);
                let highest_top = (f + stalagmite_height).min(c - 1);
                let bottom =
                    random.next_int_bound(highest_top + 1 - lowest_bottom + 1)? + lowest_bottom;
                (c - bottom, bottom - 1 - f)
            }
            _ => (stalactite_height, stalagmite_height),
        };
        let merge = random.next_boolean()
            && actual_stalactite > 0
            && actual_stalagmite > 0
            && column
                .height()
                .is_some_and(|h| actual_stalactite + actual_stalagmite == h);
        if let Some(c) = ceiling {
            s.grow(
                world,
                [pos[0], c - 1, pos[2]],
                false,
                actual_stalactite,
                merge,
            )?;
        }
        if let Some(f) = floor {
            s.grow(
                world,
                [pos[0], f + 1, pos[2]],
                true,
                actual_stalagmite,
                merge,
            )?;
        }
        Ok(())
    }
    fn speleothem_height<E>(
        &self,
        random: &mut impl RandomSource,
        dx: i32,
        dz: i32,
        density: f32,
        max_height: i32,
    ) -> TreeResult<i32, E> {
        if random.next_float() > density {
            return Ok(0);
        }
        let distance = dx.abs() + dz.abs();
        let mean = clamped_lerp(
            (f64::from(distance) - 0.0) / (f64::from(self.max_distance_from_center) - 0.0),
            f64::from(max_height) / 2.0,
            0.0,
        ) as f32;
        let value = mean + random.next_gaussian() as f32 * self.height_deviation as f32;
        let clamped = if value < 0.0 {
            0.0
        } else {
            value.min(max_height as f32)
        };
        Ok(clamped as i32)
    }
    fn can_place_pool<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let s = &self.speleothem;
        let state = read(world, p)?;
        if s.blocks.has(state, tb::WATER_BLOCK)
            || s.blocks.same_block(state, s.base)
            || s.blocks.same_block(state, s.pointed)
        {
            return Ok(false);
        }
        if s.blocks
            .has(read(world, tb::offset(p, 0, 1, 0))?, tb::WATER_FLUID)
        {
            return Ok(false);
        }
        for direction in tb::HORIZONTAL {
            if !self.adjacent_ok(world, tb::relative(p, direction, 1))? {
                return Ok(false);
            }
        }
        self.adjacent_ok(world, tb::offset(p, 0, -1, 0))
    }
    fn adjacent_ok<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let state = read(world, p)?;
        let b = &self.speleothem.blocks;
        Ok(b.has(state, tb::BASE_STONE_OVERWORLD) || b.has(state, tb::WATER_FLUID))
    }
    fn replace_with_base<W: FeatureWorld>(
        &self,
        world: &mut W,
        first: [i32; 3],
        count: i32,
        step: i32,
    ) -> TreeResult<(), W::Error> {
        let mut p = first;
        for _ in 0..count {
            if !self.speleothem.place_base_if_possible(world, p)? {
                return Ok(());
            }
            p[1] = p[1].wrapping_add(step);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct LargeDripstoneFeature {
    speleothem: Speleothem,
    search_range: i32,
    column_radius: IntProvider,
    height_scale: FloatProvider,
    max_radius_ratio: f32,
    stalactite_bluntness: FloatProvider,
    stalagmite_bluntness: FloatProvider,
    wind_speed: FloatProvider,
    min_radius_for_wind: i32,
    min_bluntness_for_wind: f32,
}
struct LargeDripstone {
    root: [i32; 3],
    up: bool,
    radius: i32,
    bluntness: f64,
    scale: f64,
}
#[derive(Clone, Copy)]
struct Wind {
    origin_y: i32,
    speed: Option<[f64; 3]>,
    max_offset: i32,
}
impl Wind {
    fn offset(&self, p: [i32; 3]) -> [i32; 3] {
        let Some(speed) = self.speed else {
            return p;
        };
        let dy = f64::from(self.origin_y.wrapping_sub(p[1]));
        let clamp = |v: f64| {
            let v = v.floor() as i32;
            if v < -self.max_offset {
                -self.max_offset
            } else {
                v.min(self.max_offset)
            }
        };
        tb::offset(p, clamp(speed[0] * dy), 0, clamp(speed[2] * dy))
    }
}
fn speleothem_height(distance: f64, radius: f64, scale: f64, bluntness: f64) -> f64 {
    let distance = if distance < bluntness {
        bluntness
    } else {
        distance
    };
    let r = distance / radius * 0.384;
    let part1 = 0.75 * horizon_core::jmath::pow(r, 1.3333333333333333);
    let part2 = horizon_core::jmath::pow(r, 0.6666666666666666);
    let part3 = 0.3333333333333333 * horizon_core::jmath::log(r);
    let height = (scale * (part1 - part2 - part3)).max(0.0);
    height / 0.384 * radius
}
impl LargeDripstone {
    fn height_at(&self, radius: f32) -> i32 {
        speleothem_height(
            f64::from(radius),
            f64::from(self.radius),
            self.scale,
            self.bluntness,
        ) as i32
    }
}
impl LargeDripstoneFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:large_dripstone" {
            return Err(DecodeError::Field("large dripstone type"));
        }
        let registry = bindings.registry();
        let dripstone = registry
            .default_state("minecraft:dripstone_block")
            .ok_or(DecodeError::Field("dripstone block"))?;
        let pointed = registry
            .default_state("minecraft:pointed_dripstone")
            .ok_or(DecodeError::Field("pointed dripstone"))?;
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        Ok(Self {
            speleothem: Speleothem::new(
                bindings,
                dripstone,
                pointed,
                decoder.block_set(required(&object, "replaceable_blocks")?)?,
            )?,
            search_range: optional(&object, "floor_to_ceiling_search_range")
                .map(|raw| int_range(raw, "floor_to_ceiling_search_range", 1, 512))
                .transpose()?
                .unwrap_or(30),
            column_radius: decoder.int_provider(required(&object, "column_radius")?, 1, 16)?,
            height_scale: float_provider(required(&object, "height_scale")?, 0.0, 20.0)?,
            max_radius_ratio: float_range(
                required(&object, "max_column_radius_to_cave_height_ratio")?,
                "max_column_radius_to_cave_height_ratio",
                0.1,
                1.0,
            )?,
            stalactite_bluntness: float_provider(
                required(&object, "stalactite_bluntness")?,
                0.1,
                10.0,
            )?,
            stalagmite_bluntness: float_provider(
                required(&object, "stalagmite_bluntness")?,
                0.1,
                10.0,
            )?,
            wind_speed: float_provider(required(&object, "wind_speed")?, 0.0, 2.0)?,
            min_radius_for_wind: int_range(
                required(&object, "min_radius_for_wind")?,
                "min_radius_for_wind",
                0,
                100,
            )?,
            min_bluntness_for_wind: float_range(
                required(&object, "min_bluntness_for_wind")?,
                "min_bluntness_for_wind",
                0.0,
                5.0,
            )?,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let s = &self.speleothem;
        if !s.empty_or_water(read(world, origin)?) {
            return Ok(false);
        }
        let Some(column) = s.scan(world, origin, self.search_range, |s, st| {
            s.is_base(st) || s.blocks.same_block(st, s.lava)
        })?
        else {
            return Ok(false);
        };
        let (Some(floor), Some(ceiling)) = (column.floor, column.ceiling) else {
            return Ok(false);
        };
        let height = ceiling - floor - 1;
        if height < 4 {
            return Ok(false);
        }
        let by_height = (height as f32 * self.max_radius_ratio) as i32;
        let [min_r, max_r] = self.column_radius.bounds();
        let max_radius = if by_height < min_r {
            min_r
        } else {
            by_height.min(max_r)
        };
        let radius = random.next_int_bound(max_radius - min_r + 1)? + min_r;
        let mut stalactite = LargeDripstone {
            root: [origin[0], ceiling - 1, origin[2]],
            up: false,
            radius,
            bluntness: f64::from(self.stalactite_bluntness.sample(random)),
            scale: f64::from(self.height_scale.sample(random)),
        };
        let mut stalagmite = LargeDripstone {
            root: [origin[0], floor + 1, origin[2]],
            up: true,
            radius,
            bluntness: f64::from(self.stalagmite_bluntness.sample(random)),
            scale: f64::from(self.height_scale.sample(random)),
        };
        let suitable = |d: &LargeDripstone| {
            d.radius >= self.min_radius_for_wind
                && d.bluntness >= f64::from(self.min_bluntness_for_wind)
        };
        let wind = if suitable(&stalactite) && suitable(&stalagmite) {
            let speed = self.wind_speed.sample(random);
            let direction = random.next_float() * (std::f32::consts::PI - 0.0) + 0.0;
            Wind {
                origin_y: origin[1],
                speed: Some([
                    f64::from(trig::cos(f64::from(direction)) * speed),
                    0.0,
                    f64::from(trig::sin(f64::from(direction)) * speed),
                ]),
                max_offset: 16 - radius,
            }
        } else {
            Wind {
                origin_y: 0,
                speed: None,
                max_offset: 0,
            }
        };
        let top = self.move_back(world, &mut stalactite, wind)?;
        let bottom = self.move_back(world, &mut stalagmite, wind)?;
        if top {
            self.place_blocks(world, random, &stalactite, wind)?;
        }
        if bottom {
            self.place_blocks(world, random, &stalagmite, wind)?;
        }
        Ok(true)
    }
    fn move_back<W: FeatureWorld>(
        &self,
        world: &mut W,
        d: &mut LargeDripstone,
        wind: Wind,
    ) -> TreeResult<bool, W::Error> {
        let s = &self.speleothem;
        while d.radius > 1 {
            let mut root = d.root;
            let tries = 10.min(d.height_at(0.0));
            for _ in 0..tries {
                if s.blocks.same_block(read(world, root)?, s.lava) {
                    return Ok(false);
                }
                if self.circle_embedded(world, wind.offset(root), d.radius)? {
                    d.root = root;
                    return Ok(true);
                }
                root[1] = root[1].wrapping_add(if d.up { -1 } else { 1 });
            }
            d.radius /= 2;
        }
        Ok(false)
    }
    fn circle_embedded<W: FeatureWorld>(
        &self,
        world: &mut W,
        center: [i32; 3],
        radius: i32,
    ) -> TreeResult<bool, W::Error> {
        let s = &self.speleothem;
        if s.empty_water_or_lava(read(world, center)?) {
            return Ok(false);
        }
        let increment = 6.0f32 / radius as f32;
        let mut angle = 0.0f32;
        while angle < (std::f64::consts::PI * 2.0) as f32 {
            let dx = (trig::cos(f64::from(angle)) * radius as f32) as i32;
            let dz = (trig::sin(f64::from(angle)) * radius as f32) as i32;
            if s.empty_water_or_lava(read(world, tb::offset(center, dx, 0, dz))?) {
                return Ok(false);
            }
            angle += increment;
        }
        Ok(true)
    }
    fn place_blocks<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        d: &LargeDripstone,
        wind: Wind,
    ) -> TreeResult<(), W::Error> {
        let s = &self.speleothem;
        for dx in -d.radius..=d.radius {
            for dz in -d.radius..=d.radius {
                let current = ((dx * dx + dz * dz) as f32).sqrt();
                if current > d.radius as f32 {
                    continue;
                }
                let mut height = d.height_at(current);
                if height <= 0 {
                    continue;
                }
                if f64::from(random.next_float()) < 0.2 {
                    let factor = random.next_float() * (1.0f32 - 0.8f32) + 0.8f32;
                    height = (height as f32 * factor) as i32;
                }
                let mut pos = tb::offset(d.root, dx, 0, dz);
                let mut out_of_stone = false;
                let max_y = if d.up {
                    world
                        .height(HeightmapType::WorldSurfaceWg, pos[0], pos[2])
                        .map_err(query)?
                } else {
                    i32::MAX
                };
                let mut i = 0;
                while i < height && pos[1] < max_y {
                    let adjusted = wind.offset(pos);
                    let state = read(world, adjusted)?;
                    if s.empty_water_or_lava(state) {
                        out_of_stone = true;
                        write(world, adjusted, s.base, 2)?;
                    } else if out_of_stone && s.blocks.has(state, tb::BASE_STONE_OVERWORLD) {
                        break;
                    }
                    pos[1] = pos[1].wrapping_add(if d.up { 1 } else { -1 });
                    i += 1;
                }
            }
        }
        Ok(())
    }
}
