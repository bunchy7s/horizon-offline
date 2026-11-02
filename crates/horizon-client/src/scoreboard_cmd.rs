//! Execution of `/scoreboard`, `/team` and `/trigger` (ScoreboardCommand,
//! TeamCommand, TriggerCommand, `CommandResponseTracker`) against a
//! [`Scoreboard`], with the world supplied through [`Env`].
use crate::{
    command::{
        Arg, CmdResult, CommandError, Context,
        args::{ScoreHolderArg, Selector},
    },
    scoreboard::{
        self as sb, Comp, NumberFormat, RenderType, Scoreboard, Visibility, comp_text,
        criteria_read_only, literal,
    },
    text::{self, Text},
};

/// A score holder a command resolved: the scoreboard name and the name
/// shown in feedback.
#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub name: String,
    pub text: Text,
    /// The entity display name (`ScoreHolder.getDisplayName`), if it has one.
    pub display: Option<Comp>,
}
impl Holder {
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            text: Text::literal(name),
            display: None,
        }
    }
}
/// What the commands need from the game.
pub trait Env {
    fn sb(&self) -> &Scoreboard;
    fn sb_mut(&mut self) -> &mut Scoreboard;
    /// The holders a selector finds.
    fn select(&mut self, selector: &Selector) -> Vec<Holder>;
    /// A holder from a plain name (a player, a UUID, or just a name).
    fn named(&mut self, name: &str) -> Holder;
    /// The executing entity; `player` is whether it is the player.
    fn source(&mut self) -> Option<(Holder, bool)>;
    /// `ComponentArgument.getResolvedComponent`: selectors and scores resolved.
    fn resolve(&mut self, c: Comp) -> Comp {
        c
    }
    fn feedback(&mut self, message: Text);
}

pub fn err(key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
    CommandError::tr(key, args)
}
/// `ComponentUtils.formatList` with the gray ", " separator.
pub fn format_list(items: Vec<Text>) -> Text {
    let mut out = Text::empty();
    for (i, t) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(Text::literal(", ").with_color(text::GRAY));
        }
        out.push(t);
    }
    out
}
/// `ComponentUtils.formatList(Collection<String>)`: sorted, green.
pub fn format_names(names: Vec<String>) -> Text {
    let mut names = names;
    names.sort();
    format_list(
        names
            .into_iter()
            .map(|n| Text::literal(n).with_color(text::GREEN))
            .collect(),
    )
}

