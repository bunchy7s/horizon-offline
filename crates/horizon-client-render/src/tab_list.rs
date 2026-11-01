//! The player list shown while the player-list key is held
//! (`PlayerTabOverlay`, `Hud.extractTabList`): listed players sorted by list
//! order, spectators last, team and name; columns of at most 20 rows; heads,
//! names (team prefix and colour, italic spectators), ping bars and the
//! `list` objective (integer scores in their number format, or hearts that
//! blink when they change); the header and footer. Laid out as draw calls so
//! the parity fixture can compare them with the vanilla client.
use crate::{
    font::Font,
    hud_layout::{Blend, GuiDraw, WHITE},
    java_text::float_to_string,
    scoreboard::{Comp, NumberFormat, Objective, RenderType, Scoreboard, comp_text},
    text::Text,
};
use std::{borrow::Cow, cmp::Ordering, collections::HashMap};

/// `GameType.SPECTATOR` id.
pub const SPECTATOR: i32 = 3;
/// `PlayerTabOverlay.MAX_ROWS_PER_COL`.
pub const MAX_ROWS_PER_COL: i32 = 20;

/// One `PlayerInfo` the list can show.
#[derive(Clone, Debug, PartialEq)]
pub struct TabPlayer {
    pub uuid: u128,
    /// `GameProfile.name`: the score holder and team member name.
    pub name: String,
    /// `GameType` id (0 survival, 1 creative, 2 adventure, 3 spectator).
    pub game_mode: i32,
    /// Milliseconds; negative when unknown.
    pub latency: i32,
    /// `tabListDisplayName` as a component (its root style matters: a
    /// spectator's name is made italic at the root).
    pub display_name: Option<Comp>,
    pub list_order: i32,
    pub show_hat: bool,
    /// Skin texture path, e.g. `entity/player/wide/steve`.
    pub skin: String,
}
impl TabPlayer {
    /// A player with the default skin for their UUID.
    pub fn new(uuid: u128, name: impl Into<String>) -> Self {
        Self {
            uuid,
            name: name.into(),
            game_mode: 0,
            latency: 0,
            display_name: None,
            list_order: 0,
            show_hat: true,
            skin: default_skin(uuid).to_owned(),
        }
    }
}

/// `DefaultPlayerSkin.get(UUID)`: the default skin by `UUID.hashCode`.
pub fn default_skin(uuid: u128) -> &'static str {
    const SKINS: [&str; 18] = [
        "entity/player/slim/alex",
        "entity/player/slim/ari",
        "entity/player/slim/efe",
        "entity/player/slim/kai",
        "entity/player/slim/makena",
        "entity/player/slim/noor",
        "entity/player/slim/steve",
        "entity/player/slim/sunny",
        "entity/player/slim/zuri",
        "entity/player/wide/alex",
        "entity/player/wide/ari",
        "entity/player/wide/efe",
        "entity/player/wide/kai",
        "entity/player/wide/makena",
        "entity/player/wide/noor",
        "entity/player/wide/steve",
        "entity/player/wide/sunny",
        "entity/player/wide/zuri",
    ];
    let hilo = ((uuid >> 64) as u64) ^ (uuid as u64);
    let hash = ((hilo >> 32) as i32) ^ (hilo as i32);
    SKINS[hash.rem_euclid(SKINS.len() as i32) as usize]
}

/// `blitSprite(sprite, x, y, w, h)`.
fn sprite(name: &'static str, x: i32, y: i32, w: i32, h: i32) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Borrowed(name),
        x,
        y,
        w,
        h,
        region: None,
        color: WHITE,
        blend: crate::hud_layout::Blend::Textured,
    }
}

/// `PlayerFaceExtractor.extractRenderState`: the skin's face, then its hat
/// layer, upside down for Dinnerbone and Grumm.
#[allow(clippy::too_many_arguments)]
pub fn face(
    out: &mut Vec<GuiDraw>,
    skin: &str,
    x: i32,
    y: i32,
    size: i32,
    hat: bool,
    flip: bool,
    color: u32,
) {
    let (v, height) = if flip { (16.0, -8) } else { (8.0, 8) };
    for u in [8.0, 40.0].into_iter().take(if hat { 2 } else { 1 }) {
        out.push(GuiDraw::Blit {
            texture: Cow::Owned(skin.to_owned()),
            blend: Blend::Textured,
            x,
            y,
            w: size,
            h: size,
            uv: [u, v],
            src: [8, height],
            tex: [64, 64],
            color,
        });
    }
}

