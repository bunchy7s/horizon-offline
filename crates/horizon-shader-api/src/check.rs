//! The check a pack passes before it runs: in the game when it loads (a pack
//! that fails keeps the last good one drawing), in the launcher before it
//! installs, in the registry's CI, and in `horizon-shader check`.
//!
//! Each stage is built as the renderer builds it, parsed and validated by
//! naga, checked against its signature and the API's rules (no bindings, no
//! entry points, no `horizon_` names), then translated to SPIR-V, Metal and
//! HLSL with its options at their defaults, minimums and maximums.
use crate::manifest::{OptionKind, PackOption};
use crate::package::Pack;
use crate::stages::{Composed, StageKind, compose};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use std::collections::HashSet;
use std::fmt;

/// naga expressions a stage may hold, over all its functions.
pub const MAX_EXPRESSIONS: usize = 20_000;
/// Texture reads (samples, gathers and loads) a stage may write.
pub const MAX_TEXTURE_READS: usize = 64;

/// What is wrong, and where: a file of the pack, with the line and column
/// when known (0 when not).
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.file, self.line, self.column, self.message
            )
        }
    }
}

fn problem(file: &str, message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_owned(),
        line: 0,
        column: 0,
        message: message.into(),
    }
}

/// One stage that passed, ready for the renderer.
#[derive(Debug)]
pub struct CheckedStage {
    pub kind: StageKind,
    pub file: String,
    /// The manifest's name for it, else its file.
    pub label: String,
    pub module: naga::Module,
    pub info: ModuleInfo,
    /// The stage's WGSL without the entry points (the prelude, the shared
    /// files, the file), for a renderer that calls the stage function from
    /// its own shader (the sky).
    pub source: String,
    pub expressions: usize,
    pub texture_reads: usize,
}

impl CheckedStage {
    /// Whether the stage's function, or a function it calls, reads the prelude's `global`
    /// (a resource of the stage, such as `reflection`).
    pub fn reads(&self, global: &str) -> bool {
        let Some((function, _)) = self
            .module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(self.kind.function()))
        else {
            return false;
        };
        self.module
            .global_variables
            .iter()
            .filter(|(_, g)| g.name.as_deref() == Some(global))
            .any(|(handle, _)| !self.info[function][handle].is_empty())
    }
}

#[derive(Debug)]
pub struct Report {
    pub stages: Vec<CheckedStage>,
    /// The manifest's `[sky]`.
    pub sky: Option<crate::manifest::Stage>,
    /// The manifest's `[reflection]`.
    pub reflection: Option<crate::manifest::Reflection>,
    /// The manifest's `[shadow]`.
    pub shadow: Option<crate::manifest::Shadow>,
    /// The manifest's `[material] fog`: whether Horizon fogs the material's colour.
    pub material_fog: bool,
    /// The manifest's `[[textures]]`, and each image decoded, in that order.
    pub textures: Vec<(crate::manifest::PackTexture, Image)>,
    /// The manifest the stages were checked against.
    pub manifest: crate::manifest::Manifest,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits a channel, rows from the top.
    pub rgba: Vec<u8>,
}

