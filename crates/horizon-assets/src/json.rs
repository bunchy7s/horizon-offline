use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt};

#[derive(Debug)]
pub struct DecodeError(pub(crate) String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecodeError {}
impl From<serde_json::Error> for DecodeError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}
pub(crate) type Result<T> = std::result::Result<T, DecodeError>;
pub(crate) type Object<'a> = BTreeMap<String, &'a RawValue>;
pub(crate) fn error(message: &str) -> DecodeError {
    DecodeError(message.to_owned())
}
/// Whether a syntactically valid JSON number is classified as numeric by the
/// native reader. Its wrapping i64 scanner can hit zero after overflow, causing
/// the next integer digit to be rejected as if it followed a leading zero.
pub(crate) fn native_number_token(token: &str) -> bool {
    if token.len() >= 1024 {
        return false;
    }
    let mut accumulator = 0i64;
    for (index, digit) in token
        .trim_start_matches('-')
        .bytes()
        .take_while(u8::is_ascii_digit)
        .enumerate()
    {
        if index > 0 && accumulator == 0 {
            return false;
        }
        accumulator = accumulator
            .wrapping_mul(10)
            .wrapping_sub(i64::from(digit - b'0'));
    }
    true
}
/// Strict stream parsing rejects numeric scanner failures everywhere, including
/// ignored fields. Quoted copies of those tokens remain ordinary strings.
pub(crate) fn validate_number_tokens(raw: &RawValue) -> Result<()> {
    let text = raw.get();
    let mut quoted = false;
    let mut escaped = false;
    let mut start = None;
    for (index, byte) in text.bytes().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        if let Some(begin) = start {
            if byte.is_ascii_digit() || b"-.eE+".contains(&byte) {
                continue;
            }
            if !native_number_token(&text[begin..index]) {
                return Err(error("native reader rejects numeric token"));
            }
            start = None;
        }
        if byte == b'"' {
            quoted = true;
        } else if byte.is_ascii_digit() || byte == b'-' {
            start = Some(index);
        }
    }
    if start.is_some_and(|begin| !native_number_token(&text[begin..])) {
        return Err(error("native reader rejects numeric token"));
    }
    Ok(())
}
pub(crate) fn object(raw: &RawValue) -> Result<Object<'_>> {
    if !raw.get().starts_with('{') {
        return Err(error("expected object"));
    }
    Ok(serde_json::from_str(raw.get())?)
}
/// The members of an object in document order, which `object` loses.
pub(crate) fn ordered_object(raw: &RawValue) -> Result<Vec<(String, &RawValue)>> {
    struct Members<'a>(Vec<(String, &'a RawValue)>);
    impl<'de> serde::Deserialize<'de> for Members<'de> {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Members<'de>;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("an object")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Self::Value, A::Error> {
                    let mut members: Vec<(String, &'de RawValue)> = Vec::new();
                    while let Some(entry) = map.next_entry::<String, &'de RawValue>()? {
                        members.retain(|(name, _)| *name != entry.0);
                        members.push(entry);
                    }
                    Ok(Members(members))
                }
            }
            deserializer.deserialize_map(Visitor)
        }
    }
    if !raw.get().starts_with('{') {
        return Err(error("expected object"));
    }
    Ok(serde_json::from_str::<Members>(raw.get())?.0)
}
pub(crate) fn array(raw: &RawValue) -> Result<Vec<&RawValue>> {
    Ok(serde_json::from_str(raw.get())?)
}
pub(crate) fn required<'a>(object: &Object<'a>, key: &str) -> Result<&'a RawValue> {
    object
        .get(key)
        .copied()
        .ok_or_else(|| error(&format!("missing {key}")))
}
pub(crate) fn string(raw: &RawValue) -> Result<String> {
    Ok(serde_json::from_str(raw.get())?)
}
/// Gson's string helper accepts any primitive and preserves a number's spelling.
pub(crate) fn primitive_string(raw: &RawValue) -> Result<String> {
    match raw.get().as_bytes().first() {
        Some(b'"') => string(raw),
        Some(b't' | b'f' | b'-' | b'0'..=b'9') => Ok(raw.get().to_owned()),
        _ => Err(error("expected primitive")),
    }
}
pub(crate) fn boolean(raw: &RawValue) -> Result<bool> {
    Ok(primitive_string(raw)?.eq_ignore_ascii_case("true"))
}
fn number(raw: &RawValue) -> Result<&str> {
    match raw.get().as_bytes().first() {
        Some(b'-' | b'0'..=b'9') => Ok(raw.get()),
        _ => Err(error("expected number")),
    }
}
pub(crate) fn float(raw: &RawValue) -> Result<f32> {
    number(raw)?.parse().map_err(|_| error("invalid float"))
}
/// Gson LazilyParsedNumber.intValue truncates decimals then keeps the low 32 bits.
/// Work on decimal digits, avoiding f64 rounding and saturating Rust casts.
pub(crate) fn integer(raw: &RawValue) -> Result<i32> {
    let raw = number(raw)?;
    if raw.len() > 10_000 {
        return Err(error("number string too long"));
    }
    let negative = raw.starts_with('-');
    let raw = raw.strip_prefix('-').unwrap_or(raw);
    let (mantissa, exponent) = raw.split_once(['e', 'E']).unwrap_or((raw, "0"));
    let exponent: i32 = exponent.parse().map_err(|_| error("invalid exponent"))?;
    let fractional = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let scale =
        i64::try_from(fractional).map_err(|_| error("number too long"))? - i64::from(exponent);
    if scale.abs() >= 10_000 {
        return Err(error("unsupported decimal scale"));
    }
    let digits = mantissa.bytes().filter(|c| *c != b'.');
    let count = mantissa.len() - usize::from(mantissa.contains('.'));
    let take = (count as i64 - scale.max(0)).max(0) as usize;
    let mut value = 0u32;
    for digit in digits.take(take) {
        value = value.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'));
    }
    if scale < 0 {
        // 10^32 and larger are divisible by 2^32.
        value = if scale <= -32 {
            0
        } else {
            value.wrapping_mul(10u32.wrapping_pow((-scale) as u32))
        };
    }
    Ok(if negative {
        value.wrapping_neg()
    } else {
        value
    } as i32)
}
pub(crate) fn vector<const N: usize>(raw: &RawValue) -> Result<[f32; N]> {
    let values = array(raw)?;
    if values.len() != N {
        return Err(error("wrong vector length"));
    }
    let mut result = [0.0; N];
    for (target, value) in result.iter_mut().zip(values) {
        *target = float(value)?;
    }
    Ok(result)
}
pub(crate) fn identifier(name: &str) -> Result<String> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid) || !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(error("invalid resource identifier"));
    }
    Ok(format!("{namespace}:{path}"))
}
