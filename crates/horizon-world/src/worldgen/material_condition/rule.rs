//! Compiled material rule composition, sharing the condition worker's context.
use super::{
    Compiled, Context, Facts, MaterialCondition, MaterialConditions, MaterialSurfaceQueries,
    density::MaterialDensitySources,
};
use crate::{registry::BlockStateId, worldgen::DecodeError};
use horizon_core::{
    density::DensitySampler,
    position::BlockPos,
    random::{PositionalRandom, RandomSource},
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum MaterialRule {
    Block(BlockStateId),
    Reference(Arc<MaterialRule>),
    /// Density roots must be compiled for the same world and sampler context.
    OreVein {
        ore: BlockStateId,
        raw_ore: BlockStateId,
        filler: BlockStateId,
        raw_ore_chance: f32,
        density: Arc<dyn DensitySampler>,
        richness: Arc<dyn DensitySampler>,
        filler_gap: Arc<dyn DensitySampler>,
    },
    Sequence(Vec<MaterialRule>),
    Condition {
        if_true: MaterialCondition,
        then_run: Box<MaterialRule>,
    },
    /// Vanilla calls this rule "bandlands"; it queries terracotta bands.
    Bandlands,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialRuleError {
    UnknownRule,
    UnknownCondition,
    MissingBiomeProvider,
    BiomeQueryFailed,
    MissingBiome,
    CachedConditionUnavailable,
    MissingBandProvider,
    MissingDensityProvider,
    Density(horizon_core::density::SampleError),
    InvalidBandIndex,
}
impl std::fmt::Display for MaterialRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::Density(error) = self {
            return error.fmt(f);
        }
        f.write_str(match self {
            Self::Density(_) => unreachable!(),
            Self::MissingDensityProvider => "missing material density provider",
            Self::UnknownRule => "unknown material rule",
            Self::UnknownCondition => "unknown material condition",
            Self::MissingBiomeProvider => "missing material biome provider",
            Self::BiomeQueryFailed => "material biome lookup failed",
            Self::MissingBiome => "material position has no biome",
            Self::CachedConditionUnavailable => "condition was updated without a cached result",
            Self::MissingBandProvider => "missing material band provider",
            Self::InvalidBandIndex => "negative material band index",
        })
    }
}
impl std::error::Error for MaterialRuleError {}
impl From<horizon_core::density::SampleError> for MaterialRuleError {
    fn from(error: horizon_core::density::SampleError) -> Self {
        Self::Density(error)
    }
}

