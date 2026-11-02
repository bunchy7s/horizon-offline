//! Generate the typed mod interface from native hook signatures and catalog contracts.
use quote::ToTokens;
use std::{collections::HashSet, fs, path::Path, process::Command};
use syn::{FnArg, ImplItem, Item, Pat, Type, visit::Visit, visit_mut::VisitMut};

fn impl_functions(item: syn::ItemImpl) -> impl Iterator<Item = syn::ItemFn> {
    item.items.into_iter().filter_map(|item| match item {
        ImplItem::Fn(method) => Some(syn::ItemFn {
            attrs: method.attrs,
            vis: method.vis,
            sig: method.sig,
            block: Box::new(method.block),
        }),
        _ => None,
    })
}

/// The inherent `Game` methods of the files under `path`, each with its module (`native_hooks`,
/// `lan::guest`…) below `module`.
fn game_methods(
    path: &Path,
    module: &str,
    methods: &mut Vec<(String, syn::ItemFn)>,
) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let inner = format!("{module}::{stem}");
        if path.is_dir() {
            game_methods(&path, &inner, methods)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            for item in file.items {
                if let Item::Impl(item) = item
                    && item.trait_.is_none()
                    && let Type::Path(ty) = &*item.self_ty
                    && ty.path.segments.len() == 1
                    && ty.path.segments[0].ident == "Game"
                {
                    methods.extend(impl_functions(item).map(|method| (inner.clone(), method)));
                }
            }
        }
    }
    Ok(())
}

fn doc_comments(
    attrs: &[syn::Attribute],
    name: &str,
    preserve_empty: bool,
) -> Result<String, String> {
    let mut docs = String::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("doc")) {
        let syn::Meta::NameValue(value) = &attr.meta else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(doc),
            ..
        }) = &value.value
        else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let value = doc.value();
        for line in value.split('\n') {
            if preserve_empty || !line.is_empty() {
                docs.push_str(&format!("///{line}\n"));
            }
        }
    }
    Ok(docs)
}

fn exported_types(source: &str, generated: &str) -> Result<HashSet<String>, String> {
    fn use_names(tree: &syn::UseTree, names: &mut HashSet<String>) {
        match tree {
            syn::UseTree::Path(path) => use_names(&path.tree, names),
            syn::UseTree::Name(name) => {
                names.insert(name.ident.to_string());
            }
            syn::UseTree::Rename(rename) => {
                names.insert(rename.rename.to_string());
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    use_names(item, names);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut names = HashSet::new();
    for source in [source, generated] {
        let file = syn::parse_file(source).map_err(|e| e.to_string())?;
        for item in file.items {
            let ident = match item {
                Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    use_names(&item.tree, &mut names);
                    None
                }
                Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                Item::Union(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                Item::Mod(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    Some(item.ident)
                }
                _ => None,
            };
            if let Some(ident) = ident {
                names.insert(ident.to_string());
            }
        }
    }
    Ok(names)
}

fn host_method(
    method: &syn::ItemFn,
    name: &str,
    exports: &HashSet<String>,
) -> Result<(String, String), String> {
    struct Types<'a> {
        exports: &'a HashSet<String>,
        error: Option<String>,
    }
    impl Types<'_> {
        fn check_path(&mut self, path: &syn::Path) {
            let first = path.segments.first().unwrap().ident.to_string();
            let unqualified = path.leading_colon.is_none() && path.segments.len() == 1;
            let prelude = unqualified
                && matches!(
                    first.as_str(),
                    "bool"
                        | "char"
                        | "str"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                        | "f32"
                        | "f64"
                        | "String"
                        | "Option"
                        | "Result"
                        | "Vec"
                        | "Box"
                        | "Arc"
                        | "PathBuf"
                        | "AsRef"
                        | "AsMut"
                        | "From"
                        | "Into"
                        | "TryFrom"
                        | "TryInto"
                        | "Iterator"
                        | "IntoIterator"
                        | "DoubleEndedIterator"
                        | "ExactSizeIterator"
                        | "Extend"
                        | "Fn"
                        | "FnMut"
                        | "FnOnce"
                        | "Sized"
                        | "Send"
                        | "Sync"
                        | "Unpin"
                        | "Drop"
                        | "Default"
                        | "Clone"
                        | "Copy"
                        | "PartialEq"
                        | "PartialOrd"
                        | "Eq"
                        | "Ord"
                        | "ToOwned"
                        | "ToString"
                        | "Future"
                        | "IntoFuture"
                );
            if path.leading_colon.is_some() || !(prelude || self.exports.contains(&first)) {
                self.error = Some(format!(
                    "type {} is not available in mod_api",
                    path.to_token_stream()
                ));
            }
        }
    }
    impl<'ast> Visit<'ast> for Types<'_> {
        fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
            self.error = Some("impl Trait is not object-safe".into());
        }
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if ty.qself.is_some() {
                self.error = Some("qualified associated types are not supported".into());
            } else {
                self.check_path(&ty.path);
            }
            syn::visit::visit_type_path(self, ty);
        }
        fn visit_trait_bound(&mut self, bound: &'ast syn::TraitBound) {
            self.check_path(&bound.path);
            syn::visit::visit_trait_bound(self, bound);
        }
    }
    if method.sig.asyncness.is_some() {
        return Err(format!("{name}: async methods are not object-safe"));
    }
    if method
        .sig
        .generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
    {
        return Err(format!("{name}: generic methods are not object-safe"));
    }
    let mut types = Types {
        exports,
        error: None,
    };
    for arg in &method.sig.inputs {
        if let FnArg::Typed(arg) = arg {
            types.visit_type(&arg.ty);
        }
    }
    if let syn::ReturnType::Type(_, ty) = &method.sig.output {
        types.visit_type(ty);
    }
    if let Some(reason) = types.error {
        return Err(format!("{name}: {reason}"));
    }
    let mut sig = method.sig.clone();
    let original = sig.ident.clone();
    sig.ident = syn::parse_str(name).map_err(|e| format!("invalid helper name {name}: {e}"))?;
    let mut args = Vec::new();
    if !matches!(sig.inputs.first(), Some(FnArg::Receiver(r)) if r.reference.is_some()) {
        return Err(format!("{name}: expected a borrowed Game receiver"));
    }
    for arg in sig.inputs.iter_mut().skip(1) {
        let FnArg::Typed(arg) = arg else {
            return Err(format!("{name}: unexpected receiver"));
        };
        let Pat::Ident(pat) = &mut *arg.pat else {
            return Err(format!("{name}: unsupported argument"));
        };
        pat.mutability = None;
        args.push(pat.ident.to_string());
    }
    let docs = doc_comments(&method.attrs, name, true)?;
    Ok((
        format!("{docs}{};\n", sig.to_token_stream()),
        format!(
            "{} {{ self.{original}({}) }}\n",
            sig.to_token_stream(),
            args.join(", ")
        ),
    ))
}

