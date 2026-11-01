//! `/execute` (ExecuteCommand): walk the parsed redirect chain, forking and
//! filtering command sources, then run the final command once per source,
//! reporting each result to the `store` callbacks of its source.
use super::{Arg, Context, Dim, Game, R, Target, data::DataRef, err};
use horizon_client::{
    command::{CommandError, vanilla},
    nbt_path::NbtPath,
    text::Text,
};
use horizon_world::nbt::Tag;

/// `CommandSourceStack` fields Horizon models: the executing entity (None for
/// the server), position, rotation, anchor, whether feedback is suppressed
/// and the chain of `store` callbacks.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct Source {
    pub entity: Option<Target>,
    pub pos: [f64; 3],
    pub rot: [f32; 2],
    /// EntityAnchorArgument.Anchor: eyes (true) or feet.
    pub eyes: bool,
    /// `withSuppressedOutput`: functions do not print feedback.
    pub silent: bool,
    /// Index of the newest `store` in `ExecState::stores`.
    pub store: Option<u32>,
    /// The level this command source uses. Horizon has only this dimension's
    /// chunks loaded in the single-player client.
    pub dimension: Dim,
    /// The dimension `execute in` named, if any: a server teleports its
    /// players there (`TeleportCommand` into the source's level).
    pub level: Option<super::super::Dim>,
    /// The command block whose output this source is
    /// (`BaseCommandBlock.CloseableCommandBlockSource`).
    pub block: Option<(i32, i32, i32)>,
    /// The command block minecart whose output this source is.
    pub cart: Option<u32>,
}

/// Where `execute store` writes.
#[derive(Clone, Debug)]
pub(in crate::game) enum StoreTarget {
    Score(Vec<String>, String),
    Bossbar(String, bool),
    Data(DataRef, NbtPath, String, f64),
}
#[derive(Clone, Debug)]
pub(in crate::game) struct StoreEntry {
    /// `store result` (true) or `store success`.
    result: bool,
    target: StoreTarget,
    parent: Option<u32>,
}
/// Command execution state shared across nested commands and functions.
#[derive(Default)]
pub(in crate::game) struct ExecState {
    stores: Vec<StoreEntry>,
    /// Commands run since the outermost one began.
    pub commands: u32,
    /// What the last leaf command reports to its `store` callbacks: None for
    /// the default, `Some(None)` for nothing (a function that did not
    /// `return`), `Some(Some((success, result)))` for an explicit value.
    pub callback: Option<Option<(bool, i32)>>,
    /// The innermost function frame's `return`.
    pub returned: Option<Option<i32>>,
    /// Depth of function calls.
    pub function_depth: u32,
}

/// A modifier that brings the sources to `max_command_forks` ends the chain
/// (`BuildContexts.ERROR_FORK_LIMIT_REACHED`); the error is only shown when
/// no fork came before it (`CommandSourceStack.handleError`).
fn fork_limit_reached(forked: bool, limit: usize) -> R<i32> {
    if forked {
        Ok(0)
    } else {
        Err(err("command.forkLimit", &[&limit]))
    }
}

/// Java's `(int)`/`(short)`/`(byte)` narrowing of a double.
fn java_int(v: f64) -> i32 {
    v as i32
}

