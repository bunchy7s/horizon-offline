//! `op`, `deop` and `whitelist`. While the server runs, the game owns the lists: the command goes to its
//! console. While it is stopped, they edit vanilla's files with the code the game reads them with
//! (horizon-server-files) and answer with vanilla's console messages.
//!
//! One exception: `whitelist add` on a running offline server. The game asks Mojang for the name first, as
//! vanilla does, so a name someone owns gets that account's UUID, and the player, who joins under the
//! offline UUID, is still refused. There the offline entry is written to whitelist.json and the game
//! reloads it (`whitelist reload`), as it would be written while the server is stopped.

use crate::{config, supervisor};
use horizon_server_files::user_lists::{NameAndId, OpEntry, OpList, WhiteEntry, WhiteList};
use std::path::Path;

pub fn run(dir: &Path, command: &str, args: &[String]) -> Result<(), String> {
    let config = config::Config::load(dir)?;
    // Holding the folder's lock keeps the server from starting halfway through.
    let Ok(_lock) = supervisor::lock(dir) else {
        if let ("whitelist", [add, name]) = (command, args)
            && add == "add"
            && !config.server.online_mode
        {
            return add_offline(dir, name);
        }
        let mut words = vec![command.to_owned()];
        words.extend_from_slice(args);
        return crate::cli::command(dir, &words);
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let ops_path = dir.join("ops.json");
    let white_path = dir.join("whitelist.json");
    let message = match (command, args.as_slice()) {
        ("op", [name]) => {
            let mut ops = load(OpList::new(ops_path))?;
            let user = account(name, config.server.online_mode)?;
            if ops.contains(&user) {
                "Nothing changed. The player already is an operator".to_owned()
            } else {
                ops.add(OpEntry {
                    user,
                    level: 4,
                    bypasses_player_limit: false,
                });
                format!("Made {name} a server operator")
            }
        }
        ("deop", [name]) => {
            let mut ops = load(OpList::new(ops_path))?;
            let user = listed(ops.entries().iter().map(|e| &e.user), name);
            if user.is_some_and(|user| ops.remove(&user)) {
                format!("Made {name} no longer a server operator")
            } else {
                "Nothing changed. The player is not an operator".to_owned()
            }
        }
        ("whitelist", ["add", name]) => {
            let mut white = load(WhiteList::new(white_path))?;
            let user = account(name, config.server.online_mode)?;
            if white.contains(&user) {
                "Player is already whitelisted".to_owned()
            } else {
                white.add(WhiteEntry { user });
                format!("Added {name} to the whitelist")
            }
        }
        ("whitelist", ["remove", name]) => {
            let mut white = load(WhiteList::new(white_path))?;
            let user = listed(white.entries().iter().map(|e| &e.user), name);
            if user.is_some_and(|user| white.remove(&user)) {
                format!("Removed {name} from the whitelist")
            } else {
                "Player is not whitelisted".to_owned()
            }
        }
        ("whitelist", [state @ ("on" | "off")]) => {
            let on = *state == "on";
            match (on, config.server.whitelist) {
                (true, true) => "Whitelist is already turned on".to_owned(),
                (false, false) => "Whitelist is already turned off".to_owned(),
                _ => {
                    config::update(dir, "server", "whitelist", toml::Value::Boolean(on))?;
                    format!("Whitelist is now turned {state}")
                }
            }
        }
        ("whitelist", ["list"]) => {
            let white = load(WhiteList::new(white_path))?;
            let names: Vec<String> = white
                .entries()
                .iter()
                .map(|e| e.user.name.clone())
                .collect();
            let state = if config.server.whitelist { "on" } else { "off" };
            if names.is_empty() {
                format!("There are no whitelisted players (the whitelist is {state})")
            } else {
                format!(
                    "There are {} whitelisted player(s): {} (the whitelist is {state})",
                    names.len(),
                    names.join(", ")
                )
            }
        }
        _ => {
            return Err(
                "use: op NAME, deop NAME, whitelist on|off|list, whitelist add|remove NAME".into(),
            );
        }
    };
    println!("{message}");
    Ok(())
}

/// `whitelist add` on a running offline server: the offline entry, then the game reloads the list.
fn add_offline(dir: &Path, name: &str) -> Result<(), String> {
    if crate::api::Client::open(dir)?.is_none() {
        return Err(format!("no server is running in {}", dir.display()));
    }
    let user = account(name, false)?;
    let mut white = load(WhiteList::new(dir.join("whitelist.json")))?;
    if white.contains(&user) {
        println!("Player is already whitelisted");
        return Ok(());
    }
    white.add(WhiteEntry { user });
    white
        .save()
        .map_err(|e| format!("{}: {e}", white.file.display()))?;
    crate::cli::command(dir, &["whitelist".to_owned(), "reload".to_owned()])?;
    println!("Added {name} to the whitelist");
    Ok(())
}

/// The profile a running server would store for `name`: the offline one, or in online mode the account the
/// services know by that name.
pub fn account(name: &str, online: bool) -> Result<NameAndId, String> {
    horizon_server_files::profiles::resolve(name, online).ok_or_else(|| {
        if online {
            format!("That player does not exist (no Minecraft account is named {name}, or the services didn't answer)")
        } else {
            "That player does not exist".to_owned()
        }
    })
}

/// The entry already on a list under `name` (any case), so removing needs no lookup.
fn listed<'a>(mut users: impl Iterator<Item = &'a NameAndId>, name: &str) -> Option<NameAndId> {
    users.find(|u| u.name.eq_ignore_ascii_case(name)).cloned()
}

