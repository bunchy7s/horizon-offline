//! A text as the filter sees it: words of letters `a`..`z`, each letter with how many times it repeats.
//! Case, accents, look-alike letters (Cyrillic, Greek, full-width, small capitals, styled mathematical
//! letters, circled and squared letters), leetspeak and invisible characters are folded away;
//! separators inside a word are dropped (`n.i.g` is `nig`), and whitespace ends a word.

/// The letter code of anything that is a letter but none of `a`..`z`: it never matches.
const OTHER: u8 = 26;

/// One letter and how many times it repeats (`gg` is `g` twice).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// 0..=25 for `a`..`z`, 26 for any other letter.
    pub letter: u8,
    pub count: u8,
    /// Every one of its characters was punctuation read as a letter (`!` as `i`, `@` as `a`).
    pub weak: bool,
}

/// A whole message, normalised.
#[derive(Debug, Default)]
pub struct Text {
    pub runs: Vec<Run>,
    /// Each word's runs, `runs[start..end]`.
    pub words: Vec<(usize, usize)>,
    /// More ranges of `runs` to match as words: letters spaced out (`n i g g a`), joined, and words
    /// without the punctuation they end with (`fag!`).
    pub spaced: Vec<(usize, usize)>,
    /// Every word one after the other, a letter that ends one and starts the next merged.
    pub joined: Vec<Run>,
    /// Where each word starts in `joined`.
    pub joined_at: Vec<usize>,
    /// The last word each run of `joined` belongs to.
    joined_last: Vec<usize>,
}

enum Fold {
    Letter(u8, bool),
    Separator,
    Space,
    Skip,
}

impl Text {
    pub fn new(text: &str) -> Self {
        let mut t = Text::default();
        let mut start = 0;
        for c in text.chars() {
            match fold(c) {
                Fold::Letter(letter, weak) => {
                    let in_word = t.runs.len() > start;
                    match t.runs.last_mut() {
                        Some(last) if in_word && last.letter == letter => {
                            last.count = last.count.saturating_add(1);
                            last.weak &= weak;
                        }
                        _ => t.runs.push(Run {
                            letter,
                            count: 1,
                            weak,
                        }),
                    }
                }
                Fold::Space => {
                    t.end_word(start);
                    start = t.runs.len();
                }
                Fold::Separator | Fold::Skip => {}
            }
        }
        t.end_word(start);
        t.add_spaced();
        t.join();
        t
    }

    fn end_word(&mut self, start: usize) {
        if self.runs.len() > start {
            self.words.push((start, self.runs.len()));
        }
    }

    /// Spaced-out letters and words without their trailing punctuation, as extra words (their runs
    /// copied after the real words').
    fn add_spaced(&mut self) {
        let words = self.words.len();
        let mut w = 0;
        while w < words {
            let mut end = w;
            while end < words && self.words[end].1 - self.words[end].0 == 1 {
                end += 1;
            }
            if end - w >= 2 {
                let first = self.runs.len();
                for k in w..end {
                    let run = self.runs[self.words[k].0];
                    let in_word = self.runs.len() > first;
                    match self.runs.last_mut() {
                        Some(last) if in_word && last.letter == run.letter => {
                            last.count = last.count.saturating_add(run.count);
                        }
                        _ => self.runs.push(run),
                    }
                }
                self.spaced.push((first, self.runs.len()));
                w = end;
            } else {
                w += 1;
            }
        }
        for k in 0..words {
            let (start, mut end) = self.words[k];
            while end > start + 1 && self.runs[end - 1].weak {
                end -= 1;
            }
            if end < self.words[k].1 {
                self.spaced.push((start, end));
            }
        }
    }

    fn join(&mut self) {
        for (w, &(start, end)) in self.words.iter().enumerate() {
            let mut first = true;
            for run in &self.runs[start..end] {
                match self.joined.last_mut() {
                    Some(last) if first && last.letter == run.letter => {
                        last.count = last.count.saturating_add(run.count);
                        self.joined_at.push(self.joined.len() - 1);
                        *self.joined_last.last_mut().expect("beside joined") = w;
                    }
                    _ => {
                        if first {
                            self.joined_at.push(self.joined.len());
                        }
                        self.joined.push(*run);
                        self.joined_last.push(w);
                    }
                }
                first = false;
            }
        }
    }

    /// Whether `len` runs of `joined` from word `w`'s start reach into a later word.
    pub fn spans_words(&self, w: usize, len: usize) -> bool {
        self.joined_last
            .get(self.joined_at[w] + len - 1)
            .is_some_and(|&last| last > w)
    }
}

/// An entry of the list, normalised the same way (one word, whitespace ignored).
pub fn word(text: &str) -> Vec<Run> {
    let t = Text::new(&text.replace(char::is_whitespace, ""));
    t.words
        .first()
        .map_or_else(Vec::new, |&(s, e)| t.runs[s..e].to_vec())
}

fn letter(c: char) -> Fold {
    Fold::Letter(c as u8 - b'a', false)
}

