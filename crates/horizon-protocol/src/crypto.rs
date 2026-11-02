//! Connection encryption (Crypt.getCipher: AES/CFB8/NoPadding with the shared
//! secret as key and IV) and MD5 name-based UUIDs for offline-mode profiles
//! (UUIDUtil.createOfflinePlayerUUID), the online-mode server hash
//! (Crypt.digestData) and the shared secret. Only the AES forward cipher is
//! needed: CFB8 encrypts the shift register in both directions.
use sha1::{Digest, Sha1};

mod blocks;
pub use blocks::Blocks as Aes128;
use blocks::LANES;

/// AES/CFB8 stream state for one direction of a connection.
#[derive(Clone)]
pub struct Cfb8 {
    aes: Aes128,
    /// The last 16 ciphertext bytes (the IV before any).
    register: [u8; 16],
}
impl Cfb8 {
    /// Minecraft uses the 16-byte shared secret as both key and IV.
    pub fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
        Self {
            aes: Aes128::new(key),
            register: *iv,
        }
    }
    /// Each byte's keystream is the encrypted register, which holds the
    /// ciphertext before it: encryption goes one byte at a time.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        self.aes.cfb8_encrypt(&mut self.register, data);
    }
    /// Decryption knows every ciphertext byte up front, so the registers of
    /// [`LANES`] bytes in a row are known together and encrypted at once.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        for run in data.chunks_mut(LANES) {
            // The register, then this run's ciphertext: byte i's register
            // is window[i..i + 16].
            let mut window = [0u8; 16 + LANES];
            window[..16].copy_from_slice(&self.register);
            window[16..16 + run.len()].copy_from_slice(run);
            let mut blocks: [[u8; 16]; LANES] =
                std::array::from_fn(|i| window[i..i + 16].try_into().unwrap());
            self.aes.encrypt_lanes(&mut blocks);
            for (b, k) in run.iter_mut().zip(&blocks) {
                *b ^= k[0];
            }
            self.register
                .copy_from_slice(&window[run.len()..run.len() + 16]);
        }
    }
}

/// Fill `out` with the operating system's cryptographic randomness.
pub fn random_bytes(out: &mut [u8]) -> crate::Result<()> {
    getrandom::fill(out).map_err(|e| crate::Error::new(format!("No secure random source: {e}")))
}

/// Crypt.generateSecretKey: a random AES-128 key, the connection's shared secret.
pub fn generate_secret() -> crate::Result<[u8; 16]> {
    let mut key = [0u8; 16];
    random_bytes(&mut key)?;
    Ok(key)
}

/// `new BigInteger(digest).toString(16)`: the digest read as a signed
/// two's-complement number, in lowercase hex without leading zeros.
pub fn java_hex_digest(digest: &[u8]) -> String {
    let negative = digest.first().is_some_and(|b| b & 0x80 != 0);
    let mut magnitude = digest.to_vec();
    if negative {
        // Two's complement: invert and add one.
        let mut carry = true;
        for b in magnitude.iter_mut().rev() {
            *b = !*b;
            if carry {
                let (v, c) = b.overflowing_add(1);
                *b = v;
                carry = c;
            }
        }
    }
    let hex: String = magnitude.iter().map(|b| format!("{b:02x}")).collect();
    let hex = hex.trim_start_matches('0');
    let hex = if hex.is_empty() { "0" } else { hex };
    if negative {
        format!("-{hex}")
    } else {
        hex.to_owned()
    }
}

/// The server id sent to the session service when joining an online-mode
/// server: SHA-1 over the server id (ISO-8859-1), the shared secret and the
/// server's encoded public key, as a signed hex number.
pub fn server_hash(server_id: &str, secret: &[u8], public_key: &[u8]) -> String {
    let mut sha = Sha1::new();
    // ISO-8859-1: characters above U+00FF become '?' like String.getBytes.
    let latin1: Vec<u8> = server_id
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    sha.update(&latin1);
    sha.update(secret);
    sha.update(public_key);
    java_hex_digest(&sha.finalize())
}

