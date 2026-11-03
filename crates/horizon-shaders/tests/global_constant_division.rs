//! Original asset-free GLSL acceptance and backend controls. Division by zero has an
//! unspecified value in GLSL 1.20; these tests deliberately do not assert its result.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use horizon_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Post, load_post};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load(fragment: &str) -> Post {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "horizon-global-constant-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let shaders = fixture.0.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(
        shaders.join("gbuffers_terrain.vsh"),
        "#version 120\nvarying vec2 texcoord;\nvoid main() {\n\
         gl_Position = ftransform();\ntexcoord = gl_MultiTexCoord0.xy;\n}\n",
    )
    .unwrap();
    std::fs::write(shaders.join("gbuffers_terrain.fsh"), fragment).unwrap();
    let pack = Pack::open(&fixture.0).unwrap();
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let (properties, notes) = horizon_shaders::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    load_post(
        &pack,
        "",
        &properties,
        &macros,
        &GameInputs {
            terrain: Some(GbuffersInputs::default()),
            ..Default::default()
        },
    )
}

fn assert_terrain_translates(fragment: &str) -> naga::Module {
    let mut post = load(fragment);
    let terrain = post
        .terrain
        .take()
        .unwrap_or_else(|| panic!("terrain was skipped: {:?}", post.notes));
    let program = terrain.program.expect("draw program");
    for module in [&program.vertex, &program.fragment] {
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(module)
        .unwrap();
        let entry = &module.entry_points[0];
        let pipeline = naga::back::spv::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
        };
        let words = naga::back::spv::write_vec(
            module,
            &info,
            &naga::back::spv::Options::default(),
            Some(&pipeline),
        )
        .unwrap();
        assert!(words.len() > 5);
    }
    program.fragment
}

#[test]
fn global_constant_zero_division_loads() {
    assert_terrain_translates(
        "#version 120\n/* DRAWBUFFERS:0 */\nconst float audit = -1.0 / 0.0;\n\
         void main() { gl_FragData[0] = vec4(audit); }\n",
    );
}

#[test]
fn nonconstant_zero_division_control_loads() {
    assert_terrain_translates(
        "#version 120\n/* DRAWBUFFERS:0 */\nfloat audit = -1.0 / 0.0;\n\
         void main() { gl_FragData[0] = vec4(audit); }\n",
    );
}

#[test]
fn finite_constant_division_control_loads() {
    assert_terrain_translates(
        "#version 120\n/* DRAWBUFFERS:0 */\nconst float audit = -1.0 / 2.0;\n\
         void main() { gl_FragData[0] = vec4(audit); }\n",
    );
}

#[test]
fn macro_expanded_zero_division_loads() {
    assert_terrain_translates(
        "#version 120\n#define AUDIT_ZERO 0.0\n\
         const float audit = -1.0 / AUDIT_ZERO;\n\
         void main() { gl_FragData[0] = vec4(audit); }\n",
    );
}

#[test]
fn dependent_constants_and_vector_constructor_load() {
    assert_terrain_translates(
        "#version 120\nconst float audit = -1.0 / 0.0, copy = audit;\n\
         const vec2 pair = vec2(copy, audit);\n\
         void main() { gl_FragData[0] = vec4(pair, copy, audit); }\n",
    );
}

#[test]
fn affected_array_constant_loads() {
    assert_terrain_translates(
        "#version 330\nconst float audit[2] = float[2](-1.0 / 0.0, 0.25);\n\
         const float copy = audit[0];\n\
         void main() { gl_FragData[0] = vec4(copy, audit[1], 0.0, 1.0); }\n",
    );
}

#[test]
fn finite_comma_declarators_remain_array_size_constants() {
    for declaration in [
        "const float count = 2.0, audit = -1.0 / 0.0;",
        "const float audit = -1.0 / 0.0, count = 2.0;",
        "const float first = 1.0, audit = -1.0 / 0.0, count = first + 1.0;",
    ] {
        assert_terrain_translates(&format!(
            "#version 330\n{declaration}\nfloat values[int(count)];\n\
             void main() {{ values[1] = 0.25; gl_FragData[0] = vec4(values[1], audit, 0.0, 1.0); }}\n",
        ));
    }
}

