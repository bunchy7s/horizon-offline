//! Original extension-query contracts, from EXT_gpu_shader4 section 8.7.
use std::io::{Cursor, Write};

use horizon_shaders::{GameInputs, Macros, Pack, Program, load_post, read_properties};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Expression, ImageQuery, ScalarKind, TypeInner, VectorSize};

const VERTEX: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";

fn program(fragment: &str) -> Result<Program, Vec<String>> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in [
        ("shaders/composite.vsh", VERTEX),
        ("shaders/composite.fsh", fragment),
    ] {
        archive
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    }
    let pack =
        Pack::from_zip_bytes("original-size.zip", archive.finish().unwrap().into_inner()).unwrap();
    let mut macros = Macros::standard();
    let (properties, notes) = read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    if !post.notes.is_empty() {
        return Err(post.notes);
    }
    Ok(post.programs.into_iter().next().unwrap().program.unwrap())
}

fn validate_and_write_spirv(module: &naga::Module) -> naga::valid::ModuleInfo {
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .unwrap();
    let entry = &module.entry_points[0];
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: entry.stage,
        entry_point: entry.name.clone(),
    };
    let words =
        naga::back::spv::write_vec(module, &info, &Default::default(), Some(&pipeline)).unwrap();
    assert_eq!(words[0], 0x0723_0203);
    info
}

fn query_shapes(module: &naga::Module) -> Vec<u8> {
    let info = validate_and_write_spirv(module);
    let mut dimensions = Vec::new();
    for (function, body) in module.functions.iter() {
        for (expression, value) in body.expressions.iter() {
            if let Expression::ImageQuery {
                query: ImageQuery::Size { level: Some(_) },
                ..
            } = value
            {
                let ty = info[function][expression].ty.inner_with(&module.types);
                dimensions.push(match *ty {
                    TypeInner::Scalar(scalar)
                        if matches!(scalar.kind, ScalarKind::Sint | ScalarKind::Uint) =>
                    {
                        1
                    }
                    TypeInner::Vector { size, scalar }
                        if matches!(scalar.kind, ScalarKind::Sint | ScalarKind::Uint) =>
                    {
                        match size {
                            VectorSize::Bi => 2,
                            VectorSize::Tri => 3,
                            VectorSize::Quad => 4,
                        }
                    }
                    _ => panic!("texture size must return integer dimensions: {ty:?}"),
                });
            }
        }
    }
    dimensions
}

fn query_source(dimension: u8, sampler_prefix: &str, function: &str) -> String {
    let (ty, color) = match dimension {
        1 => ("int", "vec4(float(size), 0.0, 0.0, 1.0)"),
        2 => ("ivec2", "vec4(vec2(size), 0.0, 1.0)"),
        3 => ("ivec3", "vec4(vec3(size), 1.0)"),
        _ => unreachable!(),
    };
    format!(
        "#version 120\n#extension GL_EXT_gpu_shader4 : enable\n\
         uniform {sampler_prefix}sampler{dimension}D inputTexture;\nuniform int requestedLod;\n\
         void main() {{ {ty} size = {function}(inputTexture, requestedLod); gl_FragColor = {color}; }}\n"
    )
}

fn legacy_dimension(dimension: u8) {
    for prefix in ["", "i", "u"] {
        let source = query_source(dimension, prefix, &format!("textureSize{dimension}D"));
        let p = program(&source).unwrap_or_else(|e| panic!("{prefix}sampler{dimension}D: {e:?}"));
        validate_and_write_spirv(&p.vertex);
        assert_eq!(query_shapes(&p.fragment), [dimension]);
        assert_eq!(p.interface.samplers.len(), 1);
        assert_eq!(
            p.interface.samplers[0].ty,
            format!("{prefix}sampler{dimension}D")
        );
    }
}

#[test]
fn legacy_size_1d_has_integer_result() {
    legacy_dimension(1);
}

#[test]
fn legacy_size_2d_has_ivec2_result() {
    legacy_dimension(2);
}

#[test]
fn legacy_size_3d_has_ivec3_result() {
    legacy_dimension(3);
}

