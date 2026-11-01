//! `[discord]`: the server's chat, joins, leaves and deaths posted to a Discord channel through a webhook
//! (docs/hosting.md). Only outgoing HTTPS to Discord; nothing listens. Lines are batched for 2 s, drawn as
//! one picture of the in-game chat's text (Pixelify Sans, SIL OFL, from horizon-theme: no Mojang asset), and
//! posted at most every 2 s. Nothing posted can ping: `allowed_mentions` is empty and every `@` is defused.

use crate::api::Shared;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

/// Lines wait this long for others to join their picture.
const BATCH: Duration = Duration::from_secs(2);
/// The least time between two posts.
const GAP: Duration = Duration::from_secs(2);
/// Lines in one picture; more in a burst are counted and skipped.
const MAX_LINES: usize = 20;
const EVENTS: [&str; 5] = ["chat", "join", "leave", "death", "kick"];
/// Zero-width joiner: after an `@` or a `<`, Discord no longer reads a mention.
const ZWJ: char = '\u{200D}';

/// The `[discord]` table of server.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub webhook: String,
    pub events: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            webhook: String::new(),
            events: ["chat", "join", "leave", "death"].map(String::from).into(),
        }
    }
}

impl Settings {
    pub fn check(&self) -> Result<(), String> {
        let hook = self.webhook.trim();
        let discord = [
            "discord.com",
            "discordapp.com",
            "ptb.discord.com",
            "canary.discord.com",
        ];
        if !hook.is_empty()
            && !discord.iter().any(|host| {
                hook.strip_prefix("https://")
                    .and_then(|h| h.strip_prefix(host))
                    .is_some_and(|path| path.starts_with("/api/webhooks/"))
            })
        {
            return Err("discord.webhook must be a Discord webhook URL (https://discord.com/api/webhooks/...)".into());
        }
        match self.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
            Some(e) => Err(format!(
                "discord.events has \"{e}\": use {}",
                EVENTS.join(", ")
            )),
            None => Ok(()),
        }
    }
}

/// Starts the forwarder when `[discord]` has a webhook and is enabled.
pub fn start(shared: Arc<Shared>, settings: Settings) {
    if !settings.enabled || settings.webhook.trim().is_empty() {
        return;
    }
    let _ = thread::Builder::new()
        .name("discord".into())
        .spawn(move || forward(&shared, &settings));
}

fn forward(shared: &Shared, settings: &Settings) {
    let wanted = |kind: &str| settings.events.iter().any(|e| e == kind);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("horizon-server/", env!("CARGO_PKG_VERSION")))
        .build();
    let font = FontRef::try_from_slice(FONT).expect("the bundled font parses");
    let mut glyphs = HashMap::new();
    let mut cursor = shared.events.lock().unwrap().after(u64::MAX, &wanted).0;
    let (mut pending, mut skipped, mut since) = (Vec::new(), 0, None::<Instant>);
    let mut limiter = Limiter::default();
    loop {
        let fresh = {
            let events = shared.events.lock().unwrap();
            let events = shared
                .event_added
                .wait_timeout(events, Duration::from_millis(500))
                .unwrap()
                .0;
            let (last, fresh) = events.after(cursor, &wanted);
            cursor = last;
            fresh
        };
        for line in fresh.iter().filter_map(Line::from_event) {
            since.get_or_insert_with(Instant::now);
            if pending.len() < MAX_LINES {
                pending.push(line);
            } else {
                skipped += 1;
            }
        }
        let now = Instant::now();
        if since.is_none_or(|t| now < t + BATCH) || !limiter.ready(now) {
            continue;
        }
        if skipped > 0 {
            pending.push(Line::note(&format!("({skipped} more lines skipped)")));
        }
        match post(
            &agent,
            settings.webhook.trim(),
            &pending,
            &font,
            &mut glyphs,
            &mut limiter,
        ) {
            Ok(()) => {}
            Err(Failure::Gone(code)) => {
                return crate::supervisor::note(
                    shared,
                    "WARN",
                    &format!("Discord: the webhook answered {code}; posting stops until a restart"),
                );
            }
            Err(Failure::Other(e)) => {
                crate::supervisor::note(shared, "WARN", &format!("Discord: {e}"))
            }
        }
        (pending, skipped, since) = (Vec::new(), 0, None);
    }
}

/// One line of chat: coloured runs.
#[derive(Debug, PartialEq)]
struct Line {
    runs: Vec<Run>,
}

#[derive(Debug, PartialEq)]
struct Run {
    text: String,
    color: u32,
    bold: bool,
}

