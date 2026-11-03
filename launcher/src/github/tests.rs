use super::*;
use std::cell::RefCell;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-github-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

const T0: u64 = 1_800_000_000;
const LATER: u64 = T0 + FRESH_SECS;

const RELEASES: &str = "https://api.github.com/repos/horizon-rust/horizon/releases?per_page=100";

fn list(tag: &str) -> serde_json::Value {
    serde_json::json!([{ "tag_name": tag }])
}

fn no_mirror(_: &str) -> Answer {
    panic!("the mirror is only asked when GitHub can't answer")
}

#[test]
fn mirror_serves_only_horizons_repository() {
    let base = "https://mcp.example/github/public";
    assert_eq!(
        mirror_url_with(RELEASES, base).as_deref(),
        Some("https://mcp.example/github/public/repos/horizon-rust/horizon/releases?per_page=100")
    );
    assert_eq!(
        mirror_url_with(
            "https://api.github.com/repos/horizon-rust/horizon/releases/tags/launcher@0.2.33",
            &format!("{base}/")
        )
        .as_deref(),
        Some("https://mcp.example/github/public/repos/horizon-rust/horizon/releases/tags/launcher@0.2.33")
    );
    assert_eq!(
        mirror_url_with("https://api.github.com/repos/someone/mod/commits/abc", base),
        None
    );
    assert_eq!(
        mirror_url_with("https://github.com/horizon-rust/horizon/releases", base),
        None
    );
    assert_eq!(
        mirror_url_with(RELEASES, ""),
        None,
        "an empty mirror is off"
    );
}

#[test]
fn rate_limit_is_429_or_403_with_githubs_headers() {
    let none = |_: &str| None;
    assert!(rate_limited(429, none));
    assert!(!rate_limited(403, none), "a plain 403 is a refusal");
    assert!(rate_limited(403, |h| (h == "x-ratelimit-remaining").then(|| "0".into())));
    assert!(!rate_limited(403, |h| (h == "x-ratelimit-remaining").then(|| "12".into())));
    assert!(rate_limited(403, |h| (h == "retry-after").then(|| "60".into())));
    assert!(!rate_limited(500, |h| (h == "retry-after").then(|| "60".into())));
}

#[test]
fn etag_is_kept_and_a_304_reads_the_saved_copy() {
    let cache = scratch("etag");
    let sent = RefCell::new(Vec::new());
    let first = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            sent.borrow_mut().push(etag.map(str::to_owned));
            Answer::Fresh(list("nightly"), Some("\"abc\"".into()))
        },
        no_mirror,
    );
    assert_eq!(first.unwrap(), list("nightly"));
    // Started again within FRESH_SECS: the saved list, without asking.
    let again = json_with(
        RELEASES,
        &cache,
        T0 + FRESH_SECS - 1,
        |_, _| panic!("a fresh copy is used without asking"),
        no_mirror,
    );
    assert_eq!(again.unwrap(), list("nightly"));
    // Later: a conditional request; a 304 gives the saved list and starts a new fresh window.
    let second = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, etag| {
            sent.borrow_mut().push(etag.map(str::to_owned));
            Answer::NotModified
        },
        no_mirror,
    );
    assert_eq!(second.unwrap(), list("nightly"));
    assert_eq!(*sent.borrow(), [None, Some("\"abc\"".to_owned())]);
    let third = json_with(
        RELEASES,
        &cache,
        LATER + FRESH_SECS - 1,
        |_, _| panic!("confirmed by the 304"),
        no_mirror,
    );
    assert_eq!(third.unwrap(), list("nightly"));
    // A clock set back doesn't make the copy fresh forever.
    let back = json_with(
        RELEASES,
        &cache,
        T0 - 1,
        |_, _| Answer::Fresh(list("v9"), None),
        no_mirror,
    );
    assert_eq!(back.unwrap(), list("v9"));
    // Another URL doesn't read this one's copy.
    let other = "https://api.github.com/repos/horizon-rust/horizon/releases/latest";
    let etag_sent = RefCell::new(Some(String::new()));
    let _ = json_with(
        other,
        &cache,
        T0,
        |_, etag| {
            *etag_sent.borrow_mut() = etag.map(str::to_owned);
            Answer::Fresh(list("v1"), None)
        },
        no_mirror,
    );
    assert_eq!(*etag_sent.borrow(), None);
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn rate_limited_reads_go_through_the_mirror() {
    let cache = scratch("mirror");
    let asked = RefCell::new(None);
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |url| {
            *asked.borrow_mut() = Some(url.to_owned());
            Answer::Fresh(list("nightly"), None)
        },
    );
    assert_eq!(got.unwrap(), list("nightly"));
    assert!(asked
        .borrow()
        .as_deref()
        .unwrap()
        .ends_with("/github/public/repos/horizon-rust/horizon/releases?per_page=100"));
    // The mirror's answer is kept like GitHub's.
    let kept = json_with(RELEASES, &cache, T0 + 1, |_, _| panic!("fresh"), no_mirror);
    assert_eq!(kept.unwrap(), list("nightly"));
    // No connection to GitHub (blocked, DNS): the mirror too.
    let got = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, _| Answer::Failed("No connection: dns".into()),
        |_| Answer::Fresh(list("v2"), None),
    );
    assert_eq!(got.unwrap(), list("v2"));
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn a_404_is_not_retried_on_the_mirror() {
    let cache = scratch("404");
    let err = json_with(RELEASES, &cache, T0, |_, _| Answer::NotFound, no_mirror).unwrap_err();
    assert!(err.ends_with("answered 404"), "{err}");
    let err = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |_| Answer::NotFound,
    )
    .unwrap_err();
    assert!(err.ends_with("answered 404"), "{err}");
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn both_failing_uses_the_saved_copy_else_says_so() {
    let cache = scratch("both");
    let err = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |_| Answer::Failed("No connection: timed out".into()),
    )
    .unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
    assert!(err.contains("mirror"), "{err}");
    assert!(err.contains("timed out"), "{err}");
    assert!(err.contains("Try again in a few minutes"), "{err}");

    // With a copy from an earlier start, that copy is better than nothing.
    json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::Fresh(list("nightly"), Some("\"e\"".into())),
        no_mirror,
    )
    .unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, _| Answer::RateLimited,
        |_| Answer::RateLimited,
    );
    assert_eq!(got.unwrap(), list("nightly"));
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn other_repositories_have_no_mirror() {
    let cache = scratch("other");
    let err = json_with(
        "https://api.github.com/repos/someone/mod/commits/abc",
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        no_mirror,
    )
    .unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
    assert!(!err.contains("mirror"), "{err}");
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn a_corrupt_or_foreign_copy_is_ignored() {
    let cache = scratch("corrupt");
    fs::write(cache.join(cache_name(RELEASES)), b"not json").unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            assert_eq!(etag, None);
            Answer::Fresh(list("v1"), None)
        },
        no_mirror,
    );
    assert_eq!(got.unwrap(), list("v1"));
    let foreign = Kept {
        url: "https://api.github.com/other".into(),
        etag: "\"x\"".into(),
        saved: T0,
        body: list("x"),
    };
    fs::write(
        cache.join(cache_name(RELEASES)),
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            assert_eq!(etag, None);
            Answer::Fresh(list("v2"), None)
        },
        no_mirror,
    );
    assert_eq!(got.unwrap(), list("v2"));
    let _ = fs::remove_dir_all(cache);
}
