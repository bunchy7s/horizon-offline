//! `java.lang.Math` as HotSpot computes it on the machine Horizon runs on.
//!
//! Rust's float functions call the platform's libm, and those round
//! differently: glibc, Apple's libm and fdlibm disagree in the last bit for
//! a few percent of arguments. Vanilla's own results differ between machines
//! too, because HotSpot replaces some `Math` methods with intrinsics:
//!
//! - `atan2`, `atan`, `asin`, `acos` and `hypot` are `StrictMath`'s (fdlibm)
//!   everywhere.
//! - On x86-64, `sin`, `cos`, `tan`, `exp`, `log`, `log10` and `pow` are
//!   Intel's libm stubs, which round like glibc for nearly every argument;
//!   there they use the platform libm, as before.
//! - On AArch64 only `sin` and `cos` are intrinsics: fdlibm's kernels
//!   evaluated with fused multiply-adds (`macroAssembler_aarch64_trig.cpp`).
//!   Everything else is `StrictMath`.
//!
//! So these functions return what vanilla returns on the same machine, on
//! Linux and on an Apple silicon Mac alike.

/// `Math.sin`.
pub fn sin(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        aarch64::sin(x)
    } else {
        x.sin()
    }
}

/// `Math.cos`.
pub fn cos(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        aarch64::cos(x)
    } else {
        x.cos()
    }
}

/// `Math.tan`.
pub fn tan(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        libm::tan(x)
    } else {
        x.tan()
    }
}

/// `Math.exp`.
pub fn exp(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        libm::exp(x)
    } else {
        x.exp()
    }
}

/// `Math.log`.
pub fn log(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        fdlibm::log(x)
    } else {
        x.ln()
    }
}

/// `Math.log10`.
pub fn log10(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        fdlibm::log10(x)
    } else {
        x.log10()
    }
}

/// `Math.pow`.
pub fn pow(x: f64, y: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        libm::pow(x, y)
    } else {
        x.powf(y)
    }
}

/// `Math.cbrt`.
pub fn cbrt(x: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        fdlibm::cbrt(x)
    } else {
        x.cbrt()
    }
}

/// `Math.atan2`.
pub fn atan2(y: f64, x: f64) -> f64 {
    crate::geometry::trig::fdlibm_atan2(y, x)
}

/// `Math.atan`.
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}

/// `Math.asin`.
pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}

/// `Math.acos`.
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}

/// `Math.hypot`.
pub fn hypot(x: f64, y: f64) -> f64 {
    fdlibm::hypot(x, y)
}

fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn with_hi(x: f64, high: i32) -> f64 {
    f64::from_bits((x.to_bits() & 0xffff_ffff) | (u64::from(high as u32) << 32))
}

/// `java.lang.FdLibm` ports (`StrictMath`) of what the `libm` crate computes
/// differently.
// fdlibm's constants and NaN, verbatim.
#[allow(clippy::excessive_precision, clippy::approx_constant, clippy::eq_op)]
pub mod fdlibm {
    use super::{hi, with_hi};

    const TWO54: f64 = 1.80143985094819840000e+16;

