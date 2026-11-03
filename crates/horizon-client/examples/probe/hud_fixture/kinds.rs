//! The overlay case kinds of the HUD parity fixture.
use horizon_client::{
    brewing,
    chat::{Chat, ChatOptions},
    component,
    hud::HudAtlas,
    hud_layout::{self, GuiDraw},
    hud_overlays::{
        self, BossBar, CameraOverlays, EffectIcon, HeldName, ItemDecorations, Titles, ToolHighlight,
    },
    scoreboard::{NumberFormat, RenderType, Scoreboard},
    text::{self, Text},
};
use serde_json::Value;

fn int(c: &Value, k: &str, d: i64) -> i32 {
    c[k].as_i64().unwrap_or(d) as i32
}
fn float(c: &Value, k: &str, d: f64) -> f32 {
    c[k].as_f64().unwrap_or(d) as f32
}
/// A text component in its JSON form.
fn comp(v: &Value) -> Text {
    component::to_text(v, &mut |s| Text::literal(s))
}
/// `Options.getBackgroundColor(0.0F)` from the case's text background options.
fn background(c: &Value) -> u32 {
    hud_layout::background_color(
        c["chatOnly"].as_bool().unwrap_or(true),
        c["textOpacity"].as_f64().unwrap_or(0.5),
        0.0,
    )
}

pub fn run(
    kind: &str,
    c: &Value,
    atlas: &HudAtlas,
    state: &mut serde_json::Map<String, Value>,
) -> Vec<GuiDraw> {
    let (w, h) = (int(c, "width", 0), int(c, "height", 0));
    match kind {
        "effects" => effects(c, w),
        "title" => title(c, w, h, atlas),
        "selected" => selected(c, w, h, atlas),
        "boss" => boss(c, w, h, atlas),
        "jump" => hud_overlays::jump_bar(w, h, int(c, "cooldown", 0), float(c, "jumpScale", 0.0)),
        "sidebar" => sidebar(c, w, h, atlas),
        "decor" => decor(c, atlas),
        "overlays" => overlays(c, w, h, state),
        "chat" => chat(c, h, atlas),
        _ => Vec::new(),
    }
}

fn effects(c: &Value, w: i32) -> Vec<GuiDraw> {
    let icons: Vec<EffectIcon> = c["active"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let id = e["id"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches("minecraft:");
            let def = brewing::effect(id).expect("effect");
            EffectIcon {
                id: id.to_owned(),
                beneficial: def.category == brewing::Category::Beneficial,
                color: def.color as i32,
                ambient: e["ambient"].as_bool().unwrap_or(false),
                duration: e["duration"].as_i64().unwrap_or(0) as i32,
                show_icon: e["icon"].as_bool().unwrap_or(true),
            }
        })
        .collect();
    hud_overlays::effects(w, c["demo"].as_bool().unwrap_or(false), &icons)
}

/// Replays `Hud.setTitle` / `setSubtitle` / `setTimes` / `clearTitles` /
/// `resetTitleTimes` / `setOverlayMessage` and ticks, then extracts the
/// action bar and the title.
fn title(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let mut t = Titles::default();
    for op in c["ops"].as_array().into_iter().flatten() {
        let n = |i: usize| op[i].as_i64().unwrap_or(0) as i32;
        match op[0].as_str().unwrap_or("") {
            "title" => t.set_title(comp(&op[1])),
            "subtitle" => t.set_subtitle(comp(&op[1])),
            "times" => t.set_times(n(1), n(2), n(3)),
            "clear" => t.clear(),
            "reset" => t.reset_times(),
            "overlay" => t.set_overlay(comp(&op[1]), op[2].as_bool().unwrap_or(false)),
            "tick" => {
                for _ in 0..n(1) {
                    t.tick();
                }
            }
            other => panic!("unknown title op {other}"),
        }
    }
    let partial = float(c, "partial", 0.0);
    let bg = background(c);
    let mut out = t.overlay_message(w, h, 39, partial, &atlas.font, bg);
    out.extend(t.title(w, h, partial, &atlas.font, bg));
    out
}

