//! `java.util.Properties` as the dedicated server uses it: `load` (the
//! LineReader and loadConvert of the JDK), `store` with the sorted entries the
//! JDK writes since Java 18, and `java.util.Date.toString` for the timestamp
//! comment. Values are kept in a map keyed like Java's Hashtable; iteration
//! for storing sorts by `String.compareTo` (UTF-16 code units).
use std::collections::HashMap;

/// A malformed `\uxxxx` escape: Properties.load throws IllegalArgumentException.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed;
impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Malformed \\uxxxx encoding.")
    }
}
impl std::error::Error for Malformed {}

/// The key/value pairs, last occurrence winning (Hashtable.put).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub map: HashMap<String, String>,
}

impl Properties {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
    pub fn put(&mut self, key: &str, value: String) {
        self.map.insert(key.to_owned(), value);
    }
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }

    /// Properties.load(Reader) over already decoded characters.
    pub fn load(text: &[char]) -> Result<Self, Malformed> {
        let mut out = Self::default();
        let mut reader = LineReader {
            chars: text,
            pos: 0,
        };
        while let Some(line) = reader.read_line() {
            let mut key_len = 0;
            let mut value_start = line.len();
            let mut has_sep = false;
            let mut preceding_backslash = false;
            while key_len < line.len() {
                let c = line[key_len];
                if (c == '=' || c == ':') && !preceding_backslash {
                    value_start = key_len + 1;
                    has_sep = true;
                    break;
                } else if (c == ' ' || c == '\t' || c == '\u{c}') && !preceding_backslash {
                    value_start = key_len + 1;
                    break;
                }
                if c == '\\' {
                    preceding_backslash = !preceding_backslash;
                } else {
                    preceding_backslash = false;
                }
                key_len += 1;
            }
            while value_start < line.len() {
                let c = line[value_start];
                if c != ' ' && c != '\t' && c != '\u{c}' {
                    if !has_sep && (c == '=' || c == ':') {
                        has_sep = true;
                    } else {
                        break;
                    }
                }
                value_start += 1;
            }
            let key = load_convert(&line[..key_len])?;
            let value = load_convert(&line[value_start..])?;
            out.map.insert(key, value);
        }
        Ok(out)
    }

    /// Settings.loadFromFile: UTF-8 with errors reported, falling back to
    /// ISO-8859-1 for the whole file. A missing file is an empty set.
    pub fn load_file(bytes: &[u8]) -> Result<Self, Malformed> {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::load(&s.chars().collect::<Vec<_>>()),
            Err(_) => Self::load(&bytes.iter().map(|&b| char::from(b)).collect::<Vec<_>>()),
        }
    }

    /// Properties.load(InputStream): ISO-8859-1.
    pub fn load_latin1(bytes: &[u8]) -> Result<Self, Malformed> {
        Self::load(&bytes.iter().map(|&b| char::from(b)).collect::<Vec<_>>())
    }

    /// Properties.store(Writer, comments): comment, date, then the entries
    /// sorted by key, with `escUnicode` off (the Writer variant) or on (the
    /// OutputStream variant, whose bytes are ISO-8859-1).
    pub fn store(&self, comment: Option<&str>, date: &str, esc_unicode: bool) -> String {
        let nl = line_separator();
        let mut out = String::new();
        if let Some(c) = comment {
            write_comments(&mut out, c, nl);
        }
        out.push('#');
        out.push_str(date);
        out.push_str(nl);
        let mut keys: Vec<&String> = self.map.keys().collect();
        keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        for key in keys {
            out.push_str(&save_convert(key, true, esc_unicode));
            out.push('=');
            out.push_str(&save_convert(&self.map[key], false, esc_unicode));
            out.push_str(nl);
        }
        out
    }
}

/// System.lineSeparator().
pub fn line_separator() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