    /// `FdLibm.Log`.
    pub fn log(mut x: f64) -> f64 {
        const LN2_HI: f64 = 6.93147180369123816490e-01;
        const LN2_LO: f64 = 1.90821492927058770002e-10;
        const LG1: f64 = 6.666666666666735130e-01;
        const LG2: f64 = 3.999999999940941908e-01;
        const LG3: f64 = 2.857142874366239149e-01;
        const LG4: f64 = 2.222219843214978396e-01;
        const LG5: f64 = 1.818357216161805012e-01;
        const LG6: f64 = 1.531383769920937332e-01;
        const LG7: f64 = 1.479819860511658591e-01;
        let mut hx = hi(x);
        let lx = x.to_bits() as u32;
        let mut k = 0;
        if hx < 0x0010_0000 {
            if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
                return -TWO54 / 0.0;
            }
            if hx < 0 {
                return (x - x) / 0.0;
            }
            k -= 54;
            x *= TWO54;
            hx = hi(x);
        }
        if hx >= 0x7ff0_0000 {
            return x + x;
        }
        k += (hx >> 20) - 1023;
        hx &= 0x000f_ffff;
        let i = (hx + 0x9_5f64) & 0x10_0000;
        x = with_hi(x, hx | (i ^ 0x3ff0_0000));
        k += i >> 20;
        let f = x - 1.0;
        if (0x000f_ffff & (2 + hx)) < 3 {
            if f == 0.0 {
                if k == 0 {
                    return 0.0;
                }
                let dk = f64::from(k);
                return dk * LN2_HI + dk * LN2_LO;
            }
            let r = f * f * (0.5 - 0.33333333333333333 * f);
            if k == 0 {
                return f - r;
            }
            let dk = f64::from(k);
            return dk * LN2_HI - ((r - dk * LN2_LO) - f);
        }
        let s = f / (2.0 + f);
        let dk = f64::from(k);
        let z = s * s;
        let mut i = hx - 0x6_147a;
        let w = z * z;
        let j = 0x6b851 - hx;
        let t1 = w * (LG2 + w * (LG4 + w * LG6));
        let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
        i |= j;
        let r = t2 + t1;
        if i > 0 {
            let hfsq = 0.5 * f * f;
            if k == 0 {
                f - (hfsq - s * (hfsq + r))
            } else {
                dk * LN2_HI - ((hfsq - (s * (hfsq + r) + dk * LN2_LO)) - f)
            }
        } else if k == 0 {
            f - s * (f - r)
        } else {
            dk * LN2_HI - ((s * (f - r) - dk * LN2_LO) - f)
        }
    }

    /// `FdLibm.Log10`.
    pub fn log10(mut x: f64) -> f64 {
        const IVLN10: f64 = 4.34294481903251816668e-01;
        const LOG10_2HI: f64 = 3.01029995663611771306e-01;
        const LOG10_2LO: f64 = 3.69423907715893078616e-13;
        let mut hx = hi(x);
        let lx = x.to_bits() as u32;
        let mut k = 0;
        if hx < 0x0010_0000 {
            if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
                return -TWO54 / 0.0;
            }
            if hx < 0 {
                return (x - x) / 0.0;
            }
            k -= 54;
            x *= TWO54;
            hx = hi(x);
        }
        if hx >= 0x7ff0_0000 {
            return x + x;
        }
        k += (hx >> 20) - 1023;
        let i = ((k as u32) >> 31) as i32;
        hx = (hx & 0x000f_ffff) | ((0x3ff - i) << 20);
        let y = f64::from(k + i);
        x = with_hi(x, hx);
        let z = y * LOG10_2LO + IVLN10 * log(x);
        z + y * LOG10_2HI
    }

    /// `FdLibm.Cbrt`.
    pub fn cbrt(mut x: f64) -> f64 {
        const B1: i32 = 715094163;
        const B2: i32 = 696219795;
        const C: f64 = 5.42857142857142815906e-01;
        const D: f64 = -7.05306122448979611050e-01;
        const E: f64 = 1.41428571428571436819e+00;
        const F: f64 = 1.60714285714285720630e+00;
        const G: f64 = 3.57142857142857150787e-01;
        if x == 0.0 || !x.is_finite() {
            return x;
        }
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        x = x.abs();
        let mut t = 0.0f64;
        if x < f64::MIN_POSITIVE {
            t = 18014398509481984.0; // 2^54
            t *= x;
            t = with_hi(t, hi(t) / 3 + B2);
        } else {
            t = with_hi(t, hi(x) / 3 + B1);
        }
        let mut r = t * t / x;
        let mut s = C + r * t;
        t *= G + F / (s + E + D / s);
        t = f64::from_bits(t.to_bits() & 0xffff_ffff_0000_0000);
        t = with_hi(t, hi(t) + 1);
        s = t * t;
        r = x / s;
        let w = t + t;
        r = (r - t) / (w + r);
        t += t * r;
        sign * t
    }

    /// `FdLibm.Hypot`.
    pub fn hypot(x: f64, y: f64) -> f64 {
        let mut a = x.abs();
        let mut b = y.abs();
        if !a.is_finite() || !b.is_finite() {
            if a == f64::INFINITY || b == f64::INFINITY {
                return f64::INFINITY;
            }
            return a + b;
        }
        if b > a {
            std::mem::swap(&mut a, &mut b);
        }
        let mut ha = hi(a);
        let mut hb = hi(b);
        if ha - hb > 0x3c0_0000 {
            return a + b;
        }
        let mut k = 0;
        if a > f64::from_bits(0x5f30_0000_ffff_ffff) {
            ha -= 0x2580_0000;
            hb -= 0x2580_0000;
            a *= f64::from_bits(0x1a70_0000_0000_0000); // 2^-600
            b *= f64::from_bits(0x1a70_0000_0000_0000);
            k += 600;
        }
        if b < f64::from_bits(0x20b0_0000_0000_0000) {
            // 2^-500
            if b < f64::MIN_POSITIVE {
                if b == 0.0 {
                    return a;
                }
                let t1 = f64::from_bits(0x7fd0_0000_0000_0000); // 2^1022
                b *= t1;
                a *= t1;
                k -= 1022;
            } else {
                ha += 0x2580_0000;
                hb += 0x2580_0000;
                a *= f64::from_bits(0x6570_0000_0000_0000); // 2^600
                b *= f64::from_bits(0x6570_0000_0000_0000);
                k -= 600;
            }
        }
        let mut w = a - b;
        if w > b {
            let t1 = with_hi(0.0, ha);
            let t2 = a - t1;
            w = (t1 * t1 - (b * (-b) - t2 * (a + t1))).sqrt();
        } else {
            a += a;
            let y1 = with_hi(0.0, hb);
            let y2 = b - y1;
            let t1 = with_hi(0.0, ha + 0x0010_0000);
            let t2 = a - t1;
            w = (t1 * y1 - (w * (-w) - (t1 * y2 + t2 * b))).sqrt();
        }
        if k != 0 {
            f64::from_bits(((1023 + k) as u64) << 52) * w
        } else {
            w
        }
    }
}

