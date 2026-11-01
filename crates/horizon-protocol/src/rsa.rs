//! RSA for the online-mode login: the server's X.509 public key
//! (Crypt.byteToPublicKey) and PKCS#1 v1.5 encryption of the shared secret
//! and challenge (Crypt.encryptUsingKey, Java's "RSA" cipher =
//! RSA/ECB/PKCS1Padding), and on the server's side the key pair it makes at
//! startup (Crypt.generateKeyPair: 1024 bits) and the decryption
//! (Crypt.decryptUsingKey); and the PKCS#1 v1.5 signature checks of secure
//! chat (SHA256withRSA for messages, SHA1withRSA for the services' profile
//! key signatures). Modular exponentiation uses Montgomery multiplication on
//! 32-bit limbs; RSA moduli are always odd.
use crate::{Error, Result};

/// A non-negative integer, little-endian 32-bit limbs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_be_bytes(bytes: &[u8], limbs: usize) -> Self {
        let mut v = vec![0u32; limbs];
        for (i, b) in bytes.iter().rev().enumerate() {
            if *b != 0 {
                v[i / 4] |= u32::from(*b) << (8 * (i % 4));
            }
        }
        Self(v)
    }
    fn to_be_bytes(&self, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        for i in 0..len.min(self.0.len() * 4) {
            out[len - 1 - i] = (self.0[i / 4] >> (8 * (i % 4))) as u8;
        }
        out
    }
    fn bits(&self) -> usize {
        self.0
            .iter()
            .rposition(|&l| l != 0)
            .map_or(0, |i| i * 32 + 32 - self.0[i].leading_zeros() as usize)
    }
    fn bit(&self, i: usize) -> bool {
        self.0[i / 32] >> (i % 32) & 1 == 1
    }
    fn ge(&self, other: &Self) -> bool {
        for (a, b) in self.0.iter().rev().zip(other.0.iter().rev()) {
            if a != b {
                return a > b;
            }
        }
        true
    }
    /// self · other, as many limbs as both together.
    fn mul(&self, other: &Self) -> Self {
        let mut out = vec![0u32; self.0.len() + other.0.len()];
        for (i, &a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.0.iter().enumerate() {
                let t = u64::from(out[i + j]) + u64::from(a) * u64::from(b) + carry;
                out[i + j] = t as u32;
                carry = t >> 32;
            }
            out[i + other.0.len()] = carry as u32;
        }
        Self(out)
    }
    /// self · m + a, one limb longer.
    fn mul_small_add(&self, m: u32, a: u32) -> Self {
        let mut out = Vec::with_capacity(self.0.len() + 1);
        let mut carry = u64::from(a);
        for &l in &self.0 {
            let t = u64::from(l) * u64::from(m) + carry;
            out.push(t as u32);
            carry = t >> 32;
        }
        out.push(carry as u32);
        Self(out)
    }
    /// (self / d, self mod d).
    fn div_small(&self, d: u32) -> (Self, u32) {
        let mut out = vec![0u32; self.0.len()];
        let mut rem = 0u64;
        for i in (0..self.0.len()).rev() {
            let t = rem << 32 | u64::from(self.0[i]);
            out[i] = (t / u64::from(d)) as u32;
            rem = t % u64::from(d);
        }
        (Self(out), rem as u32)
    }
    fn is_one(&self) -> bool {
        self.0.first() == Some(&1) && self.0[1..].iter().all(|&l| l == 0)
    }
    fn sub_assign(&mut self, other: &Self) {
        let mut borrow = 0i64;
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            let d = i64::from(*a) - i64::from(*b) - borrow;
            borrow = i64::from(d < 0);
            *a = d.rem_euclid(1 << 32) as u32;
        }
    }
}

/// Montgomery arithmetic modulo an odd `n`.
struct Mont {
    n: Big,
    /// -n⁻¹ mod 2³².
    n0: u32,
    /// R² mod n, R = 2^(32·limbs).
    r2: Big,
}

