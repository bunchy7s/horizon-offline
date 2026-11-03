//! What runs around the game for one instance (`instances::Meta::launch`): a command run before
//! it, a wrapper it runs through, environment variables, and a command run after it exits
//! ([`after_exit`], started by the launcher once it sees the game end). All empty by default.
//! The game and the commands also get the instance's own variables ([`instance_vars`]).
//!
//! [`wrap`] turns the game's command into one process that does all three, so the launcher still
//! follows one program (its output, its exit, Stop):
//!
//! - Unix: `/bin/sh -c '(<pre-launch>) || exit; exec "$@"' horizon <wrapper…> <game> <args…>`.
//!   The pre-launch command is the player's own shell line, run in the instance's folder; the
//!   game's words go through `"$@"`, so they are never parsed by the shell. `exec` makes the
//!   shell become the wrapper (or the game).
//! - Windows: `cmd /D /S /C "<pre-launch> && <wrapper…> <game> <args…>"`, each word quoted.
//!
//! Without a pre-launch command the wrapper is the program itself, and no shell runs.
use crate::backend::split_args;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// A shell command run in the instance's folder before the game; the game starts only if it
    /// succeeds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pre_launch: String,
    /// A command line the game runs through, such as `nice -n 5` (quotes group a word).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wrapper: String,
    /// A shell command run in the instance's folder after the game exits, whatever its exit.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub post_exit: String,
    /// Environment variables for the game (and the two commands above).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// The most RAM Horizon may hold that the player chose, in MiB; none (the default): no
    /// limit, so big worlds and modpacks keep their render distance. Read it with
    /// [`Launch::memory_limit`]. A Java instance has `-Xmx` instead (`instances::Java::memory_mb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u32>,
    /// Linux: run the game through Feral's GameMode (`gamemoderun`), when it is installed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub gamemode: bool,
    /// Linux: MangoHud's overlay (`mangohud`), when it is installed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mangohud: bool,
    /// Linux: the dedicated graphics card of a computer with two (PRIME render offload).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dedicated_gpu: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Tells the game its limit (`HORIZON_MEMORY_LIMIT_MB`). Horizon's only: a Java instance has `-Xmx`.
/// Before [`wrap`], so the player's own variable wins.
pub fn limit_memory(cmd: &mut Command, launch: &Launch) {
    if let Some(mb) = launch.memory_limit() {
        cmd.env("HORIZON_MEMORY_LIMIT_MB", mb.to_string());
    }
}

/// The memory slider's stops, in MiB.
const SLIDER_STEP: u32 = 64;
/// The slider's top when the computer's memory is not known.
const UNKNOWN_TOTAL_MB: u32 = 64 * 1024;

/// The value at `fraction` of a slider over `total_mb`: none at its very end (no limit), and at
/// the start the smallest value there is, as nothing keeps a player from going that low.
pub fn memory_at(fraction: f32, total_mb: u32) -> Option<u32> {
    if fraction >= 1. {
        return None;
    }
    let mb = fraction.max(0.) * total_mb as f32;
    Some(((mb / SLIDER_STEP as f32).round() as u32 * SLIDER_STEP).max(1))
}

/// What [`parse_memory`] says of text that isn't a memory limit.
#[derive(Debug, PartialEq, Eq)]
pub struct NotMemory;

/// A memory limit as a player types it: a number of MB, or of GB with `gb`, and nothing or `off`
/// for no limit. Nothing keeps a player from going low. Err when it is none of those.
pub fn parse_memory(typed: &str) -> Result<Option<u32>, NotMemory> {
    let typed = typed.trim().to_ascii_lowercase();
    if typed.is_empty() || typed == "off" {
        return Ok(None);
    }
    let (number, scale) = if let Some(gb) = typed.strip_suffix("gb").or(typed.strip_suffix('g')) {
        (gb, 1024)
    } else {
        let mb = typed.strip_suffix("mb").or(typed.strip_suffix('m'));
        (mb.unwrap_or(&typed), 1)
    };
    let number: u32 = number.trim().parse().map_err(|_| NotMemory)?;
    number.checked_mul(scale).map(Some).ok_or(NotMemory)
}