/// Decodes the PNG `bytes` into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    let max = crate::manifest::MAX_TEXTURE_SIZE;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(format!(
            "{width}×{height}; a texture is at most {max}×{max}"
        ));
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = (width * height) as usize;
    let rgba = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colour was not expanded".into()),
    };
    if rgba.len() != pixels * 4 {
        return Err("unexpected pixel data".into());
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Checks every stage of `pack`, and that each option is an `override` of
/// its type in some stage. Returns every problem found, not just the first.
pub fn check(pack: &Pack) -> Result<Report, Vec<Problem>> {
    let m = &pack.manifest;
    let stages = m
        .sky
        .iter()
        .map(|s| (StageKind::Sky, s))
        .chain(m.vertex.iter().map(|s| (StageKind::Vertex, s)))
        .chain(m.material.iter().map(|s| (StageKind::Material, s)))
        .chain(m.weather.iter().map(|s| (StageKind::Weather, s)))
        .chain(m.post.iter().map(|s| (StageKind::Post, s)));
    let (mut checked, mut problems) = (Vec::new(), Vec::new());
    for (kind, stage) in stages {
        match check_stage(pack, kind, &stage.file) {
            Ok(mut c) => {
                c.label = stage.name.clone().unwrap_or_else(|| stage.file.clone());
                checked.push(c);
            }
            Err(p) => problems.extend(p),
        }
    }
    if problems.is_empty() {
        for option in &m.options {
            let used = checked.iter().any(|c| {
                c.module
                    .overrides
                    .iter()
                    .any(|(_, o)| o.name.as_deref() == Some(&option.id))
            });
            if !used {
                problems.push(problem(
                    crate::MANIFEST,
                    format!(
                        "option {:?}: no stage declares `override {}: {}`",
                        option.id,
                        option.id,
                        option.kind.wgsl_type()
                    ),
                ));
            }
        }
    }
    let mut textures = Vec::new();
    for t in &m.textures {
        match decode_png(pack.source_bytes(&t.file).unwrap_or_default()) {
            Ok(image) => textures.push((t.clone(), image)),
            Err(e) => problems.push(problem(&t.file, e)),
        }
    }
    if problems.is_empty() {
        Ok(Report {
            sky: pack.manifest.sky.clone(),
            stages: checked,
            textures,
            reflection: m.reflection.clone(),
            shadow: m.shadow.clone(),
            material_fog: m.material.as_ref().and_then(|s| s.fog).unwrap_or(true),
            manifest: m.clone(),
        })
    } else {
        Err(problems)
    }
}

/// Builds and checks one stage of `pack` from its file.
pub fn check_stage(pack: &Pack, kind: StageKind, file: &str) -> Result<CheckedStage, Vec<Problem>> {
    // First without the entry points, so a missing or mistyped stage function
    // is reported as such rather than as a call that doesn't resolve.
    let mut own = compose(pack, kind, file);
    let entries = own.parts.pop().expect("entries are the last part");
    own.text.truncate(entries.1.start);
    // The material prelude's LabPBR functions call into the renderer: a stand-in here.
    if kind == StageKind::Material {
        own.text += crate::stages::PBR_STUB;
    }
    let module = parse(&own)?;
    own.text.truncate(entries.1.start);
    let mut problems = signature(&module, kind, file);
    problems.extend(api_rules(&module, kind, &own));
    problems.extend(options(&module, &pack.manifest.options, file));
    if !problems.is_empty() {
        return Err(problems);
    }
    let composed = compose(pack, kind, file);
    let module = parse(&composed)?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|e| {
            let at = e.spans().next().and_then(|(span, _)| span.to_range());
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(s) = source {
                message += &format!(": {s}");
                source = s.source();
            }
            vec![located(&composed, at.map(|r| r.start), message)]
        })?;
    let (expressions, texture_reads) = cost(&module);
    let mut problems = Vec::new();
    if expressions > MAX_EXPRESSIONS {
        problems.push(problem(
            file,
            format!("{expressions} expressions; a stage may hold {MAX_EXPRESSIONS}"),
        ));
    }
    if texture_reads > MAX_TEXTURE_READS {
        problems.push(problem(
            file,
            format!("{texture_reads} texture reads; a stage may write {MAX_TEXTURE_READS}"),
        ));
    }
    problems.extend(translate(&module, &info, &pack.manifest.options, file));
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(CheckedStage {
        kind,
        file: file.to_owned(),
        label: file.to_owned(),
        module,
        info,
        source: own.text,
        expressions,
        texture_reads,
    })
}

fn located(composed: &Composed, offset: Option<usize>, message: String) -> Problem {
    match offset {
        Some(at) => {
            let (file, line, column) = composed.locate(at);
            Problem {
                file: file.to_owned(),
                line,
                column,
                message,
            }
        }
        None => problem(&composed.parts[composed.parts.len() - 2].0, message),
    }
}

fn parse(composed: &Composed) -> Result<naga::Module, Vec<Problem>> {
    naga::front::wgsl::parse_str(&composed.text).map_err(|e| {
        let label = e.labels().next();
        let mut message = e.message().to_owned();
        if let Some((_, text)) = label.filter(|(_, t)| !t.is_empty() && !message.contains(*t)) {
            message += &format!(" ({text})");
        }
        let at = label.and_then(|(span, _)| span.to_range()).map(|r| r.start);
        vec![located(composed, at, message)]
    })
}

