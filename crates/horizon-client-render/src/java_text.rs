//! Java text conventions the vanilla option files depend on: shortest-digit
//! `Double.toString`/`Float.toString`, Gson's HTML-safe string escaping and a
//! reader for Gson's lenient JSON (`JsonParser.parseString`).

/// Digits and decimal exponent of the shortest round-trip form.
fn shortest(text: &str) -> (String, i32) {
    // `{:e}` yields "d.ddde-5" or "de5".
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exponent.parse().unwrap_or(0))
}

fn render(negative: bool, digits: &str, exponent: i32) -> String {
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    let digits = if digits.is_empty() { "0" } else { digits };
    if (-3..7).contains(&exponent) {
        if exponent >= 0 {
            let int_len = exponent as usize + 1;
            if digits.len() <= int_len {
                out.push_str(digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exponent - 1) as usize));
            out.push_str(digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exponent.to_string());
    }
    out
}

/// `Double.toString`.
pub fn double_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let (digits, exponent) = shortest(&format!("{:e}", value.abs()));
    render(value < 0.0, &digits, exponent)
}

/// `Float.toString`.
pub fn float_to_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let (digits, exponent) = shortest(&format!("{:e}", value.abs()));
    render(value < 0.0, &digits, exponent)
}

/// Gson's default (HTML-safe) JSON string literal.
pub fn gson_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A parsed JSON element as Gson models it.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number's source text (`LazilyParsedNumber`).
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object,
}