impl Mont {
    fn new(n: Big) -> Self {
        let k = n.0.len();
        // Newton's iteration for n⁻¹ mod 2³².
        let mut inv = 1u32;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(n.0[0].wrapping_mul(inv)));
        }
        // R² mod n by doubling 1 2·32·k times.
        let mut r2 = Big(vec![0; k]);
        r2.0[0] = 1;
        for _ in 0..64 * k {
            let carry = r2.0[k - 1] >> 31;
            for i in (1..k).rev() {
                r2.0[i] = r2.0[i] << 1 | r2.0[i - 1] >> 31;
            }
            r2.0[0] <<= 1;
            if carry == 1 || r2.ge(&n) {
                r2.sub_assign(&n);
            }
        }
        Self {
            n0: inv.wrapping_neg(),
            n,
            r2,
        }
    }
    /// a·b·R⁻¹ mod n (CIOS).
    fn mul(&self, a: &Big, b: &Big) -> Big {
        let k = self.n.0.len();
        let mut t = vec![0u32; k + 2];
        for i in 0..k {
            let mut carry = 0u64;
            for j in 0..k {
                let s = u64::from(t[j]) + u64::from(a.0[j]) * u64::from(b.0[i]) + carry;
                t[j] = s as u32;
                carry = s >> 32;
            }
            let s = u64::from(t[k]) + carry;
            t[k] = s as u32;
            t[k + 1] = (s >> 32) as u32;
            let m = t[0].wrapping_mul(self.n0);
            let mut carry = (u64::from(t[0]) + u64::from(m) * u64::from(self.n.0[0])) >> 32;
            for j in 1..k {
                let s = u64::from(t[j]) + u64::from(m) * u64::from(self.n.0[j]) + carry;
                t[j - 1] = s as u32;
                carry = s >> 32;
            }
            let s = u64::from(t[k]) + carry;
            t[k - 1] = s as u32;
            t[k] = t[k + 1] + (s >> 32) as u32;
        }
        let overflow = t[k] != 0;
        let mut r = Big(t[..k].to_vec());
        if overflow || r.ge(&self.n) {
            r.sub_assign(&self.n);
        }
        r
    }
    /// base^exp mod n, base < n.
    fn pow(&self, base: &Big, exp: &Big) -> Big {
        let k = self.n.0.len();
        let mut one = Big(vec![0; k]);
        one.0[0] = 1;
        let b = self.mul(base, &self.r2);
        let mut x = self.mul(&one, &self.r2);
        for i in (0..exp.bits()).rev() {
            x = self.mul(&x, &x);
            if exp.bit(i) {
                x = self.mul(&x, &b);
            }
        }
        self.mul(&x, &one)
    }
}

/// A minimal DER reader: (tag, contents) of the element at the front.
fn der(input: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let bad = || Error::new("Malformed public key");
    let (&tag, rest) = input.split_first().ok_or_else(bad)?;
    let (&first, mut rest) = rest.split_first().ok_or_else(bad)?;
    let len = if first < 0x80 {
        usize::from(first)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return Err(bad());
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |l, b| l << 8 | usize::from(*b));
        rest = &rest[n..];
        len
    };
    if rest.len() < len {
        return Err(bad());
    }
    Ok((tag, &rest[..len], &rest[len..]))
}

/// The DER of OID 1.2.840.113549.1.1.1 (rsaEncryption).
const RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RsaPublicKey {
    /// Modulus and exponent, big-endian without leading zeros.
    pub modulus: Vec<u8>,
    pub exponent: Vec<u8>,
}

fn strip(int: &[u8]) -> Vec<u8> {
    let start = int.iter().position(|&b| b != 0).unwrap_or(int.len());
    int[start..].to_vec()
}

impl RsaPublicKey {
    /// X509EncodedKeySpec: SubjectPublicKeyInfo { AlgorithmIdentifier, BIT STRING RSAPublicKey }.
    pub fn from_der(spki: &[u8]) -> Result<Self> {
        let bad = || Error::new("Malformed public key");
        let (tag, body, _) = der(spki)?;
        if tag != 0x30 {
            return Err(bad());
        }
        let (tag, algorithm, rest) = der(body)?;
        let (oid_tag, oid, _) = der(algorithm)?;
        if tag != 0x30 || oid_tag != 0x06 || oid != RSA_ENCRYPTION {
            return Err(Error::new("Not an RSA public key"));
        }
        let (tag, bits, _) = der(rest)?;
        if tag != 0x03 || bits.first() != Some(&0) {
            return Err(bad());
        }
        let (tag, key, _) = der(&bits[1..])?;
        if tag != 0x30 {
            return Err(bad());
        }
        let (n_tag, n, rest) = der(key)?;
        let (e_tag, e, _) = der(rest)?;
        if n_tag != 0x02 || e_tag != 0x02 {
            return Err(bad());
        }
        let key = Self {
            modulus: strip(n),
            exponent: strip(e),
        };
        if key.modulus.len() < 64 || key.modulus.last().is_none_or(|b| b & 1 == 0) {
            return Err(bad());
        }
        Ok(key)
    }
    /// Length of the modulus in bytes (and of every ciphertext).
    pub fn size(&self) -> usize {
        self.modulus.len()
    }
    /// The raw RSA operation m^e mod n on a `size()`-byte block.
    fn apply(&self, block: &[u8], exponent: &[u8]) -> Vec<u8> {
        let limbs = self.size().div_ceil(4);
        let mont = Mont::new(Big::from_be_bytes(&self.modulus, limbs));
        let m = Big::from_be_bytes(block, limbs);
        mont.pow(
            &m,
            &Big::from_be_bytes(exponent, limbs.max(exponent.len().div_ceil(4))),
        )
        .to_be_bytes(self.size())
    }
    /// RSAES-PKCS1-v1_5 (RFC 8017 7.2.1) with random non-zero padding.
    pub fn encrypt(&self, message: &[u8]) -> Result<Vec<u8>> {
        let k = self.size();
        if message.len() + 11 > k {
            return Err(Error::new("Message too long for the RSA key"));
        }
        let mut block = vec![0u8; k];
        block[1] = 2;
        let padding = &mut block[2..k - message.len() - 1];
        crate::crypto::random_bytes(padding)?;
        for b in padding.iter_mut() {
            while *b == 0 {
                let mut one = [0u8];
                crate::crypto::random_bytes(&mut one)?;
                *b = one[0];
            }
        }
        block[k - message.len()..].copy_from_slice(message);
        Ok(self.apply(&block, &self.exponent))
    }
}

