//! `options.txt` exactly as `Options.load`/`Options.save` handle it in 26.3:
//! the processing order, per-option codecs and validation (`OptionInstance`),
//! plain fields, key mappings, sound volumes and model parts. Typed access for
//! the game lives in `settings`.
use crate::{
    java_text::{self, Json},
    key_names,
};
use std::sync::LazyLock;

/// A stored option value.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Bool(bool),
    Int(i32),
    Double(f64),
    Str(String),
    List(Vec<String>),
}

/// How an `OptionInstance` decodes, validates and encodes its value.
#[derive(Clone, Copy, Debug)]
pub enum Spec {
    /// `Codec.BOOL`.
    Bool,
    /// `IntRange(min, max)`: codec `intRange(min, max + 1)`, validated to `max`.
    Int(i32, i32),
    /// `ClampingLazyMaxIntRange(min, ..)` while the game is not running.
    LazyInt(i32),
    /// `IntRange(imin, imax).xmap(i / k, (int)(v * k))` with `doubleRange(lo, hi)`.
    Scaled(i32, i32, f64, f64, f64),
    /// The frame rate limit: `IntRange(1, 26).xmap(i * 10, v / 10)`, codec 10..=260.
    FrameLimit,
    /// Field of view degrees stored as `(fov - 70) / 40`.
    Fov,
    /// `IntRange(-200, 100).xmap(10^(i/100), floor(log10(v) * 100))`.
    WheelSensitivity,
    /// `UnitDouble`.
    Unit,
    /// `UnitDouble.xmap(square, sqrt)` with an explicit `doubleRange(0, 1)` codec.
    UnitSquare,
    /// `UnitDouble.xmap(square, sqrt)` using the value set's own codec.
    UnitSquareImplicit,
    /// String-represented enum.
    Named(&'static [&'static str]),
    /// Enum stored as its id, wrapping out-of-range ids.
    Id(i32),
    /// `CloudStatus` with the boolean alternative.
    Clouds,
    /// Any string (`LazyEnum` while the game is not running).
    AnyStr,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Option(Spec),
    PlainBool,
    PlainInt,
    PlainStr,
    List,
    Tutorial,
    Key,
}

struct Def {
    key: String,
    kind: Kind,
    default: V,
}

const TUTORIAL_STEPS: [&str; 6] = [
    "movement",
    "find_tree",
    "punch_tree",
    "open_inventory",
    "craft_planks",
    "none",
];
const MODEL_PARTS: [&str; 7] = [
    "cape",
    "jacket",
    "left_sleeve",
    "right_sleeve",
    "left_pants_leg",
    "right_pants_leg",
    "hat",
];
const GRAPHICS_BACKEND: &[&str] = &["default", "opengl", "vulkan"];
const GRAPHICS_PRESET: &[&str] = &["fast", "fancy", "fabulous", "custom"];
/// Horizon's render distance cap (vanilla stops at 32): the server protocol's
/// view distance tops out at 127 anyway.
pub const MAX_RENDER_DISTANCE: i32 = 128;
const INACTIVITY: &[&str] = &["minimized", "afk"];
const CLOUDS: &[&str] = &["false", "fast", "true"];
const MAIN_HAND: &[&str] = &["left", "right"];
const PRESENCE: &[&str] = &["none", "limited", "all"];
const MUSIC_TOAST: &[&str] = &["never", "pause", "pause_and_toast"];
const MUSIC_FREQUENCY: &[&str] = &["DEFAULT", "FREQUENT", "CONSTANT"];

fn def(key: &str, kind: Kind, default: V) -> Def {
    Def {
        key: key.to_owned(),
        kind,
        default,
    }
}