/// MD5 (RFC 1321).
pub fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301u32,
        0xefcd_ab89u32,
        0x98ba_dcfeu32,
        0x1032_5476u32,
    );
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// UUID.nameUUIDFromBytes("OfflinePlayer:" + name): MD5, version 3, IETF variant.
pub fn offline_uuid(name: &str) -> u128 {
    let mut h = md5(format!("OfflinePlayer:{name}").as_bytes());
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    u128::from_be_bytes(h)
}
/// The canonical 8-4-4-4-12 form.
pub fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn aes128_fips197_vector() {
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let mut block: [u8; 16] = hex("00112233445566778899aabbccddeeff").try_into().unwrap();
        Aes128::new(&key).encrypt_block(&mut block);
        assert_eq!(block.to_vec(), hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
    }

    #[test]
    fn cfb8_nist_sp800_38a_vector() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let plain = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        let expected = hex("3b79424c9c0dd436bace9e0ed4586a4f32b9");
        let mut data = plain.clone();
        // Split across calls: the stream state carries over.
        let mut enc = Cfb8::new(&key, &iv);
        let (a, b) = data.split_at_mut(5);
        enc.encrypt(a);
        enc.encrypt(b);
        assert_eq!(data, expected);
        let mut dec = Cfb8::new(&key, &iv);
        dec.decrypt(&mut data);
        assert_eq!(data, plain);
    }

    /// Bytes from a fixed xorshift: the same every run.
    fn noise(n: usize, mut seed: u64) -> Vec<u8> {
        (0..n)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect()
    }

    #[test]
    fn lanes_match_single_blocks_and_the_aes_crate() {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let key: [u8; 16] = noise(16, 1).try_into().unwrap();
        let aes = Aes128::new(&key);
        let reference = aes::Aes128::new(&key.into());
        let mut lanes: [[u8; 16]; LANES] =
            std::array::from_fn(|i| noise(16, 7 + i as u64).try_into().unwrap());
        let inputs = lanes;
        aes.encrypt_lanes(&mut lanes);
        for (input, out) in inputs.iter().zip(&lanes) {
            let mut one = *input;
            aes.encrypt_block(&mut one);
            assert_eq!(&one, out);
            let mut expected = aes::Block::from(*input);
            reference.encrypt_block(&mut expected);
            assert_eq!(out[..], expected[..]);
        }
    }

    /// Decryption in runs of [`LANES`] gives what the one-byte-at-a-time
    /// definition gives, however the stream is cut into reads.
    #[test]
    fn cfb8_decrypts_like_the_byte_by_byte_definition() {
        let key: [u8; 16] = noise(16, 3).try_into().unwrap();
        let iv: [u8; 16] = noise(16, 4).try_into().unwrap();
        let plain = noise(5000, 5);
        let mut cipher = plain.clone();
        Cfb8::new(&key, &iv).encrypt(&mut cipher);
        // The definition: P[i] = C[i] ^ E(register)[0], register += C[i].
        let aes = Aes128::new(&key);
        let mut register = iv;
        let expected: Vec<u8> = cipher
            .iter()
            .map(|&c| {
                let mut block = register;
                aes.encrypt_block(&mut block);
                register.copy_within(1.., 0);
                register[15] = c;
                c ^ block[0]
            })
            .collect();
        assert_eq!(expected, plain);
        for cuts in [
            &[5000][..],
            &[1, 7, 8, 9, 15, 16, 17, 1000, 3927],
            &[3, 4997],
        ] {
            let mut data = cipher.clone();
            let mut dec = Cfb8::new(&key, &iv);
            let mut rest = &mut data[..];
            for &n in cuts {
                let (head, tail) = rest.split_at_mut(n);
                dec.decrypt(head);
                rest = tail;
            }
            assert!(rest.is_empty());
            assert_eq!(data, plain, "cuts {cuts:?}");
        }
    }

    /// The digests of single names published with the protocol docs.
    #[test]
    fn server_hash_vectors() {
        let hash = |name: &str| java_hex_digest(&Sha1::digest(name.as_bytes()));
        assert_eq!(hash("Notch"), "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48");
        assert_eq!(hash("jeb_"), "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
        assert_eq!(hash("simon"), "88e16a1019277b15d58faf0541e11910eb756f6");
        // The three inputs are hashed in order: one update equals three.
        assert_eq!(server_hash("", b"Not", b"ch"), hash("Notch"));
        assert_eq!(java_hex_digest(&[0; 4]), "0");
        assert_eq!(java_hex_digest(&[0xff; 2]), "-1");
        assert_eq!(java_hex_digest(&[0x80, 0]), "-8000");
    }

    #[test]
    fn secrets_are_random() {
        let (a, b) = (generate_secret().unwrap(), generate_secret().unwrap());
        assert_ne!(a, b);
    }

    #[test]
    fn md5_and_offline_uuids() {
        assert_eq!(md5(b"").to_vec(), hex("d41d8cd98f00b204e9800998ecf8427e"));
        assert_eq!(
            md5(b"abc").to_vec(),
            hex("900150983cd24fb0d6963f7d28e17f72")
        );
        assert_eq!(
            md5(&[b'a'; 100]).to_vec(),
            hex("36a92cc94a9e0fa21f625f8bfb007adf")
        );
        assert_eq!(
            uuid_string(offline_uuid("Notch")),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }
}
