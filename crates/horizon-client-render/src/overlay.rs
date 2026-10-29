//! OverlayTexture: the hurt red and white flash entity.wgsl mixes over a lit
//! entity before the lightmap (entity.fsh), and the overlay codes models carry.
//!
//! Vanilla's 16 by 16 texture: rows 0 to 7 are the hurt red, row 10 is white
//! with an alpha falling with u; LivingEntityRenderer.getOverlayCoords picks
//! u from the white overlay progress and row 3 when the entity is hurt.

/// The overlay code of white flashes: `WHITE + u`, u in 1..=15.
pub const WHITE: u32 = 16;
/// The hurt red (rows 0 to 7) as RGBA bytes: 0xB2FF0000.
pub const HURT_TEXEL: [u8; 4] = [255, 0, 0, 0xB2];

/// The white row's alpha at column u: `(int) ((1 - u / 15 * 0.75) * 255)`
/// in float precision.
pub fn white_alpha(u: u32) -> u8 {
    ((1.0f32 - u as f32 / 15.0 * 0.75) * 255.0) as u8
}

/// OverlayTexture.u: the column of a white overlay progress.
pub fn white_column(progress: f32) -> u32 {
    (progress * 15.0) as u32
}

/// LivingEntityRenderer.getOverlayCoords as entity.wgsl's overlay code: the
/// red row hides the white column; column 0 of the white row is opaque white
/// with full alpha, so no overlay.
pub fn living(white_progress: f32, red: bool) -> u32 {
    if red {
        return 1;
    }
    match white_column(white_progress) {
        0 => 0,
        u => WHITE + u.min(15),
    }
}

/// CreeperRenderer.getWhiteOverlayProgress: flashing every other tenth of
/// the swelling, at least half white.
pub fn creeper_progress(swelling: f32) -> f32 {
    if (swelling * 10.0) as i32 % 2 == 0 {
        0.0
    } else {
        swelling.clamp(0.5, 1.0)
    }
}

/// The texels as WGSL constants, prepended to entity.wgsl.
pub fn wgsl_constants() -> String {
    let [r, g, b, a] = HURT_TEXEL.map(|v| format!("{v}.0 / 255.0"));
    let white: Vec<String> = (0..16)
        .map(|u| format!("{}.0 / 255.0", white_alpha(u)))
        .collect();
    format!(
        "// OverlayTexture (overlay.rs): the hurt texel and the white row's alpha by column.\n\
         const HURT_OVERLAY = vec4({r}, {g}, {b}, {a});\n\
         const WHITE_OVERLAY_ALPHA = array<f32, 16>({});\n",
        white.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texels_are_vanilla_s() {
        // OverlayTexture's constructor, printed by the 26.3 JVM.
        let vanilla = [
            255, 242, 229, 216, 203, 191, 178, 165, 152, 140, 127, 114, 101, 89, 76, 63,
        ];
        assert_eq!((0..16).map(white_alpha).collect::<Vec<_>>(), vanilla);
        assert_eq!(
            u32::from_be_bytes([HURT_TEXEL[3], HURT_TEXEL[0], HURT_TEXEL[1], HURT_TEXEL[2]]),
            0xB2FF_0000
        );
    }

    #[test]
    fn creeper_flashes_like_vanilla() {
        // (swelling, OverlayTexture.u(getWhiteOverlayProgress)) from the JVM.
        let vanilla = [
            (0.05, 0),
            (0.1, 7),
            (0.19, 7),
            (0.2, 0),
            (0.3, 7),
            (0.49, 0),
            (0.5, 7),
            (0.55, 8),
            (0.7, 10),
            (0.75, 11),
            (0.95, 14),
            (1.0, 0),
        ];
        for (swelling, u) in vanilla {
            assert_eq!(white_column(creeper_progress(swelling)), u, "{swelling}");
        }
        assert_eq!(living(creeper_progress(0.55), false), WHITE + 8);
        assert_eq!(living(creeper_progress(0.55), true), 1);
        assert_eq!(living(0.0, false), 0);
    }
}
