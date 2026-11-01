override reflections: bool = true;

fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    var color = lit.rgb * (0.9 + 0.1 * luminance(lit.rgb));
    // Water's top mirrors the world while the option is on.
    if reflections && input.fluid == 1u && input.face == 1u && frame.has_reflection == 1u {
        let mirrored = textureSampleLevel(reflection, reflection_sampler, input.screen, 0.0);
        color = mix(color, mirrored.rgb, 0.5);
    }
    return Surface(vec4<f32>(color, lit.a));
}