/// `Hud.extractTabList`: whether the list shows. A single-player world
/// shows it only with another listed player or a `list` objective.
pub fn visible(key_down: bool, local_server: bool, listed: usize, objective: bool) -> bool {
    key_down && !(local_server && listed <= 1 && !objective)
}

/// `Options.getBackgroundColor(553648127)`: the row background.
pub fn row_background(background_for_chat_only: bool, text_background_opacity: f64) -> u32 {
    if background_for_chat_only {
        0x20FF_FFFF
    } else {
        ((text_background_opacity as f32 * 255.0).floor() as i32 as u32 & 0xFF) << 24
    }
}

/// `String.compareTo`: UTF-16 code unit order.
fn compare_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
/// `String.compareToIgnoreCase` (per code point: upper then lower case).
fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    fn simple(c: char, upper: bool) -> char {
        let mut it: Box<dyn Iterator<Item = char>> = if upper {
            Box::new(c.to_uppercase())
        } else {
            Box::new(c.to_lowercase())
        };
        match (it.next(), it.next()) {
            (Some(m), None) => m,
            _ => c,
        }
    }
    let (mut x, mut y) = (a.chars(), b.chars());
    loop {
        match (x.next(), y.next()) {
            (Some(c1), Some(c2)) => {
                if c1 != c2 {
                    let (u1, u2) = (simple(c1, true), simple(c2, true));
                    if u1 != u2 {
                        let (l1, l2) = (simple(u1, false), simple(u2, false));
                        if l1 != l2 {
                            return l1.cmp(&l2);
                        }
                    }
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

/// `PlayerTabOverlay.getPlayerInfos`: `PLAYER_COMPARATOR` order (higher
/// list order first, spectators last, team name, profile name ignoring
/// case), at most 80 players.
pub fn sorted_players<'a>(players: &'a [TabPlayer], board: &Scoreboard) -> Vec<&'a TabPlayer> {
    let team = |p: &TabPlayer| {
        board
            .players_team(&p.name)
            .map_or("", |t| t.name.as_str())
            .to_owned()
    };
    let mut out: Vec<&TabPlayer> = players.iter().collect();
    out.sort_by(|a, b| {
        a.list_order
            .wrapping_neg()
            .cmp(&b.list_order.wrapping_neg())
            .then_with(|| (a.game_mode == SPECTATOR).cmp(&(b.game_mode == SPECTATOR)))
            .then_with(|| compare_utf16(&team(a), &team(b)))
            .then_with(|| compare_ignore_case(&a.name, &b.name))
    });
    out.truncate(80);
    out
}

/// What `setVisible(true)` has the narrator say:
/// `multiplayer.player.list.narration` with the listed players' names
/// (`ComponentUtils.formatList` with ", ").
pub fn narration(players: &[TabPlayer], board: &Scoreboard) -> Text {
    Text::translate(
        "multiplayer.player.list.narration",
        &[listed_names(players, board)],
    )
}

/// The names the list shows, in its order, ", " between them.
fn listed_names(players: &[TabPlayer], board: &Scoreboard) -> Text {
    sorted_players(players, board)
        .into_iter()
        .enumerate()
        .fold(Text::empty(), |names, (i, p)| {
            let names = if i == 0 {
                names
            } else {
                names.append(Text::literal(", "))
            };
            names.append(name_for_display(p, board))
        })
}

/// `MutableComponent.withStyle(ITALIC)` on a component's root.
fn italic_root(c: &Comp) -> Comp {
    use serde_json::Value;
    match c {
        Value::Object(o) => {
            let mut o = o.clone();
            o.insert("italic".into(), Value::Bool(true));
            Value::Object(o)
        }
        // A list is its first element with the rest as its siblings.
        Value::Array(list) if !list.is_empty() => {
            let mut list = list.clone();
            list[0] = italic_root(&list[0]);
            Value::Array(list)
        }
        Value::Array(_) => serde_json::json!({"text": "", "italic": true}),
        other => {
            let text = match other {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                v => v.to_string(),
            };
            serde_json::json!({"text": text, "italic": true})
        }
    }
}

/// `PlayerTabOverlay.getNameForDisplay`.
pub fn name_for_display(p: &TabPlayer, board: &Scoreboard) -> Text {
    let spectator = p.game_mode == SPECTATOR;
    let name = match &p.display_name {
        Some(d) if spectator => return comp_text(&italic_root(d)),
        Some(d) => return comp_text(d),
        None => match board.players_team(&p.name) {
            Some(team) => team.formatted_name(Text::literal(p.name.clone())),
            None => Text::literal(p.name.clone()),
        },
    };
    if spectator { name.italic() } else { name }
}

/// `PlayerTabOverlay.extractPingIcon`'s sprite.
pub fn ping_sprite(latency: i32) -> &'static str {
    match latency {
        i32::MIN..=-1 => "icon/ping_unknown",
        0..150 => "icon/ping_5",
        150..300 => "icon/ping_4",
        300..600 => "icon/ping_3",
        600..1000 => "icon/ping_2",
        _ => "icon/ping_1",
    }
}

/// `PlayerTabOverlay.HealthState`: the hearts of one player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HealthState {
    last_value: i32,
    displayed_value: i32,
    last_update_tick: i64,
    blink_until_tick: i64,
}
impl HealthState {
    fn new(value: i32) -> Self {
        Self {
            last_value: value,
            displayed_value: value,
            last_update_tick: 0,
            blink_until_tick: 0,
        }
    }
    fn update(&mut self, value: i32, tick: i64) {
        if value != self.last_value {
            let duration = if value < self.last_value { 20 } else { 10 };
            self.blink_until_tick = tick + duration;
            self.last_value = value;
            self.last_update_tick = tick;
        }
        if tick - self.last_update_tick > 20 {
            self.displayed_value = value;
        }
    }
    fn is_blinking(&self, tick: i64) -> bool {
        self.blink_until_tick > tick && (self.blink_until_tick - tick) % 6 >= 3
    }
}

