//! Commands that act on the player and the hosted game rather than the world:
//! `/spectate` (SpectateCommand), `/waypoint` (WaypointCommand), `/dialog`
//! (DialogCommand), `/posteffect` (PostEffectCommand), `/kick`,
//! `/fetchprofile`, `/publish`, `/unpublish`, `/debug`, `/jfr` and
//! `/version`, with vanilla's feedback and failures.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    command::CommandError,
    text::{self, Style, Text},
};
use horizon_protocol::packets::{
    Empty,
    play::{self, Clientbound as C, DialogHolder},
};
use std::{collections::HashMap, time::Instant};

/// Waypoint.Icon: the locator bar style and colour of a transmitter.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct WaypointIcon {
    pub style: String,
    pub color: Option<i32>,
}
impl Default for WaypointIcon {
    fn default() -> Self {
        Self {
            style: "minecraft:default".into(),
            color: None,
        }
    }
}

/// Server-side player and server state these commands keep.
#[derive(Default)]
pub(in crate::game) struct CommandExtras {
    /// ServerPlayer.camera: the entity `/spectate` attached the view to.
    pub camera: Option<Target>,
    /// ServerPlayer.postEffects, in insertion order.
    pub post_effects: Vec<String>,
    /// MinecraftServer's tick profiler (`/debug start`): when it started and
    /// the tick count then.
    pub profiler: Option<(Instant, u64)>,
    /// `debugCommandProfilerDelayStart`: the profiler starts with the next tick.
    pub profiler_delay: bool,
    /// Messages the server sends on a later tick (profile lookups).
    pub pending: Vec<Text>,
    /// The `/jfr start` recording: when it started and the tick count then.
    pub jfr: Option<(Instant, u64, String)>,
    /// Waypoint icons changed by `/waypoint modify`.
    pub waypoint_icons: HashMap<Target, WaypointIcon>,
    /// `/datapack disable vanilla` took the built-in pack out of the selection.
    pub vanilla_disabled: bool,
    /// The waypoint manager's tracked transmitters (None until first used).
    pub waypoints: Option<Vec<Target>>,
    /// Entities whose waypoint transmit range changed since they last ticked.
    pub waypoint_dirty: Vec<Target>,
    /// Attribute maps of living entities whose attributes Horizon does not
    /// model (an armor stand's, a mob's beyond its modelled three).
    pub attributes: HashMap<Target, horizon_world::attributes::entity::EntityAttributes>,
    /// The `CommandResultCallback` results of the outermost source, when a
    /// harness records them.
    pub results: Option<Vec<(bool, i32)>>,
    /// The running `/pregen` task.
    pub pregen: Option<super::pregen::PregenTask>,
}

use horizon_core::version::{BUILD_TIME, DATA_VERSION, STABLE, VANILLA_VERSION, VERSION_ID};

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn tr_error(key: &str, args: &[Text]) -> CommandError {
    CommandError::new(Text::translate(key, args))
}
/// `String.format(Locale.ROOT, "%.2f", v)`.
fn two_places(v: f64) -> String {
    format!("{v:.2}")
}
/// `Util.getFilenameFormattedDateTime` (`yyyy-MM-dd_HH.mm.ss`, local time).
fn filename_date_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}.{mi:02}.{s:02}")
}
/// UTC civil date and time of a Unix timestamp.
fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