fn has_call(block: &syn::Block, expected: &str) -> bool {
    struct Calls<'a> {
        expected: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == self.expected)
            {
                self.found = true;
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut calls = Calls {
        expected,
        found: false,
    };
    calls.visit_block(block);
    calls.found
}

fn one_function<'a>(
    items: impl Iterator<Item = &'a syn::ItemFn>,
    name: &str,
) -> Result<&'a syn::ItemFn, String> {
    let mut matches = items.filter(|item| item.sig.ident == name);
    let first = matches
        .next()
        .ok_or_else(|| format!("expected one {name} signature"))?;
    if matches.next().is_some() {
        return Err(format!("expected one {name} signature"));
    }
    Ok(first)
}

fn fresh_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_owned();
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

/// Turns a hook argument's type into its event field's. A `&mut` primitive scalar becomes the value
/// itself, written back after every mod has run; any other `&mut` stays a reference to the caller's
/// value. Either way one event goes through every mod, so each sees the changes before it. Every
/// elided reference lifetime becomes the event's `'a`, or `'b` inside a `&mut`, which can't shorten
/// it. The launcher keeps a copy for trait injections (`launcher/src/native_mods/inject.rs`); both
/// test the same types.
#[derive(Default)]
struct EventLifetimes {
    a: bool,
    b: bool,
    in_mut: bool,
}

