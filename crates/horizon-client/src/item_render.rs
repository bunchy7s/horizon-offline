//! Item rendering through the resolved item model pipeline: the
//! `ItemModelResolver` of `horizon-assets` fills an [`ItemRenderState`]
//! (`ItemStackRenderState`: layers of baked quads or special renderers, each
//! with its display transform, local transform, tints and foil), and
//! [`ItemRenderState::submit`] turns it into the quads `ItemFeatureRenderer`
//! and the special model renderers submit, in pose space. A sink then draws
//! them: [`gui`] into the HUD batch with the GUI item lighting, [`WorldSink`]
//! into entity vertices with the level's lighting.
pub mod cit;
pub mod gui;
mod special;

use crate::{
    block_entity_render::{ModelPart, diffuse},
    hud::Rect,
    mesher::{CULL_BACK, Vertex},
};
use glam::{Mat3, Mat4, Vec3};
use horizon_assets::{
    animation::AnimationMetadata,
    item_bake::{BakedItemModel, SpriteLookup, SpriteMask, bake_item_model},
    item_model::{
        ClientItem, Components, Context, DisplayContext, FoilType, ItemEnv, ItemModels,
        LayerContent, LevelState, OwnerState, SpecialModel, StackView,
    },
    model::Direction,
    palette::PalettedPermutations,
    resolve::{MISSING_MODEL, ModelLibrary},
};
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// One quad as submitted for drawing: pose-space corners, atlas UVs, the
/// transformed unit normal, ARGB colour (tint, `-1` white) and its glint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemQuad {
    pub pos: [Vec3; 4],
    pub uv: [[f32; 2]; 4],
    /// The model UVs `GLINT_TEXTURING` scrolls for the glint (vanilla's UV0,
    /// or UV3 for a special model).
    pub foil_uv: [[f32; 2]; 4],
    pub normal: Vec3,
    pub color: u32,
    pub foil: Foil,
    /// `MaterialInfo.lightEmission` (block light floor).
    pub light_emission: u8,
    /// The render type culls back faces (`entitySolid`): world drawing discards them.
    pub cull_back: bool,
    /// The quad's sprite has translucent pixels (`MaterialInfo.itemRenderType`
    /// is a translucent sheet): drawn with the translucent entities.
    pub translucent: bool,
    /// The quad's render type has an `OitPipelineSet` (`entityTranslucent`: a
    /// player head's skin): improved transparency draws it in its stages.
    pub oit: bool,
}
/// `TextureTransform.setupGlintTexturing`'s inputs.
#[derive(Clone, Copy, Debug)]
pub struct Glint {
    /// `options.glintSpeed` (vanilla default 0.5).
    pub speed: f32,
    /// `options.glintStrength` / shader `GlintAlpha` (vanilla default 0.75).
    pub alpha: f32,
    /// `Util.getMillis()`.
    pub millis: f32,
    /// The transform's scale: `GLINT_TEXTURING`'s 8.0 for GUI items,
    /// `ENTITY_GLINT_TEXTURING`'s 0.5 for items in the world and
    /// `ARMOR_ENTITY_GLINT_TEXTURING`'s 0.16 for armour (entity.wgsl).
    pub scale: f32,
}
impl Glint {
    /// `Util.getMillis() * glintSpeed * 8.0` as vanilla's long.
    fn ticks(millis: f32, speed: f32) -> i64 {
        (f64::from(millis) * f64::from(speed) * 8.0) as i64
    }
    /// The scroll clock modulo the common period of both offsets (110000 and
    /// 30000 divide 330000), for `globals.time.w`: the entity shader derives
    /// `ARMOR_ENTITY_GLINT_TEXTURING` from it.
    pub fn scroll_ticks(millis: f32, speed: f32) -> f32 {
        (Self::ticks(millis, speed) % 330_000) as f32
    }
    /// `TextureTransform.setupGlintTexturing(scale)` applied to the model UV:
    /// translation(-o0, o1) * rotateZ(PI/18) * scale. The result repeats:
    /// the shader samples the dedicated glint texture with vanilla's
    /// repeating `GlintSampler`, so the UVs stay unwrapped.
    pub fn uv(&self, model: [f32; 2]) -> [f32; 2] {
        let ticks = Self::ticks(self.millis, self.speed);
        let offset0 = (ticks % 110_000) as f32 / 110_000.0;
        let offset1 = (ticks % 30_000) as f32 / 30_000.0;
        let angle = std::f32::consts::PI / 18.0;
        let (sin, cos) = angle.sin_cos();
        [
            model[0] * self.scale * cos - model[1] * self.scale * sin - offset0,
            model[0] * self.scale * sin + model[1] * self.scale * cos + offset1,
        ]
    }
}
/// The glint inputs of one frame, read once: `GameRenderer.extract` snapshots
/// `glintSpeed` and `glintStrength` into `OptionsRenderState`
/// (`extractOptions`), and `TextureTransform.setupGlintTexturing` reads
/// `Util.getMillis()` once per glint render-type batch, not per item.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlintFrame {
    /// `Util.getMillis()` this frame.
    pub millis: f32,
    /// `OptionsRenderState.glintSpeed`.
    pub speed: f32,
    /// `OptionsRenderState.glintStrength`.
    pub alpha: f32,
    /// The atlas has `misc/enchanted_glint_item`.
    pub sprite: bool,
}
impl GlintFrame {
    /// The glint at a texturing scale, if the atlas has its sprite.
    pub fn glint(&self, scale: f32) -> Option<Glint> {
        self.sprite.then_some(Glint {
            speed: self.speed,
            alpha: self.alpha,
            millis: self.millis,
            scale,
        })
    }
    /// [`Glint::scroll_ticks`] for this frame.
    pub fn scroll_ticks(&self) -> f32 {
        Glint::scroll_ticks(self.millis, self.speed)
    }
}
/// Which glint render type a quad uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foil {
    None,
    /// `GLINT_TEXTURING` over the sprite's UVs (`itemGlintRenderType`,
    /// `entitySolidGlint`).
    Standard,
    /// `itemGlintSpecialRenderType`: compasses and clocks, a sheeted decal.
    Special,
}
/// Where submitted quads go.
pub trait ItemSink {
    fn quad(&mut self, quad: ItemQuad);
}
impl ItemSink for Vec<ItemQuad> {
    fn quad(&mut self, quad: ItemQuad) {
        self.push(quad);
    }
}