fn type_name(module: &naga::Module, ty: naga::Handle<naga::Type>) -> String {
    let t = &module.types[ty];
    if let Some(name) = &t.name {
        return name.clone();
    }
    match t.inner {
        naga::TypeInner::Scalar(s) => scalar_name(s).to_owned(),
        naga::TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", size as u8, scalar_name(scalar))
        }
        _ => format!("{:?}", t.inner),
    }
}

fn scalar_name(s: naga::Scalar) -> &'static str {
    match (s.kind, s.width) {
        (naga::ScalarKind::Bool, _) => "bool",
        (naga::ScalarKind::Float, 4) => "f32",
        (naga::ScalarKind::Sint, 4) => "i32",
        (naga::ScalarKind::Uint, 4) => "u32",
        _ => "another scalar",
    }
}

fn signature(module: &naga::Module, kind: StageKind, file: &str) -> Vec<Problem> {
    let found = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(kind.function()));
    let Some((_, f)) = found else {
        return vec![problem(file, format!("defines no `{}`", kind.signature()))];
    };
    let args: Vec<String> = f
        .arguments
        .iter()
        .map(|a| type_name(module, a.ty))
        .collect();
    let result = f.result.as_ref().map(|r| type_name(module, r.ty));
    if args == [kind.input().name] && result.as_deref() == Some(kind.output()) {
        return Vec::new();
    }
    vec![problem(
        file,
        format!(
            "`fn {}({}) -> {}` should be `{}`",
            kind.function(),
            args.join(", "),
            result.as_deref().unwrap_or("()"),
            kind.signature()
        ),
    )]
}

/// The names of everything a module defines at its top level.
fn names(module: &naga::Module) -> HashSet<String> {
    let types = module.types.iter().filter_map(|(_, t)| t.name.clone());
    let functions = module.functions.iter().filter_map(|(_, f)| f.name.clone());
    let globals = module
        .global_variables
        .iter()
        .filter_map(|(_, g)| g.name.clone());
    let constants = module.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = module.overrides.iter().filter_map(|(_, o)| o.name.clone());
    types
        .chain(functions)
        .chain(globals)
        .chain(constants)
        .chain(overrides)
        .collect()
}

/// A pack's files declare no bindings, no entry points and no `horizon_`
/// names: what the prelude doesn't define is the pack's.
fn api_rules(module: &naga::Module, kind: StageKind, own: &Composed) -> Vec<Problem> {
    // The prelude as composed: the stage's own, with the pack's `[[textures]]`. The material
    // prelude's LabPBR functions call into the renderer: its stand-in is ours too.
    let mut text = own.text[own.parts[0].1.clone()].to_owned();
    if kind == StageKind::Material {
        text += crate::stages::PBR_STUB;
    }
    let prelude = naga::front::wgsl::parse_str(&text).expect("the prelude parses");
    let ours = names(&prelude);
    let file = &own.parts[own.parts.len() - 1].0;
    let mut problems = Vec::new();
    for e in &module.entry_points {
        problems.push(problem(
            file,
            format!(
                "declares the entry point `{}`: a stage is a function, Horizon adds the entry points",
                e.name
            ),
        ));
    }
    for (_, g) in module.global_variables.iter() {
        let name = g.name.clone().unwrap_or_default();
        if g.binding.is_some() && !ours.contains(&name) {
            problems.push(problem(
                file,
                format!(
                    "declares the binding `{name}`: a stage reads only the resources of its prelude"
                ),
            ));
        }
    }
    let mut theirs: Vec<String> = names(module)
        .into_iter()
        .filter(|n| !ours.contains(n) && n.starts_with("horizon_"))
        .collect();
    theirs.sort();
    for name in theirs {
        problems.push(problem(
            file,
            format!("`{name}`: names starting with horizon_ are Horizon's"),
        ));
    }
    problems
}

/// Each `override` is an option of its type, or has a default of its own.
fn options(module: &naga::Module, options: &[PackOption], file: &str) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (_, o) in module.overrides.iter() {
        let name = o.name.as_deref().unwrap_or("?");
        let ty = type_name(module, o.ty);
        match options.iter().find(|opt| opt.id == name) {
            Some(opt) if ty != opt.kind.wgsl_type() => problems.push(problem(
                file,
                format!(
                    "`override {name}: {ty}`: option {name:?} is a {}, so its override is `{}`",
                    kind_name(&opt.kind),
                    opt.kind.wgsl_type()
                ),
            )),
            None if o.init.is_none() => problems.push(problem(
                file,
                format!("`override {name}` has no default and no [[options]] entry"),
            )),
            _ => {}
        }
    }
    problems
}

