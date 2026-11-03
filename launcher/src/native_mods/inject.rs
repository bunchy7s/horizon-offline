//! Trait-backed injections, prepared as build-script input alongside the staged game.
use super::{api, Bundle, MANIFEST};
use horizon_mod_api_gen::{TraitContract as Contract, TraitInjection as Injection};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

/// Replace trait injections with ordinary mixin helpers, leaving ordinary injections unchanged.
pub(super) fn prepare(mods: &[Bundle], source: &Path) -> Result<Vec<Injection>, String> {
    let mut result = Vec::new();
    let mut names = HashSet::new();
    let owners: HashMap<_, _> = api::with_api(mods)
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    // These are the copied bundles under mods.incoming, never the player's originals.
    for bundle in mods {
        api::strip(bundle)?;
        let path = bundle.dir.join(MANIFEST);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut manifest: toml::Table = text
            .parse()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let entries = manifest
            .get_mut("injection")
            .and_then(toml::Value::as_array_mut);
        if let Some(entries) = entries {
            for (index, entry) in entries.iter_mut().enumerate() {
                let table = entry.as_table_mut().ok_or("injection must be a table")?;
                let Some(method) = table.remove("trait_method") else {
                    continue;
                };
                if bundle.api.is_none() {
                    return Err(format!("{}: trait_method requires [api]", path.display()));
                }
                let target = if let Some(hook) = table.get("hook").and_then(toml::Value::as_str) {
                    let catalog = source.join("native-hooks.json");
                    let text = fs::read_to_string(&catalog)
                        .map_err(|e| format!("{}: {e}", catalog.display()))?;
                    let catalog: serde_json::Value =
                        serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    catalog["hooks"]
                        .as_array()
                        .and_then(|hooks| hooks.iter().find(|h| h["name"].as_str() == Some(hook)))
                        .and_then(|h| h["target"].as_str())
                        .ok_or_else(|| format!("unknown catalog hook {hook}"))?
                        .to_owned()
                } else {
                    table
                        .get("target")
                        .and_then(toml::Value::as_str)
                        .ok_or("trait_method needs a target or catalog hook")?
                        .to_owned()
                };
                let target_crate = target.split("::").next().unwrap_or_default();
                if target_crate != "horizon_client" {
                    return Err(format!("trait injection target {target:?} is in crate {target_crate:?}; only horizon-client targets are supported"));
                }
                if table.contains_key("helper") {
                    return Err(format!(
                        "{}: trait_method cannot also specify helper",
                        path.display()
                    ));
                }
                let method = method
                    .as_str()
                    .ok_or("trait_method must be a Rust identifier")?;
                let _: syn::Ident =
                    syn::parse_str(method).map_err(|_| "trait_method must be a Rust identifier")?;
                let name = format!("inject_{}_{}", bundle.id.replace('-', "_"), method);
                let ident: syn::Ident =
                    syn::parse_str(&name).map_err(|_| "invalid generated trait method")?;
                if !names.insert(name.clone()) {
                    return Err(format!("duplicate generated trait method {name}"));
                }
                let cancellable = match table.get("cancellable") {
                    None => false,
                    Some(v) => v.as_bool().ok_or("cancellable must be a boolean")?,
                };
                let mut parameters = Vec::new();
                let mut seen = HashSet::new();
                let mut value = None;
                if let Some(bindings) = table.get("bindings") {
                    for binding in bindings.as_array().ok_or("bindings must be an array")? {
                        let binding = binding.as_table().ok_or("binding must be a table")?;
                        let name = binding
                            .get("name")
                            .and_then(toml::Value::as_str)
                            .ok_or("binding needs a name")?;
                        let ident: syn::Ident =
                            syn::parse_str(name).map_err(|_| "invalid binding name")?;
                        if !seen.insert(name.to_owned()) {
                            return Err(format!("duplicate binding {name}"));
                        }
                        let ty = binding
                            .get("ty")
                            .and_then(toml::Value::as_str)
                            .ok_or("binding needs a type")?;
                        let ty: syn::Type =
                            syn::parse_str(ty).map_err(|e| format!("invalid binding type: {e}"))?;
                        if binding.get("value").and_then(toml::Value::as_str) == Some("$value") {
                            value = Some(ident.clone());
                        }
                        parameters.push((ident, ty));
                    }
                }
                let kind = table
                    .get("kind")
                    .and_then(toml::Value::as_str)
                    .unwrap_or_default();
                let contract = match (kind, cancellable) {
                    ("HEAD", false) => Contract::Listen,
                    ("HEAD", true) => Contract::Cancel,
                    ("RETURN" | "MODIFY_ARG" | "MODIFY_EXPRESSION_VALUE", false) => {
                        Contract::Value(value.ok_or_else(|| {
                            format!(
                                "{}: a {kind} trait_method needs a `$value` binding",
                                path.display()
                            )
                        })?)
                    }
                    (_, true) => {
                        return Err(format!(
                            "{}: cancellable trait_method needs HEAD",
                            path.display()
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "{}: trait_method supports HEAD, RETURN, MODIFY_ARG and MODIFY_EXPRESSION_VALUE",
                            path.display()
                        ));
                    }
                };
                let injection = Injection::new(ident, parameters, contract);
                let helper_path = format!("horizon-trait-injection-{index}.rs");
                let helper_file = bundle.dir.join(&helper_path);
                if helper_file.exists() {
                    return Err(format!(
                        "{} is reserved for generated helpers",
                        helper_file.display()
                    ));
                }
                fs::write(&helper_file, injection.helper(owners[bundle.id.as_str()]))
                    .map_err(|e| e.to_string())?;
                table.insert("helper".into(), helper_path.into());
                result.push(injection);
            }
        }
        fs::write(
            &path,
            toml::to_string(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(result)
}

pub(super) const INPUT: &str = ".horizon-mod-api-injections.json";
pub(super) const ENV: &str = "HORIZON_MOD_API_INJECTIONS";

/// New sources consume declarations in their build script; released sources still commit the
/// generated trait. Both paths use the same generation and collision checks, without rustfmt.
pub(super) fn stage(stage: &Path, injections: &[Injection]) -> Result<(), String> {
    let client = api::read(&stage.join("crates/horizon-client/Cargo.toml"))?;
    let build_time = client
        .get("build-dependencies")
        .and_then(toml::Value::as_table)
        .is_some_and(|deps| deps.contains_key("horizon-mod-api-gen"));
    let declarations = injections.iter().map(Injection::input).collect::<Vec<_>>();
    let input = serde_json::to_string(&declarations).map_err(|e| e.to_string())?;
    if build_time {
        // This file travels with cached mixin output; generated Rust lives in Cargo's OUT_DIR.
        let path = stage.join(INPUT);
        fs::write(&path, input).map_err(|e| format!("{}: {e}", path.display()))
    } else if injections.is_empty() {
        Ok(())
    } else {
        let path = stage.join("crates/horizon-client/src/mod_api/generated.rs");
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let output = horizon_mod_api_gen::inject(&source, &input)?;
        fs::write(&path, output).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, injection: &str) -> (std::path::PathBuf, Bundle) {
        let root =
            std::env::temp_dir().join(format!("horizon-inject-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("engine/Cargo.toml"),
            "[package]\nname = 'mod-example'\nversion = '1.0.0'\n",
        )
        .unwrap();
        fs::write(root.join("engine/src/lib.rs"), "").unwrap();
        fs::write(
            root.join(MANIFEST),
            format!("[mod]\nid = 'example'\nversion = '1'\n[api]\npath = 'engine'\n{injection}"),
        )
        .unwrap();
        let bundle = super::super::bundle(&root, None).unwrap();
        (root, bundle)
    }

    /// Emulate a released source tree or the current build script, including its inject() call.
    fn staged_hooks(
        stage_dir: &Path,
        injections: &[Injection],
        build_time: bool,
        existing: &str,
    ) -> Result<String, String> {
        let client = stage_dir.join("crates/horizon-client");
        fs::create_dir_all(client.join("src/mod_api")).unwrap();
        let dependencies = if build_time {
            "[build-dependencies]\nhorizon-mod-api-gen = { path = '../horizon-mod-api-gen' }\n"
        } else {
            ""
        };
        fs::write(
            client.join("Cargo.toml"),
            format!("[package]\nname = 'horizon-client'\nversion = '0.1.0'\n{dependencies}"),
        )
        .unwrap();
        let generated = client.join("src/mod_api/generated.rs");
        if !build_time {
            fs::write(&generated, existing).unwrap();
        }
        stage(stage_dir, injections)?;
        if build_time {
            assert!(!generated.exists());
            horizon_mod_api_gen::inject(
                existing,
                &fs::read_to_string(stage_dir.join(INPUT)).unwrap(),
            )
        } else {
            assert!(!stage_dir.join(INPUT).exists());
            Ok(fs::read_to_string(generated).unwrap())
        }
    }

    fn compile_and_run(root: &Path, program: &str) {
        let source = root.join("proof.rs");
        let binary = root.join(format!("proof-bin{}", std::env::consts::EXE_SUFFIX));
        fs::write(&source, program).unwrap();
        let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compiled = std::process::Command::new(compiler)
            .args(["--edition", "2024"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert!(std::process::Command::new(binary)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn owner_dispatch_and_cancel_are_generated_from_bindings() {
        let (root, bundle) = fixture("cancel", "[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = 'HEAD'\ntrait_method = 'tick'\ncancellable = true\nbindings = [{ name = 'amount', ty = '&mut i32', value = '&mut *amount' }]\n");
        let injections = prepare(&[bundle], &root).unwrap();
        let helper = fs::read_to_string(root.join("horizon-trait-injection-0.rs")).unwrap();
        let manifest = fs::read_to_string(root.join(MANIFEST)).unwrap();
        assert!(!manifest.contains("trait_method"));
        assert!(!manifest.contains("[api]"));
        for build_time in [false, true] {
            let stage_dir = root.join(if build_time { "build-time" } else { "legacy" });
            let output = staged_hooks(
                &stage_dir,
                &injections,
                build_time,
                "pub trait Hooks: Send + Sync {\n}\n",
            )
            .unwrap();
            assert!(syn::parse_file(&output).is_ok());
            let program = format!(
                r#"
            mod mod_api {{
                use std::sync::OnceLock;
                #[derive(PartialEq)] pub enum Flow {{ Continue, Cancel }}
                {output}
                struct Owner;
                impl Hooks for Owner {{
                    fn inject_example_tick(&self, event: &mut InjectExampleTickEvent) -> Flow {{
                        event.amount = event.amount * 2 + 40;
                        Flow::Cancel
                    }}
                }}
                struct Other;
                impl Hooks for Other {{
                    fn inject_example_tick(&self, _: &mut InjectExampleTickEvent) -> Flow {{ panic!("wrong owner") }}
                }}
                pub fn mods() -> &'static [Box<dyn Hooks>] {{
                    static MODS: OnceLock<Vec<Box<dyn Hooks>>> = OnceLock::new();
                    MODS.get_or_init(|| vec![Box::new(Owner), Box::new(Other)])
                }}
            }}
            {helper}
            fn main() {{
                let mut value = 1;
                assert!(matches!(horizon_trait_dispatch(&mut value), core::ops::ControlFlow::Break(())));
                assert_eq!(value, 42);
            }}
        "#
            );
            compile_and_run(&root, &program);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn value_injection_returns_what_the_owner_changed() {
        let (root, bundle) = fixture("value", "[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = 'RETURN'\ntrait_method = 'total'\nbindings = [{ name = 'total', ty = 'i32', value = '$value' }, { name = 'calls', ty = '&mut u32', value = '&mut *calls' }]\n");
        let injections = prepare(&[bundle], &root).unwrap();
        let helper = fs::read_to_string(root.join("horizon-trait-injection-0.rs")).unwrap();
        for build_time in [false, true] {
            let stage_dir = root.join(if build_time { "build-time" } else { "legacy" });
            let output = staged_hooks(
                &stage_dir,
                &injections,
                build_time,
                "pub trait Hooks: Send + Sync {\n}\n",
            )
            .unwrap();
            let program = format!(
                r#"
            mod mod_api {{
                use std::sync::atomic::{{AtomicBool, Ordering}};
                use std::sync::OnceLock;
                {output}
                struct Owner;
                impl Hooks for Owner {{
                    fn inject_example_total(&self, event: &mut InjectExampleTotalEvent) {{
                        event.total = event.total * 2 + 1;
                        event.calls += 1;
                    }}
                }}
                struct Other;
                impl Hooks for Other {{
                    fn inject_example_total(&self, _: &mut InjectExampleTotalEvent) {{ panic!("wrong owner") }}
                }}
                pub static INSTALLED: AtomicBool = AtomicBool::new(false);
                pub fn mods() -> &'static [Box<dyn Hooks>] {{
                    static MODS: OnceLock<Vec<Box<dyn Hooks>>> = OnceLock::new();
                    if !INSTALLED.load(Ordering::SeqCst) {{
                        return &[];
                    }}
                    MODS.get_or_init(|| vec![Box::new(Owner), Box::new(Other)])
                }}
            }}
            {helper}
            fn main() {{
                let mut calls = 0;
                assert_eq!(horizon_trait_dispatch(20, &mut calls), 20);
                assert_eq!(calls, 0);
                mod_api::INSTALLED.store(true, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(horizon_trait_dispatch(20, &mut calls), 41);
                assert_eq!(calls, 1);
            }}
        "#
            );
            compile_and_run(&root, &program);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_input_checks_conflicts_and_clears_removed_injections() {
        let (root, bundle) = fixture("conflicts", "[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = 'HEAD'\ntrait_method = 'tick'\n");
        let injections = prepare(&[bundle], &root).unwrap();
        for build_time in [false, true] {
            let stage_dir = root.join(if build_time { "build-time" } else { "legacy" });
            for (existing, expected) in [
                (
                    "pub trait Hooks: Send + Sync { fn inject_example_tick(&self) {} }",
                    "duplicate generated trait method",
                ),
                (
                    "pub trait Hooks: Send + Sync {} pub struct InjectExampleTickEvent;",
                    "duplicate generated event",
                ),
            ] {
                let error =
                    staged_hooks(&stage_dir, &injections, build_time, existing).unwrap_err();
                assert!(error.contains(expected), "{error}");
            }
            let output = staged_hooks(
                &stage_dir,
                &[],
                build_time,
                "pub trait Hooks: Send + Sync {}",
            )
            .unwrap();
            assert!(!output.contains("inject_example_tick"));
            if build_time {
                assert_eq!(fs::read_to_string(stage_dir.join(INPUT)).unwrap(), "[]");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trait_edits_do_not_invalidate_mixin_cache() {
        let (root, bundle) = fixture("cache", "[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = 'HEAD'\ntrait_method = 'tick'\n");
        let key = || {
            super::super::inject_cache::key(
                "revision",
                "mixins",
                super::super::Profile::Performance,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let before = key();
        fs::write(root.join("engine/src/lib.rs"), "// one line edit").unwrap();
        assert_eq!(before, key());
        fs::write(root.join("other.rs"), "// mixin input").unwrap();
        assert_ne!(before, key());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mixin_inputs_inside_api_are_not_excluded_from_cache() {
        let (root, bundle) = fixture("api-helper", "[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = 'HEAD'\nhelper = 'engine/src/lib.rs'\n");
        let key = || {
            super::super::inject_cache::key(
                "revision",
                "mixins",
                super::super::Profile::Performance,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let before = key();
        fs::write(root.join("engine/src/lib.rs"), "fn changed() {}\n").unwrap();
        assert_ne!(before, key());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mixin_crate_containing_api_is_not_excluded_from_cache() {
        let (root, bundle) = fixture("api-parent", "[[crate]]\nid = 'whole_bundle'\npath = '.'\n");
        let key = || {
            super::super::inject_cache::key(
                "revision",
                "mixins",
                super::super::Profile::Performance,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let before = key();
        fs::write(root.join("engine/src/lib.rs"), "// changed\n").unwrap();
        assert_ne!(before, key());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_build_key_tracks_dispatch_order() {
        let (root, first) = fixture("order", "");
        let mut second = first.clone();
        second.id = "second".into();
        let a = super::super::version_for(
            "0123456789012345678901234567890123456789",
            "mixins",
            super::super::Profile::Performance,
            super::super::Target::Client,
            &[first.clone(), second.clone()],
        )
        .unwrap();
        let b = super::super::version_for(
            "0123456789012345678901234567890123456789",
            "mixins",
            super::super::Profile::Performance,
            super::super::Target::Client,
            &[second, first],
        )
        .unwrap();
        assert_ne!(a, b);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trait_injections_reject_targets_in_other_crates() {
        let (root, bundle) = fixture("other-crate", "[[injection]]\nid = 'hook'\ntarget = 'horizon_world::loot::load'\nkind = 'HEAD'\ntrait_method = 'load'\n");
        let error = prepare(&[bundle], &root).err().unwrap();
        assert!(error.contains("horizon_world::loot::load") && error.contains("horizon_world"));
        fs::remove_dir_all(root).unwrap();

        let (root, bundle) = fixture("other-catalog", "[[injection]]\nid = 'hook'\nhook = 'loot_table_load'\nkind = 'HEAD'\ntrait_method = 'load'\n");
        fs::write(
            root.join("native-hooks.json"),
            r#"{"hooks":[{"name":"loot_table_load","target":"horizon_world::loot::load"}]}"#,
        )
        .unwrap();
        let error = prepare(&[bundle], &root).err().unwrap();
        assert!(error.contains("horizon_world::loot::load") && error.contains("horizon_world"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trait_injection_kinds_are_checked() {
        for (kind, extra, expected) in [
            (
                "AFTER_CALL",
                "",
                "supports HEAD, RETURN, MODIFY_ARG and MODIFY_EXPRESSION_VALUE",
            ),
            (
                "RETURN",
                "bindings = [{ name = 'amount', ty = 'i32', value = 'amount' }]\n",
                "`$value` binding",
            ),
            (
                "MODIFY_ARG",
                "cancellable = true\nbindings = [{ name = 'points', ty = 'i32', value = '$value' }]\n",
                "cancellable trait_method needs HEAD",
            ),
        ] {
            let (root, bundle) = fixture(
                "kinds",
                &format!("[[injection]]\nid = 'hook'\ntarget = 'horizon_client::game::tick'\nkind = '{kind}'\ntrait_method = 'tick'\n{extra}"),
            );
            let error = prepare(&[bundle], &root).err().unwrap();
            assert!(error.contains(expected), "{kind}: {error}");
            fs::remove_dir_all(root).unwrap();
        }
    }
}
