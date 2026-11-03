// This file is part of the FidelityFX SDK.
//
// Copyright (C) 2024 Advanced Micro Devices, Inc.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files(the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and /or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions :
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

// FidelityFX SDK 1.1.4's frame interpolation (ffx_frameinterpolation_*.h), ported to WGSL
// for Horizon, for an SDR back buffer, inverted finite depth, the whole screen as the
// interpolation rect and no distortion field. What differs from AMD's (interpolation.rs
// lists why):
// - the motion vectors are the camera's, from the depth (`prepare`), already unjittered,
//   or an entity's own where `object_motion` has one;
// - the vector fields and reconstructed depths are storage buffers of atomics;
// - both inpainting pyramids are a pass per level, and textures of their own;
// - the interpolation writes one texture and the inpainting another (rgba16float can't
//   be read_write everywhere);
// - NaNs AMD's hardware turns into 0 (max(0, NaN)) are avoided up front.

struct Params {
    // This frame's unjittered clip space to the previous frame's.
    reproject: mat4x4<f32>,
    device_to_view: vec4<f32>,
    render_size: vec2<i32>,
    display_size: vec2<i32>,
    // FSR's jitterOffset: render pixels.
    jitter: vec2<f32>,
    optical_flow_scale: vec2<f32>,
    // GetOpticalFlowSize2.
    optical_flow_size: vec2<i32>,
    reset: u32,
    tan_half_fov: f32,
    // A mip level's size, for the pyramid passes.
    level_size: vec2<i32>,
    _pad: vec2<u32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var linear_clamp: sampler;
@group(0) @binding(2) var input_depth: texture_depth_2d;
@group(0) @binding(3) var current_color: texture_2d<f32>;
@group(0) @binding(4) var previous_color: texture_2d<f32>;
@group(0) @binding(5) var dilated_depth_out: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var dilated_mv_out: texture_storage_2d<rg32float, write>;
@group(0) @binding(7) var dilated_depth: texture_2d<f32>;
@group(0) @binding(8) var dilated_mv: texture_2d<f32>;
@group(0) @binding(9) var<storage, read_write> rec_prev_depth: array<atomic<u32>>;
@group(0) @binding(10) var<storage, read_write> rec_interp_depth: array<atomic<u32>>;
@group(0) @binding(11) var<storage, read> rec_prev_depth_in: array<u32>;
@group(0) @binding(12) var<storage, read> rec_interp_depth_in: array<u32>;
@group(0) @binding(13) var<storage, read_write> game_field_x: array<atomic<u32>>;
@group(0) @binding(14) var<storage, read_write> game_field_y: array<atomic<u32>>;
@group(0) @binding(15) var<storage, read> game_field_x_in: array<u32>;
@group(0) @binding(16) var<storage, read> game_field_y_in: array<u32>;
@group(0) @binding(17) var<storage, read_write> of_field_x: array<atomic<u32>>;
@group(0) @binding(18) var<storage, read_write> of_field_y: array<atomic<u32>>;
@group(0) @binding(19) var<storage, read> of_field_x_in: array<u32>;
@group(0) @binding(20) var<storage, read> of_field_y_in: array<u32>;
@group(0) @binding(21) var optical_flow: texture_2d<i32>;
@group(0) @binding(22) var<storage, read> scene_change: array<u32, 3>;
@group(0) @binding(23) var<storage, read_write> counters: array<u32, 2>;
@group(0) @binding(24) var<storage, read> counters_in: array<u32, 2>;
// A pyramid's previous level, and the level a pyramid pass writes.
@group(0) @binding(25) var pyramid_in: texture_2d<f32>;
@group(0) @binding(26) var pyramid_out: texture_storage_2d<rgba16float, write>;
// The whole game vector field and inpainting pyramids.
@group(0) @binding(27) var game_pyramid: texture_2d<f32>;
@group(0) @binding(28) var inpainting_pyramid: texture_2d<f32>;
@group(0) @binding(29) var disocclusion_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(30) var disocclusion: texture_2d<f32>;
@group(0) @binding(31) var interpolated_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(32) var interpolated: texture_2d<f32>;
@group(0) @binding(33) var output: texture_storage_2d<rgba16float, write>;
// What moved on its own (motion.wgsl): xy its motion vector where z is 1.
@group(0) @binding(34) var object_motion: texture_2d<f32>;

const EPSILON: f32 = 1e-03;
const RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD: f32 = EPSILON;
const COUNTER_FRAME_INDEX_SINCE_LAST_RESET: u32 = 1u;

fn render_index(p: vec2<i32>) -> u32 {
    return u32(p.y * params.render_size.x + p.x);
}

fn is_on_screen(p: vec2<i32>, size: vec2<i32>) -> bool {
    return all(vec2<u32>(p) < vec2<u32>(size));
}

fn is_uv_inside(uv: vec2<f32>) -> bool {
    return uv.x > 0.0 && uv.x < 1.0 && uv.y > 0.0 && uv.y < 1.0;
}

fn min_divided_by_max(v0: f32, v1: f32) -> f32 {
    let m = max(v0, v1);
    return select(0.0, min(v0, v1) / m, m != 0.0);
}

fn normalized_dot3(v0: vec3<f32>, v1: vec3<f32>) -> f32 {
    let m = max(length(v0), length(v1));
    return select(1.0, dot(v0 / m, v1 / m), m > 0.0);
}

fn linear_from_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow(c / 1.055 + 0.055 / 1.055, vec3(2.4)), c / 12.92, c < vec3(0.04045));
}