/// The digest a PKCS#1 v1.5 signature covers: Java's `SHA1withRSA` and
/// `SHA256withRSA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureHash {
    Sha1,
    Sha256,
}

impl SignatureHash {
    /// The DER DigestInfo before the digest (RFC 8017 9.2, note 1).
    fn prefix(self) -> &'static [u8] {
        match self {
            Self::Sha1 => &[
                0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
                0x14,
            ],
            Self::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x01, 0x05, 0x00, 0x04, 0x20,
            ],
        }
    }
    fn digest(self, message: &[u8]) -> Vec<u8> {
        use sha1::Digest;
        match self {
            Self::Sha1 => sha1::Sha1::digest(message).to_vec(),
            Self::Sha256 => sha2::Sha256::digest(message).to_vec(),
        }
    }
}

impl RsaPublicKey {
    /// `Signature.verify` for RSASSA-PKCS1-v1_5 (RFC 8017 8.2.2): the
    /// signature is exactly the modulus' length and below it, and opens to
    /// `00 01 FF.. 00 DigestInfo`. Anything else is a failed check, as
    /// SignatureValidator reports Java's exceptions.
    pub fn verify(&self, hash: SignatureHash, message: &[u8], signature: &[u8]) -> bool {
        let k = self.size();
        // Equal lengths compare as big-endian numbers.
        if signature.len() != k || signature >= self.modulus.as_slice() {
            return false;
        }
        emsa_pkcs1(hash, message, k)
            .is_some_and(|expected| self.apply(signature, &self.exponent) == expected)
    }
}

/// EMSA-PKCS1-v1_5 (RFC 8017 9.2): `00 01 FF.. 00 DigestInfo` for a
/// `k`-byte modulus; None when the modulus is too short for the digest.
fn emsa_pkcs1(hash: SignatureHash, message: &[u8], k: usize) -> Option<Vec<u8>> {
    let (prefix, digest) = (hash.prefix(), hash.digest(message));
    let t = prefix.len() + digest.len();
    if k < t + 11 {
        return None;
    }
    let mut block = vec![0xff; k];
    block[0] = 0;
    block[1] = 1;
    block[k - t - 1] = 0;
    block[k - t..k - digest.len()].copy_from_slice(prefix);
    block[k - digest.len()..].copy_from_slice(&digest);
    Some(block)
}

/// Small odd primes for sieving prime candidates.
const SMALL_PRIMES: [u32; 53] = [
    3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
    101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179, 181, 191, 193,
    197, 199, 211, 223, 227, 229, 233, 239, 241, 251,
];
/// Miller-Rabin rounds: a composite passes all of them with odds below 2⁻⁸⁰.
const ROUNDS: usize = 40;
const E: u32 = 65537;

/// A random odd number of `limbs` limbs with its two top bits set, so two of
/// them multiply to exactly twice the bits.
fn candidate(limbs: usize) -> Result<Big> {
    let mut bytes = vec![0u8; limbs * 4];
    crate::crypto::random_bytes(&mut bytes)?;
    bytes[0] |= 0xC0;
    bytes[limbs * 4 - 1] |= 1;
    Ok(Big::from_be_bytes(&bytes, limbs))
}

/// Miller-Rabin with random bases.
fn probably_prime(p: &Big) -> Result<bool> {
    if SMALL_PRIMES.iter().any(|&q| p.div_small(q).1 == 0) {
        return Ok(false);
    }
    let k = p.0.len();
    let mont = Mont::new(p.clone());
    let mut p_minus_one = p.clone();
    p_minus_one.0[0] -= 1; // p is odd
    let s = (0..k * 32).find(|&i| p_minus_one.bit(i)).unwrap_or(0);
    // d = (p - 1) >> s
    let mut d = Big(vec![0; k]);
    for i in 0..k * 32 - s {
        if p_minus_one.bit(i + s) {
            d.0[i / 32] |= 1 << (i % 32);
        }
    }
    let mut two = Big(vec![0; k]);
    two.0[0] = 2;
    'rounds: for _ in 0..ROUNDS {
        // A random base in [2, p - 2].
        let mut a = candidate(k)?;
        a.0[k - 1] %= p.0[k - 1];
        if !a.ge(&two) || a.ge(&p_minus_one) {
            continue;
        }
        let mut x = mont.pow(&a, &d);
        if x.is_one() || x == p_minus_one {
            continue;
        }
        for _ in 1..s {
            x = mont.pow(&x, &two);
            if x == p_minus_one {
                continue 'rounds;
            }
            if x.is_one() {
                return Ok(false);
            }
        }
        return Ok(false);
    }
    Ok(true)
}

