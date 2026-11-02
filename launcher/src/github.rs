//! GitHub's REST API, as the launcher reads it: release lists, releases by tag, commits.
//!
//! Without a token GitHub allows 60 API calls an hour per address, and players behind a shared one
//! (a school, a dorm, a carrier's NAT) spend them together. So:
//! - every answer is kept on disk (`<data>/cache/github/`) and read again without asking for
//!   [`FRESH_SECS`]: a launcher started a few times in a row, or several pages asking for the same
//!   list, spend one call. After that the read is a conditional request with the saved ETag. An
//!   anonymous 304 still counts against the hour (measured 2026-11-01), so that only saves the
//!   download; with `HORIZON_GITHUB_TOKEN` (CI) a 304 is free;
//! - when GitHub answers with its rate limit (or not at all), Horizon's own repository is read
//!   through Kira's public mirror ([`MIRROR`], kira's `src/launcher-github.js`), which caches
//!   GitHub's answers for everyone;
//! - when both fail, the last answer kept on disk is used, else the error says what happened.
//!
//! Release files are not read here: their `browser_download_url` (github.com, then
//! release-assets.githubusercontent.com) is not part of the API and has no such limit.
use crate::backend;
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};

const API: &str = "https://api.github.com";
/// Kira's read-only mirror of Horizon's release reads. `HORIZON_GITHUB_MIRROR` replaces it (an
/// empty value turns it off).
pub const MIRROR: &str = "https://mcp.playhorizon.dev/github/public";

/// How long a saved answer is used without asking again: the launcher looks for releases every
/// [`backend::CHECK_EVERY`], so a few minutes late at worst.
pub const FRESH_SECS: u64 = 5 * 60;

/// What one source answered.
#[derive(Debug)]
pub(crate) enum Answer {
    /// 200, with the ETag GitHub gave (none from the mirror).
    Fresh(serde_json::Value, Option<String>),
    /// 304: the copy on disk is still current.
    NotModified,
    NotFound,
    /// 429, or 403 with GitHub's rate-limit headers.
    RateLimited,
    /// Any other status, or no connection: the error to show.
    Failed(String),
}

/// GitHub's JSON for `url` (`https://api.github.com/...`). A 404 is the error `<url> answered 404`.
pub fn json(url: &str) -> Result<serde_json::Value, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    json_with(url, &cache_dir(), now, github, mirror)
}

/// [`json`] with its cache folder, the time (Unix seconds) and its two sources given (tests).
pub(crate) fn json_with(
    url: &str,
    cache: &Path,
    now: u64,
    github: impl Fn(&str, Option<&str>) -> Answer,
    mirror: impl Fn(&str) -> Answer,
) -> Result<serde_json::Value, String> {
    let file = cache.join(cache_name(url));
    let kept = read_kept(&file, url);
    if let Some(k) = kept.as_ref() {
        // A copy saved "later" than now (the clock was set back) is not trusted as fresh.
        if now >= k.saved && now - k.saved < FRESH_SECS {
            return Ok(k.body.clone());
        }
    }
    let etag = kept
        .as_ref()
        .map(|k| k.etag.as_str())
        .filter(|e| !e.is_empty());
    let why = match github(url, etag) {
        Answer::Fresh(body, etag) => {
            write_kept(&file, url, etag.as_deref().unwrap_or(""), &body, now);
            return Ok(body);
        }
        Answer::NotModified => match kept {
            Some(k) => {
                write_kept(&file, url, &k.etag, &k.body, now);
                return Ok(k.body);
            }
            // A 304 to a request that had no ETag: can't happen, but don't trust it.
            None => "GitHub answered 304 without a saved copy".to_owned(),
        },
        Answer::NotFound => return Err(format!("{url} answered 404")),
        Answer::RateLimited => RATE_LIMITED.to_owned(),
        Answer::Failed(e) => e,
    };
    let from_mirror = match mirror_url(url) {
        Some(m) => match mirror(&m) {
            Answer::Fresh(body, _) => {
                write_kept(&file, url, "", &body, now);
                return Ok(body);
            }
            Answer::NotFound => return Err(format!("{url} answered 404")),
            Answer::RateLimited => Some("too many requests from this network".to_owned()),
            Answer::Failed(e) => Some(e),
            Answer::NotModified => Some("an unexpected 304".to_owned()),
        },
        None => None,
    };
    if let Some(k) = kept {
        return Ok(k.body);
    }
    Err(match from_mirror {
        Some(m) => format!(
            "{why}, and Horizon's mirror of GitHub couldn't help either ({m}). Try again in a few minutes."
        ),
        None => why,
    })
}

