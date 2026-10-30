//! Command execution (the `execute` side of each vanilla command class):
//! entity selectors resolved against the game, vanilla feedback and errors.
mod admin;
pub(super) mod compute;
mod data;
mod effects;
pub(super) mod execute;
pub(super) mod extra;
mod items;
mod misc;
mod place;
mod players;
pub(super) mod pregen;
mod raid;
mod terrain_cmds;
mod text_commands;
mod world_cmds;

use super::{Dim, Game, Screen, chat::PLAYER_NAME};
use horizon_client::{
    command::{
        Arg, CommandError, Context, Reader,
        args::{BlockInput, Coordinates, GameMode, ItemInput, Selector, Sort},
        vanilla::{self, DIFFICULTIES},
    },
    gamerules::{self, Value},
    items::ItemStack,
    text::{self, Style, Text},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::registry::BlockStateId;

pub(super) type R<T> = Result<T, CommandError>;

/// An entity a selector found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    /// The player in place: the host's, or the guest whose turn it is.
    Player,
    /// Another player of a hosted world or server: a guest by connection
    /// (game/commands/players.rs).
    Guest(u32),
    Mob(u32),
    Item(u32),
    /// An experience orb, by its entity id.
    Orb(u32),
    /// A projectile, by its entity id.
    Projectile(u32),
    Stand(u32),
    Frame(u32),
    Painting(u32),
    Boat(u32),
    Cart(u32),
    /// An entity of a type Horizon does not simulate (markers, displays,
    /// interactions...): kept as its saved data at a position.
    Other(u32),
    /// An End crystal, by its entity id (`minecraft:end_crystal`).
    Crystal(u32),
    /// The Ender Dragon, by its entity id (`minecraft:ender_dragon`).
    Dragon(u32),
}
/// An entity of a type Horizon does not simulate: its compound lives in the
/// entity memory (`persist.raw`), its position and rotation here.
#[derive(Clone, Debug)]
pub(in crate::game) struct OtherEntity {
    pub id: u32,
    pub kind: String,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}
pub(super) fn err(key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
    CommandError::tr(key, args)
}
fn texts(args: &[Text]) -> Vec<Text> {
    args.to_vec()
}
/// Java's String.valueOf(float/double) for the values commands print.
fn java_float(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1.0e7 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}
/// String.format(Locale.ROOT, "%f").
fn format_double(v: f64) -> String {
    format!("{v:.6}")
}

