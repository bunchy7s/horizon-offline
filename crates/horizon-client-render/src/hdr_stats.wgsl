// Horizon's HDR emission thresholds (gfx/hdr_stats.rs), drawn when HDR output
// turns on: for every block sprite, the brightness its pixels must reach to
// shine, measured against the sprite itself rather than a fixed level, so a
// resource pack's light sources shine where they are bright in their own
// drawing. One texel per sprite (x the layer, y the array): r for a light
// source whose bright parts shine (a torch's flame, not its handle), g for one
// that shines all over (a froglight), b 1 when the sprite has an opaque pixel.
@group(0) @binding(0) var sprites: texture_2d_array<f32>;
@group(0) @binding(1) var sprites_1: texture_2d_array<f32>;
@group(0) @binding(2) var sprites_2: texture_2d_array<f32>;
@group(0) @binding(3) var sprites_3: texture_2d_array<f32>;
@group(0) @binding(4) var sprites_4: texture_2d_array<f32>;
@group(0) @binding(5) var sprites_5: texture_2d_array<f32>;
@group(0) @binding(6) var sprites_6: texture_2d_array<f32>;
@group(0) @binding(7) var sprites_7: texture_2d_array<f32>;

struct Params {
    // Where between the sprite's darkest and brightest opaque pixel the
    // threshold lies, for bright parts and for all over (hdr.rs `Masks`).
    parts: f32,
    whole: f32,
};
@group(0) @binding(8) var<uniform> params: Params;

// hdr.wgsl's brightness: the mean of luminance and the brightest channel, in
// linear light.
fn brightness(c: vec3<f32>) -> f32 {
    let l = pow(max(c, vec3(0.0)), vec3(2.2));
    return 0.5 * (dot(l, vec3(0.2126, 0.7152, 0.0722)) + max(l.r, max(l.g, l.b)));
}

fn texel(array: u32, at: vec2<i32>, layer: i32) -> vec4<f32> {
    switch array {
        case 0u: { return textureLoad(sprites, at, layer, 0); }
        case 1u: { return textureLoad(sprites_1, at, layer, 0); }
        case 2u: { return textureLoad(sprites_2, at, layer, 0); }
        case 3u: { return textureLoad(sprites_3, at, layer, 0); }
        case 4u: { return textureLoad(sprites_4, at, layer, 0); }
        case 5u: { return textureLoad(sprites_5, at, layer, 0); }
        case 6u: { return textureLoad(sprites_6, at, layer, 0); }
        default: { return textureLoad(sprites_7, at, layer, 0); }
    }
}

@vertex fn vs_stats(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_stats(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let layer = i32(pos.x);
    let array = u32(pos.y);
    // Every array holds sprites of the same size.
    let size = vec2<i32>(textureDimensions(sprites));
    var lo = 1.0;
    var hi = 0.0;
    var opaque = false;
    for (var y = 0; y < size.y; y++) {
        for (var x = 0; x < size.x; x++) {
            let c = texel(array, vec2(x, y), layer);
            if c.a >= 0.5 {
                let b = brightness(c.rgb);
                lo = min(lo, b);
                hi = max(hi, b);
                opaque = true;
            }
        }
    }
    if !opaque {
        return vec4(1.0, 1.0, 0.0, 1.0);
    }
    return vec4(mix(lo, hi, params.parts), mix(lo, hi, params.whole), 1.0, 1.0);
}