fn raw_rgb_to_luminance(c: vec3<f32>) -> f32 {
    return dot(linear_from_srgb(c), vec3(0.2126, 0.7152, 0.0722));
}

struct Bilinear {
    base: vec2<i32>,
    weights: vec4<f32>,
};

fn bilinear(uv: vec2<f32>, size: vec2<i32>) -> Bilinear {
    let px = uv * vec2<f32>(size) - 0.5;
    let f = fract(px);
    var b: Bilinear;
    b.base = vec2<i32>(floor(px));
    b.weights = vec4(
        (1.0 - f.x) * (1.0 - f.y),
        f.x * (1.0 - f.y),
        (1.0 - f.x) * f.y,
        f.x * f.y,
    );
    return b;
}

fn bilinear_offset(i: i32) -> vec2<i32> {
    return vec2(i & 1, i >> 1u);
}

fn device_to_view_depth(d: f32) -> f32 {
    return params.device_to_view.y / (d - params.device_to_view.x);
}

fn view_space_position(px: vec2<i32>, size: vec2<i32>, d: f32) -> vec3<f32> {
    let z = device_to_view_depth(d);
    let ndc = vec2<f32>(px) / vec2<f32>(size) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return vec3(params.device_to_view.z * ndc.x * z, params.device_to_view.w * ndc.y * z, z);
}

// MOTION VECTOR FIELD (ffx_frameinterpolation_common.h)
const PRIMARY_VECTOR_BIT: u32 = 0x80000000u;
const PRIORITY_LOW_MAX: u32 = 31u;
const PRIORITY_HIGH_MAX: u32 = 1023u;
const PRIORITY_LOW_OFFSET: u32 = 16u;
const PRIORITY_HIGH_OFFSET: u32 = 21u;

struct VectorFieldEntry {
    motion_vector: vec2<f32>,
    high_priority: f32,
    low_priority: f32,
    valid: bool,
    primary: bool,
    secondary: bool,
    in_painted: bool,
    velocity: f32,
    neg_outside: bool,
    pos_outside: bool,
};

fn new_entry() -> VectorFieldEntry {
    return VectorFieldEntry(vec2(0.0), 0.0, 0.0, false, false, false, false, 0.0, false, false);
}

fn is_primary(packed: u32) -> bool {
    return (packed & PRIMARY_VECTOR_BIT) != 0u;
}

