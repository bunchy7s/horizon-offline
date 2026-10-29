// Animated block sprites (gfx/animate.rs), as vanilla's animate_sprite_blit
// and animate_sprite_interpolate shaders draw them: each job writes its
// frame, or the frame mixed with the next one, into one mip level of its
// sprite array layer. One invocation per texel; the job is the z coordinate.
struct Job {
    layer: u32,
    // Word offsets of the two frames' mip chains in `frames`.
    current: u32,
    next: u32,
    // Thousandths of the way from `current` to `next` (0: `current` alone).
    millis: u32,
    // Edge of the chains' first level.
    edge: u32,
}
@group(0) @binding(0) var<storage, read> frames: array<u32>;
@group(0) @binding(1) var<storage, read> jobs: array<Job>;
@group(0) @binding(2) var level: texture_storage_2d_array<rgba8unorm, write>;

fn channels(rgba: u32) -> vec4<u32> {
    return vec4(rgba & 0xffu, (rgba >> 8u) & 0xffu, (rgba >> 16u) & 0xffu, rgba >> 24u);
}

@compute @workgroup_size(8, 8)
fn animate(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level).x;
    if id.x >= size || id.y >= size {
        return;
    }
    let job = jobs[id.z];
    // mipmap::generate: the levels at least as large as the chain's first
    // repeat it by whole pixels, the smaller ones are the chain's own.
    var texel: u32;
    if size >= job.edge {
        let scale = size / job.edge;
        texel = id.y / scale * job.edge + id.x / scale;
    } else {
        // Every level above this one, each a quarter of the one before.
        texel = 4u * (job.edge * job.edge - size * size) / 3u + id.y * size + id.x;
    }
    var color = channels(frames[job.current + texel]);
    if job.millis > 0u {
        // mix(current, next, millis / 1000), rounded to the nearest 8-bit
        // value (halves up) in integers: the same on every GPU.
        let next = channels(frames[job.next + texel]);
        color = (color * (1000u - job.millis) + next * job.millis + 500u) / 1000u;
    }
    textureStore(level, id.xy, job.layer, vec4<f32>(color) / 255.0);
}
