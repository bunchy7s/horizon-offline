//! `cargo horizon-mod`: make Horizon content mods (`docs/modpacks.md`): blocks,
//! items, data and assets, no code. Code mods are native (docs/native-mods.md).
//!
//! - `new <path> [--name "Name"]`: a content mod from the template.
//! - `build`: the `.hmod` in `target/horizon-mod/`, with the mod's `assets/`
//!   and `data/`.
//! - `run [--game <horizon-client>] [-- <game args>]`: build, then play with
//!   the mod.
//! - `check <file> [--json]`: what the registry checks of a submission
//!   (`.hmod`, `.hnmod`, `.hpack`, resource pack `.zip`, `.hshader`).
//! - `registry build|review|check|list`: the registry repository's CI.
use horizon_mods::{Capability, Content, Manifest, Package, manifest::MANIFEST};
mod check;
mod registry;

use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    // Cargo runs `cargo-horizon-mod horizon-mod <args>`.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "horizon-mod") {
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..]),
        Some("build") => build().map(|p| {
            println!("Built {}", p.display());
        }),
        Some("run") => run(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("registry") => registry::main(&args[1..]),
        _ => Err("usage: cargo horizon-mod new <path> [--name \"Name\"] | build | run [--game <horizon-client>] [-- <game args>] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json] | registry build|review|check|list".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The value after `flag` in `args`.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

fn new(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod new <path>")?;
    let dir = Path::new(path);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let base = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the path needs a folder name")?;
    let id: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let name = flag(args, "--name").map_or_else(|| base.replace(['-', '_'], " "), str::to_string);
    let fill = |t: &str| t.replace("{id}", &id).replace("{name}", &name);
    // A block copying a vanilla one, with its model, name, loot, recipe and tag.
    let files = [
        (
            MANIFEST,
            include_str!("../template/content/horizon-mod.toml"),
        ),
        (
            "data/{id}/horizon/block/glowing_amethyst.json",
            include_str!("../template/content/block.json"),
        ),
        (
            "assets/{id}/blockstates/glowing_amethyst.json",
            include_str!("../template/content/blockstate.json"),
        ),
        (
            "assets/{id}/models/block/glowing_amethyst.json",
            include_str!("../template/content/model.json"),
        ),
        (
            "assets/{id}/items/glowing_amethyst.json",
            include_str!("../template/content/item.json"),
        ),
        (
            "assets/{id}/lang/en_us.json",
            include_str!("../template/content/lang.json"),
        ),
        (
            "data/{id}/loot_table/blocks/glowing_amethyst.json",
            include_str!("../template/content/loot.json"),
        ),
        (
            "data/{id}/recipe/glowing_amethyst.json",
            include_str!("../template/content/recipe.json"),
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            include_str!("../template/content/pickaxe.json"),
        ),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    if flag(args, "--template").is_some_and(|t| t != "content") {
        return Err(
            "only content mods are made here: code mods are native (docs/native-mods.md)".into(),
        );
    }
    for (file, text) in files {
        let file = fill(file);
        let file = file.as_str();
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?)?;
    Content::read(dir, &manifest)?;
    println!(
        "Created {} (mod id {id}). Next: cd {path} && cargo horizon-mod run",
        dir.display()
    );
    Ok(())
}

/// Packs the content mod in the current folder into `target/horizon-mod/<id>.hmod`: the
/// manifest and the files in `assets/` and `data/`.
fn build() -> Result<PathBuf, String> {
    build_in(Path::new("."))
}

fn build_in(root: &Path) -> Result<PathBuf, String> {
    let manifest_text = std::fs::read_to_string(root.join(MANIFEST))
        .map_err(|e| format!("{MANIFEST}: {e} (run this in a mod's folder)"))?;
    let manifest = Manifest::parse(&manifest_text)?;
    if root.join("Cargo.toml").exists() {
        return Err("a Cargo.toml: code mods are native now (docs/native-mods.md)".into());
    }
    if !manifest.capabilities.contains(&Capability::Registries) {
        return Err(format!(
            "a content mod has capability registries in {MANIFEST}"
        ));
    }
    // The same checks as the game's, and the files in the order it lists them.
    let content = Content::read(root, &manifest)?;
    let mut reader = content.reader()?;
    let mut files = Vec::with_capacity(content.files.len());
    for name in &content.files {
        files.push((name.as_str(), reader.read(name)?));
    }
    let files: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let dir = root.join("target/horizon-mod");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.hmod", manifest.id));
    // Write then rename, so a running game never reads half a file.
    let partial = path.with_extension("hmod.partial");
    let bytes = Package::write_files(&manifest_text, None, &files)?;
    std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `check <file>`: the registry's checks of a submission, printed; fails
/// when one does.
fn check(args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod check <file>")?;
    let path = Path::new(file);
    let kind = match flag(args, "--kind") {
        Some(k) => horizon_mods::index::Kind::from(k.to_string()),
        None => check::kind_of(path)
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?,
    };
    let signature = flag(args, "--signature")
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
        .transpose()?;
    let report = check::check(path, kind, flag(args, "--key"), signature.as_deref());
    if args.iter().any(|a| a == "--json") {
        let json = match &report {
            Ok(r) => serde_json::json!({"ok": true, "report": r.json()}),
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        println!("{json}");
    }
    let report = report?;
    if !args.iter().any(|a| a == "--json") {
        print!("{}", report.text());
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let path = build()?;
    let game = flag(args, "--game")
        .map(str::to_string)
        .or_else(|| std::env::var("HORIZON_CLIENT").ok())
        .unwrap_or_else(|| "horizon-client".into());
    let extra = args.iter().skip_while(|a| *a != "--").skip(1);
    println!("Starting {game} with {}", path.display());
    let status = Command::new(&game)
        .arg("--dev-mod")
        .arg(&path)
        .args(extra)
        .status()
        .map_err(|e| {
            format!("{game}: {e} (pass --game <path to horizon-client> or set HORIZON_CLIENT)")
        })?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("the game exited with {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_template_builds_and_checks_with_no_code() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
        let path = dir.join("gems");
        let new_args = [
            path.display().to_string(),
            "--template".into(),
            "content".into(),
        ];
        new(&new_args).unwrap();
        assert!(!path.join("Cargo.toml").exists());
        let hmod = build_in(&path).unwrap();
        assert_eq!(hmod, path.join("target/horizon-mod/gems.hmod"));
        let package = Package::read(&hmod).unwrap();
        assert!(package.wasm.is_none());
        let content = Content::read(&hmod, &package.manifest).unwrap();
        assert!(
            content
                .files
                .contains(&"data/gems/horizon/block/glowing_amethyst.json".into())
        );
        assert_eq!(content.files.len(), 8, "{:?}", content.files);
        let report = check::check(&hmod, check::kind_of(&hmod).unwrap(), None, None).unwrap();
        assert!(
            report.text().contains("no code: content only"),
            "{}",
            report.text()
        );
        // Code is native now.
        std::fs::write(path.join("Cargo.toml"), "").unwrap();
        assert!(build_in(&path).unwrap_err().contains("native"));
        assert!(
            new(&[
                dir.join("x").display().to_string(),
                "--template".into(),
                "lua".into()
            ])
            .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