/// Data special renderers read from the game (registries, skins).
pub trait SpecialHost {
    /// The banner atlas texture of a banner pattern (`entity/banner/<asset>`).
    fn banner_texture(&self, pattern: &str) -> Option<String>;
    /// The shield atlas texture of a banner pattern (`entity/shield/<asset>`).
    fn shield_texture(&self, pattern: &str) -> Option<String>;
    /// The skin texture of a `minecraft:profile` component, if loaded.
    fn player_skin(&self, _profile: &Value) -> Option<String> {
        None
    }
}
/// A host with no registries: patterns and skins fall back to defaults.
pub struct NoHost;
impl SpecialHost for NoHost {
    fn banner_texture(&self, _: &str) -> Option<String> {
        None
    }
    fn shield_texture(&self, _: &str) -> Option<String> {
        None
    }
}

/// The geometry of one layer.
#[derive(Clone, Debug)]
pub enum LayerGeometry {
    Quads(Arc<BakedItemModel>),
    Special {
        model: SpecialModel,
        argument: Value,
    },
}
/// `ItemStackRenderState.LayerRenderState`.
#[derive(Clone, Debug)]
pub struct LayerRender {
    pub geometry: LayerGeometry,
    pub tints: Vec<i32>,
    pub foil: FoilType,
    /// `applyTransform`: the display transform, then the local transform.
    pub matrix: Mat4,
    /// `particleMaterial` (atlas sprite name).
    pub particle: String,
    pub uses_block_light: bool,
    /// Untransformed extents (quad corners or the special model's parts).
    pub extents: Arc<Vec<Vec3>>,
}
/// `ItemStackRenderState`.
#[derive(Clone, Debug)]
pub struct ItemRenderState {
    pub display: DisplayContext,
    pub layers: Vec<LayerRender>,
    pub oversized_in_gui: bool,
    pub animated: bool,
    /// The OptiFine CIT glint layers the stack draws in the GUI (`cit_glint`).
    pub cit_glint: Option<Arc<crate::cit_glint::StackGlint>>,
    /// HDR output's tier of an item that glows (`dynamic_light::Luminance::hdr_tier`).
    pub hdr_tier: u8,
    /// In HDR output, the block light of an item that glows: its block's, as a placed one has
    /// it, so a torch doesn't go dark in the shade while it shines. 0 for the rest.
    pub hdr_light: u8,
}
impl ItemRenderState {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
    /// `usesBlockLight` of the first layer (GUI lighting choice).
    pub fn uses_block_light(&self) -> bool {
        self.layers.first().is_some_and(|l| l.uses_block_light)
    }
    /// `getModelBoundingBox`: the transformed extents of every layer, as
    /// (min, max); a zero box at the origin when there are none.
    pub fn bounding_box(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for layer in &self.layers {
            for p in layer.extents.iter() {
                let q = layer.matrix.transform_point3(*p);
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if lo.x > hi.x {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (lo, hi)
        }
    }
    /// `ItemStackRenderState.submit` under `pose`.
    pub fn submit(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        pose: Mat4,
        sink: &mut dyn ItemSink,
    ) {
        for layer in &self.layers {
            let m = pose * layer.matrix;
            match &layer.geometry {
                LayerGeometry::Quads(baked) => {
                    submit_quads(renderer, baked, layer, self.display, m, sprites, sink);
                }
                LayerGeometry::Special { model, argument } => special::submit(
                    renderer,
                    model,
                    argument,
                    layer.foil != FoilType::None,
                    m,
                    sprites,
                    host,
                    sink,
                ),
            }
        }
    }
    /// [`Self::submit`] into a [`WorldSink`] of `light`, `lights` and `tier`,
    /// posed on the GPU (`gfx::entity_skin`): the same vertices in the same
    /// order. False, with nothing drawn, when a layer cannot take that path: a
    /// special model, a glint, or a quad that glows or is translucent (another
    /// material).
    pub fn submit_skinned(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        pose: Mat4,
        (light, lights, tier): (u32, Option<(Vec3, Vec3)>, u8),
        skin: &mut crate::mobs::skin::Skin,
    ) -> bool {
        use crate::gfx::entity_skin::{SkinJob, SkinPart};
        // The axes' order in a stored vertex (`SkinVertex::axis`).
        const AXES: [Direction; 6] = [
            Direction::East,
            Direction::Up,
            Direction::South,
            Direction::West,
            Direction::Down,
            Direction::North,
        ];
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                return false;
            };
            if layer.foil != FoilType::None {
                return false;
            }
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                return false;
            };
            let translucent = |run: &crate::mobs::skin::ItemRun| {
                let sprite = &baked.quads[run.quad].material.sprite;
                renderer.sprites.translucent(sprite_name(sprite))
            };
            if stored.runs.iter().any(translucent) {
                return false;
            }
        }
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                continue;
            };
            let m = pose * layer.matrix;
            let mut normals = FaceNormals::new(m);
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                continue;
            };
            for run in &stored.runs {
                // `submit_quads`' sprite and tint, once for the run's quads.
                let q = &baked.quads[run.quad];
                let Some(r) = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get("misc/missingno"))
                    .or_else(|| sprites.get("horizon/missingno"))
                else {
                    continue;
                };
                let color = if run.tint_index >= 0 {
                    layer
                        .tints
                        .get(run.tint_index as usize)
                        .copied()
                        .unwrap_or(-1)
                } else {
                    -1
                };
                let colors = AXES.map(|d| shaded(argb(color), normals.face(d), lights));
                let frame = &mut *skin.frame;
                frame.jobs.push(SkinJob {
                    first: run.first,
                    count: run.count,
                    part: frame.parts.len() as u32,
                    dst: frame.vertices,
                    rect: [r.u0, r.v0, r.u1, r.v1],
                    light,
                    overlay: u32::from(tier) << 16,
                    mode: SkinJob::ITEM,
                    ..Default::default()
                });
                frame.parts.push(SkinPart::item(m, colors));
                frame.vertices += run.count;
            }
            let bounds = stored.bounds;
            skin.grow_bounds(m, bounds);
        }
        true
    }
}