/// What the list is drawn from this frame.
pub struct TabInputs<'a> {
    /// The listed players (`getListedOnlinePlayers`), in any order.
    pub players: &'a [TabPlayer],
    pub board: &'a Scoreboard,
    /// The objective in the `list` display slot.
    pub objective: Option<&'a Objective>,
    pub screen_width: i32,
    /// `ClientPacketListener.onlineMode`: heads are shown.
    pub online_mode: bool,
    /// `Hud.getGuiTicks`.
    pub gui_ticks: i64,
    /// [`row_background`].
    pub background: u32,
    /// The player is in the level with an upside-down name
    /// (`AvatarRenderer.isPlayerUpsideDown`).
    pub upside_down: &'a dyn Fn(u128) -> bool,
}

/// `PlayerTabOverlay`: the header, footer, visibility and heart states.
#[derive(Clone, Debug, Default)]
pub struct TabList {
    pub header: Option<Text>,
    pub footer: Option<Text>,
    visible: bool,
    health: HashMap<u128, HealthState>,
}
struct Entry {
    name: Text,
    score: i32,
    formatted: Option<Text>,
    score_width: i32,
}
impl TabList {
    /// `setVisible`: showing or hiding forgets the heart states.
    /// `setVisible`: true when the list has just come up, which vanilla
    /// narrates ([`narration`]).
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible != visible {
            self.health.clear();
            self.visible = visible;
            return visible;
        }
        false
    }
    /// `reset`.
    pub fn reset(&mut self) {
        self.header = None;
        self.footer = None;
    }
    /// `extractRenderState`.
    pub fn layout(&mut self, i: &TabInputs, font: &Font) -> Vec<GuiDraw> {
        let width = |t: &Text| font.width_text(t) as i32;
        let players = sorted_players(i.players, i.board);
        let spacer = font.width(" ") as i32;
        let mut max_name = 0;
        let mut max_score = 0;
        let mut entries = Vec::with_capacity(players.len());
        for p in &players {
            let name = name_for_display(p, i.board).resolve_legacy();
            max_name = max_name.max(width(&name));
            let mut entry = Entry {
                name,
                score: 0,
                formatted: None,
                score_width: 0,
            };
            if let Some(objective) = i.objective {
                let info = i.board.score(&p.name, &objective.name);
                if let Some(s) = info {
                    entry.score = s.value;
                }
                if objective.render_type != RenderType::Hearts {
                    if let Some(s) = info {
                        let default =
                            objective.format_or_default(NumberFormat::player_list_default());
                        let text = s
                            .number_format
                            .as_ref()
                            .unwrap_or(&default)
                            .format(s.value)
                            .resolve_legacy();
                        entry.score_width = width(&text);
                        entry.formatted = Some(text);
                    }
                    let w = if entry.score_width > 0 {
                        spacer + entry.score_width
                    } else {
                        0
                    };
                    max_score = max_score.max(w);
                }
            }
            entries.push(entry);
        }
        if !self.health.is_empty() {
            self.health
                .retain(|id, _| players.iter().any(|p| p.uuid == *id));
        }
        let slots = players.len() as i32;
        let mut rows = slots;
        let mut cols = 1;
        while rows > MAX_ROWS_PER_COL {
            cols += 1;
            rows = (slots + cols - 1) / cols;
        }
        let show_head = i.online_mode;
        let width_for_score = match i.objective {
            Some(o) if o.render_type == RenderType::Hearts => 90,
            Some(_) => max_score,
            None => 0,
        };
        let sw = i.screen_width;
        let head = if show_head { 9 } else { 0 };
        let slot_width = (cols * (head + max_name + width_for_score + 13)).min(sw - 50) / cols;
        let xxo = sw / 2 - (slot_width * cols + (cols - 1) * 5) / 2;
        let mut yyo = 10;
        let mut max_line = slot_width * cols + (cols - 1) * 5;
        let split = |t: &Option<Text>| {
            t.as_ref().map(|t| {
                font.split_lines(&t.resolve_legacy(), (sw - 50) as f32)
                    .into_iter()
                    .map(|(line, _)| line)
                    .collect::<Vec<_>>()
            })
        };
        let header = split(&self.header);
        let footer = split(&self.footer);
        for line in header.iter().chain(footer.iter()).flatten() {
            max_line = max_line.max(width(line));
        }
        let mut out = Vec::new();
        let band = |y0: i32, y1: i32| GuiDraw::Fill {
            x0: sw / 2 - max_line / 2 - 1,
            y0,
            x1: sw / 2 + max_line / 2 + 1,
            y1,
            color: 0x8000_0000,
        };
        if let Some(lines) = &header {
            out.push(band(yyo - 1, yyo + lines.len() as i32 * 9));
            for line in lines {
                let w = width(line);
                out.push(GuiDraw::Text {
                    text: line.clone(),
                    x: sw / 2 - w / 2,
                    y: yyo,
                    color: 0xFFFF_FFFF,
                    shadow: true,
                });
                yyo += 9;
            }
            yyo += 1;
        }
        out.push(band(yyo - 1, yyo + rows * 9));
        for (n, (p, entry)) in players.iter().zip(&entries).enumerate() {
            let n = n as i32;
            let (col, row) = (n / rows, n % rows);
            let mut xo = xxo + col * slot_width + col * 5;
            let yo = yyo + row * 9;
            out.push(GuiDraw::Fill {
                x0: xo,
                y0: yo,
                x1: xo + slot_width,
                y1: yo + 8,
                color: i.background,
            });
            if show_head {
                face(
                    &mut out,
                    &p.skin,
                    xo,
                    yo,
                    8,
                    p.show_hat,
                    (i.upside_down)(p.uuid),
                    WHITE,
                );
                xo += 9;
            }
            let spectator = p.game_mode == SPECTATOR;
            out.push(GuiDraw::Text {
                text: entry.name.clone(),
                x: xo,
                y: yo,
                color: if spectator { 0x90FF_FFFF } else { 0xFFFF_FFFF },
                shadow: true,
            });
            if let Some(objective) = i.objective.filter(|_| !spectator) {
                let left = xo + max_name + 1;
                let right = left + width_for_score;
                if right - left > 5 {
                    if objective.render_type == RenderType::Hearts {
                        self.hearts(
                            yo,
                            left,
                            right,
                            p.uuid,
                            entry.score,
                            i.gui_ticks,
                            font,
                            &mut out,
                        );
                    } else if let Some(text) = &entry.formatted {
                        out.push(GuiDraw::Text {
                            text: text.clone(),
                            x: right - entry.score_width,
                            y: yo,
                            color: 0xFFFF_FFFF,
                            shadow: true,
                        });
                    }
                }
            }
            out.push(sprite(
                ping_sprite(p.latency),
                xo - head + slot_width - 11,
                yo,
                10,
                8,
            ));
        }
        if let Some(lines) = &footer {
            yyo += rows * 9 + 1;
            out.push(band(yyo - 1, yyo + lines.len() as i32 * 9));
            for line in lines {
                let w = width(line);
                out.push(GuiDraw::Text {
                    text: line.clone(),
                    x: sw / 2 - w / 2,
                    y: yyo,
                    color: 0xFFFF_FFFF,
                    shadow: true,
                });
                yyo += 9;
            }
        }
        out
    }
    /// `extractTablistHearts`.
    #[allow(clippy::too_many_arguments)]
    fn hearts(
        &mut self,
        yo: i32,
        left: i32,
        right: i32,
        id: u128,
        score: i32,
        tick: i64,
        font: &Font,
        out: &mut Vec<GuiDraw>,
    ) {
        let health = self
            .health
            .entry(id)
            .or_insert_with(|| HealthState::new(score));
        health.update(score, tick);
        let displayed = health.displayed_value;
        let blink = health.is_blinking(tick);
        // Mth.positiveCeilDiv.
        let full_hearts = -((-score.max(displayed)).div_euclid(2));
        let to_render = score.max(displayed.max(20)) / 2;
        if full_hearts <= 0 {
            return;
        }
        let per_heart = ((right - left - 4) as f32 / to_render as f32)
            .min(9.0)
            .floor() as i32;
        if per_heart <= 3 {
            let pct = (score as f32 / 20.0).clamp(0.0, 1.0);
            let color = (((1.0 - pct) * 255.0) as i32) << 16 | ((pct * 255.0) as i32) << 8;
            let hearts = score as f32 / 2.0;
            let hp = Text::translate(
                "multiplayer.player.list.hp",
                &[Text::literal(float_to_string(hearts))],
            );
            let text = if right - font.width_text(&hp) as i32 >= left {
                hp
            } else {
                Text::literal(float_to_string(hearts))
            };
            let w = font.width_text(&text) as i32;
            out.push(GuiDraw::Text {
                text,
                x: (right + left - w) / 2,
                y: yo,
                color: color as u32 | 0xFF00_0000,
                shadow: true,
            });
            return;
        }
        let container = if blink {
            "hud/heart/container_blinking"
        } else {
            "hud/heart/container"
        };
        let heart = |name: &'static str, n: i32| sprite(name, left + n * per_heart, yo, 9, 9);
        for n in full_hearts..to_render {
            out.push(heart(container, n));
        }
        for n in 0..full_hearts {
            out.push(heart(container, n));
            if blink {
                if n * 2 + 1 < displayed {
                    out.push(heart("hud/heart/full_blinking", n));
                }
                if n * 2 + 1 == displayed {
                    out.push(heart("hud/heart/half_blinking", n));
                }
            }
            if n * 2 + 1 < score {
                out.push(heart(
                    if n >= 10 {
                        "hud/heart/absorbing_full_blinking"
                    } else {
                        "hud/heart/full"
                    },
                    n,
                ));
            }
            if n * 2 + 1 == score {
                out.push(heart(
                    if n >= 10 {
                        "hud/heart/absorbing_half_blinking"
                    } else {
                        "hud/heart/half"
                    },
                    n,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_narrated_once_as_it_comes_up_with_its_names_in_order() {
        let board = Scoreboard::default();
        let mut alice = TabPlayer::new(2, "Alice");
        alice.list_order = -1;
        let players = [TabPlayer::new(1, "bob"), alice];
        assert_eq!(listed_names(&players, &board).plain(), "bob, Alice");
        let mut list = TabList::default();
        assert!(list.set_visible(true));
        assert!(!list.set_visible(true), "only as it comes up");
        assert!(!list.set_visible(false));
        assert!(list.set_visible(true));
    }

    #[test]
    fn order_ping_and_skins() {
        let board = Scoreboard::default();
        let mut a = TabPlayer::new(1, "bob");
        let mut b = TabPlayer::new(2, "Alice");
        let c = TabPlayer::new(3, "carl");
        a.game_mode = SPECTATOR;
        b.list_order = -1;
        let players = [a, b, c];
        let names: Vec<_> = sorted_players(&players, &board)
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["carl", "bob", "Alice"]);
        assert_eq!(ping_sprite(-1), "icon/ping_unknown");
        assert_eq!(ping_sprite(149), "icon/ping_5");
        assert_eq!(ping_sprite(1000), "icon/ping_1");
        // UUID.hashCode of 0 is 0: the first default skin.
        assert_eq!(default_skin(0), "entity/player/slim/alex");
        assert!(!visible(true, true, 1, false));
        assert!(visible(true, true, 1, true));
        assert!(visible(true, false, 1, false));
    }
}
