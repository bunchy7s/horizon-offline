//! One mod jar on its own, as the launcher's mod browser installs it: what
//! of it Horizon runs, said before install (Works, Partly, Doesn't run),
//! from the jar itself. The jar is carried over ([`crate::carry`]) into a
//! folder content mod, and its classes are read ([`crate::classes`]) for
//! what the carry can't take: block entities, entities, Mixins, behaviour.
//!
//! - **Works**: content only. Its blocks, items, recipes, loot, tags and
//!   worldgen port, and its code only registers them.
//! - **Partly**: the content ports; the code features listed don't.
//! - **Doesn't run**: nothing to port (everything it does is Java code), or
//!   the jar can't be carried at all.
use crate::{carry, classes, jar};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Works,
    Partly,
    No,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Works => "Works",
            Verdict::Partly => "Partly",
            Verdict::No => "Doesn't run",
        }
    }
}

/// What checking a jar found. Written as `check.json` beside the carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    /// `neoforge`, `forge`, `fabric`, `quilt`, or `` for a plain zip.
    pub loader: String,
    pub license: String,
    pub verdict: Verdict,
    /// One line: why this verdict.
    pub summary: String,
    /// What ports: "12 blocks", "40 recipes"...
    pub ports: Vec<String>,
    /// What doesn't, with why.
    pub skipped: Vec<String>,
    /// Required mods, by id.
    pub depends: Vec<String>,
    /// The folder mod's namespaces (its id first): two mods on at once may
    /// not share one.
    pub namespaces: Vec<String>,
    pub code: classes::Code,
}

/// The loader a jar is made for, from its metadata file.
fn loader(names: &[String]) -> &'static str {
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("META-INF/neoforge.mods.toml") {
        "neoforge"
    } else if has("META-INF/mods.toml") {
        "forge"
    } else if has("fabric.mod.json") {
        "fabric"
    } else if has("quilt.mod.json") {
        "quilt"
    } else {
        ""
    }
}