/// The atlas name of a sprite identifier (`minecraft:item/apple` -> `item/apple`).
pub fn sprite_name(sprite: &str) -> &str {
    sprite.strip_prefix("minecraft:").unwrap_or(sprite)
}
/// `Direction.getUnitVec3f`.
pub fn direction_vector(d: Direction) -> Vec3 {
    match d {
        Direction::Down => Vec3::NEG_Y,
        Direction::Up => Vec3::Y,
        Direction::North => Vec3::NEG_Z,
        Direction::South => Vec3::Z,
        Direction::West => Vec3::NEG_X,
        Direction::East => Vec3::X,
    }
}
/// `PoseStack.Pose.transformNormal`: the normal matrix, renormalized.
pub fn transform_normal(m: Mat4, n: Vec3) -> Vec3 {
    FaceNormals::new(m).transform(n)
}
/// [`transform_normal`] of one pose for many quads: its normal matrix inverted once, and
/// the normal of each of the six facings worked out the first time a quad uses it.
pub struct FaceNormals {
    normal: Mat3,
    faces: [Option<Vec3>; 6],
}
impl FaceNormals {
    pub fn new(m: Mat4) -> Self {
        Self {
            normal: Mat3::from_mat4(m).inverse().transpose(),
            faces: [None; 6],
        }
    }
    fn transform(&self, n: Vec3) -> Vec3 {
        (self.normal * n).normalize_or_zero()
    }
    /// The pose's normal of a face pointing `d`.
    pub fn face(&mut self, d: Direction) -> Vec3 {
        let i = d as usize;
        if let Some(n) = self.faces[i] {
            return n;
        }
        let n = self.transform(direction_vector(d));
        self.faces[i] = Some(n);
        n
    }
}
/// An `ARGB` int as the `u32` colour of an [`ItemQuad`].
pub fn argb(color: i32) -> u32 {
    color as u32
}

/// `ItemFeatureRenderer.prepareMainSubmit` for one cuboid layer.
fn submit_quads(
    renderer: &ItemRenderer,
    baked: &BakedItemModel,
    layer: &LayerRender,
    display: DisplayContext,
    m: Mat4,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    let foil = match layer.foil {
        FoilType::None => Foil::None,
        FoilType::Standard => Foil::Standard,
        FoilType::Special => Foil::Special,
    };
    let _ = display;
    let mut normals = FaceNormals::new(m);
    // Quads of one layer mostly share a sprite: look its rect and its
    // translucency up once.
    let mut last: Option<(&str, bool)> = None;
    let mut last_rect: Option<(&str, Option<Rect>)> = None;
    for q in &baked.quads {
        let rect = match last_rect {
            Some((sprite, rect)) if sprite == q.material.sprite => rect,
            _ => {
                let rect = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get("misc/missingno"))
                    .or_else(|| sprites.get("horizon/missingno"))
                    .copied();
                last_rect = Some((&q.material.sprite, rect));
                rect
            }
        };
        let Some(r) = rect else {
            continue;
        };
        // `getLayerColorSafe`: an untinted quad, or a tint index past the
        // tints, draws white.
        let color = if q.tint_index >= 0 {
            layer
                .tints
                .get(q.tint_index as usize)
                .copied()
                .unwrap_or(-1)
        } else {
            -1
        };
        let pos = q.positions.map(|p| m.transform_point3(Vec3::from(p)));
        let uv =
            q.uv.map(|[u, v]| [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v]);
        let translucent = match last {
            Some((sprite, translucent)) if sprite == q.material.sprite => translucent,
            _ => {
                let translucent = renderer
                    .sprites
                    .translucent(sprite_name(&q.material.sprite));
                last = Some((&q.material.sprite, translucent));
                translucent
            }
        };
        // Every item render type culls back faces: `Sheets.solidBlockSheet`
        // (`entitySolid`), `cutoutBlockSheet` (`entityCutout`) and
        // `translucentItemSheet` (`entityTranslucentCull`). Drawing both sides
        // z-fights the coplanar pairs a two-sided model already carries (a
        // calibrated sculk sensor's tendrils and amethyst planes, #4856; big
        // dripleaf, #4855; the inside of a held hay block, #4801) and stacks a
        // jelly shell's partial alpha until it looks opaque (#4508).
        let translucent = translucent || q.material.force_translucent;
        sink.quad(ItemQuad {
            pos,
            uv,
            foil_uv: q.uv,
            normal: normals.face(q.facing),
            color: argb(color),
            foil,
            light_emission: q.light_emission,
            cull_back: true,
            translucent,
            oit: false,
        });
    }
}

// ---------------------------------------------------------------------------
// Sprites for baking
// ---------------------------------------------------------------------------