fn kind_name(kind: &OptionKind) -> &'static str {
    match kind {
        OptionKind::Bool { .. } => "bool",
        OptionKind::Float { .. } => "float",
        OptionKind::Int { .. } => "int",
        OptionKind::Choice { .. } => "choice",
    }
}

fn cost(module: &naga::Module) -> (usize, usize) {
    let functions = module
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(module.entry_points.iter().map(|e| &e.function));
    let (mut expressions, mut reads) = (0, 0);
    for f in functions {
        expressions += f.expressions.len();
        reads += f
            .expressions
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e,
                    naga::Expression::ImageSample { .. } | naga::Expression::ImageLoad { .. }
                )
            })
            .count();
    }
    (expressions, reads)
}

/// The option values a pack is translated with: its defaults, then every
/// option at its minimum, then at its maximum.
fn option_sets(options: &[PackOption]) -> [(&'static str, Vec<(String, f64)>); 3] {
    let at = |pick: fn(&OptionKind) -> f64| {
        options
            .iter()
            .map(|o| (o.id.clone(), pick(&o.kind)))
            .collect()
    };
    [
        ("defaults", at(OptionKind::default_constant)),
        (
            "minimums",
            at(|k| match *k {
                OptionKind::Bool { .. } | OptionKind::Choice { .. } => 0.0,
                OptionKind::Float { min, .. } => f64::from(min),
                OptionKind::Int { min, .. } => f64::from(min),
            }),
        ),
        (
            "maximums",
            at(|k| match k {
                OptionKind::Bool { .. } => 1.0,
                OptionKind::Choice { values, .. } => (values.len() - 1) as f64,
                OptionKind::Float { max, .. } => f64::from(*max),
                OptionKind::Int { max, .. } => f64::from(*max),
            }),
        ),
    ]
}

/// Translates every entry point for Vulkan (SPIR-V), Metal and DirectX 12
/// (HLSL) with each option set, as wgpu does when it builds the pipelines.
fn translate(
    module: &naga::Module,
    info: &ModuleInfo,
    options: &[PackOption],
    file: &str,
) -> Vec<Problem> {
    let present: HashSet<&str> = module
        .overrides
        .iter()
        .filter_map(|(_, o)| o.name.as_deref())
        .collect();
    let mut problems = Vec::new();
    for (set, values) in option_sets(options) {
        let mut constants = naga::back::PipelineConstants::default();
        for (id, value) in values
            .into_iter()
            .filter(|(id, _)| present.contains(id.as_str()))
        {
            constants.insert(id, value);
        }
        for e in &module.entry_points {
            let fail = |backend: &str, err: String| {
                problem(
                    file,
                    format!("with the options at their {set}, {backend} refuses it: {err}"),
                )
            };
            let processed = naga::back::pipeline_constants::process_overrides(
                module,
                info,
                Some((e.stage, &e.name)),
                &constants,
            );
            let (m, i) = match processed {
                Ok(p) => p,
                Err(err) => {
                    problems.push(fail("the override step", err.to_string()));
                    return problems;
                }
            };
            let spv = naga::back::spv::write_vec(
                &m,
                &i,
                &naga::back::spv::Options::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: e.stage,
                    entry_point: e.name.clone(),
                }),
            );
            let msl = naga::back::msl::write_string(
                &m,
                &i,
                &naga::back::msl::Options {
                    lang_version: (2, 2),
                    ..Default::default()
                },
                &naga::back::msl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                    ..Default::default()
                },
            );
            let mut hlsl = String::new();
            let hlsl_result = naga::back::hlsl::Writer::new(
                &mut hlsl,
                &naga::back::hlsl::Options::default(),
                &naga::back::hlsl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                },
            )
            .write(&m, &i, None);
            let errors = [
                ("Vulkan (SPIR-V)", spv.err().map(|e| e.to_string())),
                ("Metal", msl.err().map(|e| e.to_string())),
                (
                    "DirectX 12 (HLSL)",
                    hlsl_result.err().map(|e| e.to_string()),
                ),
            ];
            for (backend, err) in errors {
                if let Some(err) = err {
                    problems.push(fail(backend, err));
                }
            }
            if !problems.is_empty() {
                return problems;
            }
        }
    }
    problems
}