/// `CommandResponseTracker`.
#[derive(Default)]
pub struct Tracker {
    total: i32,
    count: usize,
    only: Option<Holder>,
    nz_count: usize,
    nz_only: Option<Holder>,
}
impl Tracker {
    pub fn track(&mut self, h: &Holder, value: i32) {
        self.total = self.total.wrapping_add(value);
        self.count += 1;
        self.only = (self.count == 1).then(|| h.clone());
        if value != 0 {
            self.nz_count += 1;
            self.nz_only = (self.nz_count == 1).then(|| h.clone());
        }
    }
    pub fn mark(&mut self, h: &Holder) {
        self.track(h, 1);
    }
    pub fn total(&self) -> i32 {
        self.total
    }
    /// `sendFeedback`: `zero` fails when nothing counts; `single` and
    /// `multiple` build the message.
    pub fn send(
        &self,
        env: &mut dyn Env,
        any: bool,
        zero: Option<CommandError>,
        single: impl Fn(&Holder, i32) -> Text,
        multiple: impl Fn(usize, i32) -> Text,
    ) -> CmdResult<i32> {
        let (message, total) = self.message(any, zero, single, multiple)?;
        env.feedback(message);
        Ok(total)
    }
    /// The feedback `send` would produce, and the command's result.
    pub fn message(
        &self,
        any: bool,
        zero: Option<CommandError>,
        single: impl Fn(&Holder, i32) -> Text,
        multiple: impl Fn(usize, i32) -> Text,
    ) -> CmdResult<(Text, i32)> {
        let (only, count) = if any {
            (&self.only, self.count)
        } else {
            (&self.nz_only, self.nz_count)
        };
        if let Some(e) = zero
            && count == 0
        {
            return Err(e);
        }
        let message = match only {
            Some(h) => single(h, self.total),
            None => multiple(count, self.total),
        };
        Ok((message, self.total))
    }
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn tr(key: &str, args: &[Text]) -> Text {
    Text::translate(key, args)
}

// ---- argument helpers ------------------------------------------------------

fn string(ctx: &Context, name: &str) -> String {
    match ctx.get(name) {
        Some(Arg::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
fn int(ctx: &Context, name: &str) -> i32 {
    match ctx.get(name) {
        Some(Arg::Int(v)) => *v,
        _ => 0,
    }
}
fn boolean(ctx: &Context, name: &str) -> bool {
    matches!(ctx.get(name), Some(Arg::Bool(true)))
}
/// A component argument (kept as JSON text), resolved against the source.
fn component(ctx: &Context, name: &str, env: &mut dyn Env) -> CmdResult<Comp> {
    let Some(Arg::Str(json)) = ctx.get(name) else {
        return Ok(literal(""));
    };
    let value: Comp =
        serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
    Ok(env.resolve(value))
}
fn objective_name(ctx: &Context, env: &dyn Env, name: &str) -> CmdResult<String> {
    let id = string(ctx, name);
    if env.sb().objective(&id).is_none() {
        return Err(err("arguments.objective.notFound", &[&id]));
    }
    Ok(id)
}
fn writable_objective(ctx: &Context, env: &dyn Env, name: &str) -> CmdResult<String> {
    let id = objective_name(ctx, env, name)?;
    let criteria = env.sb().objective(&id).map(|o| o.criteria.clone());
    if criteria.is_some_and(|c| criteria_read_only(&c)) {
        return Err(err("arguments.objective.readonly", &[&id]));
    }
    Ok(id)
}
fn team_name(ctx: &Context, env: &dyn Env, name: &str) -> CmdResult<String> {
    let id = string(ctx, name);
    if env.sb().team(&id).is_none() {
        return Err(err("team.notFound", &[&id]));
    }
    Ok(id)
}
/// `ScoreHolderArgument.getNames` (and `getNamesWithDefaultWildcard`).
pub fn holders(
    ctx: &Context,
    name: &str,
    env: &mut dyn Env,
    default_wildcard: bool,
) -> CmdResult<Vec<Holder>> {
    let Some(Arg::ScoreHolder(arg)) = ctx.get(name).cloned() else {
        return Ok(Vec::new());
    };
    resolve_holders(&arg, env, default_wildcard)
}
pub fn resolve_holders(
    arg: &ScoreHolderArg,
    env: &mut dyn Env,
    default_wildcard: bool,
) -> CmdResult<Vec<Holder>> {
    let found = match arg {
        ScoreHolderArg::Wildcard => {
            let tracked = if default_wildcard {
                env.sb().tracked()
            } else {
                Vec::new()
            };
            if tracked.is_empty() {
                return Err(err("argument.scoreHolder.empty", &[]));
            }
            return Ok(tracked.iter().map(|n| env.named(n)).collect());
        }
        ScoreHolderArg::Name(n) => vec![env.named(n)],
        ScoreHolderArg::Selector(s) => env.select(s),
    };
    if found.is_empty() {
        return Err(err("argument.entity.notfound.entity", &[]));
    }
    Ok(found)
}
fn one_holder(ctx: &Context, name: &str, env: &mut dyn Env) -> CmdResult<Holder> {
    Ok(holders(ctx, name, env, false)?.remove(0))
}

/// Run a `scoreboard.*`, `team.*` or `trigger*` command; `None` when `id`
/// is not one of them.
pub fn execute(id: &str, ctx: &Context, env: &mut dyn Env) -> Option<CmdResult<i32>> {
    if id.starts_with("scoreboard.") {
        Some(scoreboard(id, ctx, env))
    } else if id.starts_with("team.") {
        Some(team(id, ctx, env))
    } else if id.starts_with("trigger") {
        Some(trigger(id, ctx, env))
    } else {
        None
    }
}

// ---- /scoreboard -----------------------------------------------------------

fn scoreboard(id: &str, ctx: &Context, env: &mut dyn Env) -> CmdResult<i32> {
    match id {
        "scoreboard.objectives.list" => {
            let names: Vec<Text> = env
                .sb()
                .objectives
                .iter()
                .map(|o| o.formatted_display_name())
                .collect();
            let n = names.len();
            if n == 0 {
                env.feedback(tr("commands.scoreboard.objectives.list.empty", &[]));
            } else {
                env.feedback(tr(
                    "commands.scoreboard.objectives.list.success",
                    &[num(n), format_list(names)],
                ));
            }
            Ok(n as i32)
        }
        "scoreboard.objectives.add" => {
            let name = string(ctx, "objective");
            let criteria = string(ctx, "criteria");
            if env.sb().objective(&name).is_some() {
                return Err(err("commands.scoreboard.objectives.add.duplicate", &[]));
            }
            let display = if ctx.get("displayName").is_some() {
                component(ctx, "displayName", env)?
            } else {
                literal(&name)
            };
            let render = sb::criteria_render_type(&criteria);
            env.sb_mut()
                .add_objective(&name, &criteria, display, render, false, None);
            let shown = env
                .sb()
                .objective(&name)
                .map(|o| o.formatted_display_name())
                .unwrap_or_default();
            env.feedback(tr("commands.scoreboard.objectives.add.success", &[shown]));
            Ok(env.sb().objectives.len() as i32)
        }
        "scoreboard.objectives.modify.displayname" => {
            let name = objective_name(ctx, env, "objective")?;
            let display = component(ctx, "displayName", env)?;
            let o = env.sb_mut().objective_mut(&name).expect("checked");
            if o.display_name != display {
                o.display_name = display;
                let shown = o.formatted_display_name();
                env.feedback(tr(
                    "commands.scoreboard.objectives.modify.displayname",
                    &[Text::literal(name), shown],
                ));
            }
            Ok(0)
        }
        "scoreboard.objectives.modify.rendertype" => {
            let name = objective_name(ctx, env, "objective")?;
            let render = RenderType::by_id(ctx.literals.last().map_or("", String::as_str));
            let o = env.sb_mut().objective_mut(&name).expect("checked");
            if o.render_type != render {
                o.render_type = render;
                let shown = o.formatted_display_name();
                env.feedback(tr(
                    "commands.scoreboard.objectives.modify.rendertype",
                    &[shown],
                ));
            }
            Ok(0)
        }
        "scoreboard.objectives.modify.displayautoupdate" => {
            let name = objective_name(ctx, env, "objective")?;
            let value = boolean(ctx, "value");
            let o = env.sb_mut().objective_mut(&name).expect("checked");
            if o.display_auto_update != value {
                o.display_auto_update = value;
                let shown = o.formatted_display_name();
                env.feedback(tr(
                    &format!(
                        "commands.scoreboard.objectives.modify.displayAutoUpdate.{}",
                        if value { "enable" } else { "disable" }
                    ),
                    &[Text::literal(name), shown],
                ));
            }
            Ok(0)
        }
        "scoreboard.objectives.modify.numberformat" => {
            let name = objective_name(ctx, env, "objective")?;
            let format = number_format(ctx, env)?;
            let set = format.is_some();
            env.sb_mut()
                .objective_mut(&name)
                .expect("checked")
                .number_format = format;
            env.feedback(tr(
                if set {
                    "commands.scoreboard.objectives.modify.objectiveFormat.set"
                } else {
                    "commands.scoreboard.objectives.modify.objectiveFormat.clear"
                },
                &[Text::literal(name)],
            ));
            Ok(0)
        }
        "scoreboard.objectives.remove" => {
            let name = objective_name(ctx, env, "objective")?;
            let shown = env
                .sb()
                .objective(&name)
                .map(|o| o.formatted_display_name())
                .unwrap_or_default();
            env.sb_mut().remove_objective(&name);
            env.feedback(tr(
                "commands.scoreboard.objectives.remove.success",
                &[shown],
            ));
            Ok(env.sb().objectives.len() as i32)
        }
        "scoreboard.objectives.setdisplay" => {
            let slot = string(ctx, "slot");
            if ctx.get("objective").is_none() {
                if !env.sb().display.contains_key(&slot) {
                    return Err(err(
                        "commands.scoreboard.objectives.display.alreadyEmpty",
                        &[],
                    ));
                }
                env.sb_mut().set_display_objective(&slot, None);
                env.feedback(tr(
                    "commands.scoreboard.objectives.display.cleared",
                    &[Text::literal(slot)],
                ));
                return Ok(0);
            }
            let name = objective_name(ctx, env, "objective")?;
            if env.sb().display.get(&slot) == Some(&name) {
                return Err(err(
                    "commands.scoreboard.objectives.display.alreadySet",
                    &[],
                ));
            }
            env.sb_mut().set_display_objective(&slot, Some(&name));
            let shown = env
                .sb()
                .objective(&name)
                .map(|o| o.display_text())
                .unwrap_or_default();
            env.feedback(tr(
                "commands.scoreboard.objectives.display.set",
                &[Text::literal(slot), shown],
            ));
            Ok(0)
        }
        _ => players(id, ctx, env),
    }
}

/// `addNumberFormats`: the format the chosen literal denotes (None = clear).
fn number_format(ctx: &Context, env: &mut dyn Env) -> CmdResult<Option<NumberFormat>> {
    if ctx.has("blank") {
        Ok(Some(NumberFormat::Blank))
    } else if ctx.has("fixed") {
        Ok(Some(NumberFormat::Fixed(component(ctx, "contents", env)?)))
    } else if ctx.has("styled") {
        let Some(Arg::Str(json)) = ctx.get("style") else {
            return Ok(None);
        };
        let v: Comp =
            serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
        Ok(Some(NumberFormat::Styled(v)))
    } else {
        Ok(None)
    }
}

fn objective_text(env: &dyn Env, name: &str) -> Text {
    env.sb()
        .objective(name)
        .map(|o| o.formatted_display_name())
        .unwrap_or_default()
}

fn players(id: &str, ctx: &Context, env: &mut dyn Env) -> CmdResult<i32> {
    match id {
        "scoreboard.players.list" => {
            if ctx.get("target").is_none() {
                let tracked: Vec<Holder> = env
                    .sb()
                    .tracked()
                    .iter()
                    .map(|n| Holder::named(n))
                    .collect();
                let n = tracked.len();
                if n == 0 {
                    env.feedback(tr("commands.scoreboard.players.list.empty", &[]));
                } else {
                    let names: Vec<Text> = tracked.into_iter().map(|h| h.text).collect();
                    env.feedback(tr(
                        "commands.scoreboard.players.list.success",
                        &[num(n), format_list(names)],
                    ));
                }
                return Ok(n as i32);
            }
            let holder = one_holder(ctx, "target", env)?;
            let scores: Vec<(Text, i32)> = env
                .sb()
                .holder_scores(&holder.name)
                .into_iter()
                .map(|(o, v)| (o.formatted_display_name(), v))
                .collect();
            if scores.is_empty() {
                env.feedback(tr(
                    "commands.scoreboard.players.list.entity.empty",
                    &[holder.text],
                ));
            } else {
                env.feedback(tr(
                    "commands.scoreboard.players.list.entity.success",
                    &[holder.text, num(scores.len())],
                ));
                for (o, v) in &scores {
                    env.feedback(tr(
                        "commands.scoreboard.players.list.entity.entry",
                        &[o.clone(), num(v)],
                    ));
                }
            }
            Ok(scores.len() as i32)
        }
        "scoreboard.players.get" => {
            let target = one_holder(ctx, "target", env)?;
            let objective = objective_name(ctx, env, "objective")?;
            let Some(score) = env.sb().score(&target.name, &objective).map(|s| s.value) else {
                return Err(CommandError::new(tr(
                    "commands.scoreboard.players.get.null",
                    &[Text::literal(objective), target.text],
                )));
            };
            let shown = objective_text(env, &objective);
            env.feedback(tr(
                "commands.scoreboard.players.get.success",
                &[target.text, num(score), shown],
            ));
            Ok(score)
        }
        "scoreboard.players.set" | "scoreboard.players.add" | "scoreboard.players.remove" => {
            let targets = holders(ctx, "targets", env, true)?;
            let objective = writable_objective(ctx, env, "objective")?;
            let value = int(ctx, "score");
            let verb = id.rsplit('.').next().unwrap_or("set");
            let mut tracker = Tracker::default();
            for t in &targets {
                let v = match verb {
                    "set" => {
                        env.sb_mut()
                            .set_with(&t.name, &objective, value, t.display.as_ref());
                        value
                    }
                    "add" => {
                        let v = env.sb_mut().get(&t.name, &objective).wrapping_add(value);
                        env.sb_mut()
                            .set_with(&t.name, &objective, v, t.display.as_ref());
                        v
                    }
                    _ => {
                        let v = env.sb_mut().get(&t.name, &objective).wrapping_sub(value);
                        env.sb_mut()
                            .set_with(&t.name, &objective, v, t.display.as_ref());
                        v
                    }
                };
                tracker.track(t, v);
            }
            let shown = objective_text(env, &objective);
            let key = format!("commands.scoreboard.players.{verb}.success");
            if verb == "set" {
                tracker.send(
                    env,
                    false,
                    None,
                    |h, _| {
                        tr(
                            &format!("{key}.single"),
                            &[shown.clone(), h.text.clone(), num(value)],
                        )
                    },
                    |n, _| {
                        tr(
                            &format!("{key}.multiple"),
                            &[shown.clone(), num(n), num(value)],
                        )
                    },
                )
            } else {
                tracker.send(
                    env,
                    true,
                    None,
                    |h, total| {
                        tr(
                            &format!("{key}.single"),
                            &[num(value), shown.clone(), h.text.clone(), num(total)],
                        )
                    },
                    |n, _| {
                        tr(
                            &format!("{key}.multiple"),
                            &[num(value), shown.clone(), num(n)],
                        )
                    },
                )
            }
        }
        "scoreboard.players.reset" => {
            let targets = holders(ctx, "targets", env, true)?;
            let mut tracker = Tracker::default();
            if ctx.get("objective").is_some() {
                let objective = objective_name(ctx, env, "objective")?;
                for t in &targets {
                    env.sb_mut().reset_one(&t.name, &objective);
                    tracker.mark(t);
                }
                let shown = objective_text(env, &objective);
                tracker.send(
                    env,
                    false,
                    None,
                    |h, _| {
                        tr(
                            "commands.scoreboard.players.reset.specific.single",
                            &[shown.clone(), h.text.clone()],
                        )
                    },
                    |n, _| {
                        tr(
                            "commands.scoreboard.players.reset.specific.multiple",
                            &[shown.clone(), num(n)],
                        )
                    },
                )
            } else {
                for t in &targets {
                    env.sb_mut().reset_all(&t.name);
                    tracker.mark(t);
                }
                tracker.send(
                    env,
                    false,
                    None,
                    |h, _| {
                        tr(
                            "commands.scoreboard.players.reset.all.single",
                            std::slice::from_ref(&h.text),
                        )
                    },
                    |n, _| tr("commands.scoreboard.players.reset.all.multiple", &[num(n)]),
                )
            }
        }
        "scoreboard.players.enable" => {
            let targets = holders(ctx, "targets", env, true)?;
            let objective = objective_name(ctx, env, "objective")?;
            let criteria = env.sb().objective(&objective).map(|o| o.criteria.clone());
            if criteria.as_deref() != Some(sb::CRITERIA_TRIGGER) {
                return Err(err("commands.scoreboard.players.enable.invalid", &[]));
            }
            let mut tracker = Tracker::default();
            for t in &targets {
                let s = env.sb_mut().score_mut(&t.name, &objective);
                if s.locked {
                    s.locked = false;
                    tracker.mark(t);
                }
            }
            let shown = objective_text(env, &objective);
            tracker.send(
                env,
                false,
                Some(err("commands.scoreboard.players.enable.failed", &[])),
                |h, _| {
                    tr(
                        "commands.scoreboard.players.enable.success.single",
                        &[shown.clone(), h.text.clone()],
                    )
                },
                |n, _| {
                    tr(
                        "commands.scoreboard.players.enable.success.multiple",
                        &[shown.clone(), num(n)],
                    )
                },
            )
        }
        "scoreboard.players.display.name" => {
            let targets = holders(ctx, "targets", env, true)?;
            let objective = objective_name(ctx, env, "objective")?;
            let display = if ctx.get("name").is_some() {
                Some(component(ctx, "name", env)?)
            } else {
                None
            };
            let mut tracker = Tracker::default();
            for t in &targets {
                env.sb_mut().score_mut(&t.name, &objective).display = display.clone();
                tracker.mark(t);
            }
            let shown = objective_text(env, &objective);
            let set_text = display.as_ref().map(comp_text);
            match set_text {
                None => tracker.send(
                    env,
                    false,
                    None,
                    |h, _| {
                        tr(
                            "commands.scoreboard.players.display.name.clear.success.single",
                            &[h.text.clone(), shown.clone()],
                        )
                    },
                    |n, _| {
                        tr(
                            "commands.scoreboard.players.display.name.clear.success.multiple",
                            &[num(n), shown.clone()],
                        )
                    },
                ),
                Some(d) => tracker.send(
                    env,
                    false,
                    None,
                    |h, _| {
                        tr(
                            "commands.scoreboard.players.display.name.set.success.single",
                            &[d.clone(), h.text.clone(), shown.clone()],
                        )
                    },
                    |n, _| {
                        tr(
                            "commands.scoreboard.players.display.name.set.success.multiple",
                            &[d.clone(), num(n), shown.clone()],
                        )
                    },
                ),
            }
        }
        "scoreboard.players.display.numberformat" => {
            let targets = holders(ctx, "targets", env, true)?;
            let objective = objective_name(ctx, env, "objective")?;
            let format = number_format(ctx, env)?;
            let set = format.is_some();
            let mut tracker = Tracker::default();
            for t in &targets {
                env.sb_mut().score_mut(&t.name, &objective).number_format = format.clone();
                tracker.mark(t);
            }
            let shown = objective_text(env, &objective);
            let kind = if set { "set" } else { "clear" };
            tracker.send(
                env,
                false,
                None,
                |h, _| {
                    tr(
                        &format!(
                            "commands.scoreboard.players.display.numberFormat.{kind}.success.single"
                        ),
                        &[h.text.clone(), shown.clone()],
                    )
                },
                |n, _| {
                    tr(
                        &format!(
                            "commands.scoreboard.players.display.numberFormat.{kind}.success.multiple"
                        ),
                        &[num(n), shown.clone()],
                    )
                },
            )
        }
        "scoreboard.players.operation" => {
            let targets = holders(ctx, "targets", env, true)?;
            let target_objective = writable_objective(ctx, env, "targetObjective")?;
            let op = string(ctx, "operation");
            let sources = holders(ctx, "source", env, true)?;
            let source_objective = objective_name(ctx, env, "sourceObjective")?;
            let mut tracker = Tracker::default();
            for t in &targets {
                for s in &sources {
                    let b = env.sb_mut().get(&s.name, &source_objective);
                    let a = env.sb_mut().get(&t.name, &target_objective);
                    let (na, nb) = apply_operation(&op, a, b)?;
                    env.sb_mut()
                        .set_with(&t.name, &target_objective, na, t.display.as_ref());
                    if op == "><" {
                        env.sb_mut().set(&s.name, &source_objective, nb);
                    }
                }
                let v = env.sb_mut().get(&t.name, &target_objective);
                tracker.track(t, v);
            }
            let shown = objective_text(env, &target_objective);
            tracker.send(
                env,
                true,
                None,
                |h, total| {
                    tr(
                        "commands.scoreboard.players.operation.success.single",
                        &[shown.clone(), h.text.clone(), num(total)],
                    )
                },
                |n, _| {
                    tr(
                        "commands.scoreboard.players.operation.success.multiple",
                        &[shown.clone(), num(n)],
                    )
                },
            )
        }
        _ => Err(err("command.failed", &[])),
    }
}

/// `OperationArgument.getSimpleOperation`: the new (target, source) values.
pub fn apply_operation(op: &str, a: i32, b: i32) -> CmdResult<(i32, i32)> {
    let div0 = || err("arguments.operation.div0", &[]);
    Ok(match op {
        "=" => (b, b),
        "+=" => (a.wrapping_add(b), b),
        "-=" => (a.wrapping_sub(b), b),
        "*=" => (a.wrapping_mul(b), b),
        "/=" => {
            if b == 0 {
                return Err(div0());
            }
            (
                a.wrapping_div(b) - i32::from((a.wrapping_rem(b) != 0) && ((a < 0) != (b < 0))),
                b,
            )
        }
        "%=" => {
            if b == 0 {
                return Err(div0());
            }
            let remainder = a.wrapping_rem(b);
            // Floor remainder has the divisor's sign; only correct opposite signs.
            let remainder = if remainder != 0 && ((remainder < 0) != (b < 0)) {
                remainder + b
            } else {
                remainder
            };
            (remainder, b)
        }
        "<" => (a.min(b), b),
        ">" => (a.max(b), b),
        "><" => (b, a),
        _ => return Err(err("arguments.operation.invalid", &[])),
    })
}

// ---- /team -----------------------------------------------------------------

fn team(id: &str, ctx: &Context, env: &mut dyn Env) -> CmdResult<i32> {
    let formatted = |env: &dyn Env, name: &str| {
        env.sb()
            .team(name)
            .map(|t| t.formatted_display_name())
            .unwrap_or_default()
    };
    match id {
        "team.list" => {
            if ctx.get("team").is_some() {
                let name = team_name(ctx, env, "team")?;
                let members: Vec<String> = env
                    .sb()
                    .team(&name)
                    .map(|t| t.players.iter().cloned().collect())
                    .unwrap_or_default();
                let shown = formatted(env, &name);
                let n = members.len();
                if n == 0 {
                    env.feedback(tr("commands.team.list.members.empty", &[shown]));
                } else {
                    env.feedback(tr(
                        "commands.team.list.members.success",
                        &[shown, num(n), format_names(members)],
                    ));
                }
                return Ok(n as i32);
            }
            let teams: Vec<Text> = env
                .sb()
                .teams
                .iter()
                .map(|t| t.formatted_display_name())
                .collect();
            let n = teams.len();
            if n == 0 {
                env.feedback(tr("commands.team.list.teams.empty", &[]));
            } else {
                env.feedback(tr(
                    "commands.team.list.teams.success",
                    &[num(n), format_list(teams)],
                ));
            }
            Ok(n as i32)
        }
        "team.add" => {
            let name = string(ctx, "team");
            if env.sb().team(&name).is_some() {
                return Err(err("commands.team.add.duplicate", &[]));
            }
            let display = if ctx.get("displayName").is_some() {
                component(ctx, "displayName", env)?
            } else {
                literal(&name)
            };
            env.sb_mut().add_team(&name).display_name = display;
            let shown = formatted(env, &name);
            env.feedback(tr("commands.team.add.success", &[shown]));
            Ok(env.sb().teams.len() as i32)
        }
        "team.remove" => {
            let name = team_name(ctx, env, "team")?;
            let shown = formatted(env, &name);
            env.sb_mut().remove_team(&name);
            env.feedback(tr("commands.team.remove.success", &[shown]));
            Ok(env.sb().teams.len() as i32)
        }
        "team.empty" => {
            let name = team_name(ctx, env, "team")?;
            let members: Vec<String> = env
                .sb()
                .team(&name)
                .map(|t| t.players.iter().cloned().collect())
                .unwrap_or_default();
            if members.is_empty() {
                return Err(err("commands.team.empty.unchanged", &[]));
            }
            for m in &members {
                env.sb_mut().remove_from_team(m);
            }
            let shown = formatted(env, &name);
            env.feedback(tr(
                "commands.team.empty.success",
                &[num(members.len()), shown],
            ));
            Ok(members.len() as i32)
        }
        "team.join" => {
            let name = team_name(ctx, env, "team")?;
            let members = if ctx.get("members").is_some() {
                holders(ctx, "members", env, true)?
            } else {
                match env.source() {
                    Some((h, _)) => vec![h],
                    None => return Err(err("permissions.requires.entity", &[])),
                }
            };
            let mut tracker = Tracker::default();
            for m in &members {
                let joined = env.sb_mut().add_to_team(&m.name, &name);
                // The message reads the display name after the change.
                tracker.track(&env.named(&m.name), i32::from(joined));
            }
            let shown = formatted(env, &name);
            tracker.send(
                env,
                false,
                None,
                |h, _| {
                    tr(
                        "commands.team.join.success.single",
                        &[h.text.clone(), shown.clone()],
                    )
                },
                |n, _| {
                    tr(
                        "commands.team.join.success.multiple",
                        &[num(n), shown.clone()],
                    )
                },
            )
        }
        "team.leave" => {
            let members = holders(ctx, "members", env, true)?;
            let mut tracker = Tracker::default();
            for m in &members {
                let left = env.sb_mut().remove_from_team(&m.name);
                tracker.track(&env.named(&m.name), i32::from(left));
            }
            tracker.send(
                env,
                false,
                None,
                |h, _| {
                    tr(
                        "commands.team.leave.success.single",
                        std::slice::from_ref(&h.text),
                    )
                },
                |n, _| tr("commands.team.leave.success.multiple", &[num(n)]),
            )
        }
        _ => team_modify(id, ctx, env),
    }
}

fn team_modify(id: &str, ctx: &Context, env: &mut dyn Env) -> CmdResult<i32> {
    let name = team_name(ctx, env, "team")?;
    let shown = |env: &dyn Env| {
        env.sb()
            .team(&name)
            .map(|t| t.formatted_display_name())
            .unwrap_or_default()
    };
    let last = ctx.literals.last().cloned().unwrap_or_default();
    match id {
        "team.modify.displayname" => {
            let display = component(ctx, "displayName", env)?;
            let team = env.sb_mut().team_mut(&name).expect("checked");
            if team.display_name == display {
                return Err(err("commands.team.option.name.unchanged", &[]));
            }
            team.display_name = display;
            let s = shown(env);
            env.feedback(tr("commands.team.option.name.success", &[s]));
        }
        "team.modify.color" | "team.modify.color.reset" => {
            let reset = id.ends_with("reset");
            let value = (!reset).then(|| string(ctx, "value"));
            let team = env.sb_mut().team_mut(&name).expect("checked");
            if team.color == value {
                return Err(err("commands.team.option.color.unchanged", &[]));
            }
            team.color.clone_from(&value);
            let s = shown(env);
            match value {
                Some(c) => env.feedback(tr(
                    "commands.team.option.color.success",
                    &[s, Text::literal(c)],
                )),
                None => env.feedback(tr("commands.team.option.color.clear.success", &[s])),
            }
        }
        "team.modify.friendlyfire" | "team.modify.seefriendlyinvisibles" => {
            let allowed = boolean(ctx, "allowed");
            let fire = id.ends_with("friendlyfire");
            let team = env.sb_mut().team_mut(&name).expect("checked");
            let current = if fire {
                team.allow_friendly_fire
            } else {
                team.see_friendly_invisibles
            };
            let base = if fire {
                "commands.team.option.friendlyfire"
            } else {
                "commands.team.option.seeFriendlyInvisibles"
            };
            if current == allowed {
                return Err(err(
                    &format!(
                        "{base}.{}",
                        if allowed {
                            "alreadyEnabled"
                        } else {
                            "alreadyDisabled"
                        }
                    ),
                    &[],
                ));
            }
            if fire {
                team.allow_friendly_fire = allowed;
            } else {
                team.see_friendly_invisibles = allowed;
            }
            let s = shown(env);
            env.feedback(tr(
                &format!("{base}.{}", if allowed { "enabled" } else { "disabled" }),
                &[s],
            ));
        }
        "team.modify.nametagvisibility" | "team.modify.deathmessagevisibility" => {
            let v = Visibility::by_name(&last).unwrap_or(Visibility::Always);
            let nametag = id.ends_with("nametagvisibility");
            let team = env.sb_mut().team_mut(&name).expect("checked");
            let slot = if nametag {
                &mut team.nametag_visibility
            } else {
                &mut team.death_message_visibility
            };
            let base = if nametag {
                "commands.team.option.nametagVisibility"
            } else {
                "commands.team.option.deathMessageVisibility"
            };
            if *slot == v {
                return Err(err(&format!("{base}.unchanged"), &[]));
            }
            *slot = v;
            let s = shown(env);
            env.feedback(tr(&format!("{base}.success"), &[s, v.display_name()]));
        }
        "team.modify.collisionrule" => {
            let rule = sb::CollisionRule::by_name(&last).unwrap_or(sb::CollisionRule::Always);
            let team = env.sb_mut().team_mut(&name).expect("checked");
            if team.collision_rule == rule {
                return Err(err("commands.team.option.collisionRule.unchanged", &[]));
            }
            team.collision_rule = rule;
            let s = shown(env);
            env.feedback(tr(
                "commands.team.option.collisionRule.success",
                &[s, rule.display_name()],
            ));
        }
        "team.modify.prefix" | "team.modify.suffix" => {
            let prefix = id.ends_with("prefix");
            let value = component(ctx, if prefix { "prefix" } else { "suffix" }, env)?;
            let team = env.sb_mut().team_mut(&name).expect("checked");
            if prefix {
                team.prefix = value.clone();
            } else {
                team.suffix = value.clone();
            }
            env.feedback(tr(
                if prefix {
                    "commands.team.option.prefix.success"
                } else {
                    "commands.team.option.suffix.success"
                },
                &[comp_text(&value)],
            ));
            return Ok(1);
        }
        _ => return Err(err("command.failed", &[])),
    }
    Ok(0)
}

// ---- /trigger --------------------------------------------------------------

fn trigger(id: &str, ctx: &Context, env: &mut dyn Env) -> CmdResult<i32> {
    let Some((player, true)) = env.source() else {
        return Err(err("permissions.requires.player", &[]));
    };
    let objective = objective_name(ctx, env, "objective")?;
    // getScore: only primed trigger objectives, locked again afterwards.
    let criteria = env.sb().objective(&objective).map(|o| o.criteria.clone());
    if criteria.as_deref() != Some(sb::CRITERIA_TRIGGER) {
        return Err(err("commands.trigger.failed.invalid", &[]));
    }
    if env
        .sb()
        .score(&player.name, &objective)
        .is_none_or(|s| s.locked)
    {
        return Err(err("commands.trigger.failed.unprimed", &[]));
    }
    env.sb_mut().score_mut(&player.name, &objective).locked = true;
    let shown = objective_text(env, &objective);
    let (value, message) = match id {
        "trigger.add" => {
            let amount = int(ctx, "value");
            let v = env.sb_mut().add(&player.name, &objective, amount);
            (v, tr("commands.trigger.add.success", &[shown, num(amount)]))
        }
        "trigger.set" => {
            let amount = int(ctx, "value");
            env.sb_mut().set(&player.name, &objective, amount);
            (
                amount,
                tr("commands.trigger.set.success", &[shown, num(amount)]),
            )
        }
        _ => {
            let v = env.sb_mut().add(&player.name, &objective, 1);
            (v, tr("commands.trigger.simple.success", &[shown]))
        }
    };
    env.feedback(message);
    Ok(value)
}

#[cfg(test)]
#[path = "scoreboard_cmd_tests.rs"]
mod tests;
