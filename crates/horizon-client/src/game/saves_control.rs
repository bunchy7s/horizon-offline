//! Worlds and the multiplayer server list from the control socket: what the Select World, Edit
//! World and Multiplayer screens do, by world folder. Changing a world needs the title screen
//! (no world open), as those screens do; deleting one needs `--confirm`.
use super::Game;
use horizon_client::{Result, title};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender, time::UNIX_EPOCH};

pub(super) const HELP: &[&str] = &[
    "worlds                         the saved worlds, newest first: folder, name, mode, last played, version",
    "world-rename FOLDER NAME       rename a world (Edit World; its folder stays)",
    "world-backup FOLDER            zip a world into backups/ (Edit World > Make Backup)",
    "world-delete FOLDER --confirm  delete a world for good (Select World > Delete)",
    "servers                        the multiplayer server list (servers.dat): name and address",
];

fn saves() -> Result<PathBuf> {
    Ok(std::env::current_dir()?.join("saves"))
}

impl Game<'_> {
    /// Some(reply) for these commands (Some(None): the reply comes later), None for others.
    pub(super) fn saves_control(
        &mut self,
        words: &[&str],
        line: &str,
        reply: &Sender<String>,
    ) -> Result<Option<Option<Value>>> {
        let folder = || {
            words
                .get(1)
                .copied()
                .ok_or_else(|| format!("{} FOLDER: the folder from worlds", words[0]))
        };
        let on_title = || {
            if self.title.is_some() {
                Ok(())
            } else {
                Err(format!(
                    "{} needs the title screen: save and leave the world first (pause, then Save and Quit to Title)",
                    words[0]
                ))
            }
        };
        let value = match words[0] {
            "worlds" => {
                let worlds: Vec<Value> = title::list_worlds(&saves()?)
                    .iter()
                    .map(|w| {
                        json!({
                            "folder": w.folder,
                            "name": w.name,
                            "mode": if w.game_mode == 1 { "creative" } else { "survival" },
                            "last_played": w.modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
                            "commands": w.commands,
                            "version": w.version,
                        })
                    })
                    .collect();
                json!({ "worlds": worlds })
            }
            "world-rename" => {
                on_title()?;
                let folder = folder()?;
                let name = line
                    .trim_start()
                    .strip_prefix("world-rename")
                    .and_then(|rest| rest.trim_start().strip_prefix(folder))
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .ok_or("world-rename FOLDER NAME: missing the new name")?;
                title::world_management::rename(&saves()?, folder, name)?;
                if let Some(title) = self.title.as_mut() {
                    title.worlds_changed();
                }
                json!({ "ok": true, "folder": folder, "name": name })
            }
            "world-delete" => {
                on_title()?;
                let folder = folder()?;
                if !words.contains(&"--confirm") {
                    return Err(format!(
                        "world-delete deletes {folder} for good: ask the player, then add --confirm"
                    )
                    .into());
                }
                title::world_management::delete(&saves()?, folder)?;
                if let Some(title) = self.title.as_mut() {
                    title.worlds_changed();
                }
                json!({ "ok": true, "deleted": folder })
            }
            "world-backup" => {
                on_title()?;
                let folder = folder()?.to_owned();
                let saves = saves()?;
                let backups = title::world_management::backups_path(&saves);
                // A big world takes a while to zip: off the game thread, as Make Backup does.
                let reply = reply.clone();
                std::thread::spawn(move || {
                    let answer = match title::world_management::backup(&saves, &folder) {
                        Ok(bytes) => {
                            json!({ "ok": true, "folder": folder, "backups": backups, "bytes": bytes })
                        }
                        Err(e) => json!({ "error": format!("backup of {folder} failed: {e}") }),
                    };
                    let _ = reply.send(answer.to_string());
                });
                return Ok(Some(None));
            }
            "servers" => {
                let file = horizon_client::multiplayer::servers_file(&saves()?);
                let servers: Vec<Value> = horizon_protocol::servers::load(&file)
                    .iter()
                    .map(|s| json!({ "name": s.name, "address": s.ip }))
                    .collect();
                json!({ "servers": servers })
            }
            _ => return Ok(None),
        };
        Ok(Some(Some(value)))
    }
}