/// The alpha masks of item and block sprites, read from the texture files
/// (every unique animation frame) and the items atlas's palette sprites.
struct FileSprites {
    root: PathBuf,
    permutations: Vec<PalettedPermutations>,
    cache: Mutex<HashMap<String, Option<SpriteMask>>>,
    /// Sprites with a pixel that is neither fully transparent nor opaque
    /// (`Transparency.hasTranslucent`), filled in as sprites load.
    translucent: Mutex<HashMap<String, bool>>,
}
impl SpriteLookup for FileSprites {
    fn mask(&self, sprite: &str) -> Option<SpriteMask> {
        if let Ok(cache) = self.cache.lock()
            && let Some(hit) = cache.get(sprite)
        {
            return hit.clone();
        }
        let mask = self.load(sprite);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(sprite.to_owned(), mask.clone());
        }
        mask
    }
}
fn argb_pixels(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}
impl FileSprites {
    fn translucent(&self, sprite: &str) -> bool {
        let known = |this: &Self| this.translucent.lock().ok()?.get(sprite).copied();
        known(self).unwrap_or_else(|| {
            self.mask(sprite);
            known(self).unwrap_or(false)
        })
    }
    fn png(&self, sprite: &str, directory: &str) -> PathBuf {
        crate::atlas_sources::texture_path(&self.root, sprite, directory)
    }
    fn image(&self, sprite: &str, directory: &str) -> Option<(usize, usize, Vec<u32>)> {
        let (w, h, rgba) = crate::hud::decode(&self.png(sprite, directory)).ok()?;
        Some((w as usize, h as usize, argb_pixels(&rgba)))
    }
    fn load(&self, sprite: &str) -> Option<SpriteMask> {
        let png = self.png(sprite, "textures");
        let (iw, ih, argb) = match self.image(sprite, "textures") {
            Some(image) => image,
            None => {
                let p = self.permutations.iter().find_map(|s| s.find(sprite))?;
                let (w, h, mut pixels) = self.image(p.base, "textures")?;
                let (_, _, key) = self.image(p.palette_key, "textures/palettes")?;
                let (_, _, target) = self.image(p.palette, "textures/palettes")?;
                horizon_assets::palette::apply_palette(&mut pixels, &key, &target)?;
                (w, h, pixels)
            }
        };
        let translucent = argb.iter().any(|p| matches!(p >> 24, 1..=254));
        if let Ok(mut cache) = self.translucent.lock() {
            cache.insert(sprite.to_owned(), translucent);
        }
        let alpha: Vec<u8> = argb.iter().map(|p| (p >> 24) as u8).collect();
        let animation =
            horizon_core::prepared_files::read_to_string(&png.with_extension("png.mcmeta"))
                .ok()
                .and_then(|m| serde_json::from_str::<Value>(&m).ok())
                .and_then(|m| m.get("animation").map(Value::to_string))
                .and_then(|a| AnimationMetadata::from_json(&a).ok());
        let (w, h) = match &animation {
            Some(a) => {
                let [w, h] = a.frame_size(iw as i32, ih as i32);
                (w as usize, h as usize)
            }
            None => (iw, ih),
        };
        if w == 0 || h == 0 {
            return None;
        }
        let columns = iw / w;
        let total = columns * (ih / h);
        let clip = animation.and_then(|a| a.clip(total as u32).ok().flatten());
        let frames: Vec<usize> = match clip {
            Some(clip) => {
                let mut unique = Vec::new();
                for (index, _) in clip.sequence() {
                    if !unique.contains(&(*index as usize)) {
                        unique.push(*index as usize);
                    }
                }
                unique
            }
            None => vec![0],
        };
        let frames = frames
            .into_iter()
            .map(|f| {
                let (fx, fy) = (f % columns * w, f / columns * h);
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| alpha[(fy + y) * iw + fx + x] == 0)
                    .collect()
            })
            .collect();
        Some(SpriteMask {
            width: w,
            height: h,
            frames,
        })
    }
}

// ---------------------------------------------------------------------------
// The renderer
// ---------------------------------------------------------------------------

/// The baked item models and everything resolution needs.
pub struct ItemRenderer {
    pub models: ItemModels,
    library: Arc<ModelLibrary>,
    sprites: FileSprites,
    baked: Mutex<HashMap<String, Arc<BakedItemModel>>>,
    extents: Mutex<HashMap<String, Arc<Vec<Vec3>>>>,
    /// GUI items already drawn: the HUD vertices of a static item at a GUI position
    /// (`gui::draw`), which are the same every frame it stays there.
    pub(crate) gui_drawn: Mutex<HashMap<gui::DrawnKey, Arc<[crate::hud::HudVertex]>>>,
    /// Entity model layers of the special renderers.
    parts: HashMap<String, ModelPart>,
    /// `colormap/grass.png` as ARGB.
    grass: Vec<i32>,
    /// `minecraft:item` registry ids (`Item.getId`).
    item_ids: HashMap<String, i32>,
    /// The enabled packs' OptiFine CIT rules and their swapped bakes; `None`
    /// without rules.
    pub cit: Option<cit::CitModels>,
}

/// Every `items/**/*.json` client item definition, by identifier. Through
/// `json_folder`: from the prepared data file when the root has one.
fn load_definitions(root: &Path) -> BTreeMap<String, ClientItem> {
    let mut out = BTreeMap::new();
    let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
        return out;
    };
    for ns in namespaces.flatten() {
        let Ok(folder) = horizon_world::json_folder::read(&ns.path().join("items")) else {
            continue;
        };
        let namespace = ns.file_name().to_string_lossy().into_owned();
        for file in &folder.files {
            if let Some(item) = file
                .bytes()
                .ok()
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|t| ClientItem::from_json(t).ok())
            {
                out.insert(format!("{namespace}:{}", file.name), item);
            }
        }
    }
    out
}

