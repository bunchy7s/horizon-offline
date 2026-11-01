use super::{LegacyRandom, RandomSource, XoroshiroRandom, seed_from_hash, seed_from_hash_utf16};
use crate::position::BlockPos;

#[allow(clippy::wrong_self_convention)] // These are seeded factories, matching Java's fromSeed/fromHashOf.
pub trait PositionalRandom {
    type Source: RandomSource;
    fn at(&self, position: BlockPos) -> Self::Source;
    fn from_seed(&self, seed: i64) -> Self::Source;
    fn from_hash_of(&self, name: &str) -> Self::Source;
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyPositionalRandom {
    seed: i64,
    base: u64,
}
impl LegacyPositionalRandom {
    pub const fn new(seed: i64) -> Self {
        let base = (seed as u64 ^ 0x5_deec_e66d) & ((1 << 48) - 1);
        Self { seed, base }
    }
}
impl PositionalRandom for LegacyPositionalRandom {
    type Source = LegacyRandom;
    fn at(&self, position: BlockPos) -> Self::Source {
        let p = (position_seed(position) as u64) & ((1 << 48) - 1);
        LegacyRandom::from_state(p ^ self.base)
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        LegacyRandom::new(seed)
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        let h = (java_string_hash(name.encode_utf16()) as i64 as u64) & ((1 << 48) - 1);
        LegacyRandom::from_state(h ^ self.base)
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        let h = (java_string_hash(name.iter().copied()) as i64 as u64) & ((1 << 48) - 1);
        LegacyRandom::from_state(h ^ self.base)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XoroshiroPositionalRandom {
    lo: u64,
    hi: u64,
}
impl XoroshiroPositionalRandom {
    pub const fn new(lo: u64, hi: u64) -> Self {
        Self { lo, hi }
    }
}
impl PositionalRandom for XoroshiroPositionalRandom {
    type Source = XoroshiroRandom;
    fn at(&self, position: BlockPos) -> Self::Source {
        XoroshiroRandom::from_state(position_seed(position) as u64 ^ self.lo, self.hi)
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        XoroshiroRandom::from_state(seed as u64 ^ self.lo, seed as u64 ^ self.hi)
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        let [lo, hi] = seed_from_hash(name);
        XoroshiroRandom::from_state(lo ^ self.lo, hi ^ self.hi)
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        let [lo, hi] = seed_from_hash_utf16(name);
        XoroshiroRandom::from_state(lo ^ self.lo, hi ^ self.hi)
    }
}
pub fn java_string_hash(name: impl IntoIterator<Item = u16>) -> i32 {
    name.into_iter().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(unit as i32)
    })
}
/// Mth.getSeed's signed Java arithmetic, including the initial int product.
pub const fn position_seed(position: BlockPos) -> i64 {
    let mixed = position.x.wrapping_mul(3_129_871) as i64
        ^ (position.z as i64).wrapping_mul(116_129_781)
        ^ position.y as i64;
    mixed
        .wrapping_mul(mixed)
        .wrapping_mul(42_317_861)
        .wrapping_add(mixed.wrapping_mul(11))
        >> 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_positional_matches_direct_seeding() {
        let seeds = [0, 1, -1, 42, 123456789, -987654321, i64::MAX, i64::MIN];
        let positions = [
            BlockPos::new(0, 0, 0),
            BlockPos::new(10, 64, -20),
            BlockPos::new(-1000, 320, 5000),
            BlockPos::new(123456, -64, -654321),
        ];

        for &s in &seeds {
            let factory = LegacyPositionalRandom::new(s);
            for &pos in &positions {
                let mut direct = LegacyRandom::new(position_seed(pos) ^ s);
                let mut opt = factory.at(pos);
                for _ in 0..10 {
                    assert_eq!(direct.next_int(), opt.next_int());
                }
            }

            for name in ["minecraft:ore", "minecraft:tree", "test", ""] {
                let mut direct =
                    LegacyRandom::new(java_string_hash(name.encode_utf16()) as i64 ^ s);
                let mut opt = factory.from_hash_of(name);
                for _ in 0..10 {
                    assert_eq!(direct.next_int(), opt.next_int());
                }
            }
        }
    }

    #[test]
    #[ignore = "benchmark"]
    fn benchmark_legacy_positional_at() {
        use std::time::Instant;
        let s = 123456789i64;
        let factory = LegacyPositionalRandom::new(s);
        let positions: Vec<BlockPos> = (0..10_000)
            .map(|i| BlockPos::new((i * 17) % 1000, (i * 31) % 256, (i * 47) % 1000))
            .collect();

        // Baseline (recalculating seed ^ MULTIPLIER & MASK every call):
        let start = Instant::now();
        let iters = 200;
        let mut sum_base = 0i64;
        for _ in 0..iters {
            for &pos in &positions {
                let mut r = LegacyRandom::new(position_seed(pos) ^ s);
                sum_base = sum_base.wrapping_add(r.next_int() as i64);
            }
        }
        let elapsed_base = start.elapsed();

        // Optimized (precomputed 48-bit base):
        let start = Instant::now();
        let mut sum_opt = 0i64;
        for _ in 0..iters {
            for &pos in &positions {
                let mut r = factory.at(pos);
                sum_opt = sum_opt.wrapping_add(r.next_int() as i64);
            }
        }
        let elapsed_opt = start.elapsed();

        assert_eq!(sum_base, sum_opt);
        println!(
            "benchmark_legacy_positional_at ({} queries):\n  Baseline (origin/main): {:?} ({:.2} ns/query)\n  Optimized (48-bit precomputed): {:?} ({:.2} ns/query)\n  Speedup: {:.2}x ({:.1}% faster)",
            positions.len() * iters,
            elapsed_base,
            elapsed_base.as_nanos() as f64 / (positions.len() * iters) as f64,
            elapsed_opt,
            elapsed_opt.as_nanos() as f64 / (positions.len() * iters) as f64,
            elapsed_base.as_secs_f64() / elapsed_opt.as_secs_f64(),
            (1.0 - elapsed_opt.as_secs_f64() / elapsed_base.as_secs_f64()) * 100.0
        );
    }
}