// ffxF32ToF16 and its inverse in integer arithmetic: pack2x16float needs a capability
// some GPUs lack.
fn f32_to_f16(v: f32) -> u32 {
    let x = bitcast<u32>(v);
    let sign = (x >> 16u) & 0x8000u;
    let e = i32((x >> 23u) & 0xffu) - 112;
    let m = x & 0x7fffffu;
    if e >= 31 {
        return sign | 0x7c00u;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let full = m | 0x800000u;
        let shift = u32(14 - e);
        return sign | ((full >> shift) + ((full >> (shift - 1u)) & 1u));
    }
    return sign | (((u32(e) << 10u) | (m >> 13u)) + ((m >> 12u) & 1u));
}

fn f16_to_f32(h: u32) -> f32 {
    let sign = (h & 0x8000u) << 16u;
    let e = (h >> 10u) & 0x1fu;
    let m = h & 0x3ffu;
    if e == 0u {
        let v = f32(m) * (1.0 / 16777216.0);
        return select(v, -v, sign != 0u);
    }
    if e == 31u {
        return bitcast<f32>(sign | 0x7f800000u);
    }
    return bitcast<f32>(sign | ((e + 112u) << 23u) | (m << 13u));
}

fn pack_entries(primary: bool, high: u32, low: u32, mv: vec2<f32>) -> vec2<u32> {
    let priority = select(0u, PRIMARY_VECTOR_BIT, primary)
        | ((high & PRIORITY_HIGH_MAX) << PRIORITY_HIGH_OFFSET)
        | ((low & PRIORITY_LOW_MAX) << PRIORITY_LOW_OFFSET);
    return vec2(priority | f32_to_f16(mv.x), priority | f32_to_f16(mv.y));
}

fn unpack_entries(packed: vec2<u32>) -> VectorFieldEntry {
    var e = new_entry();
    e.high_priority = f32((packed.x >> PRIORITY_HIGH_OFFSET) & PRIORITY_HIGH_MAX) / f32(PRIORITY_HIGH_MAX);
    e.low_priority = f32((packed.x >> PRIORITY_LOW_OFFSET) & PRIORITY_LOW_MAX) / f32(PRIORITY_LOW_MAX);
    e.primary = is_primary(packed.x);
    e.valid = e.high_priority > 0.0;
    e.secondary = e.valid && !e.primary;
    if e.secondary {
        e.high_priority = 1.0 - e.high_priority;
    }
    e.motion_vector = vec2(f16_to_f32(packed.x & 0xffffu), f16_to_f32(packed.y & 0xffffu));
    return e;
}

fn load_game_field(p: vec2<i32>) -> vec2<u32> {
    if !is_on_screen(p, params.render_size) {
        return vec2(0u);
    }
    let i = render_index(p);
    return vec2(game_field_x_in[i], game_field_y_in[i]);
}

fn compute_mv_inpainting_level(uv: vec2<f32>, level: i32, size: vec2<i32>) -> vec4<f32> {
    let b = bilinear(uv, size);
    var sum = 0.0;
    var color = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        let p = b.base + bilinear_offset(i);
        if is_on_screen(p, size) {
            let s = textureLoad(game_pyramid, p, level);
            let weight = b.weights[i] * f32(s.z > 0.0) * s.z;
            sum += weight;
            color += s * weight;
        }
    }
    return color / select(1.0, sum, sum > 0.0);
}

fn load_inpainted_game_field_mv(uv: vec2<f32>) -> VectorFieldEntry {
    let px = vec2<i32>(uv * vec2<f32>(params.render_size));
    var e = unpack_entries(load_game_field(px));
    if !e.valid {
        var size = params.render_size;
        var v = vec4(0.0);
        let levels = i32(textureNumLevels(game_pyramid));
        for (var level = 0; level < min(11, levels) && v.w == 0.0; level++) {
            size /= 2;
            v = compute_mv_inpainting_level(uv, level, size);
        }
        e.motion_vector = v.xy;
        e.high_priority = v.z;
        e.low_priority = v.w;
        e.in_painted = true;
    }
    e.neg_outside = !is_uv_inside(uv - e.motion_vector);
    e.pos_outside = !is_uv_inside(uv + e.motion_vector);
    e.velocity = length(e.motion_vector);
    return e;
}

