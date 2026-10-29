//! The scoreboard in the running game: saved data, criteria that follow the
//! player (health, food, xp, deaths, kills, statistics), score holders for
//! entities, the selector options `scores=`, `team=` and `tag=`, the
//! commands' [`Env`] and the sidebar.
use super::{Game, chat::PLAYER_NAME, commands::Target};
use horizon_client::{
    command::{
        Context,
        args::{IntRange, Selector},
    },
    hud::HudBatch,
    hud_layout, hud_overlays,
    lan::guest::Guest,
    save::{self, uuid_from_text, uuid_text},
    scoreboard::{self as sb, Comp, Scoreboard, literal},
    scoreboard_cmd::{self, Env, Holder},
    tab_list::TabPlayer,
    text::Text,
};
use horizon_world::nbt::Tag;
use std::collections::HashSet;

/// What `ServerPlayer` last recorded for the criteria that follow the player.
#[derive(Default)]
pub(super) struct Recorded {
    /// `lastRecordedHealthAndAbsorption` (compared as a float).
    health_and_absorption: Option<f32>,
    food: Option<i32>,
    air: Option<i32>,
    armor: Option<i32>,
    xp: Option<i32>,
    level: Option<i32>,
}
#[derive(Default)]
pub(super) struct ScoreState {
    pub board: Scoreboard,
    /// `/data ... storage`.
    pub storage: horizon_client::command_storage::CommandStorage,
    pub stopwatches: horizon_client::stopwatches::Stopwatches,
    loaded: bool,
    recorded: Recorded,
}

fn in_range(v: i32, (lo, hi): IntRange) -> bool {
    lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h)
}

