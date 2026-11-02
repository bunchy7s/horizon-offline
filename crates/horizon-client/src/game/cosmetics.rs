//! Cosmetics on players ([docs/cosmetics.md](../../../../docs/cosmetics.md)): only Horizon
//! clients draw them, off unless `horizon_cosmetics::enabled()`. What another player wears comes
//! from the cosmetics service by UUID (`horizon_cosmetics::Cosmetics`, its own thread); this file
//! puts the items on the avatar's bones, on the GPU skin path when the renderer takes it.

use super::Game;
use horizon_client::fast_hash::FxHashMap;
use horizon_client::mobs::anim::Poses;
use horizon_client::mobs::{Mob, Part, Polygon};
use horizon_cosmetics::{Item, Slot};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// An item as the renderer takes it.
struct Baked {
    item: Arc<Item>,
    model: Arc<Part>,
    /// Its texture's sprite (a 64×64 skin slot of the atlas).
    location: String,
    /// The frame it was last marked drawn in (its slot is kept).
    marked: Cell<u32>,
}

#[derive(Default)]
pub(super) struct Cosmetics {
    client: Option<horizon_cosmetics::Cosmetics>,
    /// By the item's address; each entry holds its item, so the address isn't reused.
    baked: RefCell<FxHashMap<usize, Baked>>,
    /// Items drawn whose texture isn't in the atlas yet.
    uploads: RefCell<Vec<Arc<Item>>>,
    frame: Cell<u32>,
    /// Kept between players so a frame of avatars allocates none: the wearer's body, an item.
    poses: RefCell<(Poses, Poses)>,
    /// Item files from a folder (`HORIZON_COSMETICS_LOCAL`): mannequins wear them too.
    local: bool,
}

impl Cosmetics {
    fn client() -> (horizon_cosmetics::Cosmetics, bool) {
        if let Some(dir) = std::env::var_os("HORIZON_COSMETICS_LOCAL") {
            let local = horizon_cosmetics::Local::new(dir.into());
            return (horizon_cosmetics::Cosmetics::new(local, None), true);
        }
        let api = std::env::var("HORIZON_COSMETICS_API")
            .unwrap_or_else(|_| horizon_cosmetics::API.to_owned());
        let agent = format!("Horizon/{}", env!("CARGO_PKG_VERSION"));
        // Beside the downloaded skins (`assets/skins` in the game folder).
        let cache = std::env::current_dir()
            .unwrap_or_default()
            .join("assets/cosmetics");
        let http = horizon_cosmetics::Http::new(&api, &agent);
        (horizon_cosmetics::Cosmetics::new(http, Some(cache)), false)
    }
}

/// The game's model of an item's part.
fn part(p: &horizon_cosmetics::Part) -> Part {
    Part {
        name: p.name.clone(),
        pose: p.pose,
        polygons: p
            .faces
            .iter()
            .map(|f| Polygon {
                normal: f.normal,
                vertices: f.vertices,
                cull_back: false,
            })
            .collect(),
        children: p.children.iter().map(part).collect(),
    }
}

/// The texture in the top-left corner of a skin slot's 64×64 RGBA.
fn slot_image(item: &Item) -> Vec<u8> {
    let t = &item.texture;
    let mut rgba = vec![0; 64 * 64 * 4];
    for y in 0..t.height as usize {
        let row = &t.rgba[y * t.width as usize * 4..(y + 1) * t.width as usize * 4];
        rgba[y * 64 * 4..y * 64 * 4 + row.len()].copy_from_slice(row);
    }
    rgba
}

/// Which slots an avatar's gear covers: a helmet or head item hides a hat, an elytra or a cape
/// what is worn on the back.
fn covered(m: &Mob, slot: Slot) -> bool {
    match slot {
        Slot::Head => m.extra.head.is_some(),
        Slot::Back => m.cape.is_some() || m.extra.chest.as_deref() == Some("minecraft:elytra"),
        Slot::Shoulder => false,
    }
}

