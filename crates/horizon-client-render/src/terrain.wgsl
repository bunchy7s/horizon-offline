// Terrain, sky and outline shaders, ported from the pinned vanilla
// terrain.vsh/fsh, fog.glsl and sample_lightmap.glsl.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // environmental start/end, render-distance start/end
    fog: vec4<f32>,
    sky_color: vec4<f32>,
    // x: sky darkening, y: game time, z: aspect, w: tan(fov/2)
    misc: vec4<f32>,
    inverse_view: mat4x4<f32>,
    sunrise: vec4<f32>,
    // xyz: sun direction, w: star brightness
    sun: vec4<f32>,
    // x: sky fog end, y: cloud fog end, z: dark disc flag
    fog_ends: vec4<f32>,
    // Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: array<vec4<f32>, 2>,
    // x: vanilla's GameTime ((game time % 24000 + partial tick) / 24000), z: how many
    // dynamic_lights are set
    time: vec4<f32>,
    // Atlas rect of the current destroy stage (read by entity.wgsl).
    crumbling_sprite: vec4<f32>,
    outline_params: vec4<f32>,
    // Horizon Extras' dynamic lights: camera-relative position and level (dynamic_light.rs).
    dynamic_lights: array<vec4<f32>, 16>,
    colored_lights: array<array<vec4<f32>, 2>, 16>,
    colored_count: vec4<u32>,
    emissive_layers: array<vec4<u32>, 32>,
    emissive_count: vec4<u32>,
    // The sunrise fan's 16 triangles (gfx/sunrise_fan.rs): xyz the apex weight's normal, w the
    // triangle's volume with the camera; then each rim vertex's normal against the apex.
    sunrise_fan: array<vec4<f32>, 32>,
    // x: Options.textureFiltering (0 none, 1 RGSS, 2 anisotropic), y: the sprite arrays' width
    // in texels (vanilla's TextureSize).
    filtering: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var sprites: texture_2d_array<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;
