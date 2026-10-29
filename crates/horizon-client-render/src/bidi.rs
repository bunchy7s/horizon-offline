//! Right-to-left text (`FormattedBidiReorder.reorder`, behind
//! `Language.getVisualOrder`): the logical text a player types becomes the
//! glyphs a font draws left to right. Arabic letters are shaped into their
//! joined presentation forms (ICU's `ArabicShaping` with `LETTERS_SHAPE`), the
//! text is reordered by the Unicode bidirectional algorithm, and characters in
//! right-to-left runs are mirrored. Left-to-right text comes back unchanged.
use crate::text::{Span, Text};
use std::borrow::Cow;
use unicode_bidi::{
    BidiClass, Direction, Level, ParagraphBidiInfo, bidi_class, get_base_direction,
};

/// How a letter joins its neighbours, as ICU's shaper sees it: `Right` joins
/// the letter before it only, `Dual` both neighbours, `Mark` is skipped over
/// (diacritics) and `None` never joins.
#[derive(Clone, Copy, PartialEq)]
enum Join {
    None,
    Right,
    Dual,
    Mark,
}
use Join::{Dual, Mark, Right};

/// ICU's joining table (`araLink`): code point ranges and their class, with the
/// lam-alef ligatures at the end. Anything else is `Join::None`.
const JOINS: &[(u32, u32, Join)] = &[
    (0x0622, 0x0625, Right),
    (0x0626, 0x0626, Dual),
    (0x0627, 0x0627, Right),
    (0x0628, 0x0628, Dual),
    (0x0629, 0x0629, Right),
    (0x062A, 0x062E, Dual),
    (0x062F, 0x0632, Right),
    (0x0633, 0x063A, Dual),
    (0x0640, 0x0647, Dual),
    (0x0648, 0x0649, Right),
    (0x064A, 0x064A, Dual),
    (0x064B, 0x0656, Mark),
    (0x0670, 0x0670, Mark),
    (0x0671, 0x0673, Right),
    (0x0675, 0x0677, Right),
    (0x0678, 0x0687, Dual),
    (0x0688, 0x0699, Right),
    (0x069A, 0x06B9, Dual),
    (0x06BA, 0x06BA, Right),
    (0x06BB, 0x06BF, Dual),
    (0x06C0, 0x06C0, Right),
    (0x06C1, 0x06C1, Dual),
    (0x06C2, 0x06CB, Right),
    (0x06CC, 0x06CC, Dual),
    (0x06CD, 0x06CD, Right),
    (0x06CE, 0x06D1, Dual),
    (0x06D2, 0x06D3, Right),
    (0x200D, 0x200D, Dual),
    (0x206D, 0x206F, Mark),
    (0xFEF5, 0xFEFC, Right),
];

/// Letters that have joined forms and the code point of the isolated one; the
/// final, initial and medial forms follow it (a right-joining letter has the
/// isolated and final only). The lam-alef ligatures are their own letters.
/// ICU leaves the extended letters (پ چ ژ ک گ ی and the rest of U+0679 to
/// U+06D3) unchanged, so they are in `JOINS` but not here.
#[rustfmt::skip]
const FORMS: &[(u32, u32)] = &[
    (0x0622, 0xFE81), (0x0623, 0xFE83), (0x0624, 0xFE85), (0x0625, 0xFE87), (0x0626, 0xFE89),
    (0x0627, 0xFE8D), (0x0628, 0xFE8F), (0x0629, 0xFE93), (0x062A, 0xFE95), (0x062B, 0xFE99),
    (0x062C, 0xFE9D), (0x062D, 0xFEA1), (0x062E, 0xFEA5), (0x062F, 0xFEA9), (0x0630, 0xFEAB),
    (0x0631, 0xFEAD), (0x0632, 0xFEAF), (0x0633, 0xFEB1), (0x0634, 0xFEB5), (0x0635, 0xFEB9),
    (0x0636, 0xFEBD), (0x0637, 0xFEC1), (0x0638, 0xFEC5), (0x0639, 0xFEC9), (0x063A, 0xFECD),
    (0x0641, 0xFED1), (0x0642, 0xFED5), (0x0643, 0xFED9), (0x0644, 0xFEDD), (0x0645, 0xFEE1),
    (0x0646, 0xFEE5), (0x0647, 0xFEE9), (0x0648, 0xFEED), (0x0649, 0xFEEF), (0x064A, 0xFEF1),
    (0xFEF5, 0xFEF5), (0xFEF7, 0xFEF7), (0xFEF9, 0xFEF9), (0xFEFB, 0xFEFB),
];

