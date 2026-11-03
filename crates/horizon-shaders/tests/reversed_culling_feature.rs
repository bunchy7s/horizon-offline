//! Original feature-negotiation control; no external shader implementation.
use horizon_shaders::{Macros, Properties};

#[test]
fn requested_reversed_culling_selects_the_supported_shader_path() {
    let properties = Properties::parse(
        "iris.features.optional=REVERSED_CULLING\niris.features.required=REVERSED_CULLING\n",
        &Macros::standard(),
    );
    let mut macros = Macros::standard();
    let notes = macros.define_features(&properties);
    assert!(
        notes.is_empty(),
        "supported reversed culling must not be rejected: {notes:?}"
    );
    // This is how an original pack chooses a voxel-compatible path rather than
    // silently falling back despite the renderer supporting reversed culling.
    let selected = Properties::parse(
        "#ifdef IRIS_FEATURE_REVERSED_CULLING\nshadow.culling=reversed\n#else\nshadow.culling=true\n#endif\n",
        &macros,
    );
    assert_eq!(selected.get("shadow.culling"), Some("reversed"));
    // The feature is opt-in: the fallback remains selected without a request.
    let fallback = Properties::parse(
        "#ifdef IRIS_FEATURE_REVERSED_CULLING\nshadow.culling=reversed\n#else\nshadow.culling=true\n#endif\n",
        &Macros::standard(),
    );
    assert_eq!(fallback.get("shadow.culling"), Some("true"));
}