/// Where `mb` sits on a slider over `total_mb`; [`memory_at`]'s inverse.
pub fn memory_fraction(mb: Option<u32>, total_mb: u32) -> f32 {
    mb.map_or(1., |mb| (mb as f32 / total_mb.max(1) as f32).min(1.))
}

/// This computer's memory in MiB, the top of the slider.
pub fn total_memory_mb() -> u32 {
    system_memory_bytes()
        .map(|bytes| (bytes >> 20) as u32)
        .filter(|mb| *mb > 0)
        .unwrap_or(UNKNOWN_TOTAL_MB)
}

#[cfg(windows)]
fn system_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: all-zero is a valid value of this plain C struct, its length is set as the call
    // requires, and the call only writes into it.
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (GlobalMemoryStatusEx(&mut status) != 0).then_some(status.ullTotalPhys)
    }
}

#[cfg(target_os = "macos")]
fn system_memory_bytes() -> Option<u64> {
    let out = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_memory_bytes() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// What this Linux computer has for the three switches. Elsewhere, nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub gamemode: bool,
    pub mangohud: bool,
    /// More than one graphics card.
    pub two_gpus: bool,
    /// NVIDIA's own driver is loaded: offload asks for it, not Mesa.
    pub nvidia: bool,
}

/// `HORIZON_LAUNCHER_LINUX_TOOLS=gamemode,mangohud,gpu,nvidia`: what [`Found::here`] says is
/// installed, on any system (for tests and screenshots).
pub const TOOLS_ENV: &str = "HORIZON_LAUNCHER_LINUX_TOOLS";

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            let file = dir.join(program);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            file.is_file()
        })
    })
}

impl Found {
    /// What is installed here: on Linux, `gamemoderun` and `mangohud` on the PATH, the cards
    /// under `/sys/class/drm`, and `/proc/driver/nvidia`.
    pub fn here() -> Found {
        if let Some(list) = std::env::var_os(TOOLS_ENV) {
            let list = list.to_string_lossy();
            let has = |n: &str| list.split(',').any(|w| w.trim() == n);
            return Found {
                gamemode: has("gamemode"),
                mangohud: has("mangohud"),
                two_gpus: has("gpu"),
                nvidia: has("nvidia"),
            };
        }
        if !cfg!(target_os = "linux") {
            return Found::default();
        }
        let cards = std::fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.strip_prefix("card")
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            })
            .count();
        Found {
            gamemode: on_path("gamemoderun"),
            mangohud: on_path("mangohud"),
            two_gpus: cards >= 2,
            nvidia: std::path::Path::new("/proc/driver/nvidia/gpus").is_dir(),
        }
    }

    /// Whether any switch can be shown.
    pub fn any(&self) -> bool {
        self.gamemode || self.mangohud || self.two_gpus
    }
}

/// The programs the switches put before the game (and the player's wrapper), outermost first.
fn switch_programs(launch: &Launch, found: &Found) -> Vec<OsString> {
    let mut out = Vec::new();
    if launch.gamemode && found.gamemode {
        out.push("gamemoderun".into());
    }
    if launch.mangohud && found.mangohud {
        out.push("mangohud".into());
    }
    out
}

/// The variables the dedicated GPU switch sets: NVIDIA's offload variables with its driver,
/// else Mesa's `DRI_PRIME`. The player's own variables come after and win.
pub fn switch_env(launch: &Launch, found: &Found) -> Vec<(&'static str, &'static str)> {
    match (launch.dedicated_gpu && found.two_gpus, found.nvidia) {
        (false, _) => vec![],
        (true, true) => vec![
            ("__NV_PRIME_RENDER_OFFLOAD", "1"),
            ("__VK_LAYER_NV_optimus", "NVIDIA_only"),
            ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
        ],
        (true, false) => vec![("DRI_PRIME", "1")],
    }
}

impl Launch {
    /// The most RAM Horizon may hold, in MiB; none: no limit.
    pub fn memory_limit(&self) -> Option<u32> {
        self.memory_limit_mb
    }

