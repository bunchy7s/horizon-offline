//! Signed release checksums (docs/security.md, "Signed updates").
//!
//! Every release carries `SHA256SUMS` (`sha256sum`'s format, `<hex>  <file>` per line) and
//! `SHA256SUMS.sig`: one base64 Ed25519 signature per line, each over `horizon release <tag>\n`
//! followed by SHA256SUMS's bytes, so a signed list can't be passed off under another tag.
//! release.yml and launcher.yml sign with the key(s) in the `release` environment's secrets
//! (tools/ci/sign_release.sh, OpenSSL only); the launcher and `horizon-server update` check a
//! download against the public keys built in from `release-keys.txt` before installing it, and
//! refuse it otherwise.
//!
//! Releases published before signing began carry no SHA256SUMS. Their archives' SHA-256s are
//! pinned in `legacy-releases.txt` (taken from GitHub when signing began, by
//! tools/ci/legacy_releases.py), so they stay installable for a rollback without trusting
//! whatever GitHub serves for them later.

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

/// The checksum list's asset name.
pub const SUMS: &str = "SHA256SUMS";
/// Its signatures' asset name.
pub const SIGNATURE: &str = "SHA256SUMS.sig";

const KEYS: &str = include_str!("../release-keys.txt");
const LEGACY: &str = include_str!("../legacy-releases.txt");

/// An Ed25519 public key.
pub type PublicKey = [u8; 32];

/// The public keys built in: those in `release-keys.txt`. A file that doesn't parse gives none,
/// so every download is refused rather than trusted (a test keeps the file valid).
pub fn trusted_keys() -> Vec<PublicKey> {
    parse_keys(KEYS).unwrap_or_default()
}

/// The keys in a keys file: one base64 key per line; blank lines and `#` comments are skipped.
pub fn parse_keys(text: &str) -> Result<Vec<PublicKey>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let bytes = STANDARD
                .decode(line)
                .map_err(|e| format!("{line}: not base64 ({e})"))?;
            PublicKey::try_from(bytes.as_slice())
                .map_err(|_| format!("{line}: not a 32-byte Ed25519 public key"))
        })
        .collect()
}

/// What a signature covers: the tag, then the checksum list.
pub fn message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut message = format!("horizon release {tag}\n").into_bytes();
    message.extend_from_slice(sums);
    message
}

/// A checksum list for `(file name, SHA-256 hex)` pairs, as `sha256sum` writes it.
pub fn sums_file(files: &[(String, String)]) -> String {
    files
        .iter()
        .map(|(name, hex)| format!("{hex}  {name}\n"))
        .collect()
}

/// The `(file name, SHA-256 hex)` pairs of a checksum list (`<hex>  <name>` or `<hex> *<name>`).
pub fn parse_sums(text: &str) -> Result<Vec<(String, String)>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (hex, name) = line
                .split_once(' ')
                .ok_or_else(|| format!("{SUMS}: {line:?} is not `<sha256>  <file>`"))?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
                return Err(format!("{SUMS}: {line:?} is not `<sha256>  <file>`"));
            }
            Ok((name.to_string(), hex.to_ascii_lowercase()))
        })
        .collect()
}

/// Checks that one of `keys` signed `sums` for `tag`, and returns the list.
pub fn verify(
    tag: &str,
    sums: &[u8],
    signature: &str,
    keys: &[PublicKey],
) -> Result<Vec<(String, String)>, String> {
    if keys.is_empty() {
        return Err(
            "This build has no release key built in, so it can't check downloads: none are installed."
                .into(),
        );
    }
    let message = message(tag, sums);
    let signed = signature
        .lines()
        .filter_map(|line| STANDARD.decode(line.trim()).ok())
        .any(|signature| {
            keys.iter().any(|key| {
                UnparsedPublicKey::new(&ED25519, key)
                    .verify(&message, &signature)
                    .is_ok()
            })
        });
    if !signed {
        return Err(format!(
            "{tag}'s signature doesn't match Horizon's release key, so it was not installed. \
             The download may have been tampered with: please report it on Discord or GitHub."
        ));
    }
    let text = std::str::from_utf8(sums).map_err(|_| format!("{SUMS} is not text"))?;
    parse_sums(text)
}

/// The channel nightly builds are signed for (nightly-build.yml): their message is
/// `horizon release nightly`, which no release's tag is, so a nightly's signature never passes for a
/// release's, nor a release's for a nightly's.
pub const NIGHTLY: &str = "nightly";

/// Whether `tag` is a nightly's: `nightly`, or a past one kept as `nightly-<date>`.
pub fn is_nightly(tag: &str) -> bool {
    tag == NIGHTLY || tag.starts_with("nightly-")
}

