// Flat section draws (gfx/flat.rs): copies each section draw's indices into
// one index list, in draw order, offset by the draw's base vertex. Solid
// draws index quads (0 1 2 0 2 3 per quad, as Renderer::quad_indices);
// translucent ones read their sorted indices.
struct Range {
    count: u32,
    // First sorted index, or QUADS for a solid draw (`flatten_seen`: its
    // entry in occlusion culling's phase 2 draws).
    first: u32,
    base: u32,
    // First index in the list (`flatten_seen`: its draw call).
    dst: u32,
}
const QUADS: u32 = 0xffffffffu;
const GROUP: u32 = 64u;
@group(0) @binding(0) var<storage, read> ranges: array<Range>;
@group(0) @binding(1) var<storage, read> sorted: array<u32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

// Index `k` of a solid draw: quad k / 6's vertices 0 1 2 0 2 3.
fn quad(k: u32) -> u32 {
    let c = k % 6u;
    return (k / 6u) * 4u + select(select(c - 2u, 0u, c == 3u), c, c < 3u);
}

@compute @workgroup_size(64)
fn flatten(@builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32) {
    let r = group.y * 65535u + group.x;
    if r >= arrayLength(&ranges) {
        return;
    }
    let d = ranges[r];
    for (var k = lane; k < d.count; k += GROUP) {
        var i: u32;
        if d.first == QUADS {
            i = quad(k);
        } else {
            i = sorted[d.first + k];
        }
        out[d.dst + k] = d.base + i;
    }
}

// Occlusion culling's phase 2 (gfx/cull.rs), after its test: the draws as
// the test left them (`DrawIndexedIndirectArgs`, the instance count 1 where
// it saw the section), and phase 2's draw calls, whose index counts start
// at 0. A draw the test saw takes the next indices of its call's run.
@group(0) @binding(3) var<storage, read> draws: array<u32>;
@group(0) @binding(4) var<storage, read_write> calls: array<atomic<u32>>;
const UNSEEN: u32 = 0xffffffffu;
var<workgroup> start: u32;

@compute @workgroup_size(64)
fn flatten_seen(@builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32) {
    let r = group.y * 65535u + group.x;
    if r >= arrayLength(&ranges) {
        return;
    }
    let d = ranges[r];
    if lane == 0u {
        var s = UNSEEN;
        if draws[d.first * 5u + 1u] != 0u {
            s = atomicLoad(&calls[d.dst * 5u + 2u]) + atomicAdd(&calls[d.dst * 5u], d.count);
        }
        start = s;
    }
    let s = workgroupUniformLoad(&start);
    if s == UNSEEN {
        return;
    }
    for (var k = lane; k < d.count; k += GROUP) {
        out[s + k] = d.base + quad(k);
    }
}