#[test]
fn finite_local_and_global_constant_only_contexts_remain_valid() {
    assert_terrain_translates(
        "#version 330\nconst int choice = 2;\nfloat values[choice];\nconst float audit = -1.0 / 0.0;\n\
         void main() { const int audit = choice; const int count = audit;\n\
         values[1] = float(count); switch (choice) { case choice: gl_FragData[0] = vec4(values[1]); break;\n\
         default: gl_FragData[0] = vec4(1.0); } }\n",
    );
}

#[test]
fn lowered_multidimensional_array_preserves_its_type() {
    for declaration in [
        "float audit[2][1]",
        "float[2][1] audit",
        "float[1] audit[2]",
    ] {
        for denominator in ["2.0", "0.0"] {
            let module = assert_terrain_translates(&format!(
                "#version 430\nconst {declaration} = float[2][1](float[1](-1.0 / {denominator}), float[1](0.25));\n\
                 void main() {{ gl_FragData[0] = vec4(audit[1][0]); }}\n",
            ));
            let mut ty = module
                .global_variables
                .iter()
                .find(|(_, variable)| variable.name.as_deref() == Some("audit"))
                .map(|(_, variable)| variable.ty)
                .or_else(|| {
                    module
                        .constants
                        .iter()
                        .find(|(_, constant)| constant.name.as_deref() == Some("audit"))
                        .map(|(_, constant)| constant.ty)
                })
                .unwrap();
            let mut dimensions = Vec::new();
            while let naga::TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(size),
                ..
            } = module.types[ty].inner
            {
                dimensions.push(size.get());
                ty = base;
            }
            assert_eq!(dimensions, [2, 1]);
        }
    }
}

#[test]
fn local_scalar_shadow_does_not_gain_array_length() {
    let post = load(
        "#version 330\nconst float audit[2] = float[2](-1.0 / 0.0, 0.25);\n\
         void main() { float audit = 1.0; gl_FragData[0] = vec4(audit.length()); }\n",
    );
    assert!(post.terrain.is_none(), "a scalar has no array length");
    assert!(!post.notes.is_empty());
}

#[test]
fn member_name_does_not_create_a_global_dependency() {
    assert_terrain_translates(
        "#version 330\nstruct S { float audit; };\nconst float audit = -1.0 / 0.0;\n\
         const S fixed = S(2.0);\nconst float count = fixed.audit;\nfloat values[int(count)];\n\
         void main() { values[1] = 0.25; gl_FragData[0] = vec4(values[1], audit, 0.0, 1.0); }\n",
    );
}

#[test]
fn array_length_of_lowered_constant_stays_constant() {
    assert_terrain_translates(
        "#version 330\nconst float audit[2] = float[2](-1.0 / 0.0, 0.25);\n\
         const int count = audit.length();\nfloat values[count];\n\
         void main() { values[1] = 0.5; gl_FragData[0] = vec4(values[1], audit[1], 0.0, 1.0); }\n",
    );
}

#[test]
fn named_array_dimension_and_direct_length_preserve_the_type() {
    let module = assert_terrain_translates(
        "#version 330\nconst int count = 2;\n\
         const float audit[count] = float[count](-1.0 / 0.0, 0.25);\n\
         float scratch[audit.length()][1];\n\
         void main() { scratch[1][0] = audit[1]; gl_FragData[0] = vec4(scratch[1][0]); }\n",
    );
    let (_, variable) = module
        .global_variables
        .iter()
        .find(|(_, variable)| variable.name.as_deref() == Some("scratch"))
        .unwrap();
    let mut ty = variable.ty;
    let mut dimensions = Vec::new();
    while let naga::TypeInner::Array {
        base,
        size: naga::ArraySize::Constant(size),
        ..
    } = module.types[ty].inner
    {
        dimensions.push(size.get());
        ty = base;
    }
    assert_eq!(dimensions, [2, 1]);
}

#[test]
fn local_constant_read_of_lowered_global_loads() {
    assert_terrain_translates(
        "#version 120\nconst float audit = -1.0 / 0.0;\n\
         void main() { const float copy = audit; gl_FragData[0] = vec4(copy); }\n",
    );
}

#[test]
fn unrelated_nonconstant_user_call_still_reports_an_error() {
    let post = load(
        "#version 120\nfloat invalid(float x) { return x; }\n\
         const float audit = invalid(1.0);\nvoid main() { gl_FragData[0] = vec4(audit); }\n",
    );
    assert!(
        post.terrain.is_none(),
        "a user call is not a constant expression"
    );
    assert!(!post.notes.is_empty());
}

