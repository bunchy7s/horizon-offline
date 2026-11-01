//! Instance-local picker entries. Hiding an entry never removes settings or world saves.
use std::{fs, io, path::Path};

const HIDDEN: &str = ".hidden";

pub fn names(root: &Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if entry.file_type()?.is_dir()
            && super::valid_name(&name)
            && !entry.path().join(HIDDEN).exists()
        {
            names.push(name);
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

fn valid(name: &str) -> io::Result<()> {
    if super::valid_name(name) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid offline profile name",
        ))
    }
}

pub fn hide(root: &Path, name: &str) -> io::Result<()> {
    valid(name)?;
    fs::write(root.join(name).join(HIDDEN), b"")
}

/// Restore the exact name: case changes would select another offline UUID.
pub fn restore(root: &Path, name: &str) -> io::Result<()> {
    valid(name)?;
    fs::create_dir_all(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let old = entry.file_name();
        let Some(old) = old.to_str() else { continue };
        if entry.file_type()?.is_dir() && old != name && old.eq_ignore_ascii_case(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Use the existing spelling: {old}"),
            ));
        }
    }
    fs::create_dir_all(root.join(name))?;
    match fs::remove_file(root.join(name).join(HIDDEN)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_and_restore_preserve_settings_and_stay_in_one_instance() {
        let root = std::env::temp_dir().join(format!("hz-profile-store-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        restore(&a, "jordank").unwrap();
        let options = a.join("jordank/options.txt");
        fs::write(&options, "guiScale:2\n").unwrap();
        assert_eq!(names(&a).unwrap(), ["jordank"]);
        assert!(names(&b).unwrap().is_empty());
        hide(&a, "jordank").unwrap();
        assert!(names(&a).unwrap().is_empty());
        assert!(restore(&a, "JordanK").is_err());
        restore(&a, "jordank").unwrap();
        assert_eq!(fs::read_to_string(options).unwrap(), "guiScale:2\n");
        assert_eq!(names(&a).unwrap(), ["jordank"]);
        assert!(hide(&a, "../bad").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