/// Ticks `toolHighlightTimer` through the selected stacks, then extracts the name.
fn selected(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let mut highlight = ToolHighlight::default();
    let time = c["displayTime"].as_f64().unwrap_or(1.0);
    for step in c["steps"].as_array().into_iter().flatten() {
        let held = (!step.is_null()).then(|| {
            let custom = &step["custom"];
            HeldName {
                item: step["item"].as_str().unwrap_or("").to_owned(),
                hover_name: if custom.is_null() {
                    Text::translate(step["key"].as_str().unwrap_or(""), &[])
                } else {
                    comp(custom)
                },
                rarity_color: text::named_color(step["rarity"].as_str().unwrap_or("white"))
                    .unwrap_or(0xFFFFFF),
                custom_name: !custom.is_null(),
            }
        });
        highlight.tick(held, time);
    }
    let mode = c["gameMode"].as_str().unwrap_or("survival");
    let can_hurt = mode == "survival" || mode == "adventure";
    highlight.draw(w, h, 39, can_hurt, &atlas.font, background(c))
}

fn boss(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let bars: Vec<BossBar> = c["bars"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| BossBar {
            name: comp(&b["name"]),
            progress: b["progress"].as_f64().unwrap_or(0.0) as f32,
            color: b["color"].as_u64().unwrap_or(0) as usize,
            overlay: b["overlay"].as_u64().unwrap_or(0) as usize,
        })
        .collect();
    hud_overlays::boss_overlay(w, h, &bars, &atlas.font)
}

/// A `NumberFormat` from the case: null, `{type: blank}`, `{type: fixed,
/// value}` or `{type: styled, style}`.
fn number_format(v: &Value) -> Option<NumberFormat> {
    match v["type"].as_str()? {
        "blank" => Some(NumberFormat::Blank),
        "fixed" => Some(NumberFormat::Fixed(v["value"].clone())),
        "styled" => Some(NumberFormat::Styled(v["style"].clone())),
        other => panic!("unknown number format {other}"),
    }
}

fn sidebar(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let mut board = Scoreboard::default();
    let o = &c["objective"];
    board.add_objective(
        "sidebar",
        "dummy",
        o["display"].clone(),
        RenderType::Integer,
        false,
        number_format(&o["format"]),
    );
    for s in c["scores"].as_array().into_iter().flatten() {
        let owner = s["owner"].as_str().unwrap_or("");
        let score = board.score_mut(owner, "sidebar");
        score.value = s["value"].as_i64().unwrap_or(0) as i32;
        score.display = (!s["display"].is_null()).then(|| s["display"].clone());
        score.number_format = number_format(&s["format"]);
    }
    for t in c["teams"].as_array().into_iter().flatten() {
        let name = t["name"].as_str().unwrap_or("");
        let team = board.add_team(name);
        team.prefix = t["prefix"].clone();
        team.suffix = t["suffix"].clone();
        team.color = t["color"].as_str().map(str::to_owned);
        for m in t["members"].as_array().into_iter().flatten() {
            board.add_to_team(m.as_str().unwrap_or(""), name);
        }
    }
    let opacity = c["textOpacity"].as_f64().unwrap_or(0.5);
    let chat_only = c["chatOnly"].as_bool().unwrap_or(true);
    let objective = board.objective("sidebar").expect("objective").clone();
    hud_overlays::sidebar(
        w,
        h,
        &board,
        &objective,
        &atlas.font,
        hud_layout::background_color(chat_only, opacity, 0.3),
        hud_layout::background_color(chat_only, opacity, 0.4),
    )
}

/// `itemDecorations` for a stack with a count, damage and an item cooldown
/// `elapsed` ticks into `duration` (ItemCooldowns.getCooldownPercent).
fn decor(c: &Value, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let count = int(c, "count", 1);
    if count <= 0 {
        return Vec::new();
    }
    let cd = &c["cooldown"];
    let cooldown = if cd.is_null() {
        0.0
    } else {
        let (duration, elapsed) = (int(cd, "duration", 0), int(cd, "elapsed", 0));
        if duration <= elapsed {
            0.0 // ItemCooldowns.tick dropped it
        } else {
            let remaining = duration as f32 - (elapsed as f32 + float(c, "partial", 0.0));
            (remaining / duration as f32).clamp(0.0, 1.0)
        }
    };
    let max_damage = if c["unbreakable"].as_bool().unwrap_or(false) {
        0
    } else {
        int(c, "maxDamage", 0)
    };
    let d = ItemDecorations {
        count: count as u32,
        count_text: c["countText"].as_str().map(str::to_owned),
        bar: hud_overlays::damage_bar(int(c, "damage", 0), max_damage),
        cooldown,
    };
    hud_overlays::item_decorations(int(c, "x", 0), int(c, "y", 0), &d, &atlas.font)
}

