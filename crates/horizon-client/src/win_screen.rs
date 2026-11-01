//! `WinScreen`: the end poem, the credits and the post-credits text rolling up
//! under the logo, after the player leaves the End for the first time (with
//! the poem) or from Credits & Attribution (the credits alone). The texts come
//! from the resource root, so resource packs can replace them.
use crate::{
    font::Font,
    hud::{Blend, EndPortalFill, HudAtlas, HudBatch},
    text::{self, Style, Text},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::{collections::HashSet, path::Path};

/// `WinScreen.OBFUSCATE_TOKEN`: `§f§k§a§b` marks where the poem hides a name.
const OBFUSCATE_TOKEN: &str = "\u{a7}f\u{a7}k\u{a7}a\u{a7}b";
/// `WinScreen.NAME_PREFIX`.
const NAME_PREFIX: &str = "           ";
/// `WinScreen.SECTION_HEADING`.
const SECTION_HEADING: &str = "============";
/// A key the screen reacts to (`keyPressed` / `keyReleased`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// `KeyEvent.isUp`: the up arrow scrolls back.
    Up,
    /// Space speeds the scroll up.
    Space,
    /// Each held Control key speeds it up more.
    LeftControl,
    RightControl,
}
impl Key {
    pub fn from_code(code: winit::keyboard::KeyCode) -> Option<Self> {
        use winit::keyboard::KeyCode;
        match code {
            KeyCode::ArrowUp => Some(Self::Up),
            KeyCode::Space => Some(Self::Space),
            KeyCode::ControlLeft => Some(Self::LeftControl),
            KeyCode::ControlRight => Some(Self::RightControl),
            _ => None,
        }
    }
}
pub struct WinScreen {
    pub poem: bool,
    lines: Vec<Text>,
    centered: HashSet<usize>,
    total_scroll_length: i32,
    scroll: f32,
    speedup_active: bool,
    speedup_modifiers: HashSet<Key>,
    unmodified_scroll_speed: f32,
    scroll_speed: f32,
    direction: i32,
    /// `narratorComponents` joined for narration (`getNarrationMessage`):
    /// the poem's lines as read, the credits' names and titles (not the
    /// section rules), an empty part for each empty line.
    narration: String,
}
impl WinScreen {
    /// `WinScreen.<init>` and `init`: the poem (`texts/end.txt`) when `poem`,
    /// the credits (`texts/credits.json`), then `texts/postcredits.txt`.
    pub fn new(poem: bool, root: &Path, player: &str, font: &Font) -> Self {
        let speed = if poem { 0.5 } else { 0.75 };
        let mut screen = Self {
            poem,
            lines: Vec::new(),
            centered: HashSet::new(),
            total_scroll_length: 0,
            scroll: 0.0,
            speedup_active: false,
            speedup_modifiers: HashSet::new(),
            unmodified_scroll_speed: speed,
            scroll_speed: speed,
            direction: 1,
            narration: String::new(),
        };
        let mut narrated = Vec::new();
        let texts = root.join("assets/minecraft/texts");
        let read = |name: &str| match std::fs::read_to_string(texts.join(name)) {
            Ok(s) => Some(s),
            Err(e) => {
                log::error!("Couldn't load credits from file minecraft:texts/{name}: {e}");
                None
            }
        };
        if poem && let Some(s) = read("end.txt") {
            screen.add_poem_file(&s, player, font, &mut narrated);
        }
        if let Some(s) = read("credits.json") {
            match serde_json::from_str(&s) {
                Ok(json) => screen.add_credits_file(&json, &mut narrated),
                Err(e) => {
                    log::error!("Couldn't load credits from file minecraft:texts/credits.json: {e}")
                }
            }
        }
        if poem && let Some(s) = read("postcredits.txt") {
            screen.add_poem_file(&s, player, font, &mut narrated);
        }
        screen.total_scroll_length = screen.lines.len() as i32 * 12;
        screen.narration = narrated.join(". ");
        screen
    }
    /// What the screen narrates: no title of its own
    /// (`GameNarrator.NO_TITLE`), its `getNarrationMessage` is
    /// `joinForNarration` of the narrator's parts.
    pub fn screen_info(&self) -> crate::widgets::ScreenInfo {
        crate::widgets::ScreenInfo::new("win", self.narration.clone())
    }
    /// `addPoemFile`: each line with the player's name in, its obfuscated
    /// spans turned into 3 to 6 scrambled characters, wrapped at 256 pixels
    /// and followed by an empty line; 8 more empty lines at the end.
    fn add_poem_file(&mut self, file: &str, player: &str, font: &Font, narrated: &mut Vec<String>) {
        let mut random = LegacyRandom::new(8124371);
        // BufferedReader.readLine: \n, \r or \r\n end a line; no empty last line.
        let mut lines: Vec<&str> = file
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        if file.ends_with('\n') {
            lines.pop();
        }
        for line in lines.iter().flat_map(|l| l.split('\r')) {
            let mut line = line.replace("PLAYERNAME", player);
            while let Some(pos) = line.find(OBFUSCATE_TOKEN) {
                let n = random.next_int_bound(4).unwrap_or(0) as usize + 3;
                line = format!(
                    "{}\u{a7}f\u{a7}k{}{}",
                    &line[..pos],
                    &"XXXXXXXX"[..n],
                    &line[pos + OBFUSCATE_TOKEN.len()..]
                );
            }
            let text = Text::legacy(&line);
            self.lines
                .extend(font.split_lines(&text, 256.0).into_iter().map(|(t, _)| t));
            // addPoemLines: the line as read, its codes and all
            // (`Component.literal(line)`).
            narrated.push(line);
            self.add_empty_line(narrated);
        }
        for _ in 0..8 {
            self.add_empty_line(narrated);
        }
    }
    /// `addCreditsFile`: each section's heading, then each discipline's name
    /// and its titles with the names under them.
    fn add_credits_file(&mut self, json: &serde_json::Value, narrated: &mut Vec<String>) {
        let colored = |s: &str, color: u32| Text::styled(s, Style::color(color));
        let heading = || colored(SECTION_HEADING, text::WHITE);
        for section in json.as_array().into_iter().flatten() {
            let name = section["section"].as_str().unwrap_or_default();
            self.add_credits_line(heading(), true, None);
            self.add_credits_line(colored(name, text::YELLOW), true, Some(narrated));
            self.add_credits_line(heading(), true, None);
            self.add_empty_line(narrated);
            self.add_empty_line(narrated);
            for discipline in section["disciplines"].as_array().into_iter().flatten() {
                let name = discipline["discipline"].as_str().unwrap_or_default();
                if !name.is_empty() {
                    self.add_credits_line(colored(name, text::YELLOW), true, Some(narrated));
                    self.add_empty_line(narrated);
                    self.add_empty_line(narrated);
                }
                for title in discipline["titles"].as_array().into_iter().flatten() {
                    let name = title["title"].as_str().unwrap_or_default();
                    self.add_credits_line(colored(name, text::GRAY), false, Some(narrated));
                    for name in title["names"].as_array().into_iter().flatten() {
                        let line = format!("{NAME_PREFIX}{}", name.as_str().unwrap_or_default());
                        self.add_credits_line(colored(&line, text::WHITE), false, Some(narrated));
                    }
                    self.add_empty_line(narrated);
                    self.add_empty_line(narrated);
                }
            }
        }
    }
    fn add_empty_line(&mut self, narrated: &mut Vec<String>) {
        self.lines.push(Text::empty());
        narrated.push(String::new());
    }
    /// `addCreditsLine`; `narrated` when the narrator reads it too.
    fn add_credits_line(&mut self, line: Text, centered: bool, narrated: Option<&mut Vec<String>>) {
        if centered {
            self.centered.insert(self.lines.len());
        }
        if let Some(narrated) = narrated {
            narrated.push(line.plain());
        }
        self.lines.push(line);
    }
    /// The lines in order and whether each is centred.
    pub fn lines(&self) -> impl Iterator<Item = (&Text, bool)> {
        self.lines
            .iter()
            .enumerate()
            .map(|(i, l)| (l, self.centered.contains(&i)))
    }
    fn calculate_scroll_speed(&self) -> f32 {
        let speed = if self.speedup_active {
            self.unmodified_scroll_speed * (5.0 + self.speedup_modifiers.len() as f32 * 15.0)
        } else {
            self.unmodified_scroll_speed
        };
        speed * self.direction as f32
    }
    /// `keyPressed` / `keyReleased` (Escape closes the screen: `onClose`).
    pub fn key(&mut self, key: Key, pressed: bool) {
        match (key, pressed) {
            (Key::Up, _) => self.direction = if pressed { -1 } else { 1 },
            (Key::Space, _) => self.speedup_active = pressed,
            (_, true) => {
                self.speedup_modifiers.insert(key);
            }
            (_, false) => {
                self.speedup_modifiers.remove(&key);
            }
        }
        self.scroll_speed = self.calculate_scroll_speed();
    }
    /// `tick`: once the text has rolled past the top, the screen is done
    /// (`respawn`, or back to the screen that opened it).
    pub fn finished(&self, height: f32) -> bool {
        let height = height as i32;
        self.scroll > (self.total_scroll_length + height + height + 24) as f32
    }
    pub fn scroll(&self) -> f32 {
        self.scroll
    }
    /// Stay `scroll` pixels in (until a key changes the speed), for
    /// screenshots.
    pub fn hold(&mut self, scroll: f32) {
        self.scroll = scroll;
        self.scroll_speed = 0.0;
    }
    /// `extractRenderState` after a frame of `delta_ticks` game ticks:
    /// `extractBackground`, the vignette, then the logo and the lines
    /// scrolled up. The poem's End portal background samples `end_portal`;
    /// the credits draw the scrolling menu background (over the blurred
    /// panorama on the title screen).
    pub fn draw(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        delta_ticks: f32,
        end_portal: EndPortalFill,
    ) {
        let (w, h) = (batch.width, batch.height);
        if self.poem {
            let start = batch.vertices.len();
            batch.fill(atlas, 0.0, 0.0, w, h, u32::MAX);
            batch.blend_since(start, Blend::EndPortal);
            batch.end_portal = end_portal;
        } else {
            self.menu_background(batch, atlas, w, h);
        }
        // extractVignette: credits_vignette.png stretched over the screen.
        if let Some(r) = atlas.sprites.get("misc/credits_vignette").copied() {
            let start = batch.vertices.len();
            batch.rect_uv(0.0, 0.0, w, h, r, u32::MAX);
            batch.blend_since(start, Blend::Vignette);
        }
        self.scroll = (self.scroll + delta_ticks * self.scroll_speed).max(0.0);
        let logo_x = (w / 2.0).floor() - 128.0;
        let logo_y = h + 50.0;
        // Everything below is drawn translated by `y_offs`.
        let y_offs = -self.scroll;
        // LogoRenderer(false) at full alpha.
        let edition_x = (w / 2.0).floor() - 64.0;
        batch.sprite_region(
            atlas,
            "gui/title/minecraft",
            [logo_x, logo_y + y_offs, 256.0, 44.0],
            256.0,
            64.0,
        );
        batch.sprite_region(
            atlas,
            crate::title_edition::SPRITE,
            [edition_x, logo_y + 44.0 - 7.0 + y_offs, 128.0, 14.0],
            128.0,
            16.0,
        );
        let mut y_pos = logo_y + 100.0;
        let last = self.lines.len().saturating_sub(1);
        for (i, line) in self.lines.iter().enumerate() {
            let mut shift = y_offs;
            if i == last {
                // The last line stops in the middle of the screen.
                let diff = y_pos + y_offs - ((h as i32 / 2) as f32 - 6.0);
                if diff < 0.0 {
                    shift -= diff;
                }
            }
            if y_pos + y_offs + 12.0 + 8.0 > 0.0 && y_pos + y_offs < h {
                let x = if self.centered.contains(&i) {
                    // GuiGraphicsExtractor.centeredText: x - width / 2.
                    logo_x + 128.0 - (atlas.font.width_text(line) as i32 / 2) as f32
                } else {
                    logo_x
                };
                batch.text_styled(atlas, line, x, y_pos + shift, u32::MAX, true);
            }
            y_pos += 12.0;
        }
    }
    /// `extractMenuBackground`: `menu_background.png` tiled at 32 pixels,
    /// scrolled by half the text's scroll.
    fn menu_background(&self, batch: &mut HudBatch, atlas: &HudAtlas, w: f32, h: f32) {
        let v = (self.scroll * 0.5).rem_euclid(32.0);
        let start = batch.vertices.len();
        crate::ui::tiled(
            batch,
            atlas,
            "gui/menu_background",
            0.0,
            -v,
            w,
            h + 32.0,
            32.0,
            u32::MAX,
        );
        batch.clip_since(start, 0.0, 0.0, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resource root with only the credits texts: every glyph is the
    /// missing one, 6 pixels wide.
    fn screen(poem: bool, end: &str, credits: &str, post: &str) -> WinScreen {
        // One directory per call: tests run in parallel, and two of them with
        // the same arguments used to remove each other's files.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("horizon-win-screen-{}-{n}", std::process::id()));
        let texts = root.join("assets/minecraft/texts");
        std::fs::create_dir_all(&texts).unwrap();
        std::fs::write(texts.join("end.txt"), end).unwrap();
        std::fs::write(texts.join("credits.json"), credits).unwrap();
        std::fs::write(texts.join("postcredits.txt"), post).unwrap();
        let atlas = HudAtlas::fonts(&root).unwrap();
        let screen = WinScreen::new(poem, &root, "Steve", &atlas.font);
        let _ = std::fs::remove_dir_all(&root);
        screen
    }
    const CREDITS: &str = r#"[{"section":"Studio","disciplines":[
        {"discipline":"","titles":[{"title":"Head","names":["Ada","Bo"]}]},
        {"discipline":"Art","titles":[{"title":"Artist","names":[]}]}]}]"#;

    #[test]
    fn the_poem_names_the_player_and_scrambles_its_hidden_names() {
        let s = screen(
            true,
            "\u{a7}3Hello PLAYERNAME.\r\n\n\u{a7}2\u{a7}f\u{a7}k\u{a7}a\u{a7}bkey\n",
            "[]",
            "End.",
        );
        let lines: Vec<(String, bool)> = s.lines().map(|(t, c)| (t.plain(), c)).collect();
        // Each line then an empty one; the empty line adds no text line; 8
        // empty lines after each file.
        let mut want = vec!["Hello Steve.", ""];
        want.push("");
        // LegacyRandom(8124371).nextInt(4) + 3 scrambled characters.
        let n = LegacyRandom::new(8124371).next_int_bound(4).unwrap() as usize + 3;
        let hidden = format!("{}key", &"XXXXXXXX"[..n]);
        want.extend([hidden.as_str(), ""]);
        want.extend([""; 8]);
        want.extend(["End.", ""]);
        want.extend([""; 8]);
        assert_eq!(
            lines,
            want.iter()
                .map(|l| (l.to_string(), false))
                .collect::<Vec<_>>()
        );
        let hidden_line = s.lines().nth(3).unwrap().0;
        let styles: Vec<_> = hidden_line.chars().map(|(_, s)| s.obfuscated).collect();
        assert!(styles[..n].iter().all(|o| *o == Some(true)));
        assert_eq!(s.total_scroll_length, lines.len() as i32 * 12);
    }

    #[test]
    fn long_poem_lines_wrap_at_256_pixels() {
        // 6-pixel glyphs: 42 fit a line, and the space at each break goes.
        let line = vec!["a"; 50].join(" ");
        let s = screen(true, &line, "[]", "");
        let lines: Vec<String> = s.lines().take(4).map(|(t, _)| t.plain()).collect();
        assert_eq!(
            lines.iter().map(|l| l.chars().count()).collect::<Vec<_>>(),
            [41, 41, 15, 0]
        );
        assert!(lines.iter().all(|l| !l.starts_with(' ')));
    }
    #[test]
    fn credits_centre_the_headings_and_list_the_names() {
        let s = screen(false, "ignored", CREDITS, "ignored");
        let lines: Vec<(String, bool)> = s.lines().map(|(t, c)| (t.plain(), c)).collect();
        let l = |s: &str, c| (s.to_string(), c);
        let names = |n: &str| l(&format!("{NAME_PREFIX}{n}"), false);
        assert_eq!(
            lines,
            vec![
                l(SECTION_HEADING, true),
                l("Studio", true),
                l(SECTION_HEADING, true),
                l("", false),
                l("", false),
                l("Head", false),
                names("Ada"),
                names("Bo"),
                l("", false),
                l("", false),
                l("Art", true),
                l("", false),
                l("", false),
                l("Artist", false),
                l("", false),
                l("", false),
            ]
        );
        let color = |i: usize| s.lines().nth(i).unwrap().0.chars().next().unwrap().1.color;
        assert_eq!(
            [color(0), color(1), color(5), color(6), color(10)],
            [
                Some(text::WHITE),
                Some(text::YELLOW),
                Some(text::GRAY),
                Some(text::WHITE),
                Some(text::YELLOW)
            ]
        );
        // narratorComponents: the section rules aren't read, empty lines are
        // empty parts.
        let p = NAME_PREFIX;
        assert_eq!(
            s.screen_info().title,
            format!("Studio. . . Head. {p}Ada. {p}Bo. . . Art. . . Artist. . ")
        );
    }

    #[test]
    fn the_poem_is_narrated_as_read_line_by_line() {
        let s = screen(true, "Hello PLAYERNAME.\n", "[]", "End.");
        let empty = ". ".repeat(9);
        let poem = format!("Hello Steve.{empty}. End.{empty}");
        assert_eq!(s.screen_info().title, poem);
        assert_eq!(s.screen_info().id, "win");
    }

    #[test]
    fn keys_set_the_scroll_speed_and_direction() {
        let mut s = screen(false, "", CREDITS, "");
        assert_eq!(s.scroll_speed, 0.75);
        s.key(Key::Space, true);
        assert_eq!(s.scroll_speed, 0.75 * 5.0);
        s.key(Key::LeftControl, true);
        s.key(Key::RightControl, true);
        assert_eq!(s.scroll_speed, 0.75 * 35.0);
        s.key(Key::Up, true);
        assert_eq!(s.scroll_speed, -0.75 * 35.0);
        s.key(Key::Space, false);
        assert_eq!(s.scroll_speed, -0.75);
        s.key(Key::Up, false);
        s.key(Key::LeftControl, false);
        assert_eq!(s.scroll_speed, 0.75);
        // The poem rolls at 0.5 pixels a tick.
        assert_eq!(screen(true, "", CREDITS, "").scroll_speed, 0.5);
    }

    #[test]
    fn the_screen_ends_once_everything_rolled_past() {
        let mut s = screen(false, "", CREDITS, "");
        let atlas = HudAtlas::fonts(Path::new("/nonexistent")).unwrap();
        let mut batch = HudBatch::new([640, 480]);
        let h = batch.height;
        let end = (s.total_scroll_length as f32 + 2.0 * h + 24.0) / 0.75;
        s.draw(&mut batch, &atlas, end - 1.0, EndPortalFill::default());
        assert!(!s.finished(h));
        // Scrolling back never goes above the start.
        s.key(Key::Up, true);
        s.draw(&mut batch, &atlas, end * 2.0, EndPortalFill::default());
        assert_eq!(s.scroll(), 0.0);
        s.key(Key::Up, false);
        s.draw(&mut batch, &atlas, end + 1.0, EndPortalFill::default());
        assert!(s.finished(h));
    }
}
