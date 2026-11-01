//! Velocity "modern" player information forwarding for a dedicated server behind a proxy.
//!
//! The proxy authenticates the player and the server (`online-mode=false`) trusts what it forwards. Written
//! from the behaviour of a real Velocity 4.3.1, not from any other project's source: the exchange was recorded
//! with a stand-in backend and is described in docs/velocity-forwarding.md.
//!
//! The server asks on [`CHANNEL`] with one byte, the highest forwarding version it understands. The proxy
//! answers with a 32-byte HMAC-SHA256 signature followed by the signed data, made with the secret both sides
//! share: `version` (VarInt), the player's `address` (string), `uuid` (16 bytes), `name` (string) and the
//! profile `properties` (VarInt count; each a name, a value, a bool and the signature string when it is set).

use horizon_protocol::{buf::Reader, packets::login};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

/// The login plugin channel the proxy answers on.
pub const CHANNEL: &str = "velocity:player_info";

/// The version asked for and the only one accepted: address, UUID, name and properties.
const VERSION: u8 = 1;

/// What the proxy vouches for.
#[derive(Debug, PartialEq)]
pub struct Forwarded {
    pub address: IpAddr,
    pub uuid: u128,
    pub name: String,
    pub properties: Vec<login::Property>,
}

/// The request's data: the highest version this server reads.
pub fn request() -> Vec<u8> {
    vec![VERSION]
}

/// Checks the proxy's answer against the shared `secret` and reads what it forwards.
pub fn verify(secret: &[u8], payload: &[u8]) -> Result<Forwarded, String> {
    if payload.len() < 32 {
        return Err("the forwarded data is too short".into());
    }
    let (signature, data) = payload.split_at(32);
    if !equal(&hmac_sha256(secret, data), signature) {
        return Err("the forwarded data is not signed with this server's secret".into());
    }
    let bad = |e: horizon_protocol::Error| format!("the forwarded data is damaged: {e}");
    let mut r = Reader::new(data);
    let version = r.varint().map_err(bad)?;
    if version != i32::from(VERSION) {
        return Err(format!(
            "the proxy forwarded version {version}, not {VERSION}"
        ));
    }
    let address = r.string(64).map_err(bad)?;
    let address: IpAddr = address
        .parse()
        .map_err(|_| format!("the forwarded address {address:?} is not an IP address"))?;
    let uuid = r.uuid().map_err(bad)?;
    let name = r.string(16).map_err(bad)?;
    let count = r.count(64).map_err(bad)?;
    let mut properties = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.string(64).map_err(bad)?;
        let value = r.string(32767).map_err(bad)?;
        let signature = if r.bool().map_err(bad)? {
            Some(r.string(1024).map_err(bad)?)
        } else {
            None
        };
        properties.push(login::Property {
            name,
            value,
            signature,
        });
    }
    if r.remaining() != 0 {
        return Err("the forwarded data has trailing bytes".into());
    }
    Ok(Forwarded {
        address,
        uuid,
        name,
        properties,
    })
}

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut padded = [0u8; BLOCK];
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new()
        .chain_update(padded.map(|b| b ^ 0x36))
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(padded.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// Compares without stopping at the first difference, so the time says nothing about the signature.
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::buf::Writer;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        assert_eq!(
            hmac_sha256(&[0x0b; 20], b"Hi There").to_vec(),
            hex("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
        );
        assert_eq!(
            hmac_sha256(b"Jefe", b"what do ya want for nothing?").to_vec(),
            hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
        // A key longer than the block is hashed first (RFC 4231 test case 6).
        assert_eq!(
            hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )
            .to_vec(),
            hex("60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54")
        );
    }

    /// What a Velocity 4.3.1 proxy answered to `request()` for a player named Tester, recorded with a
    /// stand-in backend and the throwaway secret below.
    const SECRET: &[u8] = b"ISWncFxltwFE";
    const RECORDED: &str = "189e73f57e7fe3d387de5f252fc0226e8288e5b15f23ee12924b34cf216b7f6901093132372e302e302e31f3d28cb072253cb1baeb2dadd2be89ae0654657374657200";

    #[test]
    fn a_recorded_answer_is_read() {
        let got = verify(SECRET, &hex(RECORDED)).unwrap();
        assert_eq!(got.address, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(got.uuid, 0xf3d28cb0_7225_3cb1_baeb_2dadd2be89ae);
        assert_eq!(got.name, "Tester");
        assert!(got.properties.is_empty());
    }

    #[test]
    fn another_secret_or_a_changed_byte_is_refused() {
        let mut payload = hex(RECORDED);
        assert!(verify(b"not the secret", &payload).is_err());
        *payload.last_mut().unwrap() ^= 1;
        assert!(verify(SECRET, &payload).unwrap_err().contains("not signed"));
        assert!(verify(SECRET, &payload[..10]).is_err());
    }

    /// Signs `data` the way the proxy does, for the cases the recording doesn't cover.
    fn signed(data: &Writer) -> Vec<u8> {
        let mut out = hmac_sha256(SECRET, &data.data).to_vec();
        out.extend_from_slice(&data.data);
        out
    }

    fn body(
        version: i32,
        address: &str,
        name: &str,
        properties: &[(&str, &str, Option<&str>)],
    ) -> Writer {
        let mut w = Writer::new();
        w.varint(version);
        w.string(address);
        w.uuid(7);
        w.string(name);
        w.varint(properties.len() as i32);
        for (key, value, signature) in properties {
            w.string(key);
            w.string(value);
            w.bool(signature.is_some());
            if let Some(signature) = signature {
                w.string(signature);
            }
        }
        w
    }

    #[test]
    fn properties_are_read_with_and_without_a_signature() {
        let w = body(
            1,
            "::1",
            "Alex",
            &[("textures", "abc", Some("sig")), ("extra", "x", None)],
        );
        let got = verify(SECRET, &signed(&w)).unwrap();
        assert_eq!(got.address, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(got.properties.len(), 2);
        assert_eq!(got.properties[0].signature.as_deref(), Some("sig"));
        assert_eq!(got.properties[1].signature, None);
    }

    #[test]
    fn other_versions_and_bad_data_are_refused() {
        assert!(verify(SECRET, &signed(&body(4, "127.0.0.1", "A", &[]))).is_err());
        assert!(verify(SECRET, &signed(&body(1, "localhost", "A", &[]))).is_err());
        assert!(
            verify(
                SECRET,
                &signed(&body(1, "127.0.0.1", "much too long a name", &[]))
            )
            .is_err()
        );
        let mut w = body(1, "127.0.0.1", "A", &[]);
        w.u8(0);
        assert!(
            verify(SECRET, &signed(&w))
                .unwrap_err()
                .contains("trailing")
        );
    }
}
