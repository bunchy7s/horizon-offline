//! Block transformer lookups for tools/test_block_transformer_parity.py: reads
//! "transformer TAB face TAB clicked TAB above" lines (states as
//! "name|prop=value,...") and prints the oracle's result lines.
//!
//! usage: block_transform_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    assets::Assets,
    block_transform::{Around, Transformers},
    items::Items,
};
use horizon_world::chunk::BlockState;
use horizon_world::loot::PackOverlay;
use std::{fs, path::Path};

fn parse(text: &str) -> BlockState {
    let (name, props) = text.split_once('|').unwrap_or((text, ""));
    BlockState {
        name: name.to_owned(),
        properties: props
            .split(',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}

fn describe(state: &BlockState) -> String {
    let props: Vec<String> = state
        .properties
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{}|{}", state.name, props.join(","))
}

struct Column {
    clicked: BlockState,
    above: BlockState,
}

impl Around for Column {
    fn state(&self, offset: [i32; 3]) -> BlockState {
        match offset {
            [0, 0, 0] => self.clicked.clone(),
            [0, 1, 0] => self.above.clone(),
            _ => BlockState {
                name: "minecraft:stone".into(),
                properties: Default::default(),
            },
        }
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: block_transform_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let assets = Assets::load(root)?;
    let transformers = Transformers::load(root, assets.registry)?;
    let mut out = String::new();
    for line in input.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let around = Column {
            clicked: parse(f[2]),
            above: parse(f[3]),
        };
        if f[0] == "honeycomb" {
            let waxed = transformers.waxed(assets.registry, &around.clicked);
            out += &format!("{}\n", waxed.map_or("none".to_owned(), |s| describe(&s)));
            continue;
        }
        let id = format!("minecraft:{}", f[0]);
        match transformers.apply(&items, assets.registry, &id, f[1], &around) {
            None => out += "none\n",
            Some((index, t, state)) => {
                out += &format!(
                    "{index}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    describe(&state),
                    t.sound,
                    t.particle,
                    t.loot.as_deref().unwrap_or("-"),
                    t.drop_strategy,
                    t.update_from_neighbors,
                    t.transform_type,
                    t.consume_on_use,
                    t.damage
                );
            }
        }
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
