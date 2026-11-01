//! Paths in OptiFine `.properties` files, and the folders that hold those files. Paths here are
//! resource paths inside the overlay resource root (`assets/<namespace>/...`, `/`-separated),
//! the tree `horizon_client::packs::resource_root` builds from the enabled packs.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The resource path a value of the properties file at `props` names:
/// - `~/x`: `assets/minecraft/optifine/x`;
/// - `namespace:x`: `assets/namespace/x`;
/// - `./x`, or a bare name without `/`: beside the properties file;
/// - any other `a/b`: `assets/<the file's namespace>/a/b`.
///
/// The extension is the caller's: OptiFine adds `.png` to textures that have none.
///
/// `None` when the path would leave the resource root or is malformed: a `..` or `.` segment, an
/// empty one (`a//b`, a leading `/`), as vanilla's resource paths refuse them. Callers join the
/// result onto the overlay root, so a pack must never name a file outside it.
pub fn resolve(props: &str, value: &str) -> Option<String> {
    let path = resolve_unchecked(props, value);
    path.split('/')
        .all(|segment| !matches!(segment, "" | "." | ".."))
        .then_some(path)
}

fn resolve_unchecked(props: &str, value: &str) -> String {
    let value = value.trim();
    let dir = props.rsplit_once('/').map_or("", |(dir, _)| dir);
    if let Some(rest) = value.strip_prefix("~/") {
        return format!("assets/minecraft/optifine/{rest}");
    }
    if let Some(rest) = value.strip_prefix("./") {
        return format!("{dir}/{rest}");
    }
    if let Some((ns, rest)) = value.split_once(':') {
        return format!("assets/{ns}/{rest}");
    }
    if !value.contains('/') {
        return format!("{dir}/{value}");
    }
    let namespace = props
        .strip_prefix("assets/")
        .and_then(|p| p.split('/').next())
        .unwrap_or("minecraft");
    format!("assets/{namespace}/{value}")
}

/// Every `.properties` file under `assets/*/optifine/<feature>/` and the legacy
/// `assets/*/mcpatcher/<feature>/` of `root`, as sorted resource paths. `feature` is a folder
/// (`ctm`, `cit`, `random/entity`, `sky/world0`).
pub fn properties_files(root: &Path, feature: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
        return out;
    };
    for ns in namespaces.flatten() {
        let ns = ns.file_name().to_string_lossy().into_owned();
        for top in ["optifine", "mcpatcher"] {
            let base = format!("assets/{ns}/{top}/{feature}");
            walk(&root.join(&base), &base, 0, &mut seen, &mut out);
        }
    }
    out.sort();
    out
}

/// How deep a feature folder's tree is read: far past any real pack, short of a link loop's
/// `PATH_MAX`.
const MAX_DEPTH: usize = 16;

fn walk(dir: &Path, rel: &str, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<String>) {
    // Links are followed (overlay folders are links into the packs and vanilla), so a link back
    // to a parent would loop: each real folder is read once, and never deeper than MAX_DEPTH.
    if depth > MAX_DEPTH || !fs::canonicalize(dir).is_ok_and(|real| seen.insert(real)) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = format!("{rel}/{name}");
        // `metadata` follows links: overlay folders are links into the packs and vanilla.
        match fs::metadata(e.path()) {
            Ok(m) if m.is_dir() => walk(&e.path(), &rel, depth + 1, seen, out),
            Ok(_) if name.ends_with(".properties") => out.push(rel),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolving() {
        let props = "assets/minecraft/optifine/cit/swords/flame.properties";
        let r = |v| resolve(props, v).unwrap();
        assert_eq!(r("flame"), "assets/minecraft/optifine/cit/swords/flame");
        assert_eq!(
            r("./sub/flame.png"),
            "assets/minecraft/optifine/cit/swords/sub/flame.png"
        );
        assert_eq!(
            r("~/cit/shared/glow.png"),
            "assets/minecraft/optifine/cit/shared/glow.png"
        );
        assert_eq!(
            r("textures/item/stick.png"),
            "assets/minecraft/textures/item/stick.png"
        );
        assert_eq!(r("mymod:textures/x.png"), "assets/mymod/textures/x.png");
        assert_eq!(
            resolve(
                "assets/mymod/optifine/ctm/a.properties",
                "textures/block/b.png"
            )
            .as_deref(),
            Some("assets/mymod/textures/block/b.png")
        );
        // Nothing outside the resource root.
        for bad in [
            "~/../../../../x",
            "..:x",
            "../x.png",
            "./../x",
            "a//b",
            "/etc/passwd",
            "a/./b",
        ] {
            assert_eq!(resolve(props, bad), None, "{bad}");
        }
    }

    #[test]
    fn finds_both_folders_in_every_namespace() {
        let root = std::env::temp_dir().join(format!("horizon-optifine-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for f in [
            "assets/minecraft/optifine/ctm/glass/glass.properties",
            "assets/minecraft/optifine/ctm/glass/0.png",
            "assets/minecraft/mcpatcher/ctm/old.properties",
            "assets/mymod/optifine/ctm/x/y.properties",
            "assets/minecraft/optifine/cit/sword.properties",
        ] {
            let path = root.join(f);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        assert_eq!(
            properties_files(&root, "ctm"),
            [
                "assets/minecraft/mcpatcher/ctm/old.properties",
                "assets/minecraft/optifine/ctm/glass/glass.properties",
                "assets/mymod/optifine/ctm/x/y.properties",
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_loop_is_read_once() {
        let root =
            std::env::temp_dir().join(format!("horizon-optifine-loop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let ctm = root.join("assets/minecraft/optifine/ctm");
        fs::create_dir_all(&ctm).unwrap();
        fs::write(ctm.join("a.properties"), "").unwrap();
        std::os::unix::fs::symlink(&ctm, ctm.join("again")).unwrap();
        assert_eq!(
            properties_files(&root, "ctm"),
            ["assets/minecraft/optifine/ctm/a.properties"]
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
