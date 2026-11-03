//! UUIDs as vanilla's four-int form, to and from their text form.

pub fn uuid_text(u: [i32; 4]) -> String {
    let hex: String = u.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
pub fn uuid_from_text(s: &str) -> Option<[i32; 4]> {
    let mut hex = [0u8; 32];
    let mut len = 0;
    for b in s.bytes() {
        if b == b'-' {
            continue;
        }
        if !b.is_ascii_hexdigit() || len >= 32 {
            return None;
        }
        hex[len] = b;
        len += 1;
    }
    if len != 32 {
        return None;
    }
    let hex = std::str::from_utf8(&hex).ok()?;
    let mut out = [0i32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        *v = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()? as i32;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_from_text_rejects_multibyte_and_non_hex() {
        assert_eq!(uuid_from_text("€€€€€€€€€€aa"), None);
        assert_eq!(uuid_from_text("€€€€€€€€€€aa----"), None);
        assert_eq!(uuid_from_text("+0000001-0000-0000-0000-000000000000"), None);
        assert_eq!(uuid_from_text("00000000-0000-0000-0000-00000000000g"), None);
        assert_eq!(uuid_from_text(""), None);
    }

    #[test]
    fn uuid_from_text_parses_hex_and_dashes() {
        let u = [0x01234567, -0x76543211, 0x01234567, -0x76543211];
        let text = uuid_text(u);
        assert_eq!(uuid_from_text(&text), Some(u));
        let undashed: String = text.chars().filter(|c| *c != '-').collect();
        assert_eq!(uuid_from_text(&undashed), Some(u));
    }
}
