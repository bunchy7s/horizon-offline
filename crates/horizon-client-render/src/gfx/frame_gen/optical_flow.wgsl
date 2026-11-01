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

// FidelityFX SDK 1.1.4's optical flow (ffx_opticalflow_*.h, the GLSL path), ported to WGSL
// for Horizon. What differs from AMD's (optical_flow.rs lists why):
// - luma levels are r32float (AMD: r8ui); loads truncate to the same integers;
// - the luma pyramid is one pass per level instead of SPD's single pass;
// - wave sums and minimums are workgroup atomics, and the reductions AMD leaves to a
//   wave's lockstep have barriers;
// - the scene change textures are storage buffers;
// - the search reads its prediction from one texture and writes another (rg32sint can't
//   be read_write everywhere).

struct Params {
    // Display size (cbOF.iInputLumaResolution).
    display: vec2<i32>,
    level: u32,
    level_count: u32,
    frame_index: u32,
    transfer_function: u32,
    min_max_luminance: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var input_color: texture_2d<f32>;
@group(0) @binding(2) var luma_out: texture_storage_2d<r32float, write>;
// The current frame's luma at this level, and the previous frame's.
@group(0) @binding(3) var luma_in: texture_2d<f32>;
@group(0) @binding(4) var luma_previous: texture_2d<f32>;
// Search: the prediction from the level above; filter: the search; scale: the filtered.
@group(0) @binding(5) var flow_in: texture_2d<i32>;
@group(0) @binding(6) var flow_out: texture_storage_2d<rg32sint, write>;
@group(0) @binding(7) var<storage, read_write> scd_histogram: array<atomic<u32>>;
@group(0) @binding(8) var<storage, read_write> scd_previous_histogram: array<f32>;
@group(0) @binding(9) var<storage, read_write> scd_temp: array<atomic<u32>, 3>;
@group(0) @binding(10) var<storage, read_write> scd_output: array<atomic<u32>, 3>;

const SCD_OUTPUT_SCENE_CHANGE_SLOT: u32 = 0u;
const SCD_OUTPUT_HISTORY_BITS_SLOT: u32 = 1u;
const SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT: u32 = 2u;

fn load_luma(t: texture_2d<f32>, p: vec2<i32>) -> u32 {
    let size = vec2<i32>(textureDimensions(t));
    if any(p < vec2(0)) || any(p >= size) {
        return 0u;
    }
    return u32(textureLoad(t, p, 0).x);
}

fn load_flow(p: vec2<i32>) -> vec2<i32> {
    let size = vec2<i32>(textureDimensions(flow_in));
    if any(p < vec2(0)) || any(p >= size) {
        return vec2(0);
    }
    return textureLoad(flow_in, p, 0).xy;
}

fn store_flow(p: vec2<i32>, v: vec2<i32>) {
    let size = vec2<i32>(textureDimensions(flow_out));
    if all(p >= vec2(0)) && all(p < size) {
        textureStore(flow_out, p, vec4(v, 0, 0));
    }
}

fn packed_luma(width: i32, x: i32, luma0: u32, luma1: u32, luma2: u32, luma3: u32) -> u32 {
    var packed = luma0 | (luma1 << 8u) | (luma2 << 16u) | (luma3 << 24u);
    if x < 0 {
        let filler = packed & 0xffu;
        if x <= -1 {
            packed = (packed << 8u) | filler;
        }
        if x <= -2 {
            packed = (packed << 8u) | filler;
        }
        if x <= -3 {
            packed = (packed << 8u) | filler;
        }
    } else if x > width - 4 {
        let filler = packed & 0xff000000u;
        if x >= width - 3 {
            packed = (packed >> 8u) | filler;
        }
        if x >= width - 2 {
            packed = (packed >> 8u) | filler;
        }
        if x >= width - 1 {
            packed = (packed >> 8u) | filler;
        }
    }
    return packed;
}

fn load_packed(t: texture_2d<f32>, p: vec2<i32>) -> u32 {
    let width = params.display.x >> params.level;
    let height = params.display.y >> params.level;
    let q = vec2(clamp(p.x, 0, width - 4), clamp(p.y, 0, height - 1));
    return packed_luma(
        width,
        p.x,
        load_luma(t, q),
        load_luma(t, q + vec2(1, 0)),
        load_luma(t, q + vec2(2, 0)),
        load_luma(t, q + vec2(3, 0)),
    );
}

fn sad(a: u32, b: u32) -> u32 {
    return u32(abs(i32(a & 0xffu) - i32(b & 0xffu))
        + abs(i32((a >> 8u) & 0xffu) - i32((b >> 8u) & 0xffu))
        + abs(i32((a >> 16u) & 0xffu) - i32((b >> 16u) & 0xffu))
        + abs(i32((a >> 24u) & 0xffu) - i32((b >> 24u) & 0xffu)));
}

fn qsad(a0_in: u32, a1_in: u32, b: u32) -> vec4<u32> {
    var a0 = a0_in;
    var a1 = a1_in;
    var s: vec4<u32>;
    s.x = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    a1 = a1 >> 8u;
    s.y = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    a1 = a1 >> 8u;
    s.z = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    s.w = sad(a0, b);
    return s;
}

fn is_scene_changed() -> bool {
    if params.frame_index <= 5u {
        return true;
    }
    return (atomicLoad(&scd_output[SCD_OUTPUT_HISTORY_BITS_SLOT]) & 0xfu) != 0u;
}

// ffx_opticalflow_prepare_luma.h, for an SDR back buffer (transfer function 0: Horizon's
// frames); the HDR transfer functions follow AMD's.
fn luminance_to_perceived(luminance: f32) -> f32 {
    var perceived: f32;
    if luminance <= 216.0 / 24389.0 {
        perceived = luminance * (24389.0 / 27.0);
    } else {
        perceived = pow(luminance, 1.0 / 3.0) * 116.0 - 16.0;
    }
    return perceived * 0.01;
}

fn linear_from_pq(x: vec3<f32>) -> vec3<f32> {
    let p = pow(x, vec3(0.0126833));
    return pow(clamp(p - vec3(0.835938), vec3(0.0), vec3(1.0)) / (vec3(18.8516) - 18.6875 * p), vec3(6.27739));
}

@compute @workgroup_size(16, 16)
fn prepare_luma(@builtin(global_invocation_id) gid: vec3<u32>) {
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let pos = vec2<i32>(gid.xy) * 2 + vec2(x, y);
            if any(pos >= params.display) {
                continue;
            }
            let c = textureLoad(input_color, pos, 0).rgb;
            var y_lum = 0.0;
            if params.transfer_function == 0u {
                y_lum = dot(c, vec3(0.2126, 0.7152, 0.0722));
            } else if params.transfer_function == 1u {
                let linear = linear_from_pq(c) * (10000.0 / params.min_max_luminance.y);
                y_lum = luminance_to_perceived(dot(linear, vec3(0.2627, 0.678, 0.0593)));
            } else if params.transfer_function == 2u {
                let mn = params.min_max_luminance.x;
                let mx = params.min_max_luminance.y;
                let linear = (c - vec3(mn / 80.0)) / vec3((mx - mn) / 80.0);
                y_lum = luminance_to_perceived(dot(linear, vec3(0.2126, 0.7152, 0.0722)));
            }
            textureStore(luma_out, pos, vec4(f32(u32(y_lum * 255.0)), 0.0, 0.0, 0.0));
        }
    }
}