const WHITE: u32 = 0xFFFFFF;
const YELLOW: u32 = 0xFFFF55;
const GRAY: u32 = 0xAAAAAA;
/// ChatFormatting's colours, by their code `0`-`f`.
const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xFFAA00, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

impl Line {
    fn from_event(event: &Value) -> Option<Self> {
        let name = event["name"].as_str()?.to_owned();
        let text = |s: &str| event[s].as_str().unwrap_or_default();
        let runs = match event["type"].as_str()? {
            "chat" => std::iter::once(Run::plain(&format!("<{name}> "), WHITE))
                .chain(legacy(text("message"), WHITE))
                .collect(),
            "join" => vec![Run::plain(&format!("{name} joined the game"), YELLOW)],
            "leave" => vec![Run::plain(&format!("{name} left the game"), YELLOW)],
            "kick" => vec![Run::plain(&format!("{name} was kicked"), YELLOW)],
            // The game writes this message itself: should a nickname stand for the player there, the real name
            // follows it. Lines say who someone is only by `name` (their account), never by a nickname.
            "death" if !text("message").contains(name.as_str()) => {
                legacy(&format!("{} ({name})", text("message")), WHITE)
            }
            "death" => legacy(text("message"), WHITE),
            _ => return None,
        };
        Some(Self { runs })
    }

    fn note(text: &str) -> Self {
        Self {
            runs: vec![Run::plain(text, GRAY)],
        }
    }

    fn plain(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

impl Run {
    fn plain(text: &str, color: u32) -> Self {
        Self {
            text: text.into(),
            color,
            bold: false,
        }
    }
}

/// Text with `§` formatting codes, as coloured runs: a colour resets bold, `§r` goes back to `base`.
fn legacy(text: &str, base: u32) -> Vec<Run> {
    let mut runs = vec![Run::plain("", base)];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '§' {
            runs.last_mut().unwrap().text.push(c);
            continue;
        }
        let last = runs.last().map_or(base, |r| r.color);
        let (color, bold) = match chars.next().map(|c| c.to_ascii_lowercase()) {
            Some(code @ ('0'..='9' | 'a'..='f')) => {
                (PALETTE[code.to_digit(16).unwrap() as usize], false)
            }
            Some('l') => (last, true),
            Some('r') => (base, false),
            _ => continue,
        };
        runs.push(Run {
            text: String::new(),
            color,
            bold,
        });
    }
    runs.retain(|r| !r.text.is_empty());
    runs
}

/// Text that can't mention anyone (`@everyone`, `@here`, `<@id>`, `<@&id>`, `<#id>`), and with `markdown`
/// shows as it is.
fn defuse(text: &str, markdown: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if markdown && "\\*_~`|>#-[]()".contains(c) {
            out.push('\\');
        }
        out.push(c);
        if c == '@' || (c == '<' && matches!(chars.peek(), Some('@' | '#' | ':' | '/'))) {
            out.push(ZWJ);
        }
    }
    out
}

/// At most one post every `GAP`, later when Discord says so.
#[derive(Default)]
struct Limiter {
    next: Option<Instant>,
}

impl Limiter {
    fn ready(&self, now: Instant) -> bool {
        self.next.is_none_or(|t| now >= t)
    }

    fn posted(&mut self, now: Instant, wait: Duration) {
        self.next = Some(now + wait.max(GAP));
    }
}

enum Failure {
    /// The webhook is deleted or its token wrong: posting again won't help.
    Gone(u16),
    Other(String),
}

/// The webhook's JSON: never a mention, the lines as the picture's alt text or as the message itself.
fn payload(lines: &[Line], picture: bool) -> Value {
    let alt: String = lines.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    let mut payload = json!({ "allowed_mentions": { "parse": [] } });
    // Every post comes from the webhook's own name and avatar (one identity for the server), never a player's.
    // Discord's limits: 1024 characters of alt text, 2000 of message.
    if picture {
        let alt = truncate(&defuse(&alt, false), 1000);
        payload["attachments"] = json!([{ "id": 0, "filename": "chat.png", "description": alt }]);
    } else {
        payload["content"] = truncate(&defuse(&alt, true), 1900).into();
    }
    payload
}

