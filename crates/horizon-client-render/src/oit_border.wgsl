// Improved transparency's stages for the world border (`OIT_WORLD_BORDER`:
// `world_border.fsh` with `OIT_ADDITIVE`), appended to border.wgsl after
// oit.wgsl.
@fragment fn fs_oit_border_bounds(in: Out) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(border_color(in).a, in.clip);
}
@fragment fn fs_oit_border_transmittance(in: Out) -> OitCoefficients {
    discard;
    return OitCoefficients(vec4(0.0), vec4(0.0));
}
@fragment fn fs_oit_border_accumulate(in: Out) -> @location(0) vec4<f32> {
    return oit_accumulate_additive(border_color(in), 0.0, in.clip);
}
