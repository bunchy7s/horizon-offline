//! What a pack's WGSL reads: the frame uniform every stage sees, which the
//! renderer fills and uploads as the Rust struct below, and the stage input
//! and output structs. The WGSL of each is generated from these definitions
//! (`Struct::wgsl`), and so is the reference (docs/api/shaders-reference.md).

/// One field of a WGSL struct, for the prelude and the reference.
#[derive(Debug)]
pub struct Field {
    pub name: &'static str,
    pub ty: &'static str,
    /// Its meaning, unit and range.
    pub doc: &'static str,
}

/// A struct a pack's WGSL can name.
#[derive(Debug)]
pub struct Struct {
    pub name: &'static str,
    pub doc: &'static str,
    pub fields: &'static [Field],
}

impl Struct {
    pub fn wgsl(&self) -> String {
        let mut out = format!("struct {} {{\n", self.name);
        for f in self.fields {
            out += &format!("    {}: {},\n", f.name, f.ty);
        }
        out + "}\n"
    }
}

/// The WGSL type of a Rust field the renderer uploads.
pub trait Wgsl {
    const TYPE: &'static str;
}

macro_rules! wgsl_types {
    ($($rust:ty => $wgsl:literal),* $(,)?) => {
        $(impl Wgsl for $rust {
            const TYPE: &'static str = $wgsl;
        })*
    };
}

wgsl_types! {
    f32 => "f32",
    u32 => "u32",
    i32 => "i32",
    [f32; 2] => "vec2<f32>",
    [f32; 3] => "vec3<f32>",
    [f32; 4] => "vec4<f32>",
    [[f32; 4]; 4] => "mat4x4<f32>",
    [[f32; 4]; 8] => "array<vec4<f32>, 8>",
}

