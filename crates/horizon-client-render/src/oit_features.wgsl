// Improved transparency's stages for what the entity shader draws after the
// opaque world (`LevelRenderer.executeOit`'s "Features, World Border,
// Weather" pass), appended to entity.wgsl after oit.wgsl. The colour of each
// is the classic fragment's, so the shaders cannot drift apart.

// Items, particles, text, breaking cracks (`RenderPipelines.OIT_ENTITY` and
// the others on entity.fsh).
@fragment fn fs_oit_entity_bounds(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(entity_color(in, front).a, in.clip);
}
@fragment fn fs_oit_entity_transmittance(in: Out, @builtin(front_facing) front: bool) -> OitCoefficients {
    return oit_transmittance_fragment(entity_color(in, front).a, in.clip);
}
@fragment fn fs_oit_entity_accumulate(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return oit_accumulate_fragment(entity_color(in, front), in.clip);
}
// `OIT_ENTITY_SHADOW`.
@fragment fn fs_oit_shadow_bounds(in: Out) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(shadow_color(in).a, in.clip);
}
@fragment fn fs_oit_shadow_transmittance(in: Out) -> OitCoefficients {
    return oit_transmittance_fragment(shadow_color(in).a, in.clip);
}
@fragment fn fs_oit_shadow_accumulate(in: Out) -> @location(0) vec4<f32> {
    return oit_accumulate_fragment(shadow_color(in), in.clip);
}
// `OIT_WEATHER` (the particle shader).
@fragment fn fs_oit_weather_bounds(in: Out) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(weather_color(in).a, in.clip);
}
@fragment fn fs_oit_weather_transmittance(in: Out) -> OitCoefficients {
    return oit_transmittance_fragment(weather_color(in).a, in.clip);
}
@fragment fn fs_oit_weather_accumulate(in: Out) -> @location(0) vec4<f32> {
    return oit_accumulate_fragment(weather_color(in), in.clip);
}
// `OIT_ENERGY_SWIRL`: entity.fsh with `OIT_ADDITIVE`, which caps the alpha.
@fragment fn fs_oit_swirl_bounds(in: Out) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(min(0.99, energy_swirl_base(in).a), in.clip);
}
@fragment fn fs_oit_swirl_transmittance(in: Out) -> OitCoefficients {
    discard;
    return OitCoefficients(vec4(0.0), vec4(0.0));
}
@fragment fn fs_oit_swirl_accumulate(in: Out) -> @location(0) vec4<f32> {
    let color = energy_swirl_base(in);
    return oit_accumulate_additive(vec4(color.rgb, min(0.99, color.a)), energy_swirl_fog(in), in.clip);
}
// `OIT_LIGHTNING` and `OIT_DRAGON_RAYS`: additive (`OIT_ADDITIVE`), the
// classic fragment's colour (fs_celestial) with the alpha capped.
@fragment fn fs_oit_lightning_bounds(in: Out) -> @location(0) vec4<f32> {
    return oit_bounds_fragment(min(0.99, celestial_color(in).a), in.clip);
}
@fragment fn fs_oit_lightning_transmittance(in: Out) -> OitCoefficients {
    discard;
    return OitCoefficients(vec4(0.0), vec4(0.0));
}
@fragment fn fs_oit_lightning_accumulate(in: Out) -> @location(0) vec4<f32> {
    let color = celestial_color(in);
    return oit_accumulate_additive(vec4(color.rgb, min(0.99, color.a)), 0.0, in.clip);
}
