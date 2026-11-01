//! `StringTagVisitor` (compact SNBT, `Tag.toString`) and Java's number
//! formatting, below the text-component layer so that the protocol can hash
//! a tag's string form.
use crate::nbt::Tag;

/// `Double.toString`.
pub fn java_double(v: f64) -> String {
    java_float_text(v, &format!("{v}"), &format!("{v:e}"))
}
/// `Float.toString`.
pub fn java_float(v: f32) -> String {
    java_float_text(f64::from(v), &format!("{v}"), &format!("{v:e}"))
}
fn java_float_text(v: f64, plain: &str, sci: &str) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let a = v.abs();
    if (1.0e-3..1.0e7).contains(&a) {
        if plain.contains('.') {
            plain.to_owned()
        } else {
            format!("{plain}.0")
        }
    } else {
        let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_owned()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{exponent}")
    }
}

/// `SnbtGrammar.escapeControlCharacters`.
fn escape_control(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{8}' => "b",
        '\u{c}' => "f",
        '\n' => "n",
        '\r' => "r",
        '\t' => "t",
        _ => return None,
    })
}
/// `StringTag.quoteAndEscape`.
pub fn quote_and_escape(input: &str) -> String {
    let mut out = String::from(" ");
    let mut quote = '\0';
    for c in input.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else if c != '"' && c != '\'' {
            match escape_control(c) {
                Some(e) => {
                    out.push('\\');
                    out.push_str(e);
                }
                None => out.push(c),
            }
        } else {
            if quote == '\0' {
                quote = if c == '"' { '\'' } else { '"' };
            }
            if quote == c {
                out.push('\\');
            }
            out.push(c);
        }
    }
    if quote == '\0' {
        quote = '"';
    }
    out.replace_range(0..1, &quote.to_string());
    out.push(quote);
    out
}
fn unquoted_key(key: &str) -> bool {
    if key.eq_ignore_ascii_case("true") || key.eq_ignore_ascii_case("false") {
        return false;
    }
    let mut chars = key.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '.' || c == '_');
    // `[A-Za-z._]+[A-Za-z0-9._+-]*`
    first_ok && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}
pub fn string_of(t: &crate::nbt::JavaString) -> String {
    t.to_string_lossy()
}

/// `Tag.toString` (StringTagVisitor).
pub fn to_snbt(tag: &Tag) -> String {
    let mut out = String::new();
    visit(tag, &mut out);
    out
}
fn visit(tag: &Tag, out: &mut String) {
    match tag {
        Tag::End => out.push_str("END"),
        Tag::Byte(v) => out.push_str(&format!("{v}b")),
        Tag::Short(v) => out.push_str(&format!("{v}s")),
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => out.push_str(&format!("{v}L")),
        Tag::Float(v) => {
            out.push_str(&java_float(f32::from_bits(*v)));
            out.push('f');
        }
        Tag::Double(v) => {
            out.push_str(&java_double(f64::from_bits(*v)));
            out.push('d');
        }
        Tag::String(s) => out.push_str(&quote_and_escape(&string_of(s))),
        Tag::ByteArray(a) => {
            out.push_str("[B;");
            out.push_str(
                &a.iter()
                    .map(|v| format!("{}B", *v as i8))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push(']');
        }
        Tag::IntArray(a) => {
            out.push_str("[I;");
            out.push_str(&a.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
            out.push(']');
        }
        Tag::LongArray(a) => {
            out.push_str("[L;");
            out.push_str(
                &a.iter()
                    .map(|v| format!("{v}L"))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push(']');
        }
        Tag::List(l) => {
            out.push('[');
            for (i, t) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                visit(t, out);
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            out.push('{');
            let mut entries: Vec<(String, &Tag)> =
                c.iter().map(|(k, v)| (string_of(k), v)).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if unquoted_key(k) {
                    out.push_str(k);
                } else {
                    out.push_str(&quote_and_escape(k));
                }
                out.push(':');
                visit(v, out);
            }
            out.push('}');
        }
    }
}
