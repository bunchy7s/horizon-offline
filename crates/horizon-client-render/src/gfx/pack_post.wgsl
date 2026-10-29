// Helpers of a shader pack's post-processing (pack_post.rs): the world into colortex0 and
// depthtex0 (a depth attachment, for terrain's depth test), and colortex0 onto the screen for
// packs without `final`.

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var world_depth: texture_depth_2d;

struct World {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// Pack buffers keep GL's rows (row 0 at the bottom), and GL's window depth: Horizon's
// reverse-Z depth is 1 - d for the same near and far planes.
@fragment
fn fs_blit(@builtin(position) p: vec4<f32>) -> World {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(i32(p.x), size.y - 1 - i32(p.y));
    var out: World;
    out.color = textureLoad(source, at, 0);
    out.depth = 1.0 - textureLoad(world_depth, at, 0);
    return out;
}

// The same without depthtex0, for packs that never read it.
@fragment
fn fs_blit_color(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    return textureLoad(source, vec2<i32>(i32(p.x), size.y - 1 - i32(p.y)), 0);
}

@group(0) @binding(2) var linear: sampler;

struct Full {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_uv(@builtin(vertex_index) i: u32) -> Full {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    // Screen top (uv.y = 0 in wgpu) shows the buffer's top row, GL's last.
    return Full(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), vec2<f32>(uv.x, 1.0 - uv.y));
}

@fragment
fn fs_copy(v: Full) -> @location(0) vec4<f32> {
    return textureSample(source, linear, v.uv);
}

// One level of a buffer's mip chain from the level above (6.1.7): the average of the 2×2
// texels under each destination texel, by one bilinear sample at their shared corner.
@fragment
fn fs_down(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = max(textureDimensions(source) / 2u, vec2<u32>(1u));
    return textureSampleLevel(source, linear, p.xy / vec2<f32>(size), 0.0);
}