impl Game<'_> {
    /// Commands.performCommand for a line without its slash.
    pub(super) fn run_command(&mut self, command: &str) -> bool {
        if self.client_mod_command(command) {
            return true;
        }
        let d = vanilla::game_dispatcher();
        let parse = d.parse(command, &self.ui.registries);
        let result = match parse.executable() {
            Err(e) => {
                self.syntax_failure(&e, command);
                return false;
            }
            Ok((ctx, _)) => {
                let ctx = ctx.clone();
                let root = parse.context.clone();
                self.exec = Default::default();
                self.run_chain(&root, &ctx)
            }
        };
        let ok = result.is_ok();
        if let Err(e) = result {
            if e.input.is_some() {
                self.syntax_failure(&e, command);
            } else {
                self.command_failure(e.message);
            }
        }
        // InventoryChangeTrigger fires as slots change, not on the next tick.
        if self.player.is_some() {
            self.check_inventory_trigger();
        }
        ok
    }
    /// `run_command` with its output suppressed
    /// (`CommandSourceStack.withSuppressedOutput`): the player's own source,
    /// rights and checks, no feedback and no failure in the chat.
    pub(super) fn run_command_quietly(&mut self, command: &str) -> bool {
        let quiet = execute::Source {
            silent: true,
            ..self.player_source()
        };
        let outer = self.command_source.replace(quiet);
        let ok = self.run_command(command);
        self.command_source = outer;
        ok
    }
    fn syntax_failure(&mut self, e: &CommandError, command: &str) {
        self.command_failure(e.message.clone());
        let Some(input) = &e.input else { return };
        let cursor = e.cursor.min(input.len());
        let suggest = Style {
            click: Some(text::ClickEvent::SuggestCommand(format!("/{command}"))),
            ..Style::default()
        };
        let mut context = Text::empty();
        if cursor > 10 {
            context.push("...");
        }
        let start = (0..=cursor.saturating_sub(10))
            .rev()
            .find(|&i| input.is_char_boundary(i))
            .unwrap_or(0);
        context.push(&input[start..cursor]);
        if cursor < input.len() {
            context.push(
                Text::literal(&input[cursor..])
                    .with_color(text::RED)
                    .underlined(),
            );
        }
        context.push(
            Text::translate("command.context.here", &[])
                .with_color(text::RED)
                .italic(),
        );
        self.command_failure(context.with_color(text::GRAY).with_style(&suggest));
    }
    /// CommandSourceStack.sendFailure: red text.
    pub(super) fn command_failure(&mut self, message: Text) {
        if self.command_source.is_some_and(|s| s.silent) {
            return;
        }
        let message = Text::empty().append(message).with_color(text::RED);
        if let Some(block) = self.command_source.and_then(|s| s.block) {
            self.command_block_output(block, message);
            return;
        }
        self.system_message(message);
    }
    /// sendSuccess honouring `send_command_feedback`.
    pub(super) fn feedback(&mut self, message: Text) {
        if self.command_source.is_some_and(|s| s.silent) {
            return;
        }
        if let Some(block) = self.command_source.and_then(|s| s.block) {
            self.command_block_success(block, message);
            return;
        }
        if self.ui.rules.bool("send_command_feedback") {
            self.system_message(message.clone());
        }
        self.broadcast_to_admins(message);
    }
    /// CommandSourceStack.broadcastToAdmins (`sendSuccess` with `broadcast`)
    /// for a guest's command, the host's in a world open to LAN and the
    /// dedicated server console's when `broadcast-console-to-ops` is on:
    /// `[name: message]`, gray and italic, for the other operators when
    /// `send_command_feedback`, and for a player's command on a dedicated
    /// server the console when `log_admin_commands`. Queued while the
    /// command runs; `send_admin_feedback` sends it.
    pub(super) fn broadcast_to_admins(&mut self, message: Text) {
        let informs = match self.console_broadcast {
            Some(broadcast) => broadcast,
            // While a guest acts the host's LanHost is out of `self.lan`.
            None => self.lan_acting.is_some() || self.lan.is_some(),
        };
        if !informs
            || message
                .translation_key()
                .is_some_and(|k| admin::QUIET.contains(&k))
        {
            return;
        }
        let ops = self.ui.rules.bool("send_command_feedback");
        // MinecraftServer.sendSystemMessage, unless the server is the source.
        let console = self.console_broadcast.is_none()
            && self.lan_acting.is_some()
            && self.options.dedicated.is_some()
            && self.ui.rules.bool("log_admin_commands");
        if !ops && !console {
            return;
        }
        // getDisplayName: the entity the command runs as (`execute as`), or
        // the server's own source, "Server".
        let who = match self.command_source.and_then(|s| s.entity) {
            Some(entity) => self.target_name(entity),
            None if self.console_broadcast.is_some() => Text::literal("Server".to_owned()),
            None => self.player_display_name(),
        };
        let line = Text::translate("chat.type.admin", &[who, message])
            .with_color(text::GRAY)
            .italic();
        self.admin_feedback.push((line, ops, console));
    }
    /// Player.getDisplayName: the name (team-formatted), suggesting a
    /// `/tell`, showing the player on hover, inserting the name.
    pub(super) fn player_display_name(&self) -> Text {
        let name = self.acting_player_name().to_owned();
        let Some(uuid) = self.lan_acting_uuid else {
            return self.target_name(Target::Player);
        };
        self.target_name(Target::Player).with_component_fields(vec![
            (
                "click_event".into(),
                serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")}),
            ),
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": "minecraft:player",
                    "uuid": horizon_protocol::crypto::uuid_string(uuid), "name": name}),
            ),
            ("insertion".into(), name.into()),
        ])
    }
    fn source_position(&self) -> [f64; 3] {
        if let Some(s) = &self.command_source {
            return s.pos;
        }
        match &self.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.server_position(),
        }
    }
    /// MinecraftServer.createCommandSourceStack's position: the corner of
    /// the respawn block (`Vec3.atLowerCornerOf`).
    pub(super) fn server_position(&self) -> [f64; 3] {
        self.spawn.map_or([0.0, 64.0, 0.0], |s| {
            [s.x.floor(), s.y.floor(), s.z.floor()]
        })
    }
    fn source_rotation(&self) -> [f32; 2] {
        self.command_source
            .map_or([self.yaw, self.pitch], |s| s.rot)
    }
    /// LocalCoordinates use the anchored position; world coordinates the position.
    pub(super) fn resolve_coords(&self, c: &Coordinates) -> [f64; 3] {
        let origin = match (c, &self.command_source) {
            (Coordinates::Local(_), Some(s)) => self.anchor_position(s),
            _ => self.source_position(),
        };
        c.position(origin, self.source_rotation())
    }
    pub(super) fn coords(&self, ctx: &Context, name: &str) -> Option<Coordinates> {
        match ctx.get(name) {
            Some(Arg::Coords(c)) => Some(c.clone()),
            _ => None,
        }
    }
    pub(super) fn position_arg(&self, ctx: &Context, name: &str) -> Option<[f64; 3]> {
        self.coords(ctx, name).map(|c| self.resolve_coords(&c))
    }
    /// BlockPosArgument.getLoadedBlockPos.
    pub(super) fn block_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
        // BlockPos.containing(getPosition)
        let p = self.resolve_coords(&c).map(|v| v.floor() as i32);
        let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
        if dimension != self.dim || !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
            return Err(err("argument.pos.unloaded", &[]));
        }
        if p[1] < horizon_client::chunk::MIN_Y
            || p[1] >= horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT
        {
            return Err(err("argument.pos.outofworld", &[]));
        }
        Ok(p)
    }
    fn int(ctx: &Context, name: &str) -> Option<i32> {
        match ctx.get(name) {
            Some(Arg::Int(v)) => Some(*v),
            _ => None,
        }
    }
    // ---- entities -------------------------------------------------------

    /// The level `t` is in (`Entity.level`): a server's players have their
    /// own, which need not be the one in place; everything else is in the
    /// level in place.
    pub(super) fn target_level(&self, t: Target) -> Dim {
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        conn.and_then(|c| self.command_guest(c))
            .map_or(self.dim, |g| Dim::from_id(&g.dimension))
    }
    pub(super) fn target_position(&self, t: Target) -> Option<[f64; 3]> {
        match t {
            Target::Player => self
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.position),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
            Target::Item(id) => self
                .dropped
                .iter()
                .find(|i| i.id == id)
                .map(|i| [i.position.x, i.position.y, i.position.z]),
            Target::Orb(id) => self
                .orbs
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Frame(id) => self.item_frames.iter().find(|f| f.id == id).map(|f| {
                let (lo, hi) = f.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Painting(id) => self.paintings.iter().find(|p| p.id == id).map(|p| {
                let (lo, hi) = p.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| [b.position.x, b.position.y, b.position.z]),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Other(id) => self
                .others
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Crystal(id) => self
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| [d.position.x, d.position.y, d.position.z]),
        }
    }
    pub(super) fn target_type(&self, t: Target) -> String {
        match t {
            Target::Player | Target::Guest(_) => "minecraft:player".into(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| {
                    horizon_client::command::args::normalize(&m.kind.id)
                }),
            Target::Item(_) => "minecraft:item".into(),
            Target::Orb(_) => "minecraft:experience_orb".into(),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .map_or_else(String::new, |p| p.kind.id().into()),
            Target::Stand(_) => "minecraft:armor_stand".into(),
            Target::Frame(id) => {
                if self.item_frames.iter().any(|f| f.id == id && f.glow) {
                    "minecraft:glow_item_frame".into()
                } else {
                    "minecraft:item_frame".into()
                }
            }
            Target::Painting(_) => "minecraft:painting".into(),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or_else(|| "minecraft:oak_boat".into(), |b| b.entity_type()),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or_else(|| "minecraft:minecart".into(), |c| c.kind.id().into()),
            Target::Other(id) => self
                .others
                .iter()
                .find(|o| o.id == id)
                .map_or_else(String::new, |o| o.kind.clone()),
            Target::Crystal(_) => "minecraft:end_crystal".into(),
            Target::Dragon(_) => "minecraft:ender_dragon".into(),
        }
    }
    /// The name of the player the game acts for: a guest's own while their
    /// command or action runs (`Player.getName`).
    pub(super) fn acting_player_name(&self) -> &str {
        self.lan_acting_name.as_deref().unwrap_or(PLAYER_NAME)
    }
    /// Entity.getDisplayName.
    pub(super) fn target_name(&self, t: Target) -> Text {
        match t {
            Target::Player => {
                let name = self.acting_player_name();
                if let Some(uuid) = self.lan_acting_uuid {
                    return self.player_text(name, uuid);
                }
                match self.score.board.players_team(name) {
                    // PlayerTeam.formatNameForTeam.
                    Some(team) => team.formatted_name(Text::literal(name)),
                    None => Text::literal(name),
                }
            }
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) => self.player_text(&g.name, g.uuid),
                None => Text::empty(),
            },
            Target::Item(id) => {
                let item = self
                    .dropped
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.stack.item.to_string())
                    .unwrap_or_default();
                Text::literal(self.pack.names.display_item(&item))
            }
            Target::Mob(id)
                if self
                    .mobs
                    .iter()
                    .any(|m| m.id == id && m.custom_name.is_some()) =>
            {
                let name = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.custom_name.clone());
                Text::literal(name.unwrap_or_default())
            }
            other => {
                // Entity.getDisplayName: the CustomName component, else the type.
                let custom = match other {
                    Target::Stand(id)
                    | Target::Frame(id)
                    | Target::Painting(id)
                    | Target::Boat(id)
                    | Target::Cart(id)
                    | Target::Other(id) => self.persist_raw(id).and_then(|r| r.get("CustomName")),
                    _ => None,
                };
                if let Some(c) = custom {
                    return horizon_client::scoreboard::comp_text(
                        &horizon_client::scoreboard::tag_to_json(c),
                    );
                }
                let ty = self.target_type(other);
                Text::translate(&format!("entity.{}", ty.replace(':', ".")), &[])
            }
        }
    }
    /// EntitySelector.findEntities.
    pub(super) fn select(&mut self, s: &Selector) -> Vec<Target> {
        if let Some(name) = &s.player_name {
            return self.player_named(name).into_iter().collect();
        }
        if let Some(uuid) = s.uuid {
            return self.target_by_uuid(uuid).into_iter().collect();
        }
        let origin = {
            let p = self.source_position();
            [
                s.pos[0].unwrap_or(p[0]),
                s.pos[1].unwrap_or(p[1]),
                s.pos[2].unwrap_or(p[2]),
            ]
        };
        let mut source_candidates: Vec<Target> = Vec::new();
        match (s.kind, self.command_source) {
            // @s: the executing entity (none for a server source).
            ('s', Some(source)) => source_candidates.extend(source.entity),
            ('s', None) => source_candidates.extend(self.source_entity()),
            // A guest in place is among the players, in join order.
            _ if self.player.is_some() && self.lan_acting.is_none() => {
                source_candidates.push(Target::Player);
            }
            _ => {}
        }
        if s.kind != 's' {
            source_candidates.extend(self.player_candidates(s));
        }
        let source_dim = self.dim;
        // A selector with a spatial constraint reaches the level the command
        // runs in only; an unbounded one reaches every loaded level
        // (EntitySelector.findEntities).
        let dims = if s.world_limited() {
            vec![source_dim]
        } else {
            self.loaded_dims()
        };
        let mut out: Vec<(Target, f64)> = Vec::new();
        for dim in dims {
            let back = self.enter_command_level(dim);
            let mut candidates = Vec::new();
            if dim == source_dim {
                candidates.append(&mut source_candidates);
            }
            if s.kind != 's' {
                candidates.extend(self.level_candidates());
            }
            out.extend(self.select_in_level(candidates, s, origin));
            if let Some(back) = back {
                self.enter_command_level(back);
            }
        }
        match s.sort {
            Sort::Nearest => out.sort_by(|a, b| a.1.total_cmp(&b.1)),
            Sort::Furthest => out.sort_by(|a, b| b.1.total_cmp(&a.1)),
            Sort::Random => {
                for i in (1..out.len()).rev() {
                    let j = self.world.random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                    out.swap(i, j);
                }
            }
            Sort::Arbitrary => {}
        }
        out.into_iter()
            .take(s.limit.max(1))
            .map(|(t, _)| t)
            .collect()
    }
    /// The candidates of the level in place that pass the selector
    /// (EntitySelector.getPredicate).
    fn select_in_level(
        &mut self,
        candidates: Vec<Target>,
        s: &Selector,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let level = self.experience.level;
        let creative = self.creative;
        let passing = self.selector_extra_filter(&candidates, s);
        candidates
            .into_iter()
            .filter_map(|t| {
                let p = self.target_position(t)?;
                let ty = self.target_type(t);
                // type=: every positive must match, no negative may match.
                for (want, neg) in &s.types {
                    let matches = if let Some(tag) = want.strip_prefix('#') {
                        self.entity_tag_matches(tag, &ty)
                    } else {
                        *want == ty
                    };
                    if matches == *neg {
                        return None;
                    }
                }
                if !s.include_entities
                    && !t.is_player()
                    && s.types.iter().all(|(w, n)| *n || w != &ty)
                    && s.kind != 'e'
                    && s.kind != 'n'
                {
                    return None;
                }
                for (name, neg) in &s.names {
                    let n = self.target_name(t).plain();
                    if (n == *name) == *neg {
                        return None;
                    }
                }
                if !passing.contains(&t) {
                    return None;
                }
                let player = match t {
                    Target::Player => Some((level, if creative { "creative" } else { "survival" })),
                    Target::Guest(conn) => self.guest_level_and_mode(conn),
                    _ => None,
                };
                if let Some((lo, hi)) = s.level
                    && player.is_none_or(|(level, _)| {
                        lo.is_some_and(|l| level < l) || hi.is_some_and(|h| level > h)
                    })
                {
                    return None;
                }
                for (mode, neg) in &s.gamemodes {
                    if player.is_none_or(|(_, current)| (current == mode) == *neg) {
                        return None;
                    }
                }
                let d2 = (p[0] - origin[0]).powi(2)
                    + (p[1] - origin[1]).powi(2)
                    + (p[2] - origin[2]).powi(2);
                if let Some((lo, hi)) = s.distance
                    && (lo.is_some_and(|l| d2 < l * l) || hi.is_some_and(|h| d2 > h * h))
                {
                    return None;
                }
                if s.delta.iter().any(Option::is_some) {
                    let d = s.delta.map(|v| v.unwrap_or(0.0));
                    for i in 0..3 {
                        let (a, b) = (
                            origin[i].min(origin[i] + d[i]),
                            origin[i].max(origin[i] + d[i]) + 1.0,
                        );
                        if p[i] < a || p[i] >= b {
                            return None;
                        }
                    }
                }
                Some((t, d2))
            })
            .collect()
    }
    /// The entities of the level in place a selector can find.
    fn level_candidates(&self) -> Vec<Target> {
        use horizon_client::projectiles::ProjectileKind as K;
        let mut candidates = Vec::new();
        candidates.extend(
            self.mobs
                .iter()
                .filter(|m| m.death_time == 0)
                .map(|m| Target::Mob(m.id)),
        );
        candidates.extend(self.dropped.iter().map(|i| Target::Item(i.id)));
        candidates.extend(self.orbs.iter().map(|o| Target::Orb(o.id)));
        candidates.extend(
            self.creatures
                .projectiles
                .iter()
                // The kinds /data can write (entity_nbt).
                .filter(|p| {
                    p.is_arrow() || matches!(p.kind, K::LargeFireball { .. } | K::SmallFireball)
                })
                .map(|p| Target::Projectile(p.id)),
        );
        candidates.extend(self.armor_stands.iter().map(|a| Target::Stand(a.id)));
        candidates.extend(self.item_frames.iter().map(|f| Target::Frame(f.id)));
        candidates.extend(self.paintings.iter().map(|p| Target::Painting(p.id)));
        candidates.extend(self.boats.iter().map(|b| Target::Boat(b.id)));
        candidates.extend(self.carts.iter().map(|c| Target::Cart(c.id)));
        candidates.extend(self.others.iter().map(|o| Target::Other(o.id)));
        // The End crystals and the dragon (EndCrystal and EnderDragon
        // are entities of their own).
        candidates.extend(self.crystals.iter().map(|c| Target::Crystal(c.id)));
        if let Some(d) = &self.dragon {
            candidates.push(Target::Dragon(d.id));
        }
        candidates
    }
    /// The loaded levels, in the order the server holds them: the
    /// overworld, the nether, the end (MinecraftServer.getAllLevels).
    fn loaded_dims(&self) -> Vec<Dim> {
        let mut dims = vec![self.dim];
        dims.extend(self.levels.parked_dims());
        dims.sort_by_key(|d| match d {
            Dim::Overworld => 0,
            Dim::Nether => 1,
            Dim::End => 2,
        });
        dims
    }
    /// Whether the level in place holds the entity a target names.
    fn level_holds(&self, t: Target) -> bool {
        match t {
            // A player is in no level of its own.
            Target::Player | Target::Guest(_) => true,
            Target::Mob(id) => self.mobs.iter().any(|m| m.id == id),
            Target::Item(id) => self.dropped.iter().any(|i| i.id == id),
            Target::Orb(id) => self.orbs.iter().any(|o| o.id == id),
            Target::Projectile(id) => self.creatures.projectiles.iter().any(|p| p.id == id),
            Target::Stand(id) => self.armor_stands.iter().any(|a| a.id == id),
            Target::Frame(id) => self.item_frames.iter().any(|f| f.id == id),
            Target::Painting(id) => self.paintings.iter().any(|p| p.id == id),
            Target::Boat(id) => self.boats.iter().any(|b| b.id == id),
            Target::Cart(id) => self.carts.iter().any(|c| c.id == id),
            Target::Other(id) => self.others.iter().any(|o| o.id == id),
            Target::Crystal(id) => self.crystals.iter().any(|c| c.id == id),
            Target::Dragon(id) => self.dragon.as_ref().is_some_and(|d| d.id == id),
        }
    }
    /// Run `f` with the level that holds the entity a target names in place
    /// (the entity ids are unique across the loaded levels) and put the
    /// level back afterwards. `f` does not run when no level holds the
    /// entity.
    pub(in crate::game) fn in_target_level<R>(
        &mut self,
        t: Target,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if self.level_holds(t) {
            return Some(f(self));
        }
        for dim in self.levels.parked_dims() {
            let back = self.enter_command_level(dim);
            if self.level_holds(t) {
                let r = f(self);
                if let Some(back) = back {
                    self.enter_command_level(back);
                }
                return Some(r);
            }
            if let Some(back) = back {
                self.enter_command_level(back);
            }
        }
        None
    }
    fn entity_tag_matches(&self, tag: &str, ty: &str) -> bool {
        let path = self.options.resources.join(format!(
            "data/minecraft/tags/entity_type/{}.json",
            tag.trim_start_matches("minecraft:")
        ));
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap_or_default())
            .ok()
            .is_some_and(|v| {
                v["values"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|e| e.as_str() == Some(ty)))
            })
    }
    /// EntityArgument.getEntities / getPlayers / getEntity / getPlayer.
    pub(super) fn entities(&mut self, ctx: &Context, name: &str, players: bool) -> R<Vec<Target>> {
        let Some(Arg::Entity(s)) = ctx.get(name).cloned() else {
            return Ok(vec![self.implied_player()]);
        };
        let found: Vec<Target> = self
            .select(&s)
            .into_iter()
            .filter(|t| !players || t.is_player())
            .collect();
        if found.is_empty() {
            return Err(err(
                if players {
                    "argument.entity.notfound.player"
                } else {
                    "argument.entity.notfound.entity"
                },
                &[],
            ));
        }
        Ok(found)
    }
    // ---- dispatch -------------------------------------------------------

    fn execute_command(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "help" => self.cmd_help(ctx),
            "gamemode" => self.cmd_gamemode(ctx),
            "defaultgamemode" => {
                let Some(Arg::GameMode(m)) = ctx.get("gamemode") else {
                    return Ok(0);
                };
                self.set_default_game_mode(match m {
                    horizon_client::command::args::GameMode::Survival => 0,
                    horizon_client::command::args::GameMode::Creative => 1,
                    horizon_client::command::args::GameMode::Adventure => 2,
                    horizon_client::command::args::GameMode::Spectator => 3,
                });
                self.feedback(Text::translate(
                    "commands.defaultgamemode.success",
                    &[Text::translate(&format!("gameMode.{}", m.name()), &[])],
                ));
                Ok(0)
            }
            "give" => self.cmd_give(ctx),
            "teleport.location"
            | "teleport.destination"
            | "teleport.targets.location"
            | "teleport.targets.destination" => self.cmd_teleport(id, ctx),
            id if id.starts_with("time.") => self.cmd_time(id, ctx),
            id if id.starts_with("raid.") => self.cmd_raid(id, ctx),
            "weather" => self.cmd_weather(ctx),
            "kill" => self.cmd_kill(ctx),
            "summon" => self.cmd_summon(ctx),
            "effect.give" => self.cmd_effect_give(ctx),
            "effect.clear" => self.cmd_effect_clear(ctx),
            "enchant" => self.cmd_enchant(ctx),
            "tellraw" => self.cmd_tellraw(ctx),
            "title" => self.cmd_title(ctx),
            "setblock" => self.cmd_setblock(ctx),
            "fill" => self.cmd_fill(ctx),
            "clone" => self.cmd_clone(ctx),
            "experience" => self.cmd_experience(ctx),
            "advancement" => self.cmd_advancement(ctx),
            "gamerule" => self.cmd_gamerule(ctx),
            "difficulty" => self.cmd_difficulty(ctx),
            "seed" => {
                let seed = self.level.seed.to_string();
                let value = Text::copyable(seed);
                self.system_message(Text::translate("commands.seed.success", &[value]));
                Ok(self.level.seed as i32)
            }
            "spawnpoint" => self.cmd_spawnpoint(ctx),
            "setworldspawn" => self.cmd_setworldspawn(ctx),
            "clear" => self.cmd_clear(ctx),
            "item" => self.cmd_item(ctx),
            "item.modify" => self.cmd_item_modify(ctx),
            "spectate" => self.cmd_spectate(ctx),
            id if id.starts_with("waypoint.") => self.cmd_waypoint(id, ctx),
            id if id.starts_with("dialog.") => self.cmd_dialog(id, ctx),
            id if id.starts_with("posteffect.") => self.cmd_posteffect(id, ctx),
            "kick" => self.cmd_kick(ctx),
            "fetchprofile" => self.cmd_fetchprofile(ctx),
            "publish" => self.cmd_publish(ctx),
            "unpublish" => self.cmd_unpublish(),
            "debug" => self.cmd_debug(ctx),
            "jfr" => self.cmd_jfr(ctx),
            "version" => self.cmd_version(),
            id if id.starts_with("test.") => self.cmd_test(id, ctx),
            "compute" => self.cmd_compute(ctx),
            "place" => self.cmd_place(ctx),
            "place.feature" | "place.jigsaw" | "place.structure" => {
                self.cmd_place_generated(id, ctx)
            }
            "recipe.give" | "recipe.take" => self.cmd_recipe(id, ctx),
            "damage" => self.cmd_damage(ctx),
            "rotate" => self.cmd_rotate(ctx),
            "swing" => self.cmd_swing(ctx),
            "playsound" => self.cmd_playsound(ctx),
            "stopsound" => self.cmd_stopsound(ctx),
            "particle" => self.cmd_particle(ctx),
            id if id.starts_with("attribute.") => {
                let result = self.cmd_attribute(id, ctx);
                // AttributeInstance.setDirty: the entity reacts when it ticks.
                if result.is_ok()
                    && matches!(ctx.get("attribute"), Some(Arg::Resource(a)) if a == "minecraft:waypoint_transmit_range")
                    && !id.ends_with("get")
                    && let Ok(t) = self.entities(ctx, "target", false)
                {
                    let target = self.waypoint_target(t[0]);
                    self.cmds.waypoint_dirty.push(target);
                }
                result
            }
            "ride.mount" | "ride.dismount" => self.cmd_ride(id, ctx),
            "spreadplayers" => self.cmd_spreadplayers(ctx),
            id if id.starts_with("forceload.") => self.cmd_forceload(id, ctx),
            id if id.starts_with("pregen.") => self.cmd_pregen(id, ctx),
            "fillbiome" => self.cmd_fillbiome(ctx),
            id if id.starts_with("worldborder.") => self.cmd_worldborder(id, ctx),
            id if id.starts_with("loot.") => self.cmd_loot(id, ctx),
            "teammsg" => self.cmd_teammsg(ctx),
            id if id.starts_with("stopwatch.") => self.cmd_stopwatch(id, ctx),
            id if id.starts_with("tick.") => self.cmd_tick(id, ctx),
            id if id.starts_with("random.") => self.cmd_random(id, ctx),
            "function" => self.cmd_function(ctx),
            "return.value" | "return.fail" => self.cmd_return(id, ctx),
            "schedule.function" | "schedule.clear" => self.cmd_schedule(id, ctx),
            "reload" => self.cmd_reload(),
            id if id.starts_with("datapack.") => self.cmd_datapack(id, ctx),
            id if id.starts_with("bossbar.") => self.cmd_bossbar(id, ctx),
            id if id.starts_with("data.") => self.cmd_data(id, ctx),
            id if id.starts_with("tag.") => self.cmd_tag(id, ctx),
            id if id.starts_with("scoreboard.")
                || id.starts_with("team.")
                || id.starts_with("trigger") =>
            {
                self.cmd_scoreboard(id, ctx)
            }
            "locate.structure" | "locate.biome" => self.cmd_locate(id, ctx),
            "locate.poi" => self.cmd_locate_poi(ctx),
            "warden_spawn_tracker.clear" | "warden_spawn_tracker.set" => {
                let level = Self::int(ctx, "warning_level");
                self.cmd_warden_spawn_tracker(level);
                Ok(1)
            }
            "say" | "me" | "msg" => self.cmd_say(id, ctx),
            "list" => self.cmd_list(ctx),
            _ => Err(err("command.failed", &[])),
        }
    }
    fn cmd_help(&mut self, ctx: &Context) -> R<i32> {
        let d = vanilla::game_dispatcher();
        let level = self.ui.registries.permission;
        match ctx.get("command") {
            None => {
                let usage = d.smart_usage(0, level);
                let n = usage.len() as i32;
                for (_, line) in usage {
                    self.system_message(Text::literal(format!("/{line}")));
                }
                Ok(n)
            }
            Some(Arg::Str(command)) => {
                let parse = d.parse(command, &self.ui.registries);
                if parse.context.nodes.is_empty() {
                    return Err(err("commands.help.failed", &[]));
                }
                let last = parse.context.nodes.last().map(|n| n.0).unwrap_or(0);
                let usage = d.smart_usage(last, level);
                let prefix = &command[..parse.context.range.1.min(command.len())];
                for (_, line) in &usage {
                    self.system_message(Text::literal(format!("/{prefix} {line}")));
                }
                if usage.is_empty() {
                    self.system_message(Text::literal(format!("/{prefix}")));
                }
                Ok(usage.len().max(1) as i32)
            }
            _ => Ok(0),
        }
    }
    pub(super) fn set_game_mode(&mut self, mode: GameMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.set_mode(mode);
        if self.screen == Screen::Inventory && (self.creative || self.spectator()) {
            self.screen = Screen::Playing;
        }
        true
    }
    fn cmd_gamemode(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::GameMode(mode)) = ctx.get("gamemode").cloned() else {
            return Ok(0);
        };
        let targets = if ctx.get("target").is_some() {
            self.entities(ctx, "target", true)?
        } else {
            vec![self.implied_player()]
        };
        let mut count = 0;
        for t in targets {
            let mode_name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            // GameModeCommand.logGamemodeChange: the source's own entity.
            let own = self.source_entity() == Some(t);
            let told = !own && self.ui.rules.bool("send_command_feedback");
            let changed = self.for_player(t, |g| {
                if !g.set_game_mode(mode) {
                    return false;
                }
                if told {
                    g.system_message(Text::translate(
                        "gameMode.changed",
                        std::slice::from_ref(&mode_name),
                    ));
                }
                true
            });
            if changed != Some(true) {
                continue;
            }
            count += 1;
            if own {
                self.feedback(Text::translate(
                    "commands.gamemode.success.self",
                    &[mode_name],
                ));
            } else {
                self.feedback(Text::translate(
                    "commands.gamemode.success.other",
                    &[self.target_name(t), mode_name],
                ));
            }
        }
        Ok(count)
    }
    /// ItemStack.getDisplayName of one `item` with no components.
    pub(super) fn item_display_name(&self, item: &str) -> Text {
        self.stack_display_name(&ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        })
    }
    /// `ItemStack.getDisplayName`: the hover name (custom name in italics,
    /// a written book's title, a potion's effect name, else the item's
    /// translated name) in brackets, in the rarity colour and showing the
    /// stack on hover unless it is empty.
    pub(super) fn stack_display_name(&self, s: &ItemStack) -> Text {
        let item = &*s.item;
        let empty = item == "minecraft:air" || s.count == 0;
        let custom = s.data().and_then(|data| data.custom_name.as_ref());
        let hover = if let Some(c) = custom {
            horizon_client::component::name_text(c)
        } else if let Some(title) = s
            .data()
            .and_then(|d| d.written.as_ref())
            .map(|w| w.title.clone())
            .filter(|t| !t.trim().is_empty())
        {
            Text::literal(title)
        } else if matches!(
            item,
            "minecraft:potion"
                | "minecraft:splash_potion"
                | "minecraft:lingering_potion"
                | "minecraft:tipped_arrow"
        ) && s.potion().is_some()
        {
            Text::translate(
                &horizon_client::brewing::potion_name_key(item, s.potion()),
                &[],
            )
        } else if item == "minecraft:shield"
            && let Some(color) = s.data().and_then(|d| d.base_color.as_deref())
        {
            // `ShieldItem.getName`: the banner's base colour names the shield.
            Text::translate(&format!("item.minecraft.shield.{color}"), &[])
        } else {
            // Item.getDescriptionId: a block item's is the block's.
            let (namespace, path) = item.split_once(':').unwrap_or(("minecraft", item));
            let key = format!("item.{namespace}.{path}");
            let key = if self.pack.names.get(&key).is_some() {
                key
            } else {
                format!("block.{namespace}.{path}")
            };
            Text::translate(&key, &[])
        };
        // Component.empty().append(getHoverName()), italic for a custom name.
        let mut name = Text::empty().append(hover);
        if custom.is_some() {
            name = name.italic();
        }
        let wrapped = Text::translate("chat.square_brackets", &[name]);
        if empty {
            return wrapped;
        }
        let mut rarity = self.item_rarity(item);
        if !s.enchantments().is_empty() {
            rarity = match rarity.as_str() {
                "rare" => "epic".into(),
                "epic" => rarity,
                _ => "rare".into(),
            };
        }
        let color = match rarity.as_str() {
            "uncommon" => text::YELLOW,
            "rare" => text::AQUA,
            "epic" => text::LIGHT_PURPLE,
            _ => text::WHITE,
        };
        // HoverEvent.ShowItem: the count only when it is not one.
        let mut shown = serde_json::json!({"action": "show_item", "id": item});
        if s.count != 1 {
            shown["count"] = s.count.into();
        }
        wrapped
            .with_color(color)
            .with_component_fields(vec![("hover_event".into(), shown)])
    }
    pub(super) fn item_rarity(&self, item: &str) -> String {
        static RARITY: std::sync::OnceLock<std::collections::HashMap<String, String>> =
            std::sync::OnceLock::new();
        let map = RARITY.get_or_init(|| {
            let v: serde_json::Value = serde_json::from_slice(
                &std::fs::read(self.options.resources.join("items.json")).unwrap_or_default(),
            )
            .unwrap_or_default();
            v.as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, c)| {
                            c["minecraft:rarity"]
                                .as_str()
                                .map(|r| (k.clone(), r.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        map.get(item).cloned().unwrap_or_else(|| "common".into())
    }
    fn cmd_give(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(0);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(horizon_client::item_data::command_components)
        {
            Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let count = Self::int(ctx, "count").unwrap_or(1);
        let targets = self.entities(ctx, "targets", true)?;
        let max_stack = self.items().get(&id).map_or(64, |i| i.max_stack) as i32;
        if count > max_stack * 100 {
            return Err(CommandError::new(Text::translate(
                "commands.give.failed.toomanyitems",
                &[
                    Text::literal((max_stack * 100).to_string()),
                    self.item_display_name(&id),
                ],
            )));
        }
        // An air stack is empty: nothing is given (but the message still is).
        let empty = id == "minecraft:air";
        for &t in targets.iter().filter(|_| !empty) {
            // The player's UUID, which an item they could not hold is set to.
            let conn = match t {
                Target::Guest(conn) => Some(conn),
                _ => self.lan_acting,
            };
            let uuid = match conn {
                Some(conn) => self
                    .command_guest(conn)
                    .map(|g| horizon_client::lan::playerdata::uuid_ints(g.uuid)),
                None => Some(self.level.ensure_player_uuid()),
            };
            self.for_player(t, |g| {
                let mut remaining = count;
                while remaining > 0 {
                    let size = remaining.min(max_stack);
                    remaining -= size;
                    let left = g.give(ItemStack {
                        item: id.as_str().into(),
                        count: size as u32,
                        damage,
                        extra: extra.clone(),
                    });
                    // At the player who gets them.
                    let p = g
                        .target_position(Target::Player)
                        .unwrap_or(g.source_position());
                    if let Some(rest) = left {
                        // Inventory full: Player.createItemStackToDrop (thrown from
                        // the eyes), then setNoPickUpDelay and setTarget(player).
                        let eye = g.eye_position().unwrap_or([
                            p[0],
                            p[1] + g.target_eye_height(Target::Player),
                            p[2],
                        ]);
                        g.throw_from_eyes(rest, eye, false);
                        if let Some(item) = g.dropped.last_mut() {
                            item.thrower = None;
                            item.pickup_delay = 0;
                            item.target = uuid;
                        }
                    } else {
                        let r = &mut g.world.random;
                        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
                        g.audio
                            .play("minecraft:entity.item.pickup", Some(p), 0.2, pitch);
                    }
                }
                // containerMenu.broadcastChanges: InventoryChangeTrigger
                // (and its announcement) comes before the command's message.
                if g.player.is_some() {
                    g.check_inventory_trigger();
                }
            });
        }
        // The prototype stack: one item.
        let name = self.stack_display_name(&ItemStack {
            item: id.as_str().into(),
            count: 1,
            damage,
            extra: extra.clone(),
        });
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.give.success.single",
                &[
                    Text::literal(count.to_string()),
                    name,
                    self.target_name(targets[0]),
                ],
            )
        } else {
            Text::translate(
                "commands.give.success.multiple",
                &texts(&[
                    Text::literal(count.to_string()),
                    name,
                    Text::literal(targets.len().to_string()),
                ]),
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn teleport_to(&mut self, t: Target, pos: [f64; 3], rotation: Option<[f32; 2]>) {
        // An entity in another level is teleported in the level that holds it.
        let _ = self.in_target_level(t, |g| g.teleport_to_in_level(t, pos, rotation));
    }
    fn teleport_to_in_level(&mut self, t: Target, pos: [f64; 3], rotation: Option<[f32; 2]>) {
        let v = Vec3::new(pos[0], pos[1], pos[2]);
        match t {
            Target::Guest(_) => {
                self.for_player(t, |g| g.teleport_to(Target::Player, pos, rotation));
            }
            Target::Player => {
                // ServerPlayer.teleport: removeVehicle before placing the player.
                self.stop_riding();
                // A server's player teleported within their level is placed
                // there at once: the arrival they were waiting for elsewhere
                // in it (whose chunks lose their tickets) is abandoned.
                if let Some(conn) = self.lan_acting {
                    self.server_trips.retain(|trip| !trip.is(conn));
                }
                if let Some(p) = self.player.as_mut() {
                    p.position = v;
                    p.velocity = Vec3::new(0.0, 0.0, 0.0);
                    p.fall_distance = 0.0;
                    p.ground.on_ground = false;
                }
                self.previous_position = v;
                if let Some([yaw, pitch]) = rotation {
                    self.yaw = yaw;
                    self.pitch = pitch.clamp(-90.0, 90.0);
                }
            }
            Target::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.position = v;
                    m.previous = v;
                    // Entity.teleportSetPosition: setYRot, setYHeadRot, setXRot
                    // (clamped to -90..90), then setOldRot.
                    if let Some([yaw, pitch]) = rotation {
                        m.yaw = yaw;
                        m.body_yaw = yaw;
                        m.head_yaw = yaw;
                        m.head_pitch = pitch.clamp(-90.0, 90.0);
                        m.previous_yaw = m.yaw;
                        m.previous_head_pitch = m.head_pitch;
                    }
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.dropped.iter_mut().find(|i| i.id == id) {
                    i.position = v;
                    i.previous = v;
                }
            }
            Target::Orb(id) => {
                if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id) {
                    o.position = v;
                    o.previous = v;
                }
            }
            Target::Projectile(id) => {
                if let Some(a) = self.creatures.projectiles.iter_mut().find(|a| a.id == id) {
                    a.position = v;
                    a.previous = v;
                }
            }
            Target::Stand(id) => {
                if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
                    a.position = v;
                    if let Some([yaw, _]) = rotation {
                        a.yaw = yaw;
                    }
                }
            }
            Target::Boat(id) => {
                if let Some(b) = self.boats.iter_mut().find(|b| b.id == id) {
                    b.position = v;
                    b.previous = v;
                    if let Some([yaw, _]) = rotation {
                        b.yaw = yaw;
                    }
                }
            }
            Target::Cart(id) => {
                if let Some(c) = self.carts.iter_mut().find(|c| c.id == id) {
                    c.position = v;
                    c.previous = v;
                    if let Some([yaw, _]) = rotation {
                        c.yaw = yaw;
                    }
                }
            }
            Target::Other(id) => {
                if let Some(o) = self.others.iter_mut().find(|o| o.id == id) {
                    o.position = v;
                    if let Some([yaw, pitch]) = rotation {
                        o.yaw = yaw;
                        o.pitch = pitch;
                    }
                }
            }
            Target::Crystal(id) => {
                if let Some(c) = self.crystals.iter_mut().find(|c| c.id == id) {
                    c.position = v;
                }
            }
            Target::Dragon(id) => {
                if let Some(d) = self.dragon.as_mut().filter(|d| d.id == id) {
                    d.position = v;
                    d.previous = v;
                }
            }
            // Wall-mounted entities do not move.
            Target::Frame(_) | Target::Painting(_) => {}
        }
    }
    /// Teleport the local player into the command source's dimension. Unlike
    /// portal travel, `/tp` keeps the coordinates given by the command.
    fn teleport_player_to_dimension(
        &mut self,
        dimension: Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> R<()> {
        let Some(mut player) = self.player.take() else {
            return Ok(());
        };
        let from = self.dim;
        if self.switch_level(dimension, player.position).is_err() {
            self.player = Some(player);
            return Err(err("command.failed", &[]));
        }
        let at = player.position;
        self.trigger_dimension(from, dimension, [at.x, at.y, at.z]);
        let target = Vec3::new(pos[0], pos[1], pos[2]);
        player.position = target;
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
        player.ground.on_ground = false;
        if let Some([yaw, pitch]) = rotation {
            let pitch = pitch.clamp(-90.0, 90.0);
            self.yaw = yaw;
            self.pitch = pitch;
            player.yaw = yaw;
            player.head_yaw = yaw;
            player.pitch = pitch;
        }
        self.travelling = Some((player, target));
        self.portals.travel = None;
        self.end_travel = false;
        self.end_portal_travel = false;
        Ok(())
    }

    /// A server's player teleported into another level than theirs
    /// (ServerPlayer.teleport): queued to move after the command. False
    /// when `t` is not a player in another level.
    fn teleport_across(
        &mut self,
        t: Target,
        level: super::Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        let at = self.target_level(t);
        let Some(conn) = conn.filter(|_| at != level && self.options.dedicated.is_some()) else {
            return false;
        };
        self.level_teleports
            .push(super::server_travel::LevelTeleport {
                conn,
                dim: level,
                at: pos,
                rotation,
            });
        true
    }
    fn cmd_teleport(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = if id.starts_with("teleport.targets") {
            self.entities(ctx, "targets", false)?
        } else {
            vec![self.implied_player()]
        };
        if id.ends_with("destination") {
            let dest = self.entities(ctx, "destination", false)?[0];
            let pos = self.target_position(dest).unwrap_or(self.source_position());
            // Into the destination's level.
            let level = self.target_level(dest);
            // TeleportCommand.teleportToEntity: the destination's rotation too.
            let rotation = Some(self.target_rotation(dest));
            for &t in &targets {
                if !self.teleport_across(t, level, pos, rotation) {
                    self.teleport_to(t, pos, rotation);
                }
            }
            let message = if targets.len() == 1 {
                Text::translate(
                    "commands.teleport.success.entity.single",
                    &[self.target_name(targets[0]), self.target_name(dest)],
                )
            } else {
                Text::translate(
                    "commands.teleport.success.entity.multiple",
                    &[
                        Text::literal(targets.len().to_string()),
                        self.target_name(dest),
                    ],
                )
            };
            self.feedback(message);
            return Ok(targets.len() as i32);
        }
        let pos = self
            .position_arg(ctx, "location")
            .unwrap_or(self.source_position());
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
        {
            return Err(err("commands.teleport.invalidPosition", &[]));
        }
        let rotation = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => Some(c.rotation(self.source_rotation())),
            _ => None,
        };
        // facing: look from the new position at a location or entity.
        let facing = self.position_arg(ctx, "facingLocation").or_else(|| {
            let Some(Arg::Entity(_)) = ctx.get("facingEntity") else {
                return None;
            };
            None
        });
        let facing = match (facing, ctx.get("facingEntity")) {
            (Some(f), _) => Some(f),
            (None, Some(_)) => {
                let e = self.entities(ctx, "facingEntity", false)?[0];
                // EntityAnchorArgument.Anchor.apply: the feet, or the eyes.
                let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
                let eye_height = if eyes { self.target_eye_height(e) } else { 0.0 };
                self.target_position(e)
                    .map(|p| [p[0], p[1] + eye_height, p[2]])
            }
            _ => None,
        };
        // Entity.lookAt: from the source's anchor (the feet, unless `anchored
        // eyes`) on the teleported entity, so the eyes depend on the target.
        let anchored_eyes = self.command_source.is_some_and(|s| s.eyes);
        let look_at = |this: &Self, t: Target| {
            let f = facing?;
            let eye = if anchored_eyes {
                pos[1] + this.target_eye_height(t)
            } else {
                pos[1]
            };
            let (dx, dy, dz) = (f[0] - pos[0], f[1] - eye, f[2] - pos[2]);
            let horizontal = (dx * dx + dz * dz).sqrt();
            Some([
                ((dz.atan2(dx).to_degrees()) - 90.0) as f32,
                (-(dy.atan2(horizontal).to_degrees())) as f32,
            ])
        };
        // A server's player in another level than the source's (`execute
        // in`, the console's Overworld) moves there (ServerPlayer.teleport
        // into another level), after the command.
        let source_level = match self.command_source.and_then(|s| s.level) {
            Some(level) => level,
            None if self.source_entity().is_none() => super::Dim::Overworld,
            None => self.dim,
        };
        for &t in &targets {
            let rotation = look_at(self, t).or(rotation);
            if self.teleport_across(t, source_level, pos, rotation) {
                continue;
            }
            let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
            if t == Target::Player && dimension != self.dim {
                self.teleport_player_to_dimension(dimension, pos, rotation)?;
            } else {
                self.teleport_to(t, pos, rotation);
            }
        }
        let [x, y, z] = pos.map(|v| Text::literal(format_double(v)));
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.teleport.success.location.single",
                &[self.target_name(targets[0]), x, y, z],
            )
        } else {
            Text::translate(
                "commands.teleport.success.location.multiple",
                &[Text::literal(targets.len().to_string()), x, y, z],
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn cmd_time(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "time.query.gametime" {
            let t = self.world.game_time;
            self.system_message(Text::tr("commands.time.query.gametime", &[&t]));
            return Ok((t % i64::from(i32::MAX)) as i32);
        }
        // `time of <clock>` names the clock; otherwise the dimension's default clock.
        let clock = match ctx.get("clock") {
            Some(Arg::Resource(c)) => c.clone(),
            _ => {
                if self.dim == Dim::Nether {
                    return Err(err(
                        "commands.time.no_default_clock",
                        &[&"minecraft:the_nether"],
                    ));
                }
                "minecraft:overworld".to_owned()
            }
        };
        if clock != "minecraft:overworld" {
            // Only the overworld clock runs in Horizon (no End yet).
            return Err(err("commands.time.no_default_clock", &[&clock]));
        }
        let total = self.world.day_time;
        match id {
            "time.set" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                if total == t {
                    return Err(err("commands.time.set.already_at_time", &[&clock, &t]));
                }
                self.world.day_time = t;
                self.ui.clock.partial = 0.0;
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &t]));
                Ok(t as i32)
            }
            "time.set.marker" => {
                let Some(Arg::Resource(marker)) = ctx.get("timemarker").cloned() else {
                    return Ok(0);
                };
                let Some(m) = self.ui.markers.get(&marker).copied() else {
                    return Err(err(
                        "commands.time.no_time_marker_found",
                        &[&marker, &clock],
                    ));
                };
                if m.occurs_at(total) {
                    return Err(err(
                        "commands.time.set.already_at_time_marker",
                        &[&clock, &marker],
                    ));
                }
                self.world.day_time = m.resolve(total);
                self.ui.clock.partial = 0.0;
                self.feedback(Text::tr(
                    "commands.time.set.time_marker",
                    &[&clock, &marker],
                ));
                Ok((self.world.day_time % i64::from(i32::MAX)) as i32)
            }
            "time.add" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                self.world.day_time = (total + t).max(0);
                let now = self.world.day_time;
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &now]));
                Ok((now % i64::from(i32::MAX)) as i32)
            }
            "time.pause" | "time.resume" => {
                let pause = id == "time.pause";
                if self.ui.clock.paused == pause {
                    return Err(err(
                        if pause {
                            "commands.time.pause.already_paused"
                        } else {
                            "commands.time.pause.already_running"
                        },
                        &[&clock],
                    ));
                }
                self.ui.clock.paused = pause;
                self.feedback(Text::tr(
                    if pause {
                        "commands.time.pause"
                    } else {
                        "commands.time.resume"
                    },
                    &[&clock],
                ));
                Ok(1)
            }
            "time.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(r)) => *r,
                    _ => 1.0,
                };
                if self.ui.clock.rate == rate {
                    return Err(err(
                        "commands.time.rate.already_same",
                        &[&clock, &java_float(f64::from(rate))],
                    ));
                }
                self.ui.clock.rate = rate;
                self.feedback(Text::tr(
                    "commands.time.rate",
                    &[&clock, &java_float(f64::from(rate))],
                ));
                Ok(1)
            }
            "time.query.time" => {
                self.system_message(Text::tr("commands.time.query.absolute", &[&clock, &total]));
                Ok((total % i64::from(i32::MAX)) as i32)
            }
            "time.query.timeline" | "time.query.repetition" => {
                let Some(Arg::Resource(timeline)) = ctx.get("timeline").cloned() else {
                    return Ok(0);
                };
                let Some(period) = self.ui.timelines.get(&timeline).copied() else {
                    return Err(err(
                        "commands.time.wrong_timeline_for_clock",
                        &[&clock, &timeline],
                    ));
                };
                if id == "time.query.timeline" {
                    let ticks = period.map_or(total, |p| total.rem_euclid(p));
                    self.system_message(Text::tr(
                        "commands.time.query.timeline",
                        &[&timeline, &ticks],
                    ));
                    Ok(ticks as i32)
                } else {
                    let reps = period.map_or(0, |p| total / p);
                    self.system_message(Text::tr(
                        "commands.time.query.timeline.repetitions",
                        &[&timeline, &reps],
                    ));
                    Ok(reps as i32)
                }
            }
            _ => Ok(0),
        }
    }
    fn cmd_weather(&mut self, ctx: &Context) -> R<i32> {
        let duration = Self::int(ctx, "duration");
        let sample = |min: i32, max: i32, r: &mut horizon_core::random::LegacyRandom| {
            min + r.next_int_bound(max - min + 1).unwrap_or(0)
        };
        let w = &mut self.world.weather;
        let r = &mut self.world.random;
        let (key, clear, time, raining, thundering) = if ctx.has("clear") {
            (
                "commands.weather.set.clear",
                duration.unwrap_or_else(|| sample(12_000, 180_000, r)),
                0,
                false,
                false,
            )
        } else if ctx.has("rain") {
            (
                "commands.weather.set.rain",
                0,
                duration.unwrap_or_else(|| sample(12_000, 24_000, r)),
                true,
                false,
            )
        } else {
            (
                "commands.weather.set.thunder",
                0,
                duration.unwrap_or_else(|| sample(3_600, 15_600, r)),
                true,
                true,
            )
        };
        // ServerLevel.setWeatherParameters.
        w.clear_time = clear;
        w.rain_time = time;
        w.thunder_time = time;
        w.raining = raining;
        w.thundering = thundering;
        self.feedback(Text::translate(key, &[]));
        Ok(duration.unwrap_or(-1))
    }
    fn kill_target(&mut self, t: Target) {
        // An entity in another level is killed in the level that holds it.
        let _ = self.in_target_level(t, |g| g.kill_target_in_level(t));
    }
    fn kill_target_in_level(&mut self, t: Target) {
        match t {
            Target::Guest(_) => {
                self.for_player(t, |g| g.kill_target(Target::Player));
            }
            Target::Player => {
                if let Some(p) = self.player.as_mut() {
                    let _ = p.set_health(0.0, false);
                }
                self.ui.death_cause = Some("death.attack.genericKill".into());
            }
            Target::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.health = 0.0;
                    if m.death_time == 0 {
                        m.death_time = 1;
                    }
                }
            }
            Target::Item(id) => self.dropped.retain(|i| i.id != id),
            Target::Orb(id) => {
                if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id) {
                    o.age = u32::MAX / 2;
                }
            }
            Target::Projectile(id) => {
                self.creatures.projectiles.retain(|p| p.id != id);
            }
            Target::Stand(id) => self.armor_stands.retain(|a| a.id != id),
            Target::Frame(id) => self.item_frames.retain(|f| f.id != id),
            Target::Painting(id) => self.paintings.retain(|p| p.id != id),
            Target::Boat(id) => self.boats.retain(|b| b.id != id),
            Target::Cart(id) => self.carts.retain(|c| c.id != id),
            Target::Other(id) => self.others.retain(|o| o.id != id),
            Target::Crystal(id) => self.kill_crystal(id),
            Target::Dragon(id) => self.kill_dragon(id),
        }
        // The orbs marked for removal are gone (Entity.kill removes them).
        self.orbs.retain(|o| o.age < u32::MAX / 2);
    }
    fn cmd_kill(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", false)?
        } else {
            vec![self.implied_player()]
        };
        let mut names: Vec<Text> = Vec::new();
        for &t in &targets {
            // The name of an entity in another level is read there.
            let name = match self.in_target_level(t, |g| g.target_name(t)) {
                Some(name) => name,
                None => self.target_name(t),
            };
            names.push(name);
        }
        for &t in &targets {
            self.kill_target(t);
        }
        let message = if targets.len() == 1 {
            Text::translate("commands.kill.success.single", &[names[0].clone()])
        } else {
            Text::tr("commands.kill.success.multiple", &[&targets.len()])
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn cmd_summon(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(kind)) = ctx.get("entity").cloned() else {
            return Ok(0);
        };
        // ResourceArgument.getSummonableEntityType.
        if matches!(
            kind.as_str(),
            "minecraft:player" | "minecraft:fishing_bobber"
        ) {
            return Err(err("entity.not_summonable", &[&kind]));
        }
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or(self.source_position());
        let block_y = pos[1].floor() as i32;
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-20_000_000..20_000_000).contains(&block_y)
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
        {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        let short = kind.trim_start_matches("minecraft:").to_owned();
        if let Some(Arg::Str(nbt)) = ctx.get("nbt").cloned() {
            // With an NBT argument the entity is built from it, unfinalized.
            if self.ui.difficulty == 0
                && self
                    .res
                    .mob_types()
                    .get(&format!("minecraft:{short}"))
                    .is_some_and(|t| t.category == "monster")
            {
                return Err(err("commands.summon.failed.peaceful", &[]));
            }
            let before = self.spawned_mark();
            let name = match self.summon_with_nbt(&format!("minecraft:{short}"), pos, &nbt)? {
                Some(target) => self.target_name(target),
                // Spawned as a kind no command target stands for (primed
                // TNT, falling blocks, projectiles): its type's name.
                None if self.spawned_mark() != before => {
                    Text::translate(&format!("entity.minecraft.{short}"), &[])
                }
                None => return Err(err("commands.summon.failed", &[])),
            };
            self.feedback(Text::translate("commands.summon.success", &[name]));
            return Ok(1);
        }
        let name;
        if let Some(t) = self
            .res
            .mob_types()
            .get(&format!("minecraft:{short}"))
            .cloned()
        {
            if self.ui.difficulty == 0 && t.category == "monster" {
                return Err(err("commands.summon.failed.peaceful", &[]));
            }
            self.spawn_mob_spawned(&short, pos);
            name = Text::translate(&format!("entity.minecraft.{short}"), &[]);
        } else {
            match short.as_str() {
                "item" => {
                    // An item entity needs an item; vanilla spawns an empty one that despawns.
                    return Err(err("commands.summon.failed", &[]));
                }
                "experience_orb" => {
                    self.next_entity += 1;
                    self.orbs.push(horizon_client::entities::ExperienceOrb {
                        id: self.next_entity,
                        position: Vec3::new(pos[0], pos[1], pos[2]),
                        previous: Vec3::new(pos[0], pos[1], pos[2]),
                        velocity: Vec3::new(0.0, 0.0, 0.0),
                        value: 0,
                        age: 0,
                        on_ground: false,
                        count: 1,
                        ticks: 0,
                    });
                    name = Text::translate("entity.minecraft.experience_orb", &[]);
                }
                "lightning_bolt" => {
                    self.spawn_bolt(pos, false);
                    name = Text::translate("entity.minecraft.lightning_bolt", &[]);
                }
                "end_crystal" => {
                    // EndCrystal's constructor takes an id from the entity
                    // counter, and the crystal is shown with its base.
                    self.next_entity += 1;
                    self.crystals.push(super::dragon::Crystal {
                        id: self.next_entity,
                        position: Vec3::new(pos[0], pos[1], pos[2]),
                        show_bottom: true,
                        time: self.world.random.next_int_bound(100_000).unwrap_or(0),
                        beam: None,
                        invulnerable: false,
                    });
                    name = Text::translate("entity.minecraft.end_crystal", &[]);
                }
                k if super::minecarts::CartKind::from_id(&kind).is_some() => {
                    let cart_kind = super::minecarts::CartKind::from_id(&kind)
                        .unwrap_or(super::minecarts::CartKind::Rideable);
                    self.next_entity += 1;
                    let cart = super::minecarts::Minecart::new(
                        self.next_entity,
                        cart_kind,
                        Vec3::new(pos[0], pos[1], pos[2]),
                    );
                    self.carts.push(cart);
                    name = Text::translate(&format!("entity.minecraft.{k}"), &[]);
                }
                // SummonCommand.createEntity: any other type from an empty compound.
                _ => {
                    let before = self.spawned_mark();
                    name = match self.summon_with_nbt(&kind, pos, "{}")? {
                        Some(target) => self.target_name(target),
                        // Spawned as a kind no command target stands for
                        // (primed TNT, falling blocks, projectiles).
                        None if self.spawned_mark() != before => {
                            Text::translate(&format!("entity.minecraft.{short}"), &[])
                        }
                        None => return Err(err("commands.summon.failed", &[])),
                    };
                }
            }
        }
        self.feedback(Text::translate("commands.summon.success", &[name]));
        Ok(1)
    }
    /// What changes when an entity spawns, kinds that take their id only
    /// when first tracked (primed TNT, falling blocks, projectiles)
    /// included: a summon compares it before and after.
    fn spawned_mark(&self) -> (u32, usize) {
        (
            self.next_entity,
            self.tnts.len()
                + self.falling.len()
                + self.creatures.projectiles.len()
                + self.rockets.len(),
        )
    }
    /// BlockStateArgument -> registry state.
    pub(super) fn block_state(&self, b: &BlockInput) -> R<BlockStateId> {
        let state = horizon_world::chunk::BlockState {
            name: b.id.clone(),
            properties: b.properties.iter().cloned().collect(),
        };
        self.pack
            .assets
            .registry
            .resolve(&state)
            .map_err(|_| err("argument.block.id.invalid", &[&b.id]))
    }
    /// BlockPredicateArgument test.
    fn block_matches(&self, b: &BlockInput, state: BlockStateId) -> bool {
        let Some(s) = self.pack.assets.registry.state(state) else {
            return false;
        };
        let name_ok = if b.tag {
            let tag = b.id.trim_start_matches("minecraft:");
            self.res
                .items()
                .block_tags
                .get(tag)
                .or_else(|| self.items().block_tags.get(&b.id))
                .is_some_and(|set| {
                    set.contains(&s.name) || set.contains(s.name.trim_start_matches("minecraft:"))
                })
        } else {
            s.name == b.id
        };
        name_ok
            && b.properties
                .iter()
                .all(|(k, v)| s.properties.get(k) == Some(v))
    }
    /// Place a state like BlockInput.place: false when nothing changed.
    pub(super) fn place_state(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        strict: bool,
    ) -> R<bool> {
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if previous == state {
            return Ok(false);
        }
        // A different block replaces the block entity (Level.setBlock).
        let (old, new) = (
            self.res.facts().get(previous).name.clone(),
            self.res.facts().get(state).name.clone(),
        );
        // The command clears the old block entity without its side effects
        // (UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS); the new block gets a fresh one.
        let key = (pos[0], pos[1], pos[2]);
        if old != new {
            self.jukebox_removed(pos);
            self.world.block_entities.remove(&key);
        }
        if strict {
            self.with_level(|b, level, ticks| {
                b.set_block(
                    level,
                    ticks,
                    horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    horizon_world::level::flags::UPDATE_CLIENTS
                        | horizon_world::level::flags::UPDATE_SKIP_ALL_SIDEEFFECTS,
                );
            });
            self.process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else {
            self.level_set(pos, state)
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        // LevelChunk.setBlockState creates it when the block, not the state, is still
        // there after onPlace: a powered hopper turns itself `enabled=false`.
        let landed = self.world.block(pos[0], pos[1], pos[2]);
        if old != new
            && self.res.facts().get(landed).name == new
            && let Some(entity) = horizon_client::containers::BlockEntity::for_block(&new)
        {
            self.world.block_entities.insert(key, entity);
        }
        Ok(true)
    }
    fn cmd_setblock(&mut self, ctx: &Context) -> R<i32> {
        let pos = self.block_pos_arg(ctx, "pos")?;
        let Some(Arg::Block(b)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let state = self.block_state(&b)?;
        let current = self.world.block(pos[0], pos[1], pos[2]);
        if ctx.has("keep") && current != self.world.air {
            return Err(err("commands.setblock.failed", &[]));
        }
        if ctx.has("destroy") && current != self.world.air {
            self.destroy_block_dropping(pos)?;
        }
        let placed = self.place_state(pos, state, ctx.has("strict"))?;
        let loaded = match &b.nbt {
            Some(nbt) => self.load_block_entity_nbt(pos, nbt),
            None => false,
        };
        if !placed && !loaded {
            return Err(err("commands.setblock.failed", &[]));
        }
        self.feedback(Text::tr(
            "commands.setblock.success",
            &[&pos[0], &pos[1], &pos[2]],
        ));
        Ok(1)
    }
    /// `Level.destroyBlock(pos, true)` through the behaviour library: the
    /// destroy event, the block's drops without a tool or breaker, then its
    /// fluid's block, the block entity spilling its contents on the way.
    pub(super) fn destroy_block_dropping(&mut self, pos: [i32; 3]) -> R<()> {
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.destroy_block(level, ticks, at, true));
        self.process_level_events()
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))
    }
    /// The block entity part of `BlockInput.place`: load the block entity
    /// from the argument's tag; true when its saved data changed.
    pub(super) fn load_block_entity_nbt(&mut self, pos: [i32; 3], nbt: &str) -> bool {
        use horizon_world::nbt::Tag;
        let key = (pos[0], pos[1], pos[2]);
        let Some(e) = self.world.block_entities.get(&key) else {
            return false;
        };
        let name = self.data_block_name(pos);
        let before = horizon_client::save::encode_block_entity(key, e, &name);
        let Ok((mut tag, _)) = horizon_world::nbt::parse_snbt(nbt) else {
            return false;
        };
        if let Tag::Compound(c) = &mut tag {
            for k in ["id", "x", "y", "z"] {
                if let Some(v) = before.get(k) {
                    c.insert(k.into(), v.clone());
                }
            }
        }
        let Some((_, mut new)) = horizon_client::save::decode_block_entity(&tag) else {
            return false;
        };
        let rolled = self.take_loot_table(key, &tag, &mut new);
        let after = horizon_client::save::encode_block_entity(key, &new, &name);
        if after == before && !rolled {
            return false;
        }
        self.world.block_entities.insert(key, new);
        self.update_spawner_display(key);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        true
    }
    /// `RandomizableContainer.unpackLootTable` for a player who opens a
    /// container: a roll the container still holds fires
    /// `player_generates_container_loot` for its table, and from then on the
    /// container saves its items, not the table.
    pub(super) fn unpack_opened_loot(&mut self, key: (i32, i32, i32), name: &str) {
        let Some(rolled) = self
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .and_then(|s| s.rolled_loot.iter().find(|r| r.pos == key))
            .cloned()
        else {
            return;
        };
        // The roll is still there only while the container holds exactly its items.
        let still_rolled = self
            .world
            .block_entities
            .get_mut(&key)
            .is_some_and(|entity| {
                let mut tag = horizon_client::save::encode_block_entity(key, entity, name);
                match &mut tag {
                    horizon_world::nbt::Tag::Compound(map) => rolled.restore(map),
                    _ => false,
                }
            });
        if still_rolled {
            self.trigger_loot_table(&rolled.table);
        }
        let Some(chunk) = self.world.chunks.get_mut(&(key.0 >> 4, key.2 >> 4)) else {
            return;
        };
        let chunk = std::sync::Arc::make_mut(chunk);
        let mut extras = chunk
            .saved
            .take()
            .map(std::sync::Arc::unwrap_or_clone)
            .unwrap_or_default();
        extras.rolled_loot.retain(|r| r.pos != key);
        chunk.saved = Some(std::sync::Arc::new(extras));
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
    /// RandomizableContainer.tryLoadLootTable for a tag a command gives a
    /// container (`setblock`, `data`): an empty container with a `LootTable`
    /// is rolled now, as a loaded chunk's are (`save::roll_loaded_loot`), and
    /// the roll is kept with its chunk, so that the container saves, and
    /// `data get` shows, its table and seed until its items change. A tag
    /// without one drops what the container had. True when it had a table.
    pub(super) fn take_loot_table(
        &mut self,
        key: (i32, i32, i32),
        tag: &horizon_world::nbt::Tag,
        entity: &mut horizon_client::containers::BlockEntity,
    ) -> bool {
        use horizon_client::{containers::BlockEntity, save, structure_blocks};
        let table = save::text(tag.get("LootTable"));
        let rolled = match (table, entity) {
            (Some(table), BlockEntity::Chest(slots)) if slots.iter().all(Option::is_none) => {
                let loot = structure_blocks::loot_data(&self.options.resources, &self.pack_files());
                let at = Vec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
                let biome = self.biome_name_at(at).unwrap_or_default();
                Some(structure_blocks::roll_container(
                    slots,
                    loot.as_deref(),
                    table,
                    structure_blocks::loot_seed(tag),
                    key,
                    &biome,
                ))
            }
            _ => None,
        };
        let Some(chunk) = self.world.chunks.get_mut(&(key.0 >> 4, key.2 >> 4)) else {
            return false;
        };
        let held = |c: &horizon_client::chunk::Chunk| {
            c.saved
                .as_ref()
                .is_some_and(|s| s.rolled_loot.iter().any(|r| r.pos == key))
        };
        if rolled.is_none() && !held(chunk) {
            return false;
        }
        let chunk = std::sync::Arc::make_mut(chunk);
        let mut extras = chunk
            .saved
            .take()
            .map(std::sync::Arc::unwrap_or_clone)
            .unwrap_or_default();
        extras.rolled_loot.retain(|r| r.pos != key);
        let taken = rolled.is_some();
        extras.rolled_loot.extend(rolled);
        chunk.saved = Some(std::sync::Arc::new(extras));
        taken
    }
    /// The loot table and seed a container still holding its roll saves
    /// (`RolledLoot::restore`), in place of its items.
    pub(super) fn restore_loot_table(
        &self,
        key: (i32, i32, i32),
        tag: &mut horizon_world::nbt::Tag,
    ) {
        let horizon_world::nbt::Tag::Compound(map) = tag else {
            return;
        };
        let rolled = self
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .and_then(|s| s.rolled_loot.iter().find(|r| r.pos == key));
        if let Some(r) = rolled {
            r.restore(map);
        }
    }
    fn cmd_fill(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "from")?;
        let b = self.block_pos_arg(ctx, "to")?;
        let Some(Arg::Block(block)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let area = (0..3)
            .map(|i| i64::from(max[i] - min[i] + 1))
            .product::<i64>();
        let limit = self.ui.rules.int("max_block_modifications");
        if area > i64::from(limit) {
            return Err(err("commands.fill.toobig", &[&limit, &area]));
        }
        let state = self.block_state(&block)?;
        let air = self.world.air;
        let (hollow, outline, destroy, strict, keep) = (
            ctx.has("hollow"),
            ctx.has("outline"),
            ctx.has("destroy"),
            ctx.has("strict"),
            ctx.has("keep"),
        );
        let mut count = 0;
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let current = self.world.block(x, y, z);
                    if keep && current != air {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, current)
                    {
                        continue;
                    }
                    let edge = x == min[0]
                        || x == max[0]
                        || y == min[1]
                        || y == max[1]
                        || z == min[2]
                        || z == max[2];
                    let mut affected = false;
                    if destroy && current != air {
                        self.destroy_block_dropping([x, y, z])?;
                        affected = true;
                    }
                    let target = if (outline || hollow) && !edge {
                        if outline {
                            if affected {
                                count += 1;
                            }
                            continue;
                        }
                        air
                    } else {
                        state
                    };
                    if self.place_state([x, y, z], target, strict)? || affected {
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            return Err(err("commands.fill.failed", &[]));
        }
        self.feedback(Text::tr("commands.fill.success", &[&count]));
        Ok(count)
    }
    fn cmd_clone(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "begin")?;
        let b = self.block_pos_arg(ctx, "end")?;
        let dest = self.block_pos_arg(ctx, "destination")?;
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let dmax = [dest[0] + size[0], dest[1] + size[1], dest[2] + size[2]];
        let overlap = (0..3).all(|i| dest[i] <= max[i] && dmax[i] >= min[i]);
        let moving = ctx.has("move");
        // Mode.canOverlap: only the normal mode refuses overlapping regions.
        if !ctx.has("force") && !moving && overlap {
            return Err(err("commands.clone.overlap", &[]));
        }
        let area = (0..3).map(|i| i64::from(size[i] + 1)).product::<i64>();
        let limit = self.ui.rules.int("max_block_modifications");
        if area > i64::from(limit) {
            return Err(err("commands.clone.toobig", &[&limit, &area]));
        }
        for p in [dest, dmax] {
            if !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
                return Err(err("argument.pos.unloaded", &[]));
            }
        }
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let air = self.world.air;
        let masked = ctx.has("masked");
        let mut copies = Vec::new();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let s = self.world.block(x, y, z);
                    if masked && s == air {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, s)
                    {
                        continue;
                    }
                    copies.push((
                        [x, y, z],
                        [
                            dest[0] + x - min[0],
                            dest[1] + y - min[1],
                            dest[2] + z - min[2],
                        ],
                        s,
                        self.world.block_entities.get(&(x, y, z)).cloned(),
                    ));
                }
            }
        }
        if moving {
            for (from, _, _, _) in &copies {
                self.place_state(*from, air, true)?;
            }
        }
        let strict = ctx.has("strict");
        // CloneCommands.clone sets every destination to a barrier first, so
        // each copied state but a barrier counts as placed.
        let barrier = self.pack.assets.registry.default_state("minecraft:barrier");
        let mut count = 0;
        for (_, to, s, entity) in copies {
            self.place_state(to, s, strict)?;
            if Some(s) != barrier {
                count += 1;
            }
            if let Some(e) = entity {
                self.world.block_entities.insert((to[0], to[1], to[2]), e);
            }
        }
        if count == 0 {
            return Err(err("commands.clone.failed", &[]));
        }
        self.feedback(Text::tr("commands.clone.success", &[&count]));
        Ok(count)
    }
    /// Player.giveExperiencePoints including negative amounts.
    fn give_points(&mut self, amount: i32) {
        let e = &mut self.experience;
        e.progress += amount as f32 / horizon_client::entities::Experience::needed(e.level) as f32;
        e.total = e.total.saturating_add(amount).clamp(0, i32::MAX);
        while e.progress < 0.0 {
            let f = e.progress * horizon_client::entities::Experience::needed(e.level) as f32;
            if e.level > 0 {
                e.level -= 1;
                e.progress = 1.0 + f / horizon_client::entities::Experience::needed(e.level) as f32;
            } else {
                e.level = 0;
                e.progress = 0.0;
            }
        }
        while e.progress >= 1.0 {
            e.progress =
                (e.progress - 1.0) * horizon_client::entities::Experience::needed(e.level) as f32;
            e.level += 1;
            e.progress /= horizon_client::entities::Experience::needed(e.level) as f32;
        }
    }
    /// AdvancementCommands.perform / performCriterion.
    fn cmd_advancement(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let grant = ctx.has("grant");
        let key = if grant { "grant" } else { "revoke" };
        let index = match ctx.get("advancement") {
            Some(Arg::Resource(id)) => Some(
                *tree
                    .by_id
                    .get(id)
                    .ok_or_else(|| err("advancement.advancementNotFound", &[id]))?,
            ),
            _ => None,
        };
        let player = Text::literal(self.acting_player_name());
        let players = targets.len();
        if let (Some(i), Some(Arg::Str(criterion))) = (index, ctx.get("criterion")) {
            let a = &tree.list[i];
            let name = super::advancements::adv_name(a);
            if !a.criteria.iter().any(|c| &c.name == criterion) {
                return Err(CommandError::new(Text::translate(
                    "commands.advancement.criterionNotFound",
                    &[name, Text::literal(criterion.clone())],
                )));
            }
            let changed = if grant {
                self.grant_advancement(i, Some(criterion))
            } else {
                self.revoke_advancement(i, Some(criterion))
            };
            let to = if players == 1 { "one" } else { "many" };
            let who = if players == 1 {
                player
            } else {
                Text::literal(players.to_string())
            };
            let args = [Text::literal(criterion.clone()), name, who];
            if !changed {
                return Err(CommandError::new(Text::translate(
                    &format!("commands.advancement.{key}.criterion.to.{to}.failure"),
                    &args,
                )));
            }
            self.feedback(Text::translate(
                &format!("commands.advancement.{key}.criterion.to.{to}.success"),
                &args,
            ));
            return Ok(1);
        }
        // getAdvancements: parents first (nearest up), the target, then
        // the subtree depth first.
        let list: Vec<usize> = match index {
            None => (0..tree.list.len()).collect(),
            Some(i) => {
                let mut out = Vec::new();
                if ctx.has("until") || ctx.has("through") {
                    let mut p = tree.list[i].parent.as_ref();
                    while let Some(&pi) = p.and_then(|id| tree.by_id.get(id)) {
                        out.push(pi);
                        p = tree.list[pi].parent.as_ref();
                    }
                }
                out.push(i);
                if ctx.has("from") || ctx.has("through") {
                    let kids = tree.children();
                    fn add(kids: &[Vec<usize>], i: usize, out: &mut Vec<usize>) {
                        for &c in &kids[i] {
                            out.push(c);
                            add(kids, c, out);
                        }
                    }
                    add(&kids, i, &mut out);
                }
                out
            }
        };
        self.adv.quiet = index.is_none();
        let mut count = 0;
        for &i in &list {
            let changed = if grant {
                self.grant_advancement(i, None)
            } else {
                self.revoke_advancement(i, None)
            };
            count += i32::from(changed);
        }
        self.adv.quiet = false;
        let to = if players == 1 { "one" } else { "many" };
        let who = if players == 1 {
            player
        } else {
            Text::literal(players.to_string())
        };
        let (what, first) = if list.len() == 1 {
            ("one", super::advancements::adv_name(&tree.list[list[0]]))
        } else {
            ("many", Text::literal(list.len().to_string()))
        };
        let args = [first, who];
        if count == 0 {
            return Err(CommandError::new(Text::translate(
                &format!("commands.advancement.{key}.{what}.to.{to}.failure"),
                &args,
            )));
        }
        self.feedback(Text::translate(
            &format!("commands.advancement.{key}.{what}.to.{to}.success"),
            &args,
        ));
        Ok(count)
    }
    fn cmd_experience(&mut self, ctx: &Context) -> R<i32> {
        let levels = ctx.has("levels");
        let unit = if levels { "levels" } else { "points" };
        if ctx.has("query") {
            let target = self.entities(ctx, "target", true)?[0];
            // ExperienceCommand.queryExperience: the target's own level or points.
            let value = self
                .for_player(target, |g| {
                    let e = &g.experience;
                    if levels {
                        e.level
                    } else {
                        (e.progress * horizon_client::entities::Experience::needed(e.level) as f32)
                            as i32
                    }
                })
                .unwrap_or(0);
            self.system_message(Text::translate(
                &format!("commands.experience.query.{unit}"),
                &[self.target_name(target), Text::literal(value.to_string())],
            ));
            return Ok(value);
        }
        let targets = self.entities(ctx, "target", true)?;
        let amount = Self::int(ctx, "amount").unwrap_or(0);
        let add = ctx.has("add");
        let op = if add { "add" } else { "set" };
        // Every target changes (a set of points can be refused for some).
        let changed: Vec<Target> = targets
            .iter()
            .copied()
            .filter(|&t| {
                self.for_player(t, |g| g.change_experience(add, levels, amount)) == Some(true)
            })
            .collect();
        if changed.is_empty() {
            return Err(err("commands.experience.set.points.invalid", &[]));
        }
        let (key, who) = match changed[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(changed.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.experience.{op}.{unit}.success.{key}"),
            &[Text::literal(amount.to_string()), who],
        ));
        Ok(changed.len() as i32)
    }
    /// ExperienceCommand's add and set on the player in place: false when
    /// the points to set do not fit the level.
    fn change_experience(&mut self, add: bool, levels: bool, amount: i32) -> bool {
        if add {
            if levels {
                self.experience.level = (self.experience.level + amount).max(0);
                if self.experience.level == 0 && amount < 0 {
                    self.experience.progress = 0.0;
                }
            } else {
                self.give_points(amount);
            }
        } else if levels {
            self.experience.level = amount;
        } else {
            let needed = horizon_client::entities::Experience::needed(self.experience.level);
            if amount > needed - 1 {
                return false;
            }
            self.experience.progress = amount as f32 / needed as f32;
        }
        true
    }
    fn cmd_gamerule(&mut self, ctx: &Context) -> R<i32> {
        let Some(name) = ctx.literals.get(1).cloned() else {
            return Ok(0);
        };
        let Some(rule) = gamerules::rule(&name) else {
            return Ok(0);
        };
        let current = self.ui.rules.get(rule.name).unwrap_or(rule.default);
        let value = match ctx.get("value") {
            Some(Arg::Bool(b)) => Value::Bool(*b),
            Some(Arg::Int(i)) => Value::Int(*i),
            _ => {
                self.system_message(Text::tr("commands.gamerule.query", &[&rule.name, &current]));
                return Ok(match current {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                });
            }
        };
        if current == value {
            return Err(err("commands.gamerule.not_set", &[&rule.name, &value]));
        }
        self.ui.rules.set(rule.name, value);
        self.sync_rules();
        self.feedback(Text::tr("commands.gamerule.set", &[&rule.name, &value]));
        Ok(match value {
            Value::Bool(b) => i32::from(b),
            Value::Int(i) => i,
        })
    }
    fn cmd_difficulty(&mut self, ctx: &Context) -> R<i32> {
        let name = |d: u8| {
            Text::translate(
                &format!("options.difficulty.{}", DIFFICULTIES[d as usize]),
                &[],
            )
        };
        let Some(i) = DIFFICULTIES.iter().position(|d| ctx.has(d)) else {
            let current = self.ui.difficulty;
            self.system_message(Text::translate(
                "commands.difficulty.query",
                &[name(current)],
            ));
            return Ok(i32::from(current));
        };
        let i = i as u8;
        if self.ui.difficulty == i {
            return Err(CommandError::new(Text::translate(
                "commands.difficulty.failure",
                &[name(i)],
            )));
        }
        self.ui.difficulty = i;
        // The level data holds the world's difficulty: it survives the
        // reopen, so it follows the command.
        self.level.set_difficulty(i);
        self.sync_rules();
        if i == 0 {
            // Peaceful removes hostile mobs.
            self.mobs.retain(|m| !m.hostile());
        }
        self.feedback(Text::translate("commands.difficulty.success", &[name(i)]));
        Ok(0)
    }
    fn cmd_spawnpoint(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = if ctx.get("pos").is_some() {
            self.block_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        let yaw = ((rot[0] + 180.0).rem_euclid(360.0)) - 180.0;
        let pitch = rot[1].clamp(-90.0, 90.0);
        // SetSpawnCommand: a forced point in the source's level, for each target.
        let level = self.dim;
        for &t in &targets {
            self.for_player(t, |g| {
                if level == Dim::Overworld {
                    g.spawn = Some(Vec3::new(
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]),
                        f64::from(pos[2]) + 0.5,
                    ));
                    g.respawn_config = None;
                } else {
                    g.spawn = None;
                    g.respawn_config = Some(super::respawn_anchor::RespawnConfig {
                        dim: level,
                        pos,
                        yaw,
                        forced: true,
                    });
                }
            });
        }
        let dim = Text::literal(self.dim.id());
        let p = pos.map(|v| Text::literal(v.to_string()));
        let mut args = vec![
            p[0].clone(),
            p[1].clone(),
            p[2].clone(),
            Text::literal(java_float(f64::from(yaw))),
            Text::literal(java_float(f64::from(pitch))),
            dim,
        ];
        let key = if targets.len() == 1 {
            args.push(self.target_name(targets[0]));
            "commands.spawnpoint.success.single"
        } else {
            args.push(Text::literal(targets.len().to_string()));
            "commands.spawnpoint.success.multiple"
        };
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }
    fn cmd_setworldspawn(&mut self, ctx: &Context) -> R<i32> {
        let pos = if ctx.get("pos").is_some() {
            self.block_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        // LevelData.setRespawnData: the world's spawn (level.dat `spawn`).
        let old = self.level.world_spawn.replace(pos);
        self.level.world_spawn_angle = rot;
        // A player whose fallback is the world spawn (no spawnpoint of its
        // own; a bed or an anchor that is gone falls back to it) respawns at
        // the new one.
        let floor = |s: Vec3| [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32];
        if self.spawn.map(floor) == old {
            self.spawn = Some(Vec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ));
        }
        let args = [
            Text::literal(pos[0].to_string()),
            Text::literal(pos[1].to_string()),
            Text::literal(pos[2].to_string()),
            Text::literal(java_float(f64::from(rot[0]))),
            Text::literal(java_float(f64::from(rot[1]))),
            Text::literal(self.dim.id()),
        ];
        self.feedback(Text::translate("commands.setworldspawn.success", &args));
        Ok(1)
    }
    fn item_matches(&self, predicate: &ItemInput, item: &str) -> bool {
        if predicate.any {
            return true;
        }
        if predicate.tag {
            let tag = predicate.id.trim_start_matches("minecraft:");
            return self
                .res
                .items()
                .item_tags
                .get(tag)
                .or_else(|| self.items().item_tags.get(&predicate.id))
                .is_some_and(|s| {
                    s.contains(item) || s.contains(item.trim_start_matches("minecraft:"))
                });
        }
        predicate.id == item
    }
    /// `ItemPredicateArgument`: the item test and its `[...]` component
    /// tests (presence, `=` equality, `!` negation, `|` alternatives).
    pub(super) fn stack_matches(&self, predicate: &ItemInput, stack: &ItemStack) -> bool {
        if !self.item_matches(predicate, &stack.item) {
            return false;
        }
        let Some(tests) = predicate.components.as_deref() else {
            return true;
        };
        let inner = tests
            .trim()
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or("");
        let components = match horizon_client::item_data::encode_components(stack) {
            Some(horizon_world::nbt::Tag::Compound(c)) => c,
            _ => Default::default(),
        };
        let test = |term: &str| -> bool {
            let term = term.trim();
            let (negated, term) = match term.strip_prefix('!') {
                Some(t) => (true, t.trim()),
                None => (false, term),
            };
            let split = term.find(['=', '~']).unwrap_or(term.len());
            let id = term[..split].trim();
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            let present = components.get(&id.as_str().into());
            let result = match term[split..].chars().next() {
                Some('=') => {
                    let value = horizon_world::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (present, value) {
                        (Some(have), Some(want)) => {
                            horizon_client::nbt_path::compare_nbt(&want, Some(have))
                                && horizon_client::nbt_path::compare_nbt(have, Some(&want))
                        }
                        _ => false,
                    }
                }
                Some('~') => {
                    // Sub-predicates: `count` and component predicates.
                    let value = horizon_world::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (id.as_str(), value) {
                        ("minecraft:count", Some(v)) => {
                            let n = i64::from(stack.count);
                            match &v {
                                horizon_world::nbt::Tag::Compound(_) => {
                                    let get = |k: &str| match v.get(k) {
                                        Some(horizon_world::nbt::Tag::Int(i)) => {
                                            Some(i64::from(*i))
                                        }
                                        _ => None,
                                    };
                                    get("min").is_none_or(|m| n >= m)
                                        && get("max").is_none_or(|m| n <= m)
                                }
                                horizon_world::nbt::Tag::Int(i) => n == i64::from(*i),
                                _ => false,
                            }
                        }
                        (_, Some(v)) => present.is_some_and(|have| {
                            horizon_client::nbt_path::compare_nbt(&v, Some(have))
                        }),
                        _ => false,
                    }
                }
                _ => present.is_some(),
            };
            result != negated
        };
        split_top_level(inner, ',')
            .iter()
            .filter(|t| !t.trim().is_empty())
            .all(|term| split_top_level(term, '|').iter().any(|alt| test(alt)))
    }
    fn cmd_clear(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let predicate = match ctx.get("item") {
            Some(Arg::Item(i)) => i.clone(),
            _ => ItemInput {
                id: "*".into(),
                tag: false,
                any: true,
                components: None,
            },
        };
        let max = Self::int(ctx, "maxCount").unwrap_or(-1);
        let counting = max == 0;
        // ClearInventoryCommands.clearInventory: each player on their own.
        let mut removed = 0i32;
        let mut found = Vec::new();
        for &t in &targets {
            let n = self
                .for_player(t, |g| g.clear_matching(&predicate, max))
                .unwrap_or(0);
            removed += n;
            if n != 0 {
                found.push(t);
            }
        }
        if removed == 0 {
            return Err(if targets.len() == 1 {
                CommandError::new(Text::translate(
                    "clear.failed.single",
                    &[self.target_name(targets[0])],
                ))
            } else {
                err("clear.failed.multiple", &[&targets.len()])
            });
        }
        let kind = if counting { "test" } else { "success" };
        let (key, who) = match found[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(found.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.clear.{kind}.{key}"),
            &[Text::literal(removed.to_string()), who],
        ));
        Ok(removed)
    }
    /// Inventory.clearOrCountMatchingItems for the player in place: `max`
    /// of the matching items go (all when negative, none and only counted
    /// when 0). Returns how many matched.
    fn clear_matching(&mut self, predicate: &ItemInput, max: i32) -> i32 {
        let counting = max == 0;
        // Inventory slots 0..40: main, armor feet to head, off hand;
        // then clearOrCountMatchingItems visits the crafting grid and carried stack.
        let mut removed = 0i32;
        let mut budget = if max < 0 { i32::MAX } else { max };
        let all = |g: &Self| -> Vec<Option<ItemStack>> {
            let mut v: Vec<Option<ItemStack>> = g.inventory.clone();
            v.extend(g.armor.iter().rev().cloned());
            v.push(g.offhand.clone());
            v.extend(g.craft_grid.iter().cloned());
            v.push(g.carried.clone());
            v
        };
        let matching: Vec<bool> = all(self)
            .iter()
            .map(|s| s.as_ref().is_some_and(|s| self.stack_matches(predicate, s)))
            .collect();
        let mut slots: Vec<&mut Option<ItemStack>> = Vec::new();
        slots.extend(self.inventory.iter_mut());
        slots.extend(self.armor.iter_mut().rev());
        slots.push(&mut self.offhand);
        slots.extend(self.craft_grid.iter_mut());
        slots.push(&mut self.carried);
        for (slot, matches) in slots.into_iter().zip(matching) {
            if !matches {
                continue;
            }
            let Some(stack) = slot.as_mut() else { continue };
            let take = if counting {
                stack.count as i32
            } else {
                (stack.count as i32).min(budget)
            };
            removed += take;
            if !counting {
                budget -= take;
                stack.count -= take as u32;
                if stack.count == 0 {
                    *slot = None;
                }
                if budget == 0 {
                    break;
                }
            }
        }
        removed
    }
    /// `PoiTypes`: the point of interest type of a block state.
    fn poi_type(&self, state: horizon_world::registry::BlockStateId) -> Option<&'static str> {
        let s = self.pack.assets.registry.state(state)?;
        horizon_client::save::poi_type(s).map(|(kind, _)| kind)
    }
    /// `LocateCommand.locatePoi`: `PoiManager.findClosestWithType` within
    /// 256 blocks of the source, over the loaded chunks.
    fn cmd_locate_poi(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(poi)) = ctx.get("poi").cloned() else {
            return Ok(0);
        };
        let wanted: Vec<String> = match poi.strip_prefix('#') {
            Some(tag) => self.tag_entries("point_of_interest_type", tag),
            None => vec![poi.clone()],
        };
        let from = self.source_position().map(|v| v.floor() as i32);
        const RANGE: i32 = 256;
        let mut cache: std::collections::HashMap<u32, Option<&'static str>> =
            std::collections::HashMap::new();
        let mut best: Option<(i64, [i32; 3], &'static str)> = None;
        let (cx0, cz0) = ((from[0] - RANGE) >> 4, (from[2] - RANGE) >> 4);
        let (cx1, cz1) = ((from[0] + RANGE) >> 4, (from[2] + RANGE) >> 4);
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let Some(chunk) = self.world.chunks.get(&(cx, cz)) else {
                    continue;
                };
                for (i, section) in chunk.sections.iter().enumerate() {
                    let mut wanted_state = |state: u32| {
                        cache
                            .entry(state)
                            .or_insert_with(|| {
                                self.poi_type(horizon_world::registry::BlockStateId(state))
                                    .filter(|t| wanted.iter().any(|w| w == t))
                            })
                            .is_some()
                    };
                    if !section.blocks.maybe_has(&mut wanted_state) {
                        continue;
                    }
                    let base_y = horizon_client::chunk::MIN_Y + 16 * i as i32;
                    for j in 0..4096 {
                        let state = section.get(j);
                        let Some(kind) = cache.get(&state).copied().flatten() else {
                            continue;
                        };
                        let p = [
                            cx * 16 + (j & 15) as i32,
                            base_y + (j >> 8) as i32,
                            cz * 16 + ((j >> 4) & 15) as i32,
                        ];
                        let d = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                            .map(|d| i64::from(d) * i64::from(d))
                            .iter()
                            .sum::<i64>();
                        if d <= i64::from(RANGE * RANGE) && best.is_none_or(|(b, _, _)| d < b) {
                            best = Some((d, p, kind));
                        }
                    }
                }
            }
        }
        let Some((_, p, kind)) = best else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.poi.not_found",
                &[&poi],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (p[0] - from[0], p[2] - from[2]);
        let distance = ((dx * dx + dz * dz) as f32).sqrt().floor() as i32;
        let shown = if poi.starts_with('#') {
            format!("{poi} ({kind})")
        } else {
            poi
        };
        self.feedback(Text::translate(
            "commands.locate.poi.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    fn cmd_locate(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let origin = self.source_position();
        if id == "locate.biome" {
            let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
                return Ok(0);
            };
            // LocateCommand.locateBiome: BiomeSource.findClosestBiome3d with
            // a 6400-block radius, 32-block columns and 64-block layers.
            let from = origin.map(|v| v.floor() as i32);
            let allowed = |name: &str| self.biome_matches(&biome, name);
            let search = horizon_world::worldgen::generator::BiomeSearch {
                allowed: &allowed,
                origin: from,
                radius: 6400,
                horizontal: 32,
                vertical: 64,
                min_y: horizon_client::chunk::MIN_Y,
                max_y: horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT - 1,
            };
            let found = self
                .end
                .locator()
                .and_then(|t| t.find_closest_biome(&search));
            let Some((name, p)) = found else {
                return Err(CommandError::new(Text::tr(
                    "commands.locate.biome.not_found",
                    &[&biome],
                )));
            };
            let d2 = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                .map(|d| f64::from(d) * f64::from(d))
                .iter()
                .sum::<f64>();
            let shown = if biome.starts_with('#') {
                format!("{biome} ({name})")
            } else {
                biome.clone()
            };
            let link = Text::teleport_link(p[0], Some(p[1]), p[2]);
            let distance = (d2 as f32).sqrt().floor() as i32;
            self.feedback(Text::translate(
                "commands.locate.biome.success",
                &[
                    Text::literal(shown),
                    link,
                    Text::literal(distance.to_string()),
                ],
            ));
            return Ok(distance);
        }
        let Some(Arg::Resource(s)) = ctx.get("structure").cloned() else {
            return Ok(0);
        };
        // LocateCommand.locateStructure: an id or a #tag of structures.
        let names = match s.strip_prefix('#') {
            Some(tag) => self.structure_tag(tag),
            None => vec![s.clone()],
        };
        let from = origin.map(|v| v.floor() as i32);
        let found = self.end.locator().and_then(|t| t.locate(&names, from, 100));
        let Some((name, p)) = found else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.structure.not_found",
                &[&s],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
        let distance = (dx * dx + dz * dz).sqrt().floor() as i32;
        let shown = if s.starts_with('#') {
            format!("{s} ({name})")
        } else {
            name
        };
        self.feedback(Text::translate(
            "commands.locate.structure.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    /// The structures of a `worldgen/structure` tag, nested tags expanded.
    pub(super) fn structure_tag(&self, tag: &str) -> Vec<String> {
        self.tag_entries("worldgen/structure", tag)
    }
    /// The entries of a data pack tag of `registry`, nested tags expanded.
    pub(super) fn tag_entries(&self, registry: &str, tag: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![tag.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let (ns, path) = t.split_once(':').unwrap_or(("minecraft", &t));
            let file = self
                .options
                .resources
                .join(format!("data/{ns}/tags/{registry}/{path}.json"));
            let Ok(v) = std::fs::read(file)
                .map_err(|_| ())
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|_| ()))
            else {
                continue;
            };
            for e in v["values"].as_array().into_iter().flatten() {
                let id = e.as_str().or_else(|| e["id"].as_str()).unwrap_or_default();
                match id.strip_prefix('#') {
                    Some(nested) => pending.push(nested.to_owned()),
                    None if !id.is_empty() => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        out
    }
    /// ListPlayersCommand: every online player, the host of a hosted world
    /// first and then the guests in join order, with the count and the max.
    fn cmd_list(&mut self, ctx: &Context) -> R<i32> {
        // A single-player owner is a placed guest of the in-process server, so
        // it is the host and needs no entry of its own.
        let owner_listed = self
            .lan
            .as_ref()
            .is_some_and(|h| h.hub.singleplayer_owner.is_some());
        // The host is online unless this is a dedicated server, which has none.
        let host = (self.options.dedicated.is_none() && !owner_listed).then(|| match &self.lan {
            Some(h) => (h.host_uuid, h.host_name.clone()),
            None => (self.player_uuid(), PLAYER_NAME.to_owned()),
        });
        let guests = self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.placed)
            .map(|g| (g.uuid, g.name.clone()));
        let players: Vec<(u128, String)> = host.into_iter().chain(guests).collect();
        let max = self
            .lan
            .as_ref()
            .map_or(horizon_client::lan::MAX_PLAYERS, |h| {
                h.hub.lan.shared.max_players()
            });
        let uuids = ctx.has("uuids");
        let mut names = Text::empty();
        for (k, (uuid, name)) in players.iter().enumerate() {
            if k > 0 {
                // ComponentUtils.formatList: gray commas.
                names.push(Text::colored(", ", text::GRAY));
            }
            if uuids {
                names.push(Text::translate(
                    "commands.list.nameAndId",
                    &[
                        Text::literal(name.as_str()),
                        Text::literal(horizon_protocol::crypto::uuid_string(*uuid)),
                    ],
                ));
            } else {
                names.push(self.player_text(name, *uuid));
            }
        }
        let count = players.len();
        self.system_message(Text::translate(
            "commands.list.players",
            &[
                Text::literal(count.to_string()),
                Text::literal(max.to_string()),
                names,
            ],
        ));
        Ok(count as i32)
    }
    fn cmd_say(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let raw = match ctx.get(if id == "me" { "action" } else { "message" }) {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let message = self.expand_selectors(&raw);
        // CommandSourceStack.getDisplayName: the executing entity's name.
        let name = match self.command_source.map(|s| s.entity) {
            Some(Some(t)) => self.target_name(t),
            Some(None) => Text::literal("Server"),
            None => self.player_display_name(),
        };
        let line = match id {
            "say" => Text::translate("chat.type.announcement", &[name, message]),
            "me" => Text::translate("chat.type.emote", &[name, message]),
            _ => {
                let _ = self.entities(ctx, "targets", true)?;
                // Whispering to yourself: the incoming and outgoing lines.
                let out = Text::translate(
                    "commands.message.display.outgoing",
                    &[self.player_display_name(), message.clone()],
                )
                .with_color(text::GRAY)
                .italic();
                self.system_message(out);
                Text::translate(
                    "commands.message.display.incoming",
                    &[self.player_display_name(), message],
                )
                .with_color(text::GRAY)
                .italic()
            }
        };
        if let Some(capture) = self.chat_capture.as_mut() {
            capture.push(line.clone());
        }
        let focused = self.screen == Screen::Chat;
        self.ui.chat.add(
            &self.gui.atlas,
            line,
            None,
            horizon_client::chat::ChatSource::Player,
            self.ticks,
            focused,
        );
        Ok(1)
    }
    /// MessageArgument: `@p`-style selectors in a message become names.
    pub(super) fn expand_selectors(&mut self, raw: &str) -> Text {
        let mut out = Text::empty();
        let mut rest = raw;
        while let Some(at) = rest.find('@') {
            out.push(&rest[..at]);
            let mut reader = Reader::new(&rest[at..]);
            match (horizon_client::command::ArgType::Entity {
                single: false,
                players: false,
            })
            .parse(&mut reader, &self.ui.registries)
            {
                Ok(Arg::Entity(s)) if reader.cursor > 1 => {
                    let found = self.select(&s);
                    for (i, t) in found.iter().enumerate() {
                        if i > 0 {
                            out.push(", ");
                        }
                        out.push(self.target_name(*t));
                    }
                    rest = &rest[at + reader.cursor..];
                }
                _ => {
                    out.push("@");
                    rest = &rest[at + 1..];
                }
            }
        }
        out.push(rest);
        out
    }
}

/// Split at a separator outside brackets, braces and quotes.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut escaped, mut start) = (0i32, None::<char>, false, 0);
    for (i, ch) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}