// ffx_opticalflow_compute_luminance_pyramid.h: SPD's average of 2x2, a level per pass.
@compute @workgroup_size(8, 8)
fn luma_pyramid(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if any(p >= vec2<i32>(textureDimensions(luma_out))) {
        return;
    }
    let s = p * 2;
    let v = (textureLoad(luma_in, s, 0).x + textureLoad(luma_in, s + vec2(1, 0), 0).x
        + textureLoad(luma_in, s + vec2(0, 1), 0).x + textureLoad(luma_in, s + vec2(1, 1), 0).x) * 0.25;
    textureStore(luma_out, p, vec4(v, 0.0, 0.0, 0.0));
}

// ffx_opticalflow_generate_scd_histogram.h
const LBASE: u32 = 10u;
var<workgroup> scd_buffer: array<atomic<u32>, 2560>;

@compute @workgroup_size(32, 8)
fn scd_histogram_pass(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let histograms_per_dim = 3u;
    let div = vec2<u32>(params.display) / histograms_per_dim;
    let nx = gid.z % histograms_per_dim;
    let ny = gid.z / histograms_per_dim;
    let start = div * vec2(nx, ny);
    let stop = start + div;
    let offset = local_index * LBASE;
    for (var i = 0u; i < LBASE; i++) {
        atomicStore(&scd_buffer[offset + i], 0u);
    }
    workgroupBarrier();
    var coord = vec2<i32>(i32(start.x + 4u * gid.x), i32(start.y + gid.y));
    if u32(coord.x) < stop.x {
        for (; u32(coord.y) < stop.y; coord.y += 128) {
            let color = vec4(
                load_luma(luma_in, coord),
                load_luma(luma_in, coord + vec2(1, 0)),
                load_luma(luma_in, coord + vec2(2, 0)),
                load_luma(luma_in, coord + vec2(3, 0)),
            ) * LBASE;
            let scrambling = local_index % LBASE;
            atomicAdd(&scd_buffer[color.x + scrambling], 1u);
            atomicAdd(&scd_buffer[color.y + scrambling], 1u);
            atomicAdd(&scd_buffer[color.z + scrambling], 1u);
            atomicAdd(&scd_buffer[color.w + scrambling], 1u);
        }
    }
    workgroupBarrier();
    var value = 0u;
    for (var i = 0u; i < LBASE; i++) {
        value += atomicLoad(&scd_buffer[offset + i]);
    }
    let histogram_start = 256u * gid.z;
    atomicAdd(&scd_histogram[histogram_start + local_index], value);
}