impl Game<'_> {
    /// The source a command typed by the player starts from.
    pub(in crate::game) fn player_source(&self) -> Source {
        Source {
            // The server console has no entity (CommandSourceStack of the server).
            entity: self.player.is_some().then_some(Target::Player),
            pos: self.source_position(),
            rot: self.source_rotation(),
            eyes: false,
            silent: false,
            store: None,
            dimension: self.dim,
            level: None,
            block: None,
            cart: None,
        }
    }
    /// The server as a source (function ticks and schedules): the world spawn.
    pub(in crate::game) fn server_source(&self) -> Source {
        Source {
            entity: None,
            pos: self.server_position(),
            rot: [0.0, 0.0],
            eyes: false,
            silent: true,
            store: None,
            dimension: self.dim,
            level: None,
            block: None,
            cart: None,
        }
    }
    pub(in crate::game) fn target_rotation(&self, t: Target) -> [f32; 2] {
        match t {
            Target::Player => [self.yaw, self.pitch],
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or([0.0; 2], |g| [g.yaw, g.pitch]),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or([0.0; 2], |m| [m.yaw, m.head_pitch]),
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, 0.0]),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or([0.0; 2], |b| [b.yaw, 0.0]),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or([0.0; 2], |c| [c.yaw, 0.0]),
            Target::Other(id) => self
                .others
                .iter()
                .find(|o| o.id == id)
                .map_or([0.0; 2], |o| [o.yaw, o.pitch]),
            _ => [0.0; 2],
        }
    }
    pub(super) fn target_eye_height(&self, t: Target) -> f64 {
        match t {
            Target::Player => self
                .player
                .as_ref()
                .map_or(1.62, |p| f64::from(p.dimensions.eye_height)),
            Target::Guest(conn) => self.command_guest(conn).map_or(1.62, |g| g.eye_height()),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or(0.0, |m| m.eye_y() - m.position.y),
            // ArmorStand.getDefaultDimensions: a marker's are empty, a small
            // stand's are BABY_DIMENSIONS, whose eye height is 0.9875F.
            Target::Stand(id) => match self.armor_stands.iter().find(|a| a.id == id) {
                Some(a) if a.marker => 0.0,
                Some(a) if a.small => f64::from(0.9875f32),
                _ => self.type_eye_height(&self.target_type(t)),
            },
            // Entity.getEyeHeight: the type's dimensions.
            _ => self.type_eye_height(&self.target_type(t)),
        }
    }
    /// `EntityType.getDimensions().eyeHeight()` from `entity-types.json`,
    /// read once: a float, widened as `Entity.getEyeY` does.
    fn type_eye_height(&self, ty: &str) -> f64 {
        static EYE_HEIGHTS: std::sync::OnceLock<std::collections::HashMap<String, f64>> =
            std::sync::OnceLock::new();
        EYE_HEIGHTS
            .get_or_init(|| {
                std::fs::read(self.options.resources.join("entity-types.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| {
                        v.as_object().map(|o| {
                            o.iter()
                                .filter_map(|(k, t)| {
                                    Some((k.clone(), f64::from(t["eye_height"].as_f64()? as f32)))
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            })
            .get(ty)
            .copied()
            .unwrap_or(0.0)
    }
    /// EntityAnchorArgument.Anchor.apply.
    pub(super) fn anchor_position(&self, s: &Source) -> [f64; 3] {
        match s.entity {
            Some(t) if s.eyes => [s.pos[0], s.pos[1] + self.target_eye_height(t), s.pos[2]],
            _ => s.pos,
        }
    }
    /// Evaluate `f` with `source` as the command source, in the source's
    /// level (`CommandSourceStack.getLevel`): a server puts that level in
    /// place for it.
    pub(in crate::game) fn with_source<T>(
        &mut self,
        source: Source,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.command_source.replace(source);
        let back = self.enter_command_level(source.dimension);
        let out = f(self);
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        self.command_source = previous;
        out
    }

    // ---- running commands -----------------------------------------------

    /// Run one command line with `source` (no error reporting): the entry
    /// for functions and `execute run`.
    pub(in crate::game) fn execute_line(&mut self, line: &str, source: Source) -> R<i32> {
        let d = vanilla::game_dispatcher();
        let parse = d.parse(line, &self.ui.registries);
        let (ctx, _) = parse.executable()?;
        let ctx = ctx.clone();
        let root = parse.context.clone();
        self.with_source(source, |g| g.run_chain(&root, &ctx))
    }
    /// Run a parsed command (`root` is the first context; `ctx` the one with
    /// the executable node): `execute` and `return` walk their own chains.
    pub(in crate::game) fn run_chain(&mut self, root: &Context, ctx: &Context) -> R<i32> {
        self.exec.commands += 1;
        let limit = self.ui.rules.int("max_command_sequence_length").max(1) as u32;
        if self.exec.commands > limit {
            return Err(err("command.failed", &[]));
        }
        match root.literals.first().map(String::as_str) {
            Some("execute") => self.run_execute(root),
            Some("return") if root.child.is_some() => self.run_return_run(root),
            _ => {
                let id = ctx.command.unwrap_or("");
                self.run_leaf(id, ctx)
            }
        }
    }
    /// Run a leaf command and report its outcome to the `store` callbacks.
    pub(in crate::game) fn run_leaf(&mut self, id: &str, ctx: &Context) -> R<i32> {
        self.exec.callback = None;
        let store = self.command_source.and_then(|s| s.store);
        let result = self.execute_command(id, ctx);
        let custom = self.exec.callback.take();
        let outcome = match (&result, custom) {
            (_, Some(c)) => c,
            (Ok(v), None) => Some((true, *v)),
            (Err(_), None) => Some((false, 0)),
        };
        if let Some((success, value)) = outcome {
            self.deliver(store, success, value);
        }
        result
    }
    /// `CommandResultCallback`s of a source's `store` chain, oldest first.
    fn deliver(&mut self, store: Option<u32>, success: bool, value: i32) {
        // The outermost source's own CommandResultCallback (harness record).
        if self.exec.function_depth == 0
            && let Some(results) = self.cmds.results.as_mut()
        {
            results.push((success, value));
        }
        let mut chain = Vec::new();
        let mut at = store;
        while let Some(i) = at {
            let Some(e) = self.exec.stores.get(i as usize) else {
                break;
            };
            chain.push(e.clone());
            at = e.parent;
        }
        for e in chain.into_iter().rev() {
            let v = if e.result { value } else { i32::from(success) };
            self.store_value(&e.target, v);
        }
    }
    fn store_value(&mut self, target: &StoreTarget, value: i32) {
        match target {
            StoreTarget::Score(holders, objective) => {
                for h in holders {
                    self.score.board.set(h, objective, value);
                }
            }
            StoreTarget::Bossbar(id, is_value) => self.set_bossbar_value(id, *is_value, value),
            StoreTarget::Data(r, path, kind, scale) => {
                let Ok(mut data) = self.data_of(r) else {
                    return;
                };
                let v = f64::from(value) * scale;
                let tag = match kind.as_str() {
                    "int" => Tag::Int(java_int(v)),
                    "float" => Tag::Float((v as f32).to_bits()),
                    "short" => Tag::Short(java_int(v) as i16),
                    "long" => Tag::Long(v as i64),
                    "double" => Tag::Double(v.to_bits()),
                    _ => Tag::Byte(java_int(v) as i8),
                };
                if path.set(&mut data, &tag).is_ok() {
                    let _ = self.set_data_of(r, data);
                }
            }
        }
    }

    /// ExecuteCommand: `ctx` is the context holding the `execute` literal.
    pub(in crate::game) fn run_execute(&mut self, ctx: &Context) -> R<i32> {
        // BuildContexts: a modifier may not bring the sources to this many.
        let fork_limit = self.ui.rules.int("max_command_forks") as usize;
        let mut sources = vec![self.command_source.unwrap_or_else(|| self.player_source())];
        let mut c = ctx;
        // A fork modifier (as, at, positioned as, rotated as, facing entity,
        // on, a non-terminal if/unless) makes later errors silent
        // (CommandSourceStack.handleError with `forked`).
        let mut forked = false;
        loop {
            let word = c
                .literals
                .iter()
                .map(String::as_str)
                .find(|w| *w != "execute")
                .unwrap_or("");
            match word {
                "run" => {
                    let Some(command) = c.child.as_deref() else {
                        return Err(err("command.unknown.command", &[]));
                    };
                    let last = command.last_child();
                    if last.command.is_none() {
                        return Err(err("command.unknown.command", &[]));
                    }
                    let mut total = 0;
                    let many = sources.len() > 1;
                    let mut failure = None;
                    for s in sources {
                        match self.with_source(s, |g| g.run_chain(command, last)) {
                            Ok(n) => total += n,
                            Err(_) if forked => {}
                            // Several sources: report each failure and carry on.
                            Err(e) if many => self.command_failure(e.message),
                            Err(e) => failure = Some(e),
                        }
                        if self.exec.returned.is_some() {
                            break;
                        }
                    }
                    return match failure {
                        Some(e) => Err(e),
                        None => Ok(total),
                    };
                }
                "if" | "unless" => {
                    let want = word == "if";
                    if c.child.is_none() {
                        return self.run_conditional_terminal(c, sources, want);
                    }
                    forked = true;
                    let mut kept = Vec::new();
                    for s in sources {
                        let (keep, _) = self.with_source(s, |g| g.condition(c, want))?;
                        if kept.len() + usize::from(keep) >= fork_limit {
                            return fork_limit_reached(forked, fork_limit);
                        }
                        if keep {
                            kept.push(s);
                        }
                    }
                    sources = kept;
                }
                _ => {
                    forked |= match word {
                        "as" | "at" | "on" => true,
                        "positioned" | "rotated" => c.has("as"),
                        "facing" => c.has("entity"),
                        _ => false,
                    };
                    let mut next = Vec::new();
                    for s in sources {
                        let new = self.with_source(s, |g| g.modify(word, c, s))?;
                        if next.len() + new.len() >= fork_limit {
                            return fork_limit_reached(forked, fork_limit);
                        }
                        next.extend(new);
                    }
                    sources = next;
                }
            }
            match c.child.as_deref() {
                Some(child) => c = child,
                None => return Err(err("command.unknown.command", &[])),
            }
        }
    }
    /// A conditional that ends the chain runs as a command: a test per source.
    fn run_conditional_terminal(
        &mut self,
        c: &Context,
        sources: Vec<Source>,
        want: bool,
    ) -> R<i32> {
        let numeric = ["entity", "data", "blocks", "items", "slots"]
            .iter()
            .any(|k| c.has(k));
        let many = sources.len() > 1;
        let mut total = 0;
        let mut failure: Option<CommandError> = None;
        for s in sources {
            self.exec.callback = None;
            let (keep, count) = match self.with_source(s, |g| g.condition(c, want)) {
                Ok(v) => v,
                Err(e) => {
                    self.deliver(s.store, false, 0);
                    if many {
                        self.command_failure(e.message);
                    } else {
                        failure = Some(e);
                    }
                    continue;
                }
            };
            if keep {
                let n = if want && numeric { count } else { 1 };
                let message = if want && numeric {
                    Text::translate(
                        "commands.execute.conditional.pass_count",
                        &[Text::literal(count.to_string())],
                    )
                } else {
                    Text::translate("commands.execute.conditional.pass", &[])
                };
                self.with_source(s, |g| g.feedback(message));
                self.deliver(s.store, true, n);
                total += n;
            } else {
                self.deliver(s.store, false, 0);
                let e = if !want && numeric {
                    err("commands.execute.conditional.fail_count", &[&count])
                } else {
                    err("commands.execute.conditional.fail", &[])
                };
                if many {
                    self.command_failure(e.message);
                } else {
                    failure = Some(e);
                }
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(total),
        }
    }
    /// `return run <command>`: run it and return its result from the frame.
    fn run_return_run(&mut self, root: &Context) -> R<i32> {
        let Some(command) = root.child.as_deref() else {
            return Err(err("command.unknown.command", &[]));
        };
        let last = command.last_child();
        let result = self.run_chain(command, last);
        // The command's own callbacks do not fire; the function frame gets
        // its result instead.
        match result {
            Ok(v) => self.finish_return(Some(v)),
            Err(e) => {
                self.finish_return(None);
                self.command_failure(e.message);
            }
        }
        self.exec.callback = Some(None);
        Ok(0)
    }
    /// `Frame.returnSuccess`/`returnFailure` then `discard`.
    pub(in crate::game) fn finish_return(&mut self, value: Option<i32>) {
        self.exec.returned = Some(value);
    }

    /// The sources one modifier produces from `s`.
    fn modify(&mut self, word: &str, c: &Context, s: Source) -> R<Vec<Source>> {
        let targets = |g: &mut Self| -> R<Vec<Target>> {
            // An empty selection forks to nothing rather than failing.
            match c.get("targets") {
                Some(Arg::Entity(sel)) => {
                    let sel = sel.clone();
                    Ok(g.select(&sel))
                }
                _ => Ok(Vec::new()),
            }
        };
        Ok(match word {
            "as" => targets(self)?
                .into_iter()
                .map(|t| Source {
                    entity: Some(t),
                    ..s
                })
                .collect(),
            "at" => targets(self)?
                .into_iter()
                .filter_map(|t| {
                    // CommandSourceStack.withLevel(entity.level()).
                    let level = self.target_level(t);
                    Some(Source {
                        pos: self.target_position(t)?,
                        rot: self.target_rotation(t),
                        dimension: level,
                        level: (level != self.dim).then_some(level),
                        ..s
                    })
                })
                .collect(),
            "positioned" if c.has("as") => targets(self)?
                .into_iter()
                .filter_map(|t| {
                    // withPosition keeps the anchor (only `positioned <pos>`
                    // resets it to the feet).
                    Some(Source {
                        pos: self.target_position(t)?,
                        ..s
                    })
                })
                .collect(),
            "positioned" if c.has("over") => {
                let Some(Arg::Str(kind)) = c.get("heightmap") else {
                    return Ok(vec![s]);
                };
                let (x, z) = (s.pos[0].floor() as i32, s.pos[2].floor() as i32);
                if !self.world.chunks.contains_key(&(x >> 4, z >> 4)) {
                    return Err(err("argument.pos.unloaded", &[]));
                }
                let y = self.heightmap_at(kind, x, z);
                vec![Source {
                    pos: [s.pos[0], f64::from(y), s.pos[2]],
                    ..s
                }]
            }
            "positioned" => {
                let pos = self
                    .position_arg(c, "pos")
                    .ok_or_else(|| err("argument.pos.missing.double", &[]))?;
                vec![Source {
                    pos,
                    eyes: false,
                    ..s
                }]
            }
            "rotated" if c.has("as") => targets(self)?
                .into_iter()
                .map(|t| Source {
                    rot: self.target_rotation(t),
                    ..s
                })
                .collect(),
            "rotated" => {
                let rot = match c.get("rot") {
                    Some(Arg::Coords(r)) => r.rotation(s.rot),
                    _ => s.rot,
                };
                vec![Source { rot, ..s }]
            }
            "facing" if c.has("entity") => {
                let eyes = matches!(c.get("anchor"), Some(Arg::Anchor(true)));
                targets(self)?
                    .into_iter()
                    .filter_map(|t| {
                        let p = self.target_position(t)?;
                        let y = if eyes {
                            p[1] + self.target_eye_height(t)
                        } else {
                            p[1]
                        };
                        Some(self.facing(s, [p[0], y, p[2]]))
                    })
                    .collect()
            }
            "facing" => {
                let to = self
                    .position_arg(c, "pos")
                    .ok_or_else(|| err("argument.pos.missing.double", &[]))?;
                vec![self.facing(s, to)]
            }
            "align" => {
                let Some(Arg::Str(axes)) = c.get("axes") else {
                    return Ok(vec![s]);
                };
                let mut pos = s.pos;
                for (i, axis) in ['x', 'y', 'z'].into_iter().enumerate() {
                    if axes.contains(axis) {
                        pos[i] = pos[i].floor();
                    }
                }
                vec![Source { pos, ..s }]
            }
            "anchored" => vec![Source {
                eyes: matches!(c.get("anchor"), Some(Arg::Anchor(true))),
                ..s
            }],
            "in" => {
                // CommandSourceStack.withLevel scales the source position;
                // absolute coordinates passed to /tp are unaffected.
                let Some(Arg::Resource(dim)) = c.get("dimension") else {
                    return Ok(vec![s]);
                };
                if !matches!(
                    dim.as_str(),
                    "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
                ) {
                    return Err(err("argument.dimension.invalid", &[dim]));
                }
                let dimension = Dim::from_id(dim);
                let coordinate_scale = |dim| if dim == Dim::Nether { 8.0 } else { 1.0 };
                let scale = coordinate_scale(s.dimension) / coordinate_scale(dimension);
                vec![Source {
                    pos: [s.pos[0] * scale, s.pos[1], s.pos[2] * scale],
                    dimension,
                    level: Some(super::super::Dim::from_id(dim)),
                    ..s
                }]
            }
            "summon" => {
                let Some(Arg::Resource(kind)) = c.get("entity").cloned() else {
                    return Ok(vec![s]);
                };
                let target = self.summon_with_nbt(&kind, s.pos, "{}")?;
                let Some(t) = target else {
                    return Err(err("commands.summon.failed", &[]));
                };
                vec![Source {
                    entity: Some(t),
                    ..s
                }]
            }
            "on" => self.relation(c, s),
            "store" => vec![self.store_source(c, s)?],
            _ => return Err(err("command.unknown.command", &[])),
        })
    }
    /// The entities `on <relation>` reaches from the source entity.
    fn relation(&mut self, c: &Context, s: Source) -> Vec<Source> {
        let Some(entity) = s.entity else {
            return Vec::new();
        };
        let found: Vec<Target> = match c.literals.last().map(String::as_str) {
            Some("owner") => match entity {
                // OwnableEntity.getOwner → EntityReference.getLivingEntity:
                // a marker or a display with that UUID is not an owner.
                Target::Mob(id) => self
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.extra.owner)
                    .and_then(|u| self.target_with_uuid(u))
                    .filter(|owner| {
                        matches!(
                            owner,
                            Target::Player
                                | Target::Guest(_)
                                | Target::Mob(_)
                                | Target::Stand(_)
                                | Target::Dragon(_)
                        )
                    })
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            // Leashable.getLeashHolder: a fence knot is not an entity here.
            Some("leasher") => match entity {
                Target::Mob(id) => self
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.extra.leash.map(|holder| (holder, m.extra.leash_player)))
                    .and_then(|(holder, player)| match holder {
                        // The player who tied it (the host's when unrecorded).
                        horizon_client::mobs::leash::Holder::Player => {
                            let uuid = player.or(self.level.player_uuid)?;
                            self.target_by_uuid(horizon_client::lan::playerdata::ints_uuid(uuid))
                        }
                        horizon_client::mobs::leash::Holder::Mob(i) => Some(Target::Mob(i)),
                        horizon_client::mobs::leash::Holder::Boat(_)
                        | horizon_client::mobs::leash::Holder::Knot(_) => None,
                    })
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            // TraceableEntity.getOwner: who shot a projectile, who threw an
            // item, who summoned a vex.
            Some("origin") => match entity {
                Target::Projectile(id) => self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| match (p.owner?, p.guest) {
                        // A guest's shot: `Who::Player` is them.
                        (horizon_client::mobs::Who::Player, Some(conn))
                            if self.lan_acting == Some(conn) =>
                        {
                            Some(Target::Player)
                        }
                        (horizon_client::mobs::Who::Player, Some(conn)) => {
                            Some(Target::Guest(conn))
                        }
                        (horizon_client::mobs::Who::Player, None) => Some(Target::Player),
                        (horizon_client::mobs::Who::Mob(id), _) => Some(Target::Mob(id)),
                    })
                    .into_iter()
                    .collect(),
                Target::Item(id) => self
                    .dropped
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.thrower)
                    .and_then(|words| {
                        let uuid = words
                            .iter()
                            .fold(0u128, |all, w| all << 32 | u128::from(*w as u32));
                        self.target_by_uuid(uuid)
                    })
                    .into_iter()
                    .collect(),
                Target::Mob(id) => self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && m.kind.id == "minecraft:vex")
                    .map(|m| m.extra.counter2 as u32)
                    .filter(|owner| *owner != 0)
                    .map(Target::Mob)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            Some("vehicle") => self.ride_vehicle(entity).into_iter().collect(),
            Some("controller") | Some("passengers") => match entity {
                Target::Mob(id) if self.mobs.iter().any(|m| m.id == id && m.rider.is_some()) => {
                    vec![Target::Player]
                }
                _ => Vec::new(),
            },
            Some("target") | Some("attacker") => {
                let attacker = c.literals.last().is_some_and(|l| l == "attacker");
                match entity {
                    Target::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .and_then(|m| {
                            if attacker {
                                m.ai.last_hurt_by
                            } else {
                                m.ai.target
                            }
                        })
                        .map(|w| match w {
                            horizon_client::mobs::Who::Player => Target::Player,
                            horizon_client::mobs::Who::Mob(i) => Target::Mob(i),
                        })
                        .into_iter()
                        .collect(),
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        found
            .into_iter()
            .filter(|t| self.target_position(*t).is_some())
            .map(|t| Source {
                entity: Some(t),
                ..s
            })
            .collect()
    }
    /// `ServerLevel.getEntity(UUID)`: any entity of the level in place, not
    /// only mobs (displays, armor stands, items...). The map is keyed by id,
    /// so a killed entity and a later summon of the same UUID can both be
    /// in it; the one the level still holds is the entity.
    pub(in crate::game) fn target_with_uuid(&self, uuid: [i32; 4]) -> Option<Target> {
        if Some(uuid) == self.level.player_uuid {
            return Some(Target::Player);
        }
        self.persist.uuids.iter().find_map(|(&id, found)| {
            if *found != uuid {
                return None;
            }
            self.level_entity(i32::try_from(id).ok()?)
        })
    }
    /// The highest block of a heightmap type at a column (`Level.getHeight`).
    fn heightmap_at(&self, kind: &str, x: i32, z: i32) -> i32 {
        let top = self.world.top_y();
        for y in (self.world.bottom_y()..top).rev() {
            let state = self.world.block(x, y, z);
            if state == self.world.air {
                continue;
            }
            let facts = self.res.facts().get(state);
            let blocks = facts.legacy_solid || facts.fluid != horizon_client::blocks::Fluid::None;
            let leaves = self
                .pack
                .assets
                .registry
                .state(state)
                .is_some_and(|s| s.name.ends_with("_leaves"));
            let found = match kind {
                "ocean_floor" => facts.legacy_solid,
                "motion_blocking" => blocks,
                "motion_blocking_no_leaves" => blocks && !leaves,
                _ => true,
            };
            if found {
                return y + 1;
            }
        }
        self.world.bottom_y()
    }
    /// A `store` modifier: a new callback in the source's chain.
    fn store_source(&mut self, c: &Context, s: Source) -> R<Source> {
        let result = c.has("result");
        let target = if c.has("score") {
            let holders = self.score_holder_names(c, "targets", true)?;
            let Some(Arg::Str(objective)) = c.get("objective") else {
                return Err(err("command.failed", &[]));
            };
            if self.score.board.objective(objective).is_none() {
                return Err(err("arguments.objective.notFound", &[objective]));
            }
            StoreTarget::Score(holders, objective.clone())
        } else if c.has("bossbar") {
            let Some(Arg::Resource(id)) = c.get("id") else {
                return Err(err("command.failed", &[]));
            };
            if !self.bossbar_exists(id) {
                return Err(err("commands.bossbar.unknown", &[id]));
            }
            StoreTarget::Bossbar(id.clone(), c.has("value"))
        } else {
            let r = self.data_ref(c, 0)?;
            let path = match c.get("path") {
                Some(Arg::Str(p)) => {
                    NbtPath::parse(p).map_err(horizon_client::data_cmd::path_error)?
                }
                _ => return Err(err("command.failed", &[])),
            };
            let kind = c
                .literals
                .iter()
                .find(|l| {
                    matches!(
                        l.as_str(),
                        "int" | "float" | "short" | "long" | "double" | "byte"
                    )
                })
                .cloned()
                .unwrap_or_else(|| "int".into());
            let scale = match c.get("scale") {
                Some(Arg::Double(v)) => *v,
                _ => 1.0,
            };
            StoreTarget::Data(r, path, kind, scale)
        };
        self.exec.stores.push(StoreEntry {
            result,
            target,
            parent: s.store,
        });
        Ok(Source {
            store: Some(self.exec.stores.len() as u32 - 1),
            ..s
        })
    }
    /// CommandSourceStack.facing: rotation from the anchor point to `to`.
    fn facing(&self, s: Source, to: [f64; 3]) -> Source {
        let from = self.anchor_position(&s);
        let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let pitch = (-(dy.atan2(horizontal).to_degrees())) as f32;
        let yaw = (dz.atan2(dx).to_degrees() - 90.0) as f32;
        Source {
            rot: [yaw.rem_euclid(360.0), pitch],
            ..s
        }
    }

    // ---- conditions ----------------------------------------------------------

    /// Whether the source stays for `if`/`unless` (`want`), and the count the
    /// numeric conditions report.
    fn condition(&mut self, c: &Context, want: bool) -> R<(bool, i32)> {
        if c.has("function") {
            return self.condition_function(c, want);
        }
        let (matched, count) = self.condition_matches(c)?;
        Ok((matched == want, count))
    }
    /// `if function`/`unless function`: the source stays when a function
    /// returns a non-zero (`if`) or zero (`unless`) value; one that does not
    /// return leaves it out either way.
    fn condition_function(&mut self, c: &Context, want: bool) -> R<(bool, i32)> {
        let Some(Arg::Resource(name)) = c.get("name").cloned() else {
            return Ok((false, 0));
        };
        let results = self.call_function_ref(&name, None)?;
        let keep = results.iter().any(|r| r.is_some_and(|v| (v != 0) == want));
        Ok((keep, 0))
    }
    fn condition_matches(&mut self, c: &Context) -> R<(bool, i32)> {
        let flag = |b: bool| (b, i32::from(b));
        if c.has("data") {
            let r = self.data_ref_named(c, 0, "source")?;
            let data = self.data_of(&r)?;
            let Some(Arg::Str(p)) = c.get("path") else {
                return Ok((false, 0));
            };
            let path = NbtPath::parse(p).map_err(horizon_client::data_cmd::path_error)?;
            let n = path.count(&data) as i32;
            return Ok((n > 0, n));
        }
        if c.has("items") || c.has("slots") {
            return self.condition_items(c);
        }
        if c.has("entity") {
            let Some(Arg::Entity(sel)) = c.get("entities").cloned() else {
                return Ok((false, 0));
            };
            let n = self.select(&sel).len() as i32;
            return Ok((n > 0, n));
        }
        if c.has("loaded") {
            let p = self
                .coords(c, "pos")
                .map(|co| co.block(self.source_position(), self.source_rotation()));
            let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
            // ExecuteCommand.isChunkLoaded: ENTITY_TICKING, which is past lit.
            let loaded = |p: [i32; 3]| {
                let key = (p[0] >> 4, p[2] >> 4);
                self.world.chunks.contains_key(&key) && self.streaming.is_lit(key.0, key.1)
            };
            return Ok(flag(dimension == self.dim && p.is_some_and(loaded)));
        }
        if c.has("dimension") {
            let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
            return Ok(flag(matches!(c.get("dimension"), Some(Arg::Resource(d))
                if d.as_str() == dimension.id())));
        }
        if c.has("block") {
            let p = self.block_pos_arg(c, "pos")?;
            let Some(Arg::Block(b)) = c.get("block").cloned() else {
                return Ok((false, 0));
            };
            let state = self.world.block(p[0], p[1], p[2]);
            return Ok(flag(self.block_matches(&b, state)));
        }
        if c.has("biome") {
            let p = self.block_pos_arg(c, "pos")?;
            let Some(Arg::Resource(want_biome)) = c.get("biome").cloned() else {
                return Ok((false, 0));
            };
            let name = self.biome_name_at_block(p);
            return Ok(flag(
                name.is_some_and(|n| self.biome_matches(&want_biome, &n)),
            ));
        }
        if c.has("score") {
            return self.condition_score(c).map(flag);
        }
        if c.has("blocks") {
            return self.condition_blocks(c);
        }
        if c.has("range") && c.has("id") {
            return self.condition_stopwatch(c).map(flag);
        }
        if c.has("predicate") {
            let Some(Arg::Resource(id)) = c.get("predicate").cloned() else {
                return Ok((false, 0));
            };
            return Ok(flag(self.check_predicate(&id)?));
        }
        Ok((false, 0))
    }
    fn condition_stopwatch(&mut self, c: &Context) -> R<bool> {
        let Some(Arg::Resource(id)) = c.get("id").cloned() else {
            return Ok(false);
        };
        let Some(Arg::DoubleRange((lo, hi))) = c.get("range").cloned() else {
            return Ok(false);
        };
        let Some(t) = self.score.stopwatches.elapsed_seconds(&id) else {
            return Err(err("commands.stopwatch.does_not_exist", &[&id]));
        };
        Ok(lo.is_none_or(|l| t >= l) && hi.is_none_or(|h| t <= h))
    }
    fn condition_score(&mut self, c: &Context) -> R<bool> {
        let target = self.score_holder_names(c, "target", false)?;
        let Some(Arg::Str(to)) = c.get("targetObjective").cloned() else {
            return Ok(false);
        };
        if self.score.board.objective(&to).is_none() {
            return Err(err("arguments.objective.notFound", &[&to]));
        }
        let Some(a) = target
            .first()
            .and_then(|t| self.score.board.score(t, &to))
            .map(|s| s.value)
        else {
            return Ok(false);
        };
        if c.has("matches") {
            let Some(Arg::IntRange((lo, hi))) = c.get("range") else {
                return Ok(false);
            };
            return Ok(lo.is_none_or(|l| a >= l) && hi.is_none_or(|h| a <= h));
        }
        let source = self.score_holder_names(c, "source", false)?;
        let Some(Arg::Str(so)) = c.get("sourceObjective").cloned() else {
            return Ok(false);
        };
        if self.score.board.objective(&so).is_none() {
            return Err(err("arguments.objective.notFound", &[&so]));
        }
        let Some(b) = source
            .first()
            .and_then(|t| self.score.board.score(t, &so))
            .map(|s| s.value)
        else {
            return Ok(false);
        };
        let op = c.literals.last().map_or("", String::as_str);
        Ok(match op {
            "=" => a == b,
            "<" => a < b,
            "<=" => a <= b,
            ">" => a > b,
            ">=" => a >= b,
            _ => false,
        })
    }
    /// `if blocks`: the region at `start..end` equals the one at `destination`.
    fn condition_blocks(&mut self, c: &Context) -> R<(bool, i32)> {
        let (a, b, d) = (
            self.block_pos_arg(c, "start")?,
            self.block_pos_arg(c, "end")?,
            self.block_pos_arg(c, "destination")?,
        );
        let masked = c.has("masked");
        let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let area = i64::from(hi[0] - lo[0] + 1)
            * i64::from(hi[1] - lo[1] + 1)
            * i64::from(hi[2] - lo[2] + 1);
        if area > 32768 {
            return Err(err("commands.execute.blocks.toobig", &[&32768, &area]));
        }
        let offset = [d[0] - lo[0], d[1] - lo[1], d[2] - lo[2]];
        let mut count = 0;
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let src = self.world.block(x, y, z);
                    if masked && src == self.world.air {
                        continue;
                    }
                    let (dx, dy, dz) = (x + offset[0], y + offset[1], z + offset[2]);
                    if src != self.world.block(dx, dy, dz) {
                        return Ok((false, 0));
                    }
                    let (se, de) = (
                        self.world.block_entities.get(&(x, y, z)),
                        self.world.block_entities.get(&(dx, dy, dz)),
                    );
                    if let Some(se) = se {
                        let name = self.data_block_name([x, y, z]);
                        let Some(de) = de else {
                            return Ok((false, 0));
                        };
                        let st = horizon_client::save::encode_block_entity((0, 0, 0), se, &name);
                        let dt = horizon_client::save::encode_block_entity((0, 0, 0), de, &name);
                        if st != dt {
                            return Ok((false, 0));
                        }
                    }
                    count += 1;
                }
            }
        }
        Ok((true, count))
    }
    /// The biome name at a block position.
    fn biome_name_at_block(&self, p: [i32; 3]) -> Option<String> {
        let chunk = self.world.chunks.get(&(p[0] >> 4, p[2] >> 4))?;
        // The world's table (`world_biomes!`): a packed biome's id is past
        // the built-in list, and `Biomes::get` would clamp it to the last
        // vanilla one.
        Some(
            world_biomes!(self)
                .get(chunk.biome(p[0], p[1], p[2]))
                .name
                .clone(),
        )
    }
    /// A biome id or `#tag` matched against a biome name.
    pub(super) fn biome_matches(&self, wanted: &str, biome: &str) -> bool {
        match wanted.strip_prefix('#') {
            Some(tag) => self.biome_tag_has(tag, biome),
            None => wanted == biome,
        }
    }
    /// `#tag`: the biome tags the game's files and the world's packs define,
    /// nested tags expanded; an unknown tag falls back to the game's built-in
    /// table.
    fn biome_tag_has(&self, tag: &str, biome: &str) -> bool {
        let entries = self.tag_entries("worldgen/biome", tag);
        entries.iter().any(|id| id == biome)
            || (entries.is_empty() && self.res.mob_data().biome_has(tag, biome))
    }
}