/// The version a nightly installs as, from its release title ("Nightly 2026-10-28 · 0c02067"):
/// `nightly-2026-10-28-0c02067`. `published` (GitHub's `published_at`) dates a title without one.
/// The launcher and `horizon-server` name it the same way, and native mods read the commit back.
pub fn nightly_version(title: &str, published: &str) -> String {
    let mut words = title
        .strip_prefix("Nightly ")
        .unwrap_or_default()
        .split(" · ");
    let date = words
        .next()
        .filter(|d| is_date(d))
        .unwrap_or_else(|| published.get(..10).unwrap_or(published));
    let commit = words
        .next()
        .filter(|c| !c.is_empty() && c.chars().all(|c| c.is_ascii_hexdigit()));
    match commit {
        Some(commit) => format!("{NIGHTLY}-{date}-{commit}"),
        None => format!("{NIGHTLY}-{date}"),
    }
}

/// The date of a nightly's version or tag (`nightly-2026-10-28-0c02067`, `nightly-2026-10-28`):
/// nightlies follow every release and each other by it.
pub fn nightly_date(version: &str) -> Option<&str> {
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

/// The signed SHA-256 of `asset` in release `tag`, or why the download must be refused.
/// `sums` and `signature` are the release's SHA256SUMS and SHA256SUMS.sig, None when it has none.
/// A nightly is never a release: its tags are refused here, whatever their signature
/// ([`nightly_digest`] checks those).
pub fn signed_digest(
    tag: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    if is_nightly(tag) {
        return Err(format!(
            "{tag} is a Nightly build: only a launcher with Nightly turned on installs it."
        ));
    }
    digest_in(tag, tag, asset, sums, signature, keys)
}

/// The signed SHA-256 of `asset` in the nightly build: its checksums must be signed for the
/// [`NIGHTLY`] channel, so a release's signed list (or one replayed from a release) is refused.
pub fn nightly_digest(
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    digest_in(NIGHTLY, "The Nightly build", asset, sums, signature, keys)
}

/// `asset`'s SHA-256 in `sums`, once one of `keys` is found to have signed them for `signed_for`;
/// `name` says which build in messages.
fn digest_in(
    signed_for: &str,
    name: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    let (Some(sums), Some(signature)) = (sums, signature) else {
        return Err(format!(
            "{name} is not signed, and Horizon installs only signed releases: pick a newer version."
        ));
    };
    verify(signed_for, sums, signature, keys)?
        .into_iter()
        .find(|(n, _)| n == asset)
        .map(|(_, hex)| hex)
        .ok_or_else(|| {
            format!("{asset} is not in {name}'s signed checksums, so it was not installed.")
        })
}

/// The pinned SHA-256 of `asset` in a release from before signing (legacy-releases.txt). Callers
/// use it instead of the release's files when there is one: [`signed_digest`] otherwise.
pub fn legacy_digest(tag: &str, asset: &str) -> Option<&'static str> {
    legacy_entries(LEGACY).find_map(|(t, a, hex)| (t == tag && a == asset).then_some(hex))
}

/// The `<tag> <asset> <sha256>` lines of a legacy releases file; blank lines and `#` comments skipped.
fn legacy_entries(text: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?, parts.next()?))
        })
}

/// Whether a downloaded file's SHA-256 (`got`) is the signed one.
pub fn check(asset: &str, expected: &str, got: &str) -> Result<(), String> {
    if expected.eq_ignore_ascii_case(got) {
        Ok(())
    } else {
        Err(format!(
            "{asset} doesn't match its signed checksum (corrupted or tampered with), so it was \
             not installed. Try again."
        ))
    }
}

/// SHA-256 over data that arrives in pieces (a download).
pub struct Sha256(ring::digest::Context);

impl Default for Sha256 {
    fn default() -> Self {
        Self(ring::digest::Context::new(&ring::digest::SHA256))
    }
}

