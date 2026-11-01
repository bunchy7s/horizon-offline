//! Column-major `Matrix4f` arithmetic as JOML 1.10.9 computes it, for composing
//! item-model transformations: each operation takes the path JOML picks from
//! the matrix properties (identity, translation, affine...), and `Math.fma` is
//! `a * b + c` since Minecraft leaves `joml.useMathFma` off.

/// `Matrix4fc.PROPERTY_*` bits.
const PERSPECTIVE: u8 = 1;
const AFFINE: u8 = 2;
const IDENTITY: u8 = 4;
const TRANSLATION: u8 = 8;
const ORTHONORMAL: u8 = 16;

/// JOML `Math.fma` without `joml.useMathFma`.
#[inline]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    a * b + c
}
/// JOML `Math.absEqualsOne`.
fn abs_equals_one(r: f32) -> bool {
    r.to_bits() & 0x7FFF_FFFF == 0x3F80_0000
}

/// A JOML `Matrix4f`: `m[column][row]`, with its property bits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4], pub u8);

impl Mat4 {
    /// `new Matrix4f()`.
    pub const IDENTITY: Self = Self(
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        IDENTITY | AFFINE | TRANSLATION | ORTHONORMAL,
    );
    /// `Matrix4f.translation(x, y, z)`.
    pub fn translation(t: [f32; 3]) -> Self {
        let mut m = Self::IDENTITY;
        m.0[3][..3].copy_from_slice(&t);
        m.1 = AFFINE | TRANSLATION | ORTHONORMAL;
        m
    }
    /// `ExtraCodecs.MATRIX4F`: sixteen values in row-major order, then
    /// `determineProperties`.
    pub fn from_row_major(values: [f32; 16]) -> Self {
        let mut m = Self([[0.0; 4]; 4], 0);
        for (i, v) in values.into_iter().enumerate() {
            m.0[i & 3][i >> 2] = v;
        }
        m.1 = m.determine_properties();
        m
    }
    /// `Matrix4f.determineProperties`.
    fn determine_properties(&self) -> u8 {
        let m = &self.0;
        let mut properties = 0;
        if m[0][3] == 0.0 && m[1][3] == 0.0 {
            if m[2][3] == 0.0 && m[3][3] == 1.0 {
                properties |= AFFINE;
                if m[0] == [1.0, 0.0, 0.0, 0.0]
                    && m[1] == [0.0, 1.0, 0.0, 0.0]
                    && m[2] == [0.0, 0.0, 1.0, 0.0]
                {
                    properties |= TRANSLATION | ORTHONORMAL;
                    if m[3][0] == 0.0 && m[3][1] == 0.0 && m[3][2] == 0.0 {
                        properties |= IDENTITY;
                    }
                }
            } else if m[0][1] == 0.0
                && m[0][2] == 0.0
                && m[1][0] == 0.0
                && m[1][2] == 0.0
                && m[2][0] == 0.0
                && m[2][1] == 0.0
                && m[3][0] == 0.0
                && m[3][1] == 0.0
                && m[3][3] == 0.0
            {
                properties |= PERSPECTIVE;
            }
        }
        properties
    }
    fn has(&self, property: u8) -> bool {
        self.1 & property != 0
    }
    /// The matrix as `[row][column]` entries, the order `getRowColumn` reads.
    pub fn rows(&self) -> [[f32; 4]; 4] {
        std::array::from_fn(|r| std::array::from_fn(|c| self.0[c][r]))
    }
    /// `this * right` (`Matrix4f.mul`).
    pub fn mul(&self, right: &Self) -> Self {
        let (m, r) = (&self.0, &right.0);
        if self.has(IDENTITY) {
            return *right;
        }
        if right.has(IDENTITY) {
            return *self;
        }
        let mut out = [[0.0; 4]; 4];
        if self.has(TRANSLATION) && right.has(AFFINE) {
            // mulTranslationAffine
            for c in 0..4 {
                out[c] = [r[c][0], r[c][1], r[c][2], m[c][3]];
            }
            for row in 0..3 {
                out[3][row] = r[3][row] + m[3][row];
            }
            return Self(out, AFFINE | right.1 & ORTHONORMAL);
        }
        if self.has(AFFINE) && right.has(AFFINE) {
            // mulAffine
            for c in 0..3 {
                for row in 0..3 {
                    out[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
                out[c][3] = m[c][3];
            }
            for row in 0..3 {
                out[3][row] = fma(
                    m[0][row],
                    r[3][0],
                    fma(m[1][row], r[3][1], fma(m[2][row], r[3][2], m[3][row])),
                );
            }
            out[3][3] = m[3][3];
            return Self(out, AFFINE | self.1 & right.1 & ORTHONORMAL);
        }
        if self.has(PERSPECTIVE) && right.has(AFFINE) {
            // mulPerspectiveAffine
            for c in 0..4 {
                out[c] = [
                    m[0][0] * r[c][0],
                    m[1][1] * r[c][1],
                    m[2][2] * r[c][2],
                    m[2][3] * r[c][2],
                ];
            }
            out[3][2] = m[2][2] * r[3][2] + m[3][2];
            out[3][3] = m[2][3] * r[3][2];
            return Self(out, 0);
        }
        if right.has(AFFINE) {
            // mulAffineR
            for c in 0..3 {
                for row in 0..4 {
                    out[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
            }
            for row in 0..4 {
                out[3][row] = fma(
                    m[0][row],
                    r[3][0],
                    fma(m[1][row], r[3][1], fma(m[2][row], r[3][2], m[3][row])),
                );
            }
            return Self(
                out,
                self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION | ORTHONORMAL),
            );
        }
        // mul0
        for c in 0..4 {
            for row in 0..4 {
                out[c][row] = fma(
                    m[0][row],
                    r[c][0],
                    fma(
                        m[1][row],
                        r[c][1],
                        fma(m[2][row], r[c][2], m[3][row] * r[c][3]),
                    ),
                );
            }
        }
        Self(out, 0)
    }
    /// `Matrix4f.translate(x, y, z)`.
    pub fn translate(&self, t: [f32; 3]) -> Self {
        if self.has(IDENTITY) {
            return Self::translation(t);
        }
        // translateGeneric
        let mut out = *self;
        for row in 0..4 {
            out.0[3][row] = fma(
                self.0[0][row],
                t[0],
                fma(
                    self.0[1][row],
                    t[1],
                    fma(self.0[2][row], t[2], self.0[3][row]),
                ),
            );
        }
        out.1 = self.1 & !(PERSPECTIVE | IDENTITY);
        out
    }
    /// `Matrix4f.scale(x, y, z)`.
    pub fn scale(&self, s: [f32; 3]) -> Self {
        let one = s.iter().all(|&v| abs_equals_one(v));
        if self.has(IDENTITY) {
            // scaling
            let mut out = Self::IDENTITY;
            for (c, v) in s.into_iter().enumerate() {
                out.0[c][c] = v;
            }
            out.1 = AFFINE | if one { ORTHONORMAL } else { 0 };
            return out;
        }
        // scaleGeneric: the first three columns are scaled.
        let mut out = *self;
        for c in 0..3 {
            for row in 0..4 {
                out.0[c][row] = self.0[c][row] * s[c];
            }
        }
        out.1 =
            self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION | if one { 0 } else { ORTHONORMAL });
        out
    }
    /// `Matrix4f.rotate(Quaternionfc)`.
    pub fn rotate(&self, q: [f32; 4]) -> Self {
        let [x, y, z, w] = q;
        let (w2, x2, y2, z2) = (w * w, x * x, y * y, z * z);
        let (zw, xy, xz, yw, yz, xw) = (z * w, x * y, x * z, y * w, y * z, x * w);
        let (dzw, dxy, dxz, dyw, dyz, dxw) = (zw + zw, xy + xy, xz + xz, yw + yw, yz + yz, xw + xw);
        let r = [
            [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw],
            [-dzw + dxy, y2 - z2 + w2 - x2, dyz + dxw],
            [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2],
        ];
        let m = &self.0;
        let mut out = *self;
        if self.has(IDENTITY) {
            // rotation
            for c in 0..3 {
                out.0[c][..3].copy_from_slice(&r[c]);
            }
            out.1 = AFFINE | ORTHONORMAL;
            return out;
        }
        out.1 = self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION);
        if self.has(TRANSLATION) {
            // rotateTranslation
            for c in 0..3 {
                out.0[c] = [r[c][0], r[c][1], r[c][2], 0.0];
            }
        } else if self.has(AFFINE) {
            // rotateAffine
            for c in 0..3 {
                for row in 0..3 {
                    out.0[c][row] = m[0][row] * r[c][0] + m[1][row] * r[c][1] + m[2][row] * r[c][2];
                }
                out.0[c][3] = 0.0;
            }
        } else {
            // rotateGeneric
            for c in 0..3 {
                for row in 0..4 {
                    out.0[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
            }
        }
        out
    }
    pub fn is_finite(&self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }
}

/// `Quaternionf.normalize` as the codec applies it to four components.
pub fn normalize_quaternion(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    let inv = 1.0 / (x * x + (y * y + (z * z + w * w))).sqrt();
    [x * inv, y * inv, z * inv, w * inv]
}
/// JOML `Math.sin(float)` without fast math.
fn joml_sin(rad: f32) -> f32 {
    f64::from(rad).sin() as f32
}
/// JOML `Math.cosFromSin(float, float)` without fast math: the magnitude from
/// the sine, the sign from the angle's quadrant (exact zeros at right angles).
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0 - sin * sin).sqrt();
    let tau = (std::f64::consts::PI * 2.0) as f32;
    let a = angle + std::f32::consts::FRAC_PI_2;
    let mut b = a - (a / tau) as i32 as f32 * tau;
    if b < 0.0 {
        b += tau;
    }
    if b >= std::f32::consts::PI { -cos } else { cos }
}
/// `Quaternionf.rotationXYZ` from angles in radians.
pub fn quaternion_xyz(x: f32, y: f32, z: f32) -> [f32; 4] {
    let half = |a: f32| -> (f32, f32) {
        let h = a * 0.5;
        let s = joml_sin(h);
        (s, cos_from_sin(s, h))
    };
    let ((sx, cx), (sy, cy), (sz, cz)) = (half(x), half(y), half(z));
    let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
    [
        sx * cycz + cx * sysz,
        cx * sycz - sx * cysz,
        cx * cysz + sx * sycz,
        cx * cycz - sx * sysz,
    ]
}
/// `new Quaternionf(new AxisAngle4f(angle, axis))`: the angle is wrapped into
/// [0, 2π) and the axis is used as given.
pub fn quaternion_from_axis_angle(angle: f32, axis: [f32; 3]) -> [f32; 4] {
    let tau = (std::f64::consts::PI * 2.0) as f32;
    let angle = if angle < 0.0 {
        tau + angle % tau
    } else {
        angle
    } % tau;
    let sin = joml_sin(angle * 0.5);
    let cos = cos_from_sin(sin, angle * 0.5);
    [axis[0] * sin, axis[1] * sin, axis[2] * sin, cos]
}

/// A JOML `Matrix3f`: `m[column][row]`.
type Mat3 = [[f32; 3]; 3];

/// `Matrix3f.mul`: `a * b`.
fn mul3(a: &Mat3, b: &Mat3) -> Mat3 {
    std::array::from_fn(|c| {
        std::array::from_fn(|r| fma(a[0][r], b[c][0], fma(a[1][r], b[c][1], a[2][r] * b[c][2])))
    })
}
/// `Matrix3f.transpose`.
fn transpose3(m: &Mat3) -> Mat3 {
    std::array::from_fn(|c| std::array::from_fn(|r| m[r][c]))
}
/// `Matrix3f.rotate(Quaternionfc)`: `m * R(q)`, without fma.
fn rotate3(m: &Mat3, q: [f32; 4]) -> Mat3 {
    let [x, y, z, w] = q;
    let (w2, x2, y2, z2) = (w * w, x * x, y * y, z * z);
    let (zw, xy, xz, yw, yz, xw) = (z * w, x * y, x * z, y * w, y * z, x * w);
    let (zw, xy, xz, yw, yz, xw) = (zw + zw, xy + xy, xz + xz, yw + yw, yz + yz, xw + xw);
    let rotation = [
        [w2 + x2 - z2 - y2, xy + zw, xz - yw],
        [xy - zw, y2 - z2 + w2 - x2, yz + xw],
        [yw + xz, yz - xw, z2 - y2 - x2 + w2],
    ];
    std::array::from_fn(|c| {
        let rc = rotation[c];
        std::array::from_fn(|r| m[0][r] * rc[0] + m[1][r] * rc[1] + m[2][r] * rc[2])
    })
}
/// `Quaternionf.mul`: `a * b`.
fn quaternion_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let ([x, y, z, w], [qx, qy, qz, qw]) = (a, b);
    [
        fma(w, qx, fma(x, qw, fma(y, qz, -z * qy))),
        fma(w, qy, fma(-x, qz, fma(y, qw, z * qx))),
        fma(w, qz, fma(x, qy, fma(-y, qx, z * qw))),
        fma(w, qw, fma(-x, qx, fma(-y, qy, -z * qz))),
    ]
}

