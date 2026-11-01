//! Java discovery and execution for the vanilla exporters. They call Mojang's own code, so they
//! stay Java; everything around them is here.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Jdk {
    pub javac: PathBuf,
    pub java: PathBuf,
}

const TIMEOUT: Duration = Duration::from_secs(3600);

/// Options for every JVM preparing starts (the exporters, the data generator, javac with `-J`).
/// They are short batch jobs that run side by side: the C2 compiler's background work costs
/// more CPU than it wins back, and the serial collector keeps their heaps small. Measured on the
/// pinned jars, a fresh preparation took 105 s with the defaults and 63-66 s with these, same
/// output; the serial collector was as fast as the parallel one and its JVMs together took
/// 1.25 GB instead of 2 GB.
pub(crate) const BATCH_OPTIONS: [&str; 2] = ["-XX:TieredStopAtLevel=1", "-XX:+UseSerialGC"];

/// Exporters and JDK probes write to pipes or log files: on Windows they open no terminal.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };
    command
}

fn executable(dir: &Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    })
}

/// `fs::canonicalize`, without the `\\?\` prefix it gives a Windows drive path: the
/// home goes into `JAVA_HOME` and the PATH of preparers, whose lookups don't take it.
fn real_path(path: PathBuf) -> PathBuf {
    let real = fs::canonicalize(&path).unwrap_or(path);
    if cfg!(windows) {
        without_verbatim(real)
    } else {
        real
    }
}

fn without_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Resolve compiler symlinks before picking its sibling runtime. The first `java` on the
/// PATH may be an older, runtime-only install; it must not be paired with a newer `javac`.
fn path_homes(path: &OsStr, tool: &str) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| executable(&dir, tool))
        .filter(|javac| javac.is_file())
        .map(real_path)
        .filter_map(|javac| {
            let bin = javac.parent()?;
            // macOS's system tools are dispatchers, not a JDK home: JAVA_HOME=/usr
            // makes them call themselves. /usr/libexec/java_home finds the actual JDK.
            if bin.file_name()? != "bin"
                || (cfg!(target_os = "macos") && bin == Path::new("/usr/bin"))
            {
                return None;
            }
            Some(bin.parent()?.to_path_buf())
        })
        .collect()
}

/// The major version `javac -version` or `java -version` reports, including Java 8's `1.8`.
fn tool_major(program: &Path, name: &str) -> Result<u32, String> {
    let output = command(program)
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    // macOS ships /usr/bin/java and /usr/bin/javac as placeholders.
    if text.contains("Unable to locate a Java Runtime") {
        return Err(format!(
            "{} is macOS's placeholder (no Java installed)",
            program.display()
        ));
    }
    if !output.status.success() {
        return Err(format!(
            "{} -version failed ({}): {}",
            program.display(),
            output.status,
            text.trim()
        ));
    }
    let marker = if name == "javac" { "javac" } else { "version" };
    let version = text
        .split_whitespace()
        .skip_while(|w| *w != marker)
        .nth(1)
        .ok_or_else(|| {
            format!(
                "{} -version printed no version: {}",
                program.display(),
                text.trim()
            )
        })?;
    let mut parts = version.trim_matches('"').split(['.', '-', '+', '_']);
    let first: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Ok(if first == 1 {
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0)
    } else {
        first
    })
}

/// The folders JDK installers and tools put JDKs in, each JDK one folder below (#666: a
/// player installed Java 25 but JAVA_HOME and the PATH still named an older one).
fn jdk_parents() -> Vec<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let mut parents = Vec::new();
    if cfg!(windows) {
        let vendors = [
            "Eclipse Adoptium",
            "Java",
            "Microsoft",
            "Zulu",
            "BellSoft",
            "Amazon Corretto",
            "Semeru",
            "OpenJDK",
        ];
        for base in ["ProgramFiles", "ProgramW6432"].into_iter().filter_map(var) {
            parents.extend(vendors.iter().map(|v| base.join(v)));
        }
    } else if cfg!(target_os = "macos") {
        parents.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        parents.extend(
            home.iter()
                .map(|h| h.join("Library/Java/JavaVirtualMachines")),
        );
    } else {
        parents.extend(["/usr/lib/jvm", "/usr/java", "/usr/local/lib/jvm"].map(PathBuf::from));
    }
    parents.extend(
        home.iter()
            .flat_map(|h| [h.join(".jdks"), h.join(".sdkman/candidates/java")]),
    );
    parents
}

