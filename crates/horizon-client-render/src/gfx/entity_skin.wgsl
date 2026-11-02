// Entity models posed on the GPU (gfx/entity_skin.rs): each job is one draw of
// a model (`mobs::render::draw`), and writes the same vertices the CPU path
// would push, in the same order, into the entity vertex buffer. One workgroup
// per job; its invocations walk the model's vertices.
//
// Words, not structs: every record is read and written as u32 words, so the
// layouts are the Rust ones byte for byte.

// A model vertex (7 words): position / 16, model UV, the face's normal as an
// axis (0..2 = +x, +y, +z; 3..5 = -x, -y, -z), the part's index in the model.
const MODEL_WORDS: u32 = 7u;
// A posed part (25 words): the geometry matrix (16, by columns), then the
// normal matrix (9, by columns), its columns already normalized when the
// part's scale is not uniform (`ModelNormals.trusted` false).
const PART_WORDS: u32 = 25u;
// A job (16 words): first model vertex, vertex count, first part, first output
// vertex, the texture rect (4 floats), scroll, colour, light, overlay,
// whether the overlay is the armour glint (its model UV in `glint_uv`), 3 spare.
const JOB_WORDS: u32 = 16u;
// `mesher::Vertex` (11 words): pos (3), uv (2), layer, color, light, normal
// (4 x i8), glint_uv (2).
const VERTEX_WORDS: u32 = 11u;

@group(0) @binding(0) var<storage, read> model: array<u32>;
@group(0) @binding(1) var<storage, read> parts: array<u32>;
@group(0) @binding(2) var<storage, read> jobs: array<u32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

fn f(words: u32) -> f32 {
    return bitcast<f32>(words);
}

fn part_f(part: u32, i: u32) -> f32 {
    return f(parts[part * PART_WORDS + i]);
}

// `mobs::render::scroll_u`.
fn scroll_u(u: f32, scroll: f32) -> f32 {
    if scroll == 0.0 {
        return u;
    }
    // Rust's `%` on floats: the remainder with the dividend's sign.
    let s = u + scroll;
    return s - trunc(s);
}

// `Polygon::vertex_normal`'s packing: clamped, times 127, cut toward zero.
fn pack(n: vec3<f32>) -> u32 {
    let b = vec3<i32>(clamp(n, vec3(-1.0), vec3(1.0)) * 127.0);
    return (u32(b.x) & 0xffu) | ((u32(b.y) & 0xffu) << 8u) | ((u32(b.z) & 0xffu) << 16u)
        | (u32(-127 & 0xff) << 24u);
}

@compute @workgroup_size(64)
fn skin(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let job = group.y * groups.x + group.x;
    if job >= arrayLength(&jobs) / JOB_WORDS {
        return;
    }
    let j = job * JOB_WORDS;
    let first = jobs[j];
    let count = jobs[j + 1u];
    let first_part = jobs[j + 2u];
    let dst = jobs[j + 3u];
    let rect = vec4(f(jobs[j + 4u]), f(jobs[j + 5u]), f(jobs[j + 6u]), f(jobs[j + 7u]));
    let scroll = f(jobs[j + 8u]);
    let color = jobs[j + 9u];
    let light = jobs[j + 10u];
    let overlay = jobs[j + 11u];
    let glint = jobs[j + 12u] != 0u;
    for (var k = lane; k < count; k += 64u) {
        let v = (first + k) * MODEL_WORDS;
        let p = vec3(f(model[v]), f(model[v + 1u]), f(model[v + 2u]));
        let uv = vec2(f(model[v + 3u]), f(model[v + 4u]));
        let axis = model[v + 5u];
        let part = first_part + model[v + 6u];
        // glam's Mat4::transform_point3, in its order.
        var pos = vec3(part_f(part, 0u), part_f(part, 1u), part_f(part, 2u)) * p.x;
        pos = vec3(part_f(part, 4u), part_f(part, 5u), part_f(part, 6u)) * p.y + pos;
        pos = vec3(part_f(part, 8u), part_f(part, 9u), part_f(part, 10u)) * p.z + pos;
        pos = vec3(part_f(part, 12u), part_f(part, 13u), part_f(part, 14u)) + pos;
        // An axis normal times the normal matrix is that column, exactly.
        let c = 16u + (axis % 3u) * 3u;
        var n = vec3(part_f(part, c), part_f(part, c + 1u), part_f(part, c + 2u));
        if axis >= 3u {
            n = -n;
        }
        let o = (dst + k) * VERTEX_WORDS;
        out[o] = bitcast<u32>(pos.x);
        out[o + 1u] = bitcast<u32>(pos.y);
        out[o + 2u] = bitcast<u32>(pos.z);
        out[o + 3u] = bitcast<u32>(rect.x + (rect.z - rect.x) * scroll_u(uv.x, scroll));
        out[o + 4u] = bitcast<u32>(rect.y + (rect.w - rect.y) * uv.y);
        out[o + 5u] = overlay;
        out[o + 6u] = color;
        out[o + 7u] = light;
        out[o + 8u] = pack(n);
        if glint {
            out[o + 9u] = bitcast<u32>(uv.x);
            out[o + 10u] = bitcast<u32>(uv.y);
        } else {
            out[o + 9u] = 0u;
            out[o + 10u] = 0u;
        }
    }
}
