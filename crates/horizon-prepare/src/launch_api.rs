//! The preparer's side of the Launch API (docs/api/launch.md): the options another launcher may
//! pass and `horizon-prepare --launch-api`'s reply, so it can ask what this preparer supports
//! before it runs one.

use crate::artifacts::Lock;
use crate::pyjson::{self, Value};

/// The Launch API's version, the same line as `horizon-client --launch-api`'s
/// (docs/api/stability.md): an addition makes the next patch.
pub const VERSION: &str = "0.1.0";

/// What a launcher may pass, with its value's placeholder (none for a flag). The asset commands
/// (`sounds`, `fonts`...) stay out: `--complete` runs what a launcher needs of them.
pub const OPTIONS: &[(&str, Option<&str>)] = &[
    ("--prepare", None),
    ("--server-jar", Some("PATH")),
    ("--minecraft-dir", Some("PATH")),
    ("--version-json", Some("PATH")),
    ("--client-jar", Some("PATH")),
    ("--libraries-dir", Some("PATH")),
    ("--assets-dir", Some("PATH")),
    ("--java-home", Some("PATH")),
    ("--root", Some("PATH")),
    ("--output", Some("PATH")),
    ("--check", None),
    ("--complete", None),
    ("--progress", Some("json")),
    ("--launch-api", None),
];

/// 0 done, 1 failed or gaps found, 2 a refused command line.
pub const EXIT_CODES: &[i64] = &[0, 1, 2];

/// `--launch-api`'s reply for this preparer (`horizon`, its version) and the pins beside it.
pub fn reply(horizon: &str, lock: &Lock) -> String {
    let int = |n: i64| Value::Int(n.to_string());
    let options = OPTIONS
        .iter()
        .map(|(name, value)| {
            Value::Object(vec![
                ("name".into(), Value::str(*name)),
                ("value".into(), value.map_or(Value::Null, Value::str)),
            ])
        })
        .collect();
    let reply = Value::Object(vec![
        ("launch_api".into(), Value::str(VERSION)),
        ("horizon".into(), Value::str(horizon)),
        ("minecraft".into(), Value::str(&lock.version)),
        ("java_major".into(), int(lock.java_major)),
        ("schema".into(), int(crate::SCHEMA)),
        (
            "output_version".into(),
            int(i64::from(crate::OUTPUT_VERSION)),
        ),
        ("options".into(), Value::Array(options)),
        (
            "exit_codes".into(),
            Value::Array(EXIT_CODES.iter().map(|&c| int(c)).collect()),
        ),
    ]);
    pyjson::dumps(&reply, &pyjson::INDENT2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_names_the_pins_and_the_layout() {
        let root = std::env::temp_dir().join(format!("horizon-launch-api-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("research-lock.json"),
            r#"{"minecraft": {"version": "9.9", "java_major": 25}}"#,
        )
        .unwrap();
        let lock = Lock::read(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let reply = pyjson::loads(&reply("1.2.3", &lock)).unwrap();
        let text = |key: &str| reply.get(key).and_then(Value::as_str).map(str::to_owned);
        let int = |key: &str| match reply.get(key) {
            Some(Value::Int(n)) => n.clone(),
            other => panic!("{key}: {other:?}"),
        };
        assert_eq!(text("launch_api").as_deref(), Some(VERSION));
        assert_eq!(text("horizon").as_deref(), Some("1.2.3"));
        assert_eq!(text("minecraft").as_deref(), Some("9.9"));
        assert_eq!(int("java_major"), "25");
        assert_eq!(int("schema"), crate::SCHEMA.to_string());
        assert_eq!(int("output_version"), crate::OUTPUT_VERSION.to_string());
        let progress = reply
            .get("options")
            .and_then(|o| match o {
                Value::Array(items) => items
                    .iter()
                    .find(|i| i.get("name").and_then(Value::as_str) == Some("--progress")),
                _ => None,
            })
            .expect("--progress is listed");
        assert_eq!(progress.get("value").and_then(Value::as_str), Some("json"));
    }

    /// One Launch API, one version: the client's and the preparer's move together.
    #[test]
    fn the_client_speaks_the_same_version() {
        let client = include_str!("../../horizon-client/src/game/launch_api.rs");
        assert!(
            client.contains(&format!("pub const VERSION: &str = \"{VERSION}\";")),
            "horizon-client's Launch API version is not {VERSION}"
        );
    }
}
