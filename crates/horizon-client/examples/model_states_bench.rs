//! Compare the old two-parse loading path with catalog construction from shared facts.
//! Run: cargo run -p horizon-client --profile fast --example model_states_bench -- 15
extern crate horizon_client_lib as horizon_client;
use horizon_client::{Result, models::StateModels, sprites::Sprites};
use horizon_core::position::BlockPos;
use horizon_world::{
    DATA_VERSION,
    chunk::BlockState,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, hint::black_box, path::PathBuf, time::Instant};

struct Fixture {
    root: PathBuf,
    registry: BlockRegistry,
    bytes: usize,
    blocks: usize,
}
impl Fixture {
    fn new(target_bytes: usize) -> Result<Self> {
        let root =
            std::env::temp_dir().join(format!("horizon-model-states-{}", std::process::id()));
        fs::create_dir_all(root.join("assets/minecraft/blockstates"))?;
        fs::create_dir_all(root.join("assets/minecraft/models/block"))?;
        let mut possible = Vec::new();
        for age in 0..8 {
            for facing in ["north", "south", "east", "west"] {
                for waterlogged in ["false", "true"] {
                    possible.push(
                        json!({"age":age.to_string(), "facing":facing, "waterlogged":waterlogged}),
                    );
                }
            }
        }
        let fact = json!({"schema":{
            "age":{"integer":[0,7]},
            "facing":{"named":["north","south","east","west"]},
            "waterlogged":{"named":["false","true"]}
        },"states":possible});
        let blocks = (target_bytes / (serde_json::to_vec(&fact)?.len() + 26)).max(1);
        let mut facts = BTreeMap::new();
        let mut states = Vec::new();
        let mut defaults = BTreeMap::new();
        for i in 0..blocks {
            let block = format!("fixture_{i:04}");
            let name = format!("minecraft:{block}");
            defaults.insert(name.as_str().into(), Tag::Int(states.len() as i32));
            for properties in &possible {
                let state = BlockState {
                    name: name.clone(),
                    properties: serde_json::from_value(properties.clone())?,
                };
                states.push(state.to_nbt());
            }
            facts.insert(name, fact.clone());
            fs::write(
                root.join(format!("assets/minecraft/blockstates/{block}.json")),
                r#"{"multipart":[{"apply":[{"model":"minecraft:block/fixture","weight":2},{"model":"minecraft:block/fixture","y":90,"weight":1}]},{"when":{"waterlogged":"true"},"apply":{"model":"minecraft:block/fixture","y":180,"uvlock":true}}]}"#,
            )?;
        }
        fs::write(
            root.join("assets/minecraft/models/block/fixture.json"),
            r##"{"textures":{"all":"minecraft:block/fixture"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all","cullface":"north"}}}]}"##,
        )?;
        let document = serde_json::to_vec(&facts)?;
        let bytes = document.len();
        fs::write(root.join("model-states.json"), document)?;
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(states)),
            ("defaults", Tag::Compound(defaults)),
        ]))?;
        Ok(Self {
            root,
            registry,
            bytes,
            blocks,
        })
    }
    fn facts(&self) -> Result<BTreeMap<String, Value>> {
        Ok(serde_json::from_slice(&fs::read(
            self.root.join("model-states.json"),
        )?)?)
    }
    fn block_names<'a>(&self, names: impl Iterator<Item = &'a String>) -> Vec<&'a str> {
        names
            .filter(|name| self.registry.default_state(name).is_some())
            .filter_map(|name| name.strip_prefix("minecraft:"))
            .collect()
    }
    fn load(&self, reuse: bool) -> Result<(StateModels, Sprites)> {
        // The first parse and block filtering match models::prepare in both paths.
        let mut sprites = Sprites::default();
        let catalog = if reuse {
            let facts = self.facts()?;
            let blocks = self.block_names(facts.keys());
            StateModels::from_facts(&self.root, &self.registry, &blocks, &mut sprites, &facts)?
        } else {
            let facts: serde_json::Map<String, Value> =
                serde_json::from_slice(&fs::read(self.root.join("model-states.json"))?)?;
            let blocks = self.block_names(facts.keys());
            StateModels::load(&self.root, &self.registry, &blocks, &mut sprites)?
        };
        Ok((catalog, sprites))
    }
    fn verify(&self) -> Result<()> {
        let (before, before_sprites) = self.load(false)?;
        let (after, after_sprites) = self.load(true)?;
        assert_eq!(before_sprites.layers, after_sprites.layers);
        assert_eq!(before_sprites.opaque, after_sprites.opaque);
        assert_eq!(before_sprites.translucent, after_sprites.translucent);
        for id in 0..self.registry.len() {
            for position in [
                BlockPos::new(0, 0, 0),
                BlockPos::new(12, -4, 7),
                BlockPos::new(42, 9, -19),
            ] {
                let a = before.collect(BlockStateId(id as u32), position)?;
                let b = after.collect(BlockStateId(id as u32), position)?;
                assert_eq!(a.len(), b.len());
                for ((av, am), (bv, bm)) in a.into_iter().zip(b) {
                    assert_eq!(av, bv);
                    assert_eq!(am.ambient_occlusion, bm.ambient_occlusion);
                    assert_eq!(am.cube, bm.cube);
                    assert_eq!(am.faces.len(), bm.faces.len());
                    for (af, bf) in am.faces.iter().zip(&bm.faces) {
                        assert_eq!(af.quad, bf.quad);
                        assert_eq!(
                            (af.layer, af.opaque, af.translucent),
                            (bf.layer, bf.opaque, bf.translucent)
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn main() -> Result<()> {
    let repetitions: usize = std::env::args()
        .nth(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(15);
    if repetitions == 0 {
        return Err("repetitions must be positive".into());
    }
    let fixture = Fixture::new(2_840_000)?;
    fixture.verify()?;
    for _ in 0..2 {
        for reuse in [false, true] {
            drop(black_box(fixture.load(reuse)?));
        }
    }
    let mut before = Vec::new();
    let mut after = Vec::new();
    for iteration in 0..repetitions {
        // Alternate order to limit cache/order bias. Both paths use slice parsing.
        for reuse in if iteration % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = Instant::now();
            drop(black_box(fixture.load(black_box(reuse))?));
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if reuse {
                after.push(ms);
            } else {
                before.push(ms);
            }
        }
    }
    let before = median(&mut before);
    let after = median(&mut after);
    println!(
        "fixture: {} bytes, {} blocks, {} states; equality: all states at 3 positions",
        fixture.bytes,
        fixture.blocks,
        fixture.registry.len()
    );
    println!(
        "2 warmups/path, {repetitions} samples/path; median before {before:.3} ms, after {after:.3} ms; {:.3}x speedup ({:.2}% less time)",
        before / after,
        (1.0 - after / before) * 100.0
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_facts_match_load_and_preserve_errors() -> Result<()> {
        let fixture = Fixture::new(1)?;
        fixture.verify()?;
        let mut facts = fixture.facts()?;
        let blocks = ["fixture_0000"];
        let load_error = |facts: &BTreeMap<String, Value>| -> Result<String> {
            fs::write(
                fixture.root.join("model-states.json"),
                serde_json::to_vec(facts)?,
            )?;
            let a = StateModels::load(
                &fixture.root,
                &fixture.registry,
                &blocks,
                &mut Sprites::default(),
            )
            .err()
            .ok_or("expected load error")?
            .to_string();
            let b = StateModels::from_facts(
                &fixture.root,
                &fixture.registry,
                &blocks,
                &mut Sprites::default(),
                facts,
            )
            .err()
            .ok_or("expected shared facts error")?
            .to_string();
            assert_eq!(a, b);
            Ok(a)
        };
        let good = facts.clone();
        facts.clear();
        assert_eq!(load_error(&facts)?, "missing block model state schema");
        facts = good.clone();
        facts.get_mut("minecraft:fixture_0000").unwrap()["schema"] = Value::Null;
        assert_eq!(load_error(&facts)?, "invalid model schema");
        facts = good.clone();
        facts.get_mut("minecraft:fixture_0000").unwrap()["states"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert_eq!(
            load_error(&facts)?,
            "model state definitions disagree with the imported block registry"
        );
        fs::remove_file(fixture.root.join("model-states.json"))?;
        // A parsed document remains usable without reopening its source file.
        StateModels::from_facts(
            &fixture.root,
            &fixture.registry,
            &blocks,
            &mut Sprites::default(),
            &good,
        )?;
        assert!(
            StateModels::load(
                &fixture.root,
                &fixture.registry,
                &blocks,
                &mut Sprites::default()
            )
            .is_err()
        );
        fs::write(fixture.root.join("model-states.json"), b"null")?;
        assert!(
            StateModels::load(
                &fixture.root,
                &fixture.registry,
                &blocks,
                &mut Sprites::default()
            )
            .is_err()
        );
        fs::write(fixture.root.join("model-states.json"), b"{")?;
        assert!(
            StateModels::load(
                &fixture.root,
                &fixture.registry,
                &blocks,
                &mut Sprites::default()
            )
            .is_err()
        );
        Ok(())
    }
}
