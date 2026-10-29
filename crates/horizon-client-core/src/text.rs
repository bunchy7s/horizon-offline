//! Styled text (a flattened `Component`): spans with ChatFormatting colours
//! and decorations, translatable messages formatted like
//! `TranslatableContents` (`%s`, `%1$s`, `%%`), and legacy `§` codes.
use crate::lang;
use serde_json::{Map, Value, json};

/// ChatFormatting colours by legacy code index 0..=15.
pub const COLORS: [(char, &str, u32); 16] = [
    ('0', "black", 0x000000),
    ('1', "dark_blue", 0x0000AA),
    ('2', "dark_green", 0x00AA00),
    ('3', "dark_aqua", 0x00AAAA),
    ('4', "dark_red", 0xAA0000),
    ('5', "dark_purple", 0xAA00AA),
    ('6', "gold", 0xFFAA00),
    ('7', "gray", 0xAAAAAA),
    ('8', "dark_gray", 0x555555),
    ('9', "blue", 0x5555FF),
    ('a', "green", 0x55FF55),
    ('b', "aqua", 0x55FFFF),
    ('c', "red", 0xFF5555),
    ('d', "light_purple", 0xFF55FF),
    ('e', "yellow", 0xFFFF55),
    ('f', "white", 0xFFFFFF),
];
pub const BLACK: u32 = 0x000000;
pub const GRAY: u32 = 0xAAAAAA;
pub const DARK_GRAY: u32 = 0x555555;
pub const GREEN: u32 = 0x55FF55;
pub const AQUA: u32 = 0x55FFFF;
pub const RED: u32 = 0xFF5555;
pub const YELLOW: u32 = 0xFFFF55;
pub const GOLD: u32 = 0xFFAA00;
pub const WHITE: u32 = 0xFFFFFF;
pub const LIGHT_PURPLE: u32 = 0xFF55FF;
pub const DARK_PURPLE: u32 = 0xAA00AA;

pub fn named_color(name: &str) -> Option<u32> {
    COLORS.iter().find(|c| c.1 == name).map(|c| c.2)
}

/// Click actions carried by locally constructed text components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickEvent {
    /// `ClickEvent.OpenUrl`: an http or https link.
    OpenUrl(String),
    /// `ClickEvent.ChangePage`: a book's page, counted from 1.
    ChangePage(i32),
    SuggestCommand(String),
    /// `ClickEvent.RunCommand`: the command, with or without its `/`.
    RunCommand(String),
    CopyToClipboard(String),
    /// `ClickEvent.ShowDialog`: a `minecraft:dialog` registry id, or the
    /// dialog itself as the component carried it.
    ShowDialog(horizon_world::nbt::Tag),
    /// A local file to open with the system handler.
    /// Only allow this in locally constructed messages, never server/LAN text.
    OpenFile(String),
}

