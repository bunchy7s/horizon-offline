// The world border wall (core/world_border): camera-relative wall quads
// moved by the model offset, the forcefield texture scrolled by the texture
// offset and modulated by the status colour and fade.
struct Globals {
    view_proj: mat4x4<f32>,
}
struct Border {
    color: vec4<f32>,
    // xyz: model offset, w: texture translation (u and v)
    offset: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var forcefield: texture_2d<f32>;
@group(0) @binding(2) var forcefield_sampler: sampler;
@group(0) @binding(3) var<uniform> border: Border;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vs_border(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {
    var out: Out;
    out.clip = globals.view_proj * vec4(position + border.offset.xyz, 1.0);
    out.uv = uv + vec2(border.offset.w);
    return out;
}
@fragment fn fs_border(in: Out) -> @location(0) vec4<f32> {
    let color = textureSample(forcefield, forcefield_sampler, in.uv);
    if color.a == 0.0 {
        discard;
    }
    return color * border.color;
}