const RATE_LIMITED: &str = "GitHub's rate limit was reached for your network";

/// The mirror's URL for an API URL it serves (Horizon's own repository), or None.
pub(crate) fn mirror_url(url: &str) -> Option<String> {
    mirror_url_with(url, &mirror_base())
}

fn mirror_url_with(url: &str, base: &str) -> Option<String> {
    let path = url.strip_prefix(API)?;
    let repo = format!("/repos/{}/", backend::REPO);
    (!base.is_empty() && path.starts_with(&repo))
        .then(|| format!("{}{path}", base.trim_end_matches('/')))
}

fn mirror_base() -> String {
    std::env::var("HORIZON_GITHUB_MIRROR").unwrap_or_else(|_| MIRROR.to_owned())
}

/// GitHub itself, with the saved ETag when there is one.
fn github(url: &str, etag: Option<&str>) -> Answer {
    let mut request = backend::agent(url).get(url);
    // CI runners share addresses and hit GitHub's anonymous rate limit; they pass their own token.
    if let Some(token) = std::env::var("HORIZON_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    if let Some(etag) = etag {
        request = request.set("If-None-Match", etag);
    }
    match request.call() {
        Ok(r) if r.status() == 304 => Answer::NotModified,
        Ok(r) => {
            let etag = r.header("ETag").map(str::to_owned);
            match r.into_json() {
                Ok(body) => Answer::Fresh(body, etag),
                Err(e) => Answer::Failed(e.to_string()),
            }
        }
        Err(e) => failure(url, e),
    }
}

fn mirror(url: &str) -> Answer {
    match backend::agent(url).get(url).call() {
        Ok(r) => match r.into_json() {
            Ok(body) => Answer::Fresh(body, None),
            Err(e) => Answer::Failed(e.to_string()),
        },
        Err(e) => failure(url, e),
    }
}

fn failure(url: &str, e: ureq::Error) -> Answer {
    match e {
        ureq::Error::Status(404, _) => Answer::NotFound,
        ureq::Error::Status(code, r) if rate_limited(code, |h| r.header(h).map(str::to_owned)) => {
            Answer::RateLimited
        }
        ureq::Error::Status(code, _) => Answer::Failed(format!("{url} answered {code}")),
        ureq::Error::Transport(t) => Answer::Failed(format!("No connection: {t}")),
    }
}

/// Whether a status with these headers is GitHub's rate limit: always a 429; a 403 only with
/// `x-ratelimit-remaining: 0` (the hourly limit) or `retry-after` (the secondary one), since a 403
/// can also be a refusal.
pub(crate) fn rate_limited(code: u16, header: impl Fn(&str) -> Option<String>) -> bool {
    code == 429
        || (code == 403
            && (header("x-ratelimit-remaining").as_deref() == Some("0")
                || header("retry-after").is_some()))
}

fn cache_dir() -> PathBuf {
    backend::data_dir().join("cache").join("github")
}

fn cache_name(url: &str) -> String {
    let hash = sha2::Sha256::digest(url.as_bytes());
    let hex: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    url: String,
    /// GitHub's ETag, or empty (the mirror's answers, or GitHub gave none).
    etag: String,
    /// When it was saved or last confirmed (Unix seconds).
    saved: u64,
    body: serde_json::Value,
}

fn read_kept(file: &Path, url: &str) -> Option<Kept> {
    let kept: Kept = serde_json::from_slice(&fs::read(file).ok()?).ok()?;
    (kept.url == url).then_some(kept)
}

/// Saves an answer for the next reads; a failure only costs that saving.
fn write_kept(file: &Path, url: &str, etag: &str, body: &serde_json::Value, saved: u64) {
    let kept = Kept {
        url: url.to_owned(),
        etag: etag.to_owned(),
        saved,
        body: body.clone(),
    };
    let Some(dir) = file.parent() else { return };
    let Ok(text) = serde_json::to_vec(&kept) else {
        return;
    };
    let partial = file.with_extension("partial");
    if fs::create_dir_all(dir).is_ok() && fs::write(&partial, text).is_ok() {
        let _ = fs::rename(&partial, file);
    }
}

#[cfg(test)]
#[path = "github/tests.rs"]
mod tests;