/// HotSpot's AArch64 `Math.sin` / `Math.cos` stubs
/// (`MacroAssembler::generate_dsin_dcos`): fdlibm with fused multiply-adds.
/// Exposed on every architecture so they can be tested anywhere.
// HotSpot's tables, verbatim.
#[allow(
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::unreadable_literal
)]
pub mod aarch64 {
    use super::hi;

    const S: [f64; 6] = [
        -1.66666666666666324348e-01,
        8.33333333332248946124e-03,
        -1.98412698298579493134e-04,
        2.75573137070700676789e-06,
        -2.50507602534068634195e-08,
        1.58969099521155010221e-10,
    ];
    const C: [f64; 6] = [
        4.16666666666666019037e-02,
        -1.38888888888741095749e-03,
        2.48015872894767294178e-05,
        -2.75573143513906633035e-07,
        2.08757232129817482790e-09,
        -1.13596475577881948265e-11,
    ];
    const INVPIO2: f64 = 6.36619772367581382433e-01;
    const PIO2_1: f64 = 1.57079632673412561417e+00;
    const PIO2_1T: f64 = 6.07710050650619224932e-11;
    const PIO2_2: f64 = 6.07710050630396597660e-11;
    const PIO2_2T: f64 = 2.02226624879595063154e-21;
    const PIO2_3: f64 = 2.02226624871116645580e-21;
    const PIO2_3T: f64 = 8.47842766036889956997e-32;
    const NPIO2_HW: [i32; 32] = [
        0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB,
        0x402921FB, 0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB,
        0x40378FDB, 0x403921FB, 0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C,
        0x4041475C, 0x4042106C, 0x4042D97C, 0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB,
        0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
    ];

    /// `generate_kernel_sin`.
    fn kernel_sin(x: f64, y: f64, iy: bool) -> f64 {
        let z = x * x;
        let v = z * x;
        let mut r = z.mul_add(S[5], S[4]);
        r = z.mul_add(r, S[3]);
        r = z.mul_add(r, S[2]);
        r = z.mul_add(r, S[1]);
        if !iy {
            v.mul_add(z.mul_add(r, S[0]), x)
        } else {
            let t = (-v).mul_add(r, 0.5 * y);
            let t = (-z).mul_add(t, y);
            x + v.mul_add(S[0], t)
        }
    }

