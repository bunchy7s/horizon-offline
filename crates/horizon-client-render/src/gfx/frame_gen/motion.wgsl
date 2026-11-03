// Frame generation's motion vectors for what moves on its own (mobs, dropped items): each
// vertex drawn where the world pass drew it, with where it was last frame; only the pixels
// the world shows it at keep its vector: where the depth is its own, or, for translucent
// items (which don't write depth), where its texture shows and nothing is in front.

struct Params {
    // The world's projection this frame (TAA's jitter in it), the same without the jitter,
    // and last frame's projection from this frame's camera.
    jittered: mat4x4<f32>,
    current: mat4x4<f32>,
    previous: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) current: vec4<f32>,
    @location(1) previous: vec4<f32>,
    @location(2) uv: vec2<f32>,
    // 1: a translucent item.
    @location(3) @interpolate(flat) translucent: u32,
};

@vertex fn vs_motion(@location(0) position: vec3<f32>, @location(1) previous: vec3<f32>,
    @location(2) uv: vec2<f32>, @location(3) translucent: u32) -> Out {
    var out: Out;
    out.uv = uv;
    out.translucent = translucent;
    out.clip = params.jittered * vec4(position, 1.0);
    out.current = params.current * vec4(position, 1.0);
    out.previous = params.previous * vec4(previous, 1.0);
    return out;
}

// xy: the motion vector in UV (to where the pixel was last frame, FSR's convention), z: 1.
@fragment fn fs_motion(in: Out) -> @location(0) vec4<f32> {
    let stored = textureLoad(depth, vec2<i32>(in.clip.xy), 0);
    let tolerance = stored * 1e-3 + 1e-7;
    // Not what the world shows here: a transparent texel, or behind something.
    if in.translucent != 0u {
        let alpha = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0).a;
        if alpha < 0.1 || in.clip.z < stored - tolerance {
            discard;
        }
    } else if abs(in.clip.z - stored) > tolerance {
        discard;
    }
    if in.previous.w <= 0.0 {
        discard;
    }
    let now = in.current.xy / in.current.w;
    let then = in.previous.xy / in.previous.w;
    return vec4((then - now) * vec2(0.5, -0.5), 1.0, 0.0);
}