/// Text style; `None` fields inherit from the enclosing style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    pub click: Option<ClickEvent>,
    /// Hover text.
    pub hover: Option<Box<Text>>,
    /// `HoverEvent.ShowItem`: the event's `{id, count?, components?}` object.
    pub hover_item: Option<Box<Value>>,
    /// `Style.insertion`: the text a Shift-click puts in the chat box.
    pub insertion: Option<String>,
    /// The font (`FontDescription.Resource`), a full id like
    /// `minecraft:alt`; the default font when unset.
    pub font: Option<String>,
}
/// A style's `font` field: an identifier (`alt` is `minecraft:alt`).
pub fn font_id(v: &Value) -> Option<String> {
    crate::identifier::parse_identifier(v.as_str()?)
}
impl Style {
    pub fn color(c: u32) -> Self {
        Self {
            color: Some(c),
            ..Self::default()
        }
    }
    /// Fill unset fields from `base` (Style.applyTo).
    pub fn inherit(&mut self, base: &Style) {
        self.color = self.color.or(base.color);
        self.bold = self.bold.or(base.bold);
        self.italic = self.italic.or(base.italic);
        self.underlined = self.underlined.or(base.underlined);
        self.strikethrough = self.strikethrough.or(base.strikethrough);
        self.obfuscated = self.obfuscated.or(base.obfuscated);
        if self.click.is_none() {
            self.click.clone_from(&base.click);
        }
        if self.hover.is_none() {
            self.hover.clone_from(&base.hover);
        }
        if self.hover_item.is_none() {
            self.hover_item.clone_from(&base.hover_item);
        }
        if self.insertion.is_none() {
            self.insertion.clone_from(&base.insertion);
        }
        if self.font.is_none() {
            self.font.clone_from(&base.font);
        }
    }
    pub fn is_bold(&self) -> bool {
        self.bold == Some(true)
    }
    pub fn is_italic(&self) -> bool {
        self.italic == Some(true)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}
/// A run of styled spans, and the `Component` they came from (its JSON form)
/// when it was built through the constructors below, for sending to vanilla
/// clients. Equality compares the spans only.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub spans: Vec<Span>,
    source: Option<std::sync::Arc<Value>>,
}
impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.spans == other.spans
    }
}
impl Eq for Text {}
impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::literal(s)
    }
}
impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::literal(s)
    }
}
impl Text {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn literal(s: impl Into<String>) -> Self {
        let text = s.into();
        if text.is_empty() {
            return Self::default();
        }
        Self {
            source: Some(json!({ "text": text }).into()),
            spans: vec![Span {
                text,
                style: Style::default(),
            }],
        }
    }
    pub fn styled(s: impl Into<String>, style: Style) -> Self {
        Self::literal(s).with_style(&style)
    }
    pub fn colored(s: impl Into<String>, color: u32) -> Self {
        Self::styled(s, Style::color(color))
    }
    /// ComponentUtils.copyOnClickText: a green, bracketed clipboard link.
    pub fn copyable(s: impl Into<String>) -> Self {
        let s = s.into();
        Self::literal("[")
            .append(Self::styled(
                s.clone(),
                Style {
                    click: Some(ClickEvent::CopyToClipboard(s)),
                    hover: Some(Box::new(Self::translate("chat.copy.click", &[]))),
                    ..Style::color(GREEN)
                },
            ))
            .append("]")
    }
    /// LocateCommand's coordinate link: suggest a teleport without executing it.
    /// Structure and POI results preserve the player's current height (`~`).
    pub fn teleport_link(x: i32, y: Option<i32>, z: i32) -> Self {
        let y = y.map_or_else(|| "~".to_owned(), |y| y.to_string());
        let coords = Self::tr("chat.coordinates", &[&x, &y, &z]);
        Self::literal("[")
            .append(coords)
            .append("]")
            .with_style(&Style {
                click: Some(ClickEvent::SuggestCommand(format!("/tp @s {x} {y} {z}"))),
                hover: Some(Box::new(Self::translate("chat.coordinates.tooltip", &[]))),
                ..Style::color(GREEN)
            })
    }
    /// Component.translatable(key, args) in the current language.
    pub fn translate(key: &str, args: &[Text]) -> Self {
        let mut t = format_translation(&lang::get(key), args);
        let mut c = Map::new();
        c.insert("translate".into(), key.into());
        if !args.is_empty() {
            c.insert("with".into(), args.iter().map(Text::argument).collect());
        }
        t.source = Some(Value::Object(c).into());
        t
    }
    /// A translatable message whose arguments are plain strings.
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        let args: Vec<Text> = args.iter().map(|a| Text::literal(a.to_string())).collect();
        Self::translate(key, &args)
    }
    pub fn append(mut self, other: impl Into<Text>) -> Self {
        self.push(other);
        self
    }
    /// MutableComponent.append: `other` becomes a sibling. A styled text is
    /// wrapped first, as its style doesn't reach what is appended here.
    pub fn push(&mut self, other: impl Into<Text>) {
        let other = other.into();
        let sibling = other.component();
        let mut root = if self.spans.is_empty() && self.source.is_none() {
            json!({ "text": "" })
        } else {
            let c = self.component();
            if STYLE_KEYS.iter().any(|k| c.get(*k).is_some()) {
                json!({ "text": "", "extra": [c] })
            } else {
                c
            }
        };
        if let Value::Object(o) = &mut root {
            match o.get_mut("extra") {
                Some(Value::Array(a)) => a.push(sibling),
                _ => {
                    o.insert("extra".into(), Value::Array(vec![sibling]));
                }
            }
        }
        self.source = Some(root.into());
        self.spans.extend(other.spans);
    }
    /// Apply `style` underneath every span (MutableComponent.withStyle on a parent).
    pub fn with_style(mut self, style: &Style) -> Self {
        for span in &mut self.spans {
            span.style.inherit(style);
        }
        let mut c = self.component();
        if let Value::Object(o) = &mut c {
            for (k, v) in style_json(style) {
                o.entry(k).or_insert(v);
            }
        }
        self.source = Some(c.into());
        self
    }
    /// The `Component` as vanilla's JSON (ComponentSerialization): the one
    /// built through the constructors, else the spans as literal siblings.
    /// The key of a `Component.translatable` built through [`Text::translate`].
    pub fn translation_key(&self) -> Option<&str> {
        self.source.as_deref()?.get("translate")?.as_str()
    }
    pub fn component(&self) -> Value {
        if let Some(c) = &self.source {
            return (**c).clone();
        }
        let span = |s: &Span| {
            let mut o = Map::new();
            o.insert("text".into(), s.text.clone().into());
            o.extend(style_json(&s.style));
            Value::Object(o)
        };
        match self.spans.as_slice() {
            [] => json!({ "text": "" }),
            [one] => span(one),
            many => json!({ "text": "", "extra": many.iter().map(span).collect::<Vec<_>>() }),
        }
    }
    /// A translation argument: vanilla passes numbers as numbers, which
    /// Horizon formats into unstyled literals.
    fn argument(&self) -> Value {
        let c = self.component();
        match c
            .as_object()
            .and_then(|o| o.get("text").filter(|_| o.len() == 1))
        {
            Some(Value::String(s)) if s.parse::<i64>().is_ok_and(|n| n.to_string() == *s) => {
                s.parse::<i64>().map_or(c.clone(), Value::from)
            }
            _ => c,
        }
    }
    /// Style fields the spans don't model (an entity hover, an insertion),
    /// set on the component sent to vanilla only.
    pub fn with_component_fields(mut self, fields: Vec<(String, Value)>) -> Self {
        let mut c = self.component();
        if let Value::Object(o) = &mut c {
            for (k, v) in fields {
                o.entry(k).or_insert(v);
            }
        }
        self.source = Some(c.into());
        self
    }
    /// The same spans, sent to vanilla as `component` (a shape the spans'
    /// own JSON would not take, such as a styled root with siblings).
    pub fn with_component(mut self, component: Value) -> Self {
        self.source = Some(component.into());
        self
    }
    pub fn with_color(self, color: u32) -> Self {
        self.with_style(&Style::color(color))
    }
    pub fn italic(self) -> Self {
        self.with_style(&Style {
            italic: Some(true),
            ..Style::default()
        })
    }
    pub fn underlined(self) -> Self {
        self.with_style(&Style {
            underlined: Some(true),
            ..Style::default()
        })
    }
    pub fn bold(self) -> Self {
        self.with_style(&Style {
            bold: Some(true),
            ..Style::default()
        })
    }
    /// Component.getString: the plain characters.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }
    /// Parse legacy formatting codes (`§a`, `§l`, `§r`...) as the font does.
    pub fn legacy(s: &str) -> Self {
        let mut out = Text::default();
        let mut style = Style::default();
        let mut current = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '§' {
                current.push(c);
                continue;
            }
            let Some(code) = chars.next() else { break };
            if !current.is_empty() {
                out.spans.push(Span {
                    text: std::mem::take(&mut current),
                    style: style.clone(),
                });
            }
            let code = code.to_ascii_lowercase();
            if let Some(c) = COLORS.iter().find(|c| c.0 == code) {
                // Colour codes reset decorations (Style.applyLegacyFormat).
                style = Style::color(c.2);
            } else {
                match code {
                    'k' => style.obfuscated = Some(true),
                    'l' => style.bold = Some(true),
                    'm' => style.strikethrough = Some(true),
                    'n' => style.underlined = Some(true),
                    'o' => style.italic = Some(true),
                    'r' => style = Style::default(),
                    _ => {}
                }
            }
        }
        if !current.is_empty() {
            out.spans.push(Span {
                text: current,
                style,
            });
        }
        out
    }
    /// `StringDecomposer.iterateFormatted` on every span: legacy `§` codes
    /// inside the span contents restyle what follows them (colour codes
    /// clear the decorations, `§r` returns to the span's own style) and are
    /// not drawn.
    pub fn resolve_legacy(&self) -> Text {
        if !self.spans.iter().any(|s| s.text.contains('§')) {
            return self.clone();
        }
        let mut out = Text::default();
        for span in &self.spans {
            let mut style = span.style.clone();
            let mut current = String::new();
            let mut chars = span.text.chars();
            while let Some(c) = chars.next() {
                if c != '§' {
                    current.push(c);
                    continue;
                }
                let Some(code) = chars.next() else { break };
                let next = legacy_format(&style, code, &span.style);
                if next != style {
                    if !current.is_empty() {
                        out.spans.push(Span {
                            text: std::mem::take(&mut current),
                            style: style.clone(),
                        });
                    }
                    style = next;
                }
            }
            if !current.is_empty() {
                out.spans.push(Span {
                    text: current,
                    style,
                });
            }
        }
        out
    }
    /// `resolve_legacy` borrowing `self` when no span holds a `§` code (the
    /// common case).
    pub fn resolved(&self) -> std::borrow::Cow<'_, Text> {
        if self.spans.iter().any(|s| s.text.contains('§')) {
            std::borrow::Cow::Owned(self.resolve_legacy())
        } else {
            std::borrow::Cow::Borrowed(self)
        }
    }
    /// The text without its legacy `§` codes (ChatFormatting.stripFormatting:
    /// a `§` and a code character, `0`-`9`, `a`-`f`, `k`-`o` or `r`).
    pub fn strip_legacy(&self) -> Text {
        if !self.spans.iter().any(|s| s.text.contains('§')) {
            return self.clone();
        }
        let code =
            |c: char| matches!(c.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r');
        let mut out = Text::default();
        for span in &self.spans {
            let mut text = String::with_capacity(span.text.len());
            let mut chars = span.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '§' && chars.peek().is_some_and(|&n| code(n)) {
                    chars.next();
                } else {
                    text.push(c);
                }
            }
            if !text.is_empty() {
                out.spans.push(Span {
                    text,
                    style: span.style.clone(),
                });
            }
        }
        out
    }
    /// Characters with their effective styles.
    pub fn chars(&self) -> impl Iterator<Item = (char, &Style)> {
        self.spans
            .iter()
            .flat_map(|s| s.text.chars().map(move |c| (c, &s.style)))
    }
    /// Sub-range of characters [from, to) keeping styles.
    pub fn slice(&self, from: usize, to: usize) -> Text {
        let mut out = Text::default();
        let mut index = 0;
        for span in &self.spans {
            let mut part = String::new();
            for c in span.text.chars() {
                if index >= from && index < to {
                    part.push(c);
                }
                index += 1;
            }
            if !part.is_empty() {
                out.spans.push(Span {
                    text: part,
                    style: span.style.clone(),
                });
            }
        }
        out
    }
}

