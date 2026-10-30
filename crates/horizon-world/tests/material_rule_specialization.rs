use horizon_core::{
    density::{ConstantSampler, DensitySampler},
    position::BlockPos,
    random::{RandomSource, XoroshiroRandom},
};
use horizon_world::{
    registry::BlockStateId,
    worldgen::{
        DecodeError,
        anchor::WorldGenerationContext,
        material_condition::{
            MaterialCondition, MaterialConditions, MaterialRule, MaterialRuleError,
            MaterialSurfaceQueries,
        },
    },
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Default)]
struct Queries {
    bindings: Vec<bool>,
    temperatures: usize,
    biomes: usize,
    fail_binding: bool,
    fail_temperature: bool,
    any_biome: bool,
}
impl MaterialSurfaceQueries for Queries {
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        (!self.any_biome).then(|| BTreeSet::from(["minecraft:plains".into()]))
    }
    fn surface_depth(&mut self, _: i32, _: i32) -> i32 {
        1
    }
    fn surface_secondary(&mut self, _: i32, _: i32) -> f64 {
        0.
    }
    fn bind_density(
        &mut self,
        _: Arc<dyn DensitySampler>,
        eager: bool,
    ) -> Result<usize, DecodeError> {
        self.bindings.push(eager);
        if self.fail_binding {
            return Err(DecodeError::Field("test binding failure"));
        }
        Ok(self.bindings.len() - 1)
    }
    fn biome(&mut self, _: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        self.biomes += 1;
        Ok(Some(Arc::from("minecraft:plains")))
    }
    fn cold_enough_to_snow(
        &mut self,
        _: &str,
        _: BlockPos,
        _: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.temperatures += 1;
        if self.fail_temperature {
            Err(MaterialRuleError::BiomeQueryFailed)
        } else {
            Ok(true)
        }
    }
}
fn worker(surface: Queries) -> MaterialConditions<XoroshiroRandom, Queries> {
    MaterialConditions::new(
        WorldGenerationContext::new(-64, 384, 63),
        XoroshiroRandom::new(7).fork_positional(),
        surface,
    )
}
fn condition(if_true: MaterialCondition, then_run: MaterialRule) -> MaterialRule {
    MaterialRule::Condition {
        if_true,
        then_run: Box::new(then_run),
    }
}
fn biome(name: &str) -> MaterialCondition {
    MaterialCondition::Biome(vec![name.into()])
}
fn ore() -> MaterialRule {
    MaterialRule::OreVein {
        ore: BlockStateId(1),
        raw_ore: BlockStateId(2),
        filler: BlockStateId(3),
        raw_ore_chance: 0.02,
        density: Arc::new(ConstantSampler(1.)),
        richness: Arc::new(ConstantSampler(1.)),
        filler_gap: Arc::new(ConstantSampler(-1.)),
    }
}
#[test]
fn constant_branches_keep_binding_order_and_short_circuit_queries() {
    let mut w = worker(Queries::default());
    let rule = w
        .add_rule(MaterialRule::Sequence(vec![
            condition(biome("minecraft:desert"), ore()),
            condition(
                MaterialCondition::Not(Box::new(biome("minecraft:desert"))),
                MaterialRule::Block(BlockStateId(9)),
            ),
            ore(),
        ]))
        .unwrap();
    w.update_xz(4, 5, 0, 0);
    w.update_y(1, 1, i32::MIN, 64);
    for _ in 0..2 {
        assert_eq!(
            w.try_apply(rule, BlockPos::new(4, 64, 5)),
            Ok(Some(BlockStateId(9)))
        );
    }
    let q = w.into_surface();
    assert_eq!(q.bindings, [true, true, false, true, true, false]);
    assert_eq!(q.biomes, 0);
    assert_eq!(q.temperatures, 0);
}
#[test]
fn an_empty_biome_list_matches_nothing_on_a_surface_without_biome_ids() {
    let mut w = worker(Queries {
        any_biome: true,
        ..Queries::default()
    });
    let rule = w
        .add_rule(condition(
            MaterialCondition::Biome(Vec::new()),
            MaterialRule::Block(BlockStateId(9)),
        ))
        .unwrap();
    w.update_xz(4, 5, 0, 0);
    w.update_y(1, 1, i32::MIN, 64);
    assert_eq!(w.try_apply(rule, BlockPos::new(4, 64, 5)), Ok(None));
}
#[test]
fn unreachable_children_still_fail_compilation_without_adding_a_rule() {
    for preceding_block in [false, true] {
        let mut w = worker(Queries {
            fail_binding: true,
            ..Queries::default()
        });
        let dead = condition(biome("minecraft:desert"), ore());
        let rule = if preceding_block {
            MaterialRule::Sequence(vec![MaterialRule::Block(BlockStateId(9)), dead])
        } else {
            dead
        };
        assert!(w.add_rule(rule).is_err());
        assert_eq!(
            w.try_apply(0, BlockPos::new(0, 0, 0)),
            Err(MaterialRuleError::UnknownRule)
        );
        assert_eq!(w.into_surface().bindings, [true]);
    }
}
#[test]
fn dynamic_predicate_on_empty_child_retains_each_temperature_call_and_error() {
    for fail in [false, true] {
        let mut w = worker(Queries {
            fail_temperature: fail,
            ..Queries::default()
        });
        let rule = w
            .add_rule(condition(
                MaterialCondition::Temperature,
                condition(
                    biome("minecraft:desert"),
                    MaterialRule::Block(BlockStateId(9)),
                ),
            ))
            .unwrap();
        w.update_xz(0, 0, 0, 0);
        w.update_y(1, 1, i32::MIN, 64);
        for _ in 0..2 {
            assert_eq!(
                w.try_apply(rule, BlockPos::new(0, 64, 0)),
                if fail {
                    Err(MaterialRuleError::BiomeQueryFailed)
                } else {
                    Ok(None)
                }
            );
        }
        let q = w.into_surface();
        assert_eq!(q.temperatures, 2);
        assert_eq!(q.biomes, 1);
    }
}
