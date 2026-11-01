//! The release steps as release.yml runs them: `sums`, tools/ci/sign_release.sh (OpenSSL), `verify`.
use horizon_release_sign::{SIGNATURE, SUMS, SigningKey, trusted_keys};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TAG: &str = "v2026.10.19";

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-release-sign-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("horizon-linux-x86_64.tar.gz"), b"linux build").unwrap();
    std::fs::write(dir.join("horizon-windows-x64.zip"), b"windows build").unwrap();
    let sums = horizon_release_sign::sums_file(&[
        (
            "horizon-linux-x86_64.tar.gz".into(),
            horizon_release_sign::sha256_hex(b"linux build"),
        ),
        (
            "horizon-windows-x64.zip".into(),
            horizon_release_sign::sha256_hex(b"windows build"),
        ),
    ]);
    std::fs::write(dir.join(SUMS), sums).unwrap();
    dir
}

fn tool(args: &[&str], paths: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_horizon-release-sign"))
        .args(args)
        .args(paths)
        .output()
        .unwrap()
}

fn verify(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["verify", "--tag", TAG];
    args.extend(extra);
    tool(&args, &[dir])
}

/// An OpenSSL 3 (the runner's), or None on a machine without one (macOS ships LibreSSL).
fn openssl() -> Option<String> {
    [
        "openssl",
        "/opt/homebrew/opt/openssl@3/bin/openssl",
        "/usr/local/opt/openssl@3/bin/openssl",
    ]
    .into_iter()
    .find(|o| {
        Command::new(o)
            .arg("version")
            .output()
            .is_ok_and(|v| v.stdout.starts_with(b"OpenSSL 3"))
    })
    .map(String::from)
}

fn sign_script(dir: &Path, env: &[(&str, &str)]) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/ci/sign_release.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(script)
        .arg(TAG)
        .arg(dir.join("horizon-linux-x86_64.tar.gz"))
        .arg(dir.join("horizon-windows-x64.zip"))
        .env_remove("RELEASE_SIGNING_KEY")
        .env_remove("RELEASE_SIGNING_KEY_NEXT");
    cmd.env("OPENSSL", openssl().unwrap());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn a_signature_made_like_openssls_checks_and_tampering_fails() {
    let dir = folder("lib");
    let (key, _) = SigningKey::generate().unwrap();
    let sums = std::fs::read(dir.join(SUMS)).unwrap();
    std::fs::write(dir.join(SIGNATURE), key.sign(TAG, &sums) + "\n").unwrap();
    assert!(
        verify(&dir, &["--key", &key.public_base64()])
            .status
            .success()
    );
    std::fs::write(dir.join("horizon-windows-x64.zip"), b"tampered").unwrap();
    let out = verify(&dir, &["--key", &key.public_base64()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("doesn't match its signed checksum"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unsigned_releases_go_out_only_until_a_key_is_listed() {
    let dir = folder("unsigned");
    assert_eq!(
        verify(&dir, &["--publishing"]).status.success(),
        trusted_keys().is_empty()
    );
    assert!(!verify(&dir, &[]).status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_openssl_script_signs_what_the_launcher_checks() {
    // The script runs on Linux runners only (the `sign` jobs of launcher.yml and nightly-build.yml).
    // On Windows, Command finds System32's bash (WSL's) before the one on PATH.
    if cfg!(windows) || openssl().is_none() {
        eprintln!("skipped: tools/ci/sign_release.sh is tested on Linux (bash, OpenSSL 3)");
        return;
    }
    // A dry run: a throwaway key, its public key for the check.
    let dir = folder("ephemeral");
    std::fs::remove_file(dir.join(SUMS)).unwrap(); // the script writes its own
    let out = sign_script(&dir, &[("EPHEMERAL", "yes")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let key = std::fs::read_to_string(dir.join("ephemeral.pub")).unwrap();
    assert!(verify(&dir, &["--key", key.trim()]).status.success());
    std::fs::remove_dir_all(dir).unwrap();

    // A release: both secrets (rotating), as keygen makes them; each signature line checks alone.
    let dir = folder("secret");
    let (old, old_secret) = SigningKey::generate().unwrap();
    let (new, new_secret) = SigningKey::generate().unwrap();
    let out = sign_script(
        &dir,
        &[
            ("RELEASE_SIGNING_KEY", &old_secret),
            ("RELEASE_SIGNING_KEY_NEXT", &new_secret),
        ],
    );
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{printed}");
    assert!(!printed.contains(&old_secret) && !printed.contains(&new_secret));
    assert_eq!(
        std::fs::read_to_string(dir.join(SIGNATURE))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert!(
        verify(&dir, &["--key", &old.public_base64()])
            .status
            .success()
    );
    assert!(
        verify(&dir, &["--key", &new.public_base64()])
            .status
            .success()
    );
    let (other, _) = SigningKey::generate().unwrap();
    assert!(
        !verify(&dir, &["--key", &other.public_base64()])
            .status
            .success()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn keygen_writes_the_secret_to_a_pipe_and_the_public_key_to_a_file() {
    let dir = folder("keygen");
    let public = dir.join("key.pub");
    let out = tool(&["keygen", "--public-out"], &[&public]);
    assert!(out.status.success());
    let key = SigningKey::from_base64(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&public).unwrap().trim(),
        key.public_base64()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