impl Game<'_> {
    /// Once a frame, before drawing: starts the service's client on first use and puts the
    /// textures the last frame wanted into the atlas.
    pub(super) fn cosmetics_frame(&mut self) {
        if !horizon_cosmetics::enabled() {
            return;
        }
        let c = &mut self.cosmetics;
        if c.client.is_none() {
            let (client, local) = Cosmetics::client();
            c.client = Some(client);
            c.local = local;
        }
        c.frame.set(c.frame.get().wrapping_add(1));
        let uploads = std::mem::take(&mut *c.uploads.borrow_mut());
        for item in uploads {
            let location = format!("cosmetics/{:x}", Arc::as_ptr(&item) as usize);
            if !self.gui.atlas.sprites.contains_key(&location) {
                self.store_skin(location, slot_image(&item));
            }
        }
        // Items no player wears any more.
        self.cosmetics
            .baked
            .borrow_mut()
            .retain(|_, b| Arc::strong_count(&b.item) > 1);
    }

    /// What `m` wears, if cosmetics are shown for it: `own` for the player's own model.
    fn worn(&self, m: &Mob, own: bool) -> Option<horizon_cosmetics::Worn> {
        let client = self.cosmetics.client.as_ref()?;
        let extras = &self.settings.extras;
        if !(if own {
            extras.cosmetics_mine
        } else {
            extras.cosmetics_others
        }) {
            return None;
        }
        let uuid = if own {
            self.remote.as_ref().map(|r| r.profile())?
        } else if &*m.kind.id == "minecraft:player" {
            self.remote_player_uuid(m.id)?
        } else if self.cosmetics.local && &*m.kind.id == "minecraft:mannequin" {
            u128::from(m.id)
        } else {
            return None;
        };
        client.frame().worn(uuid)
    }

    /// The cosmetics `m` wears, on its bones. `gpu`: posed on the GPU (the mob pass's skin
    /// frame, begun); the bounds of what went there come back, for culling. Else the vertices
    /// go to `out`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cosmetics_geometry(
        &self,
        m: &Mob,
        own: bool,
        base: glam::Mat4,
        alpha: f32,
        light: u32,
        gpu: bool,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) -> Option<(glam::Vec3, glam::Vec3)> {
        use horizon_client::mobs::{anim, render};
        let worn = self.worn(m, own)?;
        if horizon_client::mobs::trader::is_invisible(m) {
            return None;
        }
        // The avatar's body, posed as it is drawn.
        let (key, _) = render::body(m);
        let body = m.kind.layer(&key).unwrap_or(&m.kind.model);
        let mut scratch = self.cosmetics.poses.borrow_mut();
        let (body_poses, poses) = &mut *scratch;
        body_poses.reset(body);
        let data = self.res.mob_data();
        anim::setup(m, &anim::state(m, alpha), body_poses, &data.animations);
        let ticks = m.age as f32 + alpha;
        let frame = self.cosmetics.frame.get();
        let mut bounds = None;
        // Items on one bone (a back item and a shoulder pet both ride the body) share its matrix.
        let mut bones: [Option<(horizon_cosmetics::Bone, glam::Mat4)>; 3] = [None; 3];
        for item in worn.iter() {
            if covered(m, item.slot) {
                continue;
            }
            let mut baked = self.cosmetics.baked.borrow_mut();
            let b = baked
                .entry(Arc::as_ptr(item) as usize)
                .or_insert_with(|| Baked {
                    item: item.clone(),
                    model: Arc::new(part(&item.root)),
                    location: format!("cosmetics/{:x}", Arc::as_ptr(item) as usize),
                    marked: Cell::new(frame.wrapping_sub(1)),
                });
            let Some(rect) = self.gui.atlas.sprites.get(&b.location) else {
                self.cosmetics.uploads.borrow_mut().push(item.clone());
                continue;
            };
            if b.marked.replace(frame) != frame {
                self.skin_downloads.borrow_mut().drawn(&b.location);
            }
            let known = bones.iter().flatten().find(|(b, _)| *b == item.bone);
            let bone = match known {
                Some(&(_, matrix)) => matrix,
                None => {
                    let Some(matrix) =
                        render::part_matrix(body, base, body_poses, item.bone.part(), 0.0)
                    else {
                        continue;
                    };
                    if let Some(free) = bones.iter_mut().find(|b| b.is_none()) {
                        *free = Some((item.bone, matrix));
                    }
                    matrix
                }
            };
            // The texture fills the top-left of its 64×64 slot.
            let (w, h) = (
                item.texture.width as f32 / 64.0,
                item.texture.height as f32 / 64.0,
            );
            let uv = [
                rect.u0,
                rect.v0,
                rect.u0 + (rect.u1 - rect.u0) * w,
                rect.v0 + (rect.v1 - rect.v0) * h,
            ];
            poses.reset(&b.model);
            item.animate(ticks, |name, pose| *poses.get(name) = pose);
            if gpu {
                let mut models = self.skin_models.borrow_mut();
                let mut frame = self.skin_frame.borrow_mut();
                let mut skin = horizon_client::mobs::skin::Skin::new(&mut models, &mut frame);
                skin.bounds = bounds;
                let paint = horizon_client::mobs::skin::Paint {
                    uv,
                    scroll: 0.0,
                    color: 0xFFFF_FFFF,
                    light,
                    overlay: 0,
                };
                if skin.draw(&b.model, bone, poses, paint) {
                    bounds = skin.bounds;
                    continue;
                }
            }
            horizon_client::mobs::render_model(
                &b.model,
                bone,
                &|p: &Part| poses.pose(p),
                uv,
                light,
                0,
                out,
            );
        }
        bounds
    }
}
