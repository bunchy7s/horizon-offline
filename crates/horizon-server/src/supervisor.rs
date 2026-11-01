//! `horizon-server run`: writes `server.properties` from `server.toml`, runs the engine with the console on
//! stdin, serves the control API, stops the game cleanly on Ctrl-C, SIGTERM or `stop`, and restarts it when
//! it crashes (docs/server.md, sections 2 and 7). An update restarts the game on another release and goes
//! back by itself when the new game doesn't come up.

use crate::{
    api::{self, Phase, Shared},
    backup, config,
    config::Config,
    engine, signals, update,
};
use serde_json::json;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ExitStatus},
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};

/// After this many crashes within [`CRASH_WINDOW`] the supervisor stops restarting the game.
const MAX_CRASHES: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(600);
/// The first wait before a restart; it doubles after each crash, up to [`MAX_BACKOFF`].
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// How long `stop` may take before the game is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(120);

/// A line from the supervisor, in the game's log format (`[HH:mm:ss] [thread/LEVEL]: message`, local time
/// as the game's own lines), on stdout and in the log the API serves.
pub fn note(shared: &Shared, level: &str, message: &str) {
    let line = format!(
        "[{}] [Supervisor/{level}]: {message}",
        chrono::Local::now().format("%H:%M:%S")
    );
    println!("{line}");
    shared.log_line(line);
}

/// The folder's lock: one supervisor per server folder. Released when the process ends, even by a crash.
pub fn lock(dir: &Path) -> Result<File, String> {
    let state = dir.join(".horizon");
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(format!("a server is already running in {}", dir.display()))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
    }
}

/// `attempt` (one that takes the folder's lock) retried for up to a second. A process another test
/// spawns holds a copy of the lock's descriptor between its fork and exec: the lock comes back once that
/// child has exec'd.
#[cfg(test)]
pub(crate) fn retried<T>(mut attempt: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let mut result = attempt();
    for _ in 0..50 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        result = attempt();
    }
    result
}

enum Exit {
    /// Asked to stop, or the game ended by itself without an error (`stop` typed in its console, the EULA
    /// not agreed, the port taken): not restarted.
    Stopped,
    /// Asked to restart (`horizon-server restart`).
    Restart,
    Crashed(ExitStatus),
    /// Started on another release, the game wasn't up in time and was stopped.
    NotUp,
}

/// A game started on another release that has yet to come up: what to go back to if it doesn't.
struct Watch {
    from: String,
    to: String,
    /// The release `previous` named before the switch.
    previous: Option<String>,
    /// The backup of the world made just before the switch.
    backup: String,
    timeout: Duration,
    deadline: Instant,
    /// `updates.keep`: the other releases kept once the game is up.
    keep: u32,
}