fn sample_optical_flow_field(uv: vec2<f32>) -> VectorFieldEntry {
    let size = params.optical_flow_size;
    let b = bilinear(uv, size);
    var e = new_entry();
    var weight_sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let p = b.base + bilinear_offset(i);
        if is_on_screen(p, size) {
            let w = b.weights[i];
            let j = render_index(p);
            let s = unpack_entries(vec2(of_field_x_in[j], of_field_y_in[j]));
            e.motion_vector += s.motion_vector * w;
            e.high_priority += s.high_priority * w;
            e.low_priority += s.low_priority * w;
            weight_sum += w;
        }
    }
    if weight_sum > 0.0 {
        e.motion_vector /= weight_sum;
        e.high_priority /= weight_sum;
        e.low_priority /= weight_sum;
    }
    e.neg_outside = !is_uv_inside(uv - e.motion_vector);
    e.pos_outside = !is_uv_inside(uv + e.motion_vector);
    e.velocity = length(e.motion_vector);
    return e;
}

fn has_scene_changed() -> bool {
    return (scene_change[1] & 0xfu) != 0u;
}

// ffx_frameinterpolation_reconstruct_dilated_velocity_and_previous_depth.h, with the
// motion vectors computed from the depth and the camera.
fn camera_motion_vector(p: vec2<i32>, depth: f32) -> vec2<f32> {
    let size = vec2<f32>(params.render_size);
    let uv = (vec2<f32>(p) + 0.5) / size - params.jitter / size;
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let previous = params.reproject * vec4(ndc, depth, 1.0);
    let previous_uv = vec2(0.5, -0.5) * previous.xy / previous.w + 0.5;
    return previous_uv - uv;
}

fn load_input_depth(p: vec2<i32>) -> f32 {
    if !is_on_screen(p, params.render_size) {
        return 0.0;
    }
    return textureLoad(input_depth, p, 0);
}

@compute @workgroup_size(8, 8)
fn prepare_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.render_size) {
        return;
    }
    let offsets = array(
        vec2(0, 0), vec2(1, 0), vec2(0, 1), vec2(0, -1), vec2(-1, 0),
        vec2(-1, 1), vec2(1, 1), vec2(-1, -1), vec2(1, -1),
    );
    var nearest = load_input_depth(p);
    var nearest_coord = p;
    for (var i = 1; i < 9; i++) {
        let q = p + offsets[i];
        if is_on_screen(q, params.render_size) {
            let d = load_input_depth(q);
            if d > nearest {
                nearest_coord = q;
                nearest = d;
            }
        }
    }
    let object = textureLoad(object_motion, nearest_coord, 0);
    var mv = camera_motion_vector(nearest_coord, load_input_depth(nearest_coord));
    if object.z > 0.5 {
        mv = object.xy;
    }
    textureStore(dilated_depth_out, p, vec4(nearest, 0.0, 0.0, 0.0));
    textureStore(dilated_mv_out, p, vec4(mv, 0.0, 0.0));
    // ReconstructPrevDepth
    let moving = mv * f32(length(mv * vec2<f32>(params.display_size)) > 0.1);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.render_size);
    let b = bilinear(uv + moving, params.render_size);
    for (var i = 0; i < 4; i++) {
        if b.weights[i] > RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD {
            let q = b.base + bilinear_offset(i);
            if is_on_screen(q, params.render_size) {
                atomicMax(&rec_prev_depth[render_index(q)], bitcast<u32>(nearest));
            }
        }
    }
}

// ffx_frameinterpolation_setup.h: the frames since the last reset (the vector fields are
// cleared by the encoder).
@compute @workgroup_size(1)
fn setup_pass() {
    if params.reset != 0u || has_scene_changed() {
        counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET] = 0u;
    } else {
        counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET] += 1u;
    }
}

// ffx_frameinterpolation_reconstruct_previous_depth.h: the interpolated frame's depth.
@compute @workgroup_size(8, 8)
fn reconstruct_previous_depth_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.render_size) {
        return;
    }
    let mv = textureLoad(dilated_mv, p, 0).xy * 0.5;
    let depth = textureLoad(dilated_depth, p, 0).x;
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.render_size);
    let b = bilinear(uv + mv, params.render_size);
    for (var i = 0; i < 4; i++) {
        let q = b.base + bilinear_offset(i);
        if b.weights[i] > RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD && is_on_screen(q, params.render_size) {
            atomicMax(&rec_interp_depth[render_index(q)], bitcast<u32>(depth));
        }
    }
}

