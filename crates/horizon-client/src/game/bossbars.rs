//! `/bossbar` (BossBarCommands) and drawing the custom boss bars.
use super::{
    Dim, Game,
    commands::{R, Target, err},
};
use horizon_client::{
    boss_overlay,
    bossbar::{self, BossBars},
    command::{Arg, Context},
    mobs::Species,
    save,
    scoreboard::{self as sb, Comp, json_to_tag},
    scoreboard_cmd::format_list,
    text::Text,
};
use horizon_core::random::RandomSource;
use horizon_protocol::packets::play::{self, BossAction, Clientbound as C};
use horizon_world::{identifier::Identifier, java_hash_map::JavaHashMap, nbt::Tag};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct BossState {
    pub bars: BossBars,
    /// CustomBossEvents' default HashMap order, used only by the list reply.
    listing: JavaHashMap<Identifier, usize>,
    loaded: bool,
    /// What each viewer was last sent, so a change becomes one packet.
    sent: HashMap<u128, HashMap<u128, Offer>>,
}

impl BossState {
    fn add_bar(&mut self, id: &str, name: Comp) -> bool {
        if !self.bars.add(id, name) {
            return false;
        }
        if let Ok(key) = Identifier::parse(id) {
            self.listing.put(key, self.bars.bars.len() - 1);
        }
        true
    }

    fn remove_bar(&mut self, id: &str) {
        self.bars.remove(id);
        if let Ok(key) = Identifier::parse(id) {
            self.listing.remove(&key);
        }
        // Vec removal shifts indices. Replacing existing map values keeps
        // its table and bucket chains intact, unlike reconstructing the map.
        for (index, bar) in self.bars.bars.iter().enumerate() {
            if let Ok(key) = Identifier::parse(&bar.id) {
                self.listing.put(key, index);
            }
        }
    }

    fn rebuild_listing(&mut self) {
        self.listing = JavaHashMap::default();
        for (index, bar) in self.bars.bars.iter().enumerate() {
            if let Ok(key) = Identifier::parse(&bar.id) {
                self.listing.put(key, index);
            }
        }
    }

