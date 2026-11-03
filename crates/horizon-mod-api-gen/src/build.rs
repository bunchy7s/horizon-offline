//! Cargo-facing generation and the launcher's trait injection boundary.
use super::{Target, catalog_hooks, free_function_source, sources};
use quote::ToTokens;
use std::{collections::HashSet, fs, path::Path};

/// Generate only this crate's files, tracking every source read by the generator.
/// `root` is supplied at run time by the consuming crate's build script.
pub fn generate(root: &Path, out_dir: &Path, target: Target) -> Result<(), String> {
    track(root)?;
    let generated = sources(root, super::FillMode::Require)?;
    let outputs = match target {
        Target::Render => vec![("native_render_generated.rs", generated.render)],
        Target::Client => {
            println!("cargo:rerun-if-env-changed=HORIZON_MOD_API_INJECTIONS");
            let hooks = if let Some(path) = std::env::var_os("HORIZON_MOD_API_INJECTIONS") {
                let path = Path::new(&path);
                println!("cargo:rerun-if-changed={}", path.display());
                let injections =
                    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
                inject(&generated.hooks, &injections)?
            } else {
                generated.hooks
            };
            vec![
                ("mod_api_generated.rs", hooks),
                ("mod_api_generated_host.rs", generated.host),
                ("mod_api_host_generated.rs", generated.implementation),
            ]
        }
    };
    for (name, code) in outputs {
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {e}"))?;
        fs::write(out_dir.join(name), code).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn track(root: &Path) -> Result<(), String> {
    fn directory(path: &Path, extension: &str, recursive: bool) -> Result<(), String> {
        // Track the directory as well: adding a helper source or fragment changes the inputs.
        println!("cargo:rerun-if-changed={}", path.display());
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() && recursive {
                directory(&path, extension, recursive)?;
            } else if path.extension().is_some_and(|ext| ext == extension) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        Ok(())
    }
    for path in ["native-hooks.json", "crates/horizon-client/src/mod_api.rs"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    directory(&root.join("tools/native-hook-fragments"), "json", false)?;
    directory(&root.join("crates/horizon-client/src/game"), "rs", true)?;
    let catalog = serde_json::from_str(
        &fs::read_to_string(root.join("native-hooks.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    for hook in catalog_hooks(root, &catalog)? {
        if let Some((krate, path)) =
            free_function_source(hook["target"].as_str().ok_or("target missing")?)
        {
            let (module, _) = path.rsplit_once("::").ok_or("target missing module")?;
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Injection {
    name: String,
    event: String,
    declaration: String,
    event_declaration: String,
}

/// Add the launcher's trait methods and event structs to the generated catalog API.
/// The JSON file is the boundary: parse declarations once, then work with typed Rust items.
pub fn inject(code: &str, injections: &str) -> Result<String, String> {
    let injections: Vec<Injection> =
        serde_json::from_str(injections).map_err(|e| format!("trait injections: {e}"))?;
    if injections.is_empty() {
        return Ok(code.to_owned());
    }
    let mut file = syn::parse_file(code).map_err(|e| format!("generated API: {e}"))?;
    let mut events: HashSet<String> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item) => Some(item.ident.to_string()),
            syn::Item::Trait(item) => Some(item.ident.to_string()),
            _ => None,
        })
        .collect();
    let hooks = file
        .items
        .iter_mut()
        .find_map(|item| match item {
            syn::Item::Trait(item) if item.ident == "Hooks" => Some(item),
            _ => None,
        })
        .ok_or("generated API missing Hooks trait")?;
    let mut methods: HashSet<_> = hooks
        .items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Fn(method) => Some(method.sig.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut structs = Vec::new();
    for injection in injections {
        let method: syn::TraitItemFn = syn::parse_str(&injection.declaration)
            .map_err(|e| format!("trait injection {}: {e}", injection.name))?;
        let event: syn::ItemStruct = syn::parse_str(&injection.event_declaration)
            .map_err(|e| format!("trait injection {}: {e}", injection.event))?;
        if method.sig.ident != injection.name || event.ident != injection.event {
            return Err("trait injection declaration does not match its name".into());
        }
        if !methods.insert(injection.name.clone()) {
            return Err(format!(
                "duplicate generated trait method {}",
                injection.name
            ));
        }
        if !events.insert(injection.event.clone()) {
            return Err(format!("duplicate generated event {}", injection.event));
        }
        hooks.items.push(syn::TraitItem::Fn(method));
        structs.push(syn::Item::Struct(event));
    }
    file.items.extend(structs);
    Ok(file.into_token_stream().to_string())
}