#[test]
fn modern_size_controls_keep_the_same_results() {
    for dimension in [1, 2, 3] {
        for prefix in ["", "i", "u"] {
            let p = program(&query_source(dimension, prefix, "textureSize")).unwrap();
            assert_eq!(query_shapes(&p.fragment), [dimension]);
        }
    }
}

#[test]
fn legacy_size_in_sampler_parameters_and_parentheses() {
    let p = program(
        "#version 120\n#extension GL_EXT_gpu_shader4 : enable\n\
         uniform sampler2D inputTexture;\n\
         ivec2 query(sampler2D source, int lod) { return textureSize2D((source), lod); }\n\
         void main() { gl_FragColor = vec4(vec2(query(inputTexture, 1)), 0.0, 1.0); }\n",
    )
    .unwrap();
    assert_eq!(query_shapes(&p.fragment), [2]);
}

#[test]
fn user_scalar_functions_keep_their_own_names() {
    let p = program(
        "#version 120\n#extension GL_EXT_gpu_shader4 : enable\n\
         float textureSize1D(float value, int lod) { return value + float(lod); }\n\
         float textureSize2D(float value, int lod) { return value + float(lod); }\n\
         float textureSize3D(float value, int lod) { return value + float(lod); }\n\
         void main() {\n\
           gl_FragColor = vec4(textureSize1D(0.25, 1), textureSize2D(0.5, 2),\n\
                               textureSize3D(0.75, 3), 1.0);\n}\n",
    )
    .unwrap();
    assert!(query_shapes(&p.fragment).is_empty());
    for name in ["textureSize1D", "textureSize2D", "textureSize3D"] {
        assert!(
            p.fragment
                .functions
                .iter()
                .any(|(_, f)| f.name.as_deref() == Some(name))
        );
    }
}

#[test]
fn declared_user_function_keeps_the_builtin_name_hidden() {
    let result = program(
        "#version 120\n#extension GL_EXT_gpu_shader4 : enable\n\
         uniform sampler2D inputTexture;\n\
         float textureSize2D(float value, int lod) { return value + float(lod); }\n\
         void main() { gl_FragColor = vec4(vec2(textureSize2D(inputTexture, 0)), 0.0, 1.0); }\n",
    );
    assert!(
        result.is_err(),
        "a user function hides all builtins with that name"
    );
}

#[test]
fn user_sampler_function_and_prototype_keep_their_own_name() {
    // Without the extension, this is an ordinary pack-defined function.
    let p = program(
        "#version 120\n\
         uniform sampler2D inputTexture;\n\
         ivec2 textureSize2D(sampler2D source, int lod);\n\
         ivec2 textureSize2D(sampler2D source, int lod) { return ivec2(17, 29); }\n\
         void main() { gl_FragColor = vec4(vec2(textureSize2D(inputTexture, 0)), 0.0, 1.0); }\n",
    )
    .unwrap();
    assert!(query_shapes(&p.fragment).is_empty());
    assert!(
        p.fragment
            .functions
            .iter()
            .any(|(_, f)| f.name.as_deref() == Some("textureSize2D"))
    );
}

#[test]
fn function_macros_keep_their_expansions() {
    let p = program(
        "#version 120\n\
         #define textureSize1D(s, lod) 17\n\
         #define textureSize2D(s, lod) ivec2(23, 29)\n\
         #define textureSize3D(s, lod) ivec3(31, 37, 41)\n\
         void main() {\n\
           int a = textureSize1D(unused, 0);\n\
           ivec2 b = textureSize2D(unused, 0);\n\
           ivec3 c = textureSize3D(unused, 0);\n\
           gl_FragColor = vec4(float(a + b.x + c.z));\n}\n",
    )
    .unwrap();
    assert!(query_shapes(&p.fragment).is_empty());
    assert!(p.interface.samplers.is_empty());
}

#[test]
fn legacy_names_do_not_accept_a_different_sampler_dimension() {
    for actual in [1, 2, 3] {
        for alias in [1, 2, 3].into_iter().filter(|alias| *alias != actual) {
            let source = query_source(actual, "", &format!("textureSize{alias}D"));
            assert!(
                program(&source).is_err(),
                "sampler{actual}D is not textureSize{alias}D"
            );
        }
    }
}
