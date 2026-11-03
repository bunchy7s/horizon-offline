//! The JSON of `.jem` and `.jpm` files as OptiFine reads it (Gson, which is lenient): a byte
//! order mark, `//`, `/* */` and `#` comments, trailing commas, single-quoted strings and bare
//! keys are all accepted, and objects keep their keys in file order, which animations depend on
//! (later ones read what earlier ones wrote).

/// A JSON value. Objects are key-value lists in file order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Nesting deeper than this is refused: parsing recurses.
const DEPTH: usize = 128;

impl Json {
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            b: text.strip_prefix('\u{feff}').unwrap_or(text).as_bytes(),
            i: 0,
        };
        let v = p.value(0)?;
        p.space()?;
        if p.i < p.b.len() {
            return Err(p.error("text after the value"));
        }
        Ok(v)
    }

    /// The value of `key` in an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        get(self.as_object()?, key)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(pairs) => Some(pairs),
            _ => None,
        }
    }
}

/// The value of `key` (the last one, when a key repeats).
pub(crate) fn get<'a>(object: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    object.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        let line = 1 + self.b[..self.i.min(self.b.len())]
            .iter()
            .filter(|c| **c == b'\n')
            .count();
        format!("{what} on line {line}")
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// Skips whitespace and comments.
    fn space(&mut self) -> Result<(), String> {
        loop {
            match (self.peek(), self.b.get(self.i + 1)) {
                (Some(c), _) if c.is_ascii_whitespace() => self.i += 1,
                (Some(b'#'), _) | (Some(b'/'), Some(b'/')) => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    let end = self.b[self.i + 2..].windows(2).position(|w| w == b"*/");
                    let Some(end) = end else {
                        return Err(self.error("unclosed comment"));
                    };
                    self.i += end + 4;
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > DEPTH {
            return Err(self.error("nested too deep"));
        }
        self.space()?;
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(q @ (b'"' | b'\'')) => self.string(q).map(Json::Str),
            Some(_) => {
                let word = self.word();
                Ok(match word.as_str() {
                    "" => return Err(self.error("expected a value")),
                    "null" => Json::Null,
                    "true" => Json::Bool(true),
                    "false" => Json::Bool(false),
                    // Gson reads an unquoted value as a string when it isn't a number.
                    _ => word.parse().map_or(Json::Str(word), Json::Num),
                })
            }
            None => Err(self.error("unexpected end")),
        }
    }

    /// An unquoted literal or key: up to the next separator.
    fn word(&mut self) -> String {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| !c.is_ascii_whitespace() && !b",:;={}[]\"'#/".contains(&c))
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[start..self.i]).into_owned()
    }

    /// The four hex digits of a `\u` escape.
    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .b
            .get(self.i..self.i + 4)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit));
        let Some(digits) = digits else {
            return Err(self.error("bad \\u escape"));
        };
        self.i += 4;
        // Hex digits are ASCII: the slice is a str.
        Ok(u32::from_str_radix(std::str::from_utf8(digits).unwrap_or("0"), 16).unwrap_or(0))
    }

    fn string(&mut self, quote: u8) -> Result<String, String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("unclosed string"));
            };
            self.i += 1;
            match c {
                _ if c == quote => break,
                b'\\' => {
                    let Some(e) = self.peek() else {
                        return Err(self.error("unclosed string"));
                    };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut code = self.hex4()?;
                            // A surrogate pair is one character in two escapes. A half on
                            // its own is U+FFFD; the escape after a lone high half is read
                            // again as itself.
                            if (0xD800..0xDC00).contains(&code)
                                && self.b.get(self.i..self.i + 2) == Some(&b"\\u"[..])
                            {
                                let next = self.i;
                                self.i += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                } else {
                                    self.i = next;
                                }
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// The items up to `close`, with `item` reading each; commas between, one more allowed.
    fn list(
        &mut self,
        close: u8,
        mut item: impl FnMut(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        self.i += 1;
        loop {
            self.space()?;
            match self.peek() {
                Some(c) if c == close => {
                    self.i += 1;
                    return Ok(());
                }
                None => return Err(self.error("unclosed list")),
                _ => {}
            }
            item(self)?;
            self.space()?;
            match self.peek() {
                Some(b',' | b';') => self.i += 1,
                Some(c) if c == close => {}
                _ => return Err(self.error("expected a comma")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        let mut items = Vec::new();
        self.list(b']', |p| {
            items.push(p.value(depth + 1)?);
            Ok(())
        })?;
        Ok(Json::Arr(items))
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        let mut pairs = Vec::new();
        self.list(b'}', |p| {
            let key = match p.peek() {
                Some(q @ (b'"' | b'\'')) => p.string(q)?,
                _ => match p.word() {
                    word if word.is_empty() => return Err(p.error("expected a key")),
                    word => word,
                },
            };
            p.space()?;
            // Gson also takes `=` and `=>` for the colon.
            match p.peek() {
                Some(b':') => p.i += 1,
                Some(b'=') => {
                    p.i += 1;
                    if p.peek() == Some(b'>') {
                        p.i += 1;
                    }
                }
                _ => return Err(p.error("expected a colon")),
            }
            pairs.push((key, p.value(depth + 1)?));
            Ok(())
        })?;
        Ok(Json::Obj(pairs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_key_order_and_reads_lenient_files() {
        let text = "\u{feff}{ // a comment\n  'z': 1, /* another */ a: [true, 'two', -3e1,],\n  # hash\n  \"m\": {\"k\": null},\n}";
        let v = Json::parse(text).unwrap();
        let keys: Vec<&str> = v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["z", "a", "m"]);
        assert_eq!(
            v.get("a").unwrap(),
            &Json::Arr(vec![
                Json::Bool(true),
                Json::Str("two".into()),
                Json::Num(-30.0)
            ])
        );
        assert_eq!(v.get("m").unwrap().get("k"), Some(&Json::Null));
        assert_eq!(
            Json::parse(r#"{"s": "a\"bé\n"}"#)
                .unwrap()
                .get("s")
                .unwrap()
                .as_str(),
            Some("a\"b\u{e9}\n")
        );
    }

    #[test]
    fn gson_separators_empty_keys_and_escapes() {
        let v = Json::parse("{a=1; b=>[2;3], \"\": 'x'}").unwrap();
        assert_eq!(v.get("a"), Some(&Json::Num(1.0)));
        assert_eq!(
            v.get("b"),
            Some(&Json::Arr(vec![Json::Num(2.0), Json::Num(3.0)]))
        );
        assert_eq!(v.get(""), Some(&Json::Str("x".into())));
        let s = Json::parse(r#""\ud83d\ude00\u0041""#).unwrap();
        assert_eq!(s.as_str(), Some("\u{1f600}A"));
        for (text, want) in [
            (r#""\ud83dA""#, "\u{fffd}A"),
            (r#""\ud83d\u0041""#, "\u{fffd}A"),
            (r#""\ud83d😀""#, "\u{fffd}\u{1f600}"),
            (r#""\ude00\ud83d""#, "\u{fffd}\u{fffd}"),
        ] {
            assert_eq!(Json::parse(text).unwrap().as_str(), Some(want), "{text}");
        }
        // Comment marks inside a string are text.
        let v = Json::parse(r#"{"s": "a#b//c"}"#).unwrap();
        assert_eq!(v.get("s"), Some(&Json::Str("a#b//c".into())));
        assert!(Json::parse(r#""\u+041""#).is_err());
    }

    #[test]
    fn refuses_broken_or_hostile_files() {
        for bad in ["", "{", "{\"a\" 1}", "[1 2]", "\"open", "{} x", "/* open"] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(Json::parse(&deep).unwrap_err().contains("deep"));
        assert!(
            Json::parse("{\n\n  \"a\": }")
                .unwrap_err()
                .contains("line 3")
        );
    }
}
