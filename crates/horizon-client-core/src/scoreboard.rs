//! The scoreboard (`Scoreboard`, `Objective`, `Score`, `PlayerTeam`,
//! `ObjectiveCriteria`, `NumberFormat`) and its saved data
//! (`data/minecraft/scoreboard.dat`, `ScoreboardSaveData.Packed`).
use crate::{
    component,
    fastutil_order::{HashSetOrder, OpenOrder},
    registries::Registries,
    text::{self, Style, Text},
};
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::collections::BTreeMap;

/// A text component in its JSON form (what `ComponentSerialization` holds).
pub type Comp = Value;

pub fn literal(s: &str) -> Comp {
    Value::String(s.to_owned())
}
pub fn comp_text(c: &Comp) -> Text {
    component::to_text(c, &mut |s| Text::literal(s))
}
/// `ComponentUtils.wrapInSquareBrackets`.
pub fn bracketed(inner: Text) -> Text {
    Text::translate("chat.square_brackets", &[inner])
}
fn hover_name(text: Text, name: &str) -> Text {
    text.with_style(&Style {
        hover: Some(Box::new(Text::literal(name))),
        ..Style::default()
    })
}

// ---- NBT <-> JSON ------------------------------------------------------

pub fn json_to_tag(v: &Value) -> Tag {
    match v {
        Value::Null => Tag::String("".into()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i32::try_from(i).map_or(Tag::Long(i), Tag::Int),
            None => Tag::Double(n.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_to_tag).collect()),
        Value::Object(o) => Tag::Compound(
            o.iter()
                .map(|(k, v)| (k.as_str().into(), json_to_tag(v)))
                .collect(),
        ),
    }
}
pub fn tag_to_json(t: &Tag) -> Value {
    match t {
        Tag::End => Value::Null,
        Tag::Byte(b) => Value::Bool(*b != 0),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => Value::from(f64::from(f32::from_bits(*v))),
        Tag::Double(v) => Value::from(f64::from_bits(*v)),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(l) => Value::Array(l.iter().map(tag_to_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_to_json(v)))
                .collect(),
        ),
        Tag::ByteArray(b) => Value::Array(b.iter().map(|v| Value::from(*v)).collect()),
        Tag::IntArray(b) => Value::Array(b.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(b) => Value::Array(b.iter().map(|v| Value::from(*v)).collect()),
    }
}
fn string_of(t: Option<&Tag>) -> Option<String> {
    match t {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
fn bool_of(t: Option<&Tag>, default: bool) -> bool {
    match t {
        Some(Tag::Byte(b)) => *b != 0,
        _ => default,
    }
}
fn int_of(t: Option<&Tag>) -> Option<i32> {
    match t {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Short(v)) => Some(i32::from(*v)),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    }
}

// ---- number formats ----------------------------------------------------

/// `NumberFormat`: how a score value is shown.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberFormat {
    Blank,
    Fixed(Comp),
    /// A style object (`{color: "red", bold: true}`).
    Styled(Comp),
}
/// A `Style` from its JSON object form.
pub fn style_from_json(v: &Value) -> Style {
    let mut s = Style::default();
    let Some(o) = v.as_object() else { return s };
    if let Some(c) = o.get("color").and_then(Value::as_str) {
        s.color = text::named_color(c).or_else(|| {
            c.strip_prefix('#')
                .and_then(|h| u32::from_str_radix(h, 16).ok())
        });
    }
    let flag = |k: &str| o.get(k).and_then(Value::as_bool);
    s.bold = flag("bold");
    s.italic = flag("italic");
    s.underlined = flag("underlined");
    s.strikethrough = flag("strikethrough");
    s.obfuscated = flag("obfuscated");
    s.font = o.get("font").and_then(text::font_id);
    s
}
impl NumberFormat {
    pub fn sidebar_default() -> Self {
        NumberFormat::Styled(serde_json::json!({"color": "red"}))
    }
    pub fn player_list_default() -> Self {
        NumberFormat::Styled(serde_json::json!({"color": "yellow"}))
    }
    pub fn format(&self, value: i32) -> Text {
        match self {
            NumberFormat::Blank => Text::empty(),
            NumberFormat::Fixed(c) => comp_text(c),
            NumberFormat::Styled(style) => {
                Text::literal(value.to_string()).with_style(&style_from_json(style))
            }
        }
    }
    /// `NumberFormatTypes.CODEC` encoding.
    pub fn to_json(&self) -> Value {
        match self {
            NumberFormat::Blank => serde_json::json!({"type": "minecraft:blank"}),
            NumberFormat::Fixed(c) => {
                serde_json::json!({"type": "minecraft:fixed", "value": canonical_component(c)})
            }
            NumberFormat::Styled(style) => {
                let mut v = canonical_style(style);
                if let Value::Object(o) = &mut v {
                    o.insert("type".into(), Value::String("minecraft:styled".into()));
                }
                v
            }
        }
    }
    /// `NumberFormatTypes.CODEC` decoding.
    pub fn from_json(v: &Value) -> Option<Self> {
        let kind = v.get("type")?.as_str()?;
        match kind.trim_start_matches("minecraft:") {
            "blank" => Some(NumberFormat::Blank),
            "fixed" => Some(NumberFormat::Fixed(v.get("value")?.clone())),
            "styled" => {
                let mut v = v.clone();
                if let Value::Object(o) = &mut v {
                    o.remove("type");
                }
                Some(NumberFormat::Styled(v))
            }
            _ => None,
        }
    }
    pub fn to_tag(&self) -> Tag {
        let mut tag = json_to_tag(&self.to_json());
        entity_uuids(&mut tag);
        tag
    }
    pub fn from_tag(t: &Tag) -> Option<Self> {
        Self::from_json(&tag_to_json(t))
    }
}