impl ItemRenderer {
    /// Load item definitions, item prototypes (`items.json`), every model,
    /// the special renderers' entity models and the grass colormap.
    pub fn load(root: &Path) -> crate::Result<Self> {
        Self::load_with_library(
            root,
            crate::model_library::ParsedModels::load(root).into_item_library(),
        )
    }
    /// Reuse the immutable models prepared with the block resources at `root`.
    /// Defer item-only validation failures until after item definitions are loaded.
    pub fn load_with_library(
        root: &Path,
        library: std::result::Result<Arc<ModelLibrary>, String>,
    ) -> crate::Result<Self> {
        Self::load_tables(root, library, true)
    }
    /// What a dedicated server reads: the item prototypes (the stacks'
    /// default components) and ids, without the client item definitions,
    /// the special renderers' entity models and the grass colormap, which
    /// only drawing reads (1.4 MB and ~1,500 files).
    pub fn load_for_server(
        root: &Path,
        library: std::result::Result<Arc<ModelLibrary>, String>,
    ) -> crate::Result<Self> {
        Self::load_tables(root, library, false)
    }
    fn load_tables(
        root: &Path,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        drawn: bool,
    ) -> crate::Result<Self> {
        let prototypes: BTreeMap<String, Components> = horizon_world::item_defaults::read(root)?
            .into_iter()
            .map(|(item, components)| Ok((item, serde_json::from_value(components)?)))
            .collect::<serde_json::Result<_>>()?;
        let models = ItemModels {
            definitions: if drawn {
                load_definitions(root)
            } else {
                BTreeMap::new()
            },
            prototypes,
        };
        let (library, cit) = cit::load(root, library?);
        let permutations = PalettedPermutations::from_atlas(
            &fs::read_to_string(root.join("assets/minecraft/atlases/items.json"))
                .unwrap_or_default(),
        );
        let parts = if drawn {
            special::load_parts(root)
        } else {
            HashMap::new()
        };
        let grass = if drawn {
            crate::hud::decode(&root.join("assets/minecraft/textures/colormap/grass.png"))
                .map(|(_, _, rgba)| argb_pixels(&rgba).into_iter().map(|p| p as i32).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let item_ids = horizon_protocol::registry::Builtin::find(root)
            .and_then(|p| horizon_protocol::registry::Builtin::load(&p).ok())
            .and_then(|b| {
                let ids = b.get("minecraft:item")?;
                let mut out = HashMap::new();
                for i in 0.. {
                    let Some(name) = ids.name(i) else { break };
                    out.insert(name.to_owned(), i);
                }
                Some(out)
            })
            .unwrap_or_default();
        Ok(Self {
            item_ids,
            models,
            library,
            sprites: FileSprites {
                root: root.to_path_buf(),
                permutations,
                cache: Mutex::default(),
                translucent: Mutex::default(),
            },
            baked: Mutex::default(),
            extents: Mutex::default(),
            gui_drawn: Mutex::default(),
            parts,
            grass,
            cit,
        })
    }
    /// `ModelBakery` item baking, cached per model.
    pub fn bake(&self, model: &str) -> Arc<BakedItemModel> {
        if let Ok(cache) = self.baked.lock()
            && let Some(hit) = cache.get(model)
        {
            return hit.clone();
        }
        let baked = bake_item_model(&self.library, &self.sprites, model)
            .or_else(|_| bake_item_model(&self.library, &self.sprites, MISSING_MODEL))
            .map(Arc::new)
            .unwrap_or_else(|_| {
                Arc::new(BakedItemModel {
                    quads: Vec::new(),
                    uses_block_light: false,
                    particle: horizon_assets::model::Material {
                        sprite: horizon_assets::item_bake::MISSING_SPRITE.to_owned(),
                        force_translucent: false,
                    },
                    transforms: Default::default(),
                })
            });
        if let Ok(mut cache) = self.baked.lock() {
            cache.insert(model.to_owned(), baked.clone());
        }
        baked
    }
    /// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
    fn quad_extents(&self, model: &str, baked: &BakedItemModel) -> Arc<Vec<Vec3>> {
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(model)
        {
            return hit.clone();
        }
        let out = Arc::new(corners(baked));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(model.to_owned(), out.clone());
        }
        out
    }
    fn special_extents(&self, model: &SpecialModel) -> Arc<Vec<Vec3>> {
        let key = model.to_json().to_string();
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(&key)
        {
            return hit.clone();
        }
        let out = Arc::new(special::extents(self, model));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(key, out.clone());
        }
        out
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemEnv,
    ) -> ItemRenderState {
        self.resolve_with_rule(stack, display, owner, level, seed, env, None)
    }
    /// [`Self::resolve`] for a stack OptiFine CIT item rule `rule` (an index
    /// into `cit.rules.rules`) matched: its model layers draw what the rule
    /// swaps them for.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_with_rule(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemEnv,
        rule: Option<usize>,
    ) -> ItemRenderState {
        let ctx = Context {
            display,
            owner,
            level,
            seed,
        };
        let resolved = self.models.resolve(stack, &ctx, env);
        let mut animated = false;
        let layers = resolved
            .layers
            .into_iter()
            .map(|layer| {
                let (geometry, baked, extents) = match layer.content {
                    LayerContent::Model { model } => {
                        let (baked, extents) = rule
                            .and_then(|rule| self.cit_layer(rule, &model))
                            .unwrap_or_else(|| {
                                let baked = self.bake(&model);
                                let extents = self.quad_extents(&model, &baked);
                                (baked, extents)
                            });
                        (LayerGeometry::Quads(baked.clone()), baked, extents)
                    }
                    LayerContent::Missing => {
                        let baked = self.bake(MISSING_MODEL);
                        let extents = self.quad_extents(MISSING_MODEL, &baked);
                        (LayerGeometry::Quads(baked.clone()), baked, extents)
                    }
                    LayerContent::Special {
                        base,
                        model,
                        argument,
                    } => {
                        let baked = self.bake(&base);
                        let extents = self.special_extents(&model);
                        (LayerGeometry::Special { model, argument }, baked, extents)
                    }
                };
                if layer.foil != FoilType::None {
                    animated = true;
                }
                LayerRender {
                    geometry,
                    tints: layer.tints,
                    foil: layer.foil,
                    matrix: glam_matrix(&baked.layer_matrix(display, &layer.transform)),
                    uses_block_light: baked.uses_block_light,
                    particle: sprite_name(&baked.particle.sprite).to_owned(),
                    extents,
                }
            })
            .collect();
        ItemRenderState {
            display,
            layers,
            oversized_in_gui: resolved.oversized_in_gui,
            animated,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
        }
    }
    /// `GrassColor.get` over the loaded colormap.
    pub fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        let rain = downfall * temperature;
        let x = ((1.0 - temperature) * 255.0) as i64;
        let y = ((1.0 - rain) * 255.0) as i64;
        let index = (y << 8 | x) as usize;
        self.grass.get(index).copied().unwrap_or(-65281)
    }
    /// Submit a special model renderer directly (worn skulls of
    /// `CustomHeadLayer`, which draw the skull model, not the item).
    #[allow(clippy::too_many_arguments)]
    pub fn submit_special(
        &self,
        model: &SpecialModel,
        argument: &Value,
        has_foil: bool,
        pose: Mat4,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        sink: &mut dyn ItemSink,
    ) {
        special::submit(self, model, argument, has_foil, pose, sprites, host, sink);
    }
    /// `Item.getId` (0 when the registry report is missing).
    pub fn item_id(&self, item: &str) -> i32 {
        self.item_ids.get(item).copied().unwrap_or(0)
    }
    pub(crate) fn part(&self, key: &str) -> Option<&ModelPart> {
        self.parts.get(key)
    }
}
/// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
fn corners(baked: &BakedItemModel) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    for q in &baked.quads {
        for p in q.positions {
            let p = Vec3::from(p);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}
/// A `horizon-assets` (row-major, JOML-exact) matrix as glam's.
pub fn glam_matrix(m: &horizon_assets::math::Mat4) -> Mat4 {
    Mat4::from_cols_array_2d(&m.rows()).transpose()
}

// ---------------------------------------------------------------------------
// Stacks
// ---------------------------------------------------------------------------

/// An NBT value as the JSON its component codec reads.
pub fn tag_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => serde_json::Number::from_f64(f64::from(f32::from_bits(*bits)))
            .map_or(Value::Null, Value::Number),
        Tag::Double(bits) => {
            serde_json::Number::from_f64(f64::from_bits(*bits)).map_or(Value::Null, Value::Number)
        }
        Tag::ByteArray(a) => Value::Array(a.iter().map(|b| Value::from(*b as i8)).collect()),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(l) => Value::Array(l.iter().map(tag_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
                .collect(),
        ),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
    }
}
/// The resolver's view of a client stack: its item, count and component patch.
pub fn stack_view(stack: &crate::items::ItemStack) -> StackView {
    let mut view = StackView::new(&stack.item, stack.count as i32);
    view.bundle_selected = stack.data().map_or(-1, |d| d.bundle_selected.0);
    if let Some(Tag::Compound(c)) = crate::item_data::components_tag(stack.damage, stack.data()) {
        for (k, v) in &c {
            let key = k.to_string_lossy();
            match key.strip_prefix('!') {
                Some(removed) => view.patch.insert(removed.to_owned(), None),
                None => view.patch.insert(key, Some(tag_json(v))),
            };
        }
    }
    view
}