// ffx_frameinterpolation_game_motion_vector_field.h
fn priority_from_view_depth(view_depth_in: f32) -> u32 {
    let d = pow(view_depth_in, 0.33);
    let factor = u32((1.0 - d * (1.0 / (1.0 + d))) * f32(PRIORITY_HIGH_MAX));
    return max(1u, factor);
}

fn update_game_field(p: vec2<i32>, packed: vec2<u32>) -> u32 {
    let i = render_index(p);
    let x = atomicMax(&game_field_x[i], packed.x);
    let y = atomicMax(&game_field_y[i], packed.y);
    return max(x, y);
}

@compute @workgroup_size(8, 8)
fn game_motion_vector_field_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.render_size) {
        return;
    }
    let size = vec2<f32>(params.render_size);
    let uv = (vec2<f32>(p) + 0.5) / size;
    let depth = textureLoad(dilated_depth, p, 0).x;
    let mv = textureLoad(dilated_mv, p, 0).xy;
    let half_mv = mv * 0.5;
    let interpolated_uv = uv + half_mv;
    let high_primary = priority_from_view_depth(device_to_view_depth(depth));
    let previous = textureSampleLevel(previous_color, linear_clamp, uv + mv, 0.0).rgb;
    let current = textureSampleLevel(current_color, linear_clamp, uv, 0.0).rgb;
    let previous_luma = 0.001 + raw_rgb_to_luminance(previous);
    let current_luma = 0.001 + raw_rgb_to_luminance(current);
    var low = u32(round(min_divided_by_max(previous_luma, current_luma) * f32(PRIORITY_LOW_MAX)))
        * u32(is_uv_inside(uv + mv));
    let primary = pack_entries(true, high_primary, low, half_mv);
    let b = bilinear(interpolated_uv, params.render_size);
    for (var i = 0; i < 4; i++) {
        let q = b.base + bilinear_offset(i);
        if is_on_screen(q, params.render_size) {
            update_game_field(q, primary);
        }
    }
    if length(half_mv * size) > EPSILON {
        var write_secondary = true;
        var primary_hits = 0u;
        let step_scale = length(1.0 / size);
        let step_mv = normalize(mv);
        let break_dist = min(length(half_mv), length(vec2(0.5, 0.5)));
        for (var s = step_scale; s <= break_dist && write_secondary; s += step_scale) {
            let secondary_uv = interpolated_uv - step_mv * s;
            let sb = bilinear(secondary_uv, params.render_size);
            let high_secondary = max(1u, PRIORITY_HIGH_MAX - high_primary);
            let to_centre_raw = vec2(0.5, 0.5) - secondary_uv;
            let to_centre = select(vec2(0.0), normalize(to_centre_raw), length(to_centre_raw) > 0.0);
            low = u32(max(0.0, dot(to_centre, step_mv)) * f32(PRIORITY_LOW_MAX));
            let secondary = pack_entries(false, high_secondary, low, half_mv);
            let q = sb.base;
            write_secondary = write_secondary && is_on_screen(q, params.render_size);
            if write_secondary {
                let existing = update_game_field(q, secondary);
                primary_hits += u32(is_primary(existing));
                write_secondary = write_secondary && primary_hits <= 3u;
            }
        }
    }
}

// ffx_frameinterpolation_compute_game_vector_field_inpainting_pyramid.h: SPD, a level per
// pass. Level 0 reads the field, the others the level above.
fn reduce_game_field(v: array<vec4<f32>, 4>) -> vec4<f32> {
    var sum = vec4(0.0);
    var weight_sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let w = f32(v[i].z > 0.0);
        sum += v[i] * w;
        weight_sum += w;
    }
    return sum / select(1.0, weight_sum, weight_sum > EPSILON);
}

