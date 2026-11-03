//! The block outline drawn by a shader pack's `gbuffers_line` (or `gbuffers_basic`, as in Iris),
//! so packs that style it (Complementary Reimagined, Super Duper Vanilla) draw it their way, and
//! packs that render the world into part of their buffers (SEUS PTGI HRR) move it with the rest.
//! The edges are the outline buffer Horizon draws itself, one instance per edge, widened to a
//! strip as `outline_clip` in terrain.wgsl does.

/// What `gbuffers_line` reads from an outline edge: its camera-relative ends, ARGB colour and
/// width in pixels. The strip's corners 0 and 1 are at the start, 2 and 3 at the end (six
/// vertices per edge: pack pipelines draw triangle lists); the
/// width is applied in screen space after the pack's transform.
pub fn inputs() -> horizon_shaders::GbuffersInputs {
    let declarations =
        "layout(location = 0) in vec3 hz_l_start;\nlayout(location = 1) in vec3 hz_l_end;\n\
         layout(location = 2) in vec4 hz_l_argb;\nlayout(location = 3) in float hz_l_width;\n\
         int hz_l_c;\n"
            .to_owned();
    // Pack pipelines draw triangle lists: six vertices per edge, the strip's corners
    // 0 1 2 and 2 1 3.
    let body = "    int hz_l_v = int(gl_VertexIndex);\n    \
         hz_l_c = hz_l_v < 3 ? hz_l_v : hz_l_v == 3 ? 2 : hz_l_v == 4 ? 1 : 3;\n    \
         hz_Vertex = vec4(hz_l_c >= 2 ? hz_l_end : hz_l_start, 1.0);\n    \
         hz_Color = hz_l_argb.zyxw;\n    \
         hz_Normal = normalize(hz_l_end - hz_l_start);\n    \
         hz_MultiTexCoord0 = vec4(0.0, 0.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n"
        .to_owned();
    // ARGB in a little-endian u32 reads as B, G, R, A.
    let attributes = [
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
    ];
    let after = "    {\n        \
         vec4 hz_l_a = hz_ModelViewProjectionMatrix * vec4(hz_l_start, 1.0);\n        \
         vec4 hz_l_b = hz_ModelViewProjectionMatrix * vec4(hz_l_end, 1.0);\n        \
         vec2 hz_l_d = (hz_l_b.xy / hz_l_b.w - hz_l_a.xy / hz_l_a.w) * hz_target.xy;\n        \
         hz_l_d = dot(hz_l_d, hz_l_d) > 1e-12 ? normalize(hz_l_d) : vec2(1.0, 0.0);\n        \
         vec2 hz_l_o = vec2(-hz_l_d.y, hz_l_d.x) * hz_l_width / hz_target.xy;\n        \
         if (hz_l_o.x < 0.0) hz_l_o = -hz_l_o;\n        \
         if ((hz_l_c & 1) != 0) hz_l_o = -hz_l_o;\n        \
         gl_Position.xy += hz_l_o * gl_Position.w;\n    }\n"
        .to_owned();
    horizon_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after,
        alpha_test: None,
    }
}

/// The outline edges as the pack's lines program reads them: one 32-byte instance per edge.
pub fn layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32];
    wgpu::VertexBufferLayout {
        array_stride: 32,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}
