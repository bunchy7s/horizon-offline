//! TLS for vanilla's management protocol (`management-server-tls-*`), on by default as in vanilla, set up as
//! vanilla's `JsonRpcSslContextProvider` sets it up: a PKCS12 keystore holding the server's certificate and
//! key, its password from the `MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD` environment variable, else
//! `management-server-tls-keystore-password`. Vanilla won't start with TLS on and no keystore; Horizon makes
//! one instead (a self-signed certificate in `.horizon/`), so turning the protocol on is enough, and logs the
//! certificate's fingerprint for clients to pin.

use horizon_server_files::properties::{SystemEntropy, generate_secret_key};
use p12_keystore::{Certificate, KeyStore, KeyStoreEntry, PrivateKeyChain};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

pub const PASSWORD_VARIABLE: &str = "MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD";

/// Where the generated keystore goes, relative to the server's folder.
pub const GENERATED: &str = ".horizon/management-keystore.p12";

/// `JsonRpcSslContextProvider.printInstructions`, with the Horizon way: server.toml's `[properties]`.
pub const INSTRUCTIONS: [&str; 5] = [
    "To use TLS for the management server, please follow these steps:",
    "1. Set the server property 'management-server-tls-enabled' to 'true' to enable TLS",
    "2. Create a keystore file of type PKCS12 containing your server certificate and private key",
    "3. Set the server property 'management-server-tls-keystore' to the path of your keystore file",
    "4. Set the keystore password via the environment variable 'MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD', or \
     server property 'management-server-tls-keystore-password'",
];

/// The server's TLS session settings, and its certificate's SHA-256 fingerprint.
pub struct Tls {
    pub config: Arc<rustls::ServerConfig>,
    pub fingerprint: String,
}

/// `getKeystorePassword`: the environment's, else server.properties'.
pub fn password(from_properties: &str) -> String {
    std::env::var(PASSWORD_VARIABLE).unwrap_or_else(|_| from_properties.to_owned())
}

/// `createFrom`: the keystore at `path` (relative to the server's folder, vanilla's working directory).
pub fn load(dir: &Path, path: &str, password: &str) -> Result<Tls, String> {
    if path.is_empty() {
        return Err("TLS is enabled but keystore is not configured".into());
    }
    let file = dir.join(path);
    if !file.is_file() {
        return Err(format!(
            "Supplied keystore is not a file or does not exist: '{path}'"
        ));
    }
    let data = std::fs::read(&file).map_err(|e| format!("'{path}': {e}"))?;
    let store = KeyStore::from_pkcs12(&data, password)
        .map_err(|e| format!("Couldn't open the keystore '{path}' (wrong password?): {e}"))?;
    let Some((_, chain)) = store.private_key_chain() else {
        return Err(format!(
            "The keystore '{path}' holds no private key with its certificate"
        ));
    };
    let certificates: Vec<CertificateDer<'static>> = chain
        .chain()
        .iter()
        .map(|c| CertificateDer::from(c.as_der().to_vec()))
        .collect();
    let Some(leaf) = certificates.first() else {
        return Err(format!(
            "The keystore '{path}' has no certificate for its key"
        ));
    };
    let fingerprint = fingerprint(leaf);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(chain.key().to_vec()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .map_err(|e| format!("The keystore '{path}': {e}"))?;
    Ok(Tls {
        config: Arc::new(config),
        fingerprint,
    })
}

/// A keystore with a new self-signed certificate for localhost and `host`, written to [`GENERATED`] for its
/// owner only; its password.
pub fn generate(dir: &Path, host: &str) -> Result<String, String> {
    let mut names: Vec<String> = ["localhost", "127.0.0.1", "::1"].map(String::from).into();
    if !host.is_empty() && !matches!(host, "0.0.0.0" | "::") && !names.iter().any(|n| n == host) {
        names.push(host.to_owned());
    }
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(names).map_err(|e| e.to_string())?;
    let certificate = Certificate::from_der(cert.der()).map_err(|e| e.to_string())?;
    let id = Sha256::digest(cert.der());
    let mut store = KeyStore::new();
    store.add_entry(
        "horizon",
        KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(
            signing_key.serialize_der(),
            &id[..20],
            [certificate],
        )),
    );
    let password = generate_secret_key(&mut SystemEntropy::default());
    let data = store.writer(&password).write().map_err(|e| e.to_string())?;
    let file = dir.join(GENERATED);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::api::write_private(&file, &data).map_err(|e| format!("{GENERATED}: {e}"))?;
    Ok(password)
}

/// SHA-256 of a certificate, as `keytool -list` prints it.
fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_keystore_opens_as_vanillas_would() {
        let dir =
            std::env::temp_dir().join(format!("horizon-management-tls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let password = generate(&dir, "example.org").unwrap();
        assert_eq!(password.len(), 40);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(GENERATED))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let tls = load(&dir, GENERATED, &password).unwrap();
        assert_eq!(tls.fingerprint.len(), 32 * 3 - 1);
        assert!(load(&dir, GENERATED, "wrong").is_err());
        assert_eq!(
            load(&dir, "", &password).err().unwrap(),
            "TLS is enabled but keystore is not configured"
        );
        assert_eq!(
            load(&dir, "nope.p12", &password).err().unwrap(),
            "Supplied keystore is not a file or does not exist: 'nope.p12'"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
