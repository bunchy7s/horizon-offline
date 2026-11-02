use super::*;
use crate::{items::ItemStack, mod_content::ContentMod};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

fn resources() -> &'static std::path::Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        let scratch = std::env::temp_dir().join(format!("fresh-farming-{}", std::process::id()));
        let dir = scratch.join("mod");
        let mut files = vec![(
            "horizon-mod.toml".to_owned(),
            "id = \"fieldtest\"\nname = \"Field test\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n".to_owned(),
        )];
        for (id, template) in [
            ("soil", "farmland"),
            ("grain", "wheat"),
            ("roots", "beetroots"),
            ("flower", "torchflower_crop"),
            ("pod", "pitcher_crop"),
            ("fake_crop", "stone"),
        ] {
            files.push((
                format!("data/fieldtest/horizon/block/{id}.json"),
                format!(r#"{{"template":"minecraft:{template}"}}"#),
            ));
        }
        files.push((
            "data/minecraft/tags/item/villager_plantable_seeds.json".into(),
            r#"{"replace":true,"values":["fieldtest:grain","fieldtest:roots","minecraft:stick"]}"#.into(),
        ));
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = horizon_mods::Package::read(&dir).unwrap();
        let content = horizon_mods::Content::read(&dir, &package.manifest).unwrap();
        let out = scratch.join("resources");
        crate::mod_content::build(&base, &[ContentMod { id: "fieldtest".into(), content }], &out)
            .unwrap();
        out
    })
}

fn farmer(types: &super::super::MobTypes) -> Mob {
    let mut m = Mob::new(
        1,
        Arc::new(types.types["minecraft:villager"].clone()),
        Vec3::new(0.5, 1.0, 0.5),
        0.0,
    );
    super::super::villager::set_profession(&mut m, 5);
    m
}

fn harvest() -> B {
    B::HarvestFarmland {
        t: timed(60, 60),
        next_ok: 0,
        target: None,
        valid: Vec::new(),
        worked: 0,
    }
}

fn put(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    state: horizon_world::registry::BlockStateId,
) {
    if !world.chunks.contains_key(&(x >> 4, 0)) {
        world
            .chunks
            .insert((x >> 4, 0), Arc::new(crate::chunk::Chunk::empty(x >> 4, 0)));
    }
    world.set(x, y, 0, state).unwrap();
}

#[test]
#[ignore = "full content root: tools/test_villager_farming_parity.py runs it"]
fn mod_farmland_is_a_secondary_site_and_farming_target() {
    assert!(super::super::world::with_resource_test_world(
        resources(),
        &[],
        true,
        |world, reg| {
            put(world, 0, 0, reg.default_state("fieldtest:soil").unwrap());
        },
        |w, types| {
            let mut m = farmer(types);
            sense_secondary(&mut m, w);
            assert_eq!(
                mem(&mut m).secondary_job_site.value(),
                Some(vec![[0, 0, 0]])
            );
            let mut lr = LegacyRandom::new(11);
            let mut events = Vec::new();
            let mut cx = Bx {
                w,
                lr: &mut lr,
                events: &mut events,
                now: 1000,
                refresh: false,
            };
            assert!(harvest().try_start(&mut m, &mut cx));
            let mut denied = w.clone();
            denied.mob_griefing = false;
            mem(&mut m).look_target.clear();
            mem(&mut m).walk_target.clear();
            cx.w = &denied;
            assert!(!harvest().try_start(&mut m, &mut cx));
        }
    ));
}

#[test]
#[ignore = "full content root: tools/test_villager_farming_parity.py runs it"]
fn mod_crops_are_harvested_only_at_their_vanilla_templates_max_age() {
    assert!(super::super::world::with_resource_test_world(
        resources(),
        &[],
        true,
        |world, reg| {
            for (x, block, age) in [
                (0, "grain", "6"),
                (1, "grain", "7"),
                (2, "roots", "3"),
                (3, "flower", "1"),
                (4, "pod", "4"),
            ] {
                let mut state = reg
                    .state(reg.default_state(&format!("fieldtest:{block}")).unwrap())
                    .unwrap()
                    .clone();
                state.properties.insert("age".into(), age.into());
                put(world, x, 1, reg.resolve(&state).unwrap());
            }
            put(
                world,
                5,
                1,
                reg.default_state("fieldtest:fake_crop").unwrap(),
            );
        },
        |w, types| {
            let mut m = farmer(types);
            let mut lr = LegacyRandom::new(12);
            let mut events = Vec::new();
            for (x, ripe) in [
                (0, false),
                (1, true),
                (2, true),
                (3, false),
                (4, false),
                (5, false),
            ] {
                events.clear();
                harvest_tick(
                    &mut m,
                    &mut Bx {
                        w,
                        lr: &mut lr,
                        events: &mut events,
                        now: 1000,
                        refresh: false,
                    },
                    &mut 0,
                    &mut Some([x, 1, 0]),
                    &mut Vec::new(),
                );
                assert_eq!(events.iter().any(|e| matches!(e, MobEvent::BreakBlock { pos, drop: true } if *pos == [x, 1, 0])), ripe, "x={x}");
            }
        }
    ));
}

