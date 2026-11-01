//! The Launch API's promises about how `horizon-client` ends (docs/api/launch.md), checked on
//! the real program: `--launch-api` prints JSON and exits 0, a refused command line exits 2
//! without a crash report, and an error exits 1 with the report's path as the last stderr line,
//! with `--home` and `--game-dir` taken relative to where the game was started.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch folder of its own, with a space in its name as launchers' instance folders have.
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("horizon-launch-api {name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The game started in `cwd` with `args`, its homes kept inside `cwd`.
fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_horizon-client"))
        .args(args)
        .current_dir(cwd)
        .env("HORIZON_HOME", cwd.join("env home"))
        .env("XDG_DATA_HOME", cwd.join("xdg data"))
        .env("XDG_CONFIG_HOME", cwd.join("xdg config"))
        .env("HORIZON_NO_AUDIO", "1")
        .env_remove("HORIZON_SESSION_FILE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn last_stderr_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn launch_api_prints_json_and_runs_nothing_else() {
    let dir = scratch("reply");
    let output = run(&dir, &["--launch-api"]);
    assert_eq!(output.status.code(), Some(0));
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reply["launch_api"].is_string(), "{reply}");
    assert_eq!(reply["minecraft"], horizon_core::VANILLA_VERSION);
    let names: Vec<_> = reply["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    for name in ["--resources", "--game-dir", "--home", "--session-stdin"] {
        assert!(names.contains(&name), "{name} not in {names:?}");
    }
    assert!(!dir.join("env home").exists(), "--launch-api opened a log");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_refused_command_line_exits_2_without_a_crash_report() {
    let dir = scratch("usage");
    for args in [
        &["--home", "home", "--bogus"][..],
        &["--home", "home", "--width", "wide"],
        &["--home", "home", "--game-dir"],
    ] {
        let output = run(&dir, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            !last_stderr_line(&output).starts_with("Horizon: crash report saved to "),
            "{args:?}"
        );
        assert!(!dir.join("home/crash-reports").exists(), "{args:?}");
    }
    assert!(!dir.join("saves").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_error_exits_1_and_names_the_crash_report_under_home() {
    let dir = scratch("error");
    let start = dir.join("start");
    std::fs::create_dir_all(&start).unwrap();
    // No prepared resources: the game stops on an error once its folders are set up.
    let output = run(
        &start,
        &[
            "--home",
            "../my home",
            "--game-dir",
            "../my game",
            "--resources",
            "../no resources",
            "--offscreen",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let line = last_stderr_line(&output);
    let report = line
        .strip_prefix("Horizon: crash report saved to ")
        .unwrap_or_else(|| panic!("last stderr line: {line}"));
    let report = Path::new(report);
    assert!(report.is_file(), "{}", report.display());
    assert_eq!(
        std::fs::canonicalize(report.parent().unwrap()).unwrap(),
        std::fs::canonicalize(dir.join("my home/crash-reports")).unwrap()
    );
    assert!(dir.join("my game").is_dir(), "--game-dir was not created");
    assert!(!start.join("saves").exists() && !start.join("options.txt").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