// ffx_opticalflow_compute_scd_divergence.h
var<workgroup> source_histogram: array<f32, 256>;
var<workgroup> filtered_histogram: array<f32, 256>;
var<workgroup> temp_buffer: array<f32, 256>;
var<workgroup> temp_buffer2: array<vec2<f32>, 256>;

@compute @workgroup_size(256)
fn scd_divergence_pass(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let factor = 1000000.0;
    let where_to_stop = 3u * 9u - 1u;
    let histogram_count = 9.0;
    let kernel = array(0.0088122291, 0.027143577, 0.065114059, 0.12164907, 0.17699835, 0.20056541);
    let li = i32(local_index);
    source_histogram[li] = f32(atomicLoad(&scd_histogram[gid.x]));
    workgroupBarrier();
    let read = li - 5;
    var val = 0.0;
    val += kernel[0] * source_histogram[clamp(read + 0, 0, 255)];
    val += kernel[1] * source_histogram[clamp(read + 1, 0, 255)];
    val += kernel[2] * source_histogram[clamp(read + 2, 0, 255)];
    val += kernel[3] * source_histogram[clamp(read + 3, 0, 255)];
    val += kernel[4] * source_histogram[clamp(read + 4, 0, 255)];
    val += kernel[5] * source_histogram[clamp(read + 5, 0, 255)];
    val += kernel[4] * source_histogram[clamp(read + 6, 0, 255)];
    val += kernel[3] * source_histogram[clamp(read + 7, 0, 255)];
    val += kernel[2] * source_histogram[clamp(read + 8, 0, 255)];
    val += kernel[1] * source_histogram[clamp(read + 9, 0, 255)];
    val += kernel[0] * source_histogram[clamp(read + 10, 0, 255)];
    val += 1.0;
    if gid.y == 0u {
        if li == 0 {
            filtered_histogram[255] = 1.0;
        } else {
            filtered_histogram[li - 1] = val;
        }
    } else if gid.y == 1u {
        filtered_histogram[li] = val;
    } else if gid.y == 2u {
        if li == 255 {
            filtered_histogram[0] = 1.0;
        } else {
            filtered_histogram[li + 1] = val;
        }
    }
    workgroupBarrier();
    temp_buffer[li] = filtered_histogram[li];
    workgroupBarrier();
    for (var stride = 128; stride > 0; stride >>= 1u) {
        if li < stride {
            temp_buffer[li] += temp_buffer[li + stride];
        }
        workgroupBarrier();
    }
    filtered_histogram[li] /= temp_buffer[0];
    let current = filtered_histogram[li];
    let previous = scd_previous_histogram[gid.x];
    temp_buffer2[li] = vec2(current * log(current / previous), previous * log(previous / current));
    workgroupBarrier();
    for (var stride = 128; stride > 0; stride >>= 1u) {
        if li < stride {
            temp_buffer2[li] += temp_buffer2[li + stride];
        }
        workgroupBarrier();
    }
    if li == 0 {
        let sum = temp_buffer2[0];
        let res_float = 1.0 - exp(-(abs(sum.x) + abs(sum.y)));
        let res_uint = u32((res_float / histogram_count) * factor);
        atomicAdd(&scd_temp[gid.y], res_uint);
        let finished = atomicAdd(&scd_output[SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT], 1u);
        if finished == where_to_stop {
            let res0 = atomicLoad(&scd_temp[0]);
            let res1 = atomicLoad(&scd_temp[1]);
            let res2 = atomicLoad(&scd_temp[2]);
            let scene_change = f32(min(res0, min(res1, res2))) / factor;
            var history = atomicLoad(&scd_output[SCD_OUTPUT_HISTORY_BITS_SLOT]) << 1u;
            if scene_change > 0.45 {
                history |= 1u;
            }
            atomicStore(&scd_output[SCD_OUTPUT_SCENE_CHANGE_SLOT], bitcast<u32>(scene_change));
            atomicStore(&scd_output[SCD_OUTPUT_HISTORY_BITS_SLOT], history);
            atomicStore(&scd_output[SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT], 0u);
            atomicStore(&scd_temp[0], 0u);
            atomicStore(&scd_temp[1], 0u);
            atomicStore(&scd_temp[2], 0u);
        }
    }
    if gid.y == 1u {
        scd_previous_histogram[gid.x] = current;
        atomicStore(&scd_histogram[gid.x], 0u);
    }
}