/// `Hud.extractCameraOverlays` then `extractSleepOverlay`; the border is
/// static, so only its warning blocks matter.
fn overlays(c: &Value, w: i32, h: i32, state: &mut serde_json::Map<String, Value>) -> Vec<GuiDraw> {
    let partial = float(c, "partial", 0.0);
    let warning = match &c["border"] {
        Value::Null => 0.0,
        b => {
            // WorldBorder.getDistanceToBorder with the static extent's clamped box.
            let max = 29_999_984.0;
            let half = b["size"].as_f64().unwrap_or(0.0) / 2.0;
            let (cx, cz) = (
                b["centerX"].as_f64().unwrap_or(0.0),
                b["centerZ"].as_f64().unwrap_or(0.0),
            );
            let (min_x, max_x) = ((cx - half).clamp(-max, max), (cx + half).clamp(-max, max));
            let (min_z, max_z) = ((cz - half).clamp(-max, max), (cz + half).clamp(-max, max));
            let (x, z) = (
                f64::from(float(c, "camX", 0.0)),
                f64::from(float(c, "camZ", 0.0)),
            );
            let distance = (x - min_x).min(max_x - x).min(z - min_z).min(max_z - z);
            hud_overlays::border_warning(
                distance as f32,
                int(b, "warningBlocks", 5),
                int(b, "warningTime", 15),
                0.0,
                0.0,
            )
        }
    };
    let ticks_frozen = int(c, "ticksFrozen", 0);
    let inputs = CameraOverlays {
        vignette: c["vignette"]
            .as_bool()
            .unwrap_or(false)
            .then(|| (float(c, "vignetteBrightness", 1.0), warning)),
        first_person: c["firstPerson"].as_bool().unwrap_or(false),
        scoping: c["scoping"].as_bool().unwrap_or(false),
        game_time_delta: partial,
        equipment: match c["head"].as_str() {
            Some("carved_pumpkin") => vec!["misc/pumpkinblur".to_owned()],
            _ => Vec::new(),
        },
        ticks_frozen,
        percent_frozen: ticks_frozen.min(140) as f32 / 140.0,
        partial,
        portal: [
            c["portal"][0].as_f64().unwrap_or(0.0) as f32,
            c["portal"][1].as_f64().unwrap_or(0.0) as f32,
        ],
        nausea: float(c, "nausea", 0.0),
        screen_effect_scale: float(c, "screenEffectScale", 1.0),
    };
    let mut scope = float(c, "scopeScale", 0.5);
    let mut out = hud_overlays::camera_overlays(w, h, &inputs, &mut scope);
    out.extend(hud_overlays::sleep_overlay(w, h, int(c, "sleepTimer", 0)));
    state.insert("scopeScale".into(), serde_json::json!(scope));
    out
}

/// Messages added at their ticks (`Hud.tickCount`), then the chat in
/// background mode at tick `now`.
fn chat(c: &Value, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let option = |k: &str| c[k].as_f64().unwrap_or(0.0);
    let mut chat = Chat::default();
    chat.options = ChatOptions {
        opacity: option("chatOpacity"),
        background_opacity: option("textOpacity"),
        scale: option("chatScale"),
        width: option("chatWidth"),
        height_focused: option("chatHeightFocused"),
        height_unfocused: option("chatHeightUnfocused"),
        line_spacing: option("chatLineSpacing"),
        colors: c["chatColors"].as_bool().unwrap_or(true),
    };
    for m in c["messages"].as_array().into_iter().flatten() {
        // GuiMessageTag indicator colours: systemSinglePlayer and chatError.
        let indicator = match m["tag"].as_str() {
            Some("system") => Some(13684944),
            Some("error") => Some(16733525),
            _ => None,
        };
        chat.add(
            atlas,
            comp(&m["text"]),
            indicator,
            horizon_client::chat::ChatSource::Client,
            m["at"].as_u64().unwrap_or(0),
            false,
        );
    }
    let focused = c["focused"].as_bool().unwrap_or(false);
    if let Some(n) = c["scroll"].as_i64() {
        chat.scroll_by(n as i32, focused);
    }
    chat.layout(h, c["now"].as_f64().unwrap_or(0.0) as f32, focused)
}