/// The style fields of a component's JSON.
const STYLE_KEYS: [&str; 10] = [
    "color",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "click_event",
    "hover_event",
    "insertion",
    "font",
];
/// Style.Serializer: the fields a style sets.
fn style_json(style: &Style) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(c) = style.color {
        let name = COLORS
            .iter()
            .find(|n| n.2 == c)
            .map_or_else(|| format!("#{c:06X}"), |n| n.1.to_owned());
        out.push(("color".to_owned(), name.into()));
    }
    for (key, flag) in [
        ("bold", style.bold),
        ("italic", style.italic),
        ("underlined", style.underlined),
        ("strikethrough", style.strikethrough),
        ("obfuscated", style.obfuscated),
    ] {
        if let Some(f) = flag {
            out.push((key.to_owned(), f.into()));
        }
    }
    if let Some(click) = &style.click {
        let event = match click {
            ClickEvent::OpenUrl(url) => json!({ "action": "open_url", "url": url }),
            ClickEvent::ChangePage(page) => json!({ "action": "change_page", "page": page }),
            ClickEvent::SuggestCommand(command) => {
                json!({ "action": "suggest_command", "command": command })
            }
            ClickEvent::CopyToClipboard(value) => {
                json!({ "action": "copy_to_clipboard", "value": value })
            }
            ClickEvent::RunCommand(command) => {
                json!({ "action": "run_command", "command": command })
            }
            // ClickEvent.ShowDialog.CODEC: the dialog the server sent it with.
            ClickEvent::ShowDialog(dialog) => json!({
                "action": "show_dialog",
                "dialog": crate::scoreboard::tag_to_json(dialog),
            }),
            ClickEvent::OpenFile(path) => json!({ "action": "open_file", "path": path }),
        };
        out.push(("click_event".to_owned(), event));
    }
    if let Some(hover) = &style.hover {
        out.push((
            "hover_event".to_owned(),
            json!({ "action": "show_text", "value": hover.component() }),
        ));
    } else if let Some(Value::Object(item)) = style.hover_item.as_deref() {
        let mut event = item.clone();
        event.insert("action".into(), "show_item".into());
        out.push(("hover_event".to_owned(), Value::Object(event)));
    }
    if let Some(insertion) = &style.insertion {
        out.push(("insertion".to_owned(), insertion.clone().into()));
    }
    if let Some(font) = &style.font {
        out.push(("font".to_owned(), font.clone().into()));
    }
    out
}