/// Posts the lines as a picture, or as plain text when drawing fails; waits out a 429 (three times at most).
fn post(
    agent: &ureq::Agent,
    webhook: &str,
    lines: &[Line],
    font: &FontRef,
    glyphs: &mut HashMap<char, Glyph>,
    limiter: &mut Limiter,
) -> Result<(), Failure> {
    let picture = render(lines, font, glyphs);
    let payload = payload(lines, picture.is_some());
    let (content_type, body) = match &picture {
        Some(png) => {
            let boundary = "horizon-discord-bridge-7f3a9c";
            let part = |headers: &str| {
                format!("--{boundary}\r\nContent-Disposition: form-data; {headers}\r\n\r\n")
            };
            let mut body =
                part("name=\"payload_json\"\r\nContent-Type: application/json").into_bytes();
            body.extend_from_slice(format!("{payload}\r\n").as_bytes());
            body.extend_from_slice(
                part("name=\"files[0]\"; filename=\"chat.png\"\r\nContent-Type: image/png")
                    .as_bytes(),
            );
            body.extend_from_slice(png);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            (format!("multipart/form-data; boundary={boundary}"), body)
        }
        None => ("application/json".into(), payload.to_string().into_bytes()),
    };
    for _ in 0..3 {
        let sent = agent
            .post(webhook)
            .set("Content-Type", &content_type)
            .send_bytes(&body);
        let now = Instant::now();
        // Errors never say the URL: it holds the webhook's token.
        let response = match sent {
            Ok(response) => response,
            Err(ureq::Error::Status(429, response)) => {
                let wait = retry_after(response);
                limiter.posted(now, wait);
                thread::sleep(wait.max(GAP));
                continue;
            }
            Err(ureq::Error::Status(code @ (401 | 403 | 404), _)) => {
                return Err(Failure::Gone(code));
            }
            Err(ureq::Error::Status(code, _)) => {
                limiter.posted(now, GAP);
                return Err(Failure::Other(format!("the webhook answered {code}")));
            }
            Err(ureq::Error::Transport(t)) => {
                limiter.posted(now, GAP * 5);
                return Err(Failure::Other(format!(
                    "Discord can't be reached ({})",
                    t.kind()
                )));
            }
        };
        let wait = match response.header("x-ratelimit-remaining") {
            Some("0") => secs(response.header("x-ratelimit-reset-after")),
            _ => GAP,
        };
        limiter.posted(now, wait);
        return Ok(());
    }
    Err(Failure::Other(
        "rate limited three times; these lines are dropped".into(),
    ))
}

/// A 429's wait: `retry_after` in the JSON body, else the header; 60 s at most.
fn retry_after(response: ureq::Response) -> Duration {
    let header = response.header("retry-after").map(str::to_owned);
    let body: Value = response
        .into_string()
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    match body["retry_after"].as_f64() {
        Some(s) => Duration::from_secs_f64(s.clamp(0.0, 60.0)),
        None => secs(header.as_deref()),
    }
}

fn secs(value: Option<&str>) -> Duration {
    let s = value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(2.0);
    Duration::from_secs_f64(s.clamp(0.0, 60.0))
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

// Drawing, in the picture's pixels: `U` per pixel of the game's GUI (GUI scale 2, like a 1080p window).
static FONT: &[u8] = include_bytes!("../../horizon-theme/fonts/PixelifySans-Medium.ttf");
const U: usize = 2;
/// Font units per pixel of Pixelify Sans's grid, which starts 60 units right of the origin and 12 below the
/// baseline. Drawn at `U` picture pixels per grid square, so its half-square accents keep their shape.
const GRID: f32 = 89.2;
const FONT_PX: f32 = U as f32 * 1000.0 / GRID;
/// Room for accents above capitals (vanilla squeezes them into 9 pixels).
const LINE_H: usize = 12 * U;
const BASELINE: f32 = (10 * U) as f32;
const PAD: usize = 2 * U;
/// Vanilla's default chat width.
const WIDTH: usize = 320 * U;

struct Glyph {
    advance: usize,
    pixels: Vec<(usize, usize)>,
}

fn glyph<'a>(font: &FontRef, glyphs: &'a mut HashMap<char, Glyph>, c: char) -> &'a Glyph {
    glyphs.entry(c).or_insert_with(|| {
        let id = font.glyph_id(c);
        let advance = font.as_scaled(PxScale::from(FONT_PX)).h_advance(id).round() as usize;
        let mut pixels = Vec::new();
        if let Some(outline) = font.outline_glyph(id.with_scale_and_position(
            FONT_PX,
            point(
                U as f32 * (1.0 - 60.0 / GRID),
                BASELINE - U as f32 * 12.0 / GRID,
            ),
        )) {
            let at = outline.px_bounds().min;
            outline.draw(|x, y, coverage| {
                let (x, y) = (at.x as i32 + x as i32, at.y as i32 + y as i32);
                if coverage >= 0.5 && x >= 0 && (0..LINE_H as i32).contains(&y) {
                    pixels.push((x as usize, y as usize));
                }
            });
        }
        Glyph {
            advance: advance.max(1),
            pixels,
        }
    })
}