@compute @workgroup_size(8, 8)
fn game_field_pyramid_first_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.level_size) {
        return;
    }
    var v: array<vec4<f32>, 4>;
    for (var i = 0; i < 4; i++) {
        let e = unpack_entries(load_game_field(p * 2 + bilinear_offset(i)));
        v[i] = vec4(e.motion_vector, e.high_priority, e.low_priority);
    }
    textureStore(pyramid_out, p, reduce_game_field(v));
}

@compute @workgroup_size(8, 8)
fn game_field_pyramid_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.level_size) {
        return;
    }
    var v: array<vec4<f32>, 4>;
    for (var i = 0; i < 4; i++) {
        v[i] = textureLoad(pyramid_in, p * 2 + bilinear_offset(i), 0);
    }
    textureStore(pyramid_out, p, reduce_game_field(v));
}

// ffx_frameinterpolation_optical_flow_vector_field.h
fn load_optical_flow(p: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(optical_flow));
    if !is_on_screen(p, size) {
        return vec2(0.0);
    }
    return vec2<f32>(textureLoad(optical_flow, p, 0).xy) * params.optical_flow_scale;
}

@compute @workgroup_size(8, 8)
fn optical_flow_vector_field_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    let rect = vec2<f32>(params.display_size);
    var avg = vec2(0.0);
    var sw = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let vs = load_optical_flow(p + vec2(x, y));
            let len = length(vs * rect);
            let w = max(0.0, 512.0 - len) * f32(len > 1.0);
            avg += vs * w;
            sw += w;
        }
    }
    // AMD divides by 0 here, and its max(0, NaN) below gives 0 weights: a zero vector.
    if sw > 0.0 {
        avg /= sw;
    }
    var v = vec2(0.0);
    sw = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let vs = load_optical_flow(p + vec2(x, y));
            let len = length(vs * rect);
            let len_factor = max(0.0, 512.0 - len) * f32(len > 1.0);
            let w = pow(max(0.0, dot(avg, vs)), 1.25) * len_factor;
            v += vs * w;
            sw += w;
        }
    }
    if sw > EPSILON {
        v /= sw;
    }
    // computeOpticalFlowFieldMvs
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.optical_flow_size);
    let half_mv = v * 0.5;
    let previous = textureSampleLevel(previous_color, linear_clamp, uv + v, 0.0).rgb;
    let current = textureSampleLevel(current_color, linear_clamp, uv, 0.0).rgb;
    let previous_luma = 0.001 + raw_rgb_to_luminance(previous);
    let current_luma = 0.001 + raw_rgb_to_luminance(current);
    let velocity = length(v * rect);
    let high = u32(velocity > 1.0)
        * u32(saturate(velocity / length(rect * 0.05)) * f32(PRIORITY_HIGH_MAX));
    if high > 0u {
        let low = u32(round(min_divided_by_max(previous_luma, current_luma) * f32(PRIORITY_LOW_MAX)))
            * u32(is_uv_inside(uv + v));
        let packed = pack_entries(true, high, low, half_mv);
        let b = bilinear(uv + half_mv, params.optical_flow_size);
        for (var i = 0; i < 4; i++) {
            let q = b.base + bilinear_offset(i);
            if is_on_screen(q, params.optical_flow_size) {
                let j = render_index(q);
                atomicMax(&of_field_x[j], packed.x);
                atomicMax(&of_field_y[j], packed.y);
            }
        }
    }
}

// ffx_frameinterpolation_disocclusion_mask.h
fn load_estimated_depth(which: u32, p: vec2<i32>) -> f32 {
    if which == 0u {
        return bitcast<f32>(rec_prev_depth_in[render_index(p)]);
    }
    return textureLoad(dilated_depth, p, 0).x;
}