// ffx_opticalflow_compute_optical_flow_v5.h: block matching of 8x8 blocks over +-8 pixels,
// 2x2 blocks per workgroup of 64.
var<workgroup> pixels: array<array<u32, 2>, 8>;
// SearchBufferSizeY (24) rows of SearchBufferSizeX (6) packed lumas.
var<workgroup> search_buffer: array<u32, 144>;
var<workgroup> sad_map: array<array<array<u32, 5>, 16>, 4>;
var<workgroup> wave_min: atomic<u32>;
var<workgroup> wave_sum: atomic<u32>;
var<workgroup> scene_changed: u32;

fn abs_2(v: vec2<i32>) -> vec2<u32> {
    let mask = v >> vec2(31u);
    return vec2<u32>((v + mask) ^ mask);
}

// FFX_OPTICALFLOW_FIX_TOP_LEFT_BIAS: the nearest of equal SADs wins.
fn encode_search_coord(coord: vec2<i32>) -> u32 {
    let a = abs_2(coord - 8);
    return (a.y << 12u) | (a.x << 8u) | (u32(coord.y) << 4u) | u32(coord.x);
}

fn decode_search_coord(bits: u32) -> vec2<i32> {
    return vec2(i32(bits & 0xfu) - 8, i32((bits >> 4u) & 0xfu) - 8);
}

fn calculate_qsads2(search_id: vec2<i32>) -> vec4<u32> {
    var s = vec4(0u);
    for (var dy = 0; dy < 8; dy++) {
        let row = (search_id.y + dy) * 6;
        let a0 = search_buffer[row + search_id.x];
        let a1 = search_buffer[row + search_id.x + 1];
        let a2 = search_buffer[row + search_id.x + 2];
        s += qsad(a0, a1, pixels[dy][0]);
        s += qsad(a1, a2, pixels[dy][1]);
    }
    return s;
}

@compute @workgroup_size(64)
fn search_pass(
    @builtin(workgroup_id) group_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let li = i32(local_index);
    let search_id = vec2(li & 3, (li >> 2u) & 15);
    let lane_block = ((li >> 1u) & 1) | (((li >> 5u) & 1) << 1u);
    let group = vec2<i32>(group_id.xy);
    let px = (group << vec2(4u)) + search_id * vec2(4, 1);
    if local_index == 0u {
        scene_changed = u32(is_scene_changed());
    }
    if workgroupUniformLoad(&scene_changed) != 0u {
        if (search_id.y & 7) == 0 && (search_id.x & 1) == 0 {
            store_flow(px >> vec2(3u), vec2(0));
        }
        return;
    }
    let use_prediction = params.level != params.level_count - 1u;
    let packed = load_packed(luma_in, px);
    let sad_4blocks = sad(packed, load_packed(luma_previous, px));
    let of_group = group << vec2(1u);
    let pixel_group = group << vec2(4u);
    for (var by = 0; by < 2; by++) {
        for (var bx = 0; bx < 2; bx++) {
            let block = vec2(bx, by);
            var current = vec2(0);
            if use_prediction {
                current = load_flow(of_group + block);
            }
            if lane_block == by * 2 + bx {
                pixels[search_id.y & 7][search_id.x & 1] = packed;
            }
            // LoadSearchBuffer
            let base = pixel_group + block * 8 + current - vec2(8, 8);
            for (var id = li; id < 144; id += 64) {
                search_buffer[id] = load_packed(luma_previous, vec2(base.x + (id % 6) * 4, base.y + id / 6));
            }
            if local_index == 0u {
                atomicStore(&wave_min, 0xffffffffu);
                atomicStore(&wave_sum, 0u);
            }
            workgroupBarrier();
            let q = calculate_qsads2(search_id);
            // PrepareSadMap
            let sx = search_id.x;
            let sy = search_id.y;
            sad_map[0][sy][sx] = (q.x << 16u) | encode_search_coord(vec2(sx * 4 + 0, sy));
            sad_map[1][sy][sx] = (q.y << 16u) | encode_search_coord(vec2(sx * 4 + 1, sy));
            sad_map[2][sy][sx] = (q.z << 16u) | encode_search_coord(vec2(sx * 4 + 2, sy));
            sad_map[3][sy][sx] = (q.w << 16u) | encode_search_coord(vec2(sx * 4 + 3, sy));
            workgroupBarrier();
            // SadMapMinReduction256 and BlockSad64 (FFX_LOCAL_SEARCH_FALLBACK)
            let min01 = min(sad_map[0][sy][sx], sad_map[1][sy][sx]);
            let min23 = min(sad_map[2][sy][sx], sad_map[3][sy][sx]);
            atomicMin(&wave_min, min(min01, min23));
            atomicAdd(&wave_sum, select(0u, sad_4blocks, lane_block == bx + by * 2));
            workgroupBarrier();
            let min_sad = atomicLoad(&wave_min);
            let block_sad_sum = atomicLoad(&wave_sum);
            var v = current + decode_search_coord(min_sad);
            if params.level == 0u && block_sad_sum <= (min_sad >> 16u) {
                v = vec2(0);
            }
            if local_index == 0u {
                store_flow(of_group + block, v);
            }
            // The next block reuses the shared buffers and counters.
            workgroupBarrier();
        }
    }
}

