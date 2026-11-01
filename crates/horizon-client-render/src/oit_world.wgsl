// Improved transparency's terrain stages and its screen passes (oit.wgsl
// has the shared maths), appended to terrain.wgsl after oit.wgsl.
// The terrain's translucent layer, once per stage.
fn oit_terrain_color(in: TerrainOut) -> vec4<f32> {
    var color = sprite(in.uv, in.layer & 0x7fffu) * in.color;
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < OIT_ALPHA_CUTOUT { discard; }
    return color;
}
// Stage 1: (-nearest, furthest, nearest device depth, nearest opaque device
// depth), combined by max blending.
@fragment fn fs_oit_bounds(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = oit_terrain_color(in);
    let linear = oit_linear_depth(in.clip.z);
    let opaque = select(0.0, in.clip.z, color.a > OIT_FULLY_OPAQUE_ALPHA);
    return vec4(-linear, linear, in.clip.z, opaque);
}
// Stage 2.
@fragment fn fs_oit_transmittance(in: TerrainOut) -> OitCoefficients {
    let color = oit_terrain_color(in);
    return oit_add_transmittance(1.0 - color.a, in.clip.z, in.clip.xy);
}
// Stage 3: premultiplied colour, weighted by the transmittance in front.
@fragment fn fs_oit_accumulate(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = oit_terrain_color(in);
    let seen = oit_sample_transmittance(vec2<i32>(in.clip.xy),
        oit_normalize_depth(in.clip.z, in.clip.xy), 1.0 - color.a);
    let weighted = vec4(color.rgb * color.a, color.a) * seen;
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    // The fog colour is premultiplied by the same weight.
    let rgb = mix(weighted.rgb, globals.fog_color.rgb * weighted.a, f * globals.fog_color.a);
    return vec4(rgb, weighted.a);
}

struct OitFragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
// After the bounds stage: a fully opaque fragment hides what is behind it, so
// the far bound stops there and its depth goes into the depth buffer.
@fragment fn fs_oit_cull(@builtin(position) pos: vec4<f32>) -> OitFragment {
    let b = textureLoad(oit_bounds, vec2<i32>(pos.xy), 0);
    let opaque = oit_linear_depth(b.a);
    return OitFragment(vec4(b.r, min(b.g, opaque), b.b, b.a), b.a);
}
// The sum over the world, renormalized to the coverage the transform gives.
@fragment fn fs_oit_composite(@builtin(position) pos: vec4<f32>) -> OitFragment {
    let p = vec2<i32>(pos.xy);
    let accumulated = textureLoad(oit_accumulated, p, 0);
    let seen = oit_sample_transmittance(p, 100000.0, 1.0);
    let coverage = 1.0 - select(seen, 0.0, seen < OIT_FULLY_OPAQUE_TRANSMITTANCE);
    // Additive surfaces add light but no coverage.
    if coverage < 0.00001 && dot(accumulated.rgb, vec3(1.0)) < 0.00001 { discard; }
    let scale = select(1.0, coverage / accumulated.a, accumulated.a > 0.00001);
    return OitFragment(vec4(accumulated.rgb * scale, coverage), textureLoad(oit_bounds, p, 0).b);
}