/// `Bidi_Mirroring_Glyph` (`UCharacter.getMirror`): each pair is two
/// neighbouring characters that mirror each other.
const MIRRORED: &str = "\
        ()<>[]{}«»༺༻༼༽᚛᚜‹›⁅⁆⁽⁾₍₎∈∋∉∌∊∍∕⧵∟⯾∠⦣∡⦛∢⦠∤⫮∼∽≃⋍≅≌≒≓\
        ≔≕≤≥≦≧≨≩≪≫≮≯≰≱≲≳≴≵≶≷≸≹≺≻≼≽≾≿⊀⊁⊂⊃⊄⊅⊆⊇⊈⊉⊊⊋⊏⊐⊑⊒⊘⦸⊢⊣⊦⫞\
        ⊨⫤⊩⫣⊫⫥⊰⊱⊲⊳⊴⊵⊶⊷⊸⟜⋉⋊⋋⋌⋐⋑⋖⋗⋘⋙⋚⋛⋜⋝⋞⋟⋠⋡⋢⋣⋤⋥⋦⋧⋨⋩⋪⋫⋬⋭⋰⋱⋲⋺\
        ⋳⋻⋴⋼⋶⋽⋷⋾⌈⌉⌊⌋〈〉❨❩❪❫❬❭❮❯❰❱❲❳❴❵⟃⟄⟅⟆⟈⟉⟋⟍⟕⟖⟝⟞⟢⟣⟤⟥⟦⟧⟨⟩⟪⟫\
        ⟬⟭⟮⟯⦃⦄⦅⦆⦇⦈⦉⦊⦋⦌⦍⦐⦎⦏⦑⦒⦓⦔⦕⦖⦗⦘⦤⦥⦨⦩⦪⦫⦬⦭⦮⦯⧀⧁⧄⧅⧏⧐⧑⧒⧔⧕⧘⧙⧚⧛\
        ⧨⧩⧸⧹⧼⧽⨫⨬⨭⨮⨴⨵⨼⨽⩤⩥⩹⩺⩻⩼⩽⩾⩿⪀⪁⪂⪃⪄⪅⪆⪇⪈⪉⪊⪋⪌⪍⪎⪏⪐⪑⪒⪓⪔⪕⪖⪗⪘⪙⪚\
        ⪛⪜⪝⪞⪟⪠⪡⪢⪦⪧⪨⪩⪪⪫⪬⪭⪯⪰⪱⪲⪳⪴⪵⪶⪷⪸⪹⪺⪻⪼⪽⪾⪿⫀⫁⫂⫃⫄⫅⫆⫇⫈⫉⫊⫋⫌⫍⫎⫏⫐\
        ⫑⫒⫓⫔⫕⫖⫬⫭⫷⫸⫹⫺⸂⸃⸄⸅⸉⸊⸌⸍⸜⸝⸠⸡⸢⸣⸤⸥⸦⸧⸨⸩⹕⹖⹗⹘⹙⹚⹛⹜〈〉《》「」『』【】\
        〔〕〖〗〘〙〚〛﹙﹚﹛﹜﹝﹞﹤﹥（）＜＞［］｛｝｟｠｢｣";

fn join(c: char) -> Join {
    let c = c as u32;
    JOINS
        .iter()
        .find(|&&(from, to, _)| (from..=to).contains(&c))
        .map_or(Join::None, |&(_, _, join)| join)
}

/// The letter whose joined form `c` is (ICU shapes the forms again), or `c`.
fn letter_of(c: char) -> char {
    let c32 = c as u32;
    if !matches!(c32, 0xFE70..=0xFEFC) {
        return c;
    }
    if (0xFE70..0xFE80).contains(&c32) {
        // The diacritics' forms pair up from fathatan on.
        return char::from_u32(0x064B + (c32 - 0xFE70) / 2).unwrap_or(c);
    }
    FORMS
        .iter()
        .find(|&&(letter, iso)| {
            let forms = if join(char::from_u32(letter).unwrap_or(c)) == Dual {
                4
            } else {
                2
            };
            (iso..iso + forms).contains(&c32)
        })
        .and_then(|&(letter, _)| char::from_u32(letter))
        .unwrap_or(c)
}