/// A uniform struct: the Rust type the renderer uploads, and its WGSL.
macro_rules! uniform {
    ($(#[doc = $doc:literal])* $name:ident {
        $($(#[doc = $fdoc:literal])* $field:ident: $ty:ty,)*
    }) => {
        $(#[doc = $doc])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name {
            $($(#[doc = $fdoc])* pub $field: $ty,)*
        }

        impl $name {
            pub const WGSL: Struct = Struct {
                name: stringify!($name),
                doc: concat!($($doc, "\n"),*),
                fields: &[$(Field {
                    name: stringify!($field),
                    ty: <$ty as Wgsl>::TYPE,
                    doc: concat!($($fdoc),*),
                }),*],
            };
            /// Each field's byte offset, in `WGSL.fields`' order.
            pub const OFFSETS: &'static [usize] = &[$(std::mem::offset_of!($name, $field)),*];
        }
    };
}

uniform! {
    /// The frame, as `frame` in every stage. Positions and directions are
    /// in world axes relative to the camera (x east, y up, z south), so they
    /// stay precise far from the world's origin; depth is reversed (1 near,
    /// 0 at the far plane and in the sky).
    Frame {
        /// Camera-relative world to view space: the camera's rotation.
        view: [[f32; 4]; 4],
        /// View to clip space (reversed depth, wgpu's clip space: z 0..1).
        projection: [[f32; 4]; 4],
        /// View to camera-relative world space.
        view_inverse: [[f32; 4]; 4],
        /// Clip to view space.
        projection_inverse: [[f32; 4]; 4],
        /// The camera's world position, in blocks (about a millimetre of precision 10 000 blocks out).
        camera_position: [f32; 3],
        /// Seconds since the pack was loaded, wrapping to 0 every hour.
        time: f32,
        /// Unit vector toward the sun (below the horizon at night).
        sun_direction: [f32; 3],
        /// The time of day, 0..1: 0 sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight.
        day_time: f32,
        /// Unit vector toward the moon.
        moon_direction: [f32; 3],
        /// Vanilla's GameTime: (game time % 24000 + partial tick) / 24000, 0..1.
        game_time: f32,
        /// Vanilla's sky colour this frame, 0..1.
        sky_color: [f32; 3],
        /// Rain strength, 0..1.
        rain: f32,
        /// Vanilla's fog colour this frame (also the clear colour), 0..1.
        fog_color: [f32; 3],
        /// Thunder strength, 0..1.
        thunder: f32,
        /// The size of the world's picture, in pixels (the resolution scale applied).
        resolution: [f32; 2],
        /// Block and sky light at the camera, 0..1 (level / 15).
        eye_light: [f32; 2],
        /// Where vanilla's fog starts, in blocks from the camera.
        fog_start: f32,
        /// Where vanilla's fog is full, in blocks.
        fog_end: f32,
        /// The render distance, in blocks.
        render_distance: f32,
        /// The far plane, in blocks.
        far: f32,
        /// Night vision strength, 0..1.
        night_vision: f32,
        /// Blindness strength, 0..1.
        blindness: f32,
        /// The darkness effect's strength, 0..1.
        darkness: f32,
        /// The Brightness option, 0..1 (Moody to Bright).
        brightness: f32,
        /// What the camera is in: 0 air, 1 water, 2 lava, 3 powder snow.
        eye_in: u32,
        /// 0 the Overworld, -1 the Nether, 1 the End, 2 any other dimension.
        dimension: i32,
        /// Frames drawn since the pack was loaded (wraps).
        frame_index: u32,
        /// 1 while F1 hides the HUD, else 0.
        hide_gui: u32,
        /// What falls in the biome at the camera while it rains: 0 nothing, 1 rain, 2 snow.
        precipitation: u32,
        /// The biome at the camera's temperature (vanilla's, -0.7..2.0; it snows below 0.15).
        temperature: f32,
        /// The biome at the camera's downfall, 0..1.
        downfall: f32,
        /// `rain`, smoothed: rises over about 30 seconds and dries over about 10, 0..1.
        wetness: f32,
        /// Last frame's camera-relative world to clip space (its `projection * view`); a point
        /// at camera-relative `p` now was at `p + camera_motion` relative to last frame's camera.
        /// This frame's on the first frame and after a gap of over half a second (a pause, a
        /// world load), when last frame's camera says nothing about this one.
        previous_view_projection: [[f32; 4]; 4],
        /// How far the camera moved since last frame, in blocks (0 after such a gap).
        camera_motion: [f32; 3],
        /// Seconds since last frame (0 on the first and after such a gap).
        delta_time: f32,
        /// `eye_light` eased with a half-life of 2 seconds: it follows the light levels' steps
        /// without stepping itself (for exposure and indoor fog).
        eye_light_smooth: [f32; 2],
        /// 1 while the camera is in water (`eye_in` 1), eased with a half-life of a quarter
        /// second in and out.
        in_water: f32,
        /// The same for lava (`eye_in` 2).
        in_lava: f32,
        /// The y of the horizontal water plane `reflection` mirrors the world about, in world
        /// blocks, while `has_reflection` is 1.
        water_plane: f32,
        /// 1 while the material stage's `reflection` holds this frame's water mirror, else 0.
        has_reflection: u32,
        /// The size of `reflection`, in pixels: half the world's picture.
        reflection_resolution: [f32; 2],
        /// Eight values a native mod hands its own pack each frame (`native_shader_params`,
        /// docs/native-mods.md): what they mean is that mod's, such as where and when
        /// something happened in the world. All 0 when no mod sets them.
        mods: [[f32; 4]; 8],
        /// Unit vector toward the body casting this frame's shadows: the sun by day, the moon
        /// by night.
        light_direction: [f32; 3],
        /// 1 while `shadow_map` holds this frame's shadow map, else 0.
        has_shadow: u32,
        /// Camera-relative world to the shadow camera's clip space, before the distortion
        /// (orthographic, looking along `-light_direction`; z 0 at the light, 1 away from it).
        shadow_view_proj: [[f32; 4]; 4],
        /// Half the side of the square the shadow map covers, in blocks (the manifest's
        /// `[shadow] distance`); 0 without a shadow map.
        shadow_distance: f32,
        /// How strongly the shadow map is distorted toward its centre, 0..1: a point at
        /// shadow clip `xy` is stored at `xy / (length(xy) * k + 1 - k)`, so texels near the
        /// camera cover less ground (`shadow_position` applies it).
        shadow_distortion: f32,
        /// The shadow map's side, in texels; 0 without a shadow map.
        shadow_resolution: f32,
        /// The ground one shadow texel covers at the map's centre, in blocks (for a bias that
        /// scales with it); 0 without a shadow map.
        shadow_center_texel: f32,
    }
}

macro_rules! fields {
    ($($name:ident: $ty:literal = $doc:literal,)*) => {
        &[$(Field { name: stringify!($name), ty: $ty, doc: $doc }),*]
    };
}

/// The post stage's input: one pixel of the picture.
pub const POST_INPUT: Struct = Struct {
    name: "PostInput",
    doc: "One pixel of the picture, for `fn post(input: PostInput) -> vec4<f32>`.",
    fields: fields! {
        uv: "vec2<f32>" = "Where the pixel is, 0..1 from the top left.",
        pixel: "vec2<f32>" = "The pixel's centre, in pixels from the top left of what the pass draws (the world's size; the window's for the last pass).",
        color: "vec4<f32>" = "The previous pass's colour here (the world's picture for the first pass).",
        depth: "f32" = "The world's depth here, reversed: 0 is the sky.",
    },
};

/// The material stage's input: one point of a block's surface.
pub const MATERIAL_INPUT: Struct = Struct {
    name: "MaterialInput",
    doc: "One point of a block's surface, for `fn material(input: MaterialInput) -> Surface`.",
    fields: fields! {
        albedo: "vec4<f32>" = "The block texture's colour here (alpha 0..1).",
        tint: "vec4<f32>" = "The vertex colour: biome tint, face shade and ambient occlusion.",
        light: "vec2<f32>" = "Block and sky light, 0..1, smoothed over the face.",
        lightmap: "vec3<f32>" = "Vanilla's light colour for that light this frame.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        uv: "vec2<f32>" = "The texture coordinate inside the block atlas.",
        translucent: "bool" = "Drawn in the translucent pass (water, stained glass, ice).",
        tags: "u32" = "The pack's sprite tags this surface's sprite has: bit i for the manifest's i-th `[[tags]]`.",
        material: "u32" = "The `[[materials]]` id of this surface's sprite (0 when no entry names it).",
        screen: "vec2<f32>" = "Where the point is on the screen, 0..1 from the top left (`opaque_depth` and `opaque_color` at `screen * frame.resolution`).",
        face: "u32" = "For a fluid, which face of the block the point is on, `Direction` order: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east. 0 elsewhere.",
        fluid: "u32" = "0 not a fluid, 1 water, 2 lava.",
        emission: "f32" = "How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks.",
        normal: "vec3<f32>" = "The surface's geometric normal, a unit vector in world axes, facing the camera. A normal within about 3.6 degrees of an axis is that axis exactly, so distant block faces do not shimmer; flowing water's slopes are steeper and keep theirs.",
        tangent: "vec4<f32>" = "The surface's tangent: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). With `normal` it turns a normal map's normals into world axes.",
        colored_light: "vec3<f32>" = "The frame's coloured lights at this point (`[[particle_lights]]` and mods' visual lights), 0..1, to light the surface with as the pack likes; 0 where none reaches.",
    },
};

/// What the material stage returns.
pub const SURFACE: Struct = Struct {
    name: "Surface",
    doc: "What `material` returns. Vanilla's own is `albedo * tint * vec4(lightmap, 1.0)`; \
          Horizon then applies the alpha test (cutout and translucent) and fog.",
    fields: fields! {
        color: "vec4<f32>" = "The lit colour, 0..1, with its alpha.",
    },
};

/// The sky stage's input: one direction of the sky.
pub const SKY_INPUT: Struct = Struct {
    name: "SkyInput",
    doc: "One direction of the sky, for `fn sky(input: SkyInput) -> vec4<f32>`. The sun, moon, \
          stars and clouds are drawn over the result.",
    fields: fields! {
        direction: "vec3<f32>" = "Unit view ray, camera-relative world axes.",
        color: "vec4<f32>" = "Vanilla's sky colour along the ray (sky, fog and sunrise).",
    },
};

/// The vertex stage's input: one corner of a block's surface.
pub const VERTEX_INPUT: Struct = Struct {
    name: "VertexInput",
    doc: "One corner of a block's surface, for `fn vertex(input: VertexInput) -> vec3<f32>`, which returns where it is drawn (camera-relative, in blocks; `input.position` where it stays).",
    fields: fields! {
        position: "vec3<f32>" = "The corner, camera-relative, in blocks (`+ frame.camera_position` for the world's).",
        uv: "vec2<f32>" = "The texture coordinate inside the block's sprite.",
        material: "u32" = "The `[[materials]]` id of the block's sprite (0 when none matches).",
        light: "vec2<f32>" = "Block and sky light, 0..1.",
        tint: "vec4<f32>" = "The vertex colour (biome tint, face shade, ambient occlusion); alpha 1.",
        face: "u32" = "For a fluid, the face of the block the corner is on (as `MaterialInput.face`), else 0.",
        fluid: "u32" = "1 on a fluid's surface (water or lava), else 0.",
    },
};

/// The weather stage's input: one point of a rain or snow streak.
pub const WEATHER_INPUT: Struct = Struct {
    name: "WeatherInput",
    doc: "One point of a rain or snow streak, for `fn weather(input: WeatherInput) -> vec4<f32>`, \
          which returns the colour blended over the world (alpha its opacity).",
    fields: fields! {
        color: "vec4<f32>" = "The rain or snow texture's colour here times the streak's vertex colour (alpha 0.1..1).",
        light: "vec2<f32>" = "Block and sky light, 0..1.",
        position: "vec3<f32>" = "The point, camera-relative, in blocks.",
        snow: "bool" = "A snow streak (rain otherwise).",
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_fields_carry_their_docs() {
        let wgsl = Frame::WGSL;
        assert_eq!(wgsl.fields.len(), 49);
        assert!(wgsl.fields.iter().all(|f| !f.doc.trim().is_empty()));
        assert_eq!(std::mem::size_of::<Frame>(), 6 * 64 + 15 * 16 + 8 * 16);
        assert!(
            wgsl.wgsl()
                .contains("    camera_position: vec3<f32>,\n    time: f32,\n")
        );
    }
}