/// The processing order of `Options.processOptions`.
static TABLE: LazyLock<Vec<Def>> = LazyLock::new(|| {
    use Kind::{List, Option as O, PlainBool as PB, PlainInt as PI, PlainStr as PS, Tutorial};
    use Spec::{
        AnyStr, Bool, Clouds, Fov, FrameLimit, Id, Int, LazyInt, Named, Scaled, Unit, UnitSquare,
        UnitSquareImplicit, WheelSensitivity,
    };
    let b = |key: &str, d: bool| def(key, O(Bool), V::Bool(d));
    let i = |key: &str, lo, hi, d: i32| def(key, O(Int(lo, hi)), V::Int(d));
    let u = |key: &str, d: f64| def(key, O(Unit), V::Double(d));
    let n =
        |key: &str, names: &'static [&'static str], d: i32| def(key, O(Named(names)), V::Int(d));
    let id = |key: &str, count: i32, d: i32| def(key, O(Id(count)), V::Int(d));
    let pb = |key: &str, d: bool| def(key, PB, V::Bool(d));
    let mut t = vec![
        b("ao", true),
        i("biomeBlendRadius", 0, 7, 2),
        def(
            "chunkSectionFadeInTime",
            O(Scaled(0, 40, 20.0, 0.0, 2.0)),
            V::Double(0.75),
        ),
        b("cutoutLeaves", true),
        b("enableVsync", true),
        def(
            "entityDistanceScaling",
            O(Scaled(2, 20, 4.0, 0.5, 5.0)),
            V::Double(1.0),
        ),
        b("entityShadows", true),
        b("forceUnicodeFont", false),
        b("japaneseGlyphVariants", false),
        def("fov", O(Fov), V::Int(70)),
        def("fovEffectScale", O(UnitSquare), V::Double(1.0)),
        def("darknessEffectScale", O(UnitSquareImplicit), V::Double(1.0)),
        u("glintSpeed", 0.5),
        u("glintStrength", 0.75),
        n("preferredGraphicsBackend", GRAPHICS_BACKEND, 0),
        n("graphicsPreset", GRAPHICS_PRESET, 1),
        id("prioritizeChunkUpdates", 3, 0),
        b("fullscreen", false),
        b("exclusiveFullscreen", false),
        b("macFullscreenMenuVisibility", false),
        u("gamma", 0.5),
        def("guiScale", O(LazyInt(0)), V::Int(0)),
        def("debugGuiScale", O(LazyInt(-1)), V::Int(0)),
        i("maxAnisotropyBit", 1, 3, 2),
        id("textureFiltering", 3, 0),
        def("maxFps", O(FrameLimit), V::Int(120)),
        b("improvedTransparency", false),
        n("inactivityFpsLimit", INACTIVITY, 1),
        i("mipmapLevels", 0, 4, 4),
        id("narrator", 4, 0),
        id("particles", 3, 0),
        b("reducedDebugInfo", false),
        def("renderClouds", O(Clouds), V::Int(2)),
        i("cloudRange", 2, 128, 128),
        i("renderDistance", 2, MAX_RENDER_DISTANCE, 12),
        i("simulationDistance", 5, 32, 12),
        u("screenEffectScale", 1.0),
        def("soundDevice", O(AnyStr), V::Str(String::new())),
        b("vignette", true),
        i("weatherRadius", 3, 10, 10),
        b("autoJump", false),
        b("rotateWithMinecart", false),
        b("operatorItemsTab", false),
        b("autoSuggestions", true),
        b("chatColors", true),
        b("chatLinks", true),
        b("chatLinksPrompt", true),
        b("discrete_mouse_scroll", false),
        b("invertXMouse", false),
        b("invertYMouse", false),
        b("realmsNotifications", true),
        b("showSubtitles", false),
        b("directionalAudio", false),
        b("bobView", true),
        b("toggleCrouch", false),
        b("toggleSprint", false),
        b("toggleAttack", false),
        b("toggleUse", false),
        i("sprintWindow", 0, 10, 7),
        b("darkMojangStudiosBackground", false),
        b("hideLightningFlashes", false),
        b("hideSplashTexts", false),
        u("mouseSensitivity", 0.5),
        u("damageTiltStrength", 1.0),
        b("highContrast", false),
        b("highContrastBlockOutline", false),
        b("narratorHotkey", true),
        def("resourcePacks", List, V::List(Vec::new())),
        def("incompatibleResourcePacks", List, V::List(Vec::new())),
        def("lastServer", PS, V::Str(String::new())),
        def("lang", PS, V::Str("en_us".into())),
        id("chatVisibility", 3, 0),
        u("chatOpacity", 1.0),
        u("chatLineSpacing", 0.0),
        u("textBackgroundOpacity", 0.5),
        b("backgroundForChatOnly", true),
        pb("hideServerAddress", false),
        pb("advancedItemTooltips", false),
        pb("pauseOnLostFocus", true),
        def("overrideWidth", PI, V::Int(0)),
        def("overrideHeight", PI, V::Int(0)),
        u("chatHeightFocused", 1.0),
        def(
            "chatDelay",
            O(Scaled(0, 60, 10.0, 0.0, 6.0)),
            V::Double(0.0),
        ),
        u("chatHeightUnfocused", 0.4375),
        u("chatScale", 1.0),
        u("chatWidth", 1.0),
        def(
            "notificationDisplayTime",
            O(Scaled(5, 100, 10.0, 0.5, 10.0)),
            V::Double(1.0),
        ),
        pb("useNativeTransport", true),
        n("mainHand", MAIN_HAND, 1),
        id("attackIndicator", 3, 1),
        def("tutorialStep", Tutorial, V::Int(0)),
        def("mouseWheelSensitivity", O(WheelSensitivity), V::Double(1.0)),
        b("allowCursorChanges", true),
        b("quitShortcuts", true),
        b("ctrlClickEmulatesRightClick", false),
        def("glDebugVerbosity", PI, V::Int(1)),
        pb("skipMultiplayerWarning", false),
        b("hideMatchedNames", true),
        pb("joinedFirstServer", false),
        pb("syncChunkWrites", false),
        b("showAutosaveIndicator", true),
        b("allowServerListing", true),
        b("inGameNotification", false),
        n("sharePresence", PRESENCE, 2),
        b("onlyShowSecureChat", false),
        b("saveChatDrafts", false),
        u("panoramaScrollSpeed", 1.0),
        b("telemetryOptInExtra", false),
        pb("onboardAccessibility", true),
        i("menuBackgroundBlurriness", 0, 10, 5),
        pb("startedCleanly", true),
        n("musicToast", MUSIC_TOAST, 0),
        n("musicFrequency", MUSIC_FREQUENCY, 0),
    ];
    for (name, key, _) in crate::keybinds::MAPPINGS {
        t.push(def(
            &format!("key_{name}"),
            Kind::Key,
            V::Str((*key).to_owned()),
        ));
    }
    for source in crate::settings::SOUND_SOURCES {
        t.push(u(&format!("soundCategory_{source}"), 1.0));
    }
    for part in MODEL_PARTS {
        t.push(pb(&format!("modelPart_{part}"), true));
    }
    t
});

