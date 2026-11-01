//! `horizon-prepare`: prepares Horizon's resources from your own Minecraft Java Edition.
//! Its flags are the ones tools/prepare_native_client.py took.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// This release's version: release.yml builds it with `HORIZON_VERSION` from the tag
/// (docs/versioning.md); a build from source says Cargo.toml's.
const VERSION: &str = match option_env!("HORIZON_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

const HELP: &str = "\
horizon-prepare: prepare Horizon's resources from your own Minecraft Java Edition

Usage: horizon-prepare [--prepare] [--server-jar PATH] [--minecraft-dir PATH] [--root PATH] [--output PATH]
                       [--version-json PATH] [--client-jar PATH] [--libraries-dir PATH] [--assets-dir PATH]
                       [--java-home PATH] [--complete] [--progress json]
       horizon-prepare --check [--root PATH] [--output PATH]
       horizon-prepare compile-exporters [--root PATH]
       horizon-prepare sounds [--minecraft-dir PATH] [--version-json PATH] [--assets-dir PATH] [--root PATH] [--output PATH]
       horizon-prepare panorama [--minecraft-dir PATH] [--version-json PATH] [--assets-dir PATH] [--root PATH] [--output PATH]
       horizon-prepare fonts [--minecraft-dir PATH] [--version-json PATH] [--assets-dir PATH] [--root PATH] [--output PATH]
       horizon-prepare packs [--minecraft-dir PATH] [--version-json PATH] [--assets-dir PATH] [--root PATH] [--output PATH]
       horizon-prepare languages [--minecraft-dir PATH] [--version-json PATH] [--assets-dir PATH] [--root PATH] [--output PATH]

  --prepare             Import the pinned game files from your own copy first
  --server-jar PATH     The pinned server jar you downloaded yourself
                        (default: HORIZON_SERVER_JAR, or the copy imported before)
  --minecraft-dir PATH  The official launcher's game directory with your installed copy
                        (default: HORIZON_MINECRAFT_DIR, or the launcher's own)
  --version-json PATH   The pinned version's JSON, where --minecraft-dir has none
  --client-jar PATH     The pinned client jar, where --minecraft-dir has none
  --libraries-dir PATH  The folder of the client's libraries (Maven layout)
  --assets-dir PATH     The asset store, with indexes/ and objects/
                        (each of these four replaces what --minecraft-dir would give; an earlier
                        import beside --root that is still verified is reused instead)
  --java-home PATH      A Java runtime of the pinned version that your launcher manages, tried
                        after JAVA_HOME and the PATH, before the official launcher's runtimes
  --root PATH           The Horizon folder to prepare (default: the folder this program is
                        in, else the nearest folder above the current one with research-lock.json)
  --output PATH         Where the resources go (default: <root>/.local/horizon-client/<version>);
                        the exporters' scratch files stay in <root>/.local
  --check               Exit 0 when the resources are complete and current for this preparer,
                        else 1: each gap on stdout with what it needs, or an error: line on
                        stderr when it can't check. Runs no Java and reads no game files; it tests
                        which files are there and their versions, not their contents (on doubt,
                        prepare again)
  --complete            Also copy the sounds, then fail unless --check would pass: one command
                        for resources the game plays with nothing missing
  --progress json       One JSON object a line on stdout: {\"message\": ...} for each log line,
                        {\"fraction\": F} as the work advances (0 to 1), {\"error\": ...} on failure

Needs Minecraft played once from the official launcher, and Java of the version
research-lock.json pins. Releases use a runtime; development or changed exporters need a JDK.
compile-exporters builds and validates a release bundle from the already imported pinned jars.
The resources go to <root>/.local/horizon-client/<version>, or --output; pass that to
horizon-client --resources. Exit codes: 0 done, 1 failed (or --check found gaps), 2 a refused
command line (docs/api/launch.md).

horizon-prepare panorama repairs the classic menu background in existing resources without Java.
horizon-prepare fonts adds the GNU Unifont glyphs (characters the bitmap fonts lack) the same way.
horizon-prepare packs adds the built-in Programmer Art and High Contrast resource packs the same way.
horizon-prepare languages adds the bundled languages and the language list the JAR lacks the same way.
Full preparation also imports all four after extracting the client JAR's placeholders.

horizon-prepare sounds copies the pinned version's sounds (about 380 MB) from the official
launcher's asset store into the same folder, checking every file. It is optional: without it the
game is silent. Nothing is downloaded; it needs the version played once from the official launcher.
";

/// The folder with research-lock.json: beside the executable in a release folder, else the
/// current folder or one above it (a checkout, with the binary in target/).
fn default_root() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = beside.filter(|d| d.join("research-lock.json").is_file()) {
        return Some(dir);
    }
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .find(|d| d.join("research-lock.json").is_file())
        .map(Path::to_path_buf)
}

fn main() -> ExitCode {
    let mut options = horizon_prepare::Options {
        root: PathBuf::new(),
        import: false,
        minecraft_dir: None,
        server_jar: None,
        java_home: None,
        inputs: Default::default(),
        output: None,
    };
    let mut root = None;
    let mut check = false;
    let mut complete = false;
    let mut json = false;
    let mut args = std::env::args_os().skip(1).peekable();
    let assets = args.next_if(|a| {
        matches!(
            a.to_str(),
            Some("sounds" | "panorama" | "fonts" | "packs" | "languages" | "compile-exporters")
        )
    });
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        let (flag, inline) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (text.clone(), None),
        };
        let mut value = |name: &str| -> Result<PathBuf, String> {
            match &inline {
                Some(v) => Ok(PathBuf::from(v)),
                None => args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{name} needs a path")),
            }
        };
        let parsed = match flag.as_str() {
            "--prepare" | "--server-jar" | "--check" if assets.is_some() => {
                Err(format!("{flag} is for preparing, not for asset copying"))
            }
            "--prepare" if inline.is_none() => {
                options.import = true;
                Ok(())
            }
            "--server-jar" => value("--server-jar").map(|v| options.server_jar = Some(v)),
            "--minecraft-dir" => value("--minecraft-dir").map(|v| options.minecraft_dir = Some(v)),
            "--version-json" => {
                value("--version-json").map(|v| options.inputs.version_json = Some(v))
            }
            "--client-jar" => value("--client-jar").map(|v| options.inputs.client_jar = Some(v)),
            "--libraries-dir" => {
                value("--libraries-dir").map(|v| options.inputs.libraries_dir = Some(v))
            }
            "--assets-dir" => value("--assets-dir").map(|v| options.inputs.assets_dir = Some(v)),
            "--java-home" => value("--java-home").map(|v| options.java_home = Some(v)),
            "--root" => value("--root").map(|v| root = Some(v)),
            "--output" => value("--output").map(|v| options.output = Some(v)),
            "--check" if inline.is_none() => {
                check = true;
                Ok(())
            }
            "--complete" if inline.is_none() && assets.is_none() => {
                complete = true;
                Ok(())
            }
            "--progress" => match value("--progress") {
                Ok(v) if v.as_os_str() == "json" => {
                    json = true;
                    Ok(())
                }
                Ok(v) => Err(format!("--progress takes json, not {}", v.display())),
                Err(e) => Err(e),
            },
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("horizon-prepare {VERSION}");
                return ExitCode::SUCCESS;
            }
            _ => Err(format!("unrecognized argument: {text}")),
        };
        if let Err(e) = parsed {
            eprintln!("horizon-prepare: error: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    }
    let Some(root) = root.or_else(default_root) else {
        eprintln!(
            "horizon-prepare: error: no research-lock.json beside this program or above the current folder; pass --root"
        );
        return ExitCode::from(2);
    };
    options.root = root;
    if check {
        if options.import || options.server_jar.is_some() {
            eprintln!("horizon-prepare: error: --check prepares nothing; leave out --prepare and --server-jar");
            return ExitCode::from(2);
        }
        if json || complete {
            eprintln!("horizon-prepare: error: --progress and --complete apply to preparing; leave them out with --check");
            return ExitCode::from(2);
        }
        return check_resources(&options);
    }
    let print = |line: &str| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    };
    let log = |line: &str| match json {
        true => print(&json_line("message", line)),
        false => print(line),
    };
    // Preparing is most of a complete run; the sounds and the check are the rest.
    let scale = if complete { 0.9 } else { 1.0 };
    let progress = |fraction: f32| {
        if json {
            print(&fraction_line(fraction * scale));
        }
    };
    let result = match assets.as_deref().and_then(|a| a.to_str()) {
        Some("compile-exporters") => horizon_prepare::compile_exporters(&options, &log)
            .inspect(|dir| log(&dir.display().to_string())),
        Some(kind @ ("sounds" | "panorama" | "fonts" | "packs" | "languages")) => {
            horizon_prepare::add_assets(
                kind,
                &options.root,
                options.output.as_deref(),
                options.minecraft_dir.as_deref(),
                &options.inputs,
                &log,
            )
        }
        _ => horizon_prepare::prepare_with_progress(&options, &log, &progress).and_then(
            |resources| match complete {
                true => complete_resources(&options, resources, &log),
                false => Ok(resources),
            },
        ),
    };
    match result {
        Ok(_) => {
            // A plain preparation reported its 1 already.
            if json && (complete || assets.is_some()) {
                print(&fraction_line(1.0));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                print(&json_line("error", &e));
            }
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--complete` after preparing: the sounds, then nothing may be missing.
fn complete_resources(
    options: &horizon_prepare::Options,
    resources: PathBuf,
    log: horizon_prepare::Log,
) -> Result<PathBuf, String> {
    horizon_prepare::add_assets(
        "sounds",
        &options.root,
        Some(&resources),
        options.minecraft_dir.as_deref(),
        &options.inputs,
        log,
    )?;
    let lacking = horizon_prepare::check(&options.root, &resources)?;
    if lacking.is_empty() {
        return Ok(resources);
    }
    for line in &lacking {
        log(line);
    }
    Err(format!(
        "the resources are not complete: {}",
        lacking.join("; ")
    ))
}

/// `{"key": "text"}` on one line: `--progress json`'s `message` and `error` (docs/api/launch.md).
fn json_line(key: &str, text: &str) -> String {
    let mut out = format!("{{\"{key}\": ");
    horizon_prepare::pyjson::write_string(&mut out, text);
    out.push('}');
    out
}

/// `--progress json`'s `{"fraction": F}`, F from 0 to 1 with three decimals.
fn fraction_line(fraction: f32) -> String {
    format!("{{\"fraction\": {:.3}}}", fraction.clamp(0.0, 1.0))
}

/// `--check`: each gap on its own line on stdout, exit 1 when there is one.
fn check_resources(options: &horizon_prepare::Options) -> ExitCode {
    let resources = horizon_prepare::artifacts::Lock::read(&options.root).map(|lock| {
        horizon_prepare::resources_dir(&options.root, &lock.version, options.output.as_deref())
    });
    match resources.and_then(|resources| horizon_prepare::check(&options.root, &resources)) {
        Ok(lacking) if lacking.is_empty() => ExitCode::SUCCESS,
        Ok(lacking) => {
            for line in lacking {
                println!("{line}");
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--progress json`'s three lines as docs/api/launch.md promises them: a launcher parses
    /// them, so a refactor must not change them silently.
    #[test]
    fn progress_json_lines_keep_their_shape() {
        assert_eq!(
            json_line("message", "Copying \"sounds\"\nnext"),
            r#"{"message": "Copying \"sounds\"\nnext"}"#
        );
        assert_eq!(
            json_line("error", "no Java 25"),
            r#"{"error": "no Java 25"}"#
        );
        assert_eq!(fraction_line(0.05), r#"{"fraction": 0.050}"#);
        assert_eq!(fraction_line(1.0), r#"{"fraction": 1.000}"#);
        for line in [json_line("message", "tab\there, é"), fraction_line(0.9)] {
            assert!(horizon_prepare::pyjson::loads(&line).is_ok(), "{line}");
        }
    }
}
