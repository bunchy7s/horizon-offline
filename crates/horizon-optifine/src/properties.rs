//! Java `.properties` files (`java.util.Properties.load`), as OptiFine reads them: `key=value`,
//! `key:value` or `key value` lines, `#` and `!` comments, a trailing `\` continuing the line,
//! and the escapes `\t`, `\n`, `\r`, `\f`, `\uXXXX` (any other `\x` is `x`). Values are trimmed,
//! as OptiFine does. A later duplicate key replaces the earlier one.
//!
//! Shader packs have their own reader (`horizon_shaders::properties`): theirs runs `#define`
//! and `#if` directives and reads no escapes.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties {
    /// In file order; each key once.
    pub entries: Vec<(String, String)>,
}

impl Properties {
    pub fn parse(text: &str) -> Self {
        Self::read(text, split)
    }

    /// As CIT Resewn reads its files: a line with an unescaped `=` splits there, so a key may
    /// hold an unescaped `:` (`components.minecraft:custom_name=Bob`); other lines as `parse`.
    pub fn parse_equals_first(text: &str) -> Self {
        Self::read(text, |line| match unescaped_equals(line) {
            Some(i) => (
                line[..i].trim_end_matches([' ', '\t', '\x0c']),
                &line[i + 1..],
            ),
            None => split(line),
        })
    }

    fn read(text: &str, split: impl Fn(&str) -> (&str, &str)) -> Self {
        let mut out = Self::default();
        // Each key's place in `entries`, so a file of many keys reads in linear time.
        let mut index = std::collections::HashMap::<String, usize>::new();
        let mut lines = text.lines();
        while let Some(first) = lines.next() {
            let first = first.trim_start();
            if first.is_empty() || first.starts_with('#') || first.starts_with('!') {
                continue;
            }
            let mut line = String::from(first);
            while ends_in_continuation(&line) {
                line.pop();
                match lines.next() {
                    Some(next) => line.push_str(next.trim_start()),
                    None => break,
                }
            }
            let (key, value) = split(&line);
            let (key, value) = (unescape(key), unescape(value).trim().to_owned());
            match index.get(&key) {
                Some(&i) => out.entries[i].1 = value,
                None => {
                    index.insert(key.clone(), out.entries.len());
                    out.entries.push((key, value));
                }
            }
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The entries whose key starts with `prefix` (`nbt.`, `texture.`, `skins.`), the prefix cut.
    pub fn with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.entries
            .iter()
            .filter_map(move |(k, v)| Some((k.strip_prefix(prefix)?, v.as_str())))
    }
}

/// An odd number of trailing backslashes.
fn ends_in_continuation(line: &str) -> bool {
    line.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

fn unescaped_equals(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'=' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// The key ends at the first unescaped `=`, `:` or whitespace; then whitespace and at most one
/// `=` or `:` are skipped.
fn split(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'=' | b':' | b' ' | b'\t' | b'\x0c' => break,
            _ => i += 1,
        }
    }
    let key_end = i.min(bytes.len());
    let mut rest = line[key_end..].trim_start_matches([' ', '\t', '\x0c']);
    if let Some(r) = rest.strip_prefix(['=', ':']) {
        rest = r.trim_start_matches([' ', '\t', '\x0c']);
    }
    (&line[..key_end], rest)
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\x0c'),
            Some('u') => {
                let mut unit = || {
                    let hex: String = chars.by_ref().take(4).collect();
                    u32::from_str_radix(&hex, 16).ok()
                };
                let Some(mut code) = unit() else { continue };
                // A character past U+FFFF is two escapes, as Java writes it: `\uD83D\uDE00`.
                if (0xD800..0xDC00).contains(&code) {
                    let rest = chars.as_str();
                    if let Some(low) = rest
                        .strip_prefix("\\u")
                        .and_then(|r| u32::from_str_radix(r.get(..4)?, 16).ok())
                        .filter(|low| (0xDC00..0xE000).contains(low))
                    {
                        chars.nth(5);
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                    }
                }
                if let Some(c) = char::from_u32(code) {
                    out.push(c);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_comments_and_continuations() {
        let p = Properties::parse(
            "# a comment\n! another\nmethod=ctm\nmatchBlocks : minecraft:glass \\\n    minecraft:white_stained_glass\ntiles 0-46\n\nweights=1\nweights=2\n",
        );
        assert_eq!(p.get("method"), Some("ctm"));
        assert_eq!(
            p.get("matchBlocks"),
            Some("minecraft:glass minecraft:white_stained_glass")
        );
        assert_eq!(p.get("tiles"), Some("0-46"));
        assert_eq!(p.get("weights"), Some("2"));
        assert_eq!(p.entries.len(), 4);
    }

    #[test]
    fn escapes_in_keys_and_values() {
        let p = Properties::parse(
            "nbt.display.Name=ipattern:*\\u00e9p\\u00e9e*\nkey\\=with\\:colon=a\\tb\nlast\\\\=x\n",
        );
        assert_eq!(p.get("nbt.display.Name"), Some("ipattern:*épée*"));
        assert_eq!(p.get("key=with:colon"), Some("a\tb"));
        // An even number of backslashes is an escaped backslash, not a continuation.
        assert_eq!(p.get("last\\"), Some("x"));
        let emoji = Properties::parse("nbt.display.Name=\\uD83D\\uDE00 Sword\n");
        assert_eq!(emoji.get("nbt.display.Name"), Some("\u{1F600} Sword"));
    }

    #[test]
    fn equals_first_keeps_colons_in_keys() {
        let text = "components.minecraft:custom_name = Bob\nnbt.a\\=b=c\ntype:item\n";
        let p = Properties::parse_equals_first(text);
        assert_eq!(p.get("components.minecraft:custom_name"), Some("Bob"));
        assert_eq!(p.get("nbt.a=b"), Some("c"), "an escaped = is the key's");
        assert_eq!(p.get("type"), Some("item"), "no = at all: Java's rules");
        assert_eq!(
            Properties::parse(text).get("components.minecraft"),
            Some("custom_name = Bob")
        );
    }

    #[test]
    fn prefixed_keys() {
        let p = Properties::parse("texture.bow_pulling_0=a\ntexture.bow_pulling_1=b\nitems=bow\n");
        let got: Vec<_> = p.with_prefix("texture.").collect();
        assert_eq!(got, [("bow_pulling_0", "a"), ("bow_pulling_1", "b")]);
    }
}