impl Game<'_> {
    /// CommandSourceStack.getPlayerOrException.
    fn source_player(&self) -> R<Target> {
        match self.command_source.map(|s| s.entity) {
            None | Some(Some(Target::Player)) => Ok(Target::Player),
            _ => Err(err("permissions.requires.player", &[])),
        }
    }
    /// EntityArgument.getEntity / getPlayer.
    fn one_entity(&mut self, ctx: &Context, name: &str, player: bool) -> R<Target> {
        Ok(self.entities(ctx, name, player)?[0])
    }

    // ---- /spectate ------------------------------------------------------

    /// `SpectateCommand.spectate`.
    pub(super) fn cmd_spectate(&mut self, ctx: &Context) -> R<i32> {
        let target = if ctx.get("target").is_some() {
            Some(self.one_entity(ctx, "target", false)?)
        } else {
            None
        };
        let player = if ctx.get("player").is_some() {
            self.one_entity(ctx, "player", true)?
        } else {
            self.source_player()?
        };
        if target == Some(player) {
            return Err(err("commands.spectate.self", &[]));
        }
        if !self.spectator() {
            return Err(tr_error(
                "commands.spectate.not_spectator",
                &[self.target_name(player)],
            ));
        }
        // EntityType.clientTrackingRange: only a marker's is zero.
        if let Some(t) = target
            && self.target_type(t) == "minecraft:marker"
        {
            return Err(tr_error(
                "commands.spectate.cannot_spectate",
                &[self.target_name(t)],
            ));
        }
        self.set_camera(target);
        match target {
            Some(t) => {
                let name = self.target_name(t);
                self.feedback(Text::translate(
                    "commands.spectate.success.started",
                    &[name],
                ));
            }
            None => self.feedback(Text::translate("commands.spectate.success.stopped", &[])),
        }
        Ok(1)
    }
    /// `ServerPlayer.setCamera`: a new camera moves the player to it.
    pub(in crate::game) fn set_camera(&mut self, target: Option<Target>) {
        let target = target.filter(|t| *t != Target::Player);
        if self.cmds.camera == target {
            return;
        }
        self.cmds.camera = target;
        if let Some(t) = target
            && let Some(p) = self.target_position(t)
            && let Some(player) = self.player.as_mut()
        {
            player.position = horizon_core::geometry::Vec3::new(p[0], p[1], p[2]);
            self.previous_position = player.position;
        }
    }
    /// Per-tick server work of these commands: the delayed tick profiler
    /// start, queued messages and the spectator camera.
    pub(in crate::game) fn tick_command_extras(&mut self) {
        if self.cmds.profiler_delay {
            self.cmds.profiler_delay = false;
            self.cmds.profiler = Some((Instant::now(), self.ticks));
        }
        for message in std::mem::take(&mut self.cmds.pending) {
            self.system_message(message);
        }
        self.tick_camera();
        self.tick_waypoints();
    }
    /// The camera part of `ServerPlayer.tick`: follow a living camera, drop a
    /// removed one, and stop when the player sneaks or leaves spectator mode.
    fn tick_camera(&mut self) {
        let Some(t) = self.cmds.camera else {
            return;
        };
        if !self.spectator() {
            self.set_camera(None);
            return;
        }
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft)
            && self.screen == super::Screen::Playing;
        match self.target_position(t) {
            Some(p) if !sneaking => {
                let rot = self.target_rotation(t);
                if let Some(player) = self.player.as_mut() {
                    player.position = horizon_core::geometry::Vec3::new(p[0], p[1], p[2]);
                }
                self.yaw = rot[0];
                self.pitch = rot[1];
            }
            _ => self.set_camera(None),
        }
    }
    /// The eye height the camera follows (`Camera.tick`'s entity).
    pub(in crate::game) fn camera_eye_height(&self) -> Option<f32> {
        self.cmds.camera.map(|t| self.target_eye_height(t) as f32)
    }

    // ---- /waypoint ------------------------------------------------------

    /// A guest's body is only `Player` during its turn; retained command state
    /// must use its connection so the next server tick still finds that body.
    pub(super) fn waypoint_target(&self, t: Target) -> Target {
        match t {
            Target::Player => self.lan_acting.map_or(t, Target::Guest),
            _ => t,
        }
    }

    /// A living entity's `waypoint_transmit_range` (`isTransmittingWaypoint`).
    fn transmit_range(&mut self, t: Target) -> f64 {
        if matches!(t, Target::Guest(_)) {
            return self
                .for_player(t, |g| g.transmit_range(Target::Player))
                .unwrap_or(0.0);
        }
        if t == Target::Player {
            return self
                .player
                .as_mut()
                .and_then(|p| {
                    let k = p.attributes.key("minecraft:waypoint_transmit_range")?;
                    p.attributes
                        .value(Some(k), horizon_world::attribute_map::ValueKind::Computed)
                        .ok()
                })
                .unwrap_or(0.0);
        }
        let catalog = self.motion.attributes.clone();
        let Some(key) = catalog.id("minecraft:waypoint_transmit_range") else {
            return 0.0;
        };
        match self.cmds.attributes.get_mut(&t) {
            Some(map) => map.value(key).unwrap_or(0.0),
            None => 0.0,
        }
    }
    /// `ServerWaypointManager.transmitters()`: the tracked set. The player
    /// joins tracked when transmitting; a changed range takes effect when the
    /// entity next ticks (`LivingEntity.onAttributeUpdated`), and
    /// `/waypoint modify` re-tracks its waypoint whatever its range.
    fn waypoint_transmitters(&mut self) -> Vec<Target> {
        if self.cmds.waypoints.is_none() {
            let tracked = if self.transmit_range(Target::Player) > 0.0 {
                vec![self.waypoint_target(Target::Player)]
            } else {
                Vec::new()
            };
            self.cmds.waypoints = Some(tracked);
        }
        let tracked = self.cmds.waypoints.clone().unwrap_or_default();
        let alive: Vec<Target> = tracked
            .into_iter()
            .filter(|t| self.target_position(*t).is_some())
            .collect();
        self.cmds.waypoints = Some(alive.clone());
        alive
            .into_iter()
            .map(|t| match t {
                Target::Guest(conn) if self.lan_acting == Some(conn) => Target::Player,
                _ => t,
            })
            .collect()
    }
    /// The waypoint part of a living entity's tick: dirty transmit ranges
    /// track or untrack it.
    pub(in crate::game) fn tick_waypoints(&mut self) {
        let dirty = std::mem::take(&mut self.cmds.waypoint_dirty);
        if dirty.is_empty() {
            return;
        }
        self.waypoint_transmitters();
        for t in dirty {
            let on = self.transmit_range(t) > 0.0;
            if let Some(tracked) = self.cmds.waypoints.as_mut() {
                tracked.retain(|x| *x != t);
                if on {
                    tracked.push(t);
                }
            }
        }
    }
    fn waypoint_icon(&self, t: Target) -> WaypointIcon {
        self.cmds
            .waypoint_icons
            .get(&self.waypoint_target(t))
            .cloned()
            .unwrap_or_default()
    }
    /// `WaypointArgument.getWaypoint`.
    fn waypoint_arg(&mut self, ctx: &Context) -> R<Target> {
        let t = self.one_entity(ctx, "waypoint", false)?;
        if !self.is_living(t) {
            return Err(err("argument.waypoint.invalid", &[]));
        }
        Ok(t)
    }
    fn is_living(&self, t: Target) -> bool {
        matches!(t, Target::Player | Target::Mob(_) | Target::Stand(_))
    }
    pub(super) fn cmd_waypoint(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "waypoint.list" {
            let dimension = self.dim.id();
            let waypoints = self.waypoint_transmitters();
            if waypoints.is_empty() {
                self.feedback(Text::translate(
                    "commands.waypoint.list.empty",
                    &[Text::literal(dimension)],
                ));
                return Ok(0);
            }
            let mut names = Text::empty();
            for (i, t) in waypoints.iter().enumerate() {
                if i > 0 {
                    names.push(", ");
                }
                let pos = self
                    .target_position(*t)
                    .unwrap_or_default()
                    .map(|v| v.floor() as i32);
                let color = self.waypoint_icon(*t).color.unwrap_or(-1) as u32 & 0xFF_FFFF;
                let style = Style {
                    color: Some(color),
                    click: Some(text::ClickEvent::SuggestCommand(format!(
                        "/execute in {dimension} run tp @s {} {} {}",
                        pos[0], pos[1], pos[2]
                    ))),
                    hover: Some(Box::new(Text::translate("chat.coordinates.tooltip", &[]))),
                    ..Style::default()
                };
                // getFeedbackDisplayName().copy().withStyle(color): the
                // waypoint colour replaces the team colour of the name.
                let name = match (
                    *t,
                    self.score
                        .board
                        .players_team(super::super::chat::PLAYER_NAME),
                ) {
                    (Target::Player, Some(team)) => {
                        let mut n = Text::empty();
                        n.push(horizon_client::scoreboard::comp_text(&team.prefix));
                        n.push(super::super::chat::PLAYER_NAME);
                        n.push(horizon_client::scoreboard::comp_text(&team.suffix));
                        n
                    }
                    _ => self.target_name(*t),
                };
                names.push(name.with_style(&style));
            }
            let n = waypoints.len() as i32;
            self.feedback(Text::translate(
                "commands.waypoint.list.success",
                &[num(n), Text::literal(dimension), names],
            ));
            return Ok(n);
        }
        let t = self.waypoint_arg(ctx)?;
        let mut icon = self.waypoint_icon(t);
        let style_literal = ctx.literals.iter().any(|l| l == "style");
        let message = if style_literal {
            icon.style = if ctx.literals.iter().any(|l| l == "reset") {
                "minecraft:default".into()
            } else {
                match ctx.get("style") {
                    Some(Arg::Resource(s)) => s.clone(),
                    _ => "minecraft:default".into(),
                }
            };
            Text::translate("commands.waypoint.modify.style", &[])
        } else if ctx.literals.iter().any(|l| l == "reset") {
            icon.color = None;
            Text::translate("commands.waypoint.modify.color.reset", &[])
        } else if ctx.literals.iter().any(|l| l == "hex") {
            let rgb = match ctx.get("color") {
                Some(Arg::Int(v)) => *v,
                _ => 0,
            };
            icon.color = Some(rgb);
            let hex = format!("{:06X}", rgb as u32 & 0xFF_FFFF);
            Text::translate(
                "commands.waypoint.modify.color",
                &[Text::colored(hex, rgb as u32 & 0xFF_FFFF)],
            )
        } else {
            let name = match ctx.get("color") {
                Some(Arg::Str(s)) => s.clone(),
                _ => "white".into(),
            };
            let rgb = text::named_color(&name).unwrap_or(0xFF_FFFF);
            icon.color = Some(rgb as i32);
            Text::translate(
                "commands.waypoint.modify.color",
                &[Text::colored(name, rgb)],
            )
        };
        let t = self.waypoint_target(t);
        self.cmds.waypoint_icons.insert(t, icon);
        // WaypointCommand.mutateIcon: untrack, change, track again.
        self.waypoint_transmitters();
        if let Some(tracked) = self.cmds.waypoints.as_mut() {
            tracked.retain(|x| *x != t);
            tracked.push(t);
        }
        self.feedback(message);
        Ok(0)
    }

    // ---- /dialog --------------------------------------------------------

    pub(super) fn cmd_dialog(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let n = targets.len() as i32;
        if id == "dialog.show" {
            let Some(Arg::Resource(dialog)) = ctx.get("dialog").cloned() else {
                return Ok(0);
            };
            self.show_dialog_to(&targets, &dialog);
        } else {
            self.clear_dialog_to(&targets);
        }
        let (single, multiple) = if id == "dialog.show" {
            (
                "commands.dialog.show.single",
                "commands.dialog.show.multiple",
            )
        } else {
            (
                "commands.dialog.clear.single",
                "commands.dialog.clear.multiple",
            )
        };
        let message = if n == 1 {
            Text::translate(single, &[self.target_name(targets[0])])
        } else {
            Text::translate(multiple, &[num(n)])
        };
        self.feedback(message);
        Ok(n)
    }

    // ---- /posteffect ----------------------------------------------------

    pub(super) fn cmd_posteffect(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "posteffect.list" {
            let t = self.one_entity(ctx, "target", true)?;
            let name = self.target_name(t);
            let effects = self.cmds.post_effects.clone();
            if effects.is_empty() {
                self.feedback(Text::translate("commands.posteffect.list.empty", &[name]));
            } else {
                self.feedback(Text::translate(
                    "commands.posteffect.list.success",
                    &[name, num(effects.len()), Text::literal(effects.join(", "))],
                ));
            }
            return Ok(effects.len() as i32);
        }
        let targets = self.entities(ctx, "targets", true)?;
        let effect = match ctx.get("posteffect") {
            Some(Arg::Resource(s)) => s.clone(),
            _ => String::new(),
        };
        let mut changed: Vec<Target> = Vec::new();
        for t in &targets {
            let done = match id {
                "posteffect.add" => {
                    if self.cmds.post_effects.contains(&effect) {
                        false
                    } else {
                        self.cmds.post_effects.push(effect.clone());
                        true
                    }
                }
                "posteffect.remove" => {
                    let before = self.cmds.post_effects.len();
                    if let Some(i) = self.cmds.post_effects.iter().position(|e| *e == effect) {
                        self.cmds.post_effects.remove(i);
                    }
                    before != self.cmds.post_effects.len()
                }
                _ => !std::mem::take(&mut self.cmds.post_effects).is_empty(),
            };
            if done {
                changed.push(*t);
            }
        }
        let key = id.trim_start_matches("posteffect.");
        if changed.is_empty() {
            return Err(err(&format!("commands.posteffect.{key}.failed"), &[]));
        }
        let n = changed.len() as i32;
        let effect_text = Text::literal(effect);
        let message = match (key, n) {
            ("clear", 1) => Text::translate(
                "commands.posteffect.clear.success.single",
                &[self.target_name(changed[0])],
            ),
            ("clear", _) => {
                Text::translate("commands.posteffect.clear.success.multiple", &[num(n)])
            }
            (_, 1) => Text::translate(
                &format!("commands.posteffect.{key}.success.single"),
                &[effect_text, self.target_name(changed[0])],
            ),
            _ => Text::translate(
                &format!("commands.posteffect.{key}.success.multiple"),
                &[effect_text, num(n)],
            ),
        };
        self.feedback(message);
        Ok(n)
    }

    // ---- /test ----------------------------------------------------------

    /// `TestCommand`. Horizon has no game test instances or test runner: every
    /// finder comes back empty, as in a world without test instance blocks.
    pub(super) fn cmd_test(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            // run <tests>, runmultiple, verify: GameTestRunner.
            "test.run" | "test.verify" | "test.create" => Err(err("command.failed", &[])),
            // runthese/runclosest/runthat/runfailed: TestFinder finds nothing.
            "test.run.found" => {
                if ctx.literals.iter().any(|l| l == "runthat") {
                    self.source_player()?;
                }
                self.feedback(Text::translate("commands.test.no_tests", &[]));
                Ok(0)
            }
            // reset*/clear*: TestCommand.reset throws CLEAR_NO_TESTS too.
            "test.clear" => {
                if ctx
                    .literals
                    .iter()
                    .any(|l| l == "resetthat" || l == "clearthat")
                {
                    self.source_player()?;
                }
                Err(err("commands.test.clear.error.no_tests", &[]))
            }
            "test.stop" => Ok(1),
            "test.locate" => {
                // CommandSourceStack.sendSystemMessage: no feedback rule.
                if !self.command_source.is_some_and(|s| s.silent) {
                    self.system_message(Text::translate("commands.test.locate.started", &[]));
                }
                Err(err("commands.test.error.no_test_instances", &[]))
            }
            _ => {
                // TestCommand.showPos: the block the player looks at (10 blocks).
                self.source_player()?;
                let p = self.test_pick_block();
                Err(err(
                    "commands.test.error.no_test_containing_pos",
                    &[&p[0], &p[1], &p[2]],
                ))
            }
        }
    }
    /// `Entity.pick(10, 1, false)`: the first non-air block on the view ray,
    /// else the block at the ray's end.
    fn test_pick_block(&self) -> [i32; 3] {
        use horizon_core::{
            geometry::{Vec3, traverse_blocks},
            position::BlockPos,
        };
        let Some(p) = self.player.as_ref() else {
            return [0; 3];
        };
        let eye = Vec3::new(
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        );
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        let dir = [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let end = Vec3::new(
            eye.x + dir[0] * 10.0,
            eye.y + dir[1] * 10.0,
            eye.z + dir[2] * 10.0,
        );
        let hit = traverse_blocks(eye, end, 256, |b: BlockPos| {
            let state = self.world.block(b.x, b.y, b.z);
            (!self.res.facts().is_air(state)).then_some([b.x, b.y, b.z])
        });
        hit.ok().flatten().unwrap_or([
            end.x.floor() as i32,
            end.y.floor() as i32,
            end.z.floor() as i32,
        ])
    }

    // ---- /kick ----------------------------------------------------------

    /// `KickCommand.kickPlayers`: only guests of a published world can be
    /// kicked; the owner never.
    pub(super) fn cmd_kick(&mut self, ctx: &Context) -> R<i32> {
        let selector = match ctx.get("targets") {
            Some(Arg::Entity(s)) => Some(s.clone()),
            _ => None,
        };
        // EntityArgument.getPlayers runs before the published check.
        let host = self.entities(ctx, "targets", true).unwrap_or_default();
        let guests: Vec<(u32, String)> = match (&self.lan, &selector) {
            (Some(lan), Some(s)) => lan
                .hub
                .guests
                .iter()
                .filter(|g| lan.hub.singleplayer_owner != Some(g.uuid))
                .filter(|g| match &s.player_name {
                    Some(name) => g.name.eq_ignore_ascii_case(name),
                    None => s.kind == 'a' || s.kind == 'e',
                })
                .map(|g| (g.conn, g.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        if host.is_empty() && guests.is_empty() {
            return Err(err("argument.entity.notfound.player", &[]));
        }
        if self.lan.is_none() || (self.serving() && self.published_port().is_none()) {
            return Err(err("commands.kick.singleplayer.failed", &[]));
        }
        let reason_raw = match ctx.get("reason") {
            Some(Arg::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let reason = match &reason_raw {
            Some(raw) => self.expand_selectors(raw),
            None => Text::translate("multiplayer.disconnect.kicked", &[]),
        };
        let tag = match &reason_raw {
            Some(_) => horizon_world::nbt::Tag::String(reason.plain().as_str().into()),
            None => horizon_world::nbt::Tag::compound([(
                "translate",
                horizon_world::nbt::Tag::String("multiplayer.disconnect.kicked".into()),
            )]),
        };
        let mut count = 0;
        for (conn, name) in guests {
            if let Some(lan) = self.lan.as_mut() {
                lan.hub.kick(conn, tag.clone());
            }
            self.feedback(Text::translate(
                "commands.kick.success",
                &[Text::literal(name), reason.clone()],
            ));
            count += 1;
        }
        if count == 0 {
            return Err(err("commands.kick.owner.failed", &[]));
        }
        Ok(count)
    }

    // ---- /fetchprofile --------------------------------------------------

    /// `FetchProfileCommand`: Horizon resolves no profiles over the network,
    /// so name and id lookups fail like an unreachable profile service; an
    /// entity with a profile (the player) prints its resolved profile.
    pub(super) fn cmd_fetchprofile(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "name") {
            let name = match ctx.get("name") {
                Some(Arg::Str(s)) => s.clone(),
                _ => String::new(),
            };
            // Resolved off-thread; the answer arrives on a later tick.
            self.cmds.pending.push(
                Text::translate("commands.fetchprofile.name.failure", &[Text::literal(name)])
                    .with_color(text::RED),
            );
            return Ok(1);
        }
        if ctx.literals.iter().any(|l| l == "id") {
            let id = match ctx.get("id") {
                Some(Arg::Str(s)) => s.to_ascii_lowercase(),
                _ => String::new(),
            };
            self.cmds.pending.push(
                Text::translate("commands.fetchprofile.id.failure", &[Text::literal(id)])
                    .with_color(text::RED),
            );
            return Ok(1);
        }
        let t = self.one_entity(ctx, "entity", false)?;
        if t != Target::Player {
            return Err(tr_error(
                "commands.fetchprofile.no_profile",
                &[self.target_name(t)],
            ));
        }
        let profile = self.player_profile_snbt();
        let name = super::super::chat::PLAYER_NAME;
        let head = format!("{{type:\"minecraft:player\",player:{profile}}}");
        let item = |key: &str, args: &[Text], run: Option<String>| {
            let inner = Text::translate(key, args).with_color(text::GREEN);
            let mut t = Text::translate("chat.square_brackets", &[inner]);
            if let Some(cmd) = run {
                t = t.with_style(&Style {
                    click: None,
                    ..Style::default()
                });
                let _ = cmd;
            }
            t
        };
        let placeholder = Text::literal("\u{FFFC}").with_color(text::WHITE);
        let mut clickable = item("commands.fetchprofile.copy_component", &[], None);
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.give_item",
            &[],
            Some(format!("give @s minecraft:player_head[profile={profile}]")),
        ));
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.summon_mannequin",
            &[],
            Some(format!(
                "summon minecraft:mannequin ~ ~ ~ {{profile:{profile}}}"
            )),
        ));
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.copy_text",
            &[placeholder],
            None,
        ));
        let _ = head;
        self.feedback(Text::translate(
            "commands.fetchprofile.entity.success",
            &[Text::literal(name), clickable],
        ));
        Ok(1)
    }
    /// ResolvableProfile.CODEC of the player's profile as SNBT.
    fn player_profile_snbt(&self) -> String {
        let [a, b, c, d] = self.level.player_uuid.unwrap_or([0; 4]);
        format!(
            "{{id:[I;{a},{b},{c},{d}],name:\"{}\",properties:[]}}",
            super::super::chat::PLAYER_NAME
        )
    }

    // ---- /publish, /unpublish -------------------------------------------

    /// `PublishCommand.publish` with `IntegratedServer.publishServer`.
    pub(super) fn cmd_publish(&mut self, ctx: &Context) -> R<i32> {
        let allow = matches!(ctx.get("allowCommands"), Some(Arg::Bool(true)));
        let port = match ctx.get("port") {
            Some(Arg::Int(p)) => *p,
            // HttpUtil.getAvailablePort.
            _ => std::net::TcpListener::bind(("0.0.0.0", 0))
                .and_then(|l| l.local_addr())
                .map_or(25564, |a| i32::from(a.port())),
        };
        let published = if self.serving() {
            self.published_port()
        } else {
            self.lan.as_ref().map(|l| l.settings.port)
        };
        if let Some(port) = published {
            return Err(err("commands.publish.alreadyPublished.lan", &[&port]));
        }
        let settings = super::super::lan::LanSettings {
            port: port as u16,
            guest_commands: allow,
            force_game_mode: false,
        };
        let started = if self.serving() {
            self.publish(settings).is_ok()
        } else {
            self.lan_start(settings).is_ok()
        };
        if !started {
            return Err(err("commands.publish.failed", &[]));
        }
        let port_text = port.to_string();
        let link = Text::literal(port_text.clone()).with_style(&Style {
            color: Some(text::GREEN),
            hover: Some(Box::new(Text::translate("chat.copy.click", &[]))),
            ..Style::default()
        });
        self.feedback(Text::translate(
            "commands.publish.started.lan",
            &[Text::translate("chat.square_brackets", &[link])],
        ));
        Ok(port)
    }
    pub(super) fn cmd_unpublish(&mut self) -> R<i32> {
        if self.serving() {
            if !self.unpublish() {
                return Err(err("commands.unpublish.notPublished", &[]));
            }
            self.feedback(Text::translate("commands.unpublish.success", &[]));
            return Ok(1);
        }
        if self.lan.is_none() {
            return Err(err("commands.unpublish.notPublished", &[]));
        }
        self.lan_close(false);
        self.feedback(Text::translate("commands.unpublish.success", &[]));
        Ok(1)
    }

    // ---- /debug, /jfr, /version -----------------------------------------

    pub(super) fn cmd_debug(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "function") {
            return self.cmd_debug_function(ctx);
        }
        // MinecraftServer.isTimeProfilerRunning / startTimeProfiler /
        // stopTimeProfiler: a stop before the delayed start took effect
        // reports empty results and leaves the start pending.
        let running = self.cmds.profiler_delay || self.cmds.profiler.is_some();
        if ctx.literals.iter().any(|l| l == "start") {
            if running {
                return Err(err("commands.debug.alreadyRunning", &[]));
            }
            self.cmds.profiler_delay = true;
            self.feedback(Text::translate("commands.debug.started", &[]));
            return Ok(0);
        }
        if !running {
            return Err(err("commands.debug.notRunning", &[]));
        }
        let (seconds, ticks) = match self.cmds.profiler.take() {
            Some((started, ticks)) => (
                started.elapsed().as_nanos() as f64 / 1.0e9,
                (self.ticks - ticks) as f64,
            ),
            None => (0.0, 0.0),
        };
        let tps = ticks / seconds;
        self.feedback(Text::translate(
            "commands.debug.stopped",
            &[
                Text::literal(two_places(seconds)),
                num(ticks as i64),
                Text::literal(two_places(tps)),
            ],
        ));
        Ok(tps as i32)
    }
    /// `DebugCommand.TraceCustomExecutor`: run the functions with a tracer
    /// writing to `debug/debug-trace-<time>.txt`.
    fn cmd_debug_function(&mut self, ctx: &Context) -> R<i32> {
        if self.exec.function_depth > 0 {
            return Err(err("commands.debug.function.noRecursion", &[]));
        }
        let Some(Arg::Resource(id)) = ctx.get("name").cloned() else {
            return Ok(0);
        };
        let functions = self.functions_named(&id)?;
        let output = format!("debug-trace-{}.txt", filename_date_time());
        let dir = std::env::current_dir().unwrap_or_default().join("debug");
        let mut trace = String::new();
        let mut total = 0;
        let mut traced: Vec<String> = Vec::new();
        for f in &functions {
            let lines = match f.instantiate(None) {
                Ok(lines) => lines,
                Err((key, args)) => {
                    let args: Vec<&dyn std::fmt::Display> =
                        args.iter().map(|s| s as &dyn std::fmt::Display).collect();
                    let reason = Text::tr(&key, &args);
                    self.command_failure(Text::translate(
                        "commands.function.instantiationFailure",
                        &[Text::literal(f.id.clone()), reason],
                    ));
                    continue;
                }
            };
            total += lines.len() as i32;
            traced.push(f.id.clone());
            trace.push_str(&f.id);
            trace.push('\n');
            self.trace_function(&f.id, &lines, 0, &mut trace);
        }
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(&output), trace))
            .is_ok();
        if !written {
            self.command_failure(Text::translate("commands.debug.function.traceFailed", &[]));
        }
        let message = if traced.len() == 1 {
            Text::translate(
                "commands.debug.function.success.single",
                &[
                    num(total),
                    Text::literal(traced[0].clone()),
                    Text::literal(output),
                ],
            )
        } else {
            Text::translate(
                "commands.debug.function.success.multiple",
                &[num(total), num(traced.len()), Text::literal(output)],
            )
        };
        self.feedback(message);
        Ok(total)
    }
    /// The tracer's `[F]`/`[C]` lines for one function, running its commands.
    fn trace_function(&mut self, id: &str, lines: &[String], depth: usize, out: &mut String) {
        let indent = |d: usize| "    ".repeat(d + 1);
        out.push_str(&format!("{}[F] {id} size={}\n", indent(depth), lines.len()));
        let source = super::execute::Source {
            silent: true,
            ..self.player_source()
        };
        for line in lines {
            out.push_str(&format!("{}[C] {line}", indent(depth + 1)));
            match self.execute_line(line, source) {
                Ok(v) => out.push_str(&format!(" -> {v}\n")),
                Err(e) => {
                    out.push('\n');
                    out.push_str(&format!("{}[E] {}", indent(depth + 2), e.message.plain()));
                }
            }
        }
    }

    /// `/jfr start|stop` (JfrCommand). Horizon has no JVM flight recorder:
    /// the recording is its own tick timing, dumped as JSON.
    pub(super) fn cmd_jfr(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "start") {
            if self.cmds.jfr.is_some() {
                return Err(err("commands.jfr.start.failed", &[]));
            }
            let stamp = filename_date_time();
            self.cmds.jfr = Some((Instant::now(), self.ticks, stamp));
            self.feedback(Text::translate("commands.jfr.started", &[]));
            return Ok(1);
        }
        let Some((started, ticks, stamp)) = self.cmds.jfr.take() else {
            return Err(tr_error(
                "commands.jfr.dump.failed",
                &[Text::literal("Not currently profiling")],
            ));
        };
        let relative = format!("debug/client-{stamp}.json");
        let path = std::env::current_dir().unwrap_or_default().join(&relative);
        let report = serde_json::json!({
            "environment": "client",
            "seconds": started.elapsed().as_secs_f64(),
            "ticks": self.ticks - ticks,
        });
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, report.to_string()));
        if let Err(e) = written {
            return Err(tr_error(
                "commands.jfr.dump.failed",
                &[Text::literal(e.to_string())],
            ));
        }
        let file = Text::literal(relative).underlined().with_style(&Style {
            hover: Some(Box::new(Text::translate("chat.copy.click", &[]))),
            ..Style::default()
        });
        self.feedback(Text::translate("commands.jfr.stopped", &[file]));
        Ok(1)
    }

    /// `VersionCommand.dumpVersion`'s lines: Horizon reports the version it
    /// implements.
    pub(in crate::game) fn version_lines(&self) -> Vec<Text> {
        let protocol = horizon_protocol::PROTOCOL_VERSION;
        let mut lines: Vec<Text> = [
            ("commands.version.id", vec![Text::literal(VERSION_ID)]),
            (
                "commands.version.name",
                vec![Text::literal(VANILLA_VERSION)],
            ),
            ("commands.version.data", vec![num(DATA_VERSION)]),
            ("commands.version.series", vec![Text::literal("main")]),
            (
                "commands.version.protocol",
                vec![num(protocol), Text::literal(format!("0x{protocol:x}"))],
            ),
            (
                "commands.version.build_time",
                vec![Text::literal(BUILD_TIME)],
            ),
            (
                "commands.version.pack.resource",
                vec![Text::literal("97.1")],
            ),
            ("commands.version.pack.data", vec![Text::literal("121.0")]),
        ]
        .into_iter()
        .map(|(key, args)| Text::translate(key, &args))
        .collect();
        lines.push(Text::translate(
            if STABLE {
                "commands.version.stable.yes"
            } else {
                "commands.version.stable.no"
            },
            &[],
        ));
        lines
    }
    /// `VersionCommand`.
    pub(super) fn cmd_version(&mut self) -> R<i32> {
        if !self.command_source.is_some_and(|s| s.silent) {
            self.system_message(Text::translate("commands.version.header", &[]));
            for line in self.version_lines() {
                self.system_message(line);
            }
        }
        Ok(1)
    }
}