impl Json {
    /// `Number.intValue()` of a `LazilyParsedNumber`: `Integer.parseInt`, else
    /// `BigDecimal.intValue()` (truncate, keep the low 32 bits).
    pub fn int_value(text: &str) -> i32 {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (mantissa, exponent) = match body.split_once(['e', 'E']) {
            Some((m, e)) => (
                m,
                e.strip_prefix('+').unwrap_or(e).parse::<i64>().unwrap_or(0),
            ),
            None => (body, 0),
        };
        let (int_part, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits: Vec<u32> = int_part
            .chars()
            .chain(fraction.chars())
            .filter_map(|c| c.to_digit(10))
            .collect();
        // value = digits * 10^scale; truncating drops digits below the point.
        let scale = exponent - fraction.len() as i64;
        let keep = if scale >= 0 {
            digits.len()
        } else {
            digits
                .len()
                .saturating_sub(scale.unsigned_abs().min(usize::MAX as u64) as usize)
        };
        let mut v: u32 = 0;
        for d in &digits[..keep] {
            v = v.wrapping_mul(10).wrapping_add(*d);
        }
        if scale > 0 {
            // v * 10^scale modulo 2^32 by repeated squaring.
            let (mut base, mut power, mut e) = (10u32, 1u32, scale as u64);
            while e > 0 {
                if e & 1 == 1 {
                    power = power.wrapping_mul(base);
                }
                base = base.wrapping_mul(base);
                e >>= 1;
            }
            v = v.wrapping_mul(power);
        }
        let v = v as i32;
        if negative { v.wrapping_neg() } else { v }
    }
    /// `Number.doubleValue()`.
    pub fn double_value(text: &str) -> f64 {
        text.parse::<f64>().unwrap_or(0.0)
    }
}

/// A syntax error aborts the whole options load, as in vanilla.
#[derive(Debug)]
pub struct SyntaxError;

struct Reader {
    chars: Vec<char>,
    pos: usize,
}

fn is_literal(c: char) -> bool {
    !matches!(
        c,
        '/' | '\\'
            | ';'
            | '#'
            | '='
            | '{'
            | '}'
            | '['
            | ']'
            | ':'
            | ','
            | ' '
            | '\t'
            | '\u{c}'
            | '\r'
            | '\n'
    )
}

impl Reader {
    fn peek_char(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    /// `nextNonWhitespace` with lenient comments; `None` at the end.
    fn skip(&mut self) -> Result<Option<char>, SyntaxError> {
        loop {
            match self.peek_char() {
                None => return Ok(None),
                Some(' ' | '\t' | '\r' | '\n') => self.pos += 1,
                Some('#') => self.line_comment(),
                Some('/') => match self.chars.get(self.pos + 1) {
                    Some('/') => self.line_comment(),
                    Some('*') => {
                        self.pos += 2;
                        loop {
                            match (self.peek_char(), self.chars.get(self.pos + 1)) {
                                (Some('*'), Some('/')) => {
                                    self.pos += 2;
                                    break;
                                }
                                (Some(_), _) => self.pos += 1,
                                (None, _) => return Err(SyntaxError),
                            }
                        }
                    }
                    _ => return Ok(Some('/')),
                },
                Some(c) => return Ok(Some(c)),
            }
        }
    }
    fn line_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            self.pos += 1;
            if c == '\n' || c == '\r' {
                break;
            }
        }
    }
    fn quoted(&mut self, quote: char) -> Result<String, SyntaxError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let c = self.peek_char().ok_or(SyntaxError)?;
            self.pos += 1;
            if c == quote {
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let e = self.peek_char().ok_or(SyntaxError)?;
            self.pos += 1;
            match e {
                'u' => {
                    let mut v = 0u32;
                    for _ in 0..4 {
                        let d = self.peek_char().ok_or(SyntaxError)?;
                        self.pos += 1;
                        v = v * 16 + d.to_digit(16).ok_or(SyntaxError)?;
                    }
                    // Lone surrogates cannot be represented; keep the replacement.
                    out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                }
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                'f' => out.push('\u{c}'),
                '\n' | '\'' | '"' | '\\' | '/' => out.push(e),
                _ => return Err(SyntaxError),
            }
        }
    }
    fn literal(&mut self) -> String {
        let start = self.pos;
        while self.peek_char().is_some_and(is_literal) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }
    /// JSON number grammar with Gson's leading-zero rule.
    fn is_number(text: &str) -> bool {
        let b: Vec<char> = text.chars().collect();
        let mut i = 0;
        if b.first() == Some(&'-') {
            i += 1;
        }
        match b.get(i) {
            Some('0') => {
                i += 1;
            }
            Some(c) if c.is_ascii_digit() => {
                while b.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            _ => return false,
        }
        if b.get(i) == Some(&'.') {
            i += 1;
            if !b.get(i).is_some_and(char::is_ascii_digit) {
                return false;
            }
            while b.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
        }
        if matches!(b.get(i), Some('e' | 'E')) {
            i += 1;
            if matches!(b.get(i), Some('+' | '-')) {
                i += 1;
            }
            if !b.get(i).is_some_and(char::is_ascii_digit) {
                return false;
            }
            while b.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
        }
        i == b.len()
    }
    fn keyword(text: &str, word: &str) -> bool {
        text.len() == word.len() && text.eq_ignore_ascii_case(word)
    }
    fn value(&mut self) -> Result<Json, SyntaxError> {
        let c = self.skip()?.ok_or(SyntaxError)?;
        match c {
            '"' | '\'' => Ok(Json::String(self.quoted(c)?)),
            '[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    match self.skip()?.ok_or(SyntaxError)? {
                        ']' => {
                            self.pos += 1;
                            // A trailing separator leaves an implicit null.
                            return Ok(Json::Array(items));
                        }
                        ',' | ';' => {
                            self.pos += 1;
                            items.push(Json::Null);
                            continue;
                        }
                        _ => {}
                    }
                    items.push(self.value()?);
                    match self.skip()?.ok_or(SyntaxError)? {
                        ']' => {
                            self.pos += 1;
                            return Ok(Json::Array(items));
                        }
                        ',' | ';' => self.pos += 1,
                        _ => return Err(SyntaxError),
                    }
                }
            }
            '{' => {
                self.pos += 1;
                loop {
                    match self.skip()?.ok_or(SyntaxError)? {
                        '}' => {
                            self.pos += 1;
                            return Ok(Json::Object);
                        }
                        '"' | '\'' => {
                            let q = self.skip()?.ok_or(SyntaxError)?;
                            self.quoted(q)?;
                        }
                        c if is_literal(c) => {
                            self.literal();
                        }
                        _ => return Err(SyntaxError),
                    }
                    match self.skip()?.ok_or(SyntaxError)? {
                        ':' => self.pos += 1,
                        '=' => {
                            self.pos += 1;
                            if self.peek_char() == Some('>') {
                                self.pos += 1;
                            }
                        }
                        _ => return Err(SyntaxError),
                    }
                    self.value()?;
                    match self.skip()?.ok_or(SyntaxError)? {
                        '}' => {
                            self.pos += 1;
                            return Ok(Json::Object);
                        }
                        ',' | ';' => self.pos += 1,
                        _ => return Err(SyntaxError),
                    }
                }
            }
            ']' | '}' | ',' | ';' | ':' | '=' | '\\' | '/' => Err(SyntaxError),
            _ => {
                let text = self.literal();
                if text.is_empty() {
                    return Err(SyntaxError);
                }
                if Self::keyword(&text, "true") {
                    Ok(Json::Bool(true))
                } else if Self::keyword(&text, "false") {
                    Ok(Json::Bool(false))
                } else if Self::keyword(&text, "null") {
                    Ok(Json::Null)
                } else if Self::is_number(&text) {
                    Ok(Json::Number(text))
                } else {
                    Ok(Json::String(text))
                }
            }
        }
    }
}

/// `JsonParser.parseString`: one lenient value and nothing after it.
pub fn parse_lenient(text: &str) -> Result<Json, SyntaxError> {
    let text = text.strip_prefix(")]}'\n").unwrap_or(text);
    let mut reader = Reader {
        chars: text.chars().collect(),
        pos: 0,
    };
    if reader.skip()?.is_none() {
        return Ok(Json::Null);
    }
    let value = reader.value()?;
    // The trailing check runs after Gson restored its legacy-strict mode:
    // only whitespace may follow the value.
    if value != Json::Null
        && reader.chars[reader.pos..]
            .iter()
            .any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
    {
        return Err(SyntaxError);
    }
    Ok(value)
}