/// `GivensParameters`: the half angle's sine and cosine.
#[derive(Clone, Copy)]
struct Givens(f32, f32);

impl Givens {
    /// `GivensParameters.fromUnnormalized`.
    fn unnormalized(sin_half: f32, cos_half: f32) -> Self {
        let inv = 1.0 / (sin_half * sin_half + cos_half * cos_half).sqrt();
        Self(inv * sin_half, inv * cos_half)
    }
    /// `MatrixUtil.PI_4`: `fromPositiveAngle(π / 4)`.
    fn quarter_pi() -> Self {
        let half = std::f32::consts::FRAC_PI_4 / 2.0;
        let sin = joml_sin(half);
        Self(sin, cos_from_sin(sin, half))
    }
    fn inverse(self) -> Self {
        Self(-self.0, self.1)
    }
    fn cos(self) -> f32 {
        self.1 * self.1 - self.0 * self.0
    }
    fn sin(self) -> f32 {
        2.0 * self.0 * self.1
    }
    /// `aroundX/Y/Z(Quaternionf)` (axis 0, 1, 2).
    fn quaternion(self, axis: usize) -> [f32; 4] {
        let mut q = [0.0, 0.0, 0.0, self.1];
        q[axis] = self.0;
        q
    }
    /// `aroundX/Y/Z(Matrix3f)` (axis 0, 1, 2).
    fn matrix(self, axis: usize) -> Mat3 {
        let (cos, sin) = (self.cos(), self.sin());
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut m = [[0.0; 3]; 3];
        m[axis][axis] = 1.0;
        m[a][a] = cos;
        m[b][b] = cos;
        m[a][b] = sin;
        m[b][a] = -sin;
        m
    }
}