/// Mth.floor for doubles.
fn floor_int(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}

fn number(json: &Json) -> Option<&str> {
    match json {
        Json::Number(n) => Some(n),
        _ => None,
    }
}
fn codec_int(json: &Json) -> Option<i32> {
    number(json).map(Json::int_value)
}
fn codec_double(json: &Json) -> Option<f64> {
    number(json).map(Json::double_value)
}
fn codec_bool(json: &Json) -> Option<bool> {
    match json {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}
fn codec_string(json: &Json) -> Option<&str> {
    match json {
        Json::String(s) => Some(s),
        _ => None,
    }
}
fn double_range(v: f64, lo: f64, hi: f64) -> Option<f64> {
    // Double.compareTo: -0.0 sorts below 0.0 and NaN above everything.
    (v.total_cmp(&lo).is_ge() && v.total_cmp(&hi).is_le() && !v.is_nan()).then_some(v)
}
fn log_mouse(value: i32) -> f64 {
    horizon_core::jmath::pow(10.0, f64::from(value) / 100.0)
}
fn unit_alt(json: &Json) -> Option<f64> {
    codec_double(json)
        .and_then(|v| double_range(v, 0.0, 1.0))
        .or_else(|| codec_bool(json).map(|b| if b { 1.0 } else { 0.0 }))
}

impl Spec {
    /// `codec.parse(JsonOps.INSTANCE, element)`.
    fn decode(self, json: &Json) -> Option<V> {
        match self {
            Spec::Bool => codec_bool(json).map(V::Bool),
            Spec::Int(lo, hi) => {
                let v = codec_int(json)?;
                (v >= lo && i64::from(v) <= i64::from(hi) + 1).then_some(V::Int(v))
            }
            Spec::LazyInt(lo) => {
                let v = codec_int(json)?;
                (v >= lo).then_some(V::Int(v))
            }
            Spec::Scaled(.., lo, hi) => double_range(codec_double(json)?, lo, hi).map(V::Double),
            Spec::FrameLimit => {
                let v = codec_int(json)?;
                (10..=260).contains(&v).then_some(V::Int(v))
            }
            Spec::Fov => Some(V::Int((codec_double(json)? * 40.0 + 70.0) as i32)),
            Spec::WheelSensitivity => {
                double_range(codec_double(json)?, log_mouse(-200), log_mouse(100)).map(V::Double)
            }
            Spec::Unit => unit_alt(json).map(V::Double),
            Spec::UnitSquare => double_range(codec_double(json)?, 0.0, 1.0).map(V::Double),
            Spec::UnitSquareImplicit => unit_alt(json).map(|v| V::Double(v * v)),
            Spec::Named(names) => {
                let s = codec_string(json)?;
                names.iter().position(|n| *n == s).map(|i| V::Int(i as i32))
            }
            Spec::Id(count) => codec_int(json).map(|v| V::Int(v.rem_euclid(count))),
            Spec::Clouds => {
                if let Some(s) = codec_string(json) {
                    return CLOUDS
                        .iter()
                        .position(|n| *n == s)
                        .map(|i| V::Int(i as i32));
                }
                codec_bool(json).map(|b| V::Int(if b { 2 } else { 0 }))
            }
            Spec::AnyStr => codec_string(json).map(|s| V::Str(s.to_owned())),
        }
    }

    /// `ValueSet.validateValue`; `None` makes `OptionInstance.set` fall back
    /// to the option's initial value.
    fn validate(self, v: &V) -> Option<V> {
        match (self, v) {
            (Spec::Bool, V::Bool(_)) => Some(v.clone()),
            (Spec::Int(lo, hi), V::Int(x)) => (*x >= lo && *x <= hi).then(|| v.clone()),
            (Spec::LazyInt(lo), V::Int(x)) => Some(V::Int((*x).clamp(lo, 2_147_483_646))),
            (Spec::Scaled(imin, imax, k, ..), V::Double(x)) => {
                let i = (x * k) as i32;
                (i >= imin && i <= imax).then(|| V::Double(f64::from(i) / k))
            }
            (Spec::FrameLimit, V::Int(x)) => {
                let i = x / 10;
                (1..=26).contains(&i).then(|| V::Int(i * 10))
            }
            (Spec::Fov, V::Int(x)) => (30..=110).contains(x).then(|| v.clone()),
            (Spec::WheelSensitivity, V::Double(x)) => {
                let i = floor_int(horizon_core::jmath::log10(*x) * 100.0);
                (-200..=100).contains(&i).then(|| V::Double(log_mouse(i)))
            }
            (Spec::Unit, V::Double(x)) => (*x >= 0.0 && *x <= 1.0).then(|| v.clone()),
            (Spec::UnitSquare | Spec::UnitSquareImplicit, V::Double(x)) => {
                let root = x.sqrt();
                (0.0..=1.0)
                    .contains(&root)
                    .then_some(V::Double(root * root))
            }
            (Spec::Named(names), V::Int(x)) => {
                (*x >= 0 && (*x as usize) < names.len()).then(|| v.clone())
            }
            (Spec::Id(count), V::Int(x)) => (*x >= 0 && *x < count).then(|| v.clone()),
            (Spec::Clouds, V::Int(x)) => (0..3).contains(x).then(|| v.clone()),
            (Spec::AnyStr, V::Str(_)) => Some(v.clone()),
            _ => None,
        }
    }

    /// `codec.encodeStart(JsonOps.INSTANCE, value)` rendered by Gson.
    fn encode(self, v: &V) -> Option<String> {
        let double = |x: f64| java_text::double_to_string(x);
        Some(match (self, v) {
            (Spec::Bool, V::Bool(b)) => b.to_string(),
            (Spec::Int(lo, hi), V::Int(x)) => {
                if *x < lo || i64::from(*x) > i64::from(hi) + 1 {
                    return None;
                }
                x.to_string()
            }
            (Spec::LazyInt(lo), V::Int(x)) => {
                if *x < lo {
                    return None;
                }
                x.to_string()
            }
            (Spec::Scaled(.., lo, hi), V::Double(x)) => {
                double_range(*x, lo, hi)?;
                double(*x)
            }
            (Spec::FrameLimit, V::Int(x)) => {
                if !(10..=260).contains(x) {
                    return None;
                }
                x.to_string()
            }
            (Spec::Fov, V::Int(x)) => double((f64::from(*x) - 70.0) / 40.0),
            (Spec::WheelSensitivity, V::Double(x)) => {
                double_range(*x, log_mouse(-200), log_mouse(100))?;
                double(*x)
            }
            (Spec::Unit, V::Double(x)) => {
                double_range(*x, 0.0, 1.0)?;
                double(*x)
            }
            (Spec::UnitSquare, V::Double(x)) => {
                double_range(*x, 0.0, 1.0)?;
                double(*x)
            }
            (Spec::UnitSquareImplicit, V::Double(x)) => {
                let root = x.sqrt();
                double_range(root, 0.0, 1.0)?;
                double(root)
            }
            (Spec::Named(names), V::Int(x)) => {
                java_text::gson_string(names.get(usize::try_from(*x).ok()?)?)
            }
            (Spec::Id(_), V::Int(x)) => x.to_string(),
            (Spec::Clouds, V::Int(x)) => {
                java_text::gson_string(CLOUDS.get(usize::try_from(*x).ok()?)?)
            }
            (Spec::AnyStr, V::Str(s)) => java_text::gson_string(s),
            _ => return None,
        })
    }
}

/// `InputConstants.getKey(name)`: the canonical name of a key, or the
/// unbound key when `name` is invalid.
pub fn canonical_key(name: &str) -> Result<String, java_text::SyntaxError> {
    for (_, n) in key_names::KEYBOARD.iter().chain(key_names::MOUSE) {
        if *n == name {
            return Ok(name.to_owned());
        }
    }
    for (prefix, table) in [
        ("key.keyboard", key_names::KEYBOARD),
        ("key.mouse", key_names::MOUSE),
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            // substring(prefix.length() + 1): the separator is skipped blindly.
            let mut chars = rest.chars();
            if chars.next().is_none() {
                // StringIndexOutOfBoundsException is not an IllegalArgumentException.
                return Err(java_text::SyntaxError);
            }
            let Some(value) = crate::options_fixes::java_parse_int(chars.as_str()) else {
                return Ok("key.keyboard.unknown".into());
            };
            return Ok(table
                .iter()
                .find(|(v, _)| *v == value)
                .map_or_else(|| format!("{prefix}.{value}"), |(_, n)| (*n).to_owned()));
        }
    }
    Ok("key.keyboard.unknown".into())
}