    /// `generate_kernel_cos`. `ix` is the high word of the *original*
    /// argument: the stub reuses the register, so a reduced argument always
    /// takes the `qx = 0.28125` branch.
    fn kernel_cos(x: f64, y: f64, ix: i32) -> f64 {
        let z = x * x;
        let mut r = z.mul_add(C[5], C[4]);
        r = z.mul_add(r, C[3]);
        let xy = x * y;
        r = z.mul_add(r, C[2]);
        r = z.mul_add(r, C[1]);
        let zz = z * z;
        r = z.mul_add(r, C[0]);
        if ix <= 0x3FD33333 {
            let v = (-zz).mul_add(r, xy);
            1.0 - 0.5f64.mul_add(z, v)
        } else {
            let qx = if ix > 0x3fe90000 {
                0.28125
            } else {
                f64::from_bits(u64::from((ix - 0x0020_0000) as u32) << 32)
            };
            let zr_xy = zz.mul_add(r, -xy);
            let h = 0.5f64.mul_add(z, -qx);
            let a = 1.0 - qx;
            a - (h - zr_xy)
        }
    }

    /// `generate__ieee754_rem_pio2` for |x| <= 2^19·π/2; `None` beyond.
    fn rem_pio2(x: f64, ix: i32) -> Option<(i32, f64, f64)> {
        if ix < 0x4002d97c {
            return Some(if x.to_bits() as i64 >= 0 {
                let z = x - PIO2_1;
                if ix != 0x3ff921fb {
                    let y0 = z - PIO2_1T;
                    (1, y0, (z - y0) - PIO2_1T)
                } else {
                    let z = z - PIO2_2;
                    let y0 = z - PIO2_2T;
                    (1, y0, (z - y0) - PIO2_2T)
                }
            } else {
                let z = x + PIO2_1;
                if ix != 0x3ff921fb {
                    let y0 = z + PIO2_1T;
                    (-1, y0, (z - y0) + PIO2_1T)
                } else {
                    let z = z + PIO2_2;
                    let y0 = z + PIO2_2T;
                    (-1, y0, (z - y0) + PIO2_2T)
                }
            });
        }
        if ix > 0x413921fb {
            return None;
        }
        let t = x.abs();
        let fnv = t.mul_add(INVPIO2, 0.5).trunc();
        let n = fnv as i32;
        let mut r = (-fnv).mul_add(PIO2_1, t);
        let mut w = fnv * PIO2_1T;
        let mut y0 = r - w;
        if n > 32 || ix == NPIO2_HW[(n - 1) as usize] {
            let j = ix >> 20;
            let exponent = |y: f64| ((y.to_bits() << 1) >> 53) as i32;
            if j - exponent(y0) > 16 {
                let t = r;
                let w2 = fnv * PIO2_2;
                r = t - w2;
                w = fnv.mul_add(PIO2_2T, w2 - (t - r));
                y0 = r - w;
                if j - exponent(y0) > 49 {
                    let t = r;
                    let w3 = fnv * PIO2_3;
                    r = t - w3;
                    w = fnv.mul_add(PIO2_3T, w3 - (t - r));
                    y0 = r - w;
                }
            }
        }
        let y1 = (r - y0) - w;
        Some(if x > 0.0 { (n, y0, y1) } else { (-n, -y0, -y1) })
    }

    fn sin_cos(x: f64, is_cos: bool) -> f64 {
        let ix = hi(x) & 0x7fff_ffff;
        if ix < 0x3e400000 {
            return if is_cos { 1.0 } else { x };
        }
        if ix <= 0x3fe921fb {
            return if is_cos {
                kernel_cos(x, 0.0, ix)
            } else {
                kernel_sin(x, 0.0, false)
            };
        }
        if ix >= 0x7ff00000 {
            return f64::from_bits(0x7ff0_0000_0000_0001);
        }
        // Beyond 2^19·π/2 the stub's Payne–Hanek reduction isn't ported:
        // StrictMath's is used instead.
        let Some((n, y0, y1)) = rem_pio2(x, ix) else {
            return if is_cos { libm::cos(x) } else { libm::sin(x) };
        };
        let (value, negate) = if (n & 1 == 1) != is_cos {
            (
                kernel_cos(y0, y1, ix),
                if is_cos { (n ^ (n >> 1)) & 1 } else { n & 2 },
            )
        } else {
            (
                kernel_sin(y0, y1, true),
                if is_cos { (n ^ (n >> 1)) & 1 } else { n & 2 },
            )
        };
        if negate != 0 { -value } else { value }
    }

