// The built-in stand-ins for the vanilla post shaders a `post_effect` chain can
// name (`core/screenquad`, `post/rotscale`, `post/invert`, `post/blit`,
// `post/bits`, `post/color_convolve`, `post/box_blur`, `post/spiderclip`).
// Targets keep GL's rows (row 0 is the bottom edge): the passes draw with clip y
// negated (`vs_quad_gl`, `vs_rotscale`) and read their textures as they are;
// `vs_quad` with `fs_blit` is the flip between the frame and a chain's `main`.

// OutSize, InSize | BlurSize, menu blur radius.
@group(0) @binding(0) var<uniform> info: array<vec4<f32>, 2>;
// The vertex and fragment stage's uniform block, packed as std140.
@group(0) @binding(1) var<uniform> vs_cfg: array<vec4<f32>, 8>;
@group(0) @binding(2) var<uniform> fs_cfg: array<vec4<f32>, 8>;
@group(0) @binding(3) var in_tex: texture_2d<f32>;
@group(0) @binding(4) var in_smp: sampler;
@group(0) @binding(5) var blur_tex: texture_2d<f32>;
@group(0) @binding(6) var blur_smp: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) scaled: vec2<f32>,
}

fn vcf(i: u32) -> f32 {
    return vs_cfg[i / 4u][i % 4u];
}
fn fcf(i: u32) -> f32 {
    return fs_cfg[i / 4u][i % 4u];
}
fn read_in(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(in_tex, in_smp, uv, 0.0);
}
fn read_blur(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(blur_tex, blur_smp, uv, 0.0);
}

fn corner(i: u32) -> vec2<f32> {
    return vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
}

@vertex fn vs_quad(@builtin(vertex_index) i: u32) -> Out {
    let p = corner(i);
    var out: Out;
    out.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = p;
    out.scaled = p;
    return out;
}

@vertex fn vs_quad_gl(@builtin(vertex_index) i: u32) -> Out {
    let p = corner(i);
    var out: Out;
    out.clip = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    out.uv = p;
    out.scaled = p;
    return out;
}

@vertex fn vs_rotscale(@builtin(vertex_index) i: u32) -> Out {
    let p = corner(i);
    var out: Out;
    out.clip = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    out.uv = p;
    let radians = vcf(4u) * 0.0174532925;
    let c = cos(radians);
    let s = sin(radians);
    let rotated = vec2(p.x * c - p.y * s, p.y * c + p.x * s);
    out.scaled = rotated * vec2(vcf(0u), vcf(1u)) + vec2(vcf(2u), vcf(3u));
    return out;
}

@fragment fn fs_invert(in: Out) -> @location(0) vec4<f32> {
    let d = read_in(in.uv);
    let mixed = mix(d, 1.0 - d, fcf(0u));
    return vec4(mixed.rgb, 1.0);
}

@fragment fn fs_blit(in: Out) -> @location(0) vec4<f32> {
    return read_in(in.uv) * vec4(fcf(0u), fcf(1u), fcf(2u), fcf(3u));
}

@fragment fn fs_bits(in: Out) -> @location(0) vec4<f32> {
    let resolution = fcf(0u);
    let mosaic_in = info[0].zw / fcf(1u);
    let fract_pix = fract(in.uv * mosaic_in) / mosaic_in;
    var base = read_in(in.uv - fract_pix);
    let quantised = base.rgb - fract(base.rgb * resolution) / resolution;
    let luma = dot(quantised, vec3(0.3, 0.59, 0.11));
    let chroma = (quantised - luma) * 1.5;
    return vec4(luma + chroma, 1.0);
}

@fragment fn fs_color_convolve(in: Out) -> @location(0) vec4<f32> {
    let t = read_in(in.uv).rgb;
    let red = dot(t, vec3(fcf(0u), fcf(1u), fcf(2u)));
    let green = dot(t, vec3(fcf(4u), fcf(5u), fcf(6u)));
    let blue = dot(t, vec3(fcf(8u), fcf(9u), fcf(10u)));
    let rgb = vec3(red, green, blue);
    let luma = dot(rgb, vec3(0.3, 0.59, 0.11));
    return vec4((rgb - luma) * 1.8 + luma, 1.0);
}

@fragment fn fs_box_blur(in: Out) -> @location(0) vec4<f32> {
    let step = (1.0 / info[0].zw) * vec2(fcf(0u), fcf(1u));
    let configured = fcf(2u);
    var radius = info[1].z;
    if configured >= 0.5 {
        radius = round(configured);
    }
    var blurred = vec4(0.0);
    for (var a = -radius + 0.5; a <= radius; a += 2.0) {
        blurred += read_in(in.uv + step * a);
    }
    blurred += read_in(in.uv + step * radius) / 2.0;
    return blurred / (radius + 0.5);
}

@fragment fn fs_spiderclip(in: Out) -> @location(0) vec4<f32> {
    let scaled_texel = read_in(in.scaled);
    let blur_texel = read_blur(in.uv);
    let scissor = vec4(fcf(0u), fcf(1u), fcf(2u), fcf(3u));
    let vignette = vec4(fcf(4u), fcf(5u), fcf(6u), fcf(7u));
    let s = in.scaled;
    var o = scaled_texel;
    if s.x < scissor.x { o = blur_texel; }
    if s.y < scissor.y { o = blur_texel; }
    if s.x > scissor.z { o = blur_texel; }
    if s.y > scissor.w { o = blur_texel; }
    if s.x < vignette.x { o = mix(blur_texel, o, (scissor.x - s.x) / (scissor.x - vignette.x)); }
    if s.y < vignette.y { o = mix(blur_texel, o, (scissor.y - s.y) / (scissor.y - vignette.y)); }
    if s.x > vignette.z { o = mix(blur_texel, o, (scissor.z - s.x) / (scissor.z - vignette.z)); }
    if s.y > vignette.w { o = mix(blur_texel, o, (scissor.w - s.y) / (scissor.w - vignette.w)); }
    return vec4(o.rgb, 1.0);
}