// Layers past the first array's SPRITE_SPLIT (2048, gfx.rs): mods' sprites.
@group(0) @binding(5) var sprites_1: texture_2d_array<f32>;
@group(0) @binding(6) var sprites_2: texture_2d_array<f32>;
@group(0) @binding(7) var sprites_3: texture_2d_array<f32>;
@group(0) @binding(8) var sprites_4: texture_2d_array<f32>;
@group(0) @binding(9) var sprites_5: texture_2d_array<f32>;
@group(0) @binding(10) var sprites_6: texture_2d_array<f32>;
@group(0) @binding(11) var sprites_7: texture_2d_array<f32>;
// Per block sprite (x the layer in its array, y the array), the brightness its
// pixels must reach to shine in HDR output: r where the bright parts shine, g
// where all of it does (gfx/hdr_stats.rs, measured from the sprite itself).
@group(0) @binding(12) var emission_thresholds: texture_2d<f32>;
// How many sprite arrays hold layers (a pipeline constant): 1 for vanilla,
// which then samples exactly as before.
override SPRITE_ARRAYS: u32 = 1u;
override SPRITE_SPLIT: u32 = 2048u;
// A sprite sampled with the given gradients, from whichever array holds its layer.
fn sprite_grad(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    if SPRITE_ARRAYS == 1u {
        return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy);
    }
    switch layer / SPRITE_SPLIT {
        case 0u: { return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy); }
        case 1u: { return textureSampleGrad(sprites_1, sprite_sampler, uv, l, dx, dy); }
        case 2u: { return textureSampleGrad(sprites_2, sprite_sampler, uv, l, dx, dy); }
        case 3u: { return textureSampleGrad(sprites_3, sprite_sampler, uv, l, dx, dy); }
        case 4u: { return textureSampleGrad(sprites_4, sprite_sampler, uv, l, dx, dy); }
        case 5u: { return textureSampleGrad(sprites_5, sprite_sampler, uv, l, dx, dy); }
        case 6u: { return textureSampleGrad(sprites_6, sprite_sampler, uv, l, dx, dy); }
        default: { return textureSampleGrad(sprites_7, sprite_sampler, uv, l, dx, dy); }
    }
}
// The same at an explicit mip level.
fn sprite_level(uv: vec2<f32>, layer: u32, lod: f32) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    if SPRITE_ARRAYS == 1u {
        return textureSampleLevel(sprites, sprite_sampler, uv, l, lod);
    }
    switch layer / SPRITE_SPLIT {
        case 0u: { return textureSampleLevel(sprites, sprite_sampler, uv, l, lod); }
        case 1u: { return textureSampleLevel(sprites_1, sprite_sampler, uv, l, lod); }
        case 2u: { return textureSampleLevel(sprites_2, sprite_sampler, uv, l, lod); }
        case 3u: { return textureSampleLevel(sprites_3, sprite_sampler, uv, l, lod); }
        case 4u: { return textureSampleLevel(sprites_4, sprite_sampler, uv, l, lod); }
        case 5u: { return textureSampleLevel(sprites_5, sprite_sampler, uv, l, lod); }
        case 6u: { return textureSampleLevel(sprites_6, sprite_sampler, uv, l, lod); }
        default: { return textureSampleLevel(sprites_7, sprite_sampler, uv, l, lod); }
    }
}
// texture_sampling.glsl's sampleNearest: the sprite's texels stay square pixels, blending only
// across the one screen pixel where two meet, so a linear (and anisotropic) sampler keeps them crisp.
fn sample_nearest(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>, dv: vec2<f32>,
                  texel_screen: vec2<f32>) -> vec4<f32> {
    let texel_coords = uv / pixel;
    let center = round(texel_coords) - 0.5;
    var offset = texel_coords - center;
    offset = (offset - 0.5) * pixel / max(texel_screen, vec2(1e-12)) + 0.5;
    offset = clamp(offset, vec2(0.0), vec2(1.0));
    return sprite_grad((center + offset) * pixel, layer, du, dv);
}
// sampleRGSS: four rotated-grid samples of the two nearest mip levels, faded in as the texels
// shrink below two screen pixels, over sample_nearest.
fn sample_rgss(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>, dv: vec2<f32>,
               texel_screen: vec2<f32>) -> vec4<f32> {
    let max_texel = max(texel_screen.x, texel_screen.y);
    let min_pixel = min(pixel.x, pixel.y);
    let blend = smoothstep(min_pixel, min_pixel * 2.0, max_texel);
    let du_len = length(du);
    let dv_len = length(dv);
    let effective = sqrt(min(du_len, dv_len) * max(du_len, dv_len));
    let exact = max(0.0, log2(effective / min_pixel));
    let low = floor(exact);
    let mip_blend = fract(exact);
    var offsets = array<vec2<f32>, 4>(
        vec2(0.125, 0.375), vec2(-0.125, -0.375), vec2(0.375, -0.125), vec2(-0.375, 0.125));
    var sum_low = vec4(0.0);
    var sum_high = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        let p = uv + offsets[i] * pixel;
        sum_low += sprite_level(p, layer, low);
        sum_high += sprite_level(p, layer, low + 1.0);
    }
    let rgss = mix(sum_low * 0.25, sum_high * 0.25, mip_blend);
    return mix(sample_nearest(uv, layer, pixel, du, dv, texel_screen), rgss, blend);
}
// Distant terrain tiles (lod/mesh.rs) count their UVs in blocks past 1: each block takes the
// sprite again, as separate faces would under the clamped sampler. Sprite UVs in [0, 1] stay.
fn tile_uv(uv: vec2<f32>) -> vec2<f32> {
    return uv - max(ceil(uv) - 1.0, vec2(0.0));
}
fn sprite(tiled: vec2<f32>, layer: u32) -> vec4<f32> {
    // terrain.fsh's two sampling paths: sampleRGSS for RGSS, sampleNearest otherwise.
    // The gradients come from the unwrapped UV, so the mip level holds across block seams.
    let du = dpdx(tiled);
    let dv = dpdy(tiled);
    let uv = tile_uv(tiled);
    let pixel = vec2(1.0 / globals.filtering.y);
    let texel_screen = sqrt(du * du + dv * dv);
    if u32(globals.filtering.x) == 1u {
        return sample_rgss(uv, layer, pixel, du, dv, texel_screen);
    }
    return sample_nearest(uv, layer, pixel, du, dv, texel_screen);
}

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
    // How the face shines in HDR output (mesher.rs's `EMISSION_BITS`): 0 not
    // at all, 1 to 4 the tier with its bright parts, 5 to 8 the tier all over.
    @location(5) @interpolate(flat) emission: u32,
    @location(6) @interpolate(flat) fade: u32,
    // A distant tile's tile-relative position, for its fade (`faded`).
    @location(7) local: vec3<f32>,
    // terrain.vsh's chunkVisibility: the section's fade-in, from 16 blocks out.
    @location(8) visibility: f32,
    // The camera-relative height, for the water mirror's cut at the plane.
    @location(9) height: f32,
    @location(10) @interpolate(flat) emissive_layer: u32,
}
// Whether a fading tile or section leaves this pixel out (gfx/lod.rs's
// `fade_code`, `fade_out_code`). It fades in, or out, chunk by chunk: each
// column of its own grid (`local`, the tile-relative position: 16 blocks,
// 2^level for coarse tiles) starts in turn over the first half of the fade
// and fades over the second, as a 4x4 ordered pattern in the face's plane,
// its unit the power of two of a block nearest 1.5 pixels, so it stays on
// the terrain as the camera moves. `dx` and `dy`: `local`'s screen
// derivatives.
fn faded(code: u32, local: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> bool {
    if code == 0u { return false; }
    // Sixty-fourths of the fade: 1 to 63 fading in, 65 to 128 fading out.
    let step = (code - 1u) % 129u;
    let out = step >= 64u;
    let progress = f32(select(step, step - 64u, out)) / 64.0;
    let size = exp2(f32(max((code - 1u) / 129u, 4u)));
    let cell = bitcast<vec2<u32>>(vec2<i32>(floor(local.xz / size)));
    var h = cell.x * 73856093u ^ cell.y * 19349663u;
    h = (h ^ (h >> 13u)) * 0x5bd1e995u;
    let start = f32((h ^ (h >> 15u)) & 1023u) / 2048.0;
    let ramp = clamp((progress - start) * 2.0, 0.0, 1.0);
    let shown = select(ramp, 1.0 - ramp, out);
    let n = abs(cross(dx, dy));
    var plane = local.xy;
    if n.x >= n.y && n.x >= n.z {
        plane = local.yz;
    } else if n.y >= n.z {
        plane = local.xz;
    }
    let pixel = max(length(dx), length(dy));
    let unit = exp2(ceil(log2(max(pixel * 1.5, 1.0 / 16.0))));
    let p = vec2<u32>(bitcast<vec2<u32>>(vec2<i32>(floor(plane / unit)))) & vec2(3u);
    let bayer = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    return (f32(bayer[p.y * 4u + p.x]) + 0.5) / 16.0 >= shown;
}
// A fading tile or section is drawn a hair toward the camera (depth is
// reversed: nearer is larger), the newer the more, so it wins where it draws
// over the solid tiles it replaces. Distant tiles go back by 0.2% of their
// distance, so near chunks win over tiles still under them (gfx/lod.rs's
// `previous`, columns not complete yet): near water lies a little below a
// tile's (vanilla's fluid corner heights), and under a smaller step the
// tile's drew over it, two waters, opaque. A tile's UV step is 1/32, a
// section's 1/32768 (`terrain_uv`).
fn fade_depth(clip: vec4<f32>, w: f32) -> vec4<f32> {
    let code = fade_code(w);
    let tile = instance_step(w) > 0.001;
    var scale = select(1.0, 1.0 - 2e-3, tile);
    if code != 0u && (code - 1u) % 129u < 64u {
        scale *= 1.0 + 5e-6 * f32(64u - (code - 1u) % 129u);
    } else if code != 0u && !tile {
        // A section leaving (gfx/lod.rs's `HELD`, `fade_out_code`) goes 2% of
        // its distance toward the camera: far terrain's coarser blocks stand a
        // little out of the real ones, and the tile coming in under it hid it
        // at once, before its fade.
        scale *= 1.02;
    }
    return vec4(clip.xy, clip.z * scale, clip.w);
}
// Horizon Extras' dynamic lights (dynamic_light.rs): the block light, in the vertex's units
// (16 per level), a vertex at camera-relative `pos` gets from the frame's light sources, one
// level less per block. time.z of them are set: none when the extra is off, so no loop runs.
fn dynamic_block_light(pos: vec3<f32>) -> f32 {
    let count = min(u32(globals.time.z), 16u);
    var level = 0.0;
    for (var i = 0u; i < count; i++) {
        let l = globals.dynamic_lights[i];
        level = max(level, l.w - distance(pos, l.xyz));
    }
    return level * 16.0;
}
fn native_colored_light(pos: vec3<f32>) -> vec3<f32> {
    var rgb = vec3(0.0);
    for (var i = 0u; i < min(globals.colored_count.x, 16u); i++) {
        let at = globals.colored_lights[i][0];
        let color = globals.colored_lights[i][1];
        rgb += color.rgb * max(0.0, 1.0 - distance(pos, at.xyz) / at.w) * color.a;
    }
    return min(rgb, vec3(1.0));
}
fn sample_lightmap(sky: f32, block: f32) -> vec4<f32> {
    let uv = vec2(block, sky) * 16.0;
    return textureSampleLevel(lightmap, lightmap_sampler,
        clamp(uv / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
}
// mesher.rs's TerrainVertex holds the UV in fixed point; the instance's
// origin.w is the step: 1/32768 per unit for sections (a power of two on an
// integer below 2^24, so it decodes exactly), 1/32 per block for distant
// terrain tiles (TerrainVertex::tiled). A fading tile negates it past its
// dither code (gfx/lod.rs's `instance`): the step is the fraction either way.
fn terrain_uv(packed: vec2<u32>, w: f32) -> vec2<f32> {
    return vec2<f32>(packed) * instance_step(w);
}
// A whole section's step (a power of two) carries its fade-in in the mantissa's 23 bits, as
// `1 - visibility` (gfx.rs's `section_instance`): masked off, the step; the rest is
// `RenderSection.getVisibility`. A fading one (negative) is fully visible.
fn instance_step(w: f32) -> f32 {
    if w > 0.0 {
        return bitcast<f32>(bitcast<u32>(w) & 0xff800000u);
    }
    return fract(abs(w));
}
fn instance_visibility(w: f32) -> f32 {
    if w > 0.0 {
        return 1.0 - f32(bitcast<u32>(w) & 0x7fffffu) / 8388608.0;
    }
    return 1.0;
}
// A fading distant tile's dither code (gfx/lod.rs's `fade_code`), 0 when whole.
fn fade_code(w: f32) -> u32 {
    return u32(floor(max(-w, 0.0)));
}
@vertex fn vs_terrain(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
// Flat section draws (gfx/flat.rs): one draw per arena page over the frame's
// index list. A section's vertices fill whole 256-vertex blocks of its page:
// the block gives its resident slot, the slot its camera-relative origin.
@group(1) @binding(0) var<storage, read> block_slots: array<u32>;
@group(1) @binding(1) var<storage, read> slot_origins: array<vec4<f32>>;
@vertex fn vs_terrain_flat(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> TerrainOut {
    let origin = slot_origins[block_slots[vertex >> 8u]];
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
fn terrain_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>, w: f32) -> TerrainOut {
    let pos = position + origin;
    var out: TerrainOut;
    out.clip = fade_depth(globals.view_proj * vec4(pos, 1.0), w);
    out.local = position;
    out.uv = uv;
    out.layer = layer;
    out.emissive_layer = 0xffffffffu;
    for (var i = 0u; i < min(globals.emissive_count.x, 32u); i++) {
        if globals.emissive_layers[i].x == (layer & 0x7fffu) {
            out.emissive_layer = globals.emissive_layers[i].y;
            break;
        }
    }
    let block = max(f32(light.y), dynamic_block_light(pos));
    out.color = vec4(color.rgb, 1.0) * sample_lightmap(f32(light.x) / 16.0, block / 16.0);
    if globals.colored_count.x > 0u {
        out.color = vec4(min(out.color.rgb + color.rgb * native_colored_light(pos), vec3(1.0)), out.color.a);
    }
    out.emission = 15u - (u32(round(color.a * 255.0)) & 15u);
    out.height = pos.y;
    out.spherical = length(pos);
    out.cylindrical = max(length(pos.xz), abs(pos.y));
    out.fade = fade_code(w);
    const CHUNK_FULLY_VISIBLE_RANGE: f32 = 16.0;
    out.visibility = mix(1.0, instance_visibility(w),
        clamp((length(pos) - CHUNK_FULLY_VISIBLE_RANGE) / CHUNK_FULLY_VISIBLE_RANGE, 0.0, 1.0));
    return out;
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
fn apply_fog(color: vec4<f32>, spherical: f32, cylindrical: f32) -> vec4<f32> {
    let f = max(linear_fog(spherical, globals.fog.x, globals.fog.y),
                linear_fog(cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}
// hdr.wgsl's brightness: the mean of luminance and the brightest channel, in
// linear light.
fn brightness(c: vec3<f32>) -> f32 {
    let l = pow(max(c, vec3(0.0)), vec3(2.2));
    return 0.5 * (dot(l, vec3(0.2126, 0.7152, 0.0722)) + max(l.r, max(l.g, l.b)));
}
@fragment fn fs_solid(in: TerrainOut) -> @location(0) vec4<f32> {
    return solid_color(in);
}
// The water mirror (gfx.rs's reflection pass): what lies below the water plane
// (`fog_ends.w`, camera-relative) would show above it in the mirror, so it is cut away.
@fragment fn fs_solid_mirror(in: TerrainOut) -> @location(0) vec4<f32> {
    if in.height < globals.fog_ends.w { discard; }
    return solid_color(in);
}
fn solid_color(in: TerrainOut) -> vec4<f32> {
    let dx = dpdx(in.local);
    let dy = dpdy(in.local);
    if faded(in.fade, in.local, dx, dy) { discard; }
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    var color = texel * in.color;
    // terrain.fsh: the fade-in mixes toward the fog colour.
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
    // Below 0.75, alpha tells HDR output (hdr.wgsl) a light source's pixel may
    // shine, and at which tier (mesher::hdr_tier): 0.525 at the lowest, 0 at the
    // brightest. Only pixels as bright as their sprite's threshold may: its
    // bright parts, or all of it. Nothing else reads the frame's alpha.
    var emissive = 1.0;
    if in.emission > 0u {
        let whole = in.emission > 4u;
        let tier = select(in.emission, in.emission - 4u, whole);
        let l = in.layer & 0x7fffu;
        let t = textureLoad(emission_thresholds, vec2(i32(l % SPRITE_SPLIT), i32(l / SPRITE_SPLIT)), 0);
        if brightness(texel.rgb) >= select(t.r, t.g, whole) {
            emissive = 0.7 * f32(4u - min(tier, 4u)) / 4.0;
        }
    }
    if in.emissive_layer != 0xffffffffu {
        let mask = sprite(in.uv, in.emissive_layer);
        return apply_fog(vec4(mix(color.rgb, mask.rgb, mask.a), mix(emissive, 0.1, mask.a)), in.spherical, in.cylindrical);
    }
    return apply_fog(vec4(color.rgb, emissive), in.spherical, in.cylindrical);
}
// Water, glass and other see-through faces draw whole as they fade in, the
// layer drawn first in front (gfx.rs's water tiers): dithered, the faded-out
// pixels showed the layer behind through the one in front, twice the water.
@fragment fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    var color = sprite(in.uv, in.layer & 0x7fffu) * in.color;
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < 0.01 { discard; }
    return apply_fog(color, in.spherical, in.cylindrical);
}

// Sky (SkyRenderer): the 10-vertex sky disc fan at y = 16 (radius 512) in
// the sky colour with sky fog (sky.fsh: fog 0..FogSkyEnd on the spherical
// distance, a step at FogSkyEnd on the cylindrical one), evaluated per pixel
// along the view ray with the fan's interpolated vertex distances; the
// clear colour (fog colour) elsewhere; the sunrise/sunset fan blended over
// it; the dark disc (y = -4) below the horizon when the camera is low.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}
@vertex fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    var out: SkyOut;
    out.clip = vec4(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    return normalize((globals.inverse_view * vec4(ndc.x * globals.misc.z * globals.misc.w,
        ndc.y * globals.misc.w, -1.0, 0.0)).xyz);
}
const PI: f32 = 3.14159265;
// Interpolated (spherical, cylindrical) distance where the ray meets a disc
// fan at height y, or negative when it misses.
fn disc_distance(d: vec3<f32>, y: f32) -> vec2<f32> {
    if d.y * y <= 0.0 { return vec2(-1.0); }
    let p = d * (y / d.y);
    let h = length(p.xz);
    let phi = atan2(p.z, p.x);
    let sector = PI / 4.0;
    let delta = phi - (floor(phi / sector) * sector + sector * 0.5);
    let edge = 512.0 * cos(sector * 0.5) / cos(delta);
    if h > edge { return vec2(-1.0); }
    let s = h / edge;
    let ay = abs(y);
    let rim = sqrt(512.0 * 512.0 + y * y);
    return vec2(ay + s * (rim - ay), ay + s * (512.0 - ay));
}
fn sky_fog(color: vec3<f32>, dist: vec2<f32>) -> vec3<f32> {
    let end = globals.fog_ends.x;
    let f = max(linear_fog(dist.x, 0.0, end), linear_fog(dist.y, end, end));
    return mix(color, globals.fog_color.rgb, f * globals.fog_color.a);
}
// Weight of the fan centre where the ray crosses the sunrise fan (0 when missed):
// Moller-Trumbore from the origin along d, over normals built once a frame
// (gfx/sunrise_fan.rs). The barycentrics are apex/s, k1/s and -k0/s; s is -det.
fn sunrise_weight_full(d: vec3<f32>) -> f32 {
    if globals.sunrise.a <= 0.001 { return 0.0; }
    var best = 0.0;
    var k0 = dot(d, globals.sunrise_fan[16].xyz);
    for (var i = 0; i < 16; i++) {
        let triangle = globals.sunrise_fan[i];
        let k1 = dot(d, globals.sunrise_fan[16 + (i + 1) % 16].xyz);
        // A hit needs s on the side of w, so k0 and k1 on either side of 0.
        if k0 * triangle.w <= 0.0 && k1 * triangle.w >= 0.0 {
            let apex = dot(d, triangle.xyz);
            let s = apex + k1 - k0;
            if abs(s) >= 1e-6 && s * triangle.w > 0.0 && apex * s >= 0.0 && k1 * s >= 0.0
                && k0 * s <= 0.0 {
                best = max(best, apex / s);
            }
        }
        k0 = k1;
    }
    return best;
}
// The rim's YZ projection is an ellipse. Comparing its unsquashed X/Y
// magnitudes with tan(PI/8) selects one of sixteen sectors without atan2.
fn sunrise_triangle_weight(d: vec3<f32>, i: u32) -> f32 {
    let k0 = dot(d, globals.sunrise_fan[16u+i].xyz);
    let k1 = dot(d, globals.sunrise_fan[16u+(i+1u)%16u].xyz);
    let triangle = globals.sunrise_fan[i];
    let apex = dot(d, triangle.xyz);
    let s = apex + k1 - k0;
    if abs(s) >= 1e-6 && s * triangle.w > 0.0 && apex * s >= 0.0
        && k1 * s >= 0.0 && k0 * s <= 0.0 {
        return apex / s;
    }
    return 0.0;
}
fn sunrise_weight(d: vec3<f32>) -> f32 {
    if globals.sunrise.a <= 0.001 { return 0.0; }
    if dot(d.yz, d.yz) < 1e-10 { return sunrise_weight_full(d); }
    // The camera lies in the rim plane. A forward ray can reach the fan
    // only on the apex side. Keep a guard for rounded coplanar normals.
    let apex = dot(d, globals.sunrise_fan[0].xyz);
    if apex * globals.sunrise_fan[0].w < -1e-4 { return 0.0; }
    let y = dot(d, globals.sunrise_fan[16].xyz);
    let x = -dot(d, globals.sunrise_fan[20].xyz);
    let ax = abs(x);
    let ay = abs(y);
    let tangent = 0.414213562373095;
    let quadrant_sector = select(select(0u, 1u, ay > ax * tangent),
        select(2u, 3u, ax < ay * tangent), ay > ax);
    let sector = select(select(8u + quadrant_sector, 7u - quadrant_sector, y >= 0.0),
        select(15u - quadrant_sector, quadrant_sector, y >= 0.0), x >= 0.0);
    var best = max(sunrise_triangle_weight(d, sector),
        sunrise_triangle_weight(d, (sector + 8u) % 16u));
    // Test adjacent sectors only at a rounded boundary; their exact rim
    // normals remain the authority for hit tests and interpolation weights.
    let k0 = dot(d, globals.sunrise_fan[16u+sector].xyz);
    let k1 = dot(d, globals.sunrise_fan[16u+(sector+1u)%16u].xyz);
    if min(abs(k0), abs(k1)) < max(ax, ay) * 0.001 {
        best = max(best, sunrise_triangle_weight(d, (sector + 15u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 1u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 7u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 9u) % 16u));
    }
    return best;
}
// The sky's colour along the view ray `d` (a shader pack's sky stage gets it as its input:
// gfx/native_sky.rs builds that stage from this section).
fn sky_along_ray(d: vec3<f32>) -> vec3<f32> {
    var color = globals.fog_color.rgb;
    let dist = disc_distance(d, 16.0);
    if dist.x >= 0.0 {
        color = sky_fog(globals.sky_color.rgb, dist);
    }
    let w = sunrise_weight(d);
    if w > 0.0 {
        color = mix(color, globals.sunrise.rgb, w * globals.sunrise.a);
    }
    return color;
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4(sky_along_ray(view_ray(in.ndc)), 1.0);
}
@fragment fn fs_dark_disc(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    let dist = disc_distance(d, -4.0);
    if dist.x < 0.0 { discard; }
    return vec4(sky_fog(vec3(0.0), dist), 1.0);
}

// rendertype_lines.vsh: screen-space stroke and camera-space depth separation.
fn outline_clip(vertex: u32, start: vec3<f32>, end: vec3<f32>, width: f32) -> vec4<f32> {
    let position = select(start, end, vertex >= 2u);
    let normal = normalize(end - start);
    let view_shrink = 1.0 - 1.0 / 256.0;
    let line_start = globals.view_proj * vec4((position * globals.outline_params.w) * view_shrink, 1.0);
    let line_end = globals.view_proj * vec4(((position + normal) * globals.outline_params.w) * view_shrink, 1.0);
    let ndc1 = line_start.xyz / line_start.w;
    let ndc2 = line_end.xyz / line_end.w;
    let screen = globals.outline_params.xy;
    let direction = normalize((ndc2.xy - ndc1.xy) * screen);
    var offset = vec2(-direction.y, direction.x) * width / screen;
    if offset.x < 0.0 { offset = -offset; }
    if vertex % 2u != 0u { offset = -offset; }
    return vec4((ndc1 + vec3(offset, 0.0)) * line_start.w, line_start.w);
}
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) spherical: f32,
    @location(1) cylindrical: f32,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> LineOut {
    let position = select(start, end, vertex >= 2u);
    var out: LineOut;
    out.clip = outline_clip(vertex, start, end, width);
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    // ARGB in a little-endian u32 reads as B, G, R, A.
    out.color = argb.zyxw;
    return out;
}
@fragment fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return apply_fog(in.color, in.spherical, in.cylindrical);
}
// Gizmo lines (rendertype_lines with a per-vertex colour and LineWidth in
// pixels; no layering transform).
struct DebugLineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) spherical: f32,
    @location(1) cylindrical: f32,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_debug_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> DebugLineOut {
    let position = select(start, end, vertex >= 2u);
    let view_shrink = 1.0 - 1.0 / 256.0;
    let line_start = globals.view_proj * vec4(position * view_shrink, 1.0);
    let line_end = globals.view_proj * vec4((position + (end - start)) * view_shrink, 1.0);
    let ndc1 = line_start.xyz / line_start.w;
    let ndc2 = line_end.xyz / line_end.w;
    let screen = globals.outline_params.xy;
    let direction = normalize((ndc2.xy - ndc1.xy) * screen);
    var offset = vec2(-direction.y, direction.x) * width / screen;
    if offset.x < 0.0 { offset = -offset; }
    if vertex % 2u != 0u { offset = -offset; }
    var out: DebugLineOut;
    out.clip = vec4((ndc1 + vec3(offset, 0.0)) * line_start.w, line_start.w);
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    // ARGB in a little-endian u32 reads as B, G, R, A.
    out.color = argb.zyxw;
    return out;
}
@fragment fn fs_debug_line(in: DebugLineOut) -> @location(0) vec4<f32> {
    return apply_fog(in.color, in.spherical, in.cylindrical);
}