/// A prime of `limbs` limbs whose p - 1 is prime to E.
fn prime(limbs: usize) -> Result<Big> {
    loop {
        let p = candidate(limbs)?;
        if p.div_small(E).1 != 1 && probably_prime(&p)? {
            return Ok(p);
        }
    }
}

/// DER: a tag and its length, then the contents.
pub(crate) fn der_element(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = contents.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len
            .to_be_bytes()
            .into_iter()
            .skip_while(|&b| b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(contents);
    out
}

/// A DER INTEGER from big-endian magnitude bytes (a leading zero keeps it positive).
pub(crate) fn der_integer(magnitude: &[u8]) -> Vec<u8> {
    let mut v = strip(magnitude);
    if v.first().is_none_or(|b| b & 0x80 != 0) {
        v.insert(0, 0);
    }
    der_element(0x02, &v)
}

/// The server's key pair (Crypt.generateKeyPair), or a player's chat key
/// pair from the services (Crypt.stringToPemRsaPrivateKey).
pub struct RsaPrivateKey {
    pub public: RsaPublicKey,
    /// The private exponent, big-endian.
    exponent: Vec<u8>,
}

impl std::fmt::Debug for RsaPrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RsaPrivateKey")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

impl RsaPrivateKey {
    /// A new key pair with a `bits`-bit modulus (a multiple of 64) and
    /// public exponent 65537, as Java's RSA KeyPairGenerator makes.
    pub fn generate(bits: usize) -> Result<Self> {
        let limbs = bits / 64;
        let p = prime(limbs)?;
        let q = loop {
            let q = prime(limbs)?;
            if q != p {
                break q;
            }
        };
        let n = p.mul(&q);
        // d = (1 + k·φ) / E with k making it whole: k ≡ -φ⁻¹ (mod E).
        let (mut p1, mut q1) = (p, q);
        p1.0[0] -= 1;
        q1.0[0] -= 1;
        let phi = p1.mul(&q1);
        let r = u64::from(phi.div_small(E).1);
        let inverse = (1..u64::from(E))
            .find(|k| k * r % u64::from(E) == 1)
            .ok_or_else(|| Error::new("RSA key generation failed"))?;
        let k = (u64::from(E) - inverse) as u32;
        let (d, rem) = phi.mul_small_add(k, 1).div_small(E);
        if rem != 0 {
            return Err(Error::new("RSA key generation failed"));
        }
        let size = bits / 8;
        Ok(Self {
            public: RsaPublicKey {
                modulus: strip(&n.to_be_bytes(size)),
                exponent: strip(&E.to_be_bytes()),
            },
            exponent: d.to_be_bytes(size),
        })
    }

    /// PublicKey.getEncoded: X.509 SubjectPublicKeyInfo.
    pub fn public_der(&self) -> Vec<u8> {
        let key = der_element(
            0x30,
            &[
                der_integer(&self.public.modulus),
                der_integer(&self.public.exponent),
            ]
            .concat(),
        );
        let algorithm = der_element(
            0x30,
            &[der_element(0x06, RSA_ENCRYPTION), vec![0x05, 0x00]].concat(),
        );
        let bits = der_element(0x03, &[&[0u8][..], &key].concat());
        der_element(0x30, &[algorithm, bits].concat())
    }

    /// Crypt.decryptUsingKey: RSAES-PKCS1-v1_5 decryption (RFC 8017 7.2.2).
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let bad = || Error::new("Decryption error");
        let k = self.public.size();
        // The modulus has no leading zero, so equal lengths compare as numbers.
        if ciphertext.len() != k || ciphertext >= self.public.modulus.as_slice() {
            return Err(bad());
        }
        let block = self.public.apply(ciphertext, &self.exponent);
        if block[0] != 0 || block[1] != 2 {
            return Err(bad());
        }
        let zero = block[2..].iter().position(|&b| b == 0).ok_or_else(bad)? + 2;
        if zero < 10 {
            return Err(bad());
        }
        Ok(block[zero + 1..].to_vec())
    }

    /// PKCS8EncodedKeySpec: PrivateKeyInfo { version, AlgorithmIdentifier,
    /// OCTET STRING RSAPrivateKey }. A bare PKCS#1 RSAPrivateKey is read too:
    /// the services label their PKCS#8 key "RSA PRIVATE KEY".
    pub fn from_der(der_bytes: &[u8]) -> Result<Self> {
        let bad = || Error::new("Malformed private key");
        let (tag, body, _) = der(der_bytes)?;
        if tag != 0x30 {
            return Err(bad());
        }
        let (tag, _version, rest) = der(body)?;
        if tag != 0x02 {
            return Err(bad());
        }
        let key = match der(rest)? {
            (0x30, algorithm, rest) => {
                let (oid_tag, oid, _) = der(algorithm)?;
                if oid_tag != 0x06 || oid != RSA_ENCRYPTION {
                    return Err(Error::new("Not an RSA private key"));
                }
                let (tag, octets, _) = der(rest)?;
                if tag != 0x04 {
                    return Err(bad());
                }
                let (tag, key, _) = der(octets)?;
                if tag != 0x30 {
                    return Err(bad());
                }
                let (_, _version, key) = der(key)?;
                key
            }
            // PKCS#1: the version is followed by the modulus.
            (0x02, ..) => rest,
            _ => return Err(bad()),
        };
        let (n_tag, n, rest) = der(key)?;
        let (e_tag, e, rest) = der(rest)?;
        let (d_tag, d, _) = der(rest)?;
        if n_tag != 0x02 || e_tag != 0x02 || d_tag != 0x02 {
            return Err(bad());
        }
        let public = RsaPublicKey {
            modulus: strip(n),
            exponent: strip(e),
        };
        if public.modulus.len() < 64 || public.modulus.last().is_none_or(|b| b & 1 == 0) {
            return Err(bad());
        }
        let size = public.size();
        let d = strip(d);
        if d.is_empty() || d.len() > size {
            return Err(bad());
        }
        let mut exponent = vec![0u8; size - d.len()];
        exponent.extend_from_slice(&d);
        Ok(Self { public, exponent })
    }

    /// `Signature.sign` for RSASSA-PKCS1-v1_5 (RFC 8017 8.2.1), checked with
    /// the public exponent before it leaves, so a faulty result is never sent.
    pub fn sign(&self, hash: SignatureHash, message: &[u8]) -> Result<Vec<u8>> {
        let k = self.public.size();
        let block = emsa_pkcs1(hash, message, k)
            .ok_or_else(|| Error::new("RSA key too short for the digest"))?;
        let signature = self.public.apply(&block, &self.exponent);
        if !self.public.verify(hash, message, &signature) {
            return Err(Error::new("RSA signature failed its own check"));
        }
        Ok(signature)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn hex(s: &str) -> Vec<u8> {
        let s = if s.len() % 2 == 1 {
            format!("0{s}")
        } else {
            s.to_owned()
        };
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A 1024-bit test key made with `openssl genrsa 1024`.
    pub const TEST_SPKI: &str = "30819f300d06092a864886f70d010101050003818d0030818902818100ab026074f92450a8f726f3895bb11176ca8f065f3100cd4143bac4f7d3674edcff95b41a0525d2365a549c106ec6685881b392177420fd31ff5c144ff11ba24311e751cd2c6b5d1817b6af6686a43c95d61243613005e6945b6093cd95223cec5e8856826da9d2ee1b323362ac1cebeab3659cbc45319a9b540dda816a4798ff0203010001";
    pub const TEST_N: &str = "ab026074f92450a8f726f3895bb11176ca8f065f3100cd4143bac4f7d3674edcff95b41a0525d2365a549c106ec6685881b392177420fd31ff5c144ff11ba24311e751cd2c6b5d1817b6af6686a43c95d61243613005e6945b6093cd95223cec5e8856826da9d2ee1b323362ac1cebeab3659cbc45319a9b540dda816a4798ff";
    pub const TEST_D: &str = "172bcb460e3c0d71d78b3f946d082105a78c99a06c535f7c6860f62f5e45b139e0a7b82f8e630188bbd49b10def1a351dac12a1a6371d4dce16ce893ac67c2247b9ee13e3a840abaf9ea685304558f1a93c4f354193f5713c3e53bd10dd275300db36e764fc081549afbd30cd00c31e6eb726720337a963913d96b50e4d8fd1";
    /// SHA256withRSA signature of "hello" with the test key (openssl pkeyutl), for chat signing.
    pub const TEST_SIG_HELLO: &str = "0bc8716622e44f9bf72432045e91bf1b40a96aae3c402fa86262b83758db1c90d2ed16b9fa86f29af175f2e6eedb864bf8da2667a7ada2bf5433fba36c1951af0c356e45bd5f90fb905b9a08f33a0666c4968ccffbf32fda643d736deec66f82c75fe4a8075c7f2b8a6c0ca630e0ffe4805a1f1d672506b82fc188510fb4ef80";

    #[test]
    fn montgomery_matches_small_numbers() {
        // 4^13 mod 497 = 445 (the classic example), on one limb.
        let mont = Mont::new(Big(vec![497]));
        assert_eq!(mont.pow(&Big(vec![4]), &Big(vec![13])), Big(vec![445]));
        // Two limbs: (2^40 + 5)^3 mod (2^61 - 1).
        let n = (1u128 << 61) - 1;
        let m = Mont::new(Big(vec![n as u32, (n >> 32) as u32]));
        let b = (1u128 << 40) + 5;
        let expected = b * b % n * b % n;
        let r = m.pow(&Big(vec![b as u32, (b >> 32) as u32]), &Big(vec![3, 0]));
        assert_eq!(u128::from(r.0[0]) | u128::from(r.0[1]) << 32, expected);
    }

    #[test]
    fn parses_x509_public_keys() {
        let key = RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap();
        assert_eq!(key.modulus, hex(TEST_N));
        assert_eq!(key.exponent, vec![1, 0, 1]);
        assert_eq!(key.size(), 128);
        assert!(RsaPublicKey::from_der(&hex(TEST_SPKI)[..100]).is_err());
        assert!(RsaPublicKey::from_der(b"not a key").is_err());
    }

    /// Encrypted with the public key, decrypted with the private exponent:
    /// a PKCS#1 v1.5 type 2 block with non-zero padding around the message.
    #[test]
    fn pkcs1_round_trip() {
        let key = RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap();
        let secret = [0x5au8; 16];
        let a = key.encrypt(&secret).unwrap();
        let b = key.encrypt(&secret).unwrap();
        assert_eq!(a.len(), 128);
        assert_ne!(a, b, "padding is random");
        for c in [a, b] {
            let block = key.apply(&c, &hex(TEST_D));
            assert_eq!(&block[..2], &[0, 2]);
            let zero = 2 + block[2..].iter().position(|&b| b == 0).unwrap();
            assert_eq!(zero, 128 - 16 - 1);
            assert_eq!(&block[zero + 1..], &secret);
        }
        assert!(key.encrypt(&[0; 118]).is_err());
    }

    /// A key made here: Java-style DER that parses back, and what its public
    /// half encrypts its private half decrypts.
    #[test]
    fn generated_keys_round_trip() {
        let key = RsaPrivateKey::generate(1024).unwrap();
        assert_eq!(key.public.size(), 128);
        assert_eq!(key.public.exponent, vec![1, 0, 1]);
        let der = key.public_der();
        assert_eq!(
            &der[..3],
            &[0x30, 0x81, 0x9f],
            "a 1024-bit SubjectPublicKeyInfo"
        );
        assert_eq!(RsaPublicKey::from_der(&der).unwrap(), key.public);
        for message in [&[0x5au8; 16][..], &[1, 2, 3, 4], &[]] {
            let c = key.public.encrypt(message).unwrap();
            assert_eq!(key.decrypt(&c).unwrap(), message);
        }
        assert!(key.decrypt(&[0u8; 128]).is_err());
        assert!(key.decrypt(&[1u8; 64]).is_err());
    }

    #[test]
    fn decrypts_with_a_known_key() {
        let key = RsaPrivateKey {
            public: RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap(),
            exponent: hex(TEST_D),
        };
        let c = key.public.encrypt(b"secret").unwrap();
        assert_eq!(key.decrypt(&c).unwrap(), b"secret");
    }

    #[test]
    fn finds_primes() {
        let p = Big(vec![0xFFFF_FFFB]); // 2^32 - 5 is prime
        assert!(probably_prime(&p).unwrap());
        assert!(!probably_prime(&Big(vec![0xFFFF_FFFD])).unwrap()); // 3 · 1431655765
        // 2^61 - 1 (prime) and a Carmichael-free composite 2^61 + 1.
        let m61 = (1u64 << 61) - 1;
        assert!(probably_prime(&Big(vec![m61 as u32, (m61 >> 32) as u32])).unwrap());
        let c = (1u64 << 61) + 1;
        assert!(!probably_prime(&Big(vec![c as u32, (c >> 32) as u32])).unwrap());
    }

    /// An OpenSSL signature checked with the public exponent: the block ends
    /// with SHA-256("hello"), so the modular arithmetic agrees with OpenSSL's.
    #[test]
    fn verifies_an_openssl_signature() {
        let key = RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap();
        let block = key.apply(&hex(TEST_SIG_HELLO), &key.exponent);
        assert_eq!(&block[..2], &[0, 1]);
        assert!(block.ends_with(&hex(
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        )));
        let signature = hex(TEST_SIG_HELLO);
        assert!(key.verify(SignatureHash::Sha256, b"hello", &signature));
        assert!(!key.verify(SignatureHash::Sha256, b"hellO", &signature));
        assert!(!key.verify(SignatureHash::Sha1, b"hello", &signature));
        let mut flipped = signature.clone();
        flipped[40] ^= 1;
        assert!(!key.verify(SignatureHash::Sha256, b"hello", &flipped));
        assert!(!key.verify(SignatureHash::Sha256, b"hello", &signature[1..]));
        assert!(!key.verify(SignatureHash::Sha256, b"hello", &key.modulus));
    }

    /// Signatures made here with the private exponent (EMSA-PKCS1-v1_5 by
    /// hand) check out for both digests.
    #[test]
    fn verifies_both_digests() {
        let key = RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap();
        for hash in [SignatureHash::Sha1, SignatureHash::Sha256] {
            let (prefix, digest) = (hash.prefix(), hash.digest(b"payload"));
            let mut block = vec![0xff; 128];
            block[0] = 0;
            block[1] = 1;
            let t = prefix.len() + digest.len();
            block[127 - t] = 0;
            block[128 - t..128 - digest.len()].copy_from_slice(prefix);
            block[128 - digest.len()..].copy_from_slice(&digest);
            let signature = key.apply(&block, &hex(TEST_D));
            assert!(key.verify(hash, b"payload", &signature));
            assert!(!key.verify(hash, b"payloaD", &signature));
        }
    }

    /// A 2048-bit key (`openssl genrsa 2048`, then `openssl pkcs8 -topk8 -nocrypt
    /// -outform DER`), the size of a player's chat key: its signatures are
    /// MessageSignature's 256 bytes.
    pub const CHAT_KEY_PKCS8: &str = "308204bc020100300d06092a864886f70d0101010500048204a6308204a20201000282010100bebf25d067e367ba25767067c0464701a109a37699f90a49e413cfba58694c4e51c317e43ad90b171f08dae88aabe0f51958871becf2e62e65282fbb256b70247aa0a7f186b044db793c379d1a332ccc5b50832b8564cf31282d3229abc982439fa3d8085824a8705b1d0d2a4323e300f1ce266f82f09282bbb77ebcd02da8d11c5f118141671497c3e08201e26764f681f50c84ab1475d57bfdc3f648431dcf9c2fa9e215002093ca4c33fc6986bc7e5e430ee4fb269e6b4d04bfd6bbd9d50c9ded5d04325d298815c22334a31e156eeb18aecffa2f64899c6d344723f93691531d816b440fd12a6ede94cdcfa2b86dd32077ae1be0e39c456b5d2d06aee9350203010001028201001bd2874a3b010b3df65d6c7f61a70e8fe104a62474488454eb50ed758d944c56472c49059804d50e8a0f11592672f424e40dcae54a9e526f557f7358dd982a7b13acefda35ae6bd8c47a0155a7beebaba22d1ff8dee3e9a9cea5ab01b38638bb32e3a3ecf1a8652e40480045d815b9b48750adc009ae1bfcb8334767f488986d2978982f12a06f9763bf9eebfdeb60f617521b0bce0756ec379cab2a22d00176b058fabca90175860f66a0634a0dd4219eba51c8c43ab3abbd2d2de4466bef6a815d84585f3cd76420bceaacd15be37fd9e957869ca25d0644c50e5ad399c05359b455b885ed135d3a63b1f80705def62a29cab8904f5ae5fd0f8cfcc9f2048902818100e0611a0d0009e190af7a99e9070ddcf127b41b578515888e18c555488567c2b3ebf970695e44fc7755580245b191fda81a8f670475a4048304652fb7ea9003b2b9985fafbac9f59695132bc6d6e35a3503f407ac2e6efa8dd736d617fad4946c80aabdcf06ab67da7a5cc4a0d69d9320964dc618feb2fc2cfb39c2e11bd1295902818100d9a0afc08108ba70c0a46bb82f1d8beb12cfd50a106d5483fbdf17a7919e0e1373a4c05c4287256faf7be8c42f1788ec3dd33abead70c4a67ec30ef930800f1bb34f5a762946e6fa3c7e296356d1a1059a7068d2bdac29ef4bd5cd25203ffc1a5d10a9bb1d15f6c72e5c830ff9fe5d2f445ce3da61693e6353a8d8fca7f7a73d02818040ef18f0e62ff5dd3f4b45bb50554cb9657a13e79216173cb8227379d7519de11fcb4222dc2b2ebd022fc4089348e295818a3a8fe9d39b92a6f4f9884a137f5e7ba3831d81faec5db5aaa4a29891970b7ca7648909af0847801ed87a7c168515996ed01b579cc7234d0207cd5c0ea2036e3bf7b6480f58375444f506dc5bfd91028180276aa12a7eb47400440403b6a261f99ba4169c816c2441307e8813a2b80f2681836a8da4a8300e29c260f0f1c78dbdb0fc29abf42939c984566541eb044bcd70b8f36071125cd32cf393ee2f1de46e03b3cb345127256d9af7e34b7ab3f2a2d8ab8e7f0f72df2d8939ee6b969332ecf1c8b2025ae555c884f8fc1600fcf330090281802985deefe6ccc4ceb42bbb1ce9a2ecacea4974f5ba8090ba33049e36d0c43a7c9be2da95fc0bf2c7fb0ec2ee73ee628bfda786afa68c75fa08d996b6c1674a759f7f1ec67d95206d69c4b28234b55933748b7930feb9818fa6944bdbf0258894164706d498d2cf29e241fe1b99ec94021cd29ed8a528ee5cf4fe6e6ddcbd1d97";

    /// A 1024-bit key from `openssl genrsa` as `openssl pkcs8 -topk8 -nocrypt
    /// -outform DER` writes it, its public half, and `openssl dgst -sha256
    /// -sign` of "chat".
    const PKCS8: &str = "30820275020100300d06092a864886f70d01010105000482025f3082025b02010002818100a483a9a2ce1762edb78b7359b96415e35885db2cebb0615d36ce314623a5c3ddb5fe1dccdb6066c4dfe16125baf73ca48b4658285da6adf1ccf0bd18be0f67e3a1d68e9547fc129b6d3fe74413ed5f987123f556b54a8c3a434a0fd4a02d4599d73fbdbf403bc229ed3c6e45877fbb2a20fd3850f1a19c5adbefb703eac785eb02030100010281803e646cb68100df9516523dea1e6475e4a61305ffb0575165bfbe4d73b35b4fc04b2057fef48848118c72f69c0f9f6a9160fcc9bc6d6024821ade8668d6d412eae70f2c19d1890877c4fe46aebbbf2324ad3f304dfc7bd6ec484befe4730cc72c749a557c0d0bb6a73093ef571e4fc543ff5a5ec146e9799ca98f9a71b6c08831024100d5e071afe69c04174eea3023c16a3226a577a3ae01c0166ce68dac20791c75aec47ee115ab52a86ae0a7860ba87bfb6f37a8754d72feec7073eff4d6485dd2b3024100c4ea63df21d6711e15a431400a84c127acb6a77c22f475fc807be941a6a80fc0edd4bab09f09a157e517580777c2caf832e5fa07375fc8f3f1286e9f9e9ebbe90240609199e37975a2673fcb7ff6f502d5281bf83c207a4bd8144db95429c9656f417d1968da16e4188541708296caf9481afc7f848f76c7e1cc189d14e897fe3cff024043ba4e817774f7511ea66b9e184012e35f7f2f59fdf7d100f8fc257d4421f7d126bc0d3e24653e8e29a87ec98ac27fe72032836da9f912f7fc5128c353de4ac902401a16b013e8717963d6f938b0126db15eecc678ccd0b3b8497aa6528d522f89087112694fd5082b9838ba162de4831589195284a6aa24711fc7d54ec073dfddf1";
    const PKCS8_SPKI: &str = "30819f300d06092a864886f70d010101050003818d0030818902818100a483a9a2ce1762edb78b7359b96415e35885db2cebb0615d36ce314623a5c3ddb5fe1dccdb6066c4dfe16125baf73ca48b4658285da6adf1ccf0bd18be0f67e3a1d68e9547fc129b6d3fe74413ed5f987123f556b54a8c3a434a0fd4a02d4599d73fbdbf403bc229ed3c6e45877fbb2a20fd3850f1a19c5adbefb703eac785eb0203010001";
    const PKCS8_SIG_CHAT: &str = "3a6ed84b109a293d92a53b2120925f3fa33d3b81b686e9a9f0f1f935bd80a263b0912cf0bcd5ac5ca14ffcd3301a23cb2c3d828a092bb49cc8831a3c1f9be674a44a7c906dd8d48e00edcf7c899a75b19e2eea6d94f868507005448692f8c500192268a286988a025c09ad8fa99ffb50de52c2e892310111c22cdb8b3ac4eb8a";

    /// PKCS#1 v1.5 signatures are deterministic, so signing here must give
    /// OpenSSL's bytes exactly.
    #[test]
    fn signs_like_openssl_with_a_pkcs8_key() {
        let key = RsaPrivateKey::from_der(&hex(PKCS8)).unwrap();
        assert_eq!(
            key.public,
            RsaPublicKey::from_der(&hex(PKCS8_SPKI)).unwrap()
        );
        assert_eq!(key.public_der(), hex(PKCS8_SPKI));
        let signature = key.sign(SignatureHash::Sha256, b"chat").unwrap();
        assert_eq!(signature, hex(PKCS8_SIG_CHAT));
        assert!(
            key.public
                .verify(SignatureHash::Sha256, b"chat", &signature)
        );
        assert!(RsaPrivateKey::from_der(&hex(PKCS8)[..200]).is_err());
        assert!(RsaPrivateKey::from_der(&hex(PKCS8_SPKI)).is_err());
        assert!(RsaPrivateKey::from_der(b"not a key").is_err());
    }

    /// The bare PKCS#1 form: version, n, e, d (the CRT fields aren't used).
    #[test]
    fn reads_pkcs1_private_keys() {
        let body = [
            der_integer(&[0]),
            der_integer(&hex(TEST_N)),
            der_integer(&[1, 0, 1]),
            der_integer(&hex(TEST_D)),
        ]
        .concat();
        let key = RsaPrivateKey::from_der(&der_element(0x30, &body)).unwrap();
        assert_eq!(key.public, RsaPublicKey::from_der(&hex(TEST_SPKI)).unwrap());
        let signature = key.sign(SignatureHash::Sha256, b"hello").unwrap();
        assert_eq!(signature, hex(TEST_SIG_HELLO));
    }

    #[test]
    fn generated_keys_sign() {
        let key = RsaPrivateKey::generate(1024).unwrap();
        for hash in [SignatureHash::Sha1, SignatureHash::Sha256] {
            let signature = key.sign(hash, b"message").unwrap();
            assert_eq!(signature.len(), 128);
            assert!(key.public.verify(hash, b"message", &signature));
            assert!(!key.public.verify(hash, b"messagE", &signature));
        }
    }
}
