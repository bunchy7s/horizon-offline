//! Data packs in the game: functions (`/function`, `/return`, tick and load
//! tags, `/schedule`), predicates, `/reload` and `/datapack`.
use super::{
    Game,
    commands::{
        R, err,
        execute::{ExecState, Source},
    },
};
use horizon_client::{
    command::{Arg, Context},
    datapack::{self, DataPack, DataStore, Function},
    save,
    scoreboard::bracketed,
    text::{self, Text},
};
use horizon_world::nbt::Tag;
use std::collections::BTreeSet;

/// A `TimerQueue` event.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Scheduled {
    /// `ns:function`, or `#ns:tag` for a function tag.
    pub id: String,
    pub time: i64,
    pub seq: u64,
}
#[derive(Default)]
pub(super) struct FnState {
    pub store: DataStore,
    packs: Vec<DataPack>,
    /// Enabled pack ids, in load order (later overrides earlier).
    enabled: Vec<String>,
    loaded: bool,
    /// Run the `minecraft:load` tag on the next tick.
    load_pending: bool,
    pub schedule: Vec<Scheduled>,
    next_seq: u64,
    /// A dedicated server's `function-permission-level`.
    pub permission: Option<u8>,
}
/// What a function reported through `return`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    NoReturn,
    Value(i32),
    Fail,
}

