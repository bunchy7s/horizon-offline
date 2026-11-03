//! AES-128 encryption of several blocks at once, for CFB8.
//!
//! aarch64 builds that have the AES extension at compile time (every Apple Silicon Mac:
//! `aarch64-apple-darwin` targets the M1) use the ARMv8 AES instructions here directly.
//! Everywhere else the RustCrypto `aes` crate (MIT OR Apache-2.0) does it: AES-NI chosen at
//! run time on x86 and x86_64, a constant-time software AES on CPUs without it.
//!
//! [`LANES`] blocks go through at once: the AES instructions take a few cycles each but
//! start one per cycle, so independent blocks overlap. CFB8 decryption has [`LANES`]
//! independent blocks for every [`LANES`] bytes (see `Cfb8::decrypt`).

/// Blocks encrypted per call: 8 fill the ARM pipelines; the `aes` crate pays a fixed
/// cost per call, which 32 blocks spread (measured on an AMD EPYC 7662).
pub use imp::LANES;

/// One expanded AES-128 key.
#[derive(Clone)]
pub struct Blocks(imp::Key);

impl Blocks {
    pub fn new(key: &[u8; 16]) -> Self {
        Self(imp::Key::new(key))
    }
    /// Encrypt one block in place.
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        self.0.encrypt_block(block);
    }
    /// CFB8 encryption: each byte's keystream is the encrypted register,
    /// which then shifts in that byte's ciphertext.
    pub fn cfb8_encrypt(&self, register: &mut [u8; 16], data: &mut [u8]) {
        self.0.cfb8_encrypt(register, data);
    }
    /// Encrypt [`LANES`] blocks in place.
    pub fn encrypt_lanes(&self, blocks: &mut [[u8; 16]; LANES]) {
        self.0.encrypt_lanes(blocks);
    }
}

// The only unsafe code in this crate: the ARMv8 AES intrinsics are `#[target_feature]`
// functions, which Rust lets safe code call only from a function with the same feature.
#[cfg(all(target_arch = "aarch64", target_feature = "aes"))]
#[allow(unsafe_code)]
mod imp {
    use core::arch::aarch64::{
        uint8x16_t, vaeseq_u8, vaesmcq_u8, vcombine_u8, vcreate_u8, vdupq_n_u8, veorq_u8, vextq_u8,
        vgetq_lane_u8, vgetq_lane_u64, vreinterpretq_u64_u8,
    };

    pub const LANES: usize = 8;

