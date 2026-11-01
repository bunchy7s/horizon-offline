//! `horizon-server service install|uninstall|print`: run the server at boot or login under the system's own
//! service manager (docs/server.md, section 3): a systemd unit on Linux, a launchd agent on macOS, a task at
//! logon on Windows. The service runs `horizon-server run`, which restarts the game if it crashes; the
//! service manager restarts the supervisor itself.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// The service's name for a server folder: `horizon-server-<folder>`, in letters, digits and dashes.
pub fn name(dir: &Path) -> String {
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut clean: String = folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    clean = clean.trim_matches('-').to_owned();
    if clean.is_empty() {
        clean = "server".into();
    }
    format!("horizon-server-{clean}")
}

/// The program the service runs: the `horizon-server` on the PATH (the installer's launcher, which keeps
/// running the current version after an update), else this program, through the installer's `current`
/// link when it runs from an installed release.
fn program() -> Result<PathBuf, String> {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.join(format!("horizon-server{}", std::env::consts::EXE_SUFFIX)))
        .find(|p| p.is_file());
    match on_path {
        Some(path) => Ok(path),
        None => std::env::current_exe()
            .map(|exe| through_current(&exe))
            .map_err(|e| e.to_string()),
    }
}

/// `<dir>/releases/<tag>/horizon-server` as `<dir>/current/horizon-server` while `current` points at that
/// release (install.sh and install.ps1's layout), so an update applies at the next restart. Linux reports
/// the running program with links resolved, so the release's own path is what this program sees there.
fn through_current(exe: &Path) -> PathBuf {
    let current = (|| {
        let release = exe.parent()?;
        let releases = release.parent()?;
        if releases.file_name()? != "releases" {
            return None;
        }
        let current = releases.parent()?.join("current");
        let name = exe.file_name()?;
        let same = std::fs::canonicalize(&current).ok()? == std::fs::canonicalize(release).ok()?;
        same.then(|| current.join(name))
    })();
    current.unwrap_or_else(|| exe.to_path_buf())
}

/// A systemd unit: a user unit, or with `user` a system unit running as that user.
pub fn systemd_unit(program: &Path, dir: &Path, user: Option<&str>) -> String {
    let (run_as, wanted_by) = match user {
        Some(user) => (format!("User={user}\n"), "multi-user.target"),
        None => (String::new(), "default.target"),
    };
    format!(
        "[Unit]
Description=Horizon server in {dir}
Documentation=https://github.com/horizon-rust/horizon/blob/main/docs/server.md
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
{run_as}WorkingDirectory={dir}
ExecStart=\"{program}\" --dir \"{dir}\" run
# SIGTERM makes the supervisor stop the game cleanly; saving a big world takes a moment.
TimeoutStopSec=180
Restart=on-failure
RestartSec=10

[Install]
WantedBy={wanted_by}
",
        dir = dir.display(),
        program = program.display(),
    )
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A launchd agent: started at login, and again if the supervisor exits with an error.
pub fn launchd_plist(label: &str, program: &Path, dir: &Path) -> String {
    let log = dir.join("logs/supervisor.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{program}</string>
		<string>--dir</string>
		<string>{dir}</string>
		<string>run</string>
	</array>
	<key>WorkingDirectory</key>
	<string>{dir}</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ExitTimeOut</key>
	<integer>180</integer>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
        program = xml(&program.display().to_string()),
        dir = xml(&dir.display().to_string()),
        log = xml(&log.display().to_string()),
    )
}

/// The account the Windows task runs as and starts with: `DOMAIN\user`.
fn windows_user() -> Result<String, String> {
    let user = std::env::var("USERNAME").map_err(|_| "USERNAME is not set".to_string())?;
    Ok(match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    })
}

/// A Task Scheduler task that starts the supervisor when `user` logs on, as that user and without
/// elevation, and again a minute after it exits with an error. Unlike `schtasks /Create`'s defaults it
/// has no 72-hour time limit and also starts on battery power.
pub fn windows_task_xml(user: &str, program: &Path, dir: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Horizon server in {dir}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{program}</Command>
      <Arguments>--dir "{dir}" run</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml(user),
        program = xml(&program.display().to_string()),
        dir = xml(&dir.display().to_string()),
    )
}

