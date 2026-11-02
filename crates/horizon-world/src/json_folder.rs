//! A data folder's files read from the disk once while a load holds them.
//! Several tables are built from the same folder (the recipes: the crafting
//! tables, the stations, the recipe book, brewing), and each used to list and
//! read every file again: on a hard disk or under a virus scanner, each open
//! costs. The first reader lists and reads the folder; later readers share
//! those bytes for as long as one of them holds the [`Folder`], then it is
//! gone (nothing is kept past the load).
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

/// One file: its path inside the folder without `.json` (`a/b`, as an id has
/// it), the file, and its bytes or the error reading it gave.
pub struct File {
    pub name: String,
    pub path: PathBuf,
    bytes: Result<Box<[u8]>, (io::ErrorKind, String)>,
}
impl File {
    pub fn bytes(&self) -> io::Result<&[u8]> {
        self.bytes
            .as_deref()
            .map_err(|(kind, message)| io::Error::new(*kind, message.clone()))
    }
}

/// Every `.json` file under a folder, its subfolders' too, in name order.
pub struct Folder {
    pub files: Vec<File>,
}

static OPEN: Mutex<Vec<(PathBuf, Weak<Folder>)>> = Mutex::new(Vec::new());

fn find(open: &[(PathBuf, Weak<Folder>)], dir: &Path) -> Option<Arc<Folder>> {
    open.iter()
        .find(|(d, _)| d == dir)
        .and_then(|(_, f)| f.upgrade())
}

/// `dir`'s files if another reader holds them, without reading anything.
pub fn held(dir: &Path) -> Option<Arc<Folder>> {
    find(&OPEN.lock().unwrap_or_else(|e| e.into_inner()), dir)
}

/// The `.json` files under `dir`: shared with any other reader holding them,
/// else listed and read now. An error when `dir` itself can't be listed.
pub fn read(dir: &Path) -> io::Result<Arc<Folder>> {
    if let Some(folder) = held(dir) {
        return Ok(folder);
    }
    let files = list(dir)?
        .into_iter()
        .map(|(name, path)| {
            let bytes = fs::read(&path)
                .map(Vec::into_boxed_slice)
                .map_err(|e| (e.kind(), e.to_string()));
            File { name, path, bytes }
        })
        .collect();
    let folder = Arc::new(Folder { files });
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    // Another reader may have read it meanwhile: share one copy.
    if let Some(theirs) = find(&open, dir) {
        return Ok(theirs);
    }
    open.retain(|(_, f)| f.strong_count() > 0);
    open.push((dir.to_owned(), Arc::downgrade(&folder)));
    Ok(folder)
}

/// The `.json` files under `dir` and its subfolders, in name order: each
/// one's path inside the folder without the extension (`a/b`) and its file.
/// An error when `dir` itself can't be listed; a subfolder or an entry that
/// can't be is warned and skipped, as vanilla's `PathPackResources` keeps
/// what it found.
pub fn list(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    let mut stack = vec![fs::read_dir(dir)?];
    while let Some(entries) = stack.pop() {
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    log::warn!("Horizon: failed to list an entry of {}: {e}", dir.display());
                    continue;
                }
            };
            let path = entry.path();
            if path.is_dir() {
                match fs::read_dir(&path) {
                    Ok(list) => stack.push(list),
                    Err(e) => log::warn!("Horizon: failed to list {}: {e}", path.display()),
                }
            } else if path.extension().is_some_and(|e| e == "json") {
                files.push(path);
            }
        }
    }
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| {
            let inside = path.strip_prefix(dir).unwrap_or(&path).with_extension("");
            let name: Vec<_> = inside.iter().map(|c| c.to_string_lossy()).collect();
            (name.join("/"), path)
        })
        .collect();
    // Vanilla lists resources by id, in a map keyed by it
    // (`FallbackResourceManager`): `crushing` before `crushing/iron`, where
    // a path's components put the subfolder's file first.
    named.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-json-folder-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("brewing")).unwrap();
        fs::write(dir.join("stick.json"), b"{\"a\":1}").unwrap();
        fs::write(dir.join("brewing/awkward.json"), b"{}").unwrap();
        fs::write(dir.join("notes.txt"), b"not json").unwrap();
        dir
    }

    #[test]
    fn readers_share_the_bytes_while_one_holds_them_and_read_again_after() {
        let dir = folder("share");
        let first = read(&dir).unwrap();
        let names: Vec<_> = first.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["brewing/awkward", "stick"]);
        assert_eq!(first.files[1].bytes().unwrap(), b"{\"a\":1}");
        // Held: a change on the disk isn't seen, the copy read is shared.
        fs::write(dir.join("stick.json"), b"{\"a\":2}").unwrap();
        let second = read(&dir).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        drop((first, second));
        // Let go: the next reader reads the folder again.
        assert_eq!(read(&dir).unwrap().files[1].bytes().unwrap(), b"{\"a\":2}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_listed_is_an_error() {
        let dir =
            std::env::temp_dir().join(format!("horizon-json-folder-none-{}", std::process::id()));
        assert!(read(&dir).is_err());
    }
}
