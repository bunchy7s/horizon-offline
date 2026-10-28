//! Nightly (docs/release-flow.md): the only build Horizon ships since 2026-10-28, main plus the pull
//! requests the trusted reviewers picked, published as the `nightly` pre-release a few times a day.
//! Every launcher installs it as its update (`backend::horizon_update`), as its own version
//! `nightly-<date>-<commit>`, beside the player's worlds, which it never touches. Its checksums must
//! be signed for the nightly channel (`signed::nightly_digest`).
//!
//! Before that it was opt-in and played in its own "Nightly" instance (`instances/nightly/`):
//! that instance, and its worlds, stay, and keep following the Nightly.
use crate::backend::{self, Release};
use crate::instances;
use horizon_release_sign::NIGHTLY;

/// The id of the instance (its folder in `instances/`) the opt-in Nightly played in.
pub const INSTANCE: &str = "nightly";

/// The version a nightly installs as: `nightly-2026-10-28-0c02067`, from its title
/// ("Nightly 2026-10-28 · 0c02067"), else its publication date.
pub fn version_of(release: &Release) -> String {
    let mut words = release
        .name
        .strip_prefix("Nightly ")
        .unwrap_or_default()
        .split(" · ");
    let date = words.next().filter(|d| is_date(d)).unwrap_or(&release.date);
    let commit = words
        .next()
        .filter(|c| !c.is_empty() && c.chars().all(|c| c.is_ascii_hexdigit()));
    match commit {
        Some(commit) => format!("{NIGHTLY}-{date}-{commit}"),
        None => format!("{NIGHTLY}-{date}"),
    }
}

/// The date of a nightly's version (`nightly-2026-10-28-0c02067`), or None for any other version.
pub fn date_of(version: &str) -> Option<&str> {
    let date = version.strip_prefix("nightly-")?.get(..10)?;
    is_date(date).then_some(date)
}

fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// After `version`, a Nightly, installed as an update: the old Nightly instance follows it, and the
/// older Nightlies go unless the selection or an instance still plays them. Worlds stay.
pub fn after_update(version: &str) {
    let list = instances::list();
    if list
        .iter()
        .any(|i| i.id == INSTANCE && i.version.as_deref().is_some_and(|v| date_of(v).is_some()))
    {
        let _ = instances::edit(INSTANCE, |meta| meta.version = Some(version.to_owned()));
    }
    let selected = backend::Settings::load().selected;
    let pinned: Vec<String> = instances::list()
        .into_iter()
        .filter_map(|i| i.version)
        .collect();
    for old in stale(&backend::installed(), version, selected.as_deref(), &pinned) {
        let _ = backend::remove(&old);
    }
}

/// The installed Nightlies to remove once `current` is installed: every other one that is neither
/// selected nor pinned by an instance.
fn stale(
    installed: &[String],
    current: &str,
    selected: Option<&str>,
    pinned: &[String],
) -> Vec<String> {
    installed
        .iter()
        .filter(|v| date_of(v).is_some() && *v != current && Some(v.as_str()) != selected)
        .filter(|v| !pinned.contains(v))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(name: &str, date: &str) -> Release {
        Release {
            tag: NIGHTLY.into(),
            name: name.into(),
            body: String::new(),
            date: date.into(),
            asset: Some(("https://example/horizon.zip".into(), 1)),
            prerelease: true,
            nightly: None,
        }
    }

    #[test]
    fn a_nightly_installs_under_its_date_and_commit() {
        let r = release("Nightly 2026-10-28 · 0c02067", "2026-10-28");
        assert_eq!(version_of(&r), "nightly-2026-10-28-0c02067");
        let untitled = release("Something else", "2026-10-29");
        assert_eq!(version_of(&untitled), "nightly-2026-10-29");
        assert_eq!(date_of(&version_of(&r)), Some("2026-10-28"));
        assert_eq!(date_of("v0.2.95"), None);
        assert_eq!(date_of("nightly"), None);
        assert!(horizon_release_sign::is_nightly(&version_of(&r)));
    }

    #[test]
    fn only_unused_older_nightlies_are_removed() {
        let installed = [
            "nightly-2026-10-29-aaaaaaa",
            "nightly-2026-10-28-bbbbbbb",
            "nightly-2026-10-28-ccccccc",
            "nightly-2026-10-27-ddddddd",
            "v0.2.95",
        ]
        .map(String::from);
        let pinned = ["nightly-2026-10-27-ddddddd".to_string()];
        assert_eq!(
            stale(
                &installed,
                "nightly-2026-10-29-aaaaaaa",
                Some("nightly-2026-10-28-ccccccc"),
                &pinned
            ),
            ["nightly-2026-10-28-bbbbbbb"]
        );
    }
}