fn compute_depth_clip(which: u32, uv: vec2<f32>, current_depth: f32) -> f32 {
    let current_view = device_to_view_depth(current_depth);
    let b = bilinear(uv, params.render_size);
    var depth = 0.0;
    var weight_sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let q = b.base + bilinear_offset(i);
        let w = b.weights[i];
        if is_on_screen(q, params.render_size) && w > RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD {
            let previous_depth = load_estimated_depth(which, q);
            let previous_view = device_to_view_depth(previous_depth);
            let diff = current_view - previous_view;
            if diff > 0.0 {
                let plane_depth = min(previous_depth, current_depth);
                let centre = view_space_position(params.render_size / 2, params.render_size, plane_depth);
                let corner = view_space_position(vec2(0), params.render_size, plane_depth);
                let half_viewport_width = length(vec2<f32>(params.render_size));
                let threshold = min(current_view, previous_view);
                let k_fov = length(corner) / length(centre);
                let required = 1.37e-05 * k_fov * half_viewport_width * threshold;
                depth += f32(required / diff >= 1.0) * w;
                weight_sum += w;
            }
        }
    }
    return select(0.0, saturate(1.0 - depth / weight_sum), weight_sum > 0.0);
}

@compute @workgroup_size(8, 8)
fn disocclusion_mask_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.render_size) {
        return;
    }
    let depth = bitcast<f32>(rec_interp_depth_in[render_index(p)]);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.render_size);
    let game = load_inpainted_game_field_mv(uv);
    var mask = vec2(
        1.0 - compute_depth_clip(0u, uv + game.motion_vector, depth),
        1.0 - compute_depth_clip(1u, uv - game.motion_vector, depth),
    );
    mask = vec2<f32>(mask >= vec2(EPSILON));
    let source_mv = game.motion_vector * 2.0;
    let size = vec2<f32>(params.render_size);
    let previous = vec2<i32>((uv + source_mv) * size);
    mask.x = saturate(mask.x + f32(!is_on_screen(previous, params.render_size)));
    let current = vec2<i32>((uv - source_mv) * size);
    mask.y = saturate(mask.y + f32(!is_on_screen(current, params.render_size)));
    textureStore(disocclusion_out, p, vec4(mask, 0.0, 0.0));
}

// ffx_frameinterpolation.h
struct SourceColor {
    raw: vec3<f32>,
    weight_sum: f32,
};

fn sample_bilinear(is_current: bool, uv: vec2<f32>, mv: vec2<f32>) -> SourceColor {
    let size = params.display_size;
    let b = bilinear(uv + mv, size);
    var color = vec3(0.0);
    var weight_sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let q = b.base + bilinear_offset(i);
        if is_on_screen(q, size) {
            let w = b.weights[i];
            if is_current {
                color += textureLoad(current_color, q, 0).rgb * w;
            } else {
                color += textureLoad(previous_color, q, 0).rgb * w;
            }
            weight_sum += w;
        }
    }
    return SourceColor(select(vec3(0.0), color / weight_sum, weight_sum != 0.0), weight_sum);
}

@compute @workgroup_size(8, 8)
fn interpolate_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.display_size) {
        return;
    }
    let frame_index = counters_in[COUNTER_FRAME_INDEX_SINCE_LAST_RESET];
    if frame_index == 0u {
        textureStore(interpolated_out, p, vec4(textureLoad(current_color, p, 0).rgb, 0.0));
        return;
    }
    var in_painting = 0.0;
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.display_size);
    let game = load_inpainted_game_field_mv(uv);
    let flow = sample_optical_flow_field(uv);
    var disocclusion_factor = vec2<f32>(
        saturate(textureSampleLevel(disocclusion, linear_clamp, uv, 0.0).xy) == vec2(1.0),
    );
    let previous_game = sample_bilinear(false, uv, game.motion_vector);
    let current_game = sample_bilinear(true, uv, -game.motion_vector);
    let previous_of = sample_bilinear(false, uv, flow.motion_vector);
    let current_of = sample_bilinear(true, uv, -flow.motion_vector);
    disocclusion_factor.x *= f32(!game.pos_outside);
    disocclusion_factor.y *= f32(!game.neg_outside);
    in_painting = saturate(max(in_painting, f32(length(disocclusion_factor) <= EPSILON)));
    var t = 0.5;
    t += 0.5 * (1.0 - disocclusion_factor.x);
    t -= 0.5 * (1.0 - disocclusion_factor.y);
    var color = mix(previous_game.raw, current_game.raw, saturate(t));
    let disoccluded = saturate(1.0 - min(disocclusion_factor.x, disocclusion_factor.y));
    if previous_game.weight_sum == 0.0 {
        color = current_game.raw;
    } else if current_game.weight_sum == 0.0 {
        color = previous_game.raw;
    }
    if previous_game.weight_sum == 0.0 && current_game.weight_sum == 0.0 {
        in_painting = 1.0;
    }
    var of_t = 1.0;
    if previous_of.weight_sum > 0.0 && current_of.weight_sum > 0.0 {
        of_t = 0.5;
    } else if previous_of.weight_sum > 0.0 {
        of_t = 0.0;
    }
    let of_color = mix(previous_of.raw, current_of.raw, of_t);
    let of_sim = normalized_dot3(previous_of.raw, current_of.raw);
    var game_sim = normalized_dot3(previous_game.raw, current_game.raw);
    game_sim = mix(max(EPSILON, game_sim), 1.0, disoccluded);
    var game_bias = saturate(game_sim / max(EPSILON, of_sim));
    game_bias = mix(game_bias, 1.0, f32(frame_index < 10u));
    color = mix(of_color, color, saturate(game_bias));
    textureStore(interpolated_out, p, vec4(color, in_painting));
}