/// `MatrixUtil.approxGivensQuat`.
fn approx_givens(a11: f32, a12: f32, a22: f32) -> Givens {
    let g = 3.0 + 2.0 * 2.0f32.sqrt();
    let (cos_half, sin_half) = (2.0 * (a11 - a22), a12);
    if g * sin_half * sin_half < cos_half * cos_half {
        Givens::unnormalized(sin_half, cos_half)
    } else {
        Givens::quarter_pi()
    }
}
/// `MatrixUtil.qrGivensQuat`.
fn qr_givens(a1: f32, a2: f32) -> Givens {
    let p = f64::from(a1).hypot(f64::from(a2)) as f32;
    let mut sin_half = if p > 1.0e-6 { a2 } else { 0.0 };
    let mut cos_half = a1.abs() + if p > 1.0e-6 { p } else { 1.0e-6 };
    if a1 < 0.0 {
        std::mem::swap(&mut sin_half, &mut cos_half);
    }
    Givens::unnormalized(sin_half, cos_half)
}
/// `MatrixUtil.stepJacobi`: the xy, xz then yz pairs, each by
/// `similarityTransform` (`R^T * m * R`).
fn step_jacobi(m: &mut Mat3, q: &mut [f32; 4]) {
    let mut rotate = |m: &mut Mat3, g: Givens, axis: usize| {
        *q = quaternion_mul(*q, g.quaternion(axis));
        let r = g.matrix(axis);
        *m = mul3(&transpose3(&r), &mul3(m, &r));
    };
    if m[0][1] * m[0][1] + m[1][0] * m[1][0] > 1.0e-6 {
        let g = approx_givens(m[0][0], 0.5 * (m[0][1] + m[1][0]), m[1][1]);
        rotate(m, g, 2);
    }
    if m[0][2] * m[0][2] + m[2][0] * m[2][0] > 1.0e-6 {
        let g = approx_givens(m[0][0], 0.5 * (m[0][2] + m[2][0]), m[2][2]).inverse();
        rotate(m, g, 1);
    }
    if m[1][2] * m[1][2] + m[2][1] * m[2][1] > 1.0e-6 {
        let g = approx_givens(m[1][1], 0.5 * (m[1][2] + m[2][1]), m[2][2]);
        rotate(m, g, 0);
    }
}

