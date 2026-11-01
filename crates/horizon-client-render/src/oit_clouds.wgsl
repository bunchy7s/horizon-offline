// Improved transparency's cloud stages (`clouds.fsh` under OIT_*), appended to
// entity.wgsl after oit.wgsl. The clouds' vertex shader and draw are the
// classic ones; only the fragment colour goes through the three stages.

// The cloud's colour; it fades out towards the cloud fog end everywhere but in
// the depth bounds stage, which sees the unfaded alpha.
fn oit_cloud_color(in: Out, front: bool, fade: bool) -> vec4<f32> {
    let color = textureSample(atlas, atlas_sampler, in.uv) * select(in.back_color, in.color, front);
    let faded = 1.0 - linear_fog(in.spherical, 0.0, globals.fog_ends.y);
    return vec4(color.rgb, color.a * select(1.0, faded, fade));
}
@fragment fn fs_oit_cloud_bounds(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = oit_cloud_color(in, front, false);
    if color.a < 0.01 { discard; }
    let linear = oit_linear_depth(in.clip.z);
    let opaque = select(0.0, in.clip.z, color.a > OIT_FULLY_OPAQUE_ALPHA);
    return vec4(-linear, linear, in.clip.z, opaque);
}
@fragment fn fs_oit_cloud_transmittance(in: Out, @builtin(front_facing) front: bool) -> OitCoefficients {
    let color = oit_cloud_color(in, front, true);
    return oit_add_transmittance(1.0 - color.a, in.clip.z, in.clip.xy);
}
@fragment fn fs_oit_cloud_accumulate(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = oit_cloud_color(in, front, true);
    let seen = oit_sample_transmittance(vec2<i32>(in.clip.xy),
        oit_normalize_depth(in.clip.z, in.clip.xy), 1.0 - color.a);
    return vec4(color.rgb * color.a, color.a) * seen;
}