// ---- criteria ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderType {
    Integer,
    Hearts,
}
impl RenderType {
    pub fn id(self) -> &'static str {
        match self {
            RenderType::Integer => "integer",
            RenderType::Hearts => "hearts",
        }
    }
    pub fn by_id(id: &str) -> Self {
        if id == "hearts" {
            RenderType::Hearts
        } else {
            RenderType::Integer
        }
    }
}
pub const CRITERIA_DUMMY: &str = "dummy";
pub const CRITERIA_TRIGGER: &str = "trigger";
pub const CRITERIA_DEATH_COUNT: &str = "deathCount";
pub const CRITERIA_PLAYER_KILLS: &str = "playerKillCount";
pub const CRITERIA_TOTAL_KILLS: &str = "totalKillCount";
pub const CRITERIA_HEALTH: &str = "health";
pub const CRITERIA_FOOD: &str = "food";
pub const CRITERIA_AIR: &str = "air";
pub const CRITERIA_ARMOR: &str = "armor";
pub const CRITERIA_XP: &str = "xp";
pub const CRITERIA_LEVEL: &str = "level";
/// `ObjectiveCriteria.getCustomCriteriaNames` (without the per-colour ones).
pub const CUSTOM_CRITERIA: [&str; 11] = [
    CRITERIA_DUMMY,
    CRITERIA_TRIGGER,
    CRITERIA_DEATH_COUNT,
    CRITERIA_PLAYER_KILLS,
    CRITERIA_TOTAL_KILLS,
    CRITERIA_HEALTH,
    CRITERIA_FOOD,
    CRITERIA_AIR,
    CRITERIA_ARMOR,
    CRITERIA_XP,
    CRITERIA_LEVEL,
];
/// `Stats` custom statistic ids.
pub const CUSTOM_STATS: &[&str] = &[
    "leave_game",
    "play_time",
    "total_world_time",
    "time_since_death",
    "time_since_rest",
    "sneak_time",
    "walk_one_cm",
    "crouch_one_cm",
    "sprint_one_cm",
    "walk_on_water_one_cm",
    "fall_one_cm",
    "climb_one_cm",
    "fly_one_cm",
    "walk_under_water_one_cm",
    "minecart_one_cm",
    "boat_one_cm",
    "pig_one_cm",
    "happy_ghast_one_cm",
    "horse_one_cm",
    "aviate_one_cm",
    "swim_one_cm",
    "strider_one_cm",
    "nautilus_one_cm",
    "jump",
    "drop",
    "damage_dealt",
    "damage_dealt_absorbed",
    "damage_dealt_resisted",
    "damage_taken",
    "damage_blocked_by_shield",
    "damage_absorbed",
    "damage_resisted",
    "deaths",
    "mob_kills",
    "animals_bred",
    "player_kills",
    "fish_caught",
    "talked_to_villager",
    "traded_with_villager",
    "eat_cake_slice",
    "fill_cauldron",
    "use_cauldron",
    "clean_armor",
    "clean_banner",
    "clean_shulker_box",
    "interact_with_brewingstand",
    "interact_with_beacon",
    "inspect_dropper",
    "inspect_hopper",
    "inspect_dispenser",
    "play_noteblock",
    "tune_noteblock",
    "pot_flower",
    "trigger_trapped_chest",
    "open_enderchest",
    "enchant_item",
    "play_record",
    "interact_with_furnace",
    "interact_with_crafting_table",
    "open_chest",
    "sleep_in_bed",
    "sleep_in_straw_bed",
    "open_shulker_box",
    "open_barrel",
    "interact_with_blast_furnace",
    "interact_with_smoker",
    "interact_with_lectern",
    "interact_with_campfire",
    "interact_with_cartography_table",
    "interact_with_loom",
    "interact_with_stonecutter",
    "bell_ring",
    "raid_trigger",
    "raid_win",
    "interact_with_anvil",
    "interact_with_grindstone",
    "target_hit",
    "interact_with_smithing_table",
];
pub const STAT_TYPES: &[&str] = &[
    "mined",
    "crafted",
    "used",
    "broken",
    "picked_up",
    "dropped",
    "killed",
    "killed_by",
    "custom",
];

/// The 16 team colours (`TeamColor`).
pub fn team_colors() -> impl Iterator<Item = &'static str> {
    text::COLORS.iter().map(|c| c.1)
}
pub fn team_color_rgb(name: &str) -> Option<u32> {
    text::named_color(name)
}
fn is_team_criteria(name: &str) -> bool {
    ["teamkill.", "killedByTeam."].iter().any(|p| {
        name.strip_prefix(p)
            .is_some_and(|c| team_colors().any(|t| t == c))
    })
}
/// `Identifier.bySeparator`: `None` for characters an identifier cannot hold
/// (vanilla throws).
fn identifier_by_separator(id: &str, sep: char) -> Option<String> {
    let (ns, path) = match id.find(sep) {
        Some(0) => ("minecraft", &id[1..]),
        Some(i) => (&id[..i], &id[i + 1..]),
        None => ("minecraft", id),
    };
    let ns_ok = ns
        .chars()
        .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.'));
    let path_ok = path
        .chars()
        .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'));
    (ns_ok && path_ok).then(|| format!("{ns}:{path}"))
}
/// An identifier vanilla's `Identifier` constructor rejects (it throws).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidIdentifier;
/// `ObjectiveCriteria.byName`: the criteria's canonical name (statistics are
/// named `minecraft.<type>:<namespace>.<path>` whatever the spelling used),
/// `Ok(None)` for no such criteria and `Err` where vanilla throws on an
/// invalid identifier.
pub fn criteria_by_name(name: &str, reg: &Registries) -> Result<Option<String>, InvalidIdentifier> {
    if CUSTOM_CRITERIA.contains(&name) || is_team_criteria(name) {
        return Ok(Some(name.to_owned()));
    }
    let Some(colon) = name.find(':') else {
        return Ok(None);
    };
    let ty = identifier_by_separator(&name[..colon], '.').ok_or(InvalidIdentifier)?;
    let Some(ty) = ty
        .strip_prefix("minecraft:")
        .filter(|t| STAT_TYPES.contains(t))
    else {
        return Ok(None);
    };
    let key = identifier_by_separator(&name[colon + 1..], '.').ok_or(InvalidIdentifier)?;
    let known = match ty {
        "mined" => reg.blocks.is_empty() || reg.blocks.contains_key(&key),
        "crafted" | "used" | "broken" | "picked_up" | "dropped" => {
            reg.items.is_empty() || reg.items.contains(&key)
        }
        "killed" | "killed_by" => reg.entity_types.is_empty() || reg.entity_types.contains(&key),
        "custom" => key
            .strip_prefix("minecraft:")
            .is_some_and(|k| CUSTOM_STATS.contains(&k)),
        _ => false,
    };
    Ok(known.then(|| stat_criteria(ty, &key)))
}
/// Whether `ObjectiveCriteria.byName` finds the criteria.
pub fn criteria_exists(name: &str, reg: &Registries) -> bool {
    matches!(criteria_by_name(name, reg), Ok(Some(_)))
}
/// `ObjectiveCriteria.isReadOnly`.
pub fn criteria_read_only(name: &str) -> bool {
    matches!(
        name,
        CRITERIA_HEALTH
            | CRITERIA_FOOD
            | CRITERIA_AIR
            | CRITERIA_ARMOR
            | CRITERIA_XP
            | CRITERIA_LEVEL
    )
}
pub fn criteria_render_type(name: &str) -> RenderType {
    if name == CRITERIA_HEALTH {
        RenderType::Hearts
    } else {
        RenderType::Integer
    }
}
/// `Stat.buildName`: the criteria that counts a statistic.
pub fn stat_criteria(stat_type: &str, key: &str) -> String {
    format!(
        "minecraft.{}:{}",
        stat_type.trim_start_matches("minecraft:"),
        key.replace(':', ".")
    )
}