/// `MatrixUtil.svdDecompose(Matrix4fc, ...)`, how `new Transformation(matrix)`
/// splits a matrix: translation, left rotation, scale, right rotation. The
/// right rotation diagonalizes `a^T * a` (`eigenvalueJacobi`, 5 steps), then
/// QR Givens rotations about z, y and x bring `a * R(right)` to the scale.
pub fn svd_decompose(m: &Mat4) -> ([f32; 3], [f32; 4], [f32; 3], [f32; 4]) {
    let inv = 1.0 / m.0[3][3];
    let a: Mat3 = std::array::from_fn(|c| std::array::from_fn(|r| m.0[c][r] * inv));
    let mut b = mul3(&transpose3(&a), &a);
    let mut right = [0.0, 0.0, 0.0, 1.0];
    for _ in 0..5 {
        step_jacobi(&mut b, &mut right);
    }
    right = normalize_quaternion(right);
    let (x_small, y_small) = (f64::from(b[0][0]) < 1.0e-6, f64::from(b[1][1]) < 1.0e-6);
    let mut left = [0.0, 0.0, 0.0, 1.0];
    let mut r = rotate3(&a, right);
    let mut qr = |r: &mut Mat3, g: Givens, axis: usize| {
        left = quaternion_mul(left, g.quaternion(axis));
        *r = mul3(&transpose3(&g.matrix(axis)), r);
    };
    let g = if x_small {
        qr_givens(r[1][1], -r[1][0])
    } else {
        qr_givens(r[0][0], r[0][1])
    };
    qr(&mut r, g, 2);
    let g = if x_small {
        qr_givens(r[2][2], -r[2][0])
    } else {
        qr_givens(r[0][0], r[0][2])
    };
    qr(&mut r, g.inverse(), 1);
    let g = if y_small {
        qr_givens(r[2][2], -r[2][1])
    } else {
        qr_givens(r[1][1], r[1][2])
    };
    qr(&mut r, g, 0);
    let scale = [r[0][0], r[1][1], r[2][2]];
    let translation = std::array::from_fn(|i| m.0[3][i] * inv);
    let [x, y, z, w] = right;
    (translation, left, scale, [-x, -y, -z, w])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composes_in_joml_order() {
        let t = Mat4::translation([1.0, 2.0, 3.0]);
        let s = Mat4::IDENTITY.scale([2.0, -1.0, 1.0]);
        let m = t.mul(&s);
        assert_eq!(m.0[0][0], 2.0);
        assert_eq!(m.0[1][1], -1.0);
        assert_eq!(m.0[3][..3], [1.0, 2.0, 3.0]);
        assert_eq!(s.mul(&t).0[3][..3], [2.0, -2.0, 3.0]);
    }
    #[test]
    fn quarter_turn_about_z_rotates_x_into_y() {
        let q = quaternion_from_axis_angle(std::f32::consts::FRAC_PI_2, [0.0, 0.0, 1.0]);
        let m = Mat4::IDENTITY.rotate(q);
        assert!((m.0[0][1] - 1.0).abs() < 1e-6 && m.0[0][0].abs() < 1e-6);
    }
    #[test]
    fn svd_decomposes_like_matrix_util() {
        let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        let parts = |m: [f32; 16]| {
            let (t, l, s, r) = svd_decompose(&Mat4::from_row_major(m));
            [bits(&t), bits(&l), bits(&s), bits(&r)]
        };
        // Values from MatrixUtil.svdDecompose on the 26.3 jar.
        let scaled = [
            2.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0, -2.0, 0.0, 0.0, 0.5, 0.25, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            parts(scaled),
            [
                vec![0x3f80_0000u32, 0xc000_0000, 0x3e80_0000],
                vec![0u32, 0, 0, 0x3f80_0000],
                vec![0x4000_0000u32, 0x4040_0000, 0x3f00_0000],
                vec![0x8000_0000u32, 0x8000_0000, 0x8000_0000, 0x3f80_0000],
            ]
        );
        let quarter_turn = [
            0.0, -1.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            parts(quarter_turn),
            [
                vec![0x3f00_0000u32, 0, 0],
                vec![0u32, 0, 0x3f35_04f3, 0x3f35_04f3],
                vec![0x3f7f_ffffu32, 0x3f7f_ffff, 0x3f80_0000],
                vec![0x8000_0000u32, 0x8000_0000, 0x8000_0000, 0x3f80_0000],
            ]
        );
        let sheared = [
            1.0, 0.5, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.25, 0.0, 2.0, 3.0, 0.0, 0.0, 0.0, 2.0,
        ];
        assert_eq!(
            parts(sheared),
            [
                vec![0u32, 0, 0x3fc0_0000],
                vec![0x3c03_7010u32, 0x3d38_d722, 0x3ea8_ef95, 0x3f71_60d9],
                vec![0x3f23_2630u32, 0x3ec6_bdde, 0x3f81_5b1f],
                vec![0xbcc5_7e11u32, 0xbda8_1e36, 0xbee2_916d, 0x3f64_860a],
            ]
        );
    }
    #[test]
    fn half_turn_has_exact_zeros_like_joml() {
        let half_turn = 180.0 * (std::f64::consts::PI / 180.0) as f32;
        assert_eq!(quaternion_xyz(0.0, half_turn, 0.0), [0.0, 1.0, 0.0, 0.0]);
    }
}