/// The JDKs (folders with `bin/javac`) one level below `parents`; a macOS bundle's home is
/// its `Contents/Home`. Those named for `major` come first, then the others newest name first.
fn installed_jdks(parents: &[PathBuf], major: i64, tool: &str) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = parents
        .iter()
        .filter_map(|parent| fs::read_dir(parent).ok())
        .flatten()
        .flatten()
        .map(|entry| {
            let bundle = entry.path().join("Contents/Home");
            if bundle.is_dir() {
                bundle
            } else {
                entry.path()
            }
        })
        .filter(|home| executable(&home.join("bin"), tool).is_file())
        .collect();
    let named = |home: &Path| {
        home.iter().any(|part| {
            part.to_string_lossy()
                .split(|c: char| !c.is_ascii_digit())
                .any(|n| n == major.to_string())
        })
    };
    homes.sort_by(|a, b| named(b).cmp(&named(a)).then_with(|| b.cmp(a)));
    homes.dedup();
    homes
}

/// The Java runtimes the official launcher downloaded into `runtime` (a game directory's
/// `runtime` folder: `<component>/<platform>/<component>/`, a macOS one in `jre.bundle`).
pub fn minecraft_runtimes(runtime: &Path) -> Vec<PathBuf> {
    let children = |dir: &Path| -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        paths.sort();
        paths
    };
    let mut homes = Vec::new();
    for component in children(runtime) {
        for platform in children(&component) {
            let home = platform.join(component.file_name().unwrap_or_default());
            for home in [home.join("jre.bundle/Contents/Home"), home] {
                if executable(&home.join("bin"), "java").is_file() {
                    homes.push(home);
                    break;
                }
            }
        }
    }
    homes
}

/// Finds a JDK whose compiler and runtime are both at least `major`, the version
/// research-lock.json pins: `JAVA_HOME`, else JDKs on the PATH, else, on macOS, the JDK
/// `/usr/libexec/java_home -v <major>` reports, else the homes in `more` (the launcher's pick,
/// Minecraft's own runtimes), else the folders installers use (`jdk_parents`).
pub fn find(major: i64, more: &[PathBuf]) -> Result<Jdk, String> {
    find_tools(major, more, true)
}

/// Finds a compatible runtime; precompiled exporters do not need javac.
pub fn find_runtime(major: i64, more: &[PathBuf]) -> Result<Jdk, String> {
    find_tools(major, more, false)
}

fn find_tools(major: i64, more: &[PathBuf], compiler: bool) -> Result<Jdk, String> {
    let tool = if compiler { "javac" } else { "java" };
    let need = format!(
        "Java {major} is needed ({tool}, e.g. Eclipse Temurin {major}). \
         Install it, or set JAVA_HOME to it, and try again."
    );
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
        let bin = Path::new(&home).join("bin");
        candidates.push((
            executable(&bin, "javac"),
            executable(&bin, "java"),
            "JAVA_HOME",
        ));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for home in path_homes(&path, tool) {
            let bin = home.join("bin");
            candidates.push((executable(&bin, "javac"), executable(&bin, "java"), "PATH"));
        }
    }
    if cfg!(target_os = "macos") {
        if let Ok(out) = command("/usr/libexec/java_home")
            .args(["-v", &major.to_string()])
            .stderr(Stdio::null())
            .output()
        {
            let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !home.is_empty() {
                let bin = Path::new(&home).join("bin");
                candidates.push((
                    executable(&bin, "javac"),
                    executable(&bin, "java"),
                    "/usr/libexec/java_home",
                ));
            }
        }
    }
    for home in more {
        let bin = home.join("bin");
        candidates.push((
            executable(&bin, "javac"),
            executable(&bin, "java"),
            "Minecraft's Java",
        ));
    }
    for home in installed_jdks(&jdk_parents(), major, tool) {
        let bin = home.join("bin");
        candidates.push((
            executable(&bin, "javac"),
            executable(&bin, "java"),
            "an installed JDK",
        ));
    }
    let mut problems = Vec::new();
    for (javac, java, from) in candidates {
        if (compiler && !javac.is_file()) || !java.is_file() {
            problems.push(format!(
                "{from} ({}) has no compatible Java tools",
                javac.parent().unwrap().display()
            ));
            continue;
        }
        let checked = [(&javac, "javac"), (&java, "java")]
            .into_iter()
            .filter(|(_, name)| compiler || *name == "java")
            .try_for_each(|(program, name)| {
                let found = tool_major(program, name)?;
                if i64::from(found) >= major {
                    Ok(())
                } else {
                    Err(format!("{name} {found} ({})", program.display()))
                }
            });
        match checked {
            // Absolute: the exporters run in another working directory, where a relative
            // `JAVA_HOME`, `--java-home` or `PATH` entry would name nothing.
            Ok(()) => {
                return Ok(Jdk {
                    javac: std::path::absolute(&javac).unwrap_or(javac),
                    java: std::path::absolute(&java).unwrap_or(java),
                });
            }
            Err(e) => problems.push(format!("{from}: {e}")),
        }
    }
    if problems.is_empty() {
        Err(format!("{need} No {tool} was found."))
    } else {
        Err(format!("{need} Found: {}.", problems.join("; ")))
    }
}