#[test]
fn dependent_global_initializers_store_in_file_order() {
    let module = assert_terrain_translates(
        "#version 120\nconst float first = -1.0 / 0.0;\n\
         const float second = first, third = second;\n\
         void main() { gl_FragData[0] = vec4(third); }\n",
    );
    fn stores(
        module: &naga::Module,
        f: &naga::Function,
        body: &naga::Block,
        out: &mut Vec<String>,
    ) {
        for statement in body {
            match statement {
                naga::Statement::Store { pointer, .. } => {
                    if let naga::Expression::GlobalVariable(global) = f.expressions[*pointer]
                        && let Some(name) = &module.global_variables[global].name
                        && matches!(name.as_str(), "first" | "second" | "third")
                    {
                        out.push(name.clone());
                    }
                }
                naga::Statement::Call { function, .. } => {
                    let callee = &module.functions[*function];
                    stores(module, callee, &callee.body, out);
                }
                naga::Statement::Block(block) => stores(module, f, block, out),
                naga::Statement::If { accept, reject, .. } => {
                    stores(module, f, accept, out);
                    stores(module, f, reject, out);
                }
                _ => {}
            }
        }
    }
    let entry = &module.entry_points[0].function;
    let mut names = Vec::new();
    stores(&module, entry, &entry.body, &mut names);
    assert_eq!(names, ["first", "second", "third"]);
}

#[test]
fn array_length_uses_declaration_scope() {
    for denominator in ["2.0", "0.0"] {
        let module = assert_terrain_translates(&format!(
            "#version 330\nconst int count = 2;\nconst float audit[count] = float[count](-1.0 / {denominator}, 0.25);\n\
             int query_length() {{ int count = 3; int hz_array_length_audit = 4; int hz_array_length_audit_ = 5; const int n = audit.length(); return n; }}\n\
             void main() {{ gl_FragData[0] = vec4(float(query_length()), audit[1], 0.0, 1.0); }}\n",
        ));
        let (_, function) = module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some("query_length"))
            .unwrap();
        let (local, declaration) = function
            .local_variables
            .iter()
            .find(|(_, local)| local.name.as_deref() == Some("n"))
            .unwrap();
        let mut value = declaration.init.or_else(|| function.body.iter().find_map(|statement| match statement {
            naga::Statement::Store { pointer, value }
                if matches!(function.expressions[*pointer], naga::Expression::LocalVariable(handle) if handle == local) => Some(*value),
            _ => None,
        })).unwrap();
        let mut expressions = &function.expressions;
        loop {
            match expressions[value] {
                naga::Expression::As { expr, .. } => value = expr,
                naga::Expression::Constant(constant) => {
                    value = module.constants[constant].init;
                    expressions = &module.global_expressions;
                }
                _ => break,
            }
        }
        assert!(
            matches!(
                expressions[value],
                naga::Expression::Literal(naga::Literal::I32(2))
            ),
            "array length must stay constant 2 with denominator {denominator}: {:?}",
            expressions[value],
        );
    }
}

#[test]
fn sized_parameter_length_stays_constant_for_a_lowered_global_argument() {
    for parameter in ["values", "audit"] {
        let module = assert_terrain_translates(&format!(
            "#version 330\nconst float audit[3] = float[3](-1.0 / 0.0, 0.25, 0.5);\n\
             int query(const float {parameter}[3]) {{ return {parameter}.length(); }}\n\
             void main() {{ gl_FragData[0] = vec4(float(query(audit)), audit[1], 0.0, 1.0); }}\n",
        ));
        assert!(
            module.global_variables.iter().any(|(_, variable)| {
                variable.name.as_deref() == Some("audit")
                    && variable.space == naga::AddressSpace::Private
            }),
            "the caller must pass an actually lowered global array"
        );
        let (_, function) = module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some("query"))
            .unwrap();
        let mut value = function
            .body
            .iter()
            .find_map(|statement| match statement {
                naga::Statement::Return { value } => *value,
                _ => None,
            })
            .expect("the parameter-length return");
        let mut expressions = &function.expressions;
        loop {
            match expressions[value] {
                naga::Expression::As { expr, .. } => value = expr,
                naga::Expression::Constant(constant) => {
                    value = module.constants[constant].init;
                    expressions = &module.global_expressions;
                }
                _ => break,
            }
        }
        assert!(
            matches!(
                expressions[value],
                naga::Expression::Literal(naga::Literal::I32(3))
            ),
            "sized parameter {parameter} must return constant length 3: {:?}",
            expressions[value]
        );
    }
}