/// The lines as an RGBA PNG on a transparent ground: wrapped at the chat's width, each glyph over its shadow.
fn render(lines: &[Line], font: &FontRef, glyphs: &mut HashMap<char, Glyph>) -> Option<Vec<u8>> {
    // Wrap into rows of (char, colour, bold, x), breaking at the last space when one fits.
    let mut rows: Vec<Vec<(char, u32, bool, usize)>> = Vec::new();
    for line in lines {
        let mut row = Vec::new();
        let mut x = 0;
        for run in &line.runs {
            for c in run.text.chars().filter(|c| !c.is_control()) {
                let advance = glyph(font, glyphs, c).advance + U * usize::from(run.bold);
                if x + advance > WIDTH && !row.is_empty() {
                    let cut = row
                        .iter()
                        .rposition(|g: &(char, u32, bool, usize)| g.0 == ' ')
                        .map_or(row.len(), |i| i + 1);
                    let rest: Vec<_> = row.split_off(cut);
                    rows.push(std::mem::take(&mut row));
                    let shift = rest.first().map_or(0, |g| g.3);
                    row = rest
                        .into_iter()
                        .map(|(c, k, b, gx)| (c, k, b, gx - shift))
                        .collect();
                    x = row.last().map_or(0, |g| {
                        g.3 + glyph(font, glyphs, g.0).advance + U * usize::from(g.2)
                    });
                }
                row.push((c, run.color, run.bold, x));
                x += advance;
            }
        }
        rows.push(row);
    }
    let used = rows
        .iter()
        .filter_map(|r| {
            r.last()
                .map(|g| g.3 + glyph(font, glyphs, g.0).advance + 2 * U)
        })
        .max()
        .unwrap_or(1);
    let (w, h) = (used + 2 * PAD, rows.len() * LINE_H + 2 * PAD);
    // Transparent: the glyphs' shadows keep them readable on Discord's light and dark themes.
    let mut rgba = vec![0; w * h * 4];
    for (r, row) in rows.iter().enumerate() {
        for &(c, color, bold, gx) in row {
            let shadow = (color & 0xFC_FC_FC) >> 2;
            for (offset, k) in [(U, shadow), (0, color)] {
                for &(px, py) in &glyph(font, glyphs, c).pixels {
                    for b in 0..=usize::from(bold) {
                        let (x, y) = (
                            PAD + gx + px + offset + b * U,
                            PAD + r * LINE_H + py + offset,
                        );
                        if let Some(p) = rgba.get_mut((y * w + x) * 4..(y * w + x) * 4 + 4) {
                            p.copy_from_slice(&[(k >> 16) as u8, (k >> 8) as u8, k as u8, 255]);
                        }
                    }
                }
            }
        }
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_posted_can_ping() {
        for (input, banned) in [
            ("@everyone look", "@everyone"),
            ("hey @here", "@here"),
            ("<@123456789>", "<@1"),
            ("<@&42> role", "<@&"),
            ("<#99> chan", "<#9"),
        ] {
            let out = defuse(input, true);
            assert!(!defuse(input, false).contains(banned));
            assert!(!out.contains(banned), "{input} -> {out}");
            assert!(out.contains(ZWJ));
        }
        assert_eq!(defuse("**bold** _x_", true), "\\*\\*bold\\*\\* \\_x\\_");
        assert_eq!(defuse("**bold**", false), "**bold**");
        // The webhook body asks Discord to parse no mention at all, picture or plain text.
        let line = Line::from_event(
            &json!({"type": "chat", "name": "Kim0", "uuid": "1", "message": "@everyone"}),
        )
        .unwrap();
        assert_eq!(line.plain(), "<Kim0> @everyone");
        assert!(!defuse(&line.plain(), false).contains("@everyone"));
        for picture in [true, false] {
            let p = payload(std::slice::from_ref(&line), picture).to_string();
            assert!(p.contains(r#""allowed_mentions":{"parse":[]}"#), "{p}");
            assert!(!p.contains("@everyone") && p.contains("Kim0"), "{p}");
        }
    }

    #[test]
    fn section_codes_colour_the_runs() {
        let runs = legacy("a§cred§lbold§rplain§9blue§zx", WHITE);
        let got: Vec<_> = runs
            .iter()
            .map(|r| (r.text.as_str(), r.color, r.bold))
            .collect();
        assert_eq!(
            got,
            [
                ("a", WHITE, false),
                ("red", 0xFF5555, false),
                ("bold", 0xFF5555, true),
                ("plain", WHITE, false),
                ("bluex", 0x5555FF, false)
            ]
        );
        let join =
            Line::from_event(&json!({"type": "join", "name": "Kim0", "address": "10.0.0.1"}))
                .unwrap();
        assert_eq!(join.runs[0].color, YELLOW);
        assert!(
            !join.plain().contains("10.0.0.1"),
            "never a player's address"
        );
        assert!(Line::from_event(&json!({"type": "log", "line": "x"})).is_none());
        // A nickname never stands in for the account's name.
        let nicked = json!({"type": "chat", "name": "Kim0", "display_name": "Bob", "nick": "Bob", "message": "hi"});
        assert_eq!(Line::from_event(&nicked).unwrap().plain(), "<Kim0> hi");
        let death = |m: &str| {
            Line::from_event(&json!({"type": "death", "name": "Kim0", "message": m}))
                .unwrap()
                .plain()
        };
        assert_eq!(
            death("Kim0 fell from a high place"),
            "Kim0 fell from a high place"
        );
        assert_eq!(
            death("Bob fell from a high place"),
            "Bob fell from a high place (Kim0)"
        );
        // A message the server's chat filter held back never reaches Discord, whatever the settings say.
        let filtered = json!({"type": "chat_filtered", "name": "Kim0", "message": "x"});
        assert!(Line::from_event(&filtered).is_none());
        let asked = Settings {
            events: vec!["chat_filtered".into()],
            ..Settings::default()
        };
        assert!(asked.check().is_err());
    }

    #[test]
    fn the_picture_is_the_chat_box() {
        let font = FontRef::try_from_slice(FONT).unwrap();
        let mut glyphs = HashMap::new();
        let line = |m: &str| {
            Line::from_event(&json!({"type": "chat", "name": "Kim0", "message": m})).unwrap()
        };
        let png = render(&[line("hi"), line("§ahello there")], &font, &mut glyphs).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let info = decoder.read_info().unwrap().info().clone();
        assert_eq!(info.height as usize, 2 * LINE_H + 2 * PAD);
        assert!(info.width as usize > 20 * U && (info.width as usize) < WIDTH + 2 * PAD + 2 * U);
        // A long message wraps at the chat's width instead of making a wider picture.
        let long = render(&[line(&"word ".repeat(80))], &font, &mut glyphs).unwrap();
        let info = png::Decoder::new(std::io::Cursor::new(&long))
            .read_info()
            .unwrap()
            .info()
            .clone();
        assert!(info.width as usize <= WIDTH + 2 * PAD + 2 * U);
        assert!(info.height as usize > 3 * LINE_H);
        // Glyphs are crisp pixels: an `l` is drawn, inside its line.
        assert!(!glyph(&font, &mut glyphs, 'l').pixels.is_empty());
    }

    #[test]
    fn one_post_per_gap_or_longer_when_told() {
        let mut limiter = Limiter::default();
        let now = Instant::now();
        assert!(limiter.ready(now));
        limiter.posted(now, Duration::ZERO);
        assert!(!limiter.ready(now + Duration::from_millis(1999)));
        assert!(limiter.ready(now + GAP));
        limiter.posted(now, Duration::from_secs(7));
        assert!(!limiter.ready(now + Duration::from_secs(6)));
        assert!(limiter.ready(now + Duration::from_secs(7)));
        assert_eq!(secs(Some("1.5")), Duration::from_millis(1500));
        assert_eq!(secs(Some("9999")), Duration::from_secs(60));
    }

    #[test]
    fn only_discord_webhooks() {
        let with = |hook: &str| {
            Settings {
                webhook: hook.into(),
                ..Settings::default()
            }
            .check()
        };
        assert!(with("").is_ok());
        assert!(with("https://discord.com/api/webhooks/1/abc").is_ok());
        assert!(with("http://discord.com/api/webhooks/1/abc").is_err());
        assert!(with("https://discord.com.evil.example/api/webhooks/1").is_err());
        assert!(with("https://example.com/api/webhooks/1").is_err());
        let bad = Settings {
            events: vec!["log".into()],
            ..Settings::default()
        };
        assert!(bad.check().is_err());
    }
}
