//! docs/api/server-reference.md: `horizon-server`'s commands, every `server.toml` key and the control API's
//! endpoints, generated from the source, and the tests that fail when it is out of date. Update it with:
//! `HORIZON_WRITE_DOCS=1 cargo test -p horizon-server reference`.

use crate::{api::ROUTES, config};
use std::fmt::Write;

const DOC: &str = "docs/api/server-reference.md";

/// One `key = value  # what it means` line of the template, under its `[section]`.
struct Key {
    section: String,
    name: String,
    default: String,
    what: String,
}

/// The template's sections, each with the comment lines above it, and its keys, in order.
fn template() -> (Vec<(String, String)>, Vec<Key>) {
    let (mut sections, mut keys, mut notes) = (Vec::new(), Vec::new(), Vec::<&str>::new());
    for line in config::TEMPLATE.lines().map(str::trim) {
        if let Some(note) = line.strip_prefix('#') {
            notes.push(note.trim());
        } else if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((section.to_owned(), notes.join(" ")));
            notes.clear();
        } else if let Some((name, rest)) = line.split_once(" = ") {
            let (default, what) = rest.split_once(" # ").unwrap_or((rest, ""));
            keys.push(Key {
                section: sections.last().map(|(s, _)| s.clone()).unwrap_or_default(),
                name: name.trim().to_owned(),
                default: default.trim().to_owned(),
                what: what.trim().to_owned(),
            });
        }
    }
    (sections, keys)
}

pub fn markdown() -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "<!-- Generated from crates/horizon-server/src (the help text, config::TEMPLATE, api::ROUTES) by reference.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-server reference -->"
    );
    let _ = writeln!(w, "# Server reference\n");
    let _ = writeln!(
        w,
        "Everything `horizon-server` offers a host or a tool: its commands, every `server.toml` key and the \
         control API. How to host: [hosting.md](../hosting.md); the design: [server.md](../server.md); \
         stability and versions: [stability.md](stability.md).\n"
    );

    let _ = writeln!(w, "## Commands\n\n`horizon-server help` prints:\n");
    let _ = writeln!(w, "```text\n{}```\n", crate::HELP);

    let _ = writeln!(w, "## `server.toml`\n");
    let (sections, keys) = template();
    let _ = writeln!(
        w,
        "{} Unknown keys are refused. `horizon-server init` writes this file with these defaults; \
         `horizon-server config set SECTION.KEY=VALUE` changes it.\n",
        sections[0].1
    );
    for (i, (section, note)) in sections.iter().enumerate() {
        let _ = writeln!(w, "### `[{section}]`\n");
        if i > 0 && !note.is_empty() {
            let _ = writeln!(w, "{note}\n");
        }
        let rows: Vec<&Key> = keys.iter().filter(|k| &k.section == section).collect();
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(
            w,
            "| Key | Default | In `server.properties` | What it does |\n| --- | --- | --- | --- |"
        );
        for k in rows {
            let vanilla = config::TYPED
                .iter()
                .find(|(s, n, _)| s == section && *n == k.name)
                .map_or(String::new(), |(_, _, v)| format!("`{v}`"));
            let _ = writeln!(
                w,
                "| `{}` | `{}` | {vanilla} | {} |",
                k.name, k.default, k.what
            );
        }
        let _ = writeln!(w);
    }

    let _ = writeln!(w, "## Control API (`/api/v1`)\n");
    let _ = writeln!(
        w,
        "HTTP and JSON on 127.0.0.1. A running server writes its address and a random token to \
         `.horizon/api.json`, readable only by its own user; every request sends \
         `Authorization: Bearer <token>` and a `Host` that is that address. Errors answer \
         `{{\"error\": \"...\"}}` with 400, 401, 403, 404, 405 or 409. The web panel uses the same \
         routes with its session.\n"
    );
    let _ = writeln!(
        w,
        "| Method | Path | Takes | Answers | What it does |\n| --- | --- | --- | --- | --- |"
    );
    for r in &ROUTES {
        let takes = if r.takes.is_empty() {
            "nothing"
        } else {
            r.takes
        };
        let _ = writeln!(
            w,
            "| `{}` | `{}` | {takes} | {} | {} |",
            r.method, r.path, r.answers, r.what
        );
    }
    out
}

#[test]
fn the_server_reference_is_up_to_date() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DOC);
    let want = markdown();
    if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "{DOC} is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-server reference"
    );
}

/// The template, which the reference lists, holds every key `Config` reads and no other.
#[test]
fn the_reference_lists_every_setting() {
    let config = toml::Value::try_from(config::Config::default()).unwrap();
    let mut listed: Vec<String> = template()
        .1
        .iter()
        .map(|k| format!("{}.{}", k.section, k.name))
        .collect();
    let mut read: Vec<String> = config
        .as_table()
        .unwrap()
        .iter()
        .filter_map(|(section, keys)| Some((section, keys.as_table()?)))
        .flat_map(|(section, keys)| keys.keys().map(move |k| format!("{section}.{k}")))
        .collect();
    listed.sort();
    read.sort();
    assert_eq!(listed, read);
    for key in template().1 {
        assert!(
            !key.what.is_empty(),
            "{}.{} needs a `# what it does` comment in config::TEMPLATE",
            key.section,
            key.name
        );
    }
}

/// `ROUTES` holds exactly the endpoints the API's code answers.
#[test]
fn the_routes_table_matches_the_code() {
    let source = include_str!("api.rs");
    let mut answered: Vec<(String, String)> = Vec::new();
    for method in ["GET", "POST", "PATCH", "DELETE"] {
        let open = format!("(\"{method}\", \"/api/v1/");
        for (at, _) in source.match_indices(&open) {
            let rest = &source[at + open.len()..];
            let path = &rest[..rest.find('"').unwrap()];
            if rest[path.len()..].starts_with("\")") {
                answered.push((method.into(), format!("/api/v1/{path}")));
            }
        }
    }
    let mut listed: Vec<(String, String)> = ROUTES
        .iter()
        .map(|r| (r.method.into(), r.path.into()))
        .collect();
    answered.sort();
    listed.sort();
    assert_eq!(answered, listed);
}