fn read_list(text: &str) -> Result<Vec<String>, java_text::SyntaxError> {
    // GsonHelper.fromNullableJson with a List<String> type token.
    Ok(java_text::parse_strict_string_list(text)?.unwrap_or_default())
}

/// All option values in `processOptions` order.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionsFile {
    values: Vec<V>,
    /// `fullscreenResolution`, kept verbatim.
    pub fullscreen_resolution: Option<String>,
}

impl Default for OptionsFile {
    fn default() -> Self {
        Self {
            values: TABLE.iter().map(|d| d.default.clone()).collect(),
            fullscreen_resolution: None,
        }
    }
}

impl OptionsFile {
    fn index(key: &str) -> Option<usize> {
        TABLE.iter().position(|d| d.key == key)
    }
    pub fn get(&self, key: &str) -> Option<&V> {
        Self::index(key).map(|i| &self.values[i])
    }
    pub fn bool(&self, key: &str) -> bool {
        matches!(self.get(key), Some(V::Bool(true)))
    }
    pub fn int(&self, key: &str) -> i32 {
        match self.get(key) {
            Some(V::Int(i)) => *i,
            _ => 0,
        }
    }
    pub fn double(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(V::Double(d)) => *d,
            _ => 0.0,
        }
    }
    pub fn string(&self, key: &str) -> String {
        match self.get(key) {
            Some(V::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(V::List(l)) => l.clone(),
            _ => Vec::new(),
        }
    }
    /// The serialized name of an enum-valued option.
    pub fn name(&self, key: &str) -> String {
        let Some(i) = Self::index(key) else {
            return String::new();
        };
        match (TABLE[i].kind, &self.values[i]) {
            (Kind::Option(Spec::Named(names)), V::Int(x)) => {
                names.get(*x as usize).copied().unwrap_or("").to_owned()
            }
            (Kind::Option(Spec::Clouds), V::Int(x)) => {
                CLOUDS.get(*x as usize).copied().unwrap_or("").to_owned()
            }
            _ => String::new(),
        }
    }
    /// `OptionInstance.set` with a candidate value (invalid values revert to
    /// the initial value).
    pub fn set(&mut self, key: &str, value: V) {
        let Some(i) = Self::index(key) else { return };
        let Kind::Option(spec) = TABLE[i].kind else {
            self.values[i] = value;
            return;
        };
        self.values[i] = spec
            .validate(&value)
            .unwrap_or_else(|| TABLE[i].default.clone());
    }
    /// Select an enum option by serialized name.
    pub fn set_name(&mut self, key: &str, name: &str) {
        let Some(i) = Self::index(key) else { return };
        let names: &[&str] = match TABLE[i].kind {
            Kind::Option(Spec::Named(names)) => names,
            Kind::Option(Spec::Clouds) => CLOUDS,
            _ => return,
        };
        if let Some(p) = names.iter().position(|n| *n == name) {
            self.values[i] = V::Int(p as i32);
        }
    }
    /// `Options.modelCustomisation`: the PlayerModelPart bits the player shows
    /// (cape first, as `modelPart_*` is in `MODEL_PARTS`).
    pub fn model_customisation(&self) -> u8 {
        MODEL_PARTS
            .iter()
            .enumerate()
            .filter(|(_, part)| self.bool(&format!("modelPart_{part}")))
            .fold(0, |bits, (bit, _)| bits | 1 << bit)
    }
    /// Key mapping value by mapping name.
    pub fn key_binding(&self, name: &str) -> String {
        self.string(&format!("key_{name}"))
    }

    /// `Options.load` on the text of an options file.
    pub fn parse(text: &str) -> Self {
        let mut file = Self::default();
        let mut raw: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for line in split_lines(text) {
            if let Some((k, v)) = line.split_once(':') {
                raw.insert(k.to_owned(), v.to_owned());
            }
        }
        let version = raw
            .get("version")
            .and_then(|v| crate::options_fixes::java_parse_int(v))
            .unwrap_or(0);
        // A fixer failure abandons the whole file, leaving the defaults.
        if crate::options_fixes::update(&mut raw, version).is_ok() {
            file.apply(&raw);
        }
        file
    }

    fn apply(&mut self, raw: &std::collections::HashMap<String, String>) {
        // A syntax error aborts the rest of the load, fullscreenResolution included.
        if self.try_apply(raw).is_ok()
            && let Some(r) = raw.get("fullscreenResolution")
        {
            self.fullscreen_resolution = Some(r.clone());
        }
    }

    fn try_apply(
        &mut self,
        raw: &std::collections::HashMap<String, String>,
    ) -> Result<(), java_text::SyntaxError> {
        for (i, d) in TABLE.iter().enumerate() {
            let Some(text) = raw.get(&d.key) else {
                continue;
            };
            match d.kind {
                Kind::Option(spec) => {
                    let json =
                        java_text::parse_lenient(if text.is_empty() { "\"\"" } else { text })?;
                    if let Some(decoded) = spec.decode(&json) {
                        self.values[i] =
                            spec.validate(&decoded).unwrap_or_else(|| d.default.clone());
                    }
                }
                Kind::PlainBool => self.values[i] = V::Bool(text == "true"),
                Kind::PlainInt => {
                    if let Some(v) = crate::options_fixes::java_parse_int(text) {
                        self.values[i] = V::Int(v);
                    }
                }
                Kind::PlainStr => self.values[i] = V::Str(text.clone()),
                Kind::List => self.values[i] = V::List(read_list(text)?),
                Kind::Tutorial => {
                    let step = TUTORIAL_STEPS.iter().position(|n| n == text).unwrap_or(5);
                    self.values[i] = V::Int(step as i32);
                }
                Kind::Key => {
                    let V::Str(current) = &self.values[i] else {
                        continue;
                    };
                    if current != text {
                        self.values[i] = V::Str(canonical_key(text)?);
                    }
                }
            }
        }
        Ok(())
    }

    /// `Options.save`.
    pub fn serialize(&self) -> String {
        let mut out = format!("version:{}\n", horizon_world::DATA_VERSION);
        for (d, v) in TABLE.iter().zip(&self.values) {
            let rendered = match (d.kind, v) {
                (Kind::Option(spec), v) => match spec.encode(v) {
                    Some(s) => s,
                    None => continue,
                },
                (Kind::PlainBool, V::Bool(b)) => b.to_string(),
                (Kind::PlainInt, V::Int(x)) => x.to_string(),
                (Kind::PlainStr | Kind::Key, V::Str(s)) => s.clone(),
                (Kind::List, V::List(items)) => {
                    let parts: Vec<String> =
                        items.iter().map(|s| java_text::gson_string(s)).collect();
                    format!("[{}]", parts.join(","))
                }
                (Kind::Tutorial, V::Int(x)) => TUTORIAL_STEPS
                    .get(*x as usize)
                    .copied()
                    .unwrap_or("none")
                    .to_owned(),
                _ => continue,
            };
            out.push_str(&d.key);
            out.push(':');
            out.push_str(&rendered);
            out.push('\n');
        }
        if let Some(r) = &self.fullscreen_resolution {
            out.push_str("fullscreenResolution:");
            out.push_str(r);
            out.push('\n');
        }
        out
    }
}