#[test]
#[ignore = "full content root: tools/test_villager_farming_parity.py runs it"]
fn farming_uses_the_first_tagged_block_item_and_consumes_one_seed() {
    assert!(super::super::world::with_resource_test_world(
        resources(),
        &[],
        true,
        |world, reg| {
            for x in 0..=1 {
                put(world, x, 0, reg.default_state("fieldtest:soil").unwrap());
            }
        },
        |w, types| {
            let mut m = farmer(types);
            let inventory = &mut super::super::villager::ext_mut(&mut m).inventory;
            *inventory = vec![
                ItemStack::new("minecraft:wheat_seeds", 2),
                ItemStack::new("minecraft:stick", 2),
                ItemStack::new("fieldtest:flower", 2),
                ItemStack::new("fieldtest:roots", 1),
                ItemStack::new("fieldtest:grain", 2),
            ];
            let mut lr = LegacyRandom::new(13);
            let mut events = Vec::new();
            harvest_tick(
                &mut m,
                &mut Bx {
                    w,
                    lr: &mut lr,
                    events: &mut events,
                    now: 1000,
                    refresh: false,
                },
                &mut 0,
                &mut Some([0, 1, 0]),
                &mut Vec::new(),
            );
            let planted = events.iter().find_map(|e| match e {
                MobEvent::SetBlock { state, .. } => {
                    Some(w.registry.state(*state).unwrap().name.as_str())
                }
                MobEvent::SetBlockName { name, .. } => Some(*name),
                _ => None,
            });
            assert_eq!(planted, Some("fieldtest:roots"));
            let inventory = &super::super::villager::ext(&m).unwrap().inventory;
            assert_eq!(
                inventory
                    .iter()
                    .map(|s| (&*s.item, s.count))
                    .collect::<Vec<_>>(),
                vec![
                    ("minecraft:wheat_seeds", 2),
                    ("minecraft:stick", 2),
                    ("fieldtest:flower", 2),
                    ("fieldtest:grain", 2)
                ]
            );
            events.clear();
            harvest_tick(
                &mut m,
                &mut Bx {
                    w,
                    lr: &mut lr,
                    events: &mut events,
                    now: 1001,
                    refresh: false,
                },
                &mut 0,
                &mut Some([1, 1, 0]),
                &mut Vec::new(),
            );
            let expected = w.registry.default_state("fieldtest:grain").unwrap();
            assert!(events.iter().any(
                |e| matches!(e, MobEvent::SetBlock { pos: [1, 1, 0], state } if *state == expected)
            ));
            assert_eq!(
                super::super::villager::ext(&m)
                    .unwrap()
                    .inventory
                    .last()
                    .unwrap()
                    .count,
                1
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, MobEvent::Sound("item.crop.plant", _, 1.0, 1.0)))
                    .count(),
                1
            );
        }
    ));
}

#[test]
#[ignore = "tools/test_villager_farming_parity.py supplies the vanilla oracle"]
fn vanilla_farming_classification_matches_the_oracle() {
    let path = std::env::var_os("HORIZON_FARMING_ORACLE").expect("vanilla oracle output");
    let rows: Vec<(u32, Vec<bool>)> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            (
                fields.next().unwrap().parse().unwrap(),
                fields.map(|f| f.parse().unwrap()).collect(),
            )
        })
        .collect();
    assert!(!rows.is_empty());
    assert!(super::super::world::with_built_test_world(
        &[],
        |world, _| {
            for (x, (state, _)) in rows.iter().enumerate() {
                put(
                    world,
                    x as i32,
                    0,
                    horizon_world::registry::BlockStateId(*state),
                );
            }
        },
        |w, _| {
            for (x, (state, expected)) in rows.iter().enumerate() {
                let p = [x as i32, 0, 0];
                assert_eq!(
                    vec![
                        super::super::villager::crop(w, p),
                        super::super::villager::ripe_crop(w, p),
                        farmland_valid(w, [p[0], 1, 0])
                    ],
                    *expected,
                    "state={state}"
                );
            }
        }
    ));
}