pub fn run(dir: &Path) -> Result<(), String> {
    signals::install();
    let config = Config::load(dir)?;
    let program = engine::program(&config, dir)?;
    let resources = engine::resources(&config, dir, Some(&program))?;
    // Half-copied game files would only crash the engine again at every restart: name what is missing.
    engine::check_prepared(&resources)?;
    // A game the system can't load would only crash again at every restart: say why at once.
    engine::check_starts(&program)?;
    let _lock = lock(dir)?;
    let shared = Shared::new(dir);
    let _endpoint = api::start(shared.clone())?;
    if let Some(address) = crate::management::start(shared.clone())? {
        note(
            &shared,
            "INFO",
            &format!("Json-RPC Management connection listening on {address}"),
        );
    }
    crate::rcon::start(shared.clone())?;
    match config.panel.address() {
        Ok(Some(address)) => match crate::panel::start(shared.clone(), address) {
            Ok(()) => note(
                &shared,
                "INFO",
                &format!(
                    "Web panel on http://{address} (`horizon-server panel` prints a login link)"
                ),
            ),
            Err(e) => note(
                &shared,
                "WARN",
                &format!("The web panel can't listen on {address}: {e}"),
            ),
        },
        Ok(None) => {}
        Err(e) => note(&shared, "WARN", &e),
    }
    sample(shared.clone());
    crate::events::poll_game(shared.clone());
    crate::discord::start(shared.clone(), config.discord.clone());
    schedule_backups(shared.clone());
    check_updates(shared.clone());
    let console = console_lines();
    let mut crashes: Vec<Instant> = Vec::new();
    let mut watch: Option<Watch> = None;
    let (mut program, mut resources) = (program, resources);
    loop {
        // Read again before every start: edits to server.toml apply on the next restart, and so does a
        // switch to another release.
        let config = Config::load(dir)?;
        if let Ok(found) = engine::program_in(&config, dir, shared.install.as_ref())
            && let Ok(found_resources) = engine::resources(&config, dir, Some(&found))
        {
            (program, resources) = (found, found_resources);
        }
        let written = config
            .write_properties(dir)
            .map_err(|e| format!("server.properties: {e}"))?;
        note(
            &shared,
            "INFO",
            &format!(
                "Starting {} with the game files in {}",
                program.display(),
                resources.display()
            ),
        );
        shared.set_phase(Phase::Starting);
        let mut child = engine::spawn(&program, &resources, dir, config.engine.low_memory)
            .map_err(|e| format!("{}: {e}", program.display()))?;
        {
            let mut state = shared.state.lock().unwrap();
            state.engine_pid = Some(child.id());
            state.engine_started = Some(Instant::now());
        }
        // The new release's time to come up starts with its game.
        if let Some(w) = &mut watch {
            w.deadline = Instant::now() + w.timeout;
        }
        let exit = supervise(&shared, &mut child, &console, &mut watch);
        {
            let mut state = shared.state.lock().unwrap();
            state.engine_pid = None;
            state.engine_started = None;
        }
        match config::sync_back(dir, &written) {
            Ok(keys) if !keys.is_empty() => note(
                &shared,
                "INFO",
                &format!(
                    "Copied into server.toml what the game changed: {}",
                    keys.join(", ")
                ),
            ),
            Ok(_) => {}
            Err(e) => note(
                &shared,
                "WARN",
                &format!("Could not update server.toml: {e}"),
            ),
        }
        // Started on another release, the game never came up: back to the release before.
        if let Some(w) = watch.take() {
            let reason = match &exit {
                Exit::Crashed(status) => format!("the game stopped ({status})"),
                Exit::NotUp => "the game wasn't up in time".into(),
                Exit::Stopped => "the game stopped before it was up".into(),
                Exit::Restart => "the game was restarted before it was up".into(),
            };
            roll_back(&shared, dir, &config, &w, &reason);
            if matches!(exit, Exit::Stopped)
                && shared.state.lock().unwrap().phase == Phase::Stopping
            {
                return Ok(());
            }
            continue;
        }
        let status = match exit {
            Exit::Stopped => return Ok(()),
            Exit::NotUp => continue,
            Exit::Restart => {
                let restore = shared.restore.lock().unwrap().take();
                if let Some(name) = restore {
                    match backup::restore(dir, &config, &name) {
                        Ok(Some(old)) => note(
                            &shared,
                            "INFO",
                            &format!(
                                "Restored {name}; the world it replaced is in {}",
                                old.display()
                            ),
                        ),
                        Ok(None) => note(&shared, "INFO", &format!("Restored {name}")),
                        Err(e) => note(&shared, "ERROR", &format!("Could not restore {name}: {e}")),
                    }
                }
                let pending = shared.update.lock().unwrap().pending.take();
                if let Some((to, timeout)) = pending {
                    watch = switch(&shared, dir, &config, &to, timeout);
                }
                continue;
            }
            Exit::Crashed(status) => status,
        };
        {
            let mut state = shared.state.lock().unwrap();
            state.crashes += 1;
            state.last_crash = Some(status.to_string());
        }
        if !config.engine.restart_on_crash {
            return Err(format!(
                "the game stopped unexpectedly ({status}); engine.restart_on_crash is off"
            ));
        }
        let now = Instant::now();
        crashes.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        crashes.push(now);
        if crashes.len() >= MAX_CRASHES {
            return Err(format!(
                "the game stopped unexpectedly {MAX_CRASHES} times in {} minutes: not restarting it. \
                 See logs/latest.log and crash-reports/",
                CRASH_WINDOW.as_secs() / 60
            ));
        }
        let wait = backoff(crashes.len());
        note(
            &shared,
            "WARN",
            &format!(
                "The game stopped unexpectedly ({status}); restarting in {} s",
                wait.as_secs()
            ),
        );
        shared.set_phase(Phase::Restarting);
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if signals::requests() > 0 || shared.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// With the game stopped: backs the world up, points `current` at the release `to` and checks that its game
/// loads on this system. What to watch while it starts; `None` when it stayed on the release it was on.
fn switch(
    shared: &Shared,
    dir: &Path,
    config: &Config,
    to: &str,
    timeout: Duration,
) -> Option<Watch> {
    let Some(install) = shared.install.as_ref() else {
        shared.finish_update(
            false,
            "Not updated: this horizon-server wasn't installed".into(),
        );
        return None;
    };
    let Some(from) = install.current() else {
        shared.finish_update(false, "Not updated: `current` points at no release".into());
        return None;
    };
    let made = backup::file_name(std::time::SystemTime::now(), Some(&update::backup_name(to)))
        .and_then(|file| backup::create(dir, config, &file).map(|_| file));
    let backup = match made {
        Ok(file) => file,
        Err(e) => {
            shared.finish_update(false, format!("Stayed on {from}: the backup failed: {e}"));
            return None;
        }
    };
    note(
        shared,
        "INFO",
        &format!(
            "Backed up the world to {}/{backup}; switching from {from} to {to}",
            backup::DIR
        ),
    );
    let previous = install.previous();
    if let Err(e) = install.switch(to) {
        shared.finish_update(false, format!("Stayed on {from}: {e}"));
        return None;
    }
    if let Err(e) =
        engine::program_in(config, dir, Some(install)).and_then(|p| engine::check_starts(&p))
    {
        let back = install
            .switch_back(&from, previous.as_deref())
            .err()
            .map_or(String::new(), |e| format!(" ({e})"));
        shared.finish_update(false, format!("Stayed on {from}{back}: {e}"));
        return None;
    }
    shared.update.lock().unwrap().busy = Some(format!("Starting the game on {to}"));
    Some(Watch {
        from,
        to: to.to_owned(),
        previous,
        backup,
        timeout,
        deadline: Instant::now() + timeout,
        keep: config.updates.keep,
    })
}

/// The game is up on the release switched to: the update is done, and the old releases beyond the ones
/// kept are deleted.
fn came_up(shared: &Shared, w: &Watch) {
    let mut message = format!("Now on {} (was {}): the game is up", w.to, w.from);
    if let Some(install) = &shared.install {
        let (removed, failed) = install.prune(w.keep);
        let pruned = update::pruned(&removed, &failed);
        if !pruned.is_empty() {
            message = format!("{message}. {pruned}");
        }
    }
    shared.finish_update(true, message);
}

/// Undoes a switch whose game never came up: `current` back on the release before, and the world as it
/// was just before the switch (the one the new game left is kept in backups/).
fn roll_back(shared: &Shared, dir: &Path, config: &Config, w: &Watch, reason: &str) {
    note(
        shared,
        "ERROR",
        &format!(
            "The game on {} didn't come up ({reason}): going back to {}",
            w.to, w.from
        ),
    );
    let Some(install) = shared.install.as_ref() else {
        return;
    };
    if let Err(e) = install.switch_back(&w.from, w.previous.as_deref()) {
        shared.finish_update(
            false,
            format!(
                "{} didn't come up ({reason}), and going back to {} failed: {e}",
                w.to, w.from
            ),
        );
        return;
    }
    let world = match backup::restore(dir, config, &w.backup) {
        Ok(Some(old)) => format!(
            "; the world is back as it was before the switch (the one {} left is in {})",
            w.to,
            old.display()
        ),
        Ok(None) => "; the world is back as it was before the switch".into(),
        Err(e) => format!("; the world couldn't be put back from {}: {e}", w.backup),
    };
    shared.finish_update(
        false,
        format!(
            "Rolled back to {}: {} didn't come up ({reason}){world}",
            w.from, w.to
        ),
    );
}

/// The game is up: it logged `Done` and answered a status request (the sampler's, for this process).
fn up(shared: &Shared) -> bool {
    let state = shared.state.lock().unwrap();
    state.phase == Phase::Running
        && state.engine_pid.is_some()
        && state.sample["pid"].as_u64() == state.engine_pid.map(u64::from)
        && !state.sample["game"].is_null()
}

/// Looks for a newer release a minute after the start and then every 6 hours (installed servers only;
/// `HORIZON_NO_UPDATE_CHECK=1` turns it off). It only tells: updating stays the owner's choice.
fn check_updates(shared: Arc<Shared>) {
    let Some(install) = shared.install.clone() else {
        return;
    };
    if std::env::var_os("HORIZON_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    let _ = thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            thread::sleep(Duration::from_secs(60));
            loop {
                if let (Ok(release), Some(current)) = (update::release(None), install.current()) {
                    let newer = update::update_available(&release.tag, &current)
                        .then_some(release.tag);
                    let told = shared.update.lock().unwrap().available.clone();
                    if let Some(tag) = newer.as_ref().filter(|t| told.as_ref() != Some(*t)) {
                        note(
                            &shared,
                            "INFO",
                            &format!(
                                "Horizon {tag} is out (this server runs {current}): `horizon-server update` \
                                 installs it, and goes back by itself if it doesn't start"
                            ),
                        );
                    }
                    shared.update.lock().unwrap().available = newer;
                }
                thread::sleep(Duration::from_secs(6 * 3600));
            }
        });
}

/// The wait before the restart that follows the `crashes`th recent crash.
fn backoff(crashes: usize) -> Duration {
    let doublings = u32::try_from(crashes.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    (FIRST_BACKOFF * 2u32.pow(doublings)).min(MAX_BACKOFF)
}

/// The supervisor's own stdin, line by line (nothing when it has none, as under a service manager).
fn console_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    rx
}

/// Copies the game's output to ours and to the API's log, line by line; the game's `Done` line means it
/// is up.
fn pump(shared: &Arc<Shared>, from: impl Read + Send + 'static, to_stderr: bool) {
    let shared = shared.clone();
    let _ = thread::Builder::new()
        .name("engine-log".into())
        .spawn(move || {
            for line in BufReader::new(from).lines() {
                let Ok(line) = line else { break };
                if to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                    if line.contains("/INFO]: Done (") {
                        let up = {
                            let mut state = shared.state.lock().unwrap();
                            let starting = state.phase == Phase::Starting;
                            if starting {
                                state.phase = Phase::Running;
                            }
                            starting
                        };
                        if up {
                            shared.emit("phase", serde_json::json!({ "phase": "running" }));
                        }
                    }
                }
                shared.log_line(line);
            }
        });
}

