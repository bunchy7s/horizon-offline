// Vanilla's post/box_blur.fsh: one direction of a box blur of 2·Radius + 1
// pixels, reading two pixels per linear sample.
struct Blur {
    // InSize (pixels), BlurDir.
    size_dir: vec4<f32>,
    // Radius, unused, unused, unused.
    radius: vec4<f32>,
}
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> blur: Blur;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2(p.x, 1.0 - p.y);
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let step = blur.size_dir.zw / blur.size_dir.xy;
    let radius = round(blur.radius.x);
    var blurred = vec4(0.0);
    for (var a = -radius + 0.5; a <= radius; a += 2.0) {
        blurred += textureSampleLevel(src, samp, in.uv + step * a, 0.0);
    }
    blurred += textureSampleLevel(src, samp, in.uv + step * radius, 0.0) / 2.0;
    return blurred / (radius + 0.5);
}