// ---- display slots -----------------------------------------------------

pub const SLOT_LIST: &str = "list";
pub const SLOT_SIDEBAR: &str = "sidebar";
pub const SLOT_BELOW_NAME: &str = "below_name";
/// Every `DisplaySlot` name, in id order.
pub fn display_slots() -> Vec<String> {
    let mut v: Vec<String> = ["list", "sidebar", "below_name"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    v.extend(team_colors().map(|c| format!("sidebar.team.{c}")));
    v
}
pub fn display_slot_valid(name: &str) -> bool {
    display_slots().iter().any(|s| s == name)
}

// ---- teams ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Always,
    Never,
    HideForOtherTeams,
    HideForOwnTeam,
}
impl Visibility {
    pub const ALL: [Visibility; 4] = [
        Visibility::Always,
        Visibility::Never,
        Visibility::HideForOtherTeams,
        Visibility::HideForOwnTeam,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Visibility::Always => "always",
            Visibility::Never => "never",
            Visibility::HideForOtherTeams => "hideForOtherTeams",
            Visibility::HideForOwnTeam => "hideForOwnTeam",
        }
    }
    pub fn by_name(n: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == n)
    }
    pub fn display_name(self) -> Text {
        Text::translate(&format!("team.visibility.{}", self.name()), &[])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionRule {
    Always,
    Never,
    PushOtherTeams,
    PushOwnTeam,
}
impl CollisionRule {
    pub const ALL: [CollisionRule; 4] = [
        CollisionRule::Always,
        CollisionRule::Never,
        CollisionRule::PushOtherTeams,
        CollisionRule::PushOwnTeam,
    ];
    pub fn name(self) -> &'static str {
        match self {
            CollisionRule::Always => "always",
            CollisionRule::Never => "never",
            CollisionRule::PushOtherTeams => "pushOtherTeams",
            CollisionRule::PushOwnTeam => "pushOwnTeam",
        }
    }
    pub fn by_name(n: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == n)
    }
    pub fn display_name(self) -> Text {
        Text::translate(&format!("team.collision.{}", self.name()), &[])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub name: String,
    pub display_name: Comp,
    pub color: Option<String>,
    pub allow_friendly_fire: bool,
    pub see_friendly_invisibles: bool,
    pub prefix: Comp,
    pub suffix: Comp,
    pub nametag_visibility: Visibility,
    pub death_message_visibility: Visibility,
    pub collision_rule: CollisionRule,
    /// `Sets.newHashSet()`, in its iteration order.
    pub players: HashSetOrder,
}
impl Team {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            display_name: literal(name),
            color: None,
            allow_friendly_fire: true,
            see_friendly_invisibles: true,
            prefix: literal(""),
            suffix: literal(""),
            nametag_visibility: Visibility::Always,
            death_message_visibility: Visibility::Always,
            collision_rule: CollisionRule::Always,
            players: HashSetOrder::default(),
        }
    }
    fn apply_color(&self, t: Text) -> Text {
        match self.color.as_deref().and_then(team_color_rgb) {
            Some(c) => t.with_color(c),
            None => t,
        }
    }
    /// `PlayerTeam.getFormattedDisplayName`.
    pub fn formatted_display_name(&self) -> Text {
        let inner = hover_name(comp_text(&self.display_name), &self.name).with_style(&Style {
            click: None,
            ..Style::default()
        });
        self.apply_color(bracketed(inner))
    }
    /// `PlayerTeam.getFormattedName`: prefix, name and suffix in the team colour.
    pub fn formatted_name(&self, member: Text) -> Text {
        let mut t = Text::empty();
        t.push(comp_text(&self.prefix));
        t.push(member);
        t.push(comp_text(&self.suffix));
        self.apply_color(t)
    }
    /// `PlayerTeam.getFormattedName` as a component (`Component.empty()
    /// .append(prefix).append(name).append(suffix)` with the team colour).
    pub fn formatted_name_comp(&self, member: &Comp) -> Comp {
        let mut c = serde_json::json!({
            "text": "",
            "extra": [self.prefix.clone(), member.clone(), self.suffix.clone()],
        });
        if let Some(color) = &self.color {
            c["color"] = Value::String(color.clone());
        }
        c
    }
}

// ---- team rules in gameplay ------------------------------------------------

