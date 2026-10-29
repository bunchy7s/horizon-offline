//! The sunrise/sunset fan (`SkyRenderer.renderSunriseAndSunset`) as the sky
//! shader tests it. The fan's 16 triangles depend only on the sunrise alpha
//! and the horizon the sun is on, so their vertices are built here once a
//! frame instead of in every sky pixel (#3143: with the per-pixel build, four
//! `sin`/`cos` per triangle, the sky pass took 10.2 ms at dawn at 1080p on an
//! Intel UHD 730 against 0.6 ms by day; now 1.8 ms).
//!
//! Möller-Trumbore from the camera reduces to triple products: with apex A
//! and rim vertices r0, r1, a ray d hits at barycentrics proportional to
//! (`[d, r0, r1]`, `[A, d, r1]`, `[A, r0, d]`), each a dot of d with a
//! constant normal, and their sum S is the negated determinant. A hit needs
//! S on the side of `[A, r0, r1]`, so only the triangles whose rim normals'
//! dots change sign there are tested in full.
use glam::Vec3;

pub const TRIANGLES: usize = 16;

/// Per triangle: xyz `r0 × r1` (its dot with a ray is the apex's weight
/// times S), w `[A, r0, r1]` (its sign is S's where the hit lies ahead).
/// Then per rim vertex: xyz `r × A`.
pub fn normals(alpha: f32, sun: Vec3) -> [[f32; 4]; 2 * TRIANGLES] {
    let (apex, rim) = vertices(alpha, sun);
    let mut out = [[0.0; 4]; 2 * TRIANGLES];
    for i in 0..TRIANGLES {
        let (r0, r1) = (rim[i], rim[(i + 1) % TRIANGLES]);
        out[i] = r0.cross(r1).extend(apex.dot(r0.cross(r1))).to_array();
        out[TRIANGLES + i] = r0.cross(apex).extend(0.0).to_array();
    }
    out
}

/// The fan's apex and rim in camera space, as vanilla builds them: the
/// fan turned to the horizon the sun sets on (or rises from), its rim
/// tilted by the alpha.
fn vertices(alpha: f32, sun: Vec3) -> (Vec3, [Vec3; TRIANGLES]) {
    // sin(sunAngle) < 0 flips the fan to the other horizon; sun.z = -sin(sunAngle).
    let flip = if -sun.z < 0.0 { 180f32 } else { 0.0 };
    let (s, c) = (flip + 90.0).to_radians().sin_cos();
    let rotate_x90 = |v: Vec3| Vec3::new(v.x, -v.z, v.y);
    let apex = rotate_x90(Vec3::new(-100.0 * s, 100.0 * c, 0.0));
    let rim = std::array::from_fn(|i| {
        let a = i as f32 * std::f32::consts::TAU / TRIANGLES as f32;
        let l = Vec3::new(a.sin() * 120.0, a.cos() * 120.0, -a.cos() * 40.0 * alpha);
        rotate_x90(Vec3::new(l.x * c - l.y * s, l.x * s + l.y * c, l.z))
    });
    (apex, rim)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// terrain.wgsl's `sunrise_weight` over the normals.
    fn weight(normals: &[[f32; 4]; 2 * TRIANGLES], d: Vec3) -> f32 {
        let n = |i: usize| Vec3::from_slice(&normals[i][..3]);
        let mut best = 0.0f32;
        let mut k0 = d.dot(n(TRIANGLES));
        for i in 0..TRIANGLES {
            let k1 = d.dot(n(TRIANGLES + (i + 1) % TRIANGLES));
            let w = normals[i][3];
            if k0 * w <= 0.0 && k1 * w >= 0.0 {
                let apex = d.dot(n(i));
                let s = apex + k1 - k0;
                if s.abs() >= 1e-6
                    && s * w > 0.0
                    && apex * s >= 0.0
                    && k1 * s >= 0.0
                    && k0 * s <= 0.0
                {
                    best = best.max(apex / s);
                }
            }
            k0 = k1;
        }
        best
    }

    /// The shader's former per-pixel Möller-Trumbore over the same vertices.
    fn reference(alpha: f32, sun: Vec3, d: Vec3) -> f32 {
        let (apex, rim) = vertices(alpha, sun);
        let mut best = 0.0f32;
        for i in 0..TRIANGLES {
            let e1 = rim[i] - apex;
            let e2 = rim[(i + 1) % TRIANGLES] - apex;
            let pv = d.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-6 {
                continue;
            }
            let tv = -apex;
            let u = tv.dot(pv) / det;
            let qv = tv.cross(e1);
            let v = d.dot(qv) / det;
            let t = e2.dot(qv) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 {
                best = best.max(1.0 - u - v);
            }
        }
        best
    }

    #[test]
    fn normals_weigh_every_ray_as_the_per_pixel_fan_did() {
        let mut hits = 0;
        for (alpha, sun_z) in [(1.0f32, -0.2f32), (0.6, 0.3), (0.05, -0.9), (0.9, 0.05)] {
            let sun = Vec3::new(0.0, (1.0 - sun_z * sun_z).sqrt(), sun_z);
            let normals = normals(alpha, sun);
            for yaw in 0..180 {
                for pitch in -45..=45 {
                    let (yaw, pitch) = (
                        (yaw as f32 * 2.0).to_radians(),
                        (pitch as f32 * 2.0).to_radians(),
                    );
                    let d = Vec3::new(
                        yaw.cos() * pitch.cos(),
                        pitch.sin(),
                        yaw.sin() * pitch.cos(),
                    );
                    let (got, want) = (weight(&normals, d), reference(alpha, sun, d));
                    assert!(
                        (got - want).abs() < 1e-4,
                        "{alpha} {sun_z} {d}: {got} vs {want}"
                    );
                    hits += usize::from(want > 0.0);
                }
            }
        }
        // The fan covers part of the sky, not none of it.
        assert!(hits > 1000, "{hits}");
    }
}