    /// FIPS-197's S-box, for the key schedule only (the rounds use AESE).
    const SBOX: [u8; 256] = {
        let mut sbox = [0u8; 256];
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            // p *= 3, q /= 3 in GF(2^8).
            p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1B } else { 0 };
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
            sbox[p as usize] = x ^ 0x63;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        sbox
    };

    /// FIPS-197 key expansion: 11 round keys, in the byte order AESE takes.
    fn round_keys(key: &[u8; 16]) -> [[u8; 16]; 11] {
        let mut w = [[0u8; 4]; 44];
        for i in 0..4 {
            w[i].copy_from_slice(&key[i * 4..i * 4 + 4]);
        }
        let mut rcon = 1u8;
        for i in 4..44 {
            let mut t = w[i - 1];
            if i % 4 == 0 {
                t = [
                    SBOX[t[1] as usize] ^ rcon,
                    SBOX[t[2] as usize],
                    SBOX[t[3] as usize],
                    SBOX[t[0] as usize],
                ];
                rcon = (rcon << 1) ^ if rcon & 0x80 != 0 { 0x1B } else { 0 };
            }
            for j in 0..4 {
                w[i][j] = w[i - 4][j] ^ t[j];
            }
        }
        let mut keys = [[0u8; 16]; 11];
        for (r, k) in keys.iter_mut().enumerate() {
            for c in 0..4 {
                k[c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
            }
        }
        keys
    }

    #[derive(Clone, Copy)]
    pub struct Key([uint8x16_t; 11]);

    impl Key {
        pub fn new(key: &[u8; 16]) -> Self {
            // SAFETY: as in `encrypt_block`.
            Self(round_keys(key).map(|k| unsafe { load(&k) }))
        }
        pub fn encrypt_block(&self, block: &mut [u8; 16]) {
            // SAFETY: this module is compiled only for targets whose CPUs all have
            // the AES extension (`cfg(target_feature = "aes")`), and NEON comes with it.
            unsafe { encrypt::<1>(&self.0, std::slice::from_mut(block).try_into().unwrap()) }
        }
        pub fn encrypt_lanes(&self, blocks: &mut [[u8; 16]; LANES]) {
            // SAFETY: as in `encrypt_block`.
            unsafe { encrypt(&self.0, blocks) }
        }
        pub fn cfb8_encrypt(&self, register: &mut [u8; 16], data: &mut [u8]) {
            // SAFETY: as in `encrypt_block`.
            unsafe { cfb8_encrypt(&self.0, register, data) }
        }
    }

    /// The register stays in a vector: one AES per byte and a byte shift.
    #[target_feature(enable = "neon,aes")]
    fn cfb8_encrypt(k: &[uint8x16_t; 11], register: &mut [u8; 16], data: &mut [u8]) {
        let mut r = load(register);
        for b in data {
            let mut s = r;
            for k in &k[..9] {
                s = vaesmcq_u8(vaeseq_u8(s, *k));
            }
            let s = veorq_u8(vaeseq_u8(s, k[9]), k[10]);
            *b ^= vgetq_lane_u8::<0>(s);
            r = vextq_u8::<1>(r, vdupq_n_u8(*b));
        }
        store(r, register);
    }

    #[target_feature(enable = "neon,aes")]
    fn store(v: uint8x16_t, b: &mut [u8; 16]) {
        let v = vreinterpretq_u64_u8(v);
        b[..8].copy_from_slice(&vgetq_lane_u64::<0>(v).to_le_bytes());
        b[8..].copy_from_slice(&vgetq_lane_u64::<1>(v).to_le_bytes());
    }

    #[target_feature(enable = "neon,aes")]
    fn load(b: &[u8; 16]) -> uint8x16_t {
        let half = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        vcombine_u8(vcreate_u8(half(0)), vcreate_u8(half(8)))
    }

    /// AESE is AddRoundKey, SubBytes and ShiftRows; AESMC is MixColumns; the last
    /// round has no MixColumns and ends with the eleventh key. Rounds go across all
    /// `N` blocks before the next, so the blocks' instructions overlap.
    #[target_feature(enable = "neon,aes")]
    fn encrypt<const N: usize>(k: &[uint8x16_t; 11], blocks: &mut [[u8; 16]; N]) {
        let mut s = blocks.each_ref().map(|b| load(b));
        for k in &k[..9] {
            for v in &mut s {
                *v = vaesmcq_u8(vaeseq_u8(*v, *k));
            }
        }
        for (v, b) in s.into_iter().zip(blocks) {
            store(veorq_u8(vaeseq_u8(v, k[9]), k[10]), b);
        }
    }
}

#[cfg(not(all(target_arch = "aarch64", target_feature = "aes")))]
mod imp {
    use aes::cipher::{BlockEncrypt, KeyInit};

    pub const LANES: usize = 32;

    #[derive(Clone)]
    pub struct Key(aes::Aes128);

    impl Key {
        pub fn new(key: &[u8; 16]) -> Self {
            Self(aes::Aes128::new(key.into()))
        }
        pub fn encrypt_block(&self, block: &mut [u8; 16]) {
            self.0.encrypt_block(block.into());
        }
        pub fn cfb8_encrypt(&self, register: &mut [u8; 16], data: &mut [u8]) {
            for b in data {
                let mut block = aes::Block::from(*register);
                self.0.encrypt_block(&mut block);
                *b ^= block[0];
                register.copy_within(1.., 0);
                register[15] = *b;
            }
        }
        pub fn encrypt_lanes(&self, blocks: &mut [[u8; 16]; LANES]) {
            let mut b = blocks.map(aes::Block::from);
            self.0.encrypt_blocks(&mut b);
            for (out, b) in blocks.iter_mut().zip(b) {
                out.copy_from_slice(&b);
            }
        }
    }
}