    /// Keeps the player's choice; none: no limit.
    pub fn set_memory_limit(&mut self, limit: Option<u32>) {
        self.memory_limit_mb = limit;
    }

    pub fn is_empty(&self) -> bool {
        self == &Launch::default()
    }

    /// What it does, in one line, or None when nothing.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.pre_launch.trim().is_empty() {
            parts.push(crate::i18n::trf(
                "runs {command} first",
                &[("command", self.pre_launch.trim())],
            ));
        }
        if !self.wrapper.trim().is_empty() {
            parts.push(crate::i18n::trf(
                "through {wrapper}",
                &[("wrapper", self.wrapper.trim())],
            ));
        }
        if !self.post_exit.trim().is_empty() {
            parts.push(crate::i18n::trf(
                "runs {command} after",
                &[("command", self.post_exit.trim())],
            ));
        }
        if let Some(mb) = self.memory_limit_mb {
            parts.push(crate::i18n::trf(
                "limited to {mb} MB",
                &[("mb", &mb.to_string())],
            ));
        }
        if self.gamemode {
            parts.push(crate::i18n::tr("with GameMode").to_string());
        }
        if self.mangohud {
            parts.push(crate::i18n::tr("with MangoHud").to_string());
        }
        if self.dedicated_gpu {
            parts.push(crate::i18n::tr("on the dedicated GPU").to_string());
        }
        match self.env.len() {
            0 => {}
            1 => parts.push(crate::i18n::tr("with 1 variable").to_string()),
            n => parts.push(crate::i18n::trf(
                "with {count} variables",
                &[("count", &n.to_string())],
            )),
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `KEY=value KEY2="two words"` (one line, or one per line) as variables.
pub fn parse_env(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for word in split_args(text) {
        let (key, value) = word
            .split_once('=')
            .ok_or_else(|| format!("{word}: write it as NAME=value"))?;
        if !valid_key(key) {
            return Err(format!(
                "{key}: a variable's name is letters, digits and _, not starting with a digit"
            ));
        }
        env.insert(key.to_string(), value.to_string());
    }
    Ok(env)
}

/// The variables as [`parse_env`] reads them back.
pub fn env_line(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(
            |(k, v)| match v.is_empty() || v.contains([' ', '"', '\'', '\\']) {
                true => format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")),
                false => format!("{k}={v}"),
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

/// The instance's variables for the game and the commands around it, named as Prism Launcher
/// names them so its scripts carry over (<https://prismlauncher.org/wiki/help-pages/custom-commands/>):
/// `INST_NAME`, `INST_ID`, `INST_DIR` and `INST_MC_DIR` (both the instance's game folder), and
/// for a Java instance `INST_JAVA`, the `java` it runs on.
pub fn instance_vars(name: &str, id: &str, dir: &std::path::Path, java: Option<&OsStr>) -> Vars {
    let mut vars = vec![
        ("INST_NAME", OsString::from(name)),
        ("INST_ID", OsString::from(id)),
        ("INST_DIR", dir.as_os_str().to_owned()),
        ("INST_MC_DIR", dir.as_os_str().to_owned()),
    ];
    if let Some(java) = java {
        vars.push(("INST_JAVA", java.to_owned()));
    }
    vars
}

/// An instance's variables ([`instance_vars`]).
pub type Vars = Vec<(&'static str, OsString)>;

/// The post-exit command, ready to run in `dir` with the instance's `vars` and variables; none
/// when there is none. `/bin/sh -c`, or `cmd /D /S /C` on Windows.
pub fn after_exit(
    launch: &Launch,
    dir: &std::path::Path,
    vars: &[(&'static str, OsString)],
) -> Option<Command> {
    let line = launch.post_exit.trim();
    if line.is_empty() {
        return None;
    }
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.raw_arg(format!("/D /S /C \"{line}\""));
        }
        #[cfg(not(windows))]
        c.args(["/D", "/S", "/C", line]);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(line);
        c
    };
    cmd.current_dir(dir)
        .envs(vars.iter().map(|(k, v)| (k, v)))
        .envs(&launch.env);
    Some(cmd)
}

/// Runs the post-exit command, if any, on its own thread: nothing waits for it, its output is
/// dropped.
pub fn run_after_exit(launch: &Launch, dir: &std::path::Path, vars: &[(&'static str, OsString)]) {
    let Some(mut cmd) = after_exit(launch, dir, vars) else {
        return;
    };
    crate::process::hide_console(&mut cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    std::thread::spawn(move || {
        if let Err(e) = cmd.status() {
            eprintln!("The post-exit command didn't run: {e}");
        }
    });
}

/// The Unix script: the pre-launch command (in a subshell, so its `exit` doesn't end the script
/// before the message), then the rest of the arguments as the program.
fn script(pre_launch: &str) -> String {
    format!(
        "(\n{pre_launch}\n) || {{ s=$?; echo \"The pre-launch command failed (exit $s), so the \
         game did not start.\" >&2; exit $s; }}\nexec \"$@\""
    )
}

/// A word quoted for a Windows command line, as programs built with the C runtime read it back.
fn windows_quote(word: &OsStr) -> String {
    let word = word.to_string_lossy();
    if !word.is_empty() && !word.contains([' ', '\t', '"']) {
        return word.into_owned();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in word.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// How [`wrap`] runs a command on each system, apart from the variables and the folder: the
/// program and its arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Run as given (`words[0]` is the program).
    Direct(Vec<OsString>),
    /// `/bin/sh -c <script> horizon <words…>`.
    Shell {
        script: String,
        words: Vec<OsString>,
    },
    /// `cmd /D /S /C "<line>"`: the line as cmd reads it.
    Cmd { line: String },
}

/// The plan for `words` (the game's program and arguments) with `launch`, on Windows or not,
/// with what the computer has for the switches.
pub fn plan(launch: &Launch, words: Vec<OsString>, windows: bool, found: &Found) -> Plan {
    let mut all = switch_programs(launch, found);
    all.extend(split_args(&launch.wrapper).into_iter().map(OsString::from));
    all.extend(words);
    let pre = launch.pre_launch.trim();
    match (pre.is_empty(), windows) {
        (true, _) => Plan::Direct(all),
        (false, false) => Plan::Shell {
            script: script(pre),
            words: all,
        },
        (false, true) => Plan::Cmd {
            line: format!(
                "{pre} && {}",
                all.iter()
                    .map(|w| windows_quote(w))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        },
    }
}

/// The game's command run with `launch`: a new command with the same folder and variables,
/// plus the instance's variables. Call it last, after every argument is added.
pub fn wrap(cmd: Command, launch: &Launch) -> Command {
    if launch.is_empty() {
        return cmd;
    }
    let words: Vec<OsString> = std::iter::once(cmd.get_program().to_owned())
        .chain(cmd.get_args().map(OsStr::to_owned))
        .collect();
    // The override only shows the switches: they act on Linux.
    let found = match cfg!(target_os = "linux") {
        true => Found::here(),
        false => Found::default(),
    };
    let mut out = match plan(launch, words, cfg!(windows), &found) {
        Plan::Direct(words) => {
            let mut c = Command::new(&words[0]);
            c.args(&words[1..]);
            c
        }
        Plan::Shell { script, words } => {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(script).arg("horizon").args(words);
            c
        }
        Plan::Cmd { line } => {
            let mut c = Command::new("cmd.exe");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.raw_arg(format!("/D /S /C \"{line}\""));
            }
            #[cfg(not(windows))]
            c.args(["/D", "/S", "/C", &line]);
            c
        }
    };
    if let Some(dir) = cmd.get_current_dir() {
        out.current_dir(dir);
    }
    for (key, value) in cmd.get_envs() {
        match value {
            Some(value) => out.env(key, value),
            None => out.env_remove(key),
        };
    }
    out.envs(switch_env(launch, &found));
    out.envs(&launch.env);
    out
}

#[cfg(test)]
#[path = "launch/tests.rs"]
mod tests;