// ---------------------------------------------------------------------------
// World drawing
// ---------------------------------------------------------------------------

/// Entity vertices for submitted quads: the `item`/`entity` shader's
/// `minecraft_mix_light` with the level's diffuse lights on the vertex
/// colour, the packed lightmap coordinates (raised by light emission) and
/// overlay 7 for the glint.
pub struct WorldSink<'a> {
    pub out: &'a mut Vec<Vertex>,
    /// Packed light (`block_entity_render::vertex_light`).
    pub light: u32,
    /// The level's diffuse lights; `None` leaves the colour unshaded.
    pub lights: Option<(Vec3, Vec3)>,
    /// The `GLINT_TEXTURING` sample for foil quads (`None` leaves them unglinted).
    pub glint: Option<Glint>,
    /// HDR output's tier (`ItemRenderState::hdr_tier`): rides in the layer's high bits.
    pub tier: u8,
}
/// The vertex colour [`WorldSink`] gives a quad of `color` (ARGB) facing `normal`:
/// shaded by `lights` (`None`: unshaded), as ABGR bytes.
fn shaded(color: u32, normal: Vec3, lights: Option<(Vec3, Vec3)>) -> u32 {
    let brightness = lights.map_or(1.0, |l| diffuse(normal, l));
    let [a, r, g, b] = color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * brightness) as u32;
    u32::from(a) << 24 | c(b) << 16 | c(g) << 8 | c(r)
}
impl ItemSink for WorldSink<'_> {
    fn quad(&mut self, q: ItemQuad) {
        let color = shaded(q.color, q.normal, self.lights);
        // `LightCoordsUtil.lightCoordsWithEmission`: block light at least the emission.
        let emission = u32::from(q.light_emission) * 16;
        let light = if (self.light >> 8 & 0xFF) < emission {
            (self.light & 0xFF) | emission << 8
        } else {
            self.light
        };
        let glint = if q.foil == Foil::None {
            None
        } else {
            self.glint
        };
        let overlay = match (glint.is_some(), q.translucent) {
            (false, true) => crate::gfx::TRANSLUCENT_ITEM_OVERLAY,
            (false, false) => 0,
            (true, true) => crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY,
            (true, false) => 7,
        };
        // Only plain and foil quads: the translucent sheet's overlays are their own material.
        let oit = if q.oit && matches!(overlay, 0 | 7) {
            crate::gfx::OIT_ENTITY
        } else {
            0
        };
        for (i, (p, uv)) in q.pos.iter().zip(q.uv).enumerate() {
            let glint_uv = glint.map_or([0.0, 0.0], |glint| glint.uv(q.foil_uv[i]));
            self.out.push(Vertex {
                normal: if q.cull_back { CULL_BACK } else { [0; 4] },
                pos: p.to_array(),
                uv,
                layer: overlay | oit | u32::from(self.tier) << 16,
                color,
                light,
                glint_uv,
            });
        }
    }
}