/// The world's pack stack: the built-in pack (unless disabled) and the
/// packs of `<world>/datapacks`, with the enabled ids from `datapacks.json`.
pub(super) struct PackStack {
    pub builtin: Option<DataPack>,
    pub packs: Vec<DataPack>,
    pub enabled: Vec<String>,
}
pub(super) fn pack_stack(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    vanilla_disabled: bool,
) -> PackStack {
    let builtin = (!vanilla_disabled).then(|| DataPack::builtin(resources));
    let packs = datapack::discover(world_root);
    let saved: Option<serde_json::Value> = std::fs::read(world_root.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let list = |key: &str| -> Vec<String> {
        saved
            .as_ref()
            .and_then(|v| v[key].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let disabled: BTreeSet<String> = list("disabled").into_iter().collect();
    let mut enabled: Vec<String> = list("enabled")
        .into_iter()
        .filter(|id| packs.iter().any(|p| &p.id == id))
        .collect();
    for p in &packs {
        if !enabled.contains(&p.id) && !disabled.contains(&p.id) {
            enabled.push(p.id.clone());
        }
    }
    PackStack {
        builtin,
        packs,
        enabled,
    }
}

/// The world's enabled packs' content files (`recipe/`, `loot_table/`,
/// `advancement/`, `tags/…`) for the tables to merge over the resource
/// root's own. Empty when the world has no packs.
pub(super) fn content_overlay(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    vanilla_disabled: bool,
) -> Vec<(String, Vec<Vec<u8>>)> {
    let stack = pack_stack(world_root, resources, vanilla_disabled);
    let packs: Vec<&DataPack> = stack
        .packs
        .iter()
        .filter(|p| stack.enabled.contains(&p.id))
        .collect();
    if packs.is_empty() {
        return Vec::new();
    }
    DataStore::load(&packs).files.into_iter().collect()
}

impl FnState {
    /// The level function files compile at (`ServerFunctionLibrary`'s
    /// `functionCompilationPermissions`): `function-permission-level` on a
    /// dedicated server, game master (2) otherwise. Whoever runs a function,
    /// its commands need no more than this.
    fn command_level(&self) -> u8 {
        self.permission.unwrap_or(2)
    }
}

impl Game<'_> {
    // ---- loading -----------------------------------------------------------

    fn datapack_settings_path(&self) -> std::path::PathBuf {
        self.world_root.join("datapacks.json")
    }
    /// Discover and load the data packs of the world (once, or on `/reload`).
    pub(super) fn ensure_datapacks(&mut self) {
        if self.fns.loaded || self.world_root.as_os_str().is_empty() {
            return;
        }
        self.reload_datapacks(false);
    }
    pub(super) fn reload_datapacks(&mut self, run_load: bool) {
        let stack = pack_stack(
            &self.world_root,
            &self.options.resources,
            self.cmds.vanilla_disabled,
        );
        self.fns.packs = stack.packs;
        self.fns.enabled = stack.enabled;
        self.rebuild_store();
        self.fns.loaded = true;
        self.fns.load_pending = true;
        // An explicit reload (`/reload`, `/datapack`) reloads the content
        // tables with the functions, as `ReloadableServerResources` does;
        // the lazy first load ran after the world already built them.
        if run_load {
            self.reload_world_tables();
        }
        let _ = run_load;
        if self.fns.schedule.is_empty() {
            self.load_schedule();
        }
    }
    fn rebuild_store(&mut self) {
        // The built-in pack first (lowest priority), unless disabled.
        let stack = pack_stack(
            &self.world_root,
            &self.options.resources,
            self.cmds.vanilla_disabled,
        );
        let packs: Vec<&DataPack> = stack
            .builtin
            .iter()
            .chain(
                stack
                    .enabled
                    .iter()
                    .filter_map(|id| stack.packs.iter().find(|p| &p.id == id)),
            )
            .collect();
        let store = DataStore::load(&packs);
        for (id, e) in &store.errors {
            log::warn!("Horizon: failed to load function {id}: {e}");
        }
        self.fns.store = store;
        self.sync_function_registries();
    }
    fn save_datapack_settings(&self) {
        let disabled: Vec<&String> = self
            .fns
            .packs
            .iter()
            .map(|p| &p.id)
            .filter(|id| !self.fns.enabled.contains(id))
            .collect();
        let json = serde_json::json!({"enabled": self.fns.enabled, "disabled": disabled});
        let _ = std::fs::write(self.datapack_settings_path(), json.to_string());
    }
    pub(super) fn sync_function_registries(&mut self) {
        let reg = &mut self.ui.registries;
        reg.functions = self.fns.store.functions.keys().cloned().collect();
        reg.function_tags = self.fns.store.function_tags.keys().cloned().collect();
        reg.predicates = self.fns.store.predicates.keys().cloned().collect();
        reg.item_modifiers = self.fns.store.item_modifiers.keys().cloned().collect();
        reg.schedules = self.fns.schedule.iter().map(|s| s.id.clone()).collect();
        reg.datapacks_enabled = std::iter::once("vanilla".to_owned())
            .chain(self.fns.enabled.iter().cloned())
            .collect();
        reg.datapacks_disabled = self
            .fns
            .packs
            .iter()
            .map(|p| p.id.clone())
            .filter(|id| !self.fns.enabled.contains(id))
            .collect();
    }

    // ---- scheduled events ------------------------------------------------------

    fn schedule_path(&self) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/scheduled_events.dat")
    }
    fn load_schedule(&mut self) {
        let Some(root) = save::read_dat(&self.schedule_path()) else {
            return;
        };
        let Some(Tag::List(events)) = root.get("data").and_then(|d| d.get("events")) else {
            return;
        };
        for e in events {
            let (Some(Tag::Long(time)), Some(Tag::String(id)), Some(cb)) =
                (e.get("trigger_time"), e.get("id"), e.get("callback"))
            else {
                continue;
            };
            let _ = cb;
            self.fns.next_seq += 1;
            self.fns.schedule.push(Scheduled {
                id: id.to_string_lossy(),
                time: *time,
                seq: self.fns.next_seq,
            });
        }
    }
    pub(super) fn save_schedule(&self) {
        if !self.fns.loaded || self.remote.is_some() {
            return;
        }
        let mut events = self.fns.schedule.clone();
        events.sort_by_key(|e| (e.time, e.seq));
        let list = events
            .iter()
            .map(|e| {
                let (kind, name) = match e.id.strip_prefix('#') {
                    Some(tag) => ("minecraft:function_tag", tag),
                    None => ("minecraft:function", e.id.as_str()),
                };
                Tag::compound([
                    ("trigger_time", Tag::Long(e.time)),
                    ("id", Tag::String(e.id.as_str().into())),
                    (
                        "callback",
                        Tag::compound([
                            ("type", Tag::String(kind.into())),
                            ("id", Tag::String(name.into())),
                        ]),
                    ),
                ])
            })
            .collect();
        if events.is_empty() && !self.schedule_path().exists() {
            return;
        }
        let _ = save::write_dat(
            &self.schedule_path(),
            Tag::compound([
                ("data", Tag::compound([("events", Tag::List(list))])),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        );
    }

    // ---- ticking -----------------------------------------------------------------

    /// `ServerFunctionManager.tick` and the scheduled events.
    pub(super) fn tick_functions(&mut self) {
        self.ensure_datapacks();
        if self.remote.is_some() {
            return;
        }
        self.exec = ExecState::default();
        let source = self.server_source();
        if self.fns.load_pending {
            self.fns.load_pending = false;
            for f in self.fns.store.tag_functions("minecraft:load") {
                self.run_function_by_id(&f, None, source);
            }
        }
        let now = self.world.game_time;
        loop {
            let due = self
                .fns
                .schedule
                .iter()
                .enumerate()
                .filter(|(_, e)| e.time <= now)
                .min_by_key(|(_, e)| (e.time, e.seq))
                .map(|(i, _)| i);
            let Some(i) = due else { break };
            let event = self.fns.schedule.remove(i);
            match event.id.strip_prefix('#') {
                Some(tag) => {
                    for f in self.fns.store.tag_functions(tag) {
                        self.run_function_by_id(&f, None, source);
                    }
                }
                None => self.run_function_by_id(&event.id, None, source),
            }
        }
        for f in self.fns.store.tag_functions("minecraft:tick") {
            self.run_function_by_id(&f, None, source);
        }
    }
    fn run_function_by_id(&mut self, id: &str, args: Option<&Tag>, source: Source) {
        self.exec.commands = 0;
        if let Some(f) = self.fns.store.functions.get(id).cloned() {
            let _ = self.run_function(&f, args, source);
        }
    }

    // ---- running -----------------------------------------------------------------

    /// Run one function with `source`: its commands in order, until a `return`.
    pub(in crate::game) fn run_function(
        &mut self,
        f: &Function,
        args: Option<&Tag>,
        source: Source,
    ) -> R<Outcome> {
        let lines = f.instantiate(args).map_err(|(key, a)| {
            let args: Vec<&dyn std::fmt::Display> =
                a.iter().map(|s| s as &dyn std::fmt::Display).collect();
            err(
                "commands.function.instantiationFailure",
                &[&f.id, &horizon_client::text::Text::tr(&key, &args).plain()],
            )
        })?;
        if self.exec.function_depth >= 128 {
            return Ok(Outcome::NoReturn);
        }
        self.exec.function_depth += 1;
        let outer = self.exec.returned.take();
        let level = std::mem::replace(&mut self.ui.registries.permission, self.fns.command_level());
        let inner = Source {
            silent: true,
            store: None,
            ..source
        };
        for line in lines {
            let _ = self.execute_line(&line, inner);
            if self.exec.returned.is_some() {
                break;
            }
            if self.exec.commands > self.ui.rules.int("max_command_sequence_length").max(1) as u32 {
                break;
            }
        }
        self.ui.registries.permission = level;
        let returned = std::mem::replace(&mut self.exec.returned, outer);
        self.exec.function_depth -= 1;
        Ok(match returned {
            None => Outcome::NoReturn,
            Some(Some(v)) => Outcome::Value(v),
            Some(None) => Outcome::Fail,
        })
    }
    /// The functions a `FunctionArgument` names: a function or a tag.
    pub(in crate::game) fn functions_named(&mut self, name: &str) -> R<Vec<Function>> {
        self.ensure_datapacks();
        match name.strip_prefix('#') {
            // ServerFunctionLibrary.getTag: an unknown tag is empty.
            Some(tag) => {
                if !self.fns.store.function_tags.contains_key(tag) {
                    return Ok(Vec::new());
                }
                Ok(self
                    .fns
                    .store
                    .tag_functions(tag)
                    .iter()
                    .filter_map(|id| self.fns.store.functions.get(id).cloned())
                    .collect())
            }
            None => self
                .fns
                .store
                .functions
                .get(name)
                .cloned()
                .map(|f| vec![f])
                .ok_or_else(|| err("arguments.function.unknown", &[&name])),
        }
    }
    /// `if function`: what each function returns (None if it did not).
    pub(in crate::game) fn call_function_ref(
        &mut self,
        name: &str,
        args: Option<&Tag>,
    ) -> R<Vec<Option<i32>>> {
        let functions = self.functions_named(name)?;
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let mut out = Vec::new();
        for f in functions {
            out.push(match self.run_function(&f, args, source)? {
                Outcome::NoReturn => None,
                Outcome::Value(v) => Some(v),
                Outcome::Fail => Some(0),
            });
        }
        Ok(out)
    }

    // ---- commands ------------------------------------------------------------------

    pub(super) fn cmd_function(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(name)) = ctx.get("name").cloned() else {
            return Ok(0);
        };
        let functions = self.functions_named(&name)?;
        if functions.is_empty() {
            let id = name.trim_start_matches('#');
            return Err(err("commands.function.scheduled.no_functions", &[&id]));
        }
        let args = if ctx.has("with") {
            let r = self.data_ref_named(ctx, 0, "source")?;
            let data = self.data_of(&r)?;
            let tag = match ctx.get("path") {
                Some(Arg::Str(p)) => {
                    let path = horizon_client::nbt_path::NbtPath::parse(p)
                        .map_err(horizon_client::data_cmd::path_error)?;
                    horizon_client::data_cmd::single_tag(&path, &data)?
                }
                _ => data,
            };
            if !matches!(tag, Tag::Compound(_)) {
                return Err(err(
                    "commands.function.error.argument_not_compound",
                    &[&"tag"],
                ));
            }
            Some(tag)
        } else if let Some(Arg::Str(text)) = ctx.get("arguments") {
            Some(
                horizon_world::nbt::parse_snbt(text)
                    .map_err(|e| err("argument.nbt.invalid", &[&e]))?
                    .0,
            )
        } else {
            None
        };
        let message = if functions.len() == 1 {
            Text::translate(
                "commands.function.scheduled.single",
                &[Text::literal(functions[0].id.clone())],
            )
        } else {
            Text::translate(
                "commands.function.scheduled.multiple",
                &[horizon_client::scoreboard_cmd::format_list(
                    functions
                        .iter()
                        .map(|f| Text::literal(f.id.clone()))
                        .collect(),
                )],
            )
        };
        self.feedback(message);
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let mut sum: i32 = 0;
        let mut any = false;
        let mut failed = false;
        for f in functions {
            match self.run_function(&f, args.as_ref(), source)? {
                Outcome::NoReturn => {}
                Outcome::Value(v) => {
                    any = true;
                    sum = sum.wrapping_add(v);
                    let message = Text::translate(
                        "commands.function.result",
                        &[Text::literal(f.id.clone()), Text::literal(v.to_string())],
                    );
                    self.feedback(message);
                }
                Outcome::Fail => {
                    any = true;
                    failed = true;
                }
            }
        }
        self.exec.callback = Some(if failed && sum == 0 {
            Some((false, 0))
        } else if any {
            Some((true, sum))
        } else {
            None
        });
        Ok(sum)
    }
    /// `return <value>` and `return fail` (`return run` is a chain).
    pub(super) fn cmd_return(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "return.fail" {
            self.finish_return(None);
            self.exec.callback = Some(Some((false, 0)));
            return Ok(0);
        }
        let value = match ctx.get("value") {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        self.finish_return(Some(value));
        Ok(value)
    }
    pub(super) fn cmd_schedule(&mut self, id: &str, ctx: &Context) -> R<i32> {
        self.ensure_datapacks();
        if id == "schedule.clear" {
            let Some(Arg::Resource(name)) = ctx.get("function").cloned() else {
                return Ok(0);
            };
            let before = self.fns.schedule.len();
            self.fns.schedule.retain(|e| e.id != name);
            let count = before - self.fns.schedule.len();
            if count == 0 {
                return Err(err("commands.schedule.cleared.failure", &[&name]));
            }
            self.feedback(Text::translate(
                "commands.schedule.cleared.success",
                &[Text::literal(count.to_string()), Text::literal(name)],
            ));
            self.sync_function_registries();
            return Ok(count as i32);
        }
        let Some(Arg::Resource(name)) = ctx.get("function").cloned() else {
            return Ok(0);
        };
        let time = match ctx.get("time") {
            Some(Arg::Int(t)) => *t,
            _ => 0,
        };
        if time == 0 {
            return Err(err("commands.schedule.same_tick", &[]));
        }
        let replace = !ctx.has("append");
        let tick_time = self.world.game_time + i64::from(time);
        let schedule_id = match name.strip_prefix('#') {
            // An unknown tag is empty, and scheduling it is allowed.
            Some(_) => name.clone(),
            None => {
                let Some(f) = self.fns.store.functions.get(&name) else {
                    return Err(err("arguments.function.unknown", &[&name]));
                };
                if f.is_macro() {
                    return Err(err("commands.schedule.macro", &[]));
                }
                name.clone()
            }
        };
        if replace {
            self.fns.schedule.retain(|e| e.id != schedule_id);
        }
        if !self
            .fns
            .schedule
            .iter()
            .any(|e| e.id == schedule_id && e.time == tick_time)
        {
            self.fns.next_seq += 1;
            let seq = self.fns.next_seq;
            self.fns.schedule.push(Scheduled {
                id: schedule_id,
                time: tick_time,
                seq,
            });
        }
        let key = if name.starts_with('#') {
            "commands.schedule.created.tag"
        } else {
            "commands.schedule.created.function"
        };
        self.feedback(Text::translate(
            key,
            &[
                Text::literal(name.trim_start_matches('#').to_owned()),
                Text::literal(time.to_string()),
                Text::literal(tick_time.to_string()),
            ],
        ));
        self.sync_function_registries();
        Ok((tick_time.rem_euclid(i64::from(i32::MAX))) as i32)
    }
    /// ReloadCommand: the message, then the (asynchronous) reload; returns 0.
    pub(super) fn cmd_reload(&mut self) -> R<i32> {
        self.feedback(Text::translate("commands.reload.success", &[]));
        self.reload_datapacks(true);
        Ok(0)
    }
    /// `PackLocationInfo.createChatLink`: the id decorated with its source
    /// (`pack.nameAndSource`, grey) in brackets, green when enabled.
    fn pack_link(&self, id: &str, enabled: bool) -> Text {
        let hint = if enabled { "disable" } else { "enable" };
        let style = text::Style {
            color: Some(if enabled { text::GREEN } else { text::RED }),
            click: Some(text::ClickEvent::SuggestCommand(format!(
                "/datapack {hint} \"{id}\""
            ))),
            ..text::Style::default()
        };
        let description = self
            .fns
            .packs
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| "Default data pack".to_owned(), DataPack::description);
        let style = text::Style {
            hover: Some(Box::new(Text::literal(format!("{id}\n{description}")))),
            ..style
        };
        let source = if id == "vanilla" {
            "pack.source.builtin"
        } else if FEATURE_PACKS.iter().any(|(p, _)| *p == id) {
            "pack.source.feature"
        } else {
            "pack.source.world"
        };
        let name = Text::translate(
            "pack.nameAndSource",
            &[Text::literal(id), Text::translate(source, &[])],
        )
        .with_color(text::GRAY);
        bracketed(name).with_style(&style)
    }
    /// The selected packs, lowest priority first.
    fn selected_packs(&self) -> Vec<String> {
        let vanilla = (!self.cmds.vanilla_disabled).then(|| "vanilla".to_owned());
        vanilla
            .into_iter()
            .chain(self.fns.enabled.iter().cloned())
            .collect()
    }
    /// `FileUtil.isPathPartPortable`: no trailing dot and no Windows device
    /// name (CON, PRN, AUX, NUL, COM and LPT with a digit from 1 to 9).
    fn is_portable_name(name: &str) -> bool {
        let stem = name.split('.').next().unwrap_or_default();
        let device = match stem.len() {
            3 => matches!(stem, "com" | "con" | "prn" | "aux" | "nul"),
            4 => matches!(&stem[..3], "com" | "lpt") && matches!(stem.as_bytes()[3], b'1'..=b'9'),
            _ => false,
        };
        !device && !name.ends_with('.')
    }
    /// `DataPackCommand.createPack`: a pack folder with its `data` folder and a
    /// `pack.mcmeta` in the world's datapacks folder.
    fn create_pack(&mut self, ctx: &Context) -> R<i32> {
        let id = match ctx.get("id") {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let description = self.component_arg(ctx, "description")?;
        // FileUtil.isValidPathSegment: lowercase letters, digits, '-', '.' and '_'.
        let valid = !id.is_empty()
            && id != "."
            && id != ".."
            && id
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-' | '.' | '_'));
        if !valid {
            return Err(err("commands.datapack.create.invalid_name", &[&id]));
        }
        if !Self::is_portable_name(&id) {
            return Err(err("commands.datapack.create.invalid_full_name", &[&id]));
        }
        // Without a world there is no datapacks folder to write to.
        if self.world_root.as_os_str().is_empty() {
            return Err(err("commands.datapack.create.io_failure", &[&id]));
        }
        let datapacks = self.world_root.join("datapacks");
        let pack = datapacks.join(&id);
        if pack.exists() {
            return Err(err("commands.datapack.create.already_exists", &[&id]));
        }
        // The data pack version (SharedConstants), written as a bare major
        // number for both bounds, as vanilla does.
        let pack_format = horizon_core::version::DATA_PACK_FORMAT.0;
        let mcmeta = serde_json::json!({
            "pack": {
                "description": description.component(),
                "min_format": pack_format,
                "max_format": pack_format,
            }
        });
        let written = std::fs::create_dir_all(&datapacks)
            .and_then(|()| std::fs::create_dir(&pack))
            .and_then(|()| std::fs::create_dir(pack.join("data")))
            .and_then(|()| {
                std::fs::write(
                    pack.join("pack.mcmeta"),
                    serde_json::to_string_pretty(&mcmeta).unwrap_or_default(),
                )
            });
        if written.is_err() {
            return Err(err("commands.datapack.create.io_failure", &[&id]));
        }
        self.feedback(Text::translate(
            "commands.datapack.create.success",
            &[Text::literal(id)],
        ));
        Ok(1)
    }
    pub(super) fn cmd_datapack(&mut self, id: &str, ctx: &Context) -> R<i32> {
        self.ensure_datapacks();
        let name = match ctx.get("name") {
            Some(Arg::Str(n)) => Some(n.clone()),
            _ => None,
        };
        let selected = self.selected_packs();
        // Feature packs need features this world lacks: never available.
        let available: Vec<String> = std::iter::once("vanilla".to_owned())
            .chain(self.fns.packs.iter().map(|p| p.id.clone()))
            .filter(|p| !selected.contains(p))
            .collect();
        let known = |g: &Self, n: &str| {
            n == "vanilla"
                || FEATURE_PACKS.iter().any(|(p, _)| *p == n)
                || g.fns.packs.iter().any(|p| p.id == n)
        };
        match id {
            "datapack.create" => self.create_pack(ctx),
            "datapack.list" => {
                let want_enabled = ctx.has("enabled") || (!ctx.has("available"));
                let want_available = ctx.has("available") || (!ctx.has("enabled"));
                let mut total = 0;
                if want_enabled {
                    if selected.is_empty() {
                        self.feedback(Text::translate("commands.datapack.list.enabled.none", &[]));
                    } else {
                        let links: Vec<Text> =
                            selected.iter().map(|p| self.pack_link(p, true)).collect();
                        total += links.len();
                        self.feedback(Text::translate(
                            "commands.datapack.list.enabled.success",
                            &[
                                Text::literal(links.len().to_string()),
                                horizon_client::scoreboard_cmd::format_list(links),
                            ],
                        ));
                    }
                }
                if want_available {
                    if available.is_empty() {
                        self.feedback(Text::translate(
                            "commands.datapack.list.available.none",
                            &[],
                        ));
                    } else {
                        let links: Vec<Text> =
                            available.iter().map(|p| self.pack_link(p, false)).collect();
                        total += links.len();
                        self.feedback(Text::translate(
                            "commands.datapack.list.available.success",
                            &[
                                Text::literal(links.len().to_string()),
                                horizon_client::scoreboard_cmd::format_list(links),
                            ],
                        ));
                    }
                }
                Ok(total as i32)
            }
            "datapack.enable" => {
                let name = name.unwrap_or_default();
                if !known(self, &name) {
                    return Err(err("commands.datapack.unknown", &[&name]));
                }
                if selected.contains(&name) {
                    return Err(err("commands.datapack.enable.failed", &[&name]));
                }
                if let Some((_, flag)) = FEATURE_PACKS.iter().find(|(p, _)| *p == name) {
                    return Err(err(
                        "commands.datapack.enable.failed.no_flags",
                        &[&name, flag],
                    ));
                }
                if name == "vanilla" {
                    self.cmds.vanilla_disabled = false;
                } else {
                    let existing = match ctx.get("existing") {
                        Some(Arg::Str(e)) => Some(e.clone()),
                        _ => None,
                    };
                    let position = if ctx.has("first") {
                        0
                    } else if let Some(e) = &existing {
                        let Some(i) = self.fns.enabled.iter().position(|p| p == e) else {
                            return Err(err("commands.datapack.unknown", &[e]));
                        };
                        if ctx.has("before") { i } else { i + 1 }
                    } else {
                        self.fns.enabled.len()
                    };
                    self.fns.enabled.insert(position, name.clone());
                }
                self.feedback(Text::translate(
                    "commands.datapack.modify.enable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change();
                Ok(self.selected_packs().len() as i32)
            }
            _ => {
                let name = name.unwrap_or_default();
                if !known(self, &name) {
                    return Err(err("commands.datapack.unknown", &[&name]));
                }
                if !selected.contains(&name) {
                    return Err(err("commands.datapack.disable.failed", &[&name]));
                }
                if name == "vanilla" {
                    self.cmds.vanilla_disabled = true;
                } else {
                    self.fns.enabled.retain(|p| p != &name);
                }
                self.feedback(Text::translate(
                    "commands.datapack.modify.disable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change();
                Ok(self.selected_packs().len() as i32)
            }
        }
    }
    /// `ReloadCommand.reloadPacks`: no message of its own.
    fn apply_pack_change(&mut self) {
        self.save_datapack_settings();
        self.rebuild_store();
        self.fns.load_pending = true;
    }

    // ---- predicates -------------------------------------------------------------------

    /// `if predicate`: a datapack predicate tested for the source entity.
    pub(in crate::game) fn check_predicate(&mut self, id: &str) -> R<bool> {
        self.ensure_datapacks();
        let Some(json) = self.fns.store.predicates.get(id).cloned() else {
            return Err(err("argument.predicate.unknown", &[&id]));
        };
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let view = source.entity.map(|t| self.entity_view_of(t));
        Ok(self.conditions_hold(&json, view.as_ref(), source.pos))
    }
}

/// The built-in feature packs and the feature flag each needs (none of them
/// enabled in a world created without experiments).
const FEATURE_PACKS: &[(&str, &str)] = &[
    ("minecart_improvements", "minecraft:minecart_improvements"),
    ("redstone_experiments", "minecraft:redstone_experiments"),
    ("trade_rebalance", "minecraft:trade_rebalance"),
];
