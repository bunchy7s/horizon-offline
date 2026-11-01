//! The code of conduct (`enable-code-of-conduct`): `DedicatedServer.readCodeOfConducts`
//! reads `codeofconduct/<language>.txt`, and
//! `ServerConfigurationPacketListenerImpl.addOptionalTasks` offers the one
//! that fits the player's language.
use std::path::Path;

/// `StringUtil.stripColor`: the pattern `(?i)§[0-9A-FK-OR]` removed.
fn strip_color(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{a7}'
            && chars.peek().is_some_and(
                |n| matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r'),
            )
        {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// `String.join("\n", Files.readAllLines(file))`: lines end at `\n`, `\r`
/// or `\r\n`, and the last line's terminator is dropped.
fn join_lines(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let bytes = text.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\n' | b'\r' => {
                lines.push(&text[start..i]);
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines.join("\n")
}

/// `DedicatedServer.readCodeOfConducts`: each `<language>.txt` of `dir`, by
/// its lowercased name, with the colour codes stripped. Files come in name
/// order (vanilla's is the file system's).
pub fn read(dir: &Path) -> Result<Vec<(String, String)>, String> {
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Err(format!(
            "Code of Conduct folder does not exist: {}",
            dir.display()
        ));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read Code of Conduct folder: {e}"))?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    let allowed = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for file in files {
        let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(language) = name.strip_suffix(".txt") else {
            continue;
        };
        let real = std::fs::canonicalize(&file).map_err(|e| e.to_string())?;
        if real.parent() != Some(allowed.as_path()) {
            return Err(format!(
                "Failed to read Code of Conduct file \"{name}\" because it links to a file outside the allowed directory"
            ));
        }
        let bytes = std::fs::read(&file)
            .map_err(|e| format!("Failed to read Code of Conduct file {name}: {e}"))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("Failed to read Code of Conduct file {name}"))?;
        out.push((language.to_lowercase(), strip_color(&join_lines(&text))));
    }
    Ok(out)
}

/// The supplier `addOptionalTasks` gives the task: the player's language,
/// else `en_us`, else the first one. None when the server has none.
pub fn pick<'a>(texts: &'a [(String, String)], language: &str) -> Option<&'a str> {
    let find = |key: &str| {
        texts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    find(&language.to_lowercase())
        .or_else(|| find("en_us"))
        .or_else(|| texts.first().map(|(_, v)| v.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts() -> Vec<(String, String)> {
        vec![
            ("de_de".into(), "Regeln".into()),
            ("en_us".into(), "Rules".into()),
        ]
    }

    #[test]
    fn the_players_language_then_en_us_then_the_first() {
        let t = texts();
        assert_eq!(pick(&t, "DE_de"), Some("Regeln"));
        assert_eq!(pick(&t, "fr_fr"), Some("Rules"));
        let only_de = &t[..1];
        assert_eq!(pick(only_de, "fr_fr"), Some("Regeln"));
        assert_eq!(pick(&[], "en_us"), None);
    }

    #[test]
    fn colour_codes_are_stripped_and_lines_joined_without_the_last_newline() {
        assert_eq!(
            strip_color("\u{a7}aGreen \u{a7}Rx \u{a7}z \u{a7}"),
            "Green x \u{a7}z \u{a7}"
        );
        assert_eq!(join_lines("a\r\nb\rc\n"), "a\nb\nc");
        assert_eq!(join_lines("a\n\n"), "a\n");
        assert_eq!(join_lines(""), "");
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let dir = std::env::temp_dir().join("horizon-no-such-coc-folder");
        assert!(
            read(&dir)
                .unwrap_err()
                .starts_with("Code of Conduct folder does not exist")
        );
    }

    #[test]
    fn only_txt_files_are_read_by_lowercased_name() {
        let dir = std::env::temp_dir().join(format!("horizon-coc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("EN_US.txt"), "\u{a7}lBe nice\nplease\n").unwrap();
        std::fs::write(dir.join("notes.md"), "ignored").unwrap();
        let read = read(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            read,
            vec![("en_us".to_string(), "Be nice\nplease".to_string())]
        );
    }
}