// ffx_frameinterpolation_compute_inpainting_pyramid.h: SPD, a level per pass.
fn reduce_inpainting(v: array<vec4<f32>, 4>) -> vec4<f32> {
    let sum = v[0].w + v[1].w + v[2].w + v[3].w;
    if sum == 0.0 {
        return vec4(0.0);
    }
    return (v[0] * v[0].w + v[1] * v[1].w + v[2] * v[2].w + v[3] * v[3].w) / sum;
}

@compute @workgroup_size(8, 8)
fn inpainting_pyramid_first_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.level_size) {
        return;
    }
    var v: array<vec4<f32>, 4>;
    for (var i = 0; i < 4; i++) {
        let q = p * 2 + bilinear_offset(i);
        var c = vec4(0.0);
        if is_on_screen(q, params.display_size) {
            c = textureLoad(interpolated, q, 0);
            c.w = saturate(1.0 - c.w);
        }
        v[i] = c;
    }
    textureStore(pyramid_out, p, reduce_inpainting(v));
}

@compute @workgroup_size(8, 8)
fn inpainting_pyramid_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.level_size) {
        return;
    }
    var v: array<vec4<f32>, 4>;
    for (var i = 0; i < 4; i++) {
        v[i] = textureLoad(pyramid_in, p * 2 + bilinear_offset(i), 0);
    }
    textureStore(pyramid_out, p, reduce_inpainting(v));
}

// ffx_frameinterpolation_inpainting.h (no HUD-less source: Horizon draws its HUD over
// the made frame itself).
fn compute_inpainting_level(uv: vec2<f32>, level: i32, size: vec2<i32>) -> vec4<f32> {
    let b = bilinear(uv, size);
    var color = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        let q = b.base + bilinear_offset(i);
        if is_on_screen(q, size) {
            let s = textureLoad(inpainting_pyramid, q, level);
            let w = b.weights[i] * f32(s.w > 0.0);
            color += vec4(s.rgb * w, w);
        }
    }
    return color;
}

fn compute_inpainting(p: vec2<i32>) -> vec3<f32> {
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.display_size);
    var color = vec4(0.0);
    var size = params.display_size;
    let levels = i32(textureNumLevels(inpainting_pyramid));
    for (var level = 0; level < min(10, levels); level++) {
        size /= 2;
        let mip = compute_inpainting_level(uv, level, size);
        if mip.w > 0.0 {
            let weight = pow(1.0 - f32(level) / 10.0, 3.0) * mip.w;
            color += vec4(mip.rgb / mip.w, 1.0) * weight;
        }
    }
    return select(vec3(0.0), color.rgb / color.w, color.w > 0.0);
}

@compute @workgroup_size(8, 8)
fn inpainting_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !is_on_screen(p, params.display_size) {
        return;
    }
    var c = textureLoad(interpolated, p, 0);
    if c.w > EPSILON {
        c = vec4(mix(c.rgb, compute_inpainting(p), c.w), 1.0);
    }
    textureStore(output, p, c);
}
