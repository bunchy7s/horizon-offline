// Entities, dropped items and particles: camera-relative vertices textured
// from the shared 2D atlas, lit through the same lightmap as terrain.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    sky_color: vec4<f32>,
    misc: vec4<f32>,
    inverse_view: mat4x4<f32>,
    sunrise: vec4<f32>,
    sun: vec4<f32>,
    // x: sky fog end, y: cloud fog end
    fog_ends: vec4<f32>,
    // Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: array<vec4<f32>, 2>,
    // x: vanilla's GameTime ((game time % 24000 + partial tick) / 24000)
    // y: Nether cardinal lighting, z: how many dynamic_lights are set,
    // w: the glint scroll clock (`Glint::scroll_ticks`).
    time: vec4<f32>,
    crumbling_sprite: vec4<f32>,
    // Selection outline parameters (read by terrain.wgsl).
    outline_params: vec4<f32>,
    // Horizon Extras' dynamic lights: camera-relative position and level (dynamic_light.rs).
    dynamic_lights: array<vec4<f32>, 16>,
    sunrise_fan: array<vec4<f32>, 32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;
// enchanted_glint_item on its own repeating texture (vanilla's GlintSampler).
@group(0) @binding(5) var glint_tex: texture_2d<f32>;
@group(0) @binding(6) var glint_sampler: sampler;
// The armour layers' GlintSampler: vanilla samples enchanted_glint_armor with
// the same repeating, linearly filtered sampler as the item glint.
@group(0) @binding(7) var glint_armor_tex: texture_2d<f32>;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) spherical: f32,
    @location(3) cylindrical: f32,
    // entity.vsh's overlayColor: the OverlayTexture texel (see overlay_texel).
    @location(5) overlay_color: vec4<f32>,
    @location(4) @interpolate(flat) overlay: u32,
    // Overlay 6: the sprite's v origin and height, for v repeated inside it.
    @location(6) @interpolate(flat) wrap: vec2<f32>,
    // Overlays 9 and 10: projection_from_position(gl_Position), the End portal's texProj0.
    @location(7) proj: vec4<f32>,
    @location(8) back_color: vec4<f32>,
    // entity.vsh's lightMapColor, multiplied in after the overlay.
    @location(9) light_color: vec4<f32>,
    @location(10) @interpolate(flat) weather_rect: vec4<f32>,
    // The `GLINT_TEXTURING` sample for overlay 7 (item foil), or the model UV
    // `ARMOR_ENTITY_GLINT_TEXTURING` scrolls for overlay 13 (armour).
    @location(11) glint_uv: vec2<f32>,
    // 1 drops the quad's back-facing pixels (Vertex::normal w = 1), like a culling render type.
    @location(12) @interpolate(flat) cull: u32,
}
// Shipped light.glsl; normals are already transformed and RGBA8_SNORM packed.
fn entity_light(normal: vec3<f32>, nether: bool) -> f32 {
    let l0 = normalize(vec3(0.2, 1.0, -0.7));
    let l1 = normalize(vec3(-0.2, select(1.0, -1.0, nether), 0.7));
    return min(1.0, (max(0.0, dot(l0, normal)) + max(0.0, dot(l1, normal))) * 0.6 + 0.4);
}
// The OverlayTexture texel entity.vsh fetches (constants from overlay.rs):
// the hurt red for overlays 1 and 8, the white row at column 15 for the
// primed TNT flash (5) and at column overlay - 16 for 16..=31 (a creeper's
// swelling). Alpha 1 changes nothing.
fn overlay_texel(overlay: u32) -> vec4<f32> {
    if overlay == 1u || overlay == 8u { return HURT_OVERLAY; }
    if overlay == 5u { return vec4(1.0, 1.0, 1.0, WHITE_OVERLAY_ALPHA[15]); }
    if overlay >= 16u && overlay < 32u { return vec4(1.0, 1.0, 1.0, WHITE_OVERLAY_ALPHA[overlay - 16u]); }
    return vec4(1.0);
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
fn entity_vertex(position: vec3<f32>, uv: vec2<f32>,
    overlay: u32, color: vec4<f32>, light: vec4<u32>, normal: vec4<f32>) -> Out {
    var out: Out;
    out.clip = globals.view_proj * vec4(position, 1.0);
    out.proj = vec4(out.clip.xy * 0.5 + out.clip.w * 0.5, out.clip.zw);
    out.uv = uv;
    let l = vec2(max(f32(light.y), dynamic_block_light(position)), f32(light.x));
    let lm = textureSampleLevel(lightmap, lightmap_sampler,
        clamp(l / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
    // Celestial bodies, clouds, beacon beams, End portals and breaking cracks are unlit.
    let unlit = overlay == 2u || overlay == 4u || overlay == 6u || overlay == 9u || overlay == 10u
        || overlay == 11u || overlay == 12u;
    out.light_color = select(lm, vec4(1.0), unlit);
    // Overlay 6 (beacon beams): the light bytes carry the sprite's v range; 11 and 12
    // (cracks): the destroy stage sprite's origin.
    out.wrap = vec2(f32(light.x | (light.y << 8u)), f32(light.z | (light.w << 8u))) / 65535.0;
    // minecraft_mix_light: the vertex colour times the cardinal light only.
    var c = color;
    var back = c;
    // Normal w: 0 keeps the baked colour, +-127 shades by the normal, and a small positive 1
    // keeps the baked colour but culls back faces (entitySolid models such as the decorated pot).
    out.cull = select(0u, 1u, normal.w > 0.0 && normal.w < 0.5);
    if abs(normal.w) >= 0.5 {
        c = vec4(c.rgb * entity_light(normal.xyz, globals.time.y != 0.0), c.a);
        back = select(c, vec4(back.rgb * entity_light(-normal.xyz, globals.time.y != 0.0), back.a), normal.w < 0.0);
    }
    out.color = c;
    out.back_color = back;
    out.overlay_color = overlay_texel(overlay);
    // Sky geometry (clouds, celestial bodies) is exempt from render-distance fog.
    out.spherical = select(length(position), 0.0, overlay >= 2u && overlay <= 4u);
    out.cylindrical = select(max(length(position.xz), abs(position.y)), 0.0, overlay >= 2u && overlay <= 4u);
    out.overlay = overlay;
    out.glint_uv = vec2(0.0);
    if overlay == 4u { out.spherical = length(position); }
    return out;
}
@vertex fn vs_entity(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>,
    @location(5) normal: vec4<f32>, @location(6) glint_uv: vec2<f32>) -> Out {
    var out = entity_vertex(position, uv, overlay, color, light, normal);
    out.glint_uv = glint_uv;
    return out;
}
// Retained bolt-local geometry, translated using the same f64 camera
// subtraction as the former CPU path before adding each local f32 position.
// The bolt offset is instance attribute 7: 6 is the entity vertex's glint UV.
@vertex fn vs_lightning(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>, @location(5) normal: vec4<f32>,
    @location(7) offset: vec3<f32>) -> Out {
    return entity_vertex(offset + position, uv, overlay, color, light, normal);
}
struct CloudInfo {
    offset: vec4<f32>,
    colors: array<vec4<f32>, 4>,
}
@group(1) @binding(0) var<uniform> cloud: CloudInfo;
@vertex fn vs_cloud(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>, @location(5) normal: vec4<f32>) -> Out {
    return entity_vertex(position + cloud.offset.xyz, uv, overlay, cloud.colors[light.x], light, normal);
}
// rendertype_end_portal.fsh: the layer colours.
const PORTAL_COLORS = array<vec3<f32>, 16>(
    vec3(0.022087, 0.098399, 0.110818),
    vec3(0.011892, 0.095924, 0.089485),
    vec3(0.027636, 0.101689, 0.100326),
    vec3(0.046564, 0.109883, 0.114838),
    vec3(0.064901, 0.117696, 0.097189),
    vec3(0.063761, 0.086895, 0.123646),
    vec3(0.084817, 0.111994, 0.166380),
    vec3(0.097489, 0.154120, 0.091064),
    vec3(0.106152, 0.131144, 0.195191),
    vec3(0.097721, 0.110188, 0.187229),
    vec3(0.133516, 0.138278, 0.148582),
    vec3(0.070006, 0.243332, 0.235792),
    vec3(0.196766, 0.142899, 0.214696),
    vec3(0.047281, 0.315338, 0.321970),
    vec3(0.204675, 0.390010, 0.302066),
    vec3(0.080955, 0.314821, 0.661491),
);
// textureProj on a repeating texture that lives in the atlas at `rect`.
fn portal_sample(rect: vec4<f32>, p: vec4<f32>) -> vec3<f32> {
    let st = fract(p.xy / p.w);
    return textureSampleLevel(atlas, atlas_sampler, rect.xy + st * rect.zw, 0.0).rgb;
}
// texProj0 * end_portal_layer(layer): the row vector through mat4(scale * rotate),
// translate and SCALE_TRANSLATE in turn.
fn portal_layer(p: vec4<f32>, layer: f32) -> vec4<f32> {
    let a = radians((layer * layer * 4321.0 + layer * 9.0) * 2.0);
    let c = cos(a);
    let s = sin(a);
    let k = (4.5 - layer / 4.0) * 2.0;
    var v = vec4(k * (c * p.x - s * p.y), k * (s * p.x + c * p.y), p.z, p.w);
    v = vec4(v.x + 17.0 / layer * v.w, v.y + (2.0 + layer / 1.5) * (globals.time.x * 1.5) * v.w, v.z, v.w);
    return vec4(0.5 * v.x + 0.25 * v.w, 0.5 * v.y + 0.25 * v.w, v.z, v.w);
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
// TextureTransform.setupGlintTexturing(0.16), ARMOR_ENTITY_GLINT_TEXTURING.
// globals.time.w is (Util.getMillis() * glintSpeed * 8) modulo 330000, the
// common period of both offsets.
fn armor_glint_uv(model: vec2<f32>) -> vec2<f32> {
    let ticks = globals.time.w;
    let offset0 = (ticks % 110000.0) / 110000.0;
    let offset1 = (ticks % 30000.0) / 30000.0;
    let angle = 0.17453292;
    let c = cos(angle);
    let s = sin(angle);
    let scaled = model * 0.16;
    return vec2(scaled.x * c - scaled.y * s - offset0,
                scaled.x * s + scaled.y * c + offset1);
}
// RenderPipelines.GLINT: the source colour is scaled by itself, so `GlintAlpha`
// is squared along with the sample.
fn glint_over(color: vec4<f32>, glint: vec4<f32>) -> vec4<f32> {
    let alpha = globals.fog_ends.w;
    let g = alpha * glint;
    return vec4(color.rgb + g.rgb * g.rgb, max(color.a, alpha));
}
// CRUMBLING_UV_BEGIN
fn crumbling_uv(uv: vec2<f32>, packed_origin: vec2<f32>, sprite_size: vec2<f32>, dimensions: vec2<u32>) -> vec2<f32> {
    let size = vec2<f32>(dimensions);
    // Undo stage_light's 16-bit quantization before adding local texels. Otherwise
    // different stages can fall on opposite sides of a nearest-sampling boundary.
    let origin = round(packed_origin * size);
    let extent = max(round(sprite_size * size), vec2(1.0));
    let local = min(floor(fract(uv) * extent), extent - vec2(1.0));
    return (origin + local + vec2(0.5)) / size;
}
// CRUMBLING_UV_END
@fragment fn fs_entity(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if in.overlay == 6u { uv.y = in.wrap.x + fract(in.uv.y) * in.wrap.y; }
    if in.overlay == 11u || in.overlay == 12u {
        // Each crack's stage sprite origin comes in its light bytes (breaking::stage_light).
        uv = crumbling_uv(in.uv, in.wrap, globals.crumbling_sprite.zw, textureDimensions(atlas));
    }
    var color = textureSample(atlas, atlas_sampler, uv) * select(in.back_color, in.color, front);
    if in.overlay == 9u || in.overlay == 10u {
        // rendertype_end_portal: end_sky and PORTAL_LAYERS (15 for portals, 16
        // for gateways) layers of end_portal.png projected from the screen.
        let layers = select(15, 16, in.overlay == 10u);
        var c = portal_sample(globals.end_portal_sprites[0], in.proj) * PORTAL_COLORS[0];
        for (var i = 0; i < layers; i++) {
            c += portal_sample(globals.end_portal_sprites[1], portal_layer(in.proj, f32(i + 1))) * PORTAL_COLORS[i];
        }
        color = vec4(c, 1.0);
    } else if in.overlay == 4u {
        // clouds.fsh: alpha fades out towards the cloud fog end.
        return vec4(color.rgb, color.a * (1.0 - linear_fog(in.spherical, 0.0, globals.fog_ends.y)));
    }
    if color.a < 0.1 || (in.cull != 0u && !front) { discard; }
    // entity.fsh calculateFinalColor: the overlay (hurt red, TNT white) over the
    // lit texture, then the lightmap, then the glint and fog.
    color = vec4(mix(in.overlay_color.rgb, color.rgb, in.overlay_color.a), color.a);
    color *= in.light_color;
    if in.overlay == 7u || in.overlay == 8u || in.overlay == 33u {
        // 33: a translucent item with the glint.
        // Enchantment glint: item.fsh's GLINT path, sampled from
        // enchanted_glint_item at the `GLINT_TEXTURING` UV.
        color = glint_over(color, textureSample(glint_tex, glint_sampler, in.glint_uv));
    } else if in.overlay == 13u {
        // Enchanted armour: enchanted_glint_armor at the scrolling model UV.
        color = glint_over(
            color,
            textureSample(glint_armor_tex, glint_sampler, armor_glint_uv(in.glint_uv)),
        );
    }
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}

// rendertype_outline.fsh: the vertex colour (the team colour; outline vertices carry no
// normal, so no cardinal light) wherever the texture is not fully clear.
// rendertype_entity_shadow.fsh: the shadow texture times the vertex colour,
// then fog, with no alpha cutout, so a faint shadow in dim light still draws.
@fragment fn fs_entity_shadow(in: Out) -> @location(0) vec4<f32> {
    var color = textureSample(atlas, atlas_sampler, in.uv) * in.color;
    color *= in.light_color;
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}

@fragment fn fs_outline(in: Out) -> @location(0) vec4<f32> {
    if textureSample(atlas, atlas_sampler, in.uv).a == 0.0 { discard; }
    return vec4(in.color.rgb, 1.0);
}

fn celestial_color(in: Out) -> vec4<f32> {
    return textureSample(atlas, atlas_sampler, in.uv) * in.color * in.light_color;
}
@fragment fn fs_celestial(in: Out) -> @location(0) vec4<f32> {
    return celestial_color(in);
}

// The sky's celestial pass (sun, moon, the End sky): fs_celestial's light,
// premultiplied for an additive (one, one) blend, and as alpha how much of it
// the texel adds, which the blend takes off the frame's alpha. HDR output
// (hdr.wgsl) lifts only where that left it low: the sun's and moon's discs, not
// the dark rest of their quads or the dim End sky.
@fragment fn fs_sky_celestial(in: Out) -> @location(0) vec4<f32> {
    let color = celestial_color(in);
    let added = color.rgb * color.a;
    return vec4(added, smoothstep(0.1, 0.3, max(added.r, max(added.g, added.b))));
}

// RenderPipelines.ENERGY_SWIRL (entity.fsh with EMISSIVE and NO_OVERLAY): the sprite, scrolled by
// the UV and repeated within itself, times the layer colour 0xFF808080, unlit; the pipeline adds
// it to the frame. The light bytes carry the sprite's origin and the colour bytes its size, both
// as breaking::stage_light packs them.
@fragment fn fs_energy_swirl(in: Out) -> @location(0) vec4<f32> {
    let bytes = round(in.color * 255.0);
    let size = vec2(bytes.x + bytes.y * 256.0, bytes.z + bytes.w * 256.0) / 65535.0;
    let color = textureSample(atlas, atlas_sampler, crumbling_uv(in.uv, in.wrap, size, textureDimensions(atlas)));
    if color.a < 0.1 { discard; }
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb * (128.0 / 255.0), globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}

// RenderPipelines.WATER_MASK: depth only (the pipeline masks the colour off).
@fragment fn fs_water_mask() -> @location(0) vec4<f32> {
    return vec4(0.0);
}

// WEATHER_MATH_BEGIN
// Unsigned 64-bit helpers (low word first). Snow adds a random double to a
// float drift before casting to float. Retaining the 53-bit base avoids early
// rounding, including ties, without requiring optional GPU float64 support.
fn weather_shr_jam(v: vec2<u32>, n: u32) -> vec2<u32> {
    if n == 0u { return v; }
    if n < 32u {
        return vec2((v.x >> n) | (v.y << (32u - n)) | select(0u, 1u, (v.x << (32u - n)) != 0u), v.y >> n);
    }
    if n == 32u { return vec2(v.y | select(0u, 1u, v.x != 0u), 0u); }
    if n < 64u {
        return vec2((v.y >> (n - 32u)) | select(0u, 1u, v.x != 0u || (v.y << (64u - n)) != 0u), 0u);
    }
    return vec2(select(0u, 1u, any(v != vec2(0u))), 0u);
}
fn weather_add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}
fn weather_sub64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2(a.x - b.x, a.y - b.y - select(0u, 1u, a.x < b.x));
}
fn weather_double_plus_float(base: vec2<u32>, delta: f32) -> f32 {
    // The random base is nonnegative and either zero or >= 2^-53.
    if all(base == vec2(0u)) { return select(delta, 0.0, delta == 0.0); }
    let db = bitcast<u32>(delta);
    let mag = db & 0x7fffffffu;
    let be = i32((base.y >> 20u) & 2047u) - 1023;
    var de = i32(mag >> 23u) - 127;
    var dm = (mag & 0x7fffffu) | 0x800000u;
    if mag < 0x800000u {
        dm = mag;
        if dm == 0u { de = -200; }
        else {
            let shift = countLeadingZeros(dm) - 8u;
            dm <<= shift;
            de = -126 - i32(shift);
        }
    }
    let e = max(be, de);
    // Leading significand bit at 61, with room for addition and guard bits.
    var a = vec2(base.x << 9u, ((base.y & 0xfffffu) | 0x100000u) << 9u | (base.x >> 23u));
    var b = vec2(0u, dm << 6u);
    a = weather_shr_jam(a, u32(e - be));
    b = weather_shr_jam(b, u32(e - de));
    var sign = 0u;
    var value: vec2<u32>;
    if (db >> 31u) == 0u { value = weather_add64(a, b); }
    else if a.y > b.y || (a.y == b.y && a.x >= b.x) { value = weather_sub64(a, b); }
    else { value = weather_sub64(b, a); sign = 0x80000000u; }
    if all(value == vec2(0u)) { return 0.0; }
    let leading = select(31u - countLeadingZeros(value.x), 63u - countLeadingZeros(value.y), value.y != 0u);
    var exponent = e + i32(leading) - 61;
    // Normalize the leading bit to 62. After jamming, bit 38 is the guard
    // bit and bits 0..37 distinguish a tie from a value above the midpoint.
    let shift = 62u - leading;
    if shift >= 32u { value = vec2(0u, value.x << (shift - 32u)); }
    else if shift != 0u { value = vec2(value.x << shift, (value.y << shift) | (value.x >> (32u - shift))); }
    // Vanilla first rounds the addition to double, then casts to float.
    // Round at bit 10 (53 retained bits) before the 24-bit rounding below;
    // otherwise a double-rounding midpoint can select the wrong float.
    let double_guard = (value.x & 512u) != 0u;
    let double_sticky = (value.x & 511u) != 0u;
    let double_odd = (value.x & 1024u) != 0u;
    value.x &= 0xfffffc00u;
    if double_guard && (double_sticky || double_odd) {
        value = weather_add64(value, vec2(1024u, 0u));
    }
    if (value.y & 0x80000000u) != 0u {
        value = weather_shr_jam(value, 1u);
        exponent += 1;
    }
    var mantissa = value.y >> 7u;
    let guard = (value.y & 64u) != 0u;
    let sticky = (value.y & 63u) != 0u || value.x != 0u;
    if guard && (sticky || (mantissa & 1u) != 0u) { mantissa += 1u; }
    if mantissa >= 0x1000000u { mantissa >>= 1u; exponent += 1; }
    return bitcast<f32>(sign | (u32(exponent + 127) << 23u) | (mantissa & 0x7fffffu));
}
fn weather_phase(animation: vec4<f32>, bases: vec4<u32>, snow: u32, time: vec4<f32>) -> vec2<f32> {
    if snow == 0u {
        let ticks = time.x + animation.y;
        let t = ticks + time.z;
        let step = -t / 32.0;
        let offset = step * animation.x;
        return vec2(0.0, offset % 32.0);
    }
    let t = time.x + time.z;
    let u_time = t * 0.01;
    let du = u_time * animation.z;
    let v_time = t * animation.w;
    let dv = v_time * 0.001;
    let u = weather_double_plus_float(bases.xy, du);
    let v = weather_double_plus_float(bases.zw, dv);
    let fall_time = time.y + time.z;
    let fall = -fall_time / 512.0;
    return vec2(u, fall + v);
}
// Atlas sampler is nearest with no mipmaps. Clamp the integer texel inside
// the sprite even when a fractional coordinate rounds up at its far edge.
fn weather_texel(uv: vec2<f32>, rect: vec4<f32>, dimensions: vec2<u32>) -> vec2<i32> {
    let size = vec2<f32>(dimensions);
    let origin = vec2<i32>(round(rect.xy * size));
    let extent = max(vec2<i32>(round(rect.zw * size)), vec2(1));
    let local = min(vec2<i32>(floor(fract(uv) * vec2<f32>(extent))), extent - vec2(1));
    return origin + local;
}
// WEATHER_MATH_END

struct WeatherTime { values: vec4<f32> }
@group(1) @binding(1) var<uniform> weather_time: WeatherTime;
@vertex fn vs_weather(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>, @location(3) animation: vec4<f32>,
    @location(4) bases: vec4<u32>, @location(5) color: vec4<f32>,
    @location(6) light: vec4<u32>, @location(7) snow: u32) -> Out {
    let phase = weather_phase(animation, bases, snow, weather_time.values);
    var out = entity_vertex(position, uv + phase, 0u, color, light, vec4(0.0));
    out.weather_rect = rect;
    return out;
}
@fragment fn fs_weather(in: Out) -> @location(0) vec4<f32> {
    // Repeat within this sprite, never across the atlas. The atlas uses nearest
    // filtering and no mipmaps, so fractional UVs have no seam derivatives.
    let texel = weather_texel(in.uv, in.weather_rect, textureDimensions(atlas));
    var color = textureLoad(atlas, texel, 0) * in.color;
    if color.a < 0.1 { discard; }
    color *= in.light_color;
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}
