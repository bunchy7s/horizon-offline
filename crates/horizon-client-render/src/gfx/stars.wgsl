// RenderPipelines.STARS: positions only, coloured by the star brightness.
struct Stars {
    // Projection * view * the sky's rotation (SkyRenderer.renderSunMoonAndStars).
    clip_from_sky: mat4x4<f32>,
    // starBrightness in every channel (RenderSystem.setShaderColor).
    color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> stars: Stars;

@vertex fn vs_stars(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return stars.clip_from_sky * vec4(position, 1.0);
}

// The colour premultiplied, as the celestial blend (src alpha, one) would: the pipeline adds it.
// The alpha is HDR output's mark (hdr.wgsl): stars are points of light, at the bright tier,
// fully lifted wherever they show at all. Nothing else reads the frame's alpha.
@fragment fn fs_stars() -> @location(0) vec4<f32> {
    let b = stars.color.a;
    return vec4(stars.color.rgb * b, 0.175);
}