/// `BufferedReader.lines()`: \n, \r\n and \r terminate lines.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Option names in processing order (for tests and tooling).
pub fn keys() -> Vec<&'static str> {
    TABLE.iter().map(|d| d.key.as_str()).collect()
}

/// What an option accepts, for the control socket (`options-get`).
fn accepts(kind: Kind) -> serde_json::Value {
    use serde_json::json;
    match kind {
        Kind::Option(Spec::Bool) | Kind::PlainBool => json!({ "type": "bool" }),
        Kind::Option(Spec::Int(lo, hi)) => json!({ "type": "int", "min": lo, "max": hi }),
        Kind::Option(Spec::LazyInt(lo)) => json!({ "type": "int", "min": lo }),
        Kind::Option(Spec::Scaled(imin, imax, k, ..)) => json!({
            "type": "double",
            "min": f64::from(imin) / k,
            "max": f64::from(imax) / k,
            "step": 1.0 / k,
        }),
        Kind::Option(Spec::FrameLimit) => {
            json!({ "type": "int", "min": 10, "max": 260, "step": 10 })
        }
        Kind::Option(Spec::Fov) => json!({ "type": "int", "min": 30, "max": 110 }),
        Kind::Option(Spec::WheelSensitivity) => {
            json!({ "type": "double", "min": log_mouse(-200), "max": log_mouse(100) })
        }
        Kind::Option(Spec::Unit | Spec::UnitSquare | Spec::UnitSquareImplicit) => {
            json!({ "type": "double", "min": 0.0, "max": 1.0 })
        }
        Kind::Option(Spec::Named(names)) => json!({ "type": "enum", "choices": names }),
        Kind::Option(Spec::Id(count)) => json!({ "type": "int", "min": 0, "max": count - 1 }),
        Kind::Option(Spec::Clouds) => json!({ "type": "enum", "choices": CLOUDS }),
        Kind::Option(Spec::AnyStr) | Kind::PlainStr => json!({ "type": "string" }),
        Kind::PlainInt => json!({ "type": "int" }),
        Kind::List => json!({ "type": "list" }),
        Kind::Tutorial => json!({ "type": "enum", "choices": TUTORIAL_STEPS }),
        Kind::Key => json!({ "type": "key", "example": "key.keyboard.w" }),
    }
}

