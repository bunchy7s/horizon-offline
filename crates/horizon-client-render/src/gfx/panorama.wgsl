// CubeMap's position-only cube and panorama.vsh/fsh: sample its untransformed direction.
struct Projection { view_proj: mat4x4<f32> }
@group(0) @binding(0) var<uniform> projection: Projection;
@group(0) @binding(1) var panorama: texture_cube<f32>;
@group(0) @binding(2) var panorama_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) direction: vec3<f32>,
}
@vertex fn vs_panorama(@builtin(vertex_index) index: u32) -> Out {
    let corners = array<vec3<f32>, 8>(
        vec3(-1.0, -1.0, -1.0), vec3(1.0, -1.0, -1.0),
        vec3(1.0, 1.0, -1.0), vec3(-1.0, 1.0, -1.0),
        vec3(-1.0, -1.0, 1.0), vec3(1.0, -1.0, 1.0),
        vec3(1.0, 1.0, 1.0), vec3(-1.0, 1.0, 1.0),
    );
    let indices = array<u32, 36>(
        4, 7, 6, 4, 6, 5, 5, 6, 2, 5, 2, 1,
        1, 2, 3, 1, 3, 0, 0, 3, 7, 0, 7, 4,
        7, 3, 2, 7, 2, 6, 0, 4, 5, 0, 5, 1,
    );
    let position = corners[indices[index]];
    var out: Out;
    out.clip = projection.view_proj * vec4(position, 1.0);
    out.direction = position;
    return out;
}
@fragment fn fs_panorama(in: Out) -> @location(0) vec4<f32> {
    return textureSample(panorama, panorama_sampler, in.direction);
}