/// `Style.applyLegacyFormat` for the code after a `§`, `§r` going back to
/// `reset` (`StringDecomposer.iterateFormatted`); an unknown code changes nothing.
pub fn legacy_format(style: &Style, code: char, reset: &Style) -> Style {
    let code = code.to_ascii_lowercase();
    if let Some(c) = COLORS.iter().find(|c| c.0 == code) {
        // A colour clears the decorations.
        return Style {
            color: Some(c.2),
            bold: Some(false),
            italic: Some(false),
            underlined: Some(false),
            strikethrough: Some(false),
            obfuscated: Some(false),
            ..style.clone()
        };
    }
    let mut s = style.clone();
    match code {
        'k' => s.obfuscated = Some(true),
        'l' => s.bold = Some(true),
        'm' => s.strikethrough = Some(true),
        'n' => s.underlined = Some(true),
        'o' => s.italic = Some(true),
        'r' => s = reset.clone(),
        _ => {}
    }
    s
}

/// TranslatableContents.decomposeTemplate: `%s` takes the next argument,
/// `%n$s` a numbered one and `%%` is a percent sign. Malformed templates are
/// shown verbatim.
pub fn format_translation(template: &str, args: &[Text]) -> Text {
    let mut out = Text::default();
    let mut literal = String::new();
    let mut next = 0usize;
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            literal.push(chars[i]);
            i += 1;
            continue;
        }
        // %(?:(\d+)\$)?([A-Za-z%]|$)
        let mut j = i + 1;
        let mut number = None;
        let digits_start = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits_start && j < chars.len() && chars[j] == '$' {
            number = chars[digits_start..j]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .ok();
            j += 1;
        } else {
            j = digits_start;
        }
        let conversion = chars.get(j).copied();
        match conversion {
            Some('%') if number.is_none() => {
                literal.push('%');
                i = j + 1;
            }
            // Not a format specifier at all: the percent sign is literal.
            Some(c) if !c.is_ascii_alphabetic() && c != '%' => {
                literal.push('%');
                i += 1;
            }
            Some('s') => {
                let index = match number {
                    Some(n) => n.wrapping_sub(1),
                    None => {
                        next += 1;
                        next - 1
                    }
                };
                let Some(arg) = args.get(index) else {
                    return Text::literal(template);
                };
                if !literal.is_empty() {
                    out.spans.push(Span {
                        text: std::mem::take(&mut literal),
                        style: Style::default(),
                    });
                }
                out.spans.extend(arg.spans.iter().cloned());
                i = j + 1;
            }
            _ => return Text::literal(template),
        }
    }
    if !literal.is_empty() {
        out.spans.push(Span {
            text: literal,
            style: Style::default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn click_events_serialize_and_inherit_as_one_style_field() {
        for (event, json) in [
            (
                ClickEvent::SuggestCommand("/seed".into()),
                json!({"action": "suggest_command", "command": "/seed"}),
            ),
            (
                ClickEvent::CopyToClipboard("-9223372036854775808".into()),
                json!({"action": "copy_to_clipboard", "value": "-9223372036854775808"}),
            ),
            (
                ClickEvent::OpenFile("/tmp/My screenshots/世界.png".into()),
                json!({"action": "open_file", "path": "/tmp/My screenshots/世界.png"}),
            ),
        ] {
            let style = Style {
                click: Some(event.clone()),
                ..Style::default()
            };
            let text = Text::literal("link").with_style(&style);
            assert_eq!(text.component()["click_event"], json);
            assert_eq!(text.spans[0].style.click, Some(event));
            let child = Text::copyable("42").with_style(&style);
            assert_eq!(
                child.spans[1].style.click,
                Some(ClickEvent::CopyToClipboard("42".into()))
            );
            assert_eq!(
                child.component()["extra"][0]["click_event"],
                json!({"action": "copy_to_clipboard", "value": "42"})
            );
        }
    }

    #[test]
    fn formats_like_translatable_contents() {
        let a = [Text::literal("Steve"), Text::colored("Zombie", RED)];
        assert_eq!(
            format_translation("%s was slain by %s", &a).plain(),
            "Steve was slain by Zombie"
        );
        assert_eq!(
            format_translation("%2$s then %1$s", &a).plain(),
            "Zombie then Steve"
        );
        assert_eq!(
            format_translation("100%% sure %s", &a).plain(),
            "100% sure Steve"
        );
        // Missing arguments or unknown conversions show the template.
        assert_eq!(format_translation("%s %s %s", &a).plain(), "%s %s %s");
        assert_eq!(format_translation("%d", &a).plain(), "%d");
        assert_eq!(
            format_translation("50% off %s", &a).plain(),
            "50% off Steve"
        );
        assert_eq!(format_translation("end %", &a).plain(), "end %");
        let t = format_translation("%s was slain by %s", &a);
        assert_eq!(t.spans.last().unwrap().style.color, Some(RED));
    }
    #[test]
    fn legacy_codes_and_inheritance() {
        let t = Text::legacy("§aGreen §lbold§r plain");
        assert_eq!(t.plain(), "Green bold plain");
        assert_eq!(t.spans[0].style.color, Some(GREEN));
        assert!(t.spans[1].style.is_bold());
        assert_eq!(t.spans[2].style, Style::default());
        let t = Text::literal("x")
            .append(Text::colored("y", GOLD))
            .with_color(RED);
        assert_eq!(t.spans[0].style.color, Some(RED));
        assert_eq!(t.spans[1].style.color, Some(GOLD));
        assert_eq!(t.slice(1, 2).plain(), "y");
    }
    #[test]
    fn resolved_borrows_clean_text_and_restyles_codes() {
        let clean = Text::colored("Steve", GOLD);
        assert!(matches!(clean.resolved(), std::borrow::Cow::Borrowed(_)));
        // A colour code inside a span clears its decorations (a bold
        // "§cSteve" draws "Steve" red, not bold).
        let styled = Style {
            bold: Some(true),
            ..Style::default()
        };
        let styled = Text::styled("§cSteve", styled);
        let resolved = styled.resolved();
        assert_eq!(resolved.plain(), "Steve");
        assert_eq!(resolved.spans[0].style.color, Some(RED));
        assert_eq!(resolved.spans[0].style.bold, Some(false));
    }
}