/// `ArabicShaping(LETTERS_SHAPE).shape`: lam followed by an alef (diacritics
/// may sit between) becomes one ligature, then every letter takes the isolated,
/// final, initial or medial form its neighbours call for. Diacritics take the
/// tatweel form between two joined letters. The text can only get shorter.
fn shape(text: &[char]) -> Vec<char> {
    let letters: Vec<char> = text.iter().map(|&c| letter_of(c)).collect();
    let mut chars = Vec::with_capacity(letters.len());
    let mut i = 0;
    while i < letters.len() {
        if letters[i] == '\u{644}' {
            let mut at = i + 1;
            while letters.get(at).is_some_and(|&c| join(c) == Mark) {
                at += 1;
            }
            let alef = letters.get(at).and_then(|&c| {
                ['\u{622}', '\u{623}', '\u{625}', '\u{627}']
                    .iter()
                    .position(|&a| a == c)
            });
            if let Some(alef) = alef {
                chars.push(char::from_u32(0xFEF5 + 2 * alef as u32).unwrap_or('\u{644}'));
                chars.extend_from_slice(&letters[i + 1..at]);
                i = at + 1;
                continue;
            }
        }
        chars.push(letters[i]);
        i += 1;
    }
    // Whether the nearest letter before joins the next one, and the joins of
    // the nearest letter after.
    let mut joins_next = false;
    let mut out = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let next_joins = chars[i + 1..]
            .iter()
            .map(|&n| join(n))
            .find(|&j| j != Mark)
            .is_some_and(|j| j != Join::None);
        let kind = join(c);
        let shaped = match kind {
            Join::None => {
                joins_next = false;
                c
            }
            Mark => match c as u32 {
                m @ 0x064B..=0x0652 => {
                    let iso = 0xFE70 + 2 * (m - 0x064B);
                    let tatweel = joins_next && next_joins && !matches!(m, 0x064C | 0x064D);
                    char::from_u32(iso + u32::from(tatweel)).unwrap_or(c)
                }
                _ => c,
            },
            Right | Dual => {
                let to_next = kind == Dual && next_joins;
                let form = match (joins_next, to_next) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (true, true) => 3,
                };
                joins_next = kind == Dual;
                FORMS
                    .binary_search_by_key(&(c as u32), |&(letter, _)| letter)
                    .ok()
                    .and_then(|at| char::from_u32(FORMS[at].1 + form))
                    .unwrap_or(c)
            }
        };
        out.push(shaped);
    }
    out
}

/// `UCharacter.getMirror`.
fn mirror(c: char) -> char {
    if bidi_class(c) != BidiClass::ON {
        return c;
    }
    let mut pairs = MIRRORED.chars();
    while let (Some(a), Some(b)) = (pairs.next(), pairs.next()) {
        if c == a {
            return b;
        }
        if c == b {
            return a;
        }
    }
    c
}

/// Whether a character can change anything: right-to-left letters and numbers,
/// the explicit direction controls and the diacritics that shaping turns into
/// letters. Without any, left-to-right text keeps its order.
fn directional(c: char) -> bool {
    c as u32 >= 0x0590
        && (matches!(c as u32, 0x064B..=0x0652)
            || matches!(
                bidi_class(c),
                BidiClass::R
                    | BidiClass::AL
                    | BidiClass::AN
                    | BidiClass::LRE
                    | BidiClass::RLE
                    | BidiClass::LRO
                    | BidiClass::RLO
                    | BidiClass::PDF
                    | BidiClass::LRI
                    | BidiClass::RLI
                    | BidiClass::FSI
                    | BidiClass::PDI
            ))
}

/// Whether `chars` keep their order and shapes: left-to-right text without
/// anything directional, in a language that reads left to right by default.
fn unchanged(mut chars: impl Iterator<Item = char>, default_rtl: bool) -> bool {
    !default_rtl && !chars.any(directional)
}

/// `FormattedBidiReorder.reorder`: `chars` shaped, in visual order, with each
/// character's position in the shaped text. A character's style is the one at
/// its position in the shaped text, as `SubStringSource` takes it (the text
/// shrinks at a lam-alef ligature).
fn reorder(chars: &[char], default_rtl: bool) -> Vec<(char, usize)> {
    let shaped = shape(chars);
    let plain: String = shaped.iter().collect();
    // DEFAULT_RTL only changes text without a strong character.
    let level =
        (default_rtl && get_base_direction(plain.as_str()) == Direction::Mixed).then(Level::rtl);
    let info = ParagraphBidiInfo::new(&plain, level);
    let levels = info.reordered_levels_per_char(0..plain.len());
    ParagraphBidiInfo::reorder_visual(&levels)
        .into_iter()
        .map(|i| {
            let c = if levels[i].is_rtl() {
                mirror(shaped[i])
            } else {
                shaped[i]
            };
            (c, i)
        })
        .collect()
}

/// `Language.getVisualOrder` of a text (its `§` codes already resolved), for
/// the current language's direction.
pub fn visual(text: &Text) -> Cow<'_, Text> {
    let default_rtl = crate::lang::bidirectional();
    if unchanged(text.chars().map(|(c, _)| c), default_rtl) {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().map(|(c, _)| c).collect();
    let styles: Vec<_> = text.chars().map(|(_, style)| style).collect();
    let mut out = Text::default();
    for (c, at) in reorder(&chars, default_rtl) {
        match out.spans.last_mut() {
            Some(span) if span.style == *styles[at] => span.text.push(c),
            _ => out.spans.push(Span {
                text: c.to_string(),
                style: styles[at].clone(),
            }),
        }
    }
    Cow::Owned(out)
}

/// The same for a plain string.
pub fn visual_str(s: &str) -> Cow<'_, str> {
    let default_rtl = crate::lang::bidirectional();
    if unchanged(s.chars(), default_rtl) {
        return Cow::Borrowed(s);
    }
    let chars: Vec<char> = s.chars().collect();
    Cow::Owned(
        reorder(&chars, default_rtl)
            .into_iter()
            .map(|(c, _)| c)
            .collect(),
    )
}