/// `Team.isAlliedTo`: both on the same team.
pub fn teams_allied(own: Option<&Team>, other: Option<&Team>) -> bool {
    matches!((own, other), (Some(a), Some(b)) if a.name == b.name)
}
/// `Entity.doTeamsAllowDamage`: an entity may hurt another unless they are
/// allies on a team without friendly fire (also `Player.canHarmPlayer`).
pub fn teams_allow_damage(own: Option<&Team>, other: Option<&Team>) -> bool {
    match own {
        None => true,
        Some(t) => !teams_allied(own, other) || t.allow_friendly_fire,
    }
}
/// `EntitySelector.pushableBy(entity).test(input)` on the server (or for the
/// local player on the client): whether `entity` pushes `input`, which must
/// be pushable and not a spectator.
pub fn pushable_by(own: Option<&Team>, their: Option<&Team>, their_pushable: bool) -> bool {
    let own_rule = own.map_or(CollisionRule::Always, |t| t.collision_rule);
    if own_rule == CollisionRule::Never || !their_pushable {
        return false;
    }
    let their_rule = their.map_or(CollisionRule::Always, |t| t.collision_rule);
    if their_rule == CollisionRule::Never {
        return false;
    }
    let same = teams_allied(own, their);
    if (own_rule == CollisionRule::PushOwnTeam || their_rule == CollisionRule::PushOwnTeam) && same
    {
        return false;
    }
    own_rule != CollisionRule::PushOtherTeams && their_rule != CollisionRule::PushOtherTeams || same
}
/// `Entity.isInvisibleTo(player)`: invisible unless the viewer spectates or
/// shares a team that sees friendly invisibles.
pub fn invisible_to(
    own: Option<&Team>,
    viewer: Option<&Team>,
    viewer_spectator: bool,
    invisible: bool,
) -> bool {
    if viewer_spectator {
        return false;
    }
    match own {
        Some(t) if teams_allied(own, viewer) && t.see_friendly_invisibles => false,
        _ => invisible,
    }
}
/// The team part of `LivingEntityRenderer.shouldShowName` for an entity
/// other than the viewer: `Some(shown)` when the entity's team decides,
/// `None` when it has no team.
pub fn team_shows_name(
    own: Option<&Team>,
    viewer: Option<&Team>,
    visible_to_viewer: bool,
) -> Option<bool> {
    let t = own?;
    Some(match t.nametag_visibility {
        Visibility::Always => visible_to_viewer,
        Visibility::Never => false,
        Visibility::HideForOtherTeams => match viewer {
            None => visible_to_viewer,
            Some(_) => {
                teams_allied(own, viewer) && (t.see_friendly_invisibles || visible_to_viewer)
            }
        },
        Visibility::HideForOwnTeam => match viewer {
            None => visible_to_viewer,
            Some(_) => !teams_allied(own, viewer) && visible_to_viewer,
        },
    })
}
/// What `LivingEntityRenderer.shouldShowName` asks of a living entity other
/// than the viewer.
#[derive(Clone, Copy, Debug, Default)]
pub struct NameView {
    /// Crouching (`isDiscrete`): its name hides beyond 32 blocks.
    pub discrete: bool,
    pub distance_sqr: f64,
    /// `isInvisibleTo(viewer)`.
    pub invisible_to_viewer: bool,
    /// F1 (`Hud.isHidden`).
    pub hud_hidden: bool,
    /// The camera is this entity.
    pub is_camera: bool,
    /// Something rides it.
    pub is_vehicle: bool,
}
/// `LivingEntityRenderer.shouldShowName` (before a mob renderer's own check
/// for a visible or targeted custom name).
pub fn name_shown(own: Option<&Team>, viewer: Option<&Team>, v: NameView) -> bool {
    if v.discrete && v.distance_sqr >= 1024.0 {
        return false;
    }
    let visible = !v.invisible_to_viewer;
    if let Some(shown) = team_shows_name(own, viewer, visible) {
        return shown;
    }
    !v.hud_hidden && !v.is_camera && visible && !v.is_vehicle
}
/// `ServerPlayer.die`: whether `other` hears the death message of `dying`
/// (`broadcastSystemMessage`, `broadcastSystemToTeam` or
/// `broadcastSystemToAllExceptTeam`, by the team's death message visibility).
pub fn death_message_reaches(board: &Scoreboard, dying: &str, other: &str) -> bool {
    let Some(team) = board.players_team(dying) else {
        return true;
    };
    let same_team = board
        .players_team(other)
        .is_some_and(|t| t.name == team.name);
    match team.death_message_visibility {
        Visibility::Always => true,
        Visibility::Never => false,
        Visibility::HideForOtherTeams => same_team && other != dying,
        Visibility::HideForOwnTeam => !same_team,
    }
}

// ---- objectives and scores ---------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub name: String,
    pub criteria: String,
    pub display_name: Comp,
    pub render_type: RenderType,
    pub display_auto_update: bool,
    pub number_format: Option<NumberFormat>,
}
impl Objective {
    /// `Objective.getFormattedDisplayName`.
    pub fn formatted_display_name(&self) -> Text {
        bracketed(hover_name(comp_text(&self.display_name), &self.name))
    }
    pub fn display_text(&self) -> Text {
        comp_text(&self.display_name)
    }
    pub fn format_or_default(&self, default: NumberFormat) -> NumberFormat {
        self.number_format.clone().unwrap_or(default)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub value: i32,
    pub locked: bool,
    pub display: Option<Comp>,
    pub number_format: Option<NumberFormat>,
}
impl Default for Score {
    fn default() -> Self {
        Self {
            value: 0,
            locked: true,
            display: None,
            number_format: None,
        }
    }
}
/// One row of an objective (`PlayerScoreEntry`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreEntry {
    pub owner: String,
    pub value: i32,
    pub display: Option<Comp>,
    pub number_format: Option<NumberFormat>,
}
impl ScoreEntry {
    pub fn is_hidden(&self) -> bool {
        self.owner.starts_with('#')
    }
    pub fn owner_name(&self) -> Text {
        match &self.display {
            Some(d) => comp_text(d),
            None => Text::literal(self.owner.clone()),
        }
    }
    pub fn format_value(&self, default: &NumberFormat) -> Text {
        self.number_format
            .as_ref()
            .unwrap_or(default)
            .format(self.value)
    }
}