    /// `Math.sin` on AArch64 HotSpot.
    pub fn sin(x: f64) -> f64 {
        sin_cos(x, false)
    }

    /// `Math.cos` on AArch64 HotSpot.
    pub fn cos(x: f64) -> f64 {
        sin_cos(x, true)
    }
}

#[cfg(test)]
mod tests {
    use super::{aarch64, fdlibm};

    fn check1(f: fn(f64) -> f64, (x, expected): (u64, u64)) {
        assert_eq!(f(f64::from_bits(x)).to_bits(), expected, "{x:x}");
    }

    fn check2(f: fn(f64, f64) -> f64, (x, y, expected): (u64, u64, u64)) {
        assert_eq!(
            f(f64::from_bits(x), f64::from_bits(y)).to_bits(),
            expected,
            "{x:x} {y:x}"
        );
    }

    #[test]
    fn matches_hotspot_on_aarch64() {
        // (argument bits, Math result bits) from HotSpot 25 on an Apple M4, each
        // one where glibc rounds differently.
        // sin
        check1(aarch64::sin, (0x3fe63c5ab66b09f0, 0x3fe47d34215b54f7));
        check1(aarch64::sin, (0xc0569150fe105e6f, 0xbfe7bc19d65ab55c));
        check1(aarch64::sin, (0x4072c3f15b47b5d6, 0xbfef32be0f0a9124));
        check1(aarch64::sin, (0x3fee86c6ffce047e, 0x3fea1a47f46cbf28));
        // cos
        check1(aarch64::cos, (0x3fbb10690eeadb00, 0x3fefd24375dbae28));
        check1(aarch64::cos, (0xc0496cc08a65245d, 0x3feab1a44a8022c4));
        check1(aarch64::cos, (0x40099262f89e73f8, 0xbfeff3aa148795ce));
        check1(aarch64::cos, (0x4001ce4e022387bc, 0xbfe37dd512be4d00));
        // log
        check1(fdlibm::log, (0x40569150fe105e6f, 0x401202e102f27b78));
        check1(fdlibm::log, (0x3ff5dec70f518390, 0x3fd4009cda1f8856));
        check1(fdlibm::log, (0x3feb3d17325d493e, 0xbfc49ee8c4866f84));
        check1(fdlibm::log, (0x3fe735a40edf77ba, 0xbfd48e1816b18bf2));
        // log10
        check1(fdlibm::log10, (0x3ff5dec70f518390, 0x3fc15fb52aa5f051));
        check1(fdlibm::log10, (0x3feb3d17325d493e, 0xbfb1e933cae02f5f));
        check1(fdlibm::log10, (0x3fe735a40edf77ba, 0xbfc1da98cbb8974c));
        check1(fdlibm::log10, (0x3fe7febd9ca1bab0, 0xbfc000e8e3b0b56e));
        // cbrt
        check1(fdlibm::cbrt, (0x401bb21f8568bd0c, 0x3ffe7ee30c7a56f9));
        check1(fdlibm::cbrt, (0x3fc59f3a45c8fb80, 0x3fe1b0791e0f8a74));
        check1(fdlibm::cbrt, (0xbfdd73cdc693ddab, 0xbfe8b4a6f43f1b73));
        check1(fdlibm::cbrt, (0xbfeb0715a8425e10, 0xbfee3f99498da6f6));
        // hypot
        check2(
            fdlibm::hypot,
            (0xc0569150fe105e6f, 0xc0056b970bf3b41a, 0x405693db72f461a6),
        );
        check2(
            fdlibm::hypot,
            (0x3fdc5999df9270f4, 0xc006c3ad990f72c0, 0x400709de26a1e40a),
        );
        check2(
            fdlibm::hypot,
            (0x401bb21f8568bd0c, 0xbffeb93d1a2ef904, 0x401cbdbb623d20c3),
        );
        check2(
            fdlibm::hypot,
            (0x4026d31c32ddb50b, 0xc00e7b2771d6867c, 0x40281027bb313aae),
        );
        // pow
        check2(
            libm::pow,
            (0x3fee04c313607ea6, 0xc00fd156138baf96, 0x3ff4a199ac2c8e9c),
        );
        check2(
            libm::pow,
            (0x401bb21f8568bd0c, 0xbffeb93d1a2ef904, 0x3f98ecc16a60feee),
        );
        check2(
            libm::pow,
            (0x407383ff427dadbd, 0x3ffdc5d178d00264, 0x40e56695c058d704),
        );
        check2(
            libm::pow,
            (0x3fe0c95b66dd6030, 0xc00c9cfde259a3e6, 0x4024194449e9deca),
        );
        // exp
        check1(libm::exp, (0x3fdc5999df9270f4, 0x3ff8eacc53fdbf56));
        check1(libm::exp, (0x3fcffff502ecbd80, 0x3ff48b5c78bca894));
        check1(libm::exp, (0x3fc59f3a45c8fb80, 0x3ff2f1c6cb35728c));
        check1(libm::exp, (0x4056561bf10ba7f6, 0x47fdd21dc9013084));
        // tan
        check1(libm::tan, (0x40023034f56881f0, 0xbff2e3f762ac9810));
        check1(libm::tan, (0xbfebf862877c88ec, 0xbff31f61e3e8539e));
        check1(libm::tan, (0xc00e37e234490564, 0xbfe79ca6eee66726));
        check1(libm::tan, (0xc075c085c04800b5, 0x3fea18b4a92fcf22));
    }