/// Data folders named as before 1.21 (`recipes/`, `loot_tables/`) and none
/// as 1.21 names them: the game doesn't read them, and the carry upgrades
/// from 1.21.1 on. (Some jars keep a stale plural folder beside the new one.)
fn old_layout(names: &[String]) -> bool {
    let has = |kinds: [&str; 2]| {
        names.iter().any(|n| {
            let p: Vec<&str> = n.split('/').collect();
            p.len() > 3 && p[0] == "data" && kinds.contains(&p[2]) && n.ends_with(".json")
        })
    };
    has(["recipes", "loot_tables"]) && !has(["recipe", "loot_table"])
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Checks `jar` and carries it over into `out/carry/` (`out/check.json`,
/// `out/carry-report.json` beside it). The carry is the folder mod an
/// install copies.
pub fn check(jar_path: &Path, out: &Path) -> Result<Check, String> {
    let info = jar::read(jar_path)?;
    let file = fs::File::open(jar_path).map_err(|e| format!("{}: {e}", jar_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar_path.display()))?;
    let names: Vec<String> = zip
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", jar_path.display()))?;
    let mut bytes = Vec::new();
    for n in names.iter().filter(|n| n.ends_with(".class")) {
        let mut b = Vec::new();
        if let Ok(mut e) = zip.by_name(n)
            && e.size() < 1 << 24
            && e.read_to_end(&mut b).is_ok()
        {
            bytes.push(b);
        }
    }
    let code = classes::census(bytes.iter().map(Vec::as_slice));
    let mut check = Check {
        mod_id: info.mod_id.clone(),
        name: info.name.clone(),
        version: info.version.clone(),
        loader: loader(&names).to_owned(),
        license: info.license.clone(),
        verdict: Verdict::No,
        summary: String::new(),
        ports: Vec::new(),
        skipped: Vec::new(),
        depends: info.depends.clone(),
        namespaces: Vec::new(),
        code,
    };
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let done = |check: Check| -> Result<Check, String> {
        crate::pipeline::write_json(&out.join("check.json"), &check)?;
        Ok(check)
    };
    if old_layout(&names) {
        check.summary = "Made for Minecraft 1.20 or older: Horizon ports mods made for 1.21 \
                         and later."
            .into();
        return done(check);
    }
    let alone = |m: &str| m == info.mod_id || m == "minecraft";
    let pack = carry::Pack {
        loaded: &alone,
        mods: &alone,
    };
    let report = match carry::carry(
        jar_path,
        &info,
        out,
        None,
        &|_| true,
        &pack,
        &BTreeMap::new(),
    ) {
        Ok(r) => r,
        Err(e) => {
            check.summary = format!("Can't be carried over: {e}");
            return done(check);
        }
    };
    check.namespaces = std::iter::once(info.mod_id.clone())
        .chain(report.namespaces.iter().cloned())
        .collect();
    let c = &info.census;
    let recipes = report
        .recipes_upgraded
        .saturating_sub(report.recipes_unregistered.len());
    let loot = c.loot_tables + report.loot_tables_added;
    let content =
        report.blocks.len() + report.items + recipes + loot + report.worldgen + c.structures;
    for (n, one, many) in [
        (report.blocks.len(), "block", "blocks"),
        (report.items, "item", "items"),
        (recipes, "recipe", "recipes"),
        (loot, "loot table", "loot tables"),
        (report.worldgen, "worldgen file", "worldgen files"),
        (c.textures, "texture", "textures"),
    ] {
        if n > 0 {
            check.ports.push(plural(n, one, many));
        }
    }
    if !report.blocks.is_empty() {
        check
            .ports
            .push("blocks take the closest vanilla block's shape, hardness and sounds".into());
    }
    let mut skipped = Vec::new();
    if !report.blocks_unknown.is_empty() {
        skipped.push(plural(
            report.blocks_unknown.len(),
            "block with no vanilla block to copy (left out)",
            "blocks with no vanilla block to copy (left out)",
        ));
    }
    if !report.recipes_unregistered.is_empty() {
        skipped.push(plural(
            report.recipes_unregistered.len(),
            "recipe for a left-out block",
            "recipes for left-out blocks",
        ));
    }
    if !report.recipes_kept.is_empty() {
        skipped.push(plural(
            report.recipes_kept.len(),
            "recipe of a type Horizon doesn't have",
            "recipes of types Horizon doesn't have",
        ));
    }
    if !report.unreadable.is_empty() {
        skipped.push(plural(
            report.unreadable.len(),
            "file the game can't read",
            "files the game can't read",
        ));
    }
    let wg = &report.worldgen_types;
    let dropped = wg.processors_dropped.values().sum::<usize>();
    if dropped > 0 {
        skipped.push(plural(
            dropped,
            "structure processor written in Java",
            "structure processors written in Java",
        ));
    }
    let unsupported = wg.unsupported.values().sum::<usize>();
    if unsupported > 0 {
        skipped.push(plural(
            unsupported,
            "worldgen file of a type written in Java",
            "worldgen files of types written in Java",
        ));
    }
    for (f, n) in &check.code.features {
        let mut line = f.describe(*n);
        if *f == classes::Feature::Events {
            let names: Vec<&str> = check
                .code
                .events
                .iter()
                .take(3)
                .map(String::as_str)
                .collect();
            line.push_str(&format!(" ({}", names.join(", ")));
            if check.code.events.len() > 3 {
                line.push_str(", ...");
            }
            line.push(')');
        }
        skipped.push(line);
    }
    check.skipped = skipped;
    let behaviour = check.code.behaviour();
    (check.verdict, check.summary) = if content == 0 {
        let what = if behaviour || check.code.classes > 0 {
            "everything it does is Java code, which Horizon doesn't run"
        } else {
            "it has no blocks, items, recipes or worldgen"
        };
        (Verdict::No, format!("Nothing to port: {what}."))
    } else if check.skipped.is_empty() {
        (
            Verdict::Works,
            format!(
                "Content only: {} port.",
                check.ports[..check.ports.len().min(3)].join(", ")
            ),
        )
    } else {
        (
            Verdict::Partly,
            format!(
                "{} port; not {}.",
                check.ports[..check.ports.len().min(2)].join(", "),
                check.skipped[..check.skipped.len().min(2)].join(", ")
            ),
        )
    };
    done(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = dir.join("m.jar");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    const TOML: &[u8] = br#"modLoader = "javafml"
license = "MIT"
[[mods]]
modId = "ruby"
version = "1.0"
displayName = "Ruby"
"#;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("port-pack-check-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn content_only_works_and_code_makes_it_partly() {
        let dir = temp("content");
        let blockstate = br#"{"variants": {"": {"model": "ruby:block/ruby_block"}}}"#;
        let recipe = br#"{"type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:red_dye"], "result": {"id": "ruby:ruby"}}"#;
        let plain = [
            ("META-INF/neoforge.mods.toml", TOML),
            ("assets/ruby/blockstates/ruby_block.json", blockstate),
            ("assets/ruby/models/item/ruby.json", b"{}"),
            ("data/ruby/recipe/ruby.json", recipe),
        ];
        let c = check(&jar(&dir, &plain), &dir.join("out")).unwrap();
        assert_eq!(c.verdict, Verdict::Works, "{c:?}");
        assert_eq!(c.loader, "neoforge");
        assert_eq!(&c.ports[..3], ["1 block", "1 item", "1 recipe"]);
        assert!(dir.join("out/carry/horizon-mod.toml").is_file());
        assert!(dir.join("out/check.json").is_file());

        let tile = crate::classes::tests_support::class_file(
            "ruby/Tile",
            "net/minecraft/world/level/block/entity/BlockEntity",
        );
        let mut coded = plain.to_vec();
        coded.push(("ruby/Tile.class", &tile));
        let c = check(&jar(&dir, &coded), &dir.join("out")).unwrap();
        assert_eq!(c.verdict, Verdict::Partly);
        assert!(
            c.skipped[0].starts_with("1 block entity"),
            "{:?}",
            c.skipped
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recipes_for_left_out_blocks_are_left_out_too() {
        let dir = temp("left-out");
        let odd = br#"{"variants": {"wobble=1": {"model": "ruby:block/odd"}}}"#;
        let plain = br#"{"variants": {"": {"model": "ruby:block/ruby_block"}}}"#;
        let recipe = |result: &str| {
            format!(
                r#"{{"type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:red_dye"], "result": {{"id": "{result}"}}}}"#
            )
        };
        let (odd_recipe, block_recipe) = (recipe("ruby:odd"), recipe("ruby:ruby_block"));
        let files = [
            ("META-INF/neoforge.mods.toml", TOML),
            ("assets/ruby/blockstates/odd.json", odd.as_slice()),
            ("assets/ruby/blockstates/ruby_block.json", plain.as_slice()),
            ("data/ruby/recipe/odd.json", odd_recipe.as_bytes()),
            ("data/ruby/recipe/ruby_block.json", block_recipe.as_bytes()),
        ];
        let c = check(&jar(&dir, &files), &dir.join("out")).unwrap();
        assert_eq!(c.verdict, Verdict::Partly);
        assert!(c.ports.contains(&"1 recipe".to_string()), "{:?}", c.ports);
        assert!(
            c.skipped
                .contains(&"1 recipe for a left-out block".to_string()),
            "{:?}",
            c.skipped
        );
        assert!(!dir.join("out/carry/data/ruby/recipe/odd.json").exists());
        assert!(dir.join("out/outside/data/ruby/recipe/odd.json").exists());
        assert!(
            dir.join("out/carry/data/ruby/recipe/ruby_block.json")
                .exists()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn code_only_and_old_layouts_dont_run() {
        let dir = temp("code");
        let tile = crate::classes::tests_support::class_file("ruby/Mixin", "java/lang/Object");
        let c = check(
            &jar(
                &dir,
                &[
                    ("META-INF/neoforge.mods.toml", TOML),
                    ("ruby/A.class", &tile),
                ],
            ),
            &dir.join("out"),
        )
        .unwrap();
        assert_eq!(c.verdict, Verdict::No);
        assert!(c.summary.starts_with("Nothing to port"), "{}", c.summary);
        let c = check(
            &jar(
                &dir,
                &[
                    ("META-INF/mods.toml", TOML),
                    ("data/ruby/recipes/ruby.json", b"{}"),
                ],
            ),
            &dir.join("out"),
        )
        .unwrap();
        assert_eq!(c.verdict, Verdict::No);
        assert!(c.summary.contains("1.20 or older"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