/// `Scoreboard`: objectives, scores, display slots and teams, kept in the
/// iteration orders of vanilla's hash maps.
#[derive(Clone, Debug, PartialEq)]
pub struct Scoreboard {
    /// `objectivesByName` in its iterator order (`getObjectives()`).
    pub objectives: Vec<Objective>,
    objective_order: OpenOrder,
    /// `objectivesByCriteria`: each criteria's objectives in creation order.
    by_criteria: BTreeMap<String, Vec<String>>,
    /// `playerScores`: each holder's scores by objective. A holder stays
    /// tracked when its last objective is removed.
    pub scores: BTreeMap<String, BTreeMap<String, Score>>,
    holder_order: OpenOrder,
    pub display: BTreeMap<String, String>,
    /// `teamsByName` in its iterator order (`getPlayerTeams()`).
    pub teams: Vec<Team>,
    team_order: OpenOrder,
    team_of: BTreeMap<String, String>,
}
impl Default for Scoreboard {
    fn default() -> Self {
        Self {
            objectives: Vec::new(),
            objective_order: OpenOrder::new(16, 0.5),
            by_criteria: BTreeMap::new(),
            scores: BTreeMap::new(),
            holder_order: OpenOrder::new(16, 0.5),
            display: BTreeMap::new(),
            teams: Vec::new(),
            team_order: OpenOrder::default(),
            team_of: BTreeMap::new(),
        }
    }
}
fn ranked<T>(items: &mut [T], order: &OpenOrder, name: impl Fn(&T) -> &str) {
    let rank: BTreeMap<&str, usize> = order.iter().enumerate().map(|(i, k)| (k, i)).collect();
    items.sort_by_key(|t| rank.get(name(t)).copied().unwrap_or(usize::MAX));
}
impl Scoreboard {
    fn reorder(&mut self) {
        ranked(&mut self.objectives, &self.objective_order, |o| &o.name);
        ranked(&mut self.teams, &self.team_order, |t| &t.name);
    }
    pub fn objective(&self, name: &str) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.name == name)
    }
    pub fn objective_mut(&mut self, name: &str) -> Option<&mut Objective> {
        self.objectives.iter_mut().find(|o| o.name == name)
    }
    /// `getObjectives().stream()` (what saving uses).
    pub fn objectives_streamed(&self) -> Vec<&Objective> {
        self.objective_order
            .stream()
            .filter_map(|n| self.objective(n))
            .collect()
    }
    pub fn add_objective(
        &mut self,
        name: &str,
        criteria: &str,
        display_name: Comp,
        render_type: RenderType,
        display_auto_update: bool,
        number_format: Option<NumberFormat>,
    ) -> bool {
        if self.objective(name).is_some() {
            return false;
        }
        self.objectives.push(Objective {
            name: name.to_owned(),
            criteria: criteria.to_owned(),
            display_name,
            render_type,
            display_auto_update,
            number_format,
        });
        self.objective_order.insert(name);
        self.by_criteria
            .entry(criteria.to_owned())
            .or_default()
            .push(name.to_owned());
        self.reorder();
        true
    }
    /// `removeObjective`: holders left without scores stay tracked.
    pub fn remove_objective(&mut self, name: &str) {
        let Some(criteria) = self.objective(name).map(|o| o.criteria.clone()) else {
            return;
        };
        self.objectives.retain(|o| o.name != name);
        self.objective_order.remove(name);
        // Removing shifts the keys after it back.
        self.reorder();
        self.display.retain(|_, o| o != name);
        if let Some(list) = self.by_criteria.get_mut(&criteria) {
            list.retain(|n| n != name);
        }
        for s in self.scores.values_mut() {
            s.remove(name);
        }
    }
    pub fn display_objective(&self, slot: &str) -> Option<&Objective> {
        self.objective(self.display.get(slot)?)
    }
    pub fn set_display_objective(&mut self, slot: &str, objective: Option<&str>) {
        match objective {
            Some(o) => {
                self.display.insert(slot.to_owned(), o.to_owned());
            }
            None => {
                self.display.remove(slot);
            }
        }
    }
    pub fn score(&self, holder: &str, objective: &str) -> Option<&Score> {
        self.scores.get(holder)?.get(objective)
    }
    /// `getOrCreatePlayerScore`: a new score starts locked at zero.
    pub fn score_mut(&mut self, holder: &str, objective: &str) -> &mut Score {
        self.holder_order.insert(holder);
        self.scores
            .entry(holder.to_owned())
            .or_default()
            .entry(objective.to_owned())
            .or_default()
    }
    pub fn get(&mut self, holder: &str, objective: &str) -> i32 {
        self.score_mut(holder, objective).value
    }
    /// `ScoreAccess.set`; `display` is the holder's display name, which an
    /// objective that updates its display copies to the score.
    pub fn set_with(&mut self, holder: &str, objective: &str, value: i32, display: Option<&Comp>) {
        let auto = self
            .objective(objective)
            .is_some_and(|o| o.display_auto_update);
        let s = self.score_mut(holder, objective);
        if auto
            && let Some(d) = display
            && s.display.as_ref().map(canonical_component) != Some(canonical_component(d))
        {
            s.display = Some(d.clone());
        }
        s.value = value;
    }
    pub fn set(&mut self, holder: &str, objective: &str, value: i32) {
        self.set_with(holder, objective, value, None);
    }
    pub fn add(&mut self, holder: &str, objective: &str, amount: i32) -> i32 {
        let v = self.get(holder, objective).wrapping_add(amount);
        self.set(holder, objective, v);
        v
    }
    /// `forAllObjectives`: change the score of `holder` in every objective
    /// using `criteria`, writable or not, in the order they were added.
    pub fn for_all_objectives(&mut self, criteria: &str, holder: &str, f: impl FnMut(i32) -> i32) {
        self.for_all_objectives_with(criteria, holder, None, f);
    }
    /// `forAllObjectives` for a holder with a display name.
    pub fn for_all_objectives_with(
        &mut self,
        criteria: &str,
        holder: &str,
        display: Option<&Comp>,
        mut f: impl FnMut(i32) -> i32,
    ) {
        let names = self.by_criteria.get(criteria).cloned().unwrap_or_default();
        for n in names {
            let v = f(self.get(holder, &n));
            self.set_with(holder, &n, v, display);
        }
    }
    pub fn has_objectives_for(&self, criteria: &str) -> bool {
        self.by_criteria
            .get(criteria)
            .is_some_and(|l| !l.is_empty())
    }
    /// `resetAllPlayerScores`.
    pub fn reset_all(&mut self, holder: &str) {
        if self.scores.remove(holder).is_some() {
            self.holder_order.remove(holder);
        }
    }
    /// `resetSinglePlayerScore`: a holder left without scores is forgotten.
    pub fn reset_one(&mut self, holder: &str, objective: &str) {
        if let Some(s) = self.scores.get_mut(holder) {
            s.remove(objective);
            if s.is_empty() {
                self.reset_all(holder);
            }
        }
    }
    /// `listPlayerScores(objective)`, in `playerScores.forEach` order.
    pub fn list_scores(&self, objective: &str) -> Vec<ScoreEntry> {
        self.holder_order
            .iter()
            .filter_map(|owner| {
                let s = self.scores.get(owner)?.get(objective)?;
                Some(ScoreEntry {
                    owner: owner.to_owned(),
                    value: s.value,
                    display: s.display.clone(),
                    number_format: s.number_format.clone(),
                })
            })
            .collect()
    }
    /// A holder's scores as (objective, value) in objective order.
    pub fn holder_scores(&self, holder: &str) -> Vec<(&Objective, i32)> {
        let Some(s) = self.scores.get(holder) else {
            return Vec::new();
        };
        self.objectives
            .iter()
            .filter_map(|o| Some((o, s.get(&o.name)?.value)))
            .collect()
    }
    /// `getTrackedPlayers` (`playerScores.keySet().stream()`).
    pub fn tracked(&self) -> Vec<String> {
        self.holder_order.stream().map(str::to_owned).collect()
    }

    // teams
    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.iter().find(|t| t.name == name)
    }
    pub fn team_mut(&mut self, name: &str) -> Option<&mut Team> {
        self.teams.iter_mut().find(|t| t.name == name)
    }
    pub fn add_team(&mut self, name: &str) -> &mut Team {
        if self.team(name).is_none() {
            self.teams.push(Team::new(name));
            self.team_order.insert(name);
            self.reorder();
        }
        self.team_mut(name).expect("just added")
    }
    /// `getPlayerTeams().stream()` (what saving uses).
    pub fn teams_streamed(&self) -> Vec<&Team> {
        self.team_order
            .stream()
            .filter_map(|n| self.team(n))
            .collect()
    }
    /// `removePlayerTeam`: its members no longer have a team.
    pub fn remove_team(&mut self, name: &str) {
        let Some(at) = self.teams.iter().position(|t| t.name == name) else {
            return;
        };
        let team = self.teams.remove(at);
        for p in team.players.iter() {
            self.team_of.remove(p);
        }
        self.team_order.remove(name);
        self.reorder();
    }
    pub fn players_team(&self, player: &str) -> Option<&Team> {
        self.team(self.team_of.get(player)?)
    }
    /// `addPlayerToTeam`: leaves the old team first (even when it is the
    /// same one); false when the team already listed the player.
    pub fn add_to_team(&mut self, player: &str, team: &str) -> bool {
        self.remove_from_team(player);
        self.team_of.insert(player.to_owned(), team.to_owned());
        match self.team_mut(team) {
            Some(t) => t.players.insert(player),
            None => false,
        }
    }
    pub fn remove_from_team(&mut self, player: &str) -> bool {
        let Some(team) = self.team_of.remove(player) else {
            return false;
        };
        if let Some(t) = self.team_mut(&team) {
            t.players.remove(player);
        }
        true
    }
    /// `Team.isAlliedTo` for two score holder names.
    pub fn allied(&self, a: &str, b: &str) -> bool {
        match (self.team_of.get(a), self.team_of.get(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }
    /// `Scoreboard.entityRemoved` for a dead entity that is not a player.
    pub fn entity_removed(&mut self, holder: &str) {
        self.reset_all(holder);
        self.remove_from_team(holder);
    }

    // saved data
    /// `ScoreboardSaveData.Packed.CODEC` encoding of `ServerScoreboard.store`:
    /// fields equal to their codec default are left out.
    pub fn pack(&self) -> Tag {
        let objectives: Vec<Tag> = self
            .objectives_streamed()
            .into_iter()
            .map(|o| {
                let mut c = vec![
                    ("Name", Tag::String(o.name.as_str().into())),
                    ("DisplayName", component_tag(&o.display_name)),
                ];
                if o.criteria != CRITERIA_DUMMY {
                    c.push(("CriteriaName", Tag::String(o.criteria.as_str().into())));
                }
                if o.render_type != RenderType::Integer {
                    c.push(("RenderType", Tag::String(o.render_type.id().into())));
                }
                if o.display_auto_update {
                    c.push(("display_auto_update", Tag::Byte(1)));
                }
                if let Some(f) = &o.number_format {
                    c.push(("format", f.to_tag()));
                }
                Tag::compound(c)
            })
            .collect();
        let mut scores = Vec::new();
        for owner in self.holder_order.stream() {
            let Some(per) = self.scores.get(owner) else {
                continue;
            };
            for o in self.objectives_streamed() {
                let Some(s) = per.get(&o.name) else { continue };
                let mut c = vec![
                    ("Name", Tag::String(owner.into())),
                    ("Objective", Tag::String(o.name.as_str().into())),
                ];
                if s.value != 0 {
                    c.push(("Score", Tag::Int(s.value)));
                }
                if s.locked {
                    c.push(("Locked", Tag::Byte(1)));
                }
                if let Some(d) = &s.display {
                    c.push(("display", component_tag(d)));
                }
                if let Some(f) = &s.number_format {
                    c.push(("format", f.to_tag()));
                }
                scores.push(Tag::compound(c));
            }
        }
        let slots = display_slots()
            .into_iter()
            .filter_map(|slot| {
                let o = self.display.get(&slot)?;
                self.objective(o)?;
                Some((slot, Tag::String(o.as_str().into())))
            })
            .collect::<Vec<_>>();
        let slots: Vec<(&str, Tag)> = slots.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let teams: Vec<Tag> = self
            .teams_streamed()
            .into_iter()
            .map(|t| {
                let mut c = vec![
                    ("Name", Tag::String(t.name.as_str().into())),
                    ("DisplayName", component_tag(&t.display_name)),
                ];
                if let Some(color) = &t.color {
                    c.push(("TeamColor", Tag::String(color.as_str().into())));
                }
                if !t.allow_friendly_fire {
                    c.push(("AllowFriendlyFire", Tag::Byte(0)));
                }
                if !t.see_friendly_invisibles {
                    c.push(("SeeFriendlyInvisibles", Tag::Byte(0)));
                }
                let empty = Value::String(String::new());
                if canonical_component(&t.prefix) != empty {
                    c.push(("MemberNamePrefix", component_tag(&t.prefix)));
                }
                if canonical_component(&t.suffix) != empty {
                    c.push(("MemberNameSuffix", component_tag(&t.suffix)));
                }
                if t.nametag_visibility != Visibility::Always {
                    c.push((
                        "NameTagVisibility",
                        Tag::String(t.nametag_visibility.name().into()),
                    ));
                }
                if t.death_message_visibility != Visibility::Always {
                    c.push((
                        "DeathMessageVisibility",
                        Tag::String(t.death_message_visibility.name().into()),
                    ));
                }
                if t.collision_rule != CollisionRule::Always {
                    c.push(("CollisionRule", Tag::String(t.collision_rule.name().into())));
                }
                if !t.players.is_empty() {
                    c.push((
                        "Players",
                        Tag::List(
                            t.players
                                .iter()
                                .map(|p| Tag::String(p.as_str().into()))
                                .collect(),
                        ),
                    ));
                }
                Tag::compound(c)
            })
            .collect();
        // The codec leaves out empty parts.
        let mut entries: Vec<(&str, Tag)> = Vec::new();
        if !objectives.is_empty() {
            entries.push(("Objectives", Tag::List(objectives)));
        }
        if !scores.is_empty() {
            entries.push(("PlayerScores", Tag::List(scores)));
        }
        if !slots.is_empty() {
            entries.push(("DisplaySlots", Tag::compound(slots)));
        }
        if !teams.is_empty() {
            entries.push(("Teams", Tag::List(teams)));
        }
        Tag::compound(entries)
    }
    /// `ServerScoreboard.load` of the decoded save data.
    pub fn unpack(data: &Tag) -> Self {
        let mut sb = Scoreboard::default();
        let list = |k: &str| match data.get(k) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        for o in list("Objectives") {
            let Some(name) = string_of(o.get("Name")) else {
                continue;
            };
            let criteria = string_of(o.get("CriteriaName")).unwrap_or_else(|| "dummy".into());
            sb.add_objective(
                &name,
                &criteria,
                o.get("DisplayName")
                    .map_or_else(|| literal(&name), tag_to_json),
                RenderType::by_id(&string_of(o.get("RenderType")).unwrap_or_default()),
                bool_of(o.get("display_auto_update"), false),
                o.get("format").and_then(NumberFormat::from_tag),
            );
        }
        for s in list("PlayerScores") {
            let (Some(owner), Some(obj)) =
                (string_of(s.get("Name")), string_of(s.get("Objective")))
            else {
                continue;
            };
            if sb.objective(&obj).is_none() {
                continue;
            }
            *sb.score_mut(&owner, &obj) = Score {
                value: int_of(s.get("Score")).unwrap_or(0),
                locked: bool_of(s.get("Locked"), false),
                display: s.get("display").map(tag_to_json),
                number_format: s.get("format").and_then(NumberFormat::from_tag),
            };
        }
        if let Some(Tag::Compound(slots)) = data.get("DisplaySlots") {
            for (k, v) in slots {
                let slot = k.to_string_lossy();
                if let Tag::String(o) = v
                    && display_slot_valid(&slot)
                {
                    let o = o.to_string_lossy();
                    if sb.objective(&o).is_some() {
                        sb.display.insert(slot, o);
                    }
                }
            }
        }
        for t in list("Teams") {
            let Some(name) = string_of(t.get("Name")) else {
                continue;
            };
            let team = sb.add_team(&name);
            if let Some(d) = t.get("DisplayName") {
                team.display_name = tag_to_json(d);
            }
            team.color = string_of(t.get("TeamColor")).filter(|c| team_color_rgb(c).is_some());
            team.allow_friendly_fire = bool_of(t.get("AllowFriendlyFire"), true);
            team.see_friendly_invisibles = bool_of(t.get("SeeFriendlyInvisibles"), true);
            team.prefix = t
                .get("MemberNamePrefix")
                .map_or_else(|| literal(""), tag_to_json);
            team.suffix = t
                .get("MemberNameSuffix")
                .map_or_else(|| literal(""), tag_to_json);
            team.nametag_visibility = string_of(t.get("NameTagVisibility"))
                .and_then(|v| Visibility::by_name(&v))
                .unwrap_or(Visibility::Always);
            team.death_message_visibility = string_of(t.get("DeathMessageVisibility"))
                .and_then(|v| Visibility::by_name(&v))
                .unwrap_or(Visibility::Always);
            team.collision_rule = string_of(t.get("CollisionRule"))
                .and_then(|v| CollisionRule::by_name(&v))
                .unwrap_or(CollisionRule::Always);
            if let Some(Tag::List(players)) = t.get("Players") {
                for p in players {
                    if let Tag::String(p) = p {
                        sb.add_to_team(&p.to_string_lossy(), &name);
                    }
                }
            }
        }
        sb
    }
}

/// A component as `ComponentSerialization.CODEC` writes it to NBT.
pub fn component_tag(c: &Comp) -> Tag {
    let mut tag = json_to_tag(&canonical_component(c));
    entity_uuids(&mut tag);
    tag
}
/// `HoverEvent.ShowEntity` writes its UUID with `UUIDUtil.CODEC`: an int array.
fn entity_uuids(tag: &mut Tag) {
    match tag {
        Tag::Compound(c) => {
            let show_entity = matches!(c.get(&"action".into()), Some(Tag::String(a)) if a.to_string_lossy() == "show_entity");
            for (k, v) in c.iter_mut() {
                if show_entity && k.to_string_lossy() == "uuid" {
                    let ints = match v {
                        Tag::List(l) => l
                            .iter()
                            .map(|t| match t {
                                Tag::Int(i) => Some(*i),
                                _ => None,
                            })
                            .collect::<Option<Vec<i32>>>(),
                        Tag::String(s) => {
                            crate::uuid::uuid_from_text(&s.to_string_lossy()).map(Vec::from)
                        }
                        _ => None,
                    };
                    if let Some(ints) = ints.filter(|i| i.len() == 4) {
                        *v = Tag::IntArray(ints);
                    }
                } else {
                    entity_uuids(v);
                }
            }
        }
        Tag::List(l) => l.iter_mut().for_each(entity_uuids),
        _ => {}
    }
}

/// A style object as `Style.Serializer.MAP_CODEC` re-encodes it.
pub fn canonical_style(style: &Value) -> Value {
    let (mut node, _) = decompose(style);
    node.remove("text");
    Value::Object(node)
}

/// A component as vanilla decodes and re-encodes it (`ComponentSerialization`):
/// a literal without style or siblings becomes a plain string, a list becomes
/// its first element with the others appended as siblings, and the content
/// type marker is left out.
pub fn canonical_component(c: &Comp) -> Comp {
    let (mut node, extra) = decompose(c);
    finish(&mut node, extra)
}
/// A component's own fields (content and style) and its siblings.
fn decompose(c: &Comp) -> (serde_json::Map<String, Value>, Vec<Comp>) {
    match c {
        Value::Array(list) if !list.is_empty() => {
            let (mut node, mut extra) = decompose(&list[0]);
            extra.extend(list[1..].iter().map(canonical_component));
            node.remove("extra");
            (node, extra)
        }
        Value::Object(o) => {
            let mut node = serde_json::Map::new();
            let mut extra = Vec::new();
            for (k, v) in o {
                match k.as_str() {
                    "type" => {}
                    "extra" => {
                        if let Value::Array(list) = v {
                            extra.extend(list.iter().map(canonical_component));
                        }
                    }
                    "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" => {
                        let b = match v {
                            Value::Bool(b) => *b,
                            Value::Number(n) => n.as_f64().is_some_and(|f| f as i8 != 0),
                            _ => continue,
                        };
                        node.insert(k.clone(), Value::Bool(b));
                    }
                    "color" => {
                        let color = v.as_str().map(|s| match s.strip_prefix('#') {
                            Some(hex) => match u32::from_str_radix(hex, 16) {
                                Ok(rgb) => format!("#{rgb:06X}"),
                                Err(_) => s.to_owned(),
                            },
                            None => s.to_owned(),
                        });
                        if let Some(color) = color {
                            node.insert(k.clone(), Value::String(color));
                        }
                    }
                    "with" => {
                        if let Value::Array(args) = v {
                            let args = args
                                .iter()
                                .map(|a| match a {
                                    Value::Object(_) | Value::Array(_) => canonical_component(a),
                                    other => other.clone(),
                                })
                                .collect();
                            node.insert(k.clone(), Value::Array(args));
                        }
                    }
                    "separator" => {
                        node.insert(k.clone(), canonical_component(v));
                    }
                    "hover_event" => {
                        let mut h = v.clone();
                        if let Some(value) = h.get("value").cloned()
                            && h.get("action").and_then(Value::as_str) == Some("show_text")
                        {
                            h["value"] = canonical_component(&value);
                        }
                        node.insert(k.clone(), h);
                    }
                    _ => {
                        node.insert(k.clone(), v.clone());
                    }
                }
            }
            (node, extra)
        }
        Value::String(s) => {
            let mut node = serde_json::Map::new();
            node.insert("text".into(), Value::String(s.clone()));
            (node, Vec::new())
        }
        other => {
            let mut node = serde_json::Map::new();
            node.insert("text".into(), Value::String(other.to_string()));
            (node, Vec::new())
        }
    }
}
fn finish(node: &mut serde_json::Map<String, Value>, extra: Vec<Comp>) -> Comp {
    if extra.is_empty()
        && node.len() == 1
        && let Some(Value::String(s)) = node.get("text")
    {
        return Value::String(s.clone());
    }
    if !extra.is_empty() {
        node.insert("extra".into(), Value::Array(extra));
    }
    Value::Object(node.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Scoreboard {
        let mut sb = Scoreboard::default();
        sb.add_objective(
            "kills",
            "playerKillCount",
            literal("Kills"),
            RenderType::Integer,
            false,
            None,
        );
        sb.add_objective(
            "hp",
            "health",
            literal("HP"),
            RenderType::Hearts,
            true,
            Some(NumberFormat::Blank),
        );
        sb
    }

    #[test]
    fn criteria_names() {
        let reg = Registries::default();
        assert!(criteria_exists("dummy", &reg));
        assert!(criteria_exists("teamkill.red", &reg));
        assert!(criteria_exists("killedByTeam.dark_blue", &reg));
        assert!(!criteria_exists("teamkill.pink", &reg));
        assert!(criteria_exists("minecraft.mined:minecraft.stone", &reg));
        assert!(criteria_exists("minecraft.custom:minecraft.jump", &reg));
        assert!(!criteria_exists("minecraft.custom:minecraft.nope", &reg));
        assert!(!criteria_exists("minecraft.flown:minecraft.stone", &reg));
        assert!(!criteria_exists("nonsense", &reg));
        assert!(criteria_read_only("health") && !criteria_read_only("dummy"));
        assert_eq!(criteria_render_type("health"), RenderType::Hearts);
        assert_eq!(
            stat_criteria("minecraft:mined", "minecraft:stone"),
            "minecraft.mined:minecraft.stone"
        );
        let reg = Registries {
            items: vec!["minecraft:stick".into()],
            ..Registries::default()
        };
        assert!(criteria_exists("minecraft.crafted:minecraft.stick", &reg));
        assert!(!criteria_exists("minecraft.crafted:minecraft.dirt", &reg));
    }

    #[test]
    fn scores_and_criteria_updates() {
        let mut sb = board();
        assert!(!sb.add_objective(
            "kills",
            "dummy",
            literal("x"),
            RenderType::Integer,
            false,
            None
        ));
        // A new score starts locked at zero.
        assert!(sb.score_mut("Steve", "kills").locked);
        sb.for_all_objectives("playerKillCount", "Steve", |v| v + 1);
        sb.for_all_objectives("playerKillCount", "Steve", |v| v + 1);
        assert_eq!(sb.get("Steve", "kills"), 2);
        assert_eq!(sb.add("Steve", "kills", 5), 7);
        assert_eq!(sb.holder_scores("Steve").len(), 1);
        sb.set("Alex", "hp", 20);
        let mut tracked = sb.tracked();
        tracked.sort();
        assert_eq!(tracked, vec!["Alex".to_owned(), "Steve".to_owned()]);
        sb.reset_one("Alex", "hp");
        assert_eq!(sb.tracked(), vec!["Steve".to_owned()]);
        sb.set_display_objective("sidebar", Some("kills"));
        sb.remove_objective("kills");
        assert!(sb.display_objective("sidebar").is_none());
        // removeObjective keeps the holder, now without scores.
        assert_eq!(sb.tracked(), vec!["Steve".to_owned()]);
        assert!(sb.holder_scores("Steve").is_empty());
    }

    #[test]
    fn teams_membership() {
        let mut sb = Scoreboard::default();
        sb.add_team("red");
        sb.add_team("blue");
        assert!(sb.add_to_team("Steve", "red"));
        // Joining the same team leaves it first, so it succeeds again.
        assert!(sb.add_to_team("Steve", "red"));
        assert!(sb.add_to_team("Alex", "red"));
        assert!(sb.allied("Steve", "Alex"));
        assert!(sb.add_to_team("Steve", "blue"));
        assert!(!sb.allied("Steve", "Alex"));
        assert_eq!(sb.team("red").map(|t| t.players.len()), Some(1));
        assert!(sb.remove_from_team("Alex"));
        assert!(!sb.remove_from_team("Alex"));
        sb.remove_team("blue");
        assert!(sb.players_team("Steve").is_none());
    }

    #[test]
    fn pack_round_trip() {
        let mut sb = board();
        sb.set("Steve", "kills", 3);
        sb.score_mut("Steve", "kills").locked = false;
        sb.score_mut("Steve", "kills").display =
            Some(serde_json::json!({"text": "S", "color": "red"}));
        sb.score_mut("Steve", "kills").number_format = Some(NumberFormat::Fixed(literal("many")));
        sb.set_display_objective("sidebar.team.red", Some("kills"));
        let t = sb.add_team("red");
        t.color = Some("red".into());
        t.prefix = literal("[R] ");
        t.allow_friendly_fire = false;
        t.nametag_visibility = Visibility::HideForOtherTeams;
        t.collision_rule = CollisionRule::PushOwnTeam;
        sb.add_to_team("Steve", "red");
        sb.objective_mut("hp").unwrap().number_format = Some(NumberFormat::Styled(
            serde_json::json!({"color": "gold", "bold": true}),
        ));
        let back = Scoreboard::unpack(&sb.pack());
        assert_eq!(back, sb);
        // And through the binary NBT format.
        let bytes = horizon_world::nbt::write_named(
            &horizon_world::nbt::NamedTag {
                name: "".into(),
                tag: sb.pack(),
            },
            Default::default(),
        )
        .unwrap();
        let (named, _) = horizon_world::nbt::read_named(&bytes, Default::default()).unwrap();
        assert_eq!(Scoreboard::unpack(&named.tag), sb);
    }

    #[test]
    fn number_formats() {
        assert_eq!(NumberFormat::Blank.format(5).plain(), "");
        assert_eq!(NumberFormat::sidebar_default().format(5).plain(), "5");
        assert_eq!(
            NumberFormat::sidebar_default().format(5).spans[0]
                .style
                .color,
            Some(text::RED)
        );
        assert_eq!(
            NumberFormat::Fixed(literal("lots")).format(5).plain(),
            "lots"
        );
        let t = NumberFormat::Styled(serde_json::json!({"color": "gold"})).to_tag();
        assert_eq!(
            NumberFormat::from_tag(&t),
            Some(NumberFormat::Styled(serde_json::json!({"color": "gold"})))
        );
    }
}