/// The names of an enum-valued option.
fn choices(kind: Kind) -> Option<&'static [&'static str]> {
    match kind {
        Kind::Option(Spec::Named(names)) => Some(names),
        Kind::Option(Spec::Clouds) => Some(CLOUDS),
        Kind::Tutorial => Some(&TUTORIAL_STEPS),
        _ => None,
    }
}

impl OptionsFile {
    /// An option for the control socket: `{"value", "type", ...}` with what
    /// it accepts (range, choices); enum values by name. None when unknown.
    pub fn describe(&self, key: &str) -> Option<serde_json::Value> {
        let i = Self::index(key)?;
        let kind = TABLE[i].kind;
        let value = match (choices(kind), &self.values[i]) {
            (Some(names), V::Int(x)) => usize::try_from(*x)
                .ok()
                .and_then(|x| names.get(x))
                .map_or(serde_json::Value::Null, |n| (*n).into()),
            (_, V::Bool(b)) => (*b).into(),
            (_, V::Int(x)) => (*x).into(),
            (_, V::Double(d)) => (*d).into(),
            (_, V::Str(s)) => s.as_str().into(),
            (_, V::List(l)) => l.clone().into(),
        };
        let mut out = accepts(kind);
        out["value"] = value;
        Some(out)
    }

    /// Set an option from text, as the control socket's `options-set` does:
    /// enums by name, key bindings by key name (`key.keyboard.w`), lists as
    /// JSON. Unlike loading `options.txt`, a value the option refuses is an
    /// error naming what it accepts, and the option keeps its value.
    pub fn set_text(&mut self, key: &str, text: &str) -> Result<(), String> {
        let i = Self::index(key).ok_or_else(|| format!("unknown option {key}"))?;
        let d = &TABLE[i];
        let refused = || format!("{key} refuses {text:?}; it accepts {}", accepts(d.kind));
        let candidate = if let Some(names) = choices(d.kind) {
            let x = names.iter().position(|n| *n == text).ok_or_else(refused)?;
            V::Int(x as i32)
        } else if matches!(d.kind, Kind::Key) {
            let name = canonical_key(text).map_err(|_| refused())?;
            if name == "key.keyboard.unknown" && text != name {
                return Err(refused());
            }
            V::Str(name)
        } else {
            match &d.default {
                V::Bool(_) => V::Bool(text.parse().map_err(|_| refused())?),
                V::Int(_) => V::Int(text.parse().map_err(|_| refused())?),
                V::Double(_) => V::Double(text.parse().map_err(|_| refused())?),
                V::Str(_) => V::Str(text.to_owned()),
                V::List(_) => V::List(serde_json::from_str(text).map_err(|_| refused())?),
            }
        };
        self.values[i] = match d.kind {
            Kind::Option(spec) => spec.validate(&candidate).ok_or_else(refused)?,
            _ => candidate,
        };
        Ok(())
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;

    #[test]
    fn describes_values_with_what_they_accept() {
        let f = OptionsFile::default();
        let fov = f.describe("fov").unwrap();
        assert_eq!(fov["value"], 70);
        assert_eq!(
            (fov["type"].as_str(), fov["min"].as_i64()),
            (Some("int"), Some(30))
        );
        assert_eq!(f.describe("renderClouds").unwrap()["value"], "true");
        assert_eq!(
            f.describe("key_key.forward").unwrap()["value"],
            "key.keyboard.w"
        );
        assert_eq!(f.describe("resourcePacks").unwrap()["type"], "list");
        assert!(f.describe("nope").is_none());
        for key in keys() {
            assert!(f.describe(key).is_some(), "{key}");
        }
    }

    #[test]
    fn sets_valid_values_and_refuses_others() {
        let mut f = OptionsFile::default();
        f.set_text("fov", "90").unwrap();
        assert_eq!(f.get("fov"), Some(&V::Int(90)));
        let e = f.set_text("fov", "200").unwrap_err();
        assert!(e.contains("\"max\":110"), "{e}");
        assert_eq!(
            f.get("fov"),
            Some(&V::Int(90)),
            "a refused value changes nothing"
        );
        f.set_text("renderClouds", "fast").unwrap();
        assert_eq!(f.name("renderClouds"), "fast");
        assert!(f.set_text("renderClouds", "fancy").is_err());
        f.set_text("maxFps", "125").unwrap();
        assert_eq!(f.get("maxFps"), Some(&V::Int(120)), "IntRange steps of 10");
        f.set_text("key_key.jump", "key.keyboard.j").unwrap();
        assert_eq!(f.key_binding("key.jump"), "key.keyboard.j");
        assert!(f.set_text("key_key.jump", "jump").is_err());
        f.set_text("resourcePacks", r#"["vanilla","file/x.zip"]"#)
            .unwrap();
        assert_eq!(f.list("resourcePacks"), ["vanilla", "file/x.zip"]);
        assert!(f.set_text("gamma", "1.5").is_err());
        assert!(
            f.set_text("nope", "1")
                .unwrap_err()
                .contains("unknown option")
        );
    }
}