    #[test]
    fn atan2_matches_strictmath() {
        // (y bits, x bits, StrictMath.atan2 bits) from HotSpot 25: x < 0 and
        // |y/x| > 2^60, where libm::atan2 rounds one ulp lower,
        check2(
            super::atan2,
            (0x440e8aff5d1e293c, 0xc007e9768e83b37e, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x440c610b44a22b37, 0xbfb07327bcb4c280, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x43eb71600e035874, 0xbfeaff77e5a9ea38, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x440edb2112965d8f, 0xbffaec4345ac33b1, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x3ff0000000000000, 0x8000000000000001, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0xbff0000000000000, 0x8000000000000001, 0xbff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x7fefffffffffffff, 0xbff0000000000000, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0xffefffffffffffff, 0xbff0000000000000, 0xbff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x38fd955bf292fbac, 0xb4f940da66879af2, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0xf8954a6a9b403072, 0xf49292c2e2d50436, 0xbff921fb54442d19),
        );
        check2(
            super::atan2,
            (0x07e155d47eb507f2, 0x83d22629ee8e8471, 0x3ff921fb54442d19),
        );
        check2(
            super::atan2,
            (0xd2471cd599853356, 0xce403e2e72061985, 0xbff921fb54442d19),
        );
        // and the other quadrants and special cases, which it already matched
        check2(
            super::atan2,
            (0x440e8aff5d1e293c, 0x4007e9768e83b37e, 0x3ff921fb54442d18),
        );
        check2(
            super::atan2,
            (0xc40e8aff5d1e293c, 0x4007e9768e83b37e, 0xbff921fb54442d18),
        );
        check2(
            super::atan2,
            (0x0000000000000000, 0xbff0000000000000, 0x400921fb54442d18),
        );
        check2(
            super::atan2,
            (0x8000000000000000, 0xbff0000000000000, 0xc00921fb54442d18),
        );
        check2(
            super::atan2,
            (0x3ff0000000000000, 0x3ff0000000000000, 0x3fe921fb54442d18),
        );
        check2(
            super::atan2,
            (0x3ff0000000000000, 0xc410000000000000, 0x400921fb54442d18),
        );
        check2(
            super::atan2,
            (0x3e7ad7f29abcaf48, 0xc0f86a0000000000, 0x400921fb5444244c),
        );
        check2(
            super::atan2,
            (0x7ff0000000000000, 0xfff0000000000000, 0x4002d97c7f3321d2),
        );
        // NaN payloads may differ, so only NaN-ness is checked.
        assert!(super::atan2(f64::NAN, 1.0).is_nan());
        assert!(super::atan2(1.0, f64::NAN).is_nan());
    }
}
