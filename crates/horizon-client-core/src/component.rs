//! Text components from commands: `ComponentArgument` reads an SNBT (or JSON)
//! value, `ComponentSerialization` turns it into styled text.
use crate::text::{Style, Text, named_color};
use horizon_world::nbt::Tag;
use serde_json::{Map, Value};

/// `SnbtGrammar`, enough for components: compounds, lists, quoted and
/// unquoted strings, numbers (with type suffixes) and booleans. Returns the
/// value and the bytes consumed.
pub fn parse_snbt(input: &str) -> Result<(Value, usize), String> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    Ok((v, p.i))
}
/// Nesting deeper than NBT's own limit (`NbtAccounter`'s 512) is refused
/// rather than overflowing the stack.
const SNBT_MAX_DEPTH: usize = 512;
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}
impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c as char))
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let v = self.object();
                self.depth -= 1;
                v
            }
            Some(b'[') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let v = self.array();
                self.depth -= 1;
                v
            }
            Some(b'"' | b'\'') => Ok(Value::String(self.string()?)),
            Some(_) => {
                let word = self.unquoted();
                if word.is_empty() {
                    return Err("expected a value".into());
                }
                Ok(match word.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => {
                        let digits = word
                            .trim_end_matches(['b', 'B', 's', 'S', 'l', 'L', 'f', 'F', 'd', 'D']);
                        // Only digits (and an exponent) make a number: `nan`
                        // or `Infinity`, which Rust's parse would take, stays
                        // a string.
                        if digits
                            .bytes()
                            .any(|c| c.is_ascii_alphabetic() && c != b'e' && c != b'E')
                        {
                            Value::String(word)
                        } else {
                            match (digits.parse::<i64>(), digits.parse::<f64>()) {
                                (Ok(n), _) if digits.len() + 1 >= word.len() => Value::from(n),
                                (_, Ok(f)) if digits.len() + 1 >= word.len() => Value::from(f),
                                _ => Value::String(word),
                            }
                        }
                    }
                })
            }
            None => Err("expected a value".into()),
        }
    }
    fn object(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut map = Map::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.expect(b':')?;
            map.insert(key, self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err("expected ',' or '}'".into()),
            }
        }
    }
    fn array(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut list = Vec::new();
        self.ws();
        // Typed arrays [B;...], [I;...], [L;...].
        if matches!(
            self.s.get(self.i..self.i + 2),
            Some([b'B' | b'I' | b'L', b';'])
        ) {
            self.i += 2;
        }
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(list));
        }
        loop {
            list.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(list));
                }
                _ => return Err("expected ',' or ']'".into()),
            }
        }
    }
    fn unquoted(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+') {
                self.i += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }
    fn string(&mut self) -> Result<String, String> {
        self.ws();
        let Some(q @ (b'"' | b'\'')) = self.peek() else {
            return Ok(self.unquoted());
        };
        self.i += 1;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.i += 1;
                    out.push(match e {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });
                }
                c if c == q => return Ok(String::from_utf8_lossy(&out).into_owned()),
                c => out.push(c),
            }
        }
        Err("unterminated string".into())
    }
}
/// The style fields of a component object.
fn object_style(o: &serde_json::Map<String, Value>) -> Style {
    let mut style = Style::default();
    if let Some(c) = o.get("color").and_then(Value::as_str) {
        style.color = named_color(c).or_else(|| {
            c.strip_prefix('#')
                .and_then(|h| u32::from_str_radix(h, 16).ok())
        });
    }
    let flag = |k: &str| o.get(k).and_then(Value::as_bool);
    style.bold = flag("bold");
    style.italic = flag("italic");
    style.underlined = flag("underlined");
    style.strikethrough = flag("strikethrough");
    style.obfuscated = flag("obfuscated");
    style.font = o.get("font").and_then(crate::text::font_id);
    // ClickEvent.CODEC: the actions a chat line acts on here. A component
    // from a server never opens a file.
    if let Some(e) = o.get("click_event").and_then(Value::as_object) {
        let text = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
        style.click = match e.get("action").and_then(Value::as_str) {
            // ClickEvent.OpenUrl.CODEC: ExtraCodecs.UNTRUSTED_URI allows http and
            // https only, the scheme in any case (Util.parseAndValidateUntrustedUri).
            Some("open_url") => text("url")
                .filter(|u| {
                    u.split_once(':').is_some_and(|(scheme, _)| {
                        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
                    })
                })
                .map(crate::text::ClickEvent::OpenUrl),
            // ClickEvent.ChangePage.CODEC: ExtraCodecs.POSITIVE_INT.
            Some("change_page") => e
                .get("page")
                .and_then(Value::as_i64)
                .and_then(|p| i32::try_from(p).ok())
                .filter(|p| *p >= 1)
                .map(crate::text::ClickEvent::ChangePage),
            Some("suggest_command") => text("command").map(crate::text::ClickEvent::SuggestCommand),
            Some("run_command") => text("command").map(crate::text::ClickEvent::RunCommand),
            Some("copy_to_clipboard") => {
                text("value").map(crate::text::ClickEvent::CopyToClipboard)
            }
            // ClickEvent.ShowDialog.CODEC: a `minecraft:dialog` registry id or
            // the dialog itself; the server may send it (isAllowedFromServer).
            Some("show_dialog") => e.get("dialog").map(|dialog| {
                crate::text::ClickEvent::ShowDialog(crate::scoreboard::json_to_tag(dialog))
            }),
            _ => None,
        };
    }
    // HoverEvent.ShowText and ShowItem.
    if let Some(e) = o.get("hover_event").and_then(Value::as_object) {
        match e.get("action").and_then(Value::as_str) {
            Some("show_text") => {
                if let Some(value) = e.get("value") {
                    style.hover = Some(Box::new(to_text(value, &mut |s| Text::literal(s))));
                }
            }
            Some("show_item") if e.get("id").is_some_and(Value::is_string) => {
                style.hover_item = Some(Box::new(Value::Object(e.clone())));
            }
            _ => {}
        }
    }
    style.insertion = o
        .get("insertion")
        .and_then(Value::as_str)
        .map(str::to_owned);
    style
}
/// `ComponentSerialization.CODEC` decoding into text. `selector` components
/// are resolved by `resolve` (ComponentUtils.updateForEntity).
pub fn to_text(v: &Value, resolve: &mut dyn FnMut(&str) -> Text) -> Text {
    to_text_with(v, resolve, &mut |_| None)
}
/// `to_text` with a hook resolving other contents (`nbt`) from the object.
pub fn to_text_with(
    v: &Value,
    resolve: &mut dyn FnMut(&str) -> Text,
    contents: &mut dyn FnMut(&serde_json::Map<String, Value>) -> Option<Text>,
) -> Text {
    match v {
        Value::String(s) => Text::literal(s.clone()),
        Value::Number(n) => Text::literal(n.to_string()),
        Value::Bool(b) => Text::literal(b.to_string()),
        // A list is its first element with the rest as siblings.
        Value::Array(list) => {
            let Some((first, rest)) = list.split_first() else {
                return Text::empty();
            };
            let mut base = to_text_with(first, resolve, contents);
            // The siblings take the first component's own style, even when
            // its text is empty.
            let style = match first {
                Value::Object(o) => object_style(o),
                _ => base
                    .spans
                    .first()
                    .map(|s| s.style.clone())
                    .unwrap_or_default(),
            };
            for sibling in rest {
                base.push(to_text_with(sibling, resolve, contents).with_style(&style));
            }
            base
        }
        Value::Object(o) => {
            let style = object_style(o);
            let content = if let Some(t) = contents(o) {
                t
            } else if let Some(t) = o.get("text") {
                match t {
                    Value::String(s) => Text::literal(s.clone()),
                    other => Text::literal(other.to_string()),
                }
            } else if let Some(key) = o.get("translate").and_then(Value::as_str) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|x| to_text_with(x, resolve, contents))
                            .collect()
                    })
                    .unwrap_or_default();
                let translated = Text::translate(key, &args);
                match o.get("fallback").and_then(Value::as_str) {
                    Some(fallback) if translated.plain() == key => Text::translate(fallback, &args),
                    _ => translated,
                }
            } else if let Some(sel) = o.get("selector").and_then(Value::as_str) {
                resolve(sel)
            } else if let Some(key) = o.get("keybind").and_then(Value::as_str) {
                crate::key_display::display_name(key)
            } else {
                Text::empty()
            };
            let mut out = content.with_style(&style);
            for extra in o
                .get("extra")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push(to_text_with(extra, resolve, contents).with_style(&style));
            }
            out
        }
        Value::Null => Text::empty(),
    }
}