impl EventLifetimes {
    /// The field's type, and whether it is a scalar copied in and written back.
    fn field(&mut self, ty: &Type) -> (Type, bool) {
        let scalar = match ty {
            Type::Reference(reference) if reference.mutability.is_some() => {
                match &*reference.elem {
                    Type::Path(path)
                        if path.qself.is_none()
                            && path.path.get_ident().is_some_and(|ident| {
                                [
                                    "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize",
                                    "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
                                ]
                                .iter()
                                .any(|scalar| ident == scalar)
                            }) =>
                    {
                        Some(&*reference.elem)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(scalar) = scalar {
            return (scalar.clone(), true);
        }
        let mut field = ty.clone();
        self.visit_type_mut(&mut field);
        (field, false)
    }
}

impl VisitMut for EventLifetimes {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(syn::parse_quote!('_));
        }
        let outer = self.in_mut;
        self.visit_lifetime_mut(reference.lifetime.as_mut().unwrap());
        self.in_mut |= reference.mutability.is_some();
        self.visit_type_mut(&mut reference.elem);
        self.in_mut = outer;
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = if self.in_mut {
                self.b = true;
                syn::parse_quote!('b)
            } else {
                self.a = true;
                syn::parse_quote!('a)
            };
        }
    }
}

fn host_for(is_level: bool, mutable: Option<bool>, ticks: bool) -> Option<(&'static str, String)> {
    mutable.map(|mutable| {
        let (name, kind) = if is_level {
            (
                "level",
                match (mutable, ticks) {
                    (true, true) => "LevelWriteHost",
                    (false, true) => "LevelTicksHost",
                    (true, false) => "LevelRandomHost",
                    (false, false) => "LevelHost",
                },
            )
        } else {
            ("host", "Host")
        };
        let mutable = if mutable || (is_level && ticks) {
            "mut "
        } else {
            ""
        };
        (name, format!("&{mutable}dyn {kind}"))
    })
}

pub fn run(check: bool) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let catalog: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("native-hooks.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let source = fs::read_to_string(root.join("crates/horizon-client/src/game/native_hooks.rs"))
        .map_err(|e| e.to_string())?;
    let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
    let mut methods = Vec::new();
    for item in file.items {
        if let Item::Impl(item) = item {
            methods.extend(impl_functions(item));
        }
    }
    let mut traits = String::new();
    let mut dispatch = String::new();
    let mut render_traits = String::new();
    let mut render_dispatch = String::new();
    let mut seen = HashSet::new();
    for hook in catalog["hooks"]
        .as_array()
        .ok_or("catalog hooks must be an array")?
    {
        let Some(api) = hook.get("mod_api") else {
            continue;
        };
        let name = hook["name"].as_str().ok_or("hook name missing")?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate hook {name}"));
        }
        let native = format!("native_{name}");
        let target = hook["target"].as_str().ok_or("target missing")?;
        let is_render = target.starts_with("horizon_client_render::");
        let external = target
            .strip_prefix("horizon_world::")
            .map(|p| ("horizon-world", p))
            .or_else(|| {
                target
                    .strip_prefix("horizon_client_render::")
                    .map(|p| ("horizon-client-render", p))
            });
        let (method, call) = if let Some((krate, path)) = external {
            let Some((module, function)) = path.rsplit_once("::") else {
                return Err(format!("wrong target for {name}"));
            };
            if function != native {
                return Err(format!("wrong target for {name}"));
            }
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
            let functions: Vec<_> = file
                .items
                .into_iter()
                .filter_map(|item| match item {
                    Item::Fn(function) => Some(function),
                    _ => None,
                })
                .collect();
            (
                one_function(functions.iter(), &native)?.clone(),
                if is_render {
                    format!("dispatch_{name}")
                } else {
                    name.to_owned()
                },
            )
        } else {
            if !target.starts_with("horizon_client::game::native_hooks::")
                || !target.ends_with(&format!("::{native}"))
            {
                return Err(format!("wrong target for {name}"));
            }
            (
                one_function(methods.iter(), &native)?.clone(),
                format!("dispatch_{name}"),
            )
        };
        if !has_call(&method.block, &call) {
            return Err(format!("missing {call} call in {native}"));
        }
        let sig = &method.sig;
        let mut args = Vec::new();
        let mut receiver_mut = None;
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(r) => {
                    receiver_mut = Some(r.mutability.is_some());
                }
                FnArg::Typed(arg) => {
                    let Pat::Ident(pat) = &*arg.pat else {
                        return Err(format!("unsupported argument in {name}"));
                    };
                    args.push((pat.ident.to_string(), &*arg.ty));
                }
            }
        }
        let is_level = hook["target"]
            .as_str()
            .is_some_and(|target| target.contains("::GameLevel<"));
        let ticks = is_level
            && args.iter().any(|(name, ty)| {
                name == "ticks" && ty.to_token_stream().to_string() == "& mut Ticks"
            });
        if ticks {
            args.retain(|(name, _)| name != "ticks");
        }
        let mut used: HashSet<_> = args.iter().map(|(name, _)| name.clone()).collect();
        let mod_list = fresh_name("mods", &mut used);
        let mod_item = fresh_name("mod_hook", &mut used);
        let replacement = fresh_name("replacement", &mut used);
        let contract = api["contract"].as_str().ok_or("contract missing")?;
        let out = api.get("out").and_then(|v| v.as_str());
        if !matches!(
            (contract, out),
            ("cancel" | "replace" | "collect", Some(_)) | ("listen", None)
        ) {
            return Err(format!("invalid contract for {name}"));
        }
        let output = if let Some(out) = out {
            let index = args
                .iter()
                .position(|(n, _)| n == out)
                .ok_or_else(|| format!("missing output {out} in {name}"))?;
            Some(args.remove(index))
        } else {
            None
        };
        let result = if let Some((_, ty)) = &output {
            let Type::Reference(reference) = ty else {
                return Err(format!("output must be a mutable reference in {name}"));
            };
            if reference.mutability.is_none() {
                return Err(format!("output must be a mutable reference in {name}"));
            }
            match contract {
                "cancel" if reference.elem.to_token_stream().to_string() == "bool" => {
                    " -> Flow".to_owned()
                }
                "replace" => format!(" -> {}", reference.elem.to_token_stream()),
                "collect" => String::new(),
                _ => return Err(format!("unexpected output type for {name}")),
            }
        } else {
            String::new()
        };
        let host = host_for(is_level, receiver_mut, ticks)
            .map(|(name, ty)| (fresh_name(name, &mut used), ty));
        let event = fresh_name("event", &mut used);
        let event_type = format!(
            "{}Event",
            name.split('_')
                .map(|word| word[..1].to_uppercase() + &word[1..])
                .collect::<String>()
        );
        let mut trait_params = vec!["&self".to_owned()];
        let mut dispatch_params = Vec::new();
        let mut call_args = Vec::new();
        if let Some((name, ty)) = &host {
            trait_params.push(format!("{name}: {ty}"));
            dispatch_params.push(format!("{name}: {ty}"));
            call_args.push(name.clone());
        }
        dispatch_params.push(if is_render {
            format!("{mod_list}: &[&dyn RenderHooks]")
        } else {
            format!("{mod_list}: &[Box<dyn Hooks>]")
        });
        let mut lifetimes = EventLifetimes::default();
        let mut fields = String::new();
        let mut inits = Vec::new();
        let mut write_back = String::new();
        let collected = output.as_ref().filter(|_| contract == "collect");
        for (name, ty) in args.iter().chain(collected) {
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
            let (field, scalar) = lifetimes.field(ty);
            fields.push_str(&format!("pub {name}: {},\n", field.to_token_stream()));
            if scalar {
                inits.push(format!("{name}: *{name}"));
                write_back.push_str(&format!("*{name} = {event}.{name};"));
            } else {
                inits.push(name.clone());
            }
        }
        if let Some((name, ty)) = output.as_ref().filter(|_| contract != "collect") {
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
        }
        let (generics, elided) = match (lifetimes.a, lifetimes.b) {
            (_, true) => ("<'a, 'b>", "<'_, '_>"),
            (true, false) => ("<'a>", "<'_>"),
            (false, false) => ("", ""),
        };
        trait_params.push(format!("{event}: &mut {event_type}{elided}"));
        call_args.push(format!("&mut {event}"));
        let unused = trait_params[1..]
            .iter()
            .map(|param| format!("let _ = {};", param.split(':').next().unwrap()))
            .collect::<String>();
        let default = match contract {
            "cancel" => "Flow::Continue",
            "replace" => "None",
            _ => "",
        };
        let invoke = call_args.join(", ");
        let trait_params = trait_params.join(", ");
        let dispatch_params = dispatch_params.join(", ");
        let trait_name = if is_render { "RenderHooks" } else { "Hooks" };
        let traits = if is_render {
            &mut render_traits
        } else {
            &mut traits
        };
        let dispatch = if is_render {
            &mut render_dispatch
        } else {
            &mut dispatch
        };
        traits.push_str(&doc_comments(&method.attrs, name, false)?);
        match (contract, out) {
            ("cancel", Some(out)) => traits.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later mods are skipped.\n"
            )),
            ("replace", Some(out)) => traits.push_str(&format!(
                "///\n/// Return `Some` to set `{out}`; the first mod returning `Some` wins.\n"
            )),
            ("collect", Some(out)) => traits.push_str(&format!(
                "///\n/// Every mod gets `{event}.{out}` in order and may add, change or remove {out}.\n"
            )),
            _ => {}
        }
        traits.push_str(&format!(
            "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
        ));
        dispatch.push_str(&format!(
            "/// What [`{trait_name}::{name}`] is told. Fields may be added: the struct is `#[non_exhaustive]`.\n#[non_exhaustive]\npub struct {event_type}{generics} {{\n{fields}}}\n"
        ));
        let build = format!("let mut {event} = {event_type} {{ {} }};", inits.join(", "));
        let body = match contract {
            "cancel" => format!(
                "if *{o} {{ return; }} {build} for {mod_item} in {mod_list} {{ if matches!({mod_item}.{name}({invoke}), Flow::Cancel) {{ *{o} = true; break; }} }} {write_back}",
                o = out.unwrap()
            ),
            "replace" => format!(
                "if {o}.is_some() {{ return; }} {build} for {mod_item} in {mod_list} {{ if let Some({replacement}) = {mod_item}.{name}({invoke}) {{ *{o} = Some({replacement}); break; }} }} {write_back}",
                o = out.unwrap()
            ),
            _ => format!(
                "{build} for {mod_item} in {mod_list} {{ {mod_item}.{name}({invoke}); }} {write_back}"
            ),
        };
        dispatch.push_str(&format!(
            "pub fn dispatch_{name}({dispatch_params}) {{ {body} }}\n"
        ));
    }
    if seen.is_empty() {
        return Err("no mod API contracts".into());
    }
    let code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\npub trait Hooks: Send + Sync {{\nfn render(&self) -> Option<&dyn RenderHooks> {{ None }}\n{traits}}}\n{dispatch}"
    );
    let api_source = fs::read_to_string(root.join("crates/horizon-client/src/mod_api.rs"))
        .map_err(|e| e.to_string())?;
    let exports = exported_types(&api_source, &code)?;
    let mut game = Vec::new();
    game_methods(
        &root.join("crates/horizon-client/src/game"),
        "horizon_client::game",
        &mut game,
    )?;
    let mut host_traits = String::new();
    let mut host_impl = String::new();
    let mut names = HashSet::new();
    for helper in catalog["helpers"]
        .as_array()
        .ok_or("catalog helpers must be an array")?
    {
        let name = helper["name"].as_str().ok_or("helper name missing")?;
        if !names.insert(name) {
            return Err(format!("duplicate helper {name}"));
        }
        let native = format!("native_{name}");
        let target = helper["target"].as_str().ok_or("helper target missing")?;
        let method = one_function(game.iter().map(|(_, method)| method), &native)?;
        let (module, _) = game
            .iter()
            .find(|(_, other)| std::ptr::eq(other, method))
            .expect("the method was found in the list");
        let expected = format!("{module}::Game<'_>::{native}");
        if target != expected {
            return Err(format!(
                "wrong target for helper {name}: {target}, it is {expected}"
            ));
        }
        let (declaration, implementation) = host_method(method, name, &exports)?;
        host_traits.push_str(&declaration);
        host_impl.push_str(&implementation);
    }
    struct StdTypes {
        path_buf: bool,
        arc: bool,
    }
    impl<'ast> Visit<'ast> for StdTypes {
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if ty.path.segments.len() == 1 {
                self.path_buf |= ty.path.segments[0].ident == "PathBuf";
                self.arc |= ty.path.segments[0].ident == "Arc";
            }
            syn::visit::visit_type_path(self, ty);
        }
    }
    let host_trait: syn::ItemTrait = syn::parse_str(&format!("trait Host {{ {host_traits} }}"))
        .map_err(|e| format!("generated Host: {e}"))?;
    let mut std_types = StdTypes {
        path_buf: false,
        arc: false,
    };
    std_types.visit_item_trait(&host_trait);
    let mut imports = String::new();
    if std_types.path_buf {
        imports.push_str("use std::path::PathBuf;\n");
    }
    if std_types.arc {
        imports.push_str("use std::sync::Arc;\n");
    }
    let host = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\n{imports}/// Services supplied by the game to trait mods.\npub trait Host {{\n{host_traits}}}\n"
    );
    let implementation = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse horizon_client::mod_api::*;\n{imports}impl Host for super::Game<'_> {{\n{host_impl}}}\n"
    );
    let render_code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\npub trait RenderHooks: Send + Sync {{\n{render_traits}}}\n{render_dispatch}"
    );
    for (relative, code) in [
        (
            "crates/horizon-client-render/src/gfx/native_render/generated.rs",
            render_code,
        ),
        ("crates/horizon-client/src/mod_api/generated.rs", code),
        ("crates/horizon-client/src/mod_api/generated_host.rs", host),
        (
            "crates/horizon-client/src/game/mod_api_host_generated.rs",
            implementation,
        ),
    ] {
        // Parse before formatting, so invalid catalog names or types fail without writing output.
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {e}"))?;
        let mut child = Command::new("rustfmt")
            .args(["--emit", "stdout", "--edition", "2021"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        child
            .stdin
            .take()
            .ok_or("rustfmt stdin")?
            .write_all(code.as_bytes())
            .map_err(|e| e.to_string())?;
        let formatted = child.wait_with_output().map_err(|e| e.to_string())?;
        if !formatted.status.success() {
            return Err("rustfmt failed".into());
        }
        let path = root.join(relative);
        if check {
            if fs::read(&path).map_err(|e| e.to_string())? != formatted.stdout {
                return Err("mod API output is stale; run cargo run -p xtask -- mod-api".into());
            }
        } else {
            fs::create_dir_all(path.parent().ok_or("output parent missing")?)
                .map_err(|e| e.to_string())?;
            fs::write(path, formatted.stdout).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct ItemStack;
    struct BlockEntityDraw(Vec<u32>);
    #[allow(dead_code)]
    mod stubs {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub(super) struct BlockStateId;
        #[derive(Clone, Copy)]
        pub(super) struct FluidState;
        pub(super) struct PlacementContext;
        #[derive(Clone, Copy)]
        pub(super) struct BlockPos;
        #[derive(Clone, Copy)]
        pub(super) struct BlockId;
        #[derive(Clone, Copy)]
        pub(super) struct Direction;
        #[derive(Clone, Copy)]
        pub(super) struct LevelDimension;
        pub(super) struct Ticks;
        pub(super) trait LevelHost {}
        pub(super) trait LevelTicksHost: LevelHost {}
        pub(super) trait LevelRandomHost: LevelHost {}
        pub(super) trait LevelWriteHost: LevelTicksHost + LevelRandomHost {}
        pub(super) struct Compound;
        pub(super) type NativeDispensed = (Option<super::ItemStack>, i32);
        pub(super) struct NativeLootDrops<'a>(pub &'a ());
        pub(super) struct NativeScreenView<'a>(pub &'a ());
        pub(super) struct Corners;
        pub(super) struct HudBatch;
        pub(super) struct NativeBlockRim;
        pub(super) struct NativeColoredLight;
        pub(super) struct NativeMinimap;
        pub(super) struct NativeMod;
        pub(super) struct NativePanelEvent;
        pub(super) struct NativeScreen;
        pub(super) struct ModBlockEntity;
        pub(super) struct ModContainer;
        pub(super) struct ModFluidTanks;
        pub(super) struct NativeCommand;
        pub(super) struct NativeLifecycleContext;
        pub(super) struct NativeServerPlayer;
        #[derive(Clone, Copy)]
        pub(super) struct ServerTickPhase;
        #[derive(Clone, Copy)]
        pub(super) struct WorldPhase;
    }
    use serde_json::{Map, Value};
    use stubs::*;
    struct Text;
    #[derive(Debug, PartialEq)]
    struct NativeZoom(u32);
    enum Flow {
        Continue,
        Cancel,
    }
    trait Host {}
    trait RenderHooks {}
    struct TestHost;
    impl Host for TestHost {}
    #[allow(dead_code)]
    mod generated {
        mod horizon_client {
            pub mod hud_layout {
                pub struct HealthInputs;
                pub struct GuiDraw;
            }
        }
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/horizon-client/src/mod_api/generated.rs"
        ));
    }
    use generated::*;
    struct Mod {
        id: u32,
        calls: Arc<Mutex<Vec<u32>>>,
    }
    impl Hooks for Mod {
        fn block_entity_submit(&self, _: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
            assert_eq!(event.pos, [1, 2, 3]);
            assert_eq!(event.block, "example:machine");
            event.draw.0.push(self.id);
        }
        fn loot_table_load(&self, event: &mut LootTableLoadEvent<'_>) {
            event.table.as_array_mut().unwrap().push(self.id.into());
        }
        fn modify_default_components(&self, event: &mut ModifyDefaultComponentsEvent<'_>) {
            let value = event
                .components
                .entry("order")
                .or_insert_with(|| serde_json::json!([]));
            value.as_array_mut().unwrap().push(self.id.into());
        }
        fn level_tick_pre(&self, _: &mut dyn Host, _: &mut LevelTickPreEvent<'_>) {
            self.calls.lock().unwrap().push(self.id);
        }
        fn tooltip(&self, _: &dyn Host, event: &mut TooltipEvent<'_>) {
            event.lines.push((Text, self.id));
        }
        fn zoom(&self, _: &dyn Host, _: &mut ZoomEvent) -> Option<NativeZoom> {
            self.calls.lock().unwrap().push(self.id);
            Some(NativeZoom(self.id))
        }
        fn should_apply_effect_tick_this_tick(
            &self,
            _: &dyn Host,
            event: &mut ShouldApplyEffectTickThisTickEvent<'_>,
        ) {
            event.apply = !event.apply;
            self.calls.lock().unwrap().push(self.id);
        }
        fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
            event.amount = event.amount * 2.0 + self.id as f32;
            if self.id == 2 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn use_item_on(&self, _: &mut dyn Host, _: &mut UseItemOnEvent<'_>) -> Flow {
            self.calls.lock().unwrap().push(self.id);
            if self.id == 1 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    #[test]
    fn block_entity_submit_dispatches_in_order_with_shared_draw() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Mod {
                id: 1,
                calls: calls.clone(),
            }),
            Box::new(Mod { id: 2, calls }),
        ];
        let mut draw = BlockEntityDraw(Vec::new());
        dispatch_block_entity_submit(
            &TestHost,
            &mods,
            [1, 2, 3],
            "example:machine",
            BlockStateId,
            &ModBlockEntity,
            &mut draw,
        );
        assert_eq!(draw.0, [1, 2]);
    }

    #[test]
    fn host_mapping_preserves_access_and_docs_and_rejects_unavailable_types() {
        let exports = super::exported_types(
            "pub use external::{Visible, Original as Renamed}; pub struct Defined; struct Hidden;",
            "pub struct Generated;",
        )
        .unwrap();
        for ty in ["Visible", "Renamed", "Defined", "Generated"] {
            let method: syn::ItemFn =
                syn::parse_str(&format!("fn native_test(&self, value: {ty}) {{ }}")).unwrap();
            assert!(super::host_method(&method, "test", &exports).is_ok());
        }
        let foreign: syn::ItemFn =
            syn::parse_quote! { fn native_foreign(&self, value: elsewhere::Visible) {} };
        assert!(super::host_method(&foreign, "foreign", &exports).is_err());
        let hidden_trait: syn::ItemFn =
            syn::parse_quote! { fn native_hidden(&self, value: &dyn Hidden) {} };
        assert!(super::host_method(&hidden_trait, "hidden", &exports).is_err());
        let read: syn::ItemFn = syn::parse_quote! {
            /// Read a value exactly.
            ///
            /// Keep the blank line too.
            fn native_read(&self, pos: [i32; 3]) -> String { todo!() }
        };
        let (declaration, implementation) = super::host_method(&read, "read", &exports).unwrap();
        assert!(
            declaration.contains("/// Read a value exactly.\n///\n/// Keep the blank line too.")
        );
        assert!(declaration.contains("& self"));
        assert!(implementation.contains("self.native_read(pos)"));
        let write: syn::ItemFn = syn::parse_quote! {
            fn native_write(&mut self, value: i32) { }
        };
        let (declaration, _) = super::host_method(&write, "write", &exports).unwrap();
        assert!(declaration.contains("& mut self"));
        let private: syn::ItemFn = syn::parse_quote! {
            fn native_private(&self, value: BinOnly) { }
        };
        assert!(
            super::host_method(&private, "private", &exports)
                .unwrap_err()
                .contains("BinOnly")
        );
        let generic: syn::ItemFn = syn::parse_quote! {
            fn native_generic<T>(&self, value: T) { }
        };
        assert!(
            super::host_method(&generic, "generic", &exports)
                .unwrap_err()
                .contains("object-safe")
        );
        let callback: syn::ItemFn = syn::parse_quote! {
            fn native_callback(&self, callback: impl FnOnce()) { }
        };
        assert!(
            super::host_method(&callback, "callback", &exports)
                .unwrap_err()
                .contains("object-safe")
        );
        let asynchronous: syn::ItemFn = syn::parse_quote! {
            async fn native_async(&self) { }
        };
        assert!(
            super::host_method(&asynchronous, "async", &exports)
                .unwrap_err()
                .contains("object-safe")
        );
    }

    #[test]
    fn lookup_rejects_missing_and_duplicate_functions() {
        let first: syn::ItemFn = syn::parse_quote! { fn native_example() {} };
        let other: syn::ItemFn = syn::parse_quote! { fn unrelated() {} };
        assert!(super::one_function([&other].into_iter(), "native_example").is_err());
        assert!(super::one_function([&first, &other].into_iter(), "native_example").is_ok());
        assert!(super::one_function([&first, &first].into_iter(), "native_example").is_err());
    }

    #[test]
    fn placement_replacement_can_refuse_and_stops_later_mods() {
        struct Level;
        impl LevelHost for Level {}
        struct Refuse(Arc<Mutex<Vec<u32>>>, u32);
        impl Hooks for Refuse {
            fn state_for_placement(
                &self,
                _: &dyn LevelHost,
                _: &mut StateForPlacementEvent<'_>,
            ) -> Option<Option<BlockStateId>> {
                self.0.lock().unwrap().push(self.1);
                Some(None)
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Refuse(calls.clone(), 1)),
            Box::new(Refuse(calls.clone(), 2)),
        ];
        let mut answer = None;
        dispatch_state_for_placement(
            &Level,
            &mods,
            LevelDimension,
            BlockStateId,
            &PlacementContext,
            FluidState,
            &mut answer,
        );
        assert_eq!(answer, Some(None));
        assert_eq!(*calls.lock().unwrap(), vec![1]);
        dispatch_state_for_placement(
            &Level,
            &mods,
            LevelDimension,
            BlockStateId,
            &PlacementContext,
            FluidState,
            &mut answer,
        );
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn generated_locals_do_not_shadow_hook_arguments() {
        let mut used = ["mods", "mods_", "mod_hook", "replacement", "host"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(super::fresh_name("mods", &mut used), "mods__");
        assert_eq!(super::fresh_name("mod_hook", &mut used), "mod_hook_");
        assert_eq!(super::fresh_name("replacement", &mut used), "replacement_");
        assert_eq!(super::fresh_name("host", &mut used), "host_");
    }

    #[test]
    fn level_hosts_match_receiver_mutability() {
        assert_eq!(
            super::host_for(true, Some(false), false),
            Some(("level", "&dyn LevelHost".into()))
        );
        assert_eq!(
            super::host_for(true, Some(true), false),
            Some(("level", "&mut dyn LevelRandomHost".into()))
        );
        assert_eq!(
            super::host_for(true, Some(false), true),
            Some(("level", "&mut dyn LevelTicksHost".into()))
        );
        assert_eq!(
            super::host_for(true, Some(true), true),
            Some(("level", "&mut dyn LevelWriteHost".into()))
        );
        assert_eq!(super::host_for(true, None, false), None);
        assert_eq!(
            super::host_for(false, Some(true), false),
            Some(("host", "&mut dyn Host".into()))
        );
    }

    #[test]
    fn event_fields_name_their_lifetimes() {
        for (ty, field, scalar, a, b) in [
            (
                "& mut WorldDrawn < '_ >",
                "& 'a mut WorldDrawn < 'b >",
                false,
                true,
                true,
            ),
            ("& str", "& 'a str", false, true, false),
            (
                "& mut Vec < (& 'static str , u32) >",
                "& 'a mut Vec < (& 'static str , u32) >",
                false,
                true,
                false,
            ),
            (
                "Option < & ItemStack >",
                "Option < & 'a ItemStack >",
                false,
                true,
                false,
            ),
            ("u32", "u32", false, false, false),
            ("& mut f32", "f32", true, false, false),
            (
                "& mut Option < bool >",
                "& 'a mut Option < bool >",
                false,
                true,
                false,
            ),
        ] {
            let mut lifetimes = super::EventLifetimes::default();
            let (field_type, is_scalar) = lifetimes.field(&syn::parse_str(ty).unwrap());
            assert_eq!(field_type.to_token_stream().to_string(), field);
            assert_eq!((is_scalar, lifetimes.a, lifetimes.b), (scalar, a, b));
        }
    }

    #[test]
    fn generated_output_is_current() {
        super::run(true).unwrap();
    }

    #[test]
    fn contracted_hook_requires_dispatch_call() {
        let missing: syn::ImplItemFn = syn::parse_quote! { fn native_zoom(&self) { unrelated(); } };
        let present: syn::ImplItemFn = syn::parse_quote! { fn native_zoom(&self) { horizon_client::mod_api::dispatch_zoom(); } };
        assert!(!super::has_call(&missing.block, "dispatch_zoom"));
        assert!(super::has_call(&present.block, "dispatch_zoom"));
    }

    #[test]
    fn dispatch_contracts() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = (1..=2)
            .map(|id| {
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .collect();
        let mut table = serde_json::json!([]);
        dispatch_loot_table_load(&mods, "table", &mut table);
        assert_eq!(table, serde_json::json!([1, 2]));
        let mut components = Map::new();
        dispatch_modify_default_components(&mods, "item", &mut components);
        assert_eq!(components["order"], serde_json::json!([1, 2]));
        let mut host = TestHost;
        dispatch_level_tick_pre(&mut host, &mods, "overworld");
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        calls.lock().unwrap().clear();
        let mut zoom = None;
        dispatch_zoom(&host, &mods, &mut zoom);
        assert_eq!(zoom, Some(NativeZoom(1)));
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_zoom(&host, &mods, &mut zoom);
        assert!(calls.lock().unwrap().is_empty());
        let mut lines = Vec::new();
        dispatch_tooltip(&host, &mods, &ItemStack, &mut lines);
        assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [1, 2]);
        let mut apply = false;
        calls.lock().unwrap().clear();
        dispatch_should_apply_effect_tick_this_tick(&host, &mods, "effect", 0, 0, &mut apply);
        assert!(!apply);
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        let mut amount = 1.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        // Each mod sees the one before: (1 × 2 + 1) × 2 + 2, kept though mod 2 cancels.
        assert_eq!(amount, 8.0);
        assert!(canceled);
        let chained: Vec<Box<dyn Hooks>> = [1, 3]
            .map(|id| {
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .into();
        let mut amount = 1.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &chained,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        assert_eq!(amount, 9.0);
        assert!(!canceled);
        calls.lock().unwrap().clear();
        let mut claimed = false;
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(claimed);
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(calls.lock().unwrap().is_empty());
    }
}