fn fold(c: char) -> Fold {
    if c.is_ascii_alphabetic() {
        return letter(c.to_ascii_lowercase());
    }
    if c.is_ascii() {
        return match c {
            '0' => letter('o'),
            '1' => letter('i'),
            '3' => letter('e'),
            '4' => letter('a'),
            '5' => letter('s'),
            '6' | '9' => letter('g'),
            '7' => letter('t'),
            '8' => letter('b'),
            '!' | '|' => Fold::Letter(b'i' - b'a', true),
            '@' => Fold::Letter(0, true),
            '$' => Fold::Letter(b's' - b'a', true),
            c if c.is_ascii_whitespace() => Fold::Space,
            _ => Fold::Separator,
        };
    }
    let u = c as u32;
    let from = |base: u32, span: u32| char::from(b'a' + ((u - base) % span) as u8);
    let mapped = match u {
        // Zero-width characters, soft hyphen, combining marks, variation selectors.
        0xAD | 0x34F | 0x200B..=0x200F | 0x2060..=0x2064 | 0xFEFF | 0x180E => return Fold::Skip,
        0x300..=0x36F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0xE0100..=0xE01EF => return Fold::Skip,
        // Full-width Latin and digits.
        0xFF21..=0xFF3A => from(0xFF21, 26),
        0xFF41..=0xFF5A => from(0xFF41, 26),
        0xFF10..=0xFF19 => return fold(char::from(b'0' + (u - 0xFF10) as u8)),
        // Mathematical alphanumerics: 52 letters a style, then digits.
        0x1D400..=0x1D6A3 => from(0x1D400, 26),
        0x1D7CE..=0x1D7FF => return fold(char::from(b'0' + ((u - 0x1D7CE) % 10) as u8)),
        // Circled, parenthesised, squared and negative letters, regional indicators.
        0x24B6..=0x24CF => from(0x24B6, 26),
        0x24D0..=0x24E9 => from(0x24D0, 26),
        0x249C..=0x24B5 => from(0x249C, 26),
        0x1F130..=0x1F149 => from(0x1F130, 26),
        0x1F150..=0x1F169 => from(0x1F150, 26),
        0x1F170..=0x1F189 => from(0x1F170, 26),
        0x1F1E6..=0x1F1FF => from(0x1F1E6, 26),
        _ => {
            if c.is_whitespace() {
                return Fold::Space;
            }
            let lower = c.to_lowercase().next().unwrap_or(c);
            match LOOK_ALIKES.iter().find(|(_, chars)| chars.contains(lower)) {
                Some((base, _)) => *base,
                None if c.is_alphabetic() => return Fold::Letter(OTHER, false),
                None => return Fold::Separator,
            }
        }
    };
    letter(mapped)
}

/// Letters with accents and letters that look like a Latin one, by the letter they read as (lower case).
const LOOK_ALIKES: &[(char, &str)] = &[
    ('a', "àáâãäåāăąǎǟǡǻȁȃȧạảấầẩẫậắằẳẵặⱥɐаαᴀᵃª"),
    ('b', "ƀɓḃḅḇвьъβʙᵇ"),
    ('c', "çćĉċčƈḉȼсϲᴄᶜ"),
    ('d', "ďđɗḋḍḏḑḓԁδᴅᵈ"),
    ('e', "èéêëēĕėęěȅȇȩẹẻẽếềểễệḕḗḙḛḝɇеёєεᴇᵉ€"),
    ('f', "ƒḟꜰᶠ"),
    ('g', "ĝğġģǥǧǵɠḡɡɢᵍ"),
    ('h', "ĥħȟḣḥḧḩḫẖнһʜʰ"),
    ('i', "ìíîïĩīĭįıǐȉȋḭḯỉịіїιɪⁱ¡ӏ"),
    ('j', "ĵǰјᴊʲ"),
    ('k', "ķĸǩḱḳḵкκᴋᵏ"),
    ('l', "ĺļľŀłḷḹḻḽʟˡ"),
    ('m', "мᴍᵐ"),
    ('n', "ñńņňŉŋǹṅṇṉṋиипηπɴⁿ"),
    ('o', "òóôõöøōŏőơǒǫǭǿȍȏȫȭȯȱọỏốồổỗộớờởỡợоοσᴏᵒº"),
    ('p', "ṕṗрρᴘᵖ"),
    ('q', "ԛ"),
    ('r', "ŕŗřȑȓṙṛṝṟгʀʳ"),
    ('s', "śŝşšșṡṣṥṧṩѕꜱˢß"),
    ('t', "ţťŧțṫṭṯṱẗтτᴛᵗ"),
    ('u', "ùúûüũūŭůűųưǔǖǘǚǜȕȗụủứừửữựυᴜᵘ"),
    ('v', "νᴠᵛ"),
    ('w', "ŵẁẃẅẇẉẘԝωᴡʷ"),
    ('x', "хχˣ"),
    ('y', "ýÿŷȳẏẙỳỵỷỹуүγʏʸ"),
    ('z', "źżžƶẑẓẕᴢᶻ"),
];