/// `CustomName` as text: a string keeps its legacy formatting codes, anything
/// else is a component (`ComponentSerialization`).
pub fn name_text(tag: &Tag) -> Text {
    match tag {
        Tag::String(s) => Text::legacy(&s.to_string_lossy()),
        t => crate::scoreboard::comp_text(&crate::scoreboard::tag_to_json(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snbt_and_json_components() {
        let input = "{text:'Hi ',color:red,extra:[{text:\"there\",bold:1b}]} rest";
        let (v, n) = parse_snbt(input).unwrap();
        assert_eq!(&input[n..], " rest");
        let t = to_text(&v, &mut |_| Text::empty());
        assert_eq!(t.plain(), "Hi there");
        assert_eq!(t.spans[1].style.color, named_color("red"));
        let (v, _) = parse_snbt("[\"a\",{\"text\":\"b\"}]").unwrap();
        assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "ab");
    }

    #[test]
    fn the_font_is_inherited_and_kept() {
        let (v, _) =
            parse_snbt(r#"{text:"a",font:"alt",extra:["b",{text:"c",font:"minecraft:uniform"}]}"#)
                .unwrap();
        let t = to_text(&v, &mut |_| Text::empty());
        let fonts: Vec<_> = t.chars().map(|(_, s)| s.font.clone()).collect();
        let (alt, uniform) = (
            Some("minecraft:alt".into()),
            Some("minecraft:uniform".into()),
        );
        assert_eq!(fonts, [alt.clone(), alt, uniform]);
        // Back to JSON with the field, which a re-read keeps.
        let again = to_text(&t.component(), &mut |_| Text::empty());
        let kept: Vec<_> = again.chars().map(|(_, s)| s.font.clone()).collect();
        assert_eq!(kept, fonts);
        // Not an identifier: no font.
        let (v, _) = parse_snbt(r#"{text:"a",font:"Bad Font"}"#).unwrap();
        let t = to_text(&v, &mut |_| Text::empty());
        assert_eq!(t.spans[0].style.font, None);
    }

    #[test]
    fn snbt_nesting_is_bounded() {
        let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        assert!(parse_snbt(&nest(512)).is_ok());
        assert!(parse_snbt(&nest(513)).is_err());
        assert!(parse_snbt(&nest(100_000)).is_err());
    }

    #[test]
    fn bare_words_that_look_numeric_stay_strings() {
        let (v, _) = parse_snbt("{text:nan}").unwrap();
        assert_eq!(v["text"], Value::String("nan".into()));
        let (v, _) = parse_snbt("{text:Infinity}").unwrap();
        assert_eq!(v["text"], Value::String("Infinity".into()));
        let (v, _) = parse_snbt("{n:1.5}").unwrap();
        assert_eq!(v["n"], Value::from(1.5));
    }
}
