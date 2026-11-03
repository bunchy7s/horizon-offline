// GPU culling (gfx/gpu_culling.rs). `apply` writes the records that changed.
// `cull`: one invocation per section id keeps the sections the CPU's walk
// chose, writes each one's origin and counts it in its page, per block of 64
// ids. `scan`: one workgroup per page turns the blocks' counts into where
// each block's draws start, and the page's count. `scatter`: each block
// writes its sections' draws there, in id order, so a page's draws come in
// the same order every frame (atomics alone would order them as the GPU
// schedules, and where faces of two sections meet at the same depth the later
// draw keeps the pixel). Nothing here computes with floats: the origins are
// the CPU's bits, copied.

struct Params {
    // Ids to test, records `apply` writes, the arena pages the counts and
    // regions cover, and where the regions and the chosen ids' bits start in
    // `tables`.
    ids: u32,
    updates: u32,
    pages: u32,
    regions: u32,
    mask: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    // The tables' first column: x, 0, z, 0.
    first: vec4<i32>,
    // Their entries along x, y and z, and 0.
    cells: vec4<u32>,
}

// gpu_culling.rs `Record`.
struct Section {
    // Section x, y (from the level's bottom), z, and its arena page (-1: a
    // free id).
    key: vec4<i32>,
    // The first vertex of its solid quads, its solid quads, then two words
    // unused here.
    a: vec4<u32>,
}

// gpu_culling.rs `Update`: the record for id `id.x`.
struct Update {
    id: vec4<u32>,
    section: Section,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> sections: array<Section>;
// gpu_culling.rs `GpuCulling::prepare`: the origins' f32 bits along x, y and
// z, each page's region of `draws` (its first draw and its size), then a bit
// per id the CPU's walk chose.
@group(0) @binding(2) var<storage, read> tables: array<u32>;
@group(0) @binding(3) var<storage, read> updates: array<Update>;
// Sections kept, draws, each page's draws, then per block of ids and page its
// draws (`cull`), turned into where they start (`scan`).
@group(0) @binding(4) var<storage, read_write> counts: array<atomic<u32>>;
// DrawIndexedIndirectArgs, five words each.
@group(0) @binding(5) var<storage, read_write> draws: array<u32>;
// Per id the instance it is drawn with: origin and UV step, as f32 bits.
@group(0) @binding(6) var<storage, read_write> origins: array<vec4<u32>>;
// Per id its page + 1 when it is drawn this frame, else 0.
@group(0) @binding(7) var<storage, read_write> runs: array<u32>;

// gfx.rs `section_instance`'s w for a whole section: 1 / mesher::UV_STEPS.
const STEP_BITS: u32 = 0x38000000u;
// Workgroups along x when the ids need more than one row of them.
const ROW: u32 = 65535u;
// Ids per block.
const BLOCK: u32 = 64u;
// Counts before the pages'.
const STATS: u32 = 2u;

// The count of `block`'s draws in `page` (`cull`), then where they start (`scan`).
fn block_count(block: u32, page: u32) -> u32 {
    return STATS + params.pages + block * params.pages + page;
}

// `page`'s region of `draws`: its first draw and its size.
fn region(page: u32) -> vec2<u32> {
    let at = params.regions + 2u * page;
    return vec2<u32>(tables[at], tables[at + 1u]);
}

// The CPU's walk chose it (gpu_culling.rs `GpuCulling::choose`).
fn chosen(id: u32) -> bool {
    return ((tables[params.mask + id / 32u] >> (id % 32u)) & 1u) != 0u;
}

// One invocation per changed record; the CPU lists each id once.
@compute @workgroup_size(64)
fn apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x < params.updates {
        let u = updates[gid.x];
        sections[u.id.x] = u.section;
    }
}

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let id = gid.y * (ROW * BLOCK) + gid.x;
    if id >= params.ids {
        return;
    }
    runs[id] = 0u;
    let s = sections[id];
    if s.key.w < 0 || s.a.y == 0u || !chosen(id) {
        return;
    }
    let page = u32(s.key.w);
    // The tables hold every column within the distance; a key before their
    // first wraps past their end.
    let c = vec3<u32>(s.key.xyz - params.first.xyz);
    if page >= params.pages || any(c >= params.cells.xyz) {
        return;
    }
    let nx = params.cells.x;
    let ny = params.cells.y;
    origins[id] = vec4<u32>(tables[c.x], tables[nx + c.y], tables[nx + ny + c.z], STEP_BITS);
    runs[id] = page + 1u;
    atomicAdd(&counts[block_count(id / BLOCK, page)], 1u);
}

var<workgroup> partial: array<u32, 256>;

// One workgroup per page: each lane sums a stretch of blocks, the lanes'
// sums are scanned, and each block's sum becomes where its draws start.
@compute @workgroup_size(256)
fn scan(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let page = group.x;
    let blocks = (params.ids + BLOCK - 1u) / BLOCK;
    let stretch = (blocks + 255u) / 256u;
    let first = min(lane * stretch, blocks);
    let last = min(first + stretch, blocks);
    var sum = 0u;
    for (var b = first; b < last; b++) {
        sum += atomicLoad(&counts[block_count(b, page)]);
    }
    partial[lane] = sum;
    workgroupBarrier();
    for (var step = 1u; step < 256u; step <<= 1u) {
        var before = 0u;
        if lane >= step {
            before = partial[lane - step];
        }
        workgroupBarrier();
        partial[lane] += before;
        workgroupBarrier();
    }
    var start = 0u;
    if lane > 0u {
        start = partial[lane - 1u];
    }
    for (var b = first; b < last; b++) {
        let count = atomicLoad(&counts[block_count(b, page)]);
        atomicStore(&counts[block_count(b, page)], start);
        start += count;
    }
    if lane == 255u {
        // A region has room for every record of its page; were one short,
        // its draws past it go.
        let total = min(partial[255], region(page).y);
        atomicStore(&counts[STATS + page], total);
        atomicAdd(&counts[0], partial[255]);
        atomicAdd(&counts[1], total);
    }
}

var<workgroup> held: array<u32, 64>;

// One workgroup per block of ids: each section's draw goes after those of the
// block's earlier sections in its page.
@compute @workgroup_size(64)
fn scatter(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let block = group.y * ROW + group.x;
    let id = block * BLOCK + lane;
    var mine = 0u;
    if id < params.ids {
        mine = runs[id];
    }
    held[lane] = mine;
    workgroupBarrier();
    if mine == 0u {
        return;
    }
    let page = mine - 1u;
    var slot = atomicLoad(&counts[block_count(block, page)]);
    for (var other = 0u; other < lane; other++) {
        if held[other] == mine {
            slot += 1u;
        }
    }
    let r = region(page);
    if slot >= r.y {
        return;
    }
    // All of its solid quads, from its first vertex, with its origin.
    let s = sections[id];
    let d = (r.x + slot) * 5u;
    draws[d] = s.a.y * 6u;
    draws[d + 1u] = 1u;
    draws[d + 2u] = 0u;
    draws[d + 3u] = s.a.x;
    draws[d + 4u] = id;
}
