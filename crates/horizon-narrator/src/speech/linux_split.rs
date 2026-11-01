//! `NarratorLinux.say`'s split of a message into the pieces flite speaks one
//! by one. Pure, so its tests run on every platform.

/// `msg.split("[,.:;/\"()\\[\\]{}!?\\\\]+")` filtered by `!x.isBlank()`: the
/// text between runs of punctuation, untrimmed, without the blank pieces
/// (which also drops the leading and trailing empty ones `split` keeps or
/// removes).
pub(super) fn units(message: &str) -> impl Iterator<Item = &str> {
    message
        .split(PUNCTUATION)
        .filter(|unit| !unit.chars().all(java_whitespace))
}

/// The pattern's character class.
const PUNCTUATION: [char; 15] = [
    ',', '.', ':', ';', '/', '"', '(', ')', '[', ']', '{', '}', '!', '?', '\\',
];

/// `Character.isWhitespace`, which `String.isBlank` asks of every code point:
/// the space, line and paragraph separators but the no-break spaces (U+00A0,
/// U+2007, U+202F), and the controls U+0009–U+000D and U+001C–U+001F. Not
/// Rust's `char::is_whitespace`, which takes U+0085 and the no-break spaces
/// but not U+001C–U+001F.
fn java_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t'..='\r'
            | '\u{1C}'..='\u{1F}'
            | ' '
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::units;

    fn split(message: &str) -> Vec<&str> {
        units(message).collect()
    }

    #[test]
    fn splits_on_runs_of_punctuation_and_keeps_the_spaces() {
        assert_eq!(
            split("Hello, world. How are you?"),
            ["Hello", " world", " How are you"]
        );
        assert_eq!(split("a,.:;/\"()[]{}!?\\b"), ["a", "b"]);
        assert_eq!(
            split("Title screen: Singleplayer button"),
            ["Title screen", " Singleplayer button"]
        );
    }

    #[test]
    fn drops_blank_pieces_and_leading_or_trailing_punctuation() {
        assert_eq!(split("...Hi... , ...there!!"), ["Hi", "there"]);
        assert_eq!(split(", ,\t, \n"), Vec::<&str>::new());
        assert_eq!(split(""), Vec::<&str>::new());
        assert_eq!(split("!!!"), Vec::<&str>::new());
    }

    #[test]
    fn keeps_what_the_pattern_does_not_name() {
        assert_eq!(split("it's 5-6 o'clock"), ["it's 5-6 o'clock"]);
        assert_eq!(split("a|b<c>d"), ["a|b<c>d"]);
    }

    #[test]
    fn blank_is_java_whitespace() {
        // No-break spaces are not whitespace to Java: the piece is spoken.
        assert_eq!(split("a,\u{A0},b"), ["a", "\u{A0}", "b"]);
        assert_eq!(split("a,\u{85},b"), ["a", "\u{85}", "b"]);
        // The information separators are.
        assert_eq!(split("a,\u{1F}\u{3000},b"), ["a", "b"]);
    }
}