struct LineReader<'a> {
    chars: &'a [char],
    pos: usize,
}
impl LineReader<'_> {
    /// LineReader.readLine: one logical line, leading whitespace skipped,
    /// comments and blank lines dropped, backslash-newline continuations joined.
    fn read_line(&mut self) -> Option<Vec<char>> {
        let mut line: Vec<char> = Vec::new();
        let mut skip_white = true;
        let mut appended = false;
        let mut backslash = false;
        let chars = self.chars;
        loop {
            if self.pos >= chars.len() {
                if line.is_empty() {
                    return None;
                }
                if backslash {
                    line.pop();
                }
                return Some(line);
            }
            let c = chars[self.pos];
            self.pos += 1;
            if skip_white {
                if c == ' ' || c == '\t' || c == '\u{c}' {
                    continue;
                }
                if !appended && (c == '\r' || c == '\n') {
                    continue;
                }
                skip_white = false;
                appended = false;
            }
            if line.is_empty() && (c == '#' || c == '!') {
                loop {
                    let &d = chars.get(self.pos)?;
                    self.pos += 1;
                    if d == '\r' || d == '\n' {
                        break;
                    }
                }
                skip_white = true;
                continue;
            }
            if c != '\n' && c != '\r' {
                line.push(c);
                backslash = if c == '\\' { !backslash } else { false };
                continue;
            }
            if line.is_empty() {
                skip_white = true;
                continue;
            }
            if self.pos >= chars.len() {
                if backslash {
                    line.pop();
                }
                return Some(line);
            }
            if backslash {
                line.pop();
                skip_white = true;
                appended = true;
                backslash = false;
                if c == '\r' && chars[self.pos] == '\n' {
                    self.pos += 1;
                }
            } else {
                return Some(line);
            }
        }
    }
}

fn load_convert(input: &[char]) -> Result<String, Malformed> {
    let mut out = String::with_capacity(input.len());
    let mut units: Vec<u16> = Vec::new();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        if !units.is_empty() {
            out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{fffd}')));
        }
    };
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        i += 1;
        if c != '\\' {
            flush(&mut units, &mut out);
            out.push(c);
            continue;
        }
        let Some(&next) = input.get(i) else {
            // A trailing lone backslash is dropped.
            break;
        };
        i += 1;
        if next == 'u' {
            if i + 4 > input.len() {
                return Err(Malformed);
            }
            let mut value = 0u32;
            for &h in &input[i..i + 4] {
                value = value * 16 + h.to_digit(16).ok_or(Malformed)?;
            }
            i += 4;
            units.push(value as u16);
            continue;
        }
        flush(&mut units, &mut out);
        out.push(match next {
            't' => '\t',
            'r' => '\r',
            'n' => '\n',
            'f' => '\u{c}',
            other => other,
        });
    }
    flush(&mut units, &mut out);
    Ok(out)
}

fn save_convert(s: &str, escape_space: bool, esc_unicode: bool) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for (index, c) in s.encode_utf16().enumerate() {
        if c > 61 && c < 127 {
            if c == u16::from(b'\\') {
                out.push_str("\\\\");
            } else {
                out.push(char::from(c as u8));
            }
            continue;
        }
        match c {
            0x20 => {
                if index == 0 || escape_space {
                    out.push('\\');
                }
                out.push(' ');
            }
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x0c => out.push_str("\\f"),
            0x3d | 0x3a | 0x23 | 0x21 => {
                out.push('\\');
                out.push(char::from(c as u8));
            }
            _ => {
                if (!(0x20..=0x7e).contains(&c)) && esc_unicode {
                    out.push_str(&format!("\\u{c:04X}"));
                } else {
                    push_unit(&mut out, c);
                }
            }
        }
    }
    fix_surrogates(out)
}

