//! What an instance's icon shows (plan R8): its own picture (one the player chose, or its
//! modpack's icon) first; else the block or colour chosen in its dialog; else its mark: the loader
//! it plays with, the grass block for plain Minecraft, Horizon's own for Horizon. The letter tile
//! is the last resort. Each is decoded once per size and kept in `Instances::icons`.
use super::*;
use gpui::RenderImage;
use horizon_launcher::browser::images;
use horizon_launcher::instances::{self, Instance, Kind as InstanceKind};
use std::sync::Arc;

/// Quilt's mini icon (CC0) and NeoForge's (CC BY 4.0), from their own art repositories: see
/// THIRD_PARTY_NOTICES.md.
const QUILT: &[u8] = include_bytes!("../assets/logos/quilt.png");
const NEOFORGE: &[u8] = include_bytes!("../assets/logos/neoforge.png");
/// Fabric and Forge publish no licence for their logos: their own icons, fetched from their own
/// sites into the image cache, never shipped.
const FABRIC: &str = "https://raw.githubusercontent.com/FabricMC/fabric-loader/master/src/main/resources/assets/fabricloader/icon.png";
const FORGE: &str = "https://files.minecraftforge.net/static/images/favicon-32x32.png";
/// The loaders a logo is drawn for.
const LOADERS: [&str; 4] = ["fabric", "quilt", "forge", "neoforge"];

impl Launcher {
    /// The instance's own picture, `size` points square. Whether there is one is asked once:
    /// `reload_instances` forgets the answer.
    pub(super) fn instance_picture(
        &mut self,
        instance: &Instance,
        size: f32,
    ) -> Option<Arc<RenderImage>> {
        let file = instance.home.join(instances::PICTURE);
        let key = format!("picture:{}", instance.id);
        self.icon_image(key, size, true, || std::fs::read(file).ok())
    }

    /// What an instance shows when nothing was chosen for it.
    pub(super) fn instance_mark(
        &mut self,
        instance: &Instance,
        size: f32,
    ) -> Option<Arc<RenderImage>> {
        let loader = match instance.kind {
            InstanceKind::Port => {
                let slug = &instance.id["port:".len()..];
                let pack = self.packs.iter().find(|p| p.slug == slug);
                pack.map(|p| p.pack.loader.to_ascii_lowercase())
            }
            _ => instance
                .java
                .as_ref()
                .and_then(|j| j.loader.as_ref())
                .map(|l| l.kind.id().to_string()),
        };
        if let Some(loader) = loader.filter(|l| LOADERS.contains(&l.as_str())) {
            return self.loader_logo(&loader, size);
        }
        if instance.java.is_some() {
            let grass = self.vanilla_icon()?;
            return self.icon_image("mark:grass".into(), size, false, || {
                std::fs::read(grass).ok()
            });
        }
        let horizon = || Some(horizon_theme::ICON_PNG.to_vec());
        self.icon_image("mark:horizon".into(), size, false, horizon)
    }

    /// A mod loader's logo by its id (`fabric`, `quilt`, `forge`, `neoforge`): None while a
    /// fetched one is on its way, or for another id.
    pub(super) fn loader_logo(&mut self, loader: &str, size: f32) -> Option<Arc<RenderImage>> {
        let bundled = |png: &'static [u8]| move || Some(png.to_vec());
        let key = format!("mark:{loader}");
        match loader {
            "quilt" => self.icon_image(key, size, false, bundled(QUILT)),
            "neoforge" => self.icon_image(key, size, false, bundled(NEOFORGE)),
            "fabric" | "forge" => {
                let url = if loader == "fabric" { FABRIC } else { FORGE };
                let file = self.mods_image(url, images::ICON)?;
                self.icon_image(key, size, false, || std::fs::read(file).ok())
            }
            _ => None,
        }
    }

    /// The image under `key` at `size` points, made from `bytes` the first time. A missing one is
    /// remembered only when `remember_missing` (a file that may never exist); else it is asked
    /// again next frame (a logo still downloading).
    fn icon_image(
        &mut self,
        key: String,
        size: f32,
        remember_missing: bool,
        bytes: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<Arc<RenderImage>> {
        let key = (key, (size * self.scale) as u32);
        if let Some(image) = self.instances.icons.get(&key) {
            return image.clone();
        }
        let image = bytes().and_then(|b| fitted(b, size, self.scale));
        if image.is_some() || remember_missing {
            self.instances.icons.insert(key, image.clone());
        }
        image
    }
}

/// An image file's bytes drawn at most `size` points square: scaled down smoothly when larger,
/// up by whole device pixels when smaller, so pixel-art logos stay sharp.
fn fitted(bytes: Vec<u8>, size: f32, scale: f32) -> Option<Arc<RenderImage>> {
    let side = (size * scale).round().max(1.) as u32;
    let png = images::fit_png(bytes, side, 4096, 64 << 20)?;
    let image = image::load_from_memory(&png).ok()?.to_rgba8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let texels: Vec<[u8; 4]> = image.pixels().map(|p| p.0).collect();
    let whole = (side as usize / w.max(h)).max(1) as f32;
    Some(art::bake(&texels, w, h, whole / scale, scale))
}

/// An image made by [`fitted`], centred in a `size` points square.
pub(super) fn centred(image: Arc<RenderImage>, size: f32, scale: f32) -> Div {
    let dim = image.size(0);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .child(
            gpui::img(image)
                .w(px(dim.width.0 as f32 / scale))
                .h(px(dim.height.0 as f32 / scale)),
        )
}

/// A picture chosen in the instance dialog, written on Save.
pub(super) struct Chosen {
    pub(super) png: Vec<u8>,
    pub(super) preview: Arc<RenderImage>,
}

/// The most a chosen file may weigh: a photo, not a film.
const MAX_PICTURE_FILE: u64 = 64 << 20;

impl Launcher {
    /// Asks for an image file and makes it the dialog's picture, read and scaled off the UI
    /// thread.
    pub(super) fn choose_instance_picture(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        let scale = self.scale;
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let made = cx
                .background_executor()
                .spawn(async move {
                    let too_big = std::fs::metadata(&path).map_err(|e| e.to_string())?.len()
                        > MAX_PICTURE_FILE;
                    let bytes = (!too_big)
                        .then(|| std::fs::read(&path).ok())
                        .flatten()
                        .ok_or_else(|| format!("{}: too big, or unreadable", path.display()))?;
                    let png = instances::picture(bytes)?;
                    let preview = fitted(png.clone(), 28., scale).ok_or("Not a picture")?;
                    Ok::<_, String>(Chosen { png, preview })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(d) = &mut this.instances.dialog {
                    match made {
                        Ok(chosen) => {
                            d.picture = Some(Some(chosen));
                            d.error = None;
                        }
                        Err(e) => d.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