/// `schtasks /XML` reads only UTF-16 files: little-endian, after a byte order mark.
fn utf16_with_bom(text: &str) -> Vec<u8> {
    std::iter::once(0xfeff)
        .chain(text.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn launchd_path(label: &str) -> Result<PathBuf, String> {
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

fn launchd_label(dir: &Path) -> String {
    format!(
        "dev.horizon.server.{}",
        name(dir).trim_start_matches("horizon-server-")
    )
}

fn systemd_path(service: &str, system: bool) -> Result<PathBuf, String> {
    Ok(if system {
        PathBuf::from("/etc/systemd/system")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map_or_else(
                || home().map(|h| h.join(".config")),
                |c| Ok(PathBuf::from(c)),
            )?
            .join("systemd/user")
    }
    .join(format!("{service}.service")))
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = crate::process::command(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}

/// A step that may fail harmlessly (stopping what isn't running): its output would only alarm.
fn quiet(program: &str, args: &[&str]) {
    let _ = crate::process::command(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// What `service print` shows and `service install` writes, and where.
fn definition(dir: &Path, system: bool) -> Result<(PathBuf, String), String> {
    let program = program()?;
    if cfg!(target_os = "macos") {
        if system {
            return Err("on macOS the server runs as a login agent: drop --system".into());
        }
        let label = launchd_label(dir);
        Ok((launchd_path(&label)?, launchd_plist(&label, &program, dir)))
    } else if cfg!(windows) {
        Err(
            "on Windows, `service install` creates a task at logon; there is no file to print"
                .into(),
        )
    } else {
        let user = system.then(|| std::env::var("USER").unwrap_or_else(|_| "root".into()));
        Ok((
            systemd_path(&name(dir), system)?,
            systemd_unit(&program, dir, user.as_deref()),
        ))
    }
}

pub fn command(dir: &Path, args: &[String]) -> Result<(), String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::config::Config::load(&dir)?;
    let system = args.iter().any(|a| a == "--system");
    let service = name(&dir);
    match args.first().map(String::as_str) {
        Some("print") => {
            let (path, text) = definition(&dir, system)?;
            println!("# {}\n{text}", path.display());
            Ok(())
        }
        Some("install") => install(&dir, &service, system),
        Some("uninstall") => uninstall(&dir, &service, system),
        _ => Err("use: service install|uninstall|print [--system]".into()),
    }
}

fn install(dir: &Path, service: &str, system: bool) -> Result<(), String> {
    if cfg!(windows) {
        // `/SC ONLOGON` makes a trigger for every user's logon, which only an administrator may
        // create ("Access is denied" otherwise). A task defined in XML can name the user instead.
        let task = windows_task_xml(&windows_user()?, &program()?, dir);
        let file = std::env::temp_dir().join(format!("{service}.xml"));
        std::fs::write(&file, utf16_with_bom(&task))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        let created = run(
            "schtasks",
            &[
                "/Create",
                "/TN",
                service,
                "/XML",
                &file.display().to_string(),
                "/F",
            ],
        );
        let _ = std::fs::remove_file(&file);
        created?;
        run("schtasks", &["/Run", "/TN", service])?;
        println!("Installed the task {service}: the server starts when you log in, and now.");
        return Ok(());
    }
    let (path, text) = definition(dir, system)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::create_dir_all(dir.join("logs")).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| {
        format!(
            "{}: {e}{}",
            path.display(),
            if system { " (--system needs sudo)" } else { "" }
        )
    })?;
    if cfg!(target_os = "macos") {
        let uid = String::from_utf8_lossy(
            &Command::new("id")
                .arg("-u")
                .output()
                .map_err(|e| e.to_string())?
                .stdout,
        )
        .trim()
        .to_owned();
        quiet(
            "launchctl",
            &[
                "bootout",
                &format!("gui/{uid}"),
                &path.display().to_string(),
            ],
        );
        run(
            "launchctl",
            &[
                "bootstrap",
                &format!("gui/{uid}"),
                &path.display().to_string(),
            ],
        )?;
        println!(
            "Installed {}: the server starts at login, and now. Its output: logs/supervisor.log",
            path.display()
        );
    } else {
        let scope: &[&str] = if system { &[] } else { &["--user"] };
        run("systemctl", &[scope, &["daemon-reload"]].concat())?;
        run(
            "systemctl",
            &[scope, &["enable", "--now", service]].concat(),
        )?;
        println!(
            "Installed {}: the server starts at boot, and now.",
            path.display()
        );
        if !system {
            println!(
                "To keep it running while you are logged out: loginctl enable-linger $USER. \
                 Its log: journalctl --user -u {service}"
            );
        }
    }
    Ok(())
}

fn uninstall(dir: &Path, service: &str, system: bool) -> Result<(), String> {
    if cfg!(windows) {
        quiet("schtasks", &["/End", "/TN", service]);
        run("schtasks", &["/Delete", "/TN", service, "/F"])?;
        println!("Removed the task {service}.");
        return Ok(());
    }
    let (path, _) = definition(dir, system)?;
    if cfg!(target_os = "macos") {
        let uid = String::from_utf8_lossy(
            &Command::new("id")
                .arg("-u")
                .output()
                .map_err(|e| e.to_string())?
                .stdout,
        )
        .trim()
        .to_owned();
        quiet(
            "launchctl",
            &[
                "bootout",
                &format!("gui/{uid}"),
                &path.display().to_string(),
            ],
        );
    } else {
        let scope: &[&str] = if system { &[] } else { &["--user"] };
        quiet(
            "systemctl",
            &[scope, &["disable", "--now", service]].concat(),
        );
    }
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !cfg!(target_os = "macos") {
        let scope: &[&str] = if system { &[] } else { &["--user"] };
        quiet("systemctl", &[scope, &["daemon-reload"]].concat());
    }
    println!(
        "Removed {}; the server's folder is untouched.",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_folder() {
        assert_eq!(
            name(Path::new("/srv/My Server!")),
            "horizon-server-my-server"
        );
        assert_eq!(name(Path::new("/")), "horizon-server-server");
    }

    #[test]
    fn units_run_the_supervisor() {
        let unit = systemd_unit(
            Path::new("/home/a/.local/bin/horizon-server"),
            Path::new("/srv/mc"),
            Some("mc"),
        );
        assert!(
            unit.contains("ExecStart=\"/home/a/.local/bin/horizon-server\" --dir \"/srv/mc\" run")
        );
        assert!(unit.contains("User=mc\n"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        let dir = Path::new("/srv/R&D");
        let plist = launchd_plist(
            "dev.horizon.server.mc",
            Path::new("/opt/h/horizon-server"),
            dir,
        );
        assert!(plist.contains("<string>/srv/R&amp;D</string>"));
        // The log is `dir.join("logs/supervisor.log")`, which a `Path` prints with the host's own
        // separator where it adds one (a `\` on Windows), and the `&` of the folder's name is escaped.
        let log = dir
            .join("logs/supervisor.log")
            .display()
            .to_string()
            .replace('&', "&amp;");
        assert!(plist.contains(&format!("<string>{log}</string>")));
        // Where the separator is `/`, the path is the one a launchd agent has on macOS.
        #[cfg(unix)]
        assert!(plist.contains("<string>/srv/R&amp;D/logs/supervisor.log</string>"));
    }

    #[test]
    fn windows_tasks_start_at_the_users_own_logon() {
        let task = windows_task_xml(
            r"PC\ann",
            Path::new(r"C:\Users\ann\horizon-server.exe"),
            Path::new(r"C:\Servers\R&D"),
        );
        // A logon trigger without a user is every user's, which needs an administrator.
        assert!(task.contains(
            "<LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>PC\\ann</UserId>"
        ));
        assert!(task.contains("<UserId>PC\\ann</UserId>\n      <LogonType>InteractiveToken"));
        assert!(task.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(task.contains(r"<Command>C:\Users\ann\horizon-server.exe</Command>"));
        assert!(task.contains(r#"<Arguments>--dir "C:\Servers\R&amp;D" run</Arguments>"#));
        assert!(task.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert_eq!(
            utf16_with_bom("<a>"),
            [0xff, 0xfe, b'<', 0, b'a', 0, b'>', 0]
        );
    }

    /// Creates and deletes a real task as this user, unelevated, as the launcher's toggle does.
    /// By hand on Windows: `cargo test -p horizon-server -- --ignored windows_task`.
    #[cfg(windows)]
    #[test]
    #[ignore = "creates a scheduled task on this computer"]
    fn windows_task_installs_without_elevation() {
        let name = format!("horizon-server-test-{}", std::process::id());
        let task = windows_task_xml(
            &windows_user().unwrap(),
            Path::new(r"C:\Windows\System32\cmd.exe"),
            &std::env::temp_dir(),
        );
        let file = std::env::temp_dir().join(format!("{name}.xml"));
        std::fs::write(&file, utf16_with_bom(&task)).unwrap();
        let created = run(
            "schtasks",
            &[
                "/Create",
                "/TN",
                &name,
                "/XML",
                &file.display().to_string(),
                "/F",
            ],
        );
        std::fs::remove_file(&file).unwrap();
        created.unwrap();
        run("schtasks", &["/Delete", "/TN", &name, "/F"]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_release_runs_through_current() {
        let root =
            std::env::temp_dir().join(format!("horizon-service-current-{}", std::process::id()));
        let release = root.join("releases/v1");
        std::fs::create_dir_all(&release).unwrap();
        let exe = release.join("horizon-server");
        std::fs::write(&exe, "").unwrap();
        // Not the release in use (or no installer layout): this program as it is.
        assert_eq!(through_current(&exe), exe);
        std::os::unix::fs::symlink(&release, root.join("current")).unwrap();
        assert_eq!(through_current(&exe), root.join("current/horizon-server"));
        let other = root.join("bin/horizon-server");
        assert_eq!(through_current(&other), other);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