impl Sha256 {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn hex(self) -> String {
        self.0
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The SHA-256 of `bytes`, in hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::default();
    hash.update(bytes);
    hash.hex()
}

/// A private signing key: `keygen` makes them, and tests sign with them (releases are signed by
/// OpenSSL in tools/ci/sign_release.sh, from the same PKCS#8 document).
pub struct SigningKey(Ed25519KeyPair);

impl SigningKey {
    /// A new key, and its PKCS#8 document in base64 (what the secret holds).
    ///
    /// The document is PKCS#8 v1 (RFC 8410: the 32-byte seed alone), which every OpenSSL 3 reads;
    /// ring makes v2 (with the public key too), which OpenSSL 3.0, Ubuntu's, refuses.
    pub fn generate() -> Result<(Self, String), String> {
        const V2_PREFIX: [u8; 16] = [
            0x30, 0x51, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        const V1_PREFIX: [u8; 16] = [
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        let v2 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .map_err(|_| "no randomness to make a key with".to_string())?;
        let seed = v2
            .as_ref()
            .strip_prefix(&V2_PREFIX)
            .and_then(|rest| rest.get(..32))
            .ok_or("ring's key document has an unexpected layout")?;
        let v1 = [&V1_PREFIX[..], seed].concat();
        let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&v1)
            .map_err(|_| "the new key doesn't parse".to_string())?;
        let same = Ed25519KeyPair::from_pkcs8(v2.as_ref())
            .is_ok_and(|k| k.public_key().as_ref() == key.public_key().as_ref());
        if !same {
            return Err("the new key's two forms disagree".into());
        }
        Ok((Self(key), STANDARD.encode(&v1)))
    }

    /// The key a secret holds (base64 PKCS#8). The error never quotes the secret.
    pub fn from_base64(text: &str) -> Result<Self, String> {
        let document = STANDARD
            .decode(text.trim())
            .map_err(|_| "the signing key is not base64".to_string())?;
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(&document)
            .map(Self)
            .map_err(|_| "the signing key is not an Ed25519 PKCS#8 key".to_string())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey::try_from(self.0.public_key().as_ref()).expect("Ed25519 public keys are 32 bytes")
    }

    /// The public key in base64, as release-keys.txt lists it.
    pub fn public_base64(&self) -> String {
        STANDARD.encode(self.public())
    }

    /// A signature line for SHA256SUMS.sig.
    pub fn sign(&self, tag: &str, sums: &[u8]) -> String {
        STANDARD.encode(self.0.sign(&message(tag, sums)).as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(tag: &str) -> (SigningKey, String, String) {
        let (key, _) = SigningKey::generate().unwrap();
        let sums = sums_file(&[
            ("horizon-linux-x86_64.tar.gz".into(), sha256_hex(b"linux")),
            ("horizon-windows-x64.zip".into(), sha256_hex(b"windows")),
        ]);
        let signature = key.sign(tag, sums.as_bytes()) + "\n";
        (key, sums, signature)
    }

    #[test]
    fn the_built_in_keys_file_parses() {
        assert!(parse_keys(KEYS).is_ok());
    }

    #[test]
    fn releases_from_before_signing_keep_their_pinned_digests() {
        let mut seen = std::collections::HashSet::new();
        for line in LEGACY.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(parts.len(), 3, "{line}");
            assert_eq!(parts[2].len(), 64, "{line}");
            assert!(
                parts[2]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "{line}"
            );
            assert!(seen.insert((parts[0], parts[1])), "listed twice: {line}");
        }
        if let Some((tag, asset, hex)) = legacy_entries(LEGACY).next() {
            assert_eq!(legacy_digest(tag, asset), Some(hex));
            assert_eq!(legacy_digest(tag, "not-an-asset.zip"), None);
        }
        assert_eq!(
            legacy_digest("v2099.01.01", "horizon-windows-x64.zip"),
            None
        );
    }

    #[test]
    fn a_signed_release_gives_its_digests() {
        let (key, sums, signature) = signed("v2026.10.19");
        let digest = signed_digest(
            "v2026.10.19",
            "horizon-windows-x64.zip",
            Some(sums.as_bytes()),
            Some(&signature),
            &[key.public()],
        );
        assert_eq!(digest, Ok(sha256_hex(b"windows")));
        assert!(check("x", &sha256_hex(b"windows"), &sha256_hex(b"windows")).is_ok());
        assert!(check("x", &sha256_hex(b"windows"), &sha256_hex(b"linux")).is_err());
    }

    /// The release channel refuses everything signed for the nightly channel, and the other way
    /// round: one key signs both, and only the message's channel keeps them apart.
    #[test]
    fn nightly_and_release_signatures_never_pass_for_each_other() {
        let (nightly, sums, signature) = signed(NIGHTLY);
        let keys = [nightly.public()];
        let asset = "horizon-windows-x64.zip";
        let digest = nightly_digest(asset, Some(sums.as_bytes()), Some(&signature), &keys);
        assert_eq!(digest, Ok(sha256_hex(b"windows")));
        // Served under a release's tag: the message names the nightly channel, not v0.2.81.
        for tag in ["v0.2.81", "v2026.10.28", "0.2.81"] {
            let as_release =
                signed_digest(tag, asset, Some(sums.as_bytes()), Some(&signature), &keys);
            assert!(as_release.is_err(), "{tag} took a nightly's signature");
        }
        // Asked for by a nightly tag on the release channel: refused before any signature is read.
        for tag in [NIGHTLY, "nightly-2026-10-27"] {
            let err = signed_digest(tag, asset, Some(sums.as_bytes()), Some(&signature), &keys);
            assert!(err.unwrap_err().contains("Nightly build"), "{tag}");
        }
        // A release's signed list replayed as tonight's nightly.
        let (release, sums, signature) = signed("v0.2.81");
        let keys = [release.public()];
        assert!(nightly_digest(asset, Some(sums.as_bytes()), Some(&signature), &keys).is_err());
        assert!(nightly_digest(asset, Some(sums.as_bytes()), None, &keys).is_err());
        assert!(is_nightly("nightly-2026-10-27") && !is_nightly("v0.2.81"));
    }

    #[test]
    fn a_nightly_is_named_by_its_date_and_commit() {
        let v = nightly_version("Nightly 2026-10-28 · 0c02067", "2026-10-28T20:40:00Z");
        assert_eq!(v, "nightly-2026-10-28-0c02067");
        assert_eq!(
            nightly_version("Something else", "2026-10-29T01:00:00Z"),
            "nightly-2026-10-29"
        );
        assert_eq!(nightly_date(&v), Some("2026-10-28"));
        assert_eq!(nightly_date("nightly-2026-10-27"), Some("2026-10-27"));
        assert_eq!(nightly_date("nightly"), None);
        assert_eq!(nightly_date("v0.2.95"), None);
        assert!(is_nightly(&v));
    }

    #[test]
    fn tampering_is_refused() {
        let (key, sums, signature) = signed("v2026.10.19");
        let keys = [key.public()];
        // Another tag: an old signed list replayed for a new release.
        assert!(verify("v2026.10.20", sums.as_bytes(), &signature, &keys).is_err());
        // A changed checksum.
        let changed = sums.replacen(&sha256_hex(b"linux"), &sha256_hex(b"evil"), 1);
        assert!(verify("v2026.10.19", changed.as_bytes(), &signature, &keys).is_err());
        // Another key.
        let (other, _) = SigningKey::generate().unwrap();
        assert!(
            verify(
                "v2026.10.19",
                sums.as_bytes(),
                &signature,
                &[other.public()]
            )
            .is_err()
        );
        // No signature, no keys, an asset that isn't listed.
        let none = signed_digest("v2026.10.19", "a", None, None, &keys).unwrap_err();
        assert!(none.contains("not signed"), "{none}");
        assert!(verify("v2026.10.19", sums.as_bytes(), &signature, &[]).is_err());
        let missing = signed_digest(
            "v2026.10.19",
            "horizon-macos-arm64.zip",
            Some(sums.as_bytes()),
            Some(&signature),
            &keys,
        );
        assert!(missing.unwrap_err().contains("not in"));
    }

    #[test]
    fn any_line_signed_by_any_trusted_key_passes() {
        // While rotating, releases carry one signature per key and launchers trust one or both.
        let (old, sums, old_line) = signed("v1");
        let (new, _) = SigningKey::generate().unwrap();
        let both = format!("{old_line}{}\n", new.sign("v1", sums.as_bytes()));
        assert!(verify("v1", sums.as_bytes(), &both, &[old.public()]).is_ok());
        assert!(verify("v1", sums.as_bytes(), &both, &[new.public()]).is_ok());
        assert!(verify("v1", sums.as_bytes(), "garbage\n\n", &[old.public()]).is_err());
    }

    #[test]
    fn keys_round_trip_through_their_text_forms() {
        let (key, secret) = SigningKey::generate().unwrap();
        // PKCS#8 v1 (version 0, the seed alone): what OpenSSL 3.0 reads.
        let document = STANDARD.decode(&secret).unwrap();
        assert_eq!((document.len(), document[4]), (48, 0));
        let again = SigningKey::from_base64(&format!("{secret}\n")).unwrap();
        assert_eq!(again.public(), key.public());
        assert_eq!(
            parse_keys(&format!("# comment\n\n{}\n", key.public_base64())),
            Ok(vec![key.public()])
        );
        assert!(parse_keys("AAAA\n").is_err());
        let error = SigningKey::from_base64("bm90IGEga2V5").err().unwrap();
        assert!(!error.contains("bm90"));
    }

    #[test]
    fn checksum_lists_read_both_sha256sum_forms() {
        let hex = sha256_hex(b"x");
        let list =
            parse_sums(&format!("{hex}  a.zip\n{} *b.tar.gz\n", hex.to_uppercase())).unwrap();
        assert_eq!(
            list,
            [("a.zip".into(), hex.clone()), ("b.tar.gz".into(), hex)]
        );
        assert!(parse_sums("nothex  a.zip\n").is_err());
    }
}