/// A list from its file; a missing file is an empty list, a broken one an error (vanilla would replace it).
fn load<E: horizon_server_files::user_lists::Entry>(
    mut list: horizon_server_files::user_lists::UserList<E>,
) -> Result<horizon_server_files::user_lists::UserList<E>, String> {
    if list.file.exists() {
        list.load()
            .map_err(|e| format!("{}: {e}", list.file.display()))?;
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_the_lists_while_stopped() {
        let dir = std::env::temp_dir().join(format!("horizon-server-lists-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(config::FILE), config::TEMPLATE).unwrap();
        // This test edits local lists; it must not look names up through Mojang's services.
        config::update(&dir, "server", "online_mode", toml::Value::Boolean(false)).unwrap();
        let s = |v: &[&str]| v.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        // Each edit takes the folder's lock: retried past a fork-held descriptor (see supervisor::retried).
        let edit = |v: &[&str]| supervisor::retried(|| run(&dir, v[0], &s(&v[1..])));
        edit(&["op", "Alex"]).unwrap();
        edit(&["whitelist", "add", "Steve"]).unwrap();
        edit(&["whitelist", "off"]).unwrap();
        assert!(
            std::fs::read_to_string(dir.join("ops.json"))
                .unwrap()
                .contains("Alex")
        );
        assert!(
            std::fs::read_to_string(dir.join("whitelist.json"))
                .unwrap()
                .contains("Steve")
        );
        assert!(!config::Config::load(&dir).unwrap().server.whitelist);
        edit(&["deop", "Alex"]).unwrap();
        assert!(
            !std::fs::read_to_string(dir.join("ops.json"))
                .unwrap()
                .contains("Alex")
        );
        // A running server owns the lists: without its API, nothing changes.
        let lock = supervisor::retried(|| supervisor::lock(&dir)).unwrap();
        assert!(run(&dir, "op", &s(&["Alex"])).is_err());
        assert!(run(&dir, "whitelist", &s(&["add", "Kai"])).is_err());
        drop(lock);
        assert!(
            !std::fs::read_to_string(dir.join("ops.json"))
                .unwrap()
                .contains("Alex")
        );
        assert!(
            !std::fs::read_to_string(dir.join("whitelist.json"))
                .unwrap()
                .contains("Kai")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