/// Every second, the game's RAM, CPU and status, for `status` and the panel's graphs.
fn sample(shared: Arc<Shared>) {
    let _ = thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut control: Option<engine::Control> = None;
            let mut previous: Option<(u32, Instant, f64)> = None;
            loop {
                thread::sleep(Duration::from_secs(1));
                let (pid, running) = {
                    let state = shared.state.lock().unwrap();
                    (state.engine_pid, state.phase == Phase::Running)
                };
                let Some(pid) = pid else {
                    control = None;
                    previous = None;
                    shared.state.lock().unwrap().sample = serde_json::Value::Null;
                    continue;
                };
                let now = Instant::now();
                let cpu = horizon_bench::cpu_seconds(pid);
                let cpu_percent = match (previous, cpu) {
                    (Some((p, at, before)), Some(cpu)) if p == pid => {
                        Some((cpu - before) / now.duration_since(at).as_secs_f64() * 100.0)
                    }
                    _ => None,
                };
                previous = cpu.map(|c| (pid, now, c));
                if !running {
                    control = None;
                } else if control.is_none() {
                    control = engine::Control::connect(&shared.dir).ok();
                }
                let game = control
                    .as_mut()
                    .map(|c| c.ask("server-status"))
                    .transpose()
                    .unwrap_or_else(|_| {
                        control = None;
                        None
                    });
                let rss = horizon_bench::rss_bytes(pid);
                shared.record(json!({
                    "t": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs()),
                    "tps": game.as_ref().and_then(|g| g["tps"].as_f64()),
                    "mspt": game.as_ref().and_then(|g| g["mspt"]["average"].as_f64()),
                    "mspt_p95": game.as_ref().and_then(|g| g["mspt"]["p95"].as_f64()),
                    "rss_bytes": rss,
                    "cpu_percent": cpu_percent,
                    "players": game.as_ref().and_then(|g| g["players"].as_array().map(Vec::len)),
                }));
                shared.state.lock().unwrap().sample = json!({
                    "pid": pid,
                    "rss_bytes": rss,
                    "cpu_percent": cpu_percent,
                    "game": game,
                });
            }
        });
}