// ffx_opticalflow_filter_optical_flow_v5.h: the vector of the 3x3 nearest to all the others.
@compute @workgroup_size(16, 4)
fn filter_flow(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    var mv: array<vec2<i32>, 9>;
    var idx = 0;
    for (var xx = -1; xx < 2; xx++) {
        for (var yy = -1; yy < 2; yy++) {
            mv[idx] = load_flow(p + vec2(xx, yy));
            idx++;
        }
    }
    var ret = 0xffffffffu;
    for (var i = 0; i < 9; i++) {
        var tmp = 0u;
        for (var j = 0; j < 9; j++) {
            let delta = mv[i] - mv[j];
            tmp = u32(delta.x * delta.x) + (u32(delta.y * delta.y) + tmp);
        }
        ret = min((tmp << 4u) | u32(i), ret);
    }
    store_flow(p, mv[ret & 0xfu]);
}

// ffx_opticalflow_scale_optical_flow_advanced_v5.h: the next level's vectors, each the
// best of its 4 nearest at this level.
var<workgroup> nearest_vectors: array<array<array<vec2<i32>, 4>, 4>, 4>;
var<workgroup> local_region: array<array<array<u32, 4>, 4>, 4>;
var<workgroup> sads: array<array<array<u32, 4>, 4>, 4>;

@compute @workgroup_size(4, 4, 4)
fn scale_pass(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    if local_index == 0u {
        scene_changed = u32(is_scene_changed());
    }
    if workgroupUniformLoad(&scene_changed) != 0u {
        store_flow(vec2<i32>(gid.xy), vec2(0));
        return;
    }
    let x_offset = i32(lid.z % 2u) - 1 + i32(gid.x % 2u);
    let y_offset = i32(lid.z / 2u) - 1 + i32(gid.y % 2u);
    let src = vec2(i32(gid.x / 2u) + x_offset, i32(gid.y / 2u) + y_offset);
    let nearest = load_flow(src);
    nearest_vectors[lid.z][lid.y][lid.x] = nearest * 2;
    let max_y = 4u;
    for (var n = lid.z; n < max_y; n += 4u) {
        let luma_pos = vec2(i32(gid.x) * 4, i32(gid.y * max_y + n));
        local_region[n][lid.y][lid.x] = load_packed(luma_in, luma_pos);
    }
    workgroupBarrier();
    var s = 0u;
    for (var n = 0u; n < max_y; n++) {
        let luma_pos = vec2(i32(gid.x) * 4, i32(gid.y * max_y + n)) + nearest;
        s += sad(local_region[n][lid.y][lid.x], load_packed(luma_previous, luma_pos));
    }
    sads[lid.z][lid.y][lid.x] = s;
    workgroupBarrier();
    if lid.z == 0u {
        var best_sad = 0xffffffffu;
        var best = 0u;
        for (var n = 0u; n < 4u; n++) {
            if sads[n][lid.y][lid.x] < best_sad {
                best_sad = sads[n][lid.y][lid.x];
                best = n;
            }
        }
        store_flow(vec2<i32>(gid.xy), nearest_vectors[best][lid.y][lid.x]);
    }
}