impl Game<'_> {
    /// `ServerPlayer.openDialog` for every target of `/dialog show`: a guest
    /// is sent `ClientboundShowDialogPacket`, the window's own player opens
    /// the dialog on its screen.
    fn show_dialog_to(&mut self, targets: &[Target], dialog: &str) {
        let Some(holder) = self.dialog_holder(dialog) else {
            return;
        };
        let packet = C::ShowDialog(play::ShowDialog {
            dialog: holder.clone(),
        });
        for &t in targets {
            match t {
                Target::Player if self.lan_acting.is_some() => {
                    self.guest_packets.push(packet.clone());
                }
                Target::Player => self.show_play_dialog(&holder),
                Target::Guest(conn) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
    /// `ServerPlayer.closeDialog` for every target of `/dialog clear`.
    fn clear_dialog_to(&mut self, targets: &[Target]) {
        let packet = C::ClearDialog(Empty);
        for &t in targets {
            match t {
                Target::Player if self.lan_acting.is_some() => {
                    self.guest_packets.push(packet.clone());
                }
                Target::Player => self.clear_dialog(),
                Target::Guest(conn) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
    /// `ServerPlayer.openDialog`'s dialog: an entry of the `minecraft:dialog`
    /// registry, or the dialog an inline `ResourceOrIdArgument` carries.
    fn dialog_holder(&self, dialog: &str) -> Option<DialogHolder> {
        let text = dialog.trim();
        if text.starts_with('{') {
            return horizon_client::command::snbt::parse_fully(text)
                .ok()
                .map(DialogHolder::Direct);
        }
        // A registry id, written with or without its namespace.
        let id = if text.contains(':') {
            text.to_owned()
        } else {
            format!("minecraft:{text}")
        };
        let index = self
            .ui
            .registries
            .ids("minecraft:dialog")
            .iter()
            .position(|name| *name == id);
        match index {
            Some(i) => Some(DialogHolder::Registry(i as i32)),
            None => {
                log::warn!("Horizon: no dialog {id} in the registry");
                None
            }
        }
    }
}
impl Game<'_> {
    /// The player's profile id: the saved id, else the offline id of the
    /// player's name.
    pub(in crate::game) fn player_uuid(&self) -> u128 {
        self.level.player_uuid.map_or_else(
            || horizon_protocol::crypto::offline_uuid(super::super::chat::PLAYER_NAME),
            |[a, b, c, d]| {
                (u128::from(a as u32) << 96)
                    | (u128::from(b as u32) << 64)
                    | (u128::from(c as u32) << 32)
                    | u128::from(d as u32)
            },
        )
    }
}