/// Units are pushed one at a time; surrogate halves are joined afterwards.
fn push_unit(out: &mut String, unit: u16) {
    match char::from_u32(u32::from(unit)) {
        Some(c) => out.push(c),
        None => out.push_str(&format!("\u{0}{unit:04x}\u{0}")),
    }
}
fn fix_surrogates(s: String) -> String {
    if !s.contains('\u{0}') {
        return s;
    }
    let mut units: Vec<u16> = Vec::new();
    let mut parts = s.split('\u{0}');
    let mut odd = false;
    for part in parts.by_ref() {
        if odd {
            units.push(u16::from_str_radix(part, 16).unwrap_or(0xfffd));
        } else {
            units.extend(part.encode_utf16());
        }
        odd = !odd;
    }
    String::from_utf16_lossy(&units)
}

/// Properties.writeComments.
fn write_comments(out: &mut String, comments: &str, nl: &str) {
    out.push('#');
    let units: Vec<u16> = comments.encode_utf16().collect();
    let mut current = 0;
    let mut last = 0;
    let len = units.len();
    while current < len {
        let c = units[current];
        if c > 0x00ff || c == u16::from(b'\n') || c == u16::from(b'\r') {
            if last != current {
                out.push_str(&String::from_utf16_lossy(&units[last..current]));
            }
            if c > 0x00ff {
                out.push_str(&format!("\\u{c:04X}"));
            } else {
                out.push_str(nl);
                if c == u16::from(b'\r')
                    && current != len - 1
                    && units[current + 1] == u16::from(b'\n')
                {
                    current += 1;
                }
                if current == len - 1
                    || units[current + 1] != u16::from(b'#')
                        && units[current + 1] != u16::from(b'!')
                {
                    out.push('#');
                }
            }
            last = current + 1;
        }
        current += 1;
    }
    if last != current {
        out.push_str(&String::from_utf16_lossy(&units[last..current]));
    }
    out.push_str(nl);
}

/// Civil date from days since 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Milliseconds since the epoch, now.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// java.util.Date.toString in UTC: "EEE MMM dd HH:mm:ss zzz yyyy".
pub fn date_string(millis: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = millis.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[m as usize - 1],
        d,
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        y
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(s: &str) -> Properties {
        Properties::load(&s.chars().collect::<Vec<_>>()).unwrap()
    }
    #[test]
    fn parses_like_java() {
        let p = load(
            "# comment\n  ! other\nkey=value\na:b\nc d\n e  =  f g \nmulti=one\\\n    two\nesc=\\u00e9\\t\\x\nempty\nlast=1\nlast=2\r\ncr=1\rnext=2",
        );
        assert_eq!(p.get("key"), Some("value"));
        assert_eq!(p.get("a"), Some("b"));
        assert_eq!(p.get("c"), Some("d"));
        assert_eq!(p.get("e"), Some("f g "));
        assert_eq!(p.get("multi"), Some("onetwo"));
        assert_eq!(p.get("esc"), Some("é\tx"));
        assert_eq!(p.get("empty"), Some(""));
        assert_eq!(p.get("last"), Some("2"));
        assert_eq!(p.get("cr"), Some("1"));
        assert_eq!(p.get("next"), Some("2"));
    }
    #[test]
    fn stores_sorted_and_escaped() {
        let mut p = Properties::default();
        p.put("level-type", "minecraft:normal".into());
        p.put("motd", " A=B#".into());
        p.put("b key", "é".into());
        let s = p.store(Some("Minecraft server properties"), "DATE", false);
        // Each line ends in System.lineSeparator(), as BufferedWriter.newLine writes it.
        assert_eq!(
            s,
            "#Minecraft server properties\n#DATE\nb\\ key=é\nlevel-type=minecraft\\:normal\nmotd=\\ A\\=B\\#\n"
                .replace('\n', line_separator())
        );
        assert!(p.store(None, "D", true).contains("b\\ key=\\u00E9"));
    }
    #[test]
    fn dates() {
        assert_eq!(date_string(0), "Thu Jan 01 00:00:00 UTC 1970");
        assert_eq!(days_from_civil(2026, 10, 12), 20_738);
        assert_eq!(civil_from_days(20_738), (2026, 10, 12));
    }
}
