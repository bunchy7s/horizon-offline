//! Profile-head I/O stays on workers; every account surface shares the same pixel-sharp cache.
use super::*;
use gpui::{img, RenderImage};
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// How long a head that fell back to the default skin waits before the real one is fetched again.
const RETRY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Heads(HashMap<String, Head>);

struct Head {
    url: Option<String>,
    pending: bool,
    face: Option<Vec<[u8; 4]>>,
    /// Set while `face` is a fallback because the skin download failed.
    retry_at: Option<Instant>,
    images: RefCell<HashMap<(u32, u32), Arc<RenderImage>>>,
}

impl Heads {
    pub fn request(&mut self, accounts: &[account::Account], resources: PathBuf, tx: &Sender<Msg>) {
        self.0
            .retain(|uuid, _| accounts.iter().any(|a| &a.uuid == uuid));
        for account in accounts {
            let url = account.skin.as_ref().map(|s| s.0.clone());
            if self.0.get(&account.uuid).is_some_and(|h| {
                h.url == url
                    && (h.pending
                        || h.face.is_some() && h.retry_at.is_none_or(|t| Instant::now() < t))
            }) {
                continue;
            }
            let previous = self.0.remove(&account.uuid).and_then(|h| h.face);
            self.0.insert(
                account.uuid.clone(),
                Head {
                    url: url.clone(),
                    pending: true,
                    face: previous,
                    retry_at: None,
                    images: RefCell::default(),
                },
            );
            let (account, resources, tx) = (account.clone(), resources.clone(), tx.clone());
            std::thread::spawn(move || {
                let (face, failed) = horizon_launcher::heads::profile_face(
                    &account,
                    &backend::data_dir().join("profile-heads"),
                    &resources,
                );
                let _ = tx.send(Msg::AccountHead(account.uuid, url, face, failed));
            });
        }
    }

    pub fn loaded(
        &mut self,
        uuid: &str,
        url: &Option<String>,
        face: Option<Vec<[u8; 4]>>,
        failed: bool,
    ) {
        if let Some(head) = self.0.get_mut(uuid).filter(|h| &h.url == url) {
            head.pending = false;
            head.retry_at = failed.then(|| Instant::now() + RETRY);
            if face.is_some() {
                head.face = face;
            }
            head.images.get_mut().clear();
        }
    }

    pub fn draw(&self, uuid: Option<&str>, size: f32, scale: f32, t: &Theme) -> Div {
        let image = uuid.and_then(|uuid| self.0.get(uuid)).and_then(|head| {
            let face = head.face.as_ref()?;
            Some(
                head.images
                    .borrow_mut()
                    .entry((size.to_bits(), scale.to_bits()))
                    .or_insert_with(|| art::bake(face, 8, 8, size / 8., scale))
                    .clone(),
            )
        });
        let d = div()
            .flex_none()
            .size(px(size))
            .overflow_hidden()
            .bg(t.well)
            .flex()
            .items_center()
            .justify_center();
        match image {
            Some(image) => d.child(img(image).size(px(size))),
            None => d.child(blocks::icon("user", size * 0.6, t.muted)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_skin_results_are_ignored_and_scaled_images_are_reused() {
        let mut heads = Heads::default();
        let current = Some("current-skin".to_string());
        heads.0.insert(
            "player".into(),
            Head {
                url: current.clone(),
                pending: true,
                face: None,
                retry_at: None,
                images: RefCell::default(),
            },
        );
        heads.loaded(
            "player",
            &Some("old-skin".into()),
            Some(vec![[255, 0, 0, 255]; 64]),
            false,
        );
        assert!(heads.0["player"].face.is_none());
        heads.loaded("player", &current, Some(vec![[0, 0, 255, 255]; 64]), true);
        assert!(
            heads.0["player"].retry_at.is_some(),
            "a fallback face is fetched again later"
        );
        heads.loaded("player", &current, Some(vec![[0, 255, 0, 255]; 64]), false);
        assert!(heads.0["player"].retry_at.is_none());
        let theme = Theme::for_appearance(
            backend::Appearance::Dark,
            horizon_launcher::themes::Accent::VIOLET,
            gpui::WindowAppearance::Dark,
        );
        let _ = heads.draw(Some("player"), 32., 2., &theme);
        let key = (32f32.to_bits(), 2f32.to_bits());
        let image = heads.0["player"].images.borrow()[&key].clone();
        assert_eq!(image.size(0).width.0, 64);
        assert_eq!(image.size(0).height.0, 64);
        let _ = heads.draw(Some("player"), 32., 2., &theme);
        assert!(Arc::ptr_eq(
            &image,
            &heads.0["player"].images.borrow()[&key]
        ));
    }
}
