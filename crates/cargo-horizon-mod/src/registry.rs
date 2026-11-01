//! `cargo horizon-mod registry build|review|check|list`: the registry repository's CI
//! (`horizon-rust/horizon-mods`, docs/api/mod-index.md). Network-free: the
//! workflow downloads the files and publishes what this writes.
use horizon_mods::index::{
    FILE_HOST,
    registry::{self, Sources},
};
use std::path::Path;

/// Where the registry's site is published (its icons).
const SITE: &str = "https://horizon-rust.github.io/horizon-mods/";

const USAGE: &str = "usage: cargo horizon-mod registry build <checkout> --files <dir> --out <index.json> [--release <url>] [--site <url>] [--generated <time>]
       cargo horizon-mod registry review <base checkout> <head checkout> --author <login> [--json]
       cargo horizon-mod registry check <base checkout> <head checkout> --files <dir>
       cargo horizon-mod registry list <checkout>
       cargo horizon-mod registry release <checkout> <id> <file> --tag <tag> --published <YYYY-MM-DD> [--notes <file>] [--signature <file>] [--game <version>]
       cargo horizon-mod registry page <checkout> <id> <horizon-registry.toml>";

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    super::flag(args, name)
}

fn positional(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--json" {
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a.as_str());
        }
    }
    out
}

fn lines(errors: Vec<String>) -> String {
    format!("{} problem(s):\n  {}", errors.len(), errors.join("\n  "))
}

pub fn main(args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or_default();
    let pos = positional(rest);
    match args.first().map(String::as_str) {
        Some("build") => {
            let [checkout] = pos[..] else {
                return Err(USAGE.into());
            };
            let (files, out) = (flag(rest, "--files"), flag(rest, "--out"));
            let (Some(files), Some(out)) = (files, out) else {
                return Err(USAGE.into());
            };
            let generated = flag(rest, "--generated")
                .map(str::to_owned)
                .or_else(|| std::env::var("REGISTRY_GENERATED").ok())
                .unwrap_or_default();
            let index = registry::build(&Sources {
                registry: Path::new(checkout),
                files: Path::new(files),
                release: flag(rest, "--release").unwrap_or(FILE_HOST),
                site: flag(rest, "--site").unwrap_or(SITE),
                generated,
            })
            .map_err(lines)?;
            let text = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
            std::fs::write(out, text + "\n").map_err(|e| format!("{out}: {e}"))?;
            println!(
                "Wrote {out}: {} project(s), {} taken down",
                index.projects.len(),
                index.revoked.len()
            );
            Ok(())
        }
        Some("review") => {
            let [base, head] = pos[..] else {
                return Err(USAGE.into());
            };
            let author = flag(rest, "--author").ok_or(USAGE)?;
            let (base, _) = registry::load(Path::new(base)).map_err(lines)?;
            let (head, _) = registry::load(Path::new(head)).map_err(lines)?;
            let review = registry::review(&base, &head, author);
            if rest.iter().any(|a| a == "--json") {
                let json = serde_json::json!({"errors": review.errors, "notes": review.notes});
                println!("{json}");
            } else {
                for n in &review.notes {
                    println!("note: {n}");
                }
                for e in &review.errors {
                    println!("error: {e}");
                }
            }
            if review.errors.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} change(s) the registry refuses",
                    review.errors.len()
                ))
            }
        }
        // `check` (and its trial run) on each file `head` lists and `base`
        // doesn't: a pull request's new files.
        Some("check") => {
            let [base, head] = pos[..] else {
                return Err(USAGE.into());
            };
            let files = Path::new(flag(rest, "--files").ok_or(USAGE)?);
            let old: Vec<String> = match registry::load(Path::new(base)) {
                Ok((entries, _)) => entries
                    .iter()
                    .flat_map(|e| e.versions.iter().map(|v| v.sha256.clone()))
                    .collect(),
                // The first pull request has no entries to compare with.
                Err(_) => Vec::new(),
            };
            let (entries, _) = registry::load(Path::new(head)).map_err(lines)?;
            let mut failed = 0;
            for e in &entries {
                for v in e.versions.iter().filter(|v| !old.contains(&v.sha256)) {
                    println!("{} {} ({}):", e.id, v.version, v.file);
                    let report = crate::check::check(
                        &files.join(&v.sha256),
                        e.kind.clone(),
                        e.key.as_deref(),
                        v.signature.as_deref(),
                    );
                    match report {
                        Ok(r) => print!("{}", r.text()),
                        Err(err) => {
                            failed += 1;
                            println!("  error: {err}");
                        }
                    }
                }
            }
            if failed == 0 {
                Ok(())
            } else {
                Err(format!("{failed} file(s) failed their check"))
            }
        }
        // The files CI must download for `build`: `<sha256> <id> <version> <file>`.
        Some("list") => {
            let [checkout] = pos[..] else {
                return Err(USAGE.into());
            };
            let (entries, _) = registry::load(Path::new(checkout)).map_err(lines)?;
            for e in &entries {
                for v in &e.versions {
                    println!("{} {} {} {}", v.sha256, e.id, v.version, v.file);
                }
            }
            Ok(())
        }
        // A release of a project's repository, downloaded: the checks of a submission, then the
        // version as the index lists it (JSON on stdout), for the registry's release sync.
        Some("release") => {
            let [checkout, id, file] = pos[..] else {
                return Err(USAGE.into());
            };
            let (tag, published) = (
                flag(rest, "--tag").ok_or(USAGE)?,
                flag(rest, "--published").ok_or(USAGE)?,
            );
            let read = |name: &str| {
                flag(rest, name)
                    .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
                    .transpose()
            };
            let (notes, signature) = (read("--notes")?, read("--signature")?);
            let entry = entry_of(checkout, id)?;
            let path = Path::new(file);
            let cache = registry::Cache::read(Path::new(checkout), id)?;
            let entry = entry.resolved(&cache.page);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            crate::check::check(
                path,
                entry.kind.clone(),
                entry.key.as_deref(),
                signature.as_deref(),
            )?;
            let version = registry::release_version(
                &entry,
                &registry::Release {
                    tag,
                    published,
                    notes: notes.as_deref().unwrap_or(""),
                    asset: flag(rest, "--name").unwrap_or(name),
                    game: flag(rest, "--game").unwrap_or("26.3"),
                    signature: signature.as_deref(),
                },
                path,
            )?;
            println!(
                "{}",
                serde_json::to_string(&version).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        // The page file of a project's repository, checked against its entry: the page as JSON.
        Some("page") => {
            let [checkout, id, file] = pos[..] else {
                return Err(USAGE.into());
            };
            let entry = entry_of(checkout, id)?;
            let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            let page = registry::page(&entry, &text)?;
            println!(
                "{}",
                serde_json::to_string(&page).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// `mods/<id>.toml` of the checkout, which must name a repository.
fn entry_of(checkout: &str, id: &str) -> Result<registry::Entry, String> {
    let (entries, _) = registry::load(Path::new(checkout)).map_err(lines)?;
    entries
        .into_iter()
        .find(|e| e.id == id && e.repo.is_some())
        .ok_or_else(|| format!("{id}: no entry with a repo"))
}