/// `ItemEntity.getSpin`.
pub fn item_spin(age_in_ticks: f32, bob_offset: f32) -> f32 {
    age_in_ticks / 20.0 + bob_offset
}
/// `ItemClusterRenderState.getRenderedAmount`.
pub fn rendered_amount(count: u32) -> u32 {
    match count {
        0..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}
/// `ItemEntityRenderer.submitMultipleFromCount`: the poses of each copy of
/// a stack (`seed` is `ItemClusterRenderState.getSeedForItemStack`).
pub fn cluster_poses(state: &ItemRenderState, pose: Mat4, amount: u32, seed: i64) -> Vec<Mat4> {
    use horizon_core::random::{LegacyRandom, RandomSource};
    let mut out = Vec::new();
    if amount == 0 {
        return out;
    }
    let mut random = LegacyRandom::new(seed);
    let (lo, hi) = state.bounding_box();
    let depth = hi.z - lo.z;
    let mut next = || random.next_float() * 2.0 - 1.0;
    if depth > 0.0625 {
        out.push(pose);
        for _ in 1..amount {
            let (x, y, z) = (next() * 0.15, next() * 0.15, next() * 0.15);
            out.push(pose * Mat4::from_translation(Vec3::new(x, y, z)));
        }
    } else {
        let offset = depth * 1.5;
        let mut p = pose
            * Mat4::from_translation(Vec3::new(0.0, 0.0, -(offset * (amount - 1) as f32 / 2.0)));
        out.push(p);
        p *= Mat4::from_translation(Vec3::new(0.0, 0.0, offset));
        for _ in 1..amount {
            let (x, y) = (next() * 0.15 * 0.5, next() * 0.15 * 0.5);
            out.push(p * Mat4::from_translation(Vec3::new(x, y, 0.0)));
            p *= Mat4::from_translation(Vec3::new(0.0, 0.0, offset));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame's glint is read once and shared: every item drawn from the
    /// same `GlintFrame` scrolls by the same phase, and without the atlas
    /// sprite no item gets a glint (`TextureTransform.setupGlintTexturing`
    /// once per batch, `GameRenderer.extractOptions` once per frame).
    #[test]
    fn a_frame_glint_is_shared_by_every_item() {
        let frame = GlintFrame {
            millis: 123_456.0,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        };
        let world = frame.glint(0.5).expect("the atlas has the glint sprite");
        assert_eq!(
            (world.speed, world.alpha, world.millis),
            (0.5, 0.75, 123_456.0)
        );
        assert_eq!(world.scale, 0.5);
        let uvs: Vec<_> = (0..1000)
            .map(|_| frame.glint(0.5).unwrap().uv([0.25, 0.75]))
            .collect();
        assert!(uvs.iter().all(|uv| *uv == uvs[0]));
        assert_eq!(frame.scroll_ticks(), Glint::scroll_ticks(123_456.0, 0.5));
        // 123456 ms * 0.5 * 8 = 493824 ticks, modulo 330000.
        assert_eq!(frame.scroll_ticks(), 163_824.0);
        let missing = GlintFrame {
            sprite: false,
            ..frame
        };
        assert!(missing.glint(0.5).is_none());
        assert_eq!(missing.scroll_ticks(), frame.scroll_ticks());
    }

    /// A bad item model surfaces only once items.json has loaded, so a broken
    /// items.json still reports itself first.
    #[test]
    fn item_model_errors_wait_for_the_item_definitions() -> crate::Result<()> {
        let root =
            std::env::temp_dir().join(format!("horizon-item-model-errors-{}", std::process::id()));
        let model = root.join("assets/horizon/models/INVALID.json");
        fs::create_dir_all(model.parent().unwrap())?;
        fs::write(model, "{}")?;
        let library = crate::model_library::ParsedModels::load(&root).into_item_library();
        assert!(library.is_err());
        fs::write(root.join("items.json"), "{")?;
        let load = || {
            ItemRenderer::load_with_library(&root, library.clone())
                .err()
                .unwrap()
                .to_string()
        };
        let broken_items = load();
        fs::write(root.join("items.json"), "{}")?;
        let bad_model = load();
        let _ = fs::remove_dir_all(&root);
        assert!(!broken_items.starts_with("item models:"));
        assert!(bad_model.starts_with("item models:"));
        Ok(())
    }

    /// Definitions live at any depth (`items/<dir>/<name>.json`), as
    /// ModelEngine's `modelengine:postman/head` does.
    #[test]
    fn item_definitions_load_from_nested_folders() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "horizon-item-definitions-nested-{}",
            std::process::id()
        ));
        let definitions = root.join("assets/horizon/items");
        fs::create_dir_all(definitions.join("postman"))?;
        let empty = r#"{"model":{"type":"minecraft:empty"}}"#;
        fs::write(definitions.join("top.json"), empty)?;
        fs::write(definitions.join("postman/head.json"), empty)?;
        let loaded = load_definitions(&root);
        let _ = fs::remove_dir_all(&root);
        assert!(loaded.contains_key("horizon:top"));
        assert!(loaded.contains_key("horizon:postman/head"));
        Ok(())
    }

    /// A dedicated server keeps the stacks' default components but reads
    /// none of the client item definitions.
    #[test]
    fn a_server_keeps_prototypes_without_definitions() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!("horizon-item-server-{}", std::process::id()));
        let definitions = root.join("assets/horizon/items");
        fs::create_dir_all(&definitions)?;
        fs::write(
            definitions.join("top.json"),
            r#"{"model":{"type":"minecraft:empty"}}"#,
        )?;
        fs::write(
            root.join("items.json"),
            r#"{"horizon:top":{"minecraft:max_stack_size":16}}"#,
        )?;
        let library = crate::model_library::ParsedModels::load(&root).into_item_library();
        let client = ItemRenderer::load_with_library(&root, library.clone())?;
        let server = ItemRenderer::load_for_server(&root, library)?;
        let _ = fs::remove_dir_all(&root);
        assert!(client.models.definitions.contains_key("horizon:top"));
        assert!(server.models.definitions.is_empty());
        assert_eq!(server.models.prototypes, client.models.prototypes);
        Ok(())
    }

    #[test]
    fn rendered_amounts_follow_the_count_bands() {
        let bands: Vec<u32> = [0, 1, 2, 16, 17, 32, 33, 48, 49, 64]
            .iter()
            .map(|c| rendered_amount(*c))
            .collect();
        assert_eq!(bands, [1, 1, 2, 2, 3, 3, 4, 4, 5, 5]);
    }
    #[test]
    fn nbt_components_read_as_json() {
        let tag = Tag::compound([
            ("a", Tag::Int(3)),
            ("b", Tag::List(vec![Tag::String("x".into())])),
            ("c", Tag::Float(1.5f32.to_bits())),
        ]);
        assert_eq!(
            tag_json(&tag),
            serde_json::json!({"a": 3, "b": ["x"], "c": 1.5})
        );
    }
    #[test]
    fn palette_trns_partial_alpha_counts_as_translucent() {
        // Slime/honey (and ice) are indexed PNGs with a tRNS chunk. Decode must
        // keep that fractional alpha so FileSprites marks them translucent (#4508).
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x03, 0x00, 0x00,
            0x00, 0x45, 0x68, 0xfd, 0x16, 0x00, 0x00, 0x00, 0x06, 0x50, 0x4c, 0x54, 0x45, 0x00,
            0xff, 0x00, 0x00, 0x80, 0x00, 0x54, 0x80, 0xe6, 0x08, 0x00, 0x00, 0x00, 0x01, 0x74,
            0x52, 0x4e, 0x53, 0xb4, 0x8c, 0xea, 0xaf, 0xf3, 0x00, 0x00, 0x00, 0x0e, 0x49, 0x44,
            0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x64, 0x60, 0x60, 0x04, 0x00, 0x00, 0x0b,
            0x00, 0x03, 0x52, 0x29, 0x78, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
            0xae, 0x42, 0x60, 0x82,
        ];
        let dir = std::env::temp_dir().join("horizon-trns-alpha");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("indexed.png");
        std::fs::write(&path, png).unwrap();
        let (_w, _h, rgba) = crate::hud::decode(&path).unwrap();
        let partial = rgba.chunks(4).filter(|p| matches!(p[3], 1..=254)).count();
        assert!(partial > 0, "tRNS alpha was stripped: {rgba:?}");
    }

    #[test]
    fn normals_follow_the_inverse_transpose() {
        let m = Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0));
        assert_eq!(transform_normal(m, Vec3::X), Vec3::X);
        let r = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((transform_normal(r, Vec3::Z) - Vec3::X).length() < 1e-6);
    }

    /// Every vanilla item on the ground, held in either hand, framed and worn on the
    /// head, turned, shaded and unshaded: the GPU path's vertices (its jobs expanded as
    /// `entity_skin.wgsl` does) are the CPU path's, to the bit and in the same order, for
    /// each state it takes. Needs the prepared resources.
    #[test]
    fn items_posed_on_the_gpu_draw_what_the_cpu_draws() {
        use crate::gfx::entity_skin::{SkinFrame, expand};
        use crate::mobs::skin::{Skin, SkinModels};
        use horizon_assets::item_model::{NoEnv, StackView};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped");
            return;
        }
        let r = ItemRenderer::load(&root).unwrap();
        let resolve = |id: &str| {
            let view = StackView::new(id, 1);
            r.resolve(&view, DisplayContext::Ground, None, None, 7, &NoEnv)
        };
        // Each display context's transforms (`ItemTransform`s) pose the same quads.
        let contexts = [
            DisplayContext::Ground,
            DisplayContext::ThirdPersonRightHand,
            DisplayContext::ThirdPersonLeftHand,
            DisplayContext::Fixed,
            DisplayContext::Head,
        ];
        let states: Vec<_> = r
            .models
            .definitions
            .keys()
            .flat_map(|id| {
                contexts.map(|context| {
                    let view = StackView::new(id, 1);
                    (id, r.resolve(&view, context, None, None, 7, &NoEnv))
                })
            })
            .collect();
        // Each sprite its own rect, so a run drawn with another's shows.
        let mut sprites: HashMap<String, Rect> = HashMap::new();
        for (_, state) in &states {
            for layer in &state.layers {
                if let LayerGeometry::Quads(baked) = &layer.geometry {
                    for q in &baked.quads {
                        let n = sprites.len() as f32;
                        sprites
                            .entry(sprite_name(&q.material.sprite).to_owned())
                            .or_insert(Rect {
                                u0: n / 4096.0,
                                v0: 0.5 - n / 8192.0,
                                u1: n / 4096.0 + 0.003,
                                v1: 0.5 - n / 8192.0 + 0.007,
                                w: 16,
                                h: 16,
                            });
                    }
                }
            }
        }
        let pose = Mat4::from_translation(Vec3::new(-2.3, 0.7, 5.1))
            * Mat4::from_rotation_y(2.1)
            * Mat4::from_scale(Vec3::splat(0.5));
        let mut models = SkinModels::default();
        let mut frame = SkinFrame::default();
        let (mut taken, mut left) = (0, 0);
        for lights in [
            Some(crate::block_entity_render::diffuse_lights(false)),
            None,
        ] {
            for (id, state) in &states {
                let mut cpu = Vec::new();
                let mut sink = WorldSink {
                    out: &mut cpu,
                    light: 0x50_a0,
                    lights,
                    glint: None,
                    tier: 1,
                };
                state.submit(&r, &sprites, &NoHost, pose, &mut sink);
                frame.clear_draws();
                let mut skin = Skin {
                    models: &mut models,
                    frame: &mut frame,
                    bounds: None,
                };
                if !state.submit_skinned(&r, &sprites, pose, (0x50_a0, lights, 1), &mut skin) {
                    assert!(frame.jobs.is_empty(), "{id}: left to the CPU after drawing");
                    left += 1;
                    continue;
                }
                taken += 1;
                let bounds = skin.bounds;
                let gpu = expand(&frame.new_model, &frame);
                assert_eq!(gpu.len(), cpu.len(), "{id}");
                for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                    assert_eq!(
                        bytemuck::bytes_of(g),
                        bytemuck::bytes_of(c),
                        "{id} vertex {i}: {g:?} != {c:?}"
                    );
                }
                if let (Some((lo, hi)), false) = (bounds, cpu.is_empty()) {
                    for v in &cpu {
                        let p = Vec3::from_array(v.pos);
                        assert!(
                            p.cmpge(lo - 1e-4).all() && p.cmple(hi + 1e-4).all(),
                            "{id}: {p} outside"
                        );
                    }
                }
            }
        }
        // The 8 kinds of the `stress_items` scene all take it, and resolved again (as
        // every frame does) they draw the models already stored.
        let stored = frame.new_model.len();
        for id in [
            "stone",
            "oak_log",
            "diamond",
            "apple",
            "iron_sword",
            "bread",
            "redstone",
            "torch",
        ] {
            frame.clear_draws();
            let mut skin = Skin {
                models: &mut models,
                frame: &mut frame,
                bounds: None,
            };
            let state = resolve(&format!("minecraft:{id}"));
            assert!(
                state.submit_skinned(&r, &sprites, pose, (0, None, 0), &mut skin),
                "{id}"
            );
        }
        assert_eq!(frame.new_model.len(), stored, "models stored again");
        eprintln!("{taken} item states on the GPU path, {left} on the CPU");
        assert!(taken > left, "{taken} taken, {left} left");
    }

    #[test]
    fn face_normals_of_a_pose_are_its_inverse_transpose() {
        // A dropped item's pose: spun, squashed and moved. Each face's normal is the
        // inverse transpose of the pose's 3x3 applied to the face's axis, renormalized,
        // here worked out in f64 from the cofactors.
        let m = Mat4::from_translation(Vec3::new(3.0, -1.0, 7.5))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_rotation_x(-0.3)
            * Mat4::from_scale(Vec3::new(0.5, 0.25, 2.0));
        let c = m.to_cols_array();
        let a = |row: usize, col: usize| f64::from(c[col * 4 + row]);
        let cofactor = |row: usize, col: usize| {
            let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
            let (c0, c1) = ((col + 1) % 3, (col + 2) % 3);
            a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
        };
        let mut normals = FaceNormals::new(m);
        // Twice each: a cached face gives what it gave the first time.
        for d in Direction::ALL.into_iter().chain(Direction::ALL) {
            let axis = direction_vector(d).to_array().map(f64::from);
            // inverse^T = cofactors / det: the det's sign is the only part that survives
            // renormalizing, and it is positive here.
            let n: Vec<f64> = (0..3)
                .map(|row| (0..3).map(|col| cofactor(row, col) * axis[col]).sum())
                .collect();
            let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            let got = normals.face(d);
            for (g, e) in got.to_array().into_iter().zip(&n) {
                assert!(
                    (f64::from(g) - e / len).abs() < 1e-6,
                    "{d:?}: {got} vs {n:?}"
                );
            }
            assert_eq!(got, transform_normal(m, direction_vector(d)), "{d:?}");
        }
    }
}