/// Automatic backups every `backups.every` while the game runs, keeping the last `backups.keep`.
fn schedule_backups(shared: Arc<Shared>) {
    let _ = thread::Builder::new()
        .name("backups".into())
        .spawn(move || {
            let mut last = Instant::now();
            loop {
                thread::sleep(Duration::from_secs(30));
                let Ok(config) = Config::load(&shared.dir) else {
                    continue;
                };
                let Ok(Some(every)) = backup::interval(&config.backups.every) else {
                    continue;
                };
                if last.elapsed() < every || shared.state.lock().unwrap().phase != Phase::Running {
                    continue;
                }
                last = Instant::now();
                match shared.back_up(&backup::auto_name(std::time::SystemTime::now())) {
                    Ok(path) => {
                        note(&shared, "INFO", &format!("Backed up to {}", path.display()));
                        let pruned = backup::prune(&shared.dir, config.backups.keep as usize);
                        if !pruned.is_empty() {
                            note(
                                &shared,
                                "INFO",
                                &format!("Deleted older backups: {}", pruned.join(", ")),
                            );
                        }
                    }
                    Err(e) => note(
                        &shared,
                        "WARN",
                        &format!("The automatic backup failed: {e}"),
                    ),
                }
            }
        });
}

/// Runs one engine process to its end: console lines in, the log out, and a clean `stop` when asked.
fn supervise(
    shared: &Arc<Shared>,
    child: &mut Child,
    console: &mpsc::Receiver<String>,
    watch: &mut Option<Watch>,
) -> Exit {
    if let Some(out) = child.stdout.take() {
        pump(shared, out, false);
    }
    if let Some(err) = child.stderr.take() {
        pump(shared, err, true);
    }
    let mut stdin = child.stdin.take();
    let mut stopping: Option<Instant> = None;
    let mut restart = false;
    let mut not_up = false;
    let mut handled = signals::requests();
    loop {
        if let Some(w) = watch.as_ref() {
            if up(shared) {
                came_up(shared, w);
                *watch = None;
            } else if Instant::now() > w.deadline && stopping.is_none() {
                note(
                    shared,
                    "WARN",
                    &format!("The game on {} isn't up in time: stopping it", w.to),
                );
                not_up = true;
                stopping = Some(Instant::now());
                let _ = child.kill();
            }
        }
        while let Ok(line) = console.try_recv() {
            let command = line.trim().trim_start_matches('/');
            if command == "stop" && stopping.is_none() {
                stopping = Some(Instant::now());
                shared.set_phase(Phase::Stopping);
            }
            if let Some(input) = stdin.as_mut()
                && writeln!(input, "{line}")
                    .and_then(|()| input.flush())
                    .is_err()
            {
                stdin = None;
            }
        }
        let requests = signals::requests();
        let asked = requests > handled || shared.stop.swap(false, Ordering::SeqCst);
        handled = requests;
        if shared.restart.swap(false, Ordering::SeqCst) && stopping.is_none() {
            restart = true;
            note(shared, "INFO", "Restarting the server");
            stopping = Some(Instant::now());
            shared.set_phase(Phase::Restarting);
            send_stop(&mut stdin);
        }
        if asked {
            restart = false;
            if stopping.is_none() {
                note(shared, "INFO", "Stopping the server");
                stopping = Some(Instant::now());
                shared.set_phase(Phase::Stopping);
                send_stop(&mut stdin);
            } else {
                note(shared, "WARN", "Stopping the game at once, without saving");
                let _ = child.kill();
            }
        }
        if stopping.is_some_and(|t| t.elapsed() > STOP_TIMEOUT) {
            note(
                shared,
                "WARN",
                &format!(
                    "The game did not stop within {} s: killing it",
                    STOP_TIMEOUT.as_secs()
                ),
            );
            let _ = child.kill();
            stopping = Some(Instant::now());
        }
        match child.try_wait() {
            Ok(Some(_)) if not_up => return Exit::NotUp,
            Ok(Some(_)) if restart => return Exit::Restart,
            Ok(Some(status)) if stopping.is_some() || status.success() => return Exit::Stopped,
            Ok(Some(status)) => return Exit::Crashed(status),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                note(shared, "ERROR", &format!("Lost track of the game: {e}"));
                return Exit::Stopped;
            }
        }
    }
}