impl Reader {
    /// Next non-whitespace char in strict mode (comments are errors).
    fn strict_skip(&mut self) -> Option<char> {
        while matches!(self.peek_char(), Some(' ' | '\t' | '\r' | '\n')) {
            self.pos += 1;
        }
        self.peek_char()
    }
}

/// `Gson.getAdapter(List<String>).read` on a strict `JsonReader`
/// (`GsonHelper.fromNullableJson`): `Ok(None)` for `null`, an error for
/// anything Gson rejects, including an empty document.
pub fn parse_strict_string_list(text: &str) -> Result<Option<Vec<String>>, SyntaxError> {
    let mut r = Reader {
        chars: text.chars().collect(),
        pos: 0,
    };
    match r.strict_skip().ok_or(SyntaxError)? {
        'n' => {
            let word = r.literal();
            if word == "null" {
                Ok(None)
            } else {
                Err(SyntaxError)
            }
        }
        '[' => {
            r.pos += 1;
            let mut items = Vec::new();
            if r.strict_skip().ok_or(SyntaxError)? == ']' {
                return Ok(Some(items));
            }
            loop {
                let c = r.strict_skip().ok_or(SyntaxError)?;
                match c {
                    '"' => items.push(r.quoted('"')?),
                    '-' | '0'..='9' => {
                        let word = r.literal();
                        if !Reader::is_number(&word) {
                            return Err(SyntaxError);
                        }
                        items.push(word);
                    }
                    't' | 'f' | 'n' => {
                        let word = r.literal();
                        match word.as_str() {
                            "true" | "false" => items.push(word),
                            "null" => {}
                            _ => return Err(SyntaxError),
                        }
                    }
                    _ => return Err(SyntaxError),
                }
                match r.strict_skip().ok_or(SyntaxError)? {
                    ',' => r.pos += 1,
                    ']' => return Ok(Some(items)),
                    _ => return Err(SyntaxError),
                }
            }
        }
        _ => Err(SyntaxError),
    }
}

/// `String.format(Locale.ROOT, "%.<n>f", value)`: Java rounds the shortest
/// round-trip decimal expansion half-up (so 1.005 gives 1.01).
pub fn format_fixed(value: f64, decimals: usize) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    let negative = value.is_sign_negative();
    let (digits, exponent) = if value == 0.0 {
        ("0".to_owned(), 0)
    } else {
        shortest(&format!("{:e}", value.abs()))
    };
    let mut int_part: Vec<u8> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    let d: Vec<u8> = digits.bytes().map(|b| b - b'0').collect();
    let point = exponent + 1; // digits before the decimal point
    if point <= 0 {
        int_part.push(0);
        frac.extend(std::iter::repeat_n(0, (-point) as usize));
        frac.extend(&d);
    } else {
        let point = point as usize;
        for i in 0..point {
            int_part.push(d.get(i).copied().unwrap_or(0));
        }
        if d.len() > point {
            frac.extend(&d[point..]);
        }
    }
    let round_up = frac.get(decimals).is_some_and(|&x| x >= 5);
    frac.truncate(decimals);
    frac.resize(decimals, 0);
    if round_up {
        let mut carry = true;
        for x in frac.iter_mut().rev().chain(int_part.iter_mut().rev()) {
            if !carry {
                break;
            }
            if *x == 9 {
                *x = 0;
            } else {
                *x += 1;
                carry = false;
            }
        }
        if carry {
            int_part.insert(0, 1);
        }
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.extend(int_part.iter().map(|&x| (b'0' + x) as char));
    if decimals > 0 {
        out.push('.');
        out.extend(frac.iter().map(|&x| (b'0' + x) as char));
    }
    out
}

/// `%.2f`.
pub fn format_fixed2(value: f64) -> String {
    format_fixed(value, 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_number_text() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.5), "0.5");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(1234567.0), "1234567.0");
        assert_eq!(double_to_string(12345678.0), "1.2345678E7");
        assert_eq!(double_to_string(100.0), "100.0");
        assert_eq!(float_to_string(0.15), "0.15");
        assert_eq!(float_to_string(1.0e10), "1.0E10");
        assert_eq!(double_to_string(-2.5), "-2.5");
    }
    #[test]
    fn lenient_json() {
        assert_eq!(parse_lenient("0.5").unwrap(), Json::Number("0.5".into()));
        assert_eq!(parse_lenient("fast").unwrap(), Json::String("fast".into()));
        assert_eq!(parse_lenient("TRUE").unwrap(), Json::Bool(true));
        assert_eq!(parse_lenient("007").unwrap(), Json::String("007".into()));
        assert!(parse_lenient("1 2").is_err());
        assert!(parse_lenient("\"abc").is_err());
        assert_eq!(parse_lenient("  ").unwrap(), Json::Null);
        assert_eq!(gson_string("a=b"), "\"a\\u003db\"");
    }
}