pub(super) enum CompiledRule<R: RandomSource> {
    Empty,
    Block(BlockStateId),
    OreVein {
        blocks: [BlockStateId; 3],
        raw_ore_chance: f32,
        densities: [usize; 3],
        random: R::Positional,
    },
    Sequence(Vec<Self>),
    Condition {
        condition: Compiled<R>,
        rule: Box<Self>,
    },
    Bandlands,
}
impl<R: RandomSource> CompiledRule<R> {
    fn compile<S: MaterialSurfaceQueries>(
        rule: MaterialRule,
        root: &R::Positional,
        context: &mut Context<S>,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<Self, DecodeError> {
        *nodes += 1;
        if depth >= 256 || *nodes > 65536 {
            return Err(DecodeError::Field("material rule size"));
        }
        Ok(match rule {
            MaterialRule::Block(block) => Self::Block(block),
            MaterialRule::Reference(value) => {
                return Self::compile((*value).clone(), root, context, depth + 1, nodes);
            }
            MaterialRule::OreVein {
                ore,
                raw_ore,
                filler,
                raw_ore_chance,
                density,
                richness,
                filler_gap,
            } => {
                let density = context.surface.bind_density(density, true)?;
                let richness = context.surface.bind_density(richness, true)?;
                let gap = context.surface.bind_density(filler_gap, false)?;
                let random = root.from_hash_of("minecraft:ore").fork_positional();
                Self::OreVein {
                    blocks: [ore, raw_ore, filler],
                    raw_ore_chance,
                    densities: [density, richness, gap],
                    random,
                }
            }
            MaterialRule::Bandlands => Self::Bandlands,
            MaterialRule::Condition { if_true, then_run } => {
                // Compile the predicate before the nested rule, regardless of
                // whether the predicate will match any subsequent position.
                let condition = Compiled::compile(if_true, root, context, depth + 1)?;
                let rule = Box::new(Self::compile(*then_run, root, context, depth + 1, nodes)?);
                // Compile both sides first: binding and errors must retain their order.
                match condition.constant_result() {
                    Some(false) => Self::Empty,
                    Some(true) => *rule,
                    None => Self::Condition { condition, rule },
                }
            }
            MaterialRule::Sequence(mut children) => {
                if children.len() == 1 {
                    return Self::compile(children.pop().unwrap(), root, context, depth + 1, nodes);
                }
                let mut result = Vec::with_capacity(children.len());
                for child in children {
                    let child = Self::compile(child, root, context, depth + 1, nodes)?;
                    if !matches!(child, Self::Empty) {
                        result.push(child);
                    }
                }
                match result.len() {
                    0 => Self::Empty,
                    1 => result.pop().unwrap(),
                    _ => Self::Sequence(result),
                }
            }
        })
    }
    fn apply<S: MaterialSurfaceQueries>(
        &mut self,
        position: BlockPos,
        context: &mut Context<S>,
    ) -> Result<Option<BlockStateId>, MaterialRuleError> {
        match self {
            Self::Empty => Ok(None),
            Self::Block(block) => Ok(Some(*block)),
            Self::OreVein {
                blocks,
                raw_ore_chance,
                densities,
                random,
            } => {
                // Densities use the context; randomness uses application coordinates.
                let density = context.surface.density(densities[0], context.position)?;
                if density <= 0.0 {
                    return Ok(None);
                }
                let mut random = random.at(position);
                if random.next_float() > density {
                    return Ok(None);
                }
                let richness = context.surface.density(densities[1], context.position)?;
                if random.next_float() < richness
                    && context.surface.density(densities[2], context.position)? < 0.0
                {
                    Ok(Some(if random.next_float() < *raw_ore_chance {
                        blocks[1]
                    } else {
                        blocks[0]
                    }))
                } else {
                    Ok(Some(blocks[2]))
                }
            }
            Self::Bandlands => context.surface.band(position).map(Some),
            Self::Condition { condition, rule } => {
                if condition.test(context)? {
                    rule.apply(position, context)
                } else {
                    Ok(None)
                }
            }
            Self::Sequence(children) => {
                for child in children {
                    if let Some(block) = child.apply(position, context)? {
                        return Ok(Some(block));
                    }
                }
                Ok(None)
            }
        }
    }
}
impl<R: RandomSource> CompiledRule<R>
where
    R::Positional: Clone,
{
    /// This rule where `facts` hold: conditions they decide are folded away,
    /// with what can no longer produce a block, and a sequence stops at its
    /// first child that always produces one. For every position the facts
    /// cover it gives the block the whole rule gives.
    fn specialize(&self, facts: Facts) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Block(block) => Self::Block(*block),
            Self::Bandlands => Self::Bandlands,
            Self::OreVein {
                blocks,
                raw_ore_chance,
                densities,
                random,
            } => Self::OreVein {
                blocks: *blocks,
                raw_ore_chance: *raw_ore_chance,
                densities: *densities,
                random: random.clone(),
            },
            Self::Condition { condition, rule } => match condition.decided(facts) {
                Some(false) => Self::Empty,
                Some(true) => rule.specialize(facts),
                None => match rule.specialize(facts) {
                    Self::Empty => Self::Empty,
                    rule => Self::Condition {
                        condition: condition.clone(),
                        rule: Box::new(rule),
                    },
                },
            },
            Self::Sequence(children) => {
                let mut result = Vec::new();
                for child in children {
                    let child = child.specialize(facts);
                    if matches!(child, Self::Empty) {
                        continue;
                    }
                    let last = child.always_produces();
                    result.push(child);
                    if last {
                        break;
                    }
                }
                match result.len() {
                    0 => Self::Empty,
                    1 => result.pop().unwrap(),
                    _ => Self::Sequence(result),
                }
            }
        }
    }
    /// Whether applying this rule never gives `None` (only an error).
    fn always_produces(&self) -> bool {
        match self {
            Self::Block(_) | Self::Bandlands => true,
            Self::Sequence(children) => children.iter().any(Self::always_produces),
            Self::Empty | Self::OreVein { .. } | Self::Condition { .. } => false,
        }
    }
    fn inputs(&self, gradients: &mut Vec<(i32, i32)>, surface: &mut bool) {
        match self {
            Self::Condition { condition, rule } => {
                condition.inputs(gradients, surface);
                rule.inputs(gradients, surface);
            }
            Self::Sequence(children) => {
                for child in children {
                    child.inputs(gradients, surface);
                }
            }
            _ => {}
        }
    }
}
/// A rule specialised (`CompiledRule::specialize`) for each band of y between
/// its vertical gradients' bounds, and below and at or above the preliminary
/// surface when the rule asks for it: most blocks of a column, deep stone,
/// then evaluate only the few rules that can apply there.
pub struct SurfaceVariants {
    /// The first y of each band after the first, ascending.
    starts: Vec<i32>,
    /// The rule index per band (two per band, below then above the surface,
    /// when `surface`).
    rules: Vec<usize>,
    pub surface: bool,
}
impl SurfaceVariants {
    /// The rule for a block at `y`, `above` the preliminary surface (ignored
    /// unless `surface`).
    #[inline]
    pub fn rule(&self, y: i32, above: bool) -> usize {
        let band = self.starts.partition_point(|&start| start <= y);
        if self.surface {
            self.rules[band * 2 + usize::from(above)]
        } else {
            self.rules[band]
        }
    }
}
impl<R: RandomSource, S: MaterialSurfaceQueries> MaterialConditions<R, S>
where
    R::Positional: Clone,
{
    /// Add the specialised variants of rule `index` (`SurfaceVariants`).
    pub fn surface_variants(&mut self, index: usize) -> Result<SurfaceVariants, MaterialRuleError> {
        let rule = self
            .rules
            .get(index)
            .ok_or(MaterialRuleError::UnknownRule)?;
        let (mut gradients, mut surface) = (Vec::new(), false);
        rule.inputs(&mut gradients, &mut surface);
        // A gradient is true at and below `low`, random between, false at
        // and above `high`: it is decided within each band these bound.
        let mut starts: Vec<i32> = gradients
            .iter()
            .flat_map(|&(low, high)| [low.saturating_add(1), high])
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let mut variants = Vec::new();
        for band in 0..=starts.len() {
            let low = if band == 0 {
                i32::MIN
            } else {
                starts[band - 1]
            };
            let high = starts.get(band).map_or(i32::MAX, |&next| next - 1);
            let sides: &[Option<bool>] = if surface {
                &[Some(false), Some(true)]
            } else {
                &[None]
            };
            for &above_surface in sides {
                let facts = Facts {
                    low,
                    high,
                    above_surface,
                };
                variants.push(self.rules[index].specialize(facts));
            }
        }
        let first = self.rules.len();
        self.rules.extend(variants);
        Ok(SurfaceVariants {
            starts,
            rules: (first..self.rules.len()).collect(),
            surface,
        })
    }
}
impl<R: RandomSource, T: MaterialSurfaceQueries> MaterialConditions<R, MaterialDensitySources<T>> {
    /// Re-target the per-cell density bindings for a reused worker.
    pub fn retarget_surface_densities(&mut self, position: [i32; 3]) -> Result<(), DecodeError> {
        self.context.surface.retarget(position)
    }
}
impl<R: RandomSource, S: MaterialSurfaceQueries> MaterialConditions<R, S> {
    /// Compile all children in order, even after an unconditional block rule.
    /// Failed compilation never adds a partial rule to this worker.
    pub fn add_rule(&mut self, rule: MaterialRule) -> Result<usize, DecodeError> {
        if self.rules.len() >= 8192 {
            return Err(DecodeError::Field("material rule count"));
        }
        let compiled = CompiledRule::compile(rule, &self.root, &mut self.context, 0, &mut 0)?;
        let index = self.rules.len();
        self.rules.push(compiled);
        Ok(index)
    }
    /// Predicates read the worker's updated context. Position arguments are
    /// forwarded to block-producing rules independently, as in vanilla.
    pub fn try_apply(
        &mut self,
        index: usize,
        position: BlockPos,
    ) -> Result<Option<BlockStateId>, MaterialRuleError> {
        self.rules
            .get_mut(index)
            .ok_or(MaterialRuleError::UnknownRule)?
            .apply(position, &mut self.context)
    }
}