fn send_stop(stdin: &mut Option<std::process::ChildStdin>) {
    if let Some(input) = stdin.as_mut() {
        let _ = writeln!(input, "stop").and_then(|()| input.flush());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_five_minutes() {
        let waits: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300]);
    }

    /// An install with releases whose game exits with `code` on `--help`, and a server folder with a world.
    #[cfg(unix)]
    fn installed(name: &str, releases: &[(&str, i32)]) -> (Arc<Shared>, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("horizon-server-sw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let root = {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::canonicalize(&root).unwrap()
        };
        for (tag, code) in releases {
            let release = root.join("releases").join(tag);
            std::fs::create_dir_all(&release).unwrap();
            std::fs::write(release.join("horizon-server"), "").unwrap();
            let engine = release.join("horizon-engine");
            std::fs::write(&engine, format!("#!/bin/sh\nexit {code}\n")).unwrap();
            std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::os::unix::fs::symlink(
            root.join("releases").join(releases[0].0),
            root.join("current"),
        )
        .unwrap();
        let dir = root.join("server");
        std::fs::create_dir_all(dir.join("world")).unwrap();
        std::fs::write(dir.join("world/level.dat"), "before").unwrap();
        let install = update::Install { root };
        (Shared::with_install(&dir, Some(install)), dir)
    }

    #[cfg(unix)]
    #[test]
    fn a_switch_backs_up_then_rolls_back_world_and_release() {
        let (shared, dir) = installed("rollback", &[("v1", 0), ("v2", 0)]);
        let install = shared.install.clone().unwrap();
        let config = Config::default();
        let watch = switch(&shared, &dir, &config, "v2", Duration::from_secs(60)).unwrap();
        assert_eq!((watch.from.as_str(), watch.to.as_str()), ("v1", "v2"));
        assert!(watch.backup.ends_with("-before-v2.zip"), "{}", watch.backup);
        assert!(dir.join(backup::DIR).join(&watch.backup).is_file());
        assert_eq!(install.current().as_deref(), Some("v2"));
        assert_eq!(
            shared.update.lock().unwrap().busy.as_deref(),
            Some("Starting the game on v2")
        );
        // The new game wrote to the world, then never came up.
        std::fs::write(dir.join("world/level.dat"), "after").unwrap();
        roll_back(
            &shared,
            &dir,
            &config,
            &watch,
            "the game stopped (exit status: 1)",
        );
        assert_eq!(install.current().as_deref(), Some("v1"));
        // `rollback` won't go to the release that failed.
        assert_eq!(install.previous(), None);
        assert_eq!(
            std::fs::read_to_string(dir.join("world/level.dat")).unwrap(),
            "before"
        );
        let update = shared.update.lock().unwrap();
        assert!(!update.ok && update.busy.is_none() && update.finished == 1);
        let last = update.last.as_deref().unwrap();
        assert!(
            last.starts_with("Rolled back to v1: v2 didn't come up (the game stopped (exit status: 1)); the world is back"),
            "{last}"
        );
        // The world the new game left is kept.
        let kept = last.rsplit_once("is in ").unwrap().1.trim_end_matches(')');
        assert_eq!(
            std::fs::read_to_string(Path::new(kept).join("level.dat")).unwrap(),
            "after"
        );
        std::fs::remove_dir_all(&install.root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_game_that_comes_up_ends_the_update_and_prunes_old_releases() {
        let (shared, dir) = installed("prune", &[("v1", 0), ("v2", 0), ("v3", 0)]);
        let install = shared.install.clone().unwrap();
        let mut config = Config::default();
        config.updates.keep = 0;
        let watch = switch(&shared, &dir, &config, "v3", Duration::from_secs(60)).unwrap();
        came_up(&shared, &watch);
        assert_eq!(install.current().as_deref(), Some("v3"));
        // The release before stays for `rollback`; the other one goes.
        assert_eq!(install.previous().as_deref(), Some("v1"));
        assert!(!install.release("v2").exists());
        let update = shared.update.lock().unwrap();
        assert!(update.ok && update.busy.is_none() && update.finished == 1);
        assert_eq!(
            update.last.as_deref(),
            Some("Now on v3 (was v1): the game is up. Deleted old releases: v2")
        );
        drop(update);
        std::fs::remove_dir_all(&install.root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_game_that_cant_load_is_never_switched_to() {
        let (shared, dir) = installed("unloadable", &[("v1", 0), ("v2", 127)]);
        let install = shared.install.clone().unwrap();
        assert!(
            switch(
                &shared,
                &dir,
                &Config::default(),
                "v2",
                Duration::from_secs(60)
            )
            .is_none()
        );
        assert_eq!(install.current().as_deref(), Some("v1"));
        let update = shared.update.lock().unwrap();
        assert!(!update.ok);
        assert!(
            update
                .last
                .as_deref()
                .unwrap()
                .starts_with("Stayed on v1: the game (")
        );
        std::fs::remove_dir_all(&install.root).unwrap();
    }

    #[test]
    fn up_means_done_and_a_status_reply_from_this_game() {
        let shared = Shared::with_install(Path::new("."), None);
        let set = |phase, pid: Option<u32>, sample: serde_json::Value| {
            let mut state = shared.state.lock().unwrap();
            (state.phase, state.engine_pid, state.sample) = (phase, pid, sample);
        };
        set(
            Phase::Starting,
            Some(7),
            json!({ "pid": 7, "game": { "tps": 20.0 } }),
        );
        assert!(!up(&shared));
        set(Phase::Running, Some(7), json!({ "pid": 7, "game": null }));
        assert!(!up(&shared));
        // A sample of the game before the restart doesn't count.
        set(
            Phase::Running,
            Some(8),
            json!({ "pid": 7, "game": { "tps": 20.0 } }),
        );
        assert!(!up(&shared));
        set(
            Phase::Running,
            Some(8),
            json!({ "pid": 8, "game": { "tps": 20.0 } }),
        );
        assert!(up(&shared));
    }

    #[test]
    fn one_supervisor_per_folder() {
        let dir = std::env::temp_dir().join(format!("horizon-server-lock-{}", std::process::id()));
        let first = lock(&dir).unwrap();
        assert!(lock(&dir).unwrap_err().contains("already running"));
        drop(first);
        retried(|| lock(&dir)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