    fn listed_bars(&self) -> Vec<&bossbar::BossBar> {
        let mut bars: Vec<_> = self
            .listing
            .entries()
            .filter_map(|(_, index)| self.bars.bars.get(*index))
            .collect();
        // The existing permissive loader can retain invalid legacy IDs.
        // Preserve those entries rather than dropping them from the reply.
        if bars.len() != self.bars.bars.len() {
            for bar in &self.bars.bars {
                if !bars.iter().any(|listed| listed.id == bar.id) {
                    bars.push(bar);
                }
            }
        }
        bars
    }
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

impl Game<'_> {
    fn bossbar_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data/minecraft/custom_boss_events.dat")
    }
    pub(super) fn load_bossbars(&mut self) {
        if self.bosses.loaded || self.remote.is_some() {
            return;
        }
        self.bosses.loaded = true;
        if let Some(root) = save::read_dat(&self.bossbar_path())
            && let Some(data) = root.get("data")
        {
            self.bosses.bars = BossBars::unpack(data);
        }
        self.bosses.rebuild_listing();
        self.ui.registries.bossbars = self.bosses.bars.ids();
    }
    pub(super) fn save_bossbars(&self) {
        if !self.bosses.loaded || self.remote.is_some() {
            return;
        }
        if self.bosses.bars.bars.is_empty() && !self.bossbar_path().exists() {
            return;
        }
        if let Err(e) = save::write_dat(
            &self.bossbar_path(),
            Tag::compound([
                ("data", self.bosses.bars.pack()),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        ) {
            log::warn!("boss bars were not written: {e}");
        }
    }
    pub(in crate::game) fn bossbar_exists(&self, id: &str) -> bool {
        self.bosses.bars.get(id).is_some()
    }
    /// `execute store ... bossbar <id> value|max`.
    pub(in crate::game) fn set_bossbar_value(&mut self, id: &str, value: bool, v: i32) {
        if let Some(b) = self.bosses.bars.get_mut(id) {
            if value {
                b.value = v.clamp(0, b.max);
            } else {
                b.max = v.max(1);
                b.value = b.value.min(b.max);
            }
        }
        self.publish_boss_bars();
    }
    fn bar_id(ctx: &Context) -> String {
        match ctx.get("id") {
            Some(Arg::Resource(id)) => id.clone(),
            _ => String::new(),
        }
    }
    fn bar_checked(&self, ctx: &Context) -> R<String> {
        let id = Self::bar_id(ctx);
        if self.bosses.bars.get(&id).is_none() {
            return Err(err("commands.bossbar.unknown", &[&id]));
        }
        Ok(id)
    }
    pub(super) fn cmd_bossbar(&mut self, id: &str, ctx: &Context) -> R<i32> {
        self.load_bossbars();
        let result = self.bossbar_command(id, ctx);
        self.ui.registries.bossbars = self.bosses.bars.ids();
        self.publish_boss_bars();
        result
    }
    fn bossbar_command(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "bossbar.add" => {
                let bar = Self::bar_id(ctx);
                if self.bosses.bars.get(&bar).is_some() {
                    return Err(err("commands.bossbar.create.failed", &[&bar]));
                }
                let name = self.component_json(ctx, "name")?;
                self.bosses.add_bar(&bar, name);
                if let Some(b) = self.bosses.bars.get_mut(&bar) {
                    // CustomBossEvents.create: Mth.createInsecureUUID(level.random).
                    b.uuid = insecure_uuid(&mut self.world.random);
                }
                let shown = self
                    .bosses
                    .bars
                    .get(&bar)
                    .map(|b| b.display_name())
                    .unwrap_or_default();
                self.feedback(Text::translate("commands.bossbar.create.success", &[shown]));
                return Ok(self.bosses.bars.bars.len() as i32);
            }
            "bossbar.list" => {
                let names: Vec<Text> = self
                    .bosses
                    .listed_bars()
                    .into_iter()
                    .map(horizon_client::bossbar::BossBar::display_name)
                    .collect();
                let n = names.len();
                if n == 0 {
                    self.feedback(Text::translate("commands.bossbar.list.bars.none", &[]));
                } else {
                    self.feedback(Text::translate(
                        "commands.bossbar.list.bars.some",
                        &[num(n), format_list(names)],
                    ));
                }
                return Ok(n as i32);
            }
            _ => {}
        }
        let bar = self.bar_checked(ctx)?;
        let shown = |g: &Self| {
            g.bosses
                .bars
                .get(&bar)
                .map(|b| b.display_name())
                .unwrap_or_default()
        };
        let last = ctx.literals.last().cloned().unwrap_or_default();
        match id {
            "bossbar.remove" => {
                let s = shown(self);
                self.bosses.remove_bar(&bar);
                self.feedback(Text::translate("commands.bossbar.remove.success", &[s]));
                Ok(self.bosses.bars.bars.len() as i32)
            }
            "bossbar.get" => {
                let b = self.bosses.bars.get(&bar).cloned().unwrap_or_else(|| {
                    horizon_client::bossbar::BossBar::new(&bar, sb::literal(&bar))
                });
                let s = b.display_name();
                Ok(match last.as_str() {
                    "value" => {
                        self.feedback(Text::translate(
                            "commands.bossbar.get.value",
                            &[s, num(b.value)],
                        ));
                        b.value
                    }
                    "max" => {
                        self.feedback(Text::translate(
                            "commands.bossbar.get.max",
                            &[s, num(b.max)],
                        ));
                        b.max
                    }
                    "visible" => {
                        let key = if b.visible {
                            "commands.bossbar.get.visible.visible"
                        } else {
                            "commands.bossbar.get.visible.hidden"
                        };
                        self.feedback(Text::translate(key, &[s]));
                        i32::from(b.visible)
                    }
                    _ => {
                        let names = self.bossbar_player_names(&b.players);
                        let n = names.len();
                        if n == 0 {
                            self.feedback(Text::translate(
                                "commands.bossbar.get.players.none",
                                &[s],
                            ));
                        } else {
                            self.feedback(Text::translate(
                                "commands.bossbar.get.players.some",
                                &[s, num(n), format_list(names)],
                            ));
                        }
                        n as i32
                    }
                })
            }
            _ => self.bossbar_set(id, ctx, &bar, &last),
        }
    }
    fn bossbar_set(&mut self, id: &str, ctx: &Context, bar: &str, last: &str) -> R<i32> {
        let shown = |g: &Self| {
            g.bosses
                .bars
                .get(bar)
                .map(|b| b.display_name())
                .unwrap_or_default()
        };
        let int = |name: &str| match ctx.get(name) {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        match id {
            "bossbar.set.name" => {
                let name = self.component_json(ctx, "name")?;
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                if b.name == name {
                    return Err(err("commands.bossbar.set.name.unchanged", &[]));
                }
                b.name = name;
                let s = shown(self);
                self.feedback(Text::translate("commands.bossbar.set.name.success", &[s]));
                Ok(0)
            }
            "bossbar.set.color" | "bossbar.set.style" => {
                let color = id.ends_with("color");
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                let slot = if color { &mut b.color } else { &mut b.style };
                if slot == last {
                    return Err(err(
                        if color {
                            "commands.bossbar.set.color.unchanged"
                        } else {
                            "commands.bossbar.set.style.unchanged"
                        },
                        &[],
                    ));
                }
                *slot = last.to_owned();
                let s = shown(self);
                self.feedback(Text::translate(
                    if color {
                        "commands.bossbar.set.color.success"
                    } else {
                        "commands.bossbar.set.style.success"
                    },
                    &[s],
                ));
                Ok(0)
            }
            "bossbar.set.value" | "bossbar.set.max" => {
                let is_value = id.ends_with("value");
                let v = int(if is_value { "value" } else { "max" });
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                let slot = if is_value { &mut b.value } else { &mut b.max };
                if *slot == v {
                    return Err(err(
                        if is_value {
                            "commands.bossbar.set.value.unchanged"
                        } else {
                            "commands.bossbar.set.max.unchanged"
                        },
                        &[],
                    ));
                }
                *slot = v;
                let s = shown(self);
                self.feedback(Text::translate(
                    if is_value {
                        "commands.bossbar.set.value.success"
                    } else {
                        "commands.bossbar.set.max.success"
                    },
                    &[s, num(v)],
                ));
                Ok(v)
            }
            "bossbar.set.visible" => {
                let visible = matches!(ctx.get("visible"), Some(Arg::Bool(true)));
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                if b.visible == visible {
                    return Err(err(
                        if visible {
                            "commands.bossbar.set.visibility.unchanged.visible"
                        } else {
                            "commands.bossbar.set.visibility.unchanged.hidden"
                        },
                        &[],
                    ));
                }
                b.visible = visible;
                let s = shown(self);
                self.feedback(Text::translate(
                    if visible {
                        "commands.bossbar.set.visible.success.visible"
                    } else {
                        "commands.bossbar.set.visible.success.hidden"
                    },
                    &[s],
                ));
                Ok(0)
            }
            _ => {
                // players [<targets>]: replace the set. No argument clears it.
                let targets = if ctx.get("targets").is_some() {
                    self.entities(ctx, "targets", true)?
                } else {
                    Vec::new()
                };
                let mut uuids = Vec::new();
                let mut names = Vec::new();
                for t in &targets {
                    let Some(uuid) = self.player_target_uuid(*t) else {
                        continue;
                    };
                    if !uuids.contains(&uuid) {
                        uuids.push(uuid);
                        names.push(self.target_name(*t));
                    }
                }
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                let mut had = b.players.clone();
                had.sort();
                let mut next = uuids.clone();
                next.sort();
                if had == next {
                    return Err(err("commands.bossbar.set.players.unchanged", &[]));
                }
                b.players = uuids;
                let n = b.players.len();
                let s = shown(self);
                if n == 0 {
                    self.feedback(Text::translate(
                        "commands.bossbar.set.players.success.none",
                        &[s],
                    ));
                } else {
                    self.feedback(Text::translate(
                        "commands.bossbar.set.players.success.some",
                        &[s, num(targets.len()), format_list(names)],
                    ));
                }
                Ok(n as i32)
            }
        }
    }
    /// A component argument as JSON with its selectors resolved.
    fn component_json(&mut self, ctx: &Context, name: &str) -> R<Comp> {
        let Some(Arg::Str(json)) = ctx.get(name) else {
            return Ok(sb::literal(""));
        };
        let value: Comp =
            serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
        Ok(self.resolve_component(value))
    }

    /// Names of the online players in a boss bar's set, in join order.
    fn bossbar_player_names(&self, players: &[[i32; 4]]) -> Vec<Text> {
        self.known_boss_players()
            .into_iter()
            .filter(|(uuid, _)| players.contains(uuid))
            .map(|(_, name)| name)
            .collect()
    }
    /// The players who can be told about a bar: the one in place, then guests.
    fn known_boss_players(&self) -> Vec<([i32; 4], Text)> {
        let mut out = Vec::new();
        if self.player.is_some() {
            out.push((self.acting_uuid(), self.target_name(Target::Player)));
        }
        if let Some(host) = &self.lan {
            for g in host.hub.guests.iter().filter(|g| g.placed) {
                let ints = horizon_client::lan::playerdata::uuid_ints(g.uuid);
                if out.iter().any(|(uuid, _)| *uuid == ints) {
                    continue;
                }
                out.push((ints, Text::literal(g.name.clone())));
            }
        }
        out
    }
    fn player_target_uuid(&self, target: Target) -> Option<[i32; 4]> {
        match target {
            Target::Player => Some(self.acting_uuid()),
            Target::Guest(conn) => self.lan.as_ref().and_then(|h| {
                h.hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .map(|g| horizon_client::lan::playerdata::uuid_ints(g.uuid))
            }),
            _ => None,
        }
    }

    // ---- ServerBossEvent delivery ------------------------------------------------

    /// Tell every viewer the boss events they should be seeing. The window
    /// that simulates the world applies them to its own overlay; everyone
    /// else gets `ClientboundBossEventPacket`s.
    pub(super) fn publish_boss_bars(&mut self) {
        if self.remote.is_some() {
            return;
        }
        self.load_bossbars();
        let now = boss_overlay::now_millis();
        let viewers = self.boss_viewers();
        let live: HashSet<u128> = viewers.iter().map(|v| v.uuid).collect();
        self.bosses.sent.retain(|id, _| live.contains(id));
        let mut packets = Vec::new();
        for viewer in &viewers {
            let desired = self.boss_events_for(viewer);
            for event in self.diff_boss_events(viewer, desired) {
                if viewer.local {
                    self.boss_overlay.apply(&event, now);
                } else if let Some(conn) = viewer.conn {
                    packets.push((conn, C::BossEvent(event)));
                }
            }
        }
        if packets.is_empty() {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        for (conn, packet) in &packets {
            host.hub.send_to(*conn, packet);
        }
    }
    fn boss_viewers(&self) -> Vec<Viewer> {
        let mut out = Vec::new();
        // While `as_guest` has swapped a guest in, the host's own position
        // is not in place. Their overlay updates once the tick puts it back.
        let local =
            self.serving.is_none() && self.options.dedicated.is_none() && self.lan_acting.is_none();
        if local && let Some(p) = &self.player {
            let ints = self.level.player_uuid.unwrap_or_else(|| self.acting_uuid());
            out.push(Viewer {
                uuid: horizon_client::lan::playerdata::ints_uuid(ints),
                ints,
                at: [p.position.x, p.position.y, p.position.z],
                alive: !p.health.dead_or_dying(),
                here: true,
                local: true,
                conn: None,
                view_chunks: self.settings.render_distance,
            });
        }
        if let Some(host) = &self.lan {
            for g in host.hub.guests.iter().filter(|g| g.placed) {
                if out.iter().any(|v| v.uuid == g.uuid) {
                    continue;
                }
                out.push(Viewer {
                    uuid: g.uuid,
                    ints: horizon_client::lan::playerdata::uuid_ints(g.uuid),
                    at: g.position,
                    alive: !g.dead,
                    here: g.dimension == self.dim.id(),
                    local: false,
                    conn: Some(g.conn),
                    view_chunks: g.view_distance,
                });
            }
        }
        out
    }
    /// Custom bars (`CustomBossEvents.onPlayerConnect` sends them as a player
    /// joins), then the dragon, withers and the raid.
    fn boss_events_for(&self, viewer: &Viewer) -> Vec<Offer> {
        let mut out = Vec::new();
        for b in &self.bosses.bars.bars {
            if !b.visible || !b.players.contains(&viewer.ints) {
                continue;
            }
            out.push(Offer {
                id: b.uuid,
                name: json_to_tag(&b.name),
                progress: b.progress(),
                color: color_index(&b.color),
                overlay: style_index(&b.style),
                darken: b.darken,
                music: b.music,
                fog: b.fog,
                scope: Scope::Global,
            });
        }
        if viewer.here
            && let Some(progress) = self.dragon_boss_progress(viewer.at, viewer.alive)
        {
            out.push(Offer {
                id: bossbar::stable_uuid("minecraft:ender_dragon"),
                name: translate_tag("entity.minecraft.ender_dragon"),
                progress,
                color: 0,
                overlay: 0,
                darken: false,
                music: true,
                fog: true,
                scope: Scope::Level(self.dim),
            });
        }
        if viewer.here {
            let range = f64::from(10.min(viewer.view_chunks.max(0)) * 16);
            for m in self
                .mobs
                .iter()
                .filter(|m| m.species() == Species::Wither && m.alive() && !m.removed)
            {
                let (dx, dz) = (m.position.x - viewer.at[0], m.position.z - viewer.at[2]);
                if dx * dx + dz * dz > range * range {
                    continue;
                }
                let progress = if m.extra.counter > 0 {
                    1.0 - m.extra.counter as f32 / horizon_client::mobs::wither::INVULNERABLE as f32
                } else {
                    (m.health / m.attrs.max_health).clamp(0.0, 1.0)
                };
                let name = if let Some(text) = &m.custom_name_text {
                    json_to_tag(&text.component())
                } else if let Some(name) = &m.custom_name {
                    Tag::String(name.as_str().into())
                } else {
                    translate_tag("entity.minecraft.wither")
                };
                out.push(Offer {
                    id: super::lan::entity_uuid(m.id),
                    name,
                    progress,
                    color: 5,
                    overlay: 0,
                    darken: true,
                    music: false,
                    fog: false,
                    scope: Scope::Level(self.dim),
                });
            }
        }
        if viewer.here
            && let Some((id, name, progress)) = self.raid_boss_at(viewer.at, viewer.alive)
        {
            out.push(Offer {
                id: bossbar::stable_uuid(&format!("raid:{id}")),
                name,
                progress,
                color: 2,
                overlay: 2,
                darken: false,
                music: false,
                fog: false,
                scope: Scope::Level(self.dim),
            });
        }
        out
    }
    /// The packets that take `viewer` from what it was last sent to `desired`
    /// (`ServerBossEvent`'s broadcasts).
    fn diff_boss_events(&mut self, viewer: &Viewer, desired: Vec<Offer>) -> Vec<play::BossEvent> {
        let full = viewer.local;
        let dim = self.dim;
        let sent = self.bosses.sent.entry(viewer.uuid).or_default();
        let mut out = Vec::new();
        let event = |id, action| play::BossEvent { id, action };
        sent.retain(|id, prev| {
            let owned = full || prev.scope == Scope::Global || prev.scope == Scope::Level(dim);
            let keep = !owned || desired.iter().any(|e| e.id == *id);
            if !keep {
                out.push(event(*id, BossAction::Remove));
            }
            keep
        });
        for offer in desired {
            let Some(prev) = sent.get(&offer.id) else {
                out.push(event(
                    offer.id,
                    BossAction::Add {
                        name: offer.name.clone(),
                        progress: offer.progress,
                        color: offer.color,
                        overlay: offer.overlay,
                        darken: offer.darken,
                        music: offer.music,
                        fog: offer.fog,
                    },
                ));
                sent.insert(offer.id, offer);
                continue;
            };
            let id = offer.id;
            if prev.progress != offer.progress {
                let progress = offer.progress;
                out.push(event(id, BossAction::UpdateProgress { progress }));
            }
            if prev.name != offer.name {
                let name = offer.name.clone();
                out.push(event(id, BossAction::UpdateName { name }));
            }
            if (prev.color, prev.overlay) != (offer.color, offer.overlay) {
                let (color, overlay) = (offer.color, offer.overlay);
                out.push(event(id, BossAction::UpdateStyle { color, overlay }));
            }
            if (prev.darken, prev.music, prev.fog) != (offer.darken, offer.music, offer.fog) {
                let (darken, music, fog) = (offer.darken, offer.music, offer.fog);
                out.push(event(
                    id,
                    BossAction::UpdateProperties { darken, music, fog },
                ));
            }
            sent.insert(id, offer);
        }
        out
    }
    /// `ClientPacketListener.handleBossUpdate`.
    pub(super) fn apply_boss_event(&mut self, packet: &play::BossEvent) {
        self.boss_overlay.apply(packet, boss_overlay::now_millis());
    }
}

/// `Mth.createInsecureUUID`.
fn insecure_uuid(random: &mut impl RandomSource) -> u128 {
    let most = random.next_long() & -61441 | 16384;
    let least = random.next_long() & 4_611_686_018_427_387_903 | i64::MIN;
    ((most as u64 as u128) << 64) | (least as u64 as u128)
}
fn translate_tag(key: &str) -> Tag {
    Tag::compound([("translate", Tag::String(key.into()))])
}
fn color_index(color: &str) -> i32 {
    bossbar::COLORS
        .iter()
        .position(|c| *c == color)
        .unwrap_or(6) as i32
}
fn style_index(style: &str) -> i32 {
    bossbar::STYLES
        .iter()
        .position(|s| *s == style)
        .unwrap_or(0) as i32
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Global,
    Level(Dim),
}
struct Viewer {
    uuid: u128,
    ints: [i32; 4],
    at: [f64; 3],
    alive: bool,
    here: bool,
    local: bool,
    conn: Option<u32>,
    view_chunks: i32,
}
/// A boss event as one viewer should see it, and as it was last sent.
struct Offer {
    id: u128,
    name: Tag,
    progress: f32,
    /// `BossBarColor` and `BossBarOverlay` ordinals.
    color: i32,
    overlay: i32,
    darken: bool,
    music: bool,
    fog: bool,
    scope: Scope,
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    fn ids(state: &BossState) -> Vec<&str> {
        state
            .listed_bars()
            .into_iter()
            .map(|bar| bar.id.as_str())
            .collect()
    }

    #[test]
    fn bossbar_listing_uses_identifier_hash_order_and_current_names() {
        let mut state = BossState::default();
        state.add_bar("audit:test", sb::literal("Title"));
        state.bars.get_mut("audit:test").unwrap().name = sb::literal("Other");
        state.add_bar("audit:second", sb::literal("Second"));
        assert_eq!(ids(&state), ["audit:second", "audit:test"]);
        let names: Vec<_> = state
            .listed_bars()
            .into_iter()
            .map(|b| b.name.clone())
            .collect();
        assert_eq!(names, [sb::literal("Second"), sb::literal("Other")]);
    }

    #[test]
    fn bossbar_listing_retains_resize_history_after_removals() {
        let mut state = BossState::default();
        state.add_bar("audit:test", sb::literal("Title"));
        state.add_bar("audit:second", sb::literal("Second"));
        for i in 0..11 {
            state.add_bar(&format!("audit:bar{i}"), sb::literal("Extra"));
        }
        for i in 0..11 {
            state.remove_bar(&format!("audit:bar{i}"));
        }
        // After the 13th put, HashMap keeps its 32-slot table. These keys
        // occupy buckets 11 and 24, rather than buckets 11 and 8 at size 16.
        assert_eq!(ids(&state), ["audit:test", "audit:second"]);
        state.remove_bar("audit:test");
        state.remove_bar("audit:second");
        assert!(ids(&state).is_empty());
        state.add_bar("audit:test", sb::literal("Again"));
        state.add_bar("audit:second", sb::literal("Second"));
        assert_eq!(ids(&state), ["audit:test", "audit:second"]);
    }

    #[test]
    fn bossbar_listing_rebuilds_for_the_loaded_store() {
        let mut state = BossState::default();
        state.bars.add("audit:test", sb::literal("Title"));
        state.bars.add("audit:second", sb::literal("Second"));
        state.rebuild_listing();
        assert_eq!(ids(&state), ["audit:second", "audit:test"]);
    }
}