impl Game<'_> {
    fn scoreboard_path(&self) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/scoreboard.dat")
    }
    fn border_path(&self) -> std::path::PathBuf {
        save::dimension_dir(&self.world_root, "minecraft:overworld")
            .join("data/minecraft/world_border.dat")
    }
    /// `ScoreboardSaveData` read from `data/minecraft/scoreboard.dat`.
    pub(super) fn load_scoreboard(&mut self) {
        if self.score.loaded || self.remote.is_some() {
            return;
        }
        self.score.loaded = true;
        if let Some(root) = save::read_dat(&self.scoreboard_path())
            && let Some(data) = root.get("data")
        {
            self.score.board = Scoreboard::unpack(data);
        }
        self.score.storage =
            horizon_client::command_storage::CommandStorage::load(&self.world_root);
        // ServerLevel.getWorldBorder: each dimension's data/minecraft/world_border.dat. Horizon
        // keeps one border, so the overworld's file stands for it; the root file is what
        // older Horizon builds wrote.
        let border_path = self.border_path();
        if let Some(root) = save::read_dat(&border_path)
            .or_else(|| save::read_dat(&self.world_root.join("data/minecraft/world_border.dat")))
            && let Some(data) = root.get("data")
        {
            self.border = horizon_client::worldborder::WorldBorder::unpack(data);
        }
        self.score.stopwatches = horizon_client::stopwatches::Stopwatches::load(&self.world_root);
        self.sync_score_registries();
    }
    pub(super) fn save_scoreboard(&mut self) {
        if !self.score.loaded || self.remote.is_some() {
            return;
        }
        self.score.storage.save(&self.world_root);
        self.score.stopwatches.save(&self.world_root);
        if self.border != horizon_client::worldborder::WorldBorder::default()
            && let Err(e) = save::write_dat(
                &self.border_path(),
                Tag::compound([
                    ("data", self.border.pack()),
                    ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
                ]),
            )
        {
            log::warn!("the world border was not written: {e}");
        }
        let path = self.scoreboard_path();
        let board = &self.score.board;
        if board == &Scoreboard::default() && !path.exists() {
            return;
        }
        if let Err(e) = save::write_dat(
            &path,
            Tag::compound([
                ("data", board.pack()),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        ) {
            log::warn!("the scoreboard was not written: {e}");
        }
    }
    /// The names commands complete: objectives, teams and tracked holders.
    pub(super) fn sync_score_registries(&mut self) {
        let board = &self.score.board;
        let reg = &mut self.ui.registries;
        reg.objectives = board.objectives.iter().map(|o| o.name.clone()).collect();
        // Trigger objectives: the locks `/trigger` and `scoreboard players enable` complete from.
        reg.trigger_scores.clear();
        for o in board.objectives.iter().filter(|o| o.criteria == "trigger") {
            let locks = board
                .scores
                .iter()
                .filter_map(|(holder, s)| s.get(&o.name).map(|s| (holder.clone(), s.locked)))
                .collect();
            reg.trigger_scores.insert(o.name.clone(), locks);
        }
        reg.trigger_objectives = reg
            .trigger_scores
            .iter()
            .filter(|(_, locks)| locks.get(PLAYER_NAME) == Some(&false))
            .map(|(o, _)| o.clone())
            .collect();
        reg.teams = board.teams.iter().map(|t| t.name.clone()).collect();
        reg.tracked = board.tracked();
        reg.storages = self.score.storage.keys();
        reg.stopwatches = self.score.stopwatches.ids();
    }

    // ---- score holders ---------------------------------------------------

    /// `ScoreHolder.getScoreboardName`: the player's name, an entity's UUID.
    pub(super) fn holder_name(&mut self, t: Target) -> Option<String> {
        match t {
            Target::Player => Some(self.acting_player_name().to_owned()),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.name.clone()),
            Target::Mob(id)
            | Target::Item(id)
            | Target::Stand(id)
            | Target::Frame(id)
            | Target::Painting(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Crystal(id)
            | Target::Dragon(id)
            | Target::Other(id) => Some(uuid_text(self.uuid_for(id))),
            // Orbs and projectiles have no UUID here.
            Target::Orb(_) | Target::Projectile(_) => None,
        }
    }
    fn holder_for(&mut self, t: Target) -> Option<Holder> {
        let name = self.holder_name(t)?;
        let shown = self.target_name(t);
        Some(Holder {
            display: Some(literal(&shown.plain())),
            text: shown,
            name,
        })
    }
    /// The entity with the given scoreboard name.
    fn target_of_holder(&self, name: &str) -> Option<Target> {
        if name == self.acting_player_name() {
            return Some(Target::Player);
        }
        let uuid = uuid_from_text(name).filter(|_| name.len() == 36)?;
        if Some(uuid) == self.level.player_uuid {
            return Some(Target::Player);
        }
        let id = self
            .persist
            .uuids
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id)?;
        if self.mobs.iter().any(|m| m.id == id) {
            Some(Target::Mob(id))
        } else if self.dropped.iter().any(|i| i.id == id) {
            Some(Target::Item(id))
        } else {
            None
        }
    }
    /// `Entity.getTags` (the `Tags` list of the saved entity).
    pub(super) fn tags_of(&self, t: Target) -> Vec<String> {
        let raw = match t {
            Target::Mob(id)
            | Target::Item(id)
            | Target::Stand(id)
            | Target::Frame(id)
            | Target::Painting(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Other(id) => self.persist.raw.get(&id),
            Target::Player => self.level.player_extra.as_ref(),
            Target::Guest(_)
            | Target::Orb(_)
            | Target::Projectile(_)
            | Target::Crystal(_)
            | Target::Dragon(_) => None,
        };
        match raw.and_then(|r| r.get("Tags")) {
            Some(Tag::List(l)) => l
                .iter()
                .filter_map(|t| match t {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// The selector options that need the scoreboard or entity data: which
    /// of `candidates` pass `scores=`, `team=` and `tag=`.
    pub(super) fn selector_extra_filter(
        &mut self,
        candidates: &[Target],
        s: &Selector,
    ) -> HashSet<Target> {
        let mut pass = HashSet::new();
        let needs_holder = !s.scores.is_empty() || !s.teams.is_empty();
        for &t in candidates {
            let mut ok = true;
            if needs_holder {
                let holder = self.holder_name(t);
                for (objective, range) in &s.scores {
                    let value = holder
                        .as_deref()
                        .and_then(|h| self.score.board.score(h, objective))
                        .map(|score| score.value);
                    if !value.is_some_and(|v| in_range(v, *range)) {
                        ok = false;
                    }
                }
                let team = holder
                    .as_deref()
                    .and_then(|h| self.score.board.players_team(h))
                    .map(|t| t.name.clone());
                for (want, negated) in &s.teams {
                    let matches = match &team {
                        Some(name) => name == want,
                        None => want.is_empty(),
                    };
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if !s.tags.is_empty() {
                let tags = self.tags_of(t);
                for (tag, negated) in &s.tags {
                    let has = if tag.is_empty() {
                        !tags.is_empty()
                    } else {
                        tags.contains(tag)
                    };
                    if has == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.nbt.is_empty() {
                // NbtPredicate: the entity's saved data contains the pattern.
                let data = self.entity_nbt(t);
                for (pattern, negated) in &s.nbt {
                    let matches = match (&data, horizon_world::nbt::parse_snbt(pattern)) {
                        (Some(d), Ok((p, _))) => horizon_client::nbt_path::compare_nbt(&p, Some(d)),
                        _ => false,
                    };
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.predicates.is_empty() {
                // EntityPredicates: the datapack predicate with the entity as
                // `this` and the entity's position as origin.
                self.ensure_datapacks();
                let view = self.entity_view_of(t);
                let origin = self.target_position(t).unwrap_or_default();
                for (id, negated) in &s.predicates {
                    let matches = self
                        .fns
                        .store
                        .predicates
                        .get(id)
                        .is_some_and(|json| self.conditions_hold(json, Some(&view), origin));
                    if matches == *negated {
                        ok = false;
                    }
                }
            }
            if ok && !s.advancements.is_empty() {
                ok = t == Target::Player && self.advancements_match(&s.advancements);
            }
            if ok {
                pass.insert(t);
            }
        }
        pass
    }

    /// `advancements=` of a selector, against the player's progress.
    fn advancements_match(
        &self,
        wanted: &[(String, horizon_client::command::args::AdvCondition)],
    ) -> bool {
        use horizon_client::command::args::AdvCondition;
        wanted.iter().all(|(id, cond)| {
            let Some(a) = self.adv.tree.get(id) else {
                return false;
            };
            match cond {
                AdvCondition::Done(want) => self.adv.progress.done(a) == *want,
                AdvCondition::Criteria(list) => list
                    .iter()
                    .all(|(c, want)| self.adv.progress.has(id, c) == *want),
            }
        })
    }

    // ---- criteria ----------------------------------------------------------

    /// `ServerStatsCounter.setValue`'s scoreboard side: the objectives of
    /// each changed statistic's criteria follow it for the player `name`.
    pub(super) fn stat_criteria_changed(
        &mut self,
        name: &str,
        events: Vec<(String, String, Option<i32>)>,
    ) {
        let board = &mut self.score.board;
        if board.objectives.is_empty() {
            return;
        }
        for (ty, key, amount) in events {
            let criteria = sb::stat_criteria(&ty, &key);
            if board.has_objectives_for(&criteria) {
                board.for_all_objectives(&criteria, name, |v| {
                    amount.map_or(0, |n| v.wrapping_add(n))
                });
            }
        }
    }
    /// `ServerScoreboard`'s packets: each placed player's client is sent
    /// what changed on the board since it was last sent it (all of it on
    /// joining), lan/scoreboard.rs.
    pub(super) fn lan_sync_scoreboard(&mut self, host: &mut super::lan::LanHost) {
        if host.hub.guests.iter().all(|g| !g.placed) {
            return;
        }
        self.load_scoreboard();
        host.hub.teams.clone_from(&self.score.board.teams);
        let view = horizon_client::lan::scoreboard::View::of(&self.score.board);
        for g in host.hub.guests.iter_mut().filter(|g| g.placed) {
            if g.scoreboard != view {
                g.send_all(&g.scoreboard.changes(&view));
                g.scoreboard = view.clone();
            }
        }
    }
    /// `ServerPlayer.doTick` score updates and the statistic criteria.
    pub(super) fn tick_scoreboard(&mut self) {
        self.load_scoreboard();
        let events = std::mem::take(&mut self.stats.events);
        self.stat_criteria_changed(PLAYER_NAME, events);
        let board = &mut self.score.board;
        if !board.objectives.is_empty() {
            let display = self.player_display_comp();
            if let Some(p) = self.player.as_mut() {
                let health_and_absorption = p.health.health() + p.health.absorption();
                let food = p.health.food.food;
                // getArmorValue: the armor attribute rounded down.
                let armor = p
                    .attribute("minecraft:armor")
                    .or_else(|_| p.attribute("armor"))
                    .unwrap_or(0.0)
                    .floor() as i32;
                let rec = &mut self.score.recorded;
                let board = &mut self.score.board;
                if rec.health_and_absorption != Some(health_and_absorption) {
                    rec.health_and_absorption = Some(health_and_absorption);
                    let value = health_and_absorption.ceil() as i32;
                    board.for_all_objectives_with(
                        sb::CRITERIA_HEALTH,
                        PLAYER_NAME,
                        Some(&display),
                        |_| value,
                    );
                }
                for (criteria, last, value) in [
                    (sb::CRITERIA_FOOD, &mut rec.food, food),
                    // The air supply is full unless the player is drowning.
                    (sb::CRITERIA_AIR, &mut rec.air, 300),
                    (sb::CRITERIA_ARMOR, &mut rec.armor, armor),
                    (sb::CRITERIA_XP, &mut rec.xp, self.experience.total),
                    (sb::CRITERIA_LEVEL, &mut rec.level, self.experience.level),
                ] {
                    if *last != Some(value) {
                        *last = Some(value);
                        board.for_all_objectives_with(
                            criteria,
                            PLAYER_NAME,
                            Some(&display),
                            |_| value,
                        );
                    }
                }
            }
        }
        if self.ticks.is_multiple_of(20) {
            self.sync_score_registries();
        }
    }
    /// `ServerPlayer.die`: the death count.
    pub(super) fn score_player_died(&mut self) {
        let display = self.player_display_comp();
        let me = self.acting_player_name().to_owned();
        let board = &mut self.score.board;
        board.for_all_objectives_with(sb::CRITERIA_DEATH_COUNT, &me, Some(&display), |v| {
            v.wrapping_add(1)
        });
        // A respawned player has everything to record again.
        self.score.recorded = Recorded::default();
    }
    /// `ServerPlayer.awardKillScore`: total kills and the team kill counts.
    pub(super) fn score_player_killed(&mut self, victim: Target) {
        let victim_name = self.holder_name(victim);
        let display = self.player_display_comp();
        let me = self.acting_player_name().to_owned();
        let board = &mut self.score.board;
        let inc = |v: i32| v.wrapping_add(1);
        board.for_all_objectives_with(sb::CRITERIA_TOTAL_KILLS, &me, Some(&display), inc);
        if victim == Target::Player {
            board.for_all_objectives_with(sb::CRITERIA_PLAYER_KILLS, &me, Some(&display), inc);
        }
        let color_of =
            |board: &Scoreboard, name: &str| board.players_team(name).and_then(|t| t.color.clone());
        if let Some(victim_name) = victim_name {
            if let Some(color) = color_of(board, &victim_name) {
                let criteria = format!("teamkill.{color}");
                board.for_all_objectives_with(&criteria, &me, Some(&display), inc);
            }
            if let Some(color) = color_of(board, &me) {
                let criteria = format!("killedByTeam.{color}");
                board.for_all_objectives(&criteria, &victim_name, inc);
            }
        }
    }
    /// `Scoreboard.entityRemoved`: a dead non-player entity leaves the board.
    pub(super) fn score_entity_removed(&mut self, id: u32) {
        let Some(uuid) = self.persist.uuids.get(&id).copied() else {
            return;
        };
        self.score.board.entity_removed(&uuid_text(uuid));
    }

    // ---- teams in gameplay -----------------------------------------------------

    /// `Entity.getScoreboardName`: another player's profile name
    /// (`Player.getScoreboardName`), any other entity's UUID.
    fn scoreboard_name(&self, id: u32, uuid: [i32; 4]) -> String {
        self.remote_player_name(id)
            .map_or_else(|| uuid_text(uuid), str::to_owned)
    }
    /// `Entity.getTeam` of a mob or another player.
    pub(super) fn mob_team(&self, id: u32) -> Option<&sb::Team> {
        let uuid = self.persist.uuids.get(&id)?;
        self.score
            .board
            .players_team(&self.scoreboard_name(id, *uuid))
    }
    pub(super) fn player_team(&self) -> Option<&sb::Team> {
        self.score.board.players_team(self.acting_player_name())
    }
    /// The teams of the player and of the mobs on one (empty without teams).
    pub(super) fn mob_teams(&self) -> horizon_client::mobs::Teams<'_> {
        let board = &self.score.board;
        if board.teams.is_empty() {
            return Default::default();
        }
        horizon_client::mobs::Teams {
            player: self.player_team(),
            mobs: self
                .persist
                .uuids
                .iter()
                .filter_map(|(id, uuid)| {
                    Some((*id, board.players_team(&self.scoreboard_name(*id, *uuid))?))
                })
                .collect(),
        }
    }
    /// The mobs on the player's team (`isAlliedTo`).
    pub(super) fn player_allies(&self) -> HashSet<u32> {
        let Some(team) = self.player_team() else {
            return HashSet::new();
        };
        self.persist
            .uuids
            .iter()
            .filter(|(id, uuid)| team.players.contains(&self.scoreboard_name(**id, **uuid)))
            .map(|(id, _)| *id)
            .collect()
    }
    /// `Player.getDisplayName`: the name with the team's prefix, suffix and colour.
    pub(super) fn player_display_comp(&self) -> Comp {
        let name = literal(self.acting_player_name());
        match self.player_team() {
            Some(t) => t.formatted_name_comp(&name),
            None => name,
        }
    }
    pub(super) fn player_display_text(&self) -> Text {
        let name = Text::literal(self.acting_player_name());
        match self.player_team() {
            Some(t) => t.formatted_name(name),
            None => name,
        }
    }

    // ---- commands ------------------------------------------------------------

    /// The scoreboard names a score holder argument resolves to.
    pub(super) fn score_holder_names(
        &mut self,
        ctx: &Context,
        name: &str,
        default_wildcard: bool,
    ) -> super::commands::R<Vec<String>> {
        let mut env = GameEnv { g: self };
        Ok(
            scoreboard_cmd::holders(ctx, name, &mut env, default_wildcard)?
                .into_iter()
                .map(|h| h.name)
                .collect(),
        )
    }
    pub(super) fn cmd_scoreboard(&mut self, id: &str, ctx: &Context) -> super::commands::R<i32> {
        let mut env = GameEnv { g: self };
        let result = scoreboard_cmd::execute(id, ctx, &mut env).unwrap_or_else(|| {
            Err(horizon_client::command::CommandError::tr(
                "command.failed",
                &[],
            ))
        });
        self.sync_score_registries();
        result
    }
    /// ComponentUtils.updateForEntity for the pieces scoreboards store:
    /// `selector` and `score` components become their text.
    pub(super) fn resolve_component(&mut self, c: Comp) -> Comp {
        match c {
            Comp::Object(mut o) => {
                if let Some(sel) = o.get("selector").and_then(Comp::as_str).map(str::to_owned) {
                    let text = self.resolve_selector_text(&sel);
                    o.remove("selector");
                    o.insert("text".into(), Comp::String(text));
                } else if let Some(score) = o.get("score").cloned() {
                    let name = score.get("name").and_then(Comp::as_str).unwrap_or("");
                    let objective = score.get("objective").and_then(Comp::as_str).unwrap_or("");
                    let name = if name == "*" { PLAYER_NAME } else { name };
                    let text = self
                        .score
                        .board
                        .score(name, objective)
                        .map_or_else(String::new, |s| s.value.to_string());
                    o.remove("score");
                    o.insert("text".into(), Comp::String(text));
                }
                for key in ["extra", "with"] {
                    if let Some(Comp::Array(list)) = o.remove(key) {
                        let list = list
                            .into_iter()
                            .map(|c| self.resolve_component(c))
                            .collect();
                        o.insert(key.into(), Comp::Array(list));
                    }
                }
                Comp::Object(o)
            }
            Comp::Array(list) => Comp::Array(
                list.into_iter()
                    .map(|c| self.resolve_component(c))
                    .collect(),
            ),
            other => other,
        }
    }
    fn resolve_selector_text(&mut self, raw: &str) -> String {
        let mut reader = horizon_client::command::Reader::new(raw);
        let ty = horizon_client::command::args::ArgType::Entity {
            single: false,
            players: false,
        };
        let Ok(horizon_client::command::Arg::Entity(sel)) =
            ty.parse(&mut reader, &self.ui.registries)
        else {
            return raw.to_owned();
        };
        let names: Vec<String> = self
            .select(&sel)
            .into_iter()
            .map(|t| self.target_name(t).plain())
            .collect();
        names.join(", ")
    }

    // ---- the sidebar -----------------------------------------------------------

    /// The objective in the player's team's sidebar slot, else the sidebar's.
    fn sidebar_objective(&self) -> Option<&sb::Objective> {
        let board = &self.score.board;
        if board.display.is_empty() {
            return None;
        }
        // A server's client is in its team by its profile name.
        let name = self
            .remote
            .as_ref()
            .map_or(PLAYER_NAME, |r| r.profile_name.as_str());
        let team_slot = board
            .players_team(name)
            .and_then(|t| t.color.as_ref())
            .map(|c| format!("sidebar.team.{c}"))
            .and_then(|slot| board.display_objective(&slot));
        team_slot.or_else(|| board.display_objective(sb::SLOT_SIDEBAR))
    }
    /// The sidebar the HUD shows: its objective and its lines, for `status`.
    pub(super) fn sidebar_status(&self) -> serde_json::Value {
        let board = &self.score.board;
        let Some(objective) = self.sidebar_objective() else {
            return serde_json::Value::Null;
        };
        let scores: serde_json::Map<String, serde_json::Value> = board
            .scores
            .iter()
            .filter_map(|(holder, s)| Some((holder.clone(), s.get(&objective.name)?.value.into())))
            .collect();
        serde_json::json!({ "objective": objective.name, "title": objective.display_text().plain(), "scores": scores })
    }

    /// `Hud.extractScoreboardSidebar` / `displayScoreboardSidebar`.
    pub(super) fn draw_scoreboard(&self, batch: &mut HudBatch) {
        let board = &self.score.board;
        let Some(objective) = self.sidebar_objective() else {
            return;
        };
        let draws = hud_overlays::sidebar(
            batch.width as i32,
            batch.height as i32,
            board,
            objective,
            &self.gui.atlas.font,
            self.settings.text_background(0.3),
            self.settings.text_background(0.4),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}

impl Game<'_> {
    /// `Hud.extractTabList`: the player list while the player-list key is
    /// held. A local world shows it with a joined LAN guest or a `list`
    /// objective.
    pub(super) fn draw_tab_list(&mut self, batch: &mut HudBatch) {
        use horizon_client::tab_list::{self, TabInputs};
        // Player heads and the player list both draw from the same skin slots.
        let ready: Vec<_> = self.skin_downloads.borrow_mut().take_ready();
        for (location, rgba) in ready {
            self.store_skin(location, rgba);
        }
        let key = self.screen == super::Screen::Playing
            && self.keys.contains(&winit::keyboard::KeyCode::Tab);
        let (players, online_mode, upside_down) = match self.remote_tab_players() {
            Some(remote) => remote,
            None => {
                // The integrated server: the local player, online mode, no ping.
                let uuid = self.level.player_uuid.map_or(0, |u| {
                    u.iter()
                        .fold(0u128, |a, &w| (a << 32) | u128::from(w as u32))
                });
                let mut me = TabPlayer::new(uuid, PLAYER_NAME);
                me.game_mode = self.mode_id();
                me.skin = "entity/player/wide/steve".into();
                let guests = self.lan.as_ref().map_or(&[][..], |h| &h.hub.guests);
                let players = local_tab_players(me, guests);
                let upside_down = players
                    .iter()
                    .filter(|p| p.name == "Dinnerbone" || p.name == "Grumm")
                    .map(|p| p.uuid)
                    .collect();
                (players, true, upside_down)
            }
        };
        let board = &self.score.board;
        let objective = board.display_objective(sb::SLOT_LIST);
        let shown = tab_list::visible(
            key,
            self.has_singleplayer_server(),
            players.len(),
            objective.is_some(),
        );
        self.tab_list.set_visible(shown);
        if !shown {
            return;
        }
        let up = |id: u128| upside_down.contains(&id);
        let inputs = TabInputs {
            players: &players,
            board,
            objective,
            screen_width: batch.width as i32,
            online_mode,
            gui_ticks: self.ticks as i64,
            // Horizon has no "chat background only" option: vanilla's default (on).
            background: tab_list::row_background(
                true,
                f64::from(self.settings.text_background_opacity),
            ),
            upside_down: &up,
        };
        let draws = self.tab_list.layout(&inputs, &self.gui.atlas.font);
        horizon_client::hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}

/// The host and guests that have entered the world, rebuilt from live state.
fn local_tab_players(me: TabPlayer, guests: &[Guest]) -> Vec<TabPlayer> {
    let mut players = vec![me];
    players.extend(guests.iter().filter(|g| g.placed).map(|g| {
        let mut player = TabPlayer::new(g.uuid, &g.name);
        player.game_mode = g.game_mode;
        player.latency = g.latency.load(std::sync::atomic::Ordering::Relaxed);
        player
    }));
    players
}

/// The commands' view of the game.
struct GameEnv<'a, 'b> {
    g: &'a mut Game<'b>,
}
impl Env for GameEnv<'_, '_> {
    fn sb(&self) -> &Scoreboard {
        &self.g.score.board
    }
    fn sb_mut(&mut self) -> &mut Scoreboard {
        &mut self.g.score.board
    }
    fn select(&mut self, selector: &Selector) -> Vec<Holder> {
        let found = self.g.select(selector);
        found
            .into_iter()
            .filter_map(|t| self.g.holder_for(t))
            .collect()
    }
    fn named(&mut self, name: &str) -> Holder {
        if name.eq_ignore_ascii_case(self.g.acting_player_name()) && self.g.player.is_some() {
            return self
                .g
                .holder_for(Target::Player)
                .unwrap_or_else(|| Holder::named(name));
        }
        match self.g.target_of_holder(name) {
            Some(t) => self.g.holder_for(t).unwrap_or_else(|| Holder::named(name)),
            None => Holder::named(name),
        }
    }
    fn source(&mut self) -> Option<(Holder, bool)> {
        let entity = match self.g.command_source {
            Some(s) => s.entity,
            None => Some(Target::Player),
        }?;
        let name = match entity {
            Target::Player => self.g.acting_player_name().to_owned(),
            Target::Mob(id) | Target::Item(id) => uuid_text(self.g.uuid_for(id)),
            _ => return None,
        };
        let text = self.g.target_name(entity);
        Some((
            Holder {
                display: Some(literal(&text.plain())),
                text,
                name,
            },
            entity == Target::Player,
        ))
    }
    fn resolve(&mut self, c: Comp) -> Comp {
        self.g.resolve_component(c)
    }
    fn feedback(&mut self, message: Text) {
        self.g.feedback(message);
    }
}

#[cfg(test)]
mod tab_list_tests {
    use super::*;
    use horizon_client::{
        lan::{Joined, PlayerSender},
        tab_list,
    };
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
        mpsc,
    };

    fn guest(uuid: u128, name: &str) -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: uuid as u32,
                uuid,
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(42)),
                address: "127.0.0.1:25565".parse().unwrap(),
            },
            uuid as i32,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    #[test]
    fn lan_tab_list_tracks_join_and_disconnect() {
        let me = TabPlayer::new(1, PLAYER_NAME);
        let mut guests = vec![guest(2, "Alex")];
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.as_slice(), std::slice::from_ref(&me));
        assert!(!tab_list::visible(true, true, players.len(), false));

        guests[0].placed = true;
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.len(), 2);
        assert_eq!(players[0], me);
        assert_eq!(players[1].uuid, 2);
        assert_eq!(players[1].name, "Alex");
        assert!(tab_list::visible(true, true, players.len(), false));
        assert!(!tab_list::visible(false, true, players.len(), false));

        guests.clear();
        let players = local_tab_players(me, &guests);
        assert_eq!(players.len(), 1);
        assert!(!tab_list::visible(true, true, players.len(), false));
        assert!(tab_list::visible(true, true, players.len(), true));
    }

    #[test]
    fn lan_tab_list_reads_current_guest_state() {
        let me = TabPlayer::new(1, PLAYER_NAME);
        let mut guests = vec![guest(2, "Alex"), guest(3, "Grumm")];
        for guest in &mut guests {
            guest.placed = true;
        }
        guests[0].game_mode = tab_list::SPECTATOR;
        let players = local_tab_players(me.clone(), &guests);
        assert_eq!(players.len(), 3);
        assert_eq!(players[1].game_mode, tab_list::SPECTATOR);
        assert_eq!(players[1].latency, 42);
        assert_eq!(players[1].skin, tab_list::default_skin(2));
        assert_eq!(players[2].name, "Grumm");

        guests[0].game_mode = 1;
        guests[0].latency.store(123, Ordering::Relaxed);
        guests.remove(1);
        let players = local_tab_players(me, &guests);
        assert_eq!(players.len(), 2);
        assert_eq!(players[1].game_mode, 1);
        assert_eq!(players[1].latency, 123);
    }
}