/// A classpath with the platform's separator.
pub fn classpath<'a>(paths: impl IntoIterator<Item = &'a Path>) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    paths
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator)
}

impl Jdk {
    /// The JDK's home, the folder above `bin` (what `JAVA_HOME` names).
    pub fn home(&self) -> PathBuf {
        self.java
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// `javac --release <major> -proc:none` with Python's `subprocess.run(..., check=True,
    /// stdout=log, stderr=STDOUT, timeout=3600)`.
    pub fn javac(&self, major: i64, args: &[String], log: &fs::File) -> Result<(), String> {
        let mut command = command(&self.javac);
        command
            .args(BATCH_OPTIONS.map(|option| format!("-J{option}")))
            .args(["--release", &major.to_string(), "-proc:none"])
            .args(args);
        run(command, log)
    }

    pub fn java(&self, args: &[String], cwd: Option<&Path>, log: &fs::File) -> Result<(), String> {
        let mut command = command(&self.java);
        command.args(BATCH_OPTIONS).args(args);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        run(command, log)
    }
}

fn run(mut command: Command, log: &fs::File) -> Result<(), String> {
    let out = log.try_clone().map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let program = Path::new(command.get_program())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("Cannot start {program}: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{program} failed ({status})"))
            };
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} took over an hour and was stopped"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_drive_paths_lose_their_prefix() {
        let jdk = r"C:\Program Files\Eclipse Adoptium\jdk-25\bin\javac.exe";
        assert_eq!(
            without_verbatim(PathBuf::from(format!(r"\\?\{jdk}"))),
            PathBuf::from(jdk)
        );
        // A UNC share keeps its verbatim form: stripping it would leave no valid path.
        let unc = r"\\?\UNC\server\jdk\bin\javac.exe";
        assert_eq!(without_verbatim(PathBuf::from(unc)), PathBuf::from(unc));
        assert_eq!(
            without_verbatim(PathBuf::from("/usr/lib/jvm/bin/javac")),
            PathBuf::from("/usr/lib/jvm/bin/javac")
        );
    }

    #[test]
    fn installed_jdks_prefer_the_pinned_major() {
        let dir = std::env::temp_dir().join(format!("horizon-prepare-jdks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let parent = dir.join("Eclipse Adoptium");
        for jdk in [
            "jdk-21.0.5.11-hotspot",
            "jdk-25.0.1.8-hotspot",
            "jdk-26.0.0-hotspot",
            "jre-25.0.1",
        ] {
            let bin = parent.join(jdk).join("bin");
            fs::create_dir_all(&bin).unwrap();
            if jdk.starts_with("jdk") {
                fs::write(executable(&bin, "javac"), b"").unwrap();
            }
        }
        // A macOS bundle keeps its home in Contents/Home.
        let bundle = dir.join("JavaVirtualMachines/temurin-25.jdk/Contents/Home");
        fs::create_dir_all(bundle.join("bin")).unwrap();
        fs::write(executable(&bundle.join("bin"), "javac"), b"").unwrap();
        let found = installed_jdks(
            &[
                parent.clone(),
                dir.join("JavaVirtualMachines"),
                dir.join("missing"),
            ],
            25,
            "javac",
        );
        assert_eq!(
            found,
            [
                bundle,
                parent.join("jdk-25.0.1.8-hotspot"),
                parent.join("jdk-26.0.0-hotspot"),
                parent.join("jdk-21.0.5.11-hotspot"),
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn minecraft_runtimes_are_found_in_the_launchers_layout() {
        let dir =
            std::env::temp_dir().join(format!("horizon-prepare-runtimes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let make = |home: PathBuf, javac: bool| {
            fs::create_dir_all(home.join("bin")).unwrap();
            if javac {
                fs::write(executable(&home.join("bin"), "java"), b"").unwrap();
            }
            home
        };
        let gamma = make(
            dir.join("java-runtime-gamma/linux/java-runtime-gamma"),
            true,
        );
        let epsilon = make(
            dir.join(
                "java-runtime-epsilon/mac-os-arm64/java-runtime-epsilon/jre.bundle/Contents/Home",
            ),
            true,
        );
        make(dir.join("jre-legacy/linux/jre-legacy"), false);
        assert_eq!(minecraft_runtimes(&dir), [epsilon, gamma]);
        assert!(minecraft_runtimes(&dir.join("missing")).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
