// Full-screen camera effects (screen_effects.rs): clip-space quads reading
// one sprite rectangle of the shared 2D atlas as if it were its own texture,
// with the sampler vanilla gives it (ReloadableTexture.setSampler: linear
// when `blur`, clamp-to-edge when `clamp`, else nearest and repeat).
@group(0) @binding(1) var atlas: texture_2d<f32>;
// The block sprites' sampler (nearest), and the entity group's lightmap
// sampler (linear) for `blur`. A blurred sprite has a one-texel border in the
// atlas, wrapped as repeat reads it (hud.rs), so the hardware's bilinear
// sample is GL_LINEAR with repeat; clamp-to-edge keeps the sample point half
// a texel inside. Each sampling has its own pipeline (gfx.rs), so a pixel
// costs what it did with nearest and repeat only: one sample. Each entry
// point reads `atlas` through one sampler only: GLSL (wgpu's OpenGL backend)
// combines a texture with its sampler, and an override would not remove the
// other sampler's branch.
@group(0) @binding(2) var nearest_sampler: sampler;
@group(0) @binding(4) var linear_sampler: sampler;
override CLAMP: bool = false;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    // With clamp-to-edge, the sprite's first and last texel centres.
    @location(3) @interpolate(flat) edges: vec4<f32>,
}
@vertex fn vs_screen(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>, @location(3) color: vec4<f32>) -> Out {
    var out: Out;
    out.clip = vec4(pos, 0.0, 1.0);
    out.uv = uv;
    out.rect = rect;
    out.color = color;
    let half = 0.5 / vec2<f32>(textureDimensions(atlas));
    out.edges = vec4(rect.xy + half, rect.zw - half);
    return out;
}
// The sprite's atlas coordinates of this pixel.
fn atlas_uv(in: Out) -> vec2<f32> {
    var f = in.uv;
    if !CLAMP {
        // Repeat: one period of the sprite.
        f = fract(f);
    }
    let uv = in.rect.xy + f * (in.rect.zw - in.rect.xy);
    if CLAMP {
        return clamp(uv, in.edges.xy, in.edges.zw);
    }
    return uv;
}
fn screen(c: vec4<f32>) -> vec4<f32> {
    if c.a <= 0.0 { discard; }
    return c;
}
@fragment fn fs_screen(in: Out) -> @location(0) vec4<f32> {
    return screen(textureSampleLevel(atlas, nearest_sampler, atlas_uv(in), 0.0) * in.color);
}
@fragment fn fs_screen_blur(in: Out) -> @location(0) vec4<f32> {
    return screen(textureSampleLevel(atlas, linear_sampler, atlas_uv(in), 0.0) * in.color);
}
// RenderPipelines.VIGNETTE blends ZERO, ONE_MINUS_SRC_COLOR: output the
// texture colour; the blend state darkens the frame by it.
@fragment fn fs_vignette(in: Out) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(atlas, nearest_sampler, atlas_uv(in), 0.0) * in.color;
    return vec4(c.rgb, 1.0);
}
@fragment fn fs_vignette_blur(in: Out) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(atlas, linear_sampler, atlas_uv(in), 0.0) * in.color;
    return vec4(c.rgb, 1.0);
}
