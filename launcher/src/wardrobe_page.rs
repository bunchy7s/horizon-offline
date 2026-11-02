//! Settings > Account's Wardrobe for a Microsoft account, shown only with the cosmetics flag on
//! (`horizon_cosmetics::enabled`): the account's skin wearing its items, turned with two buttons,
//! the early-player set to claim while its window is open, and what each slot wears. The talking
//! is `horizon_launcher::wardrobe`'s, on a background thread.
use super::*;
use gpui::RenderImage;
use horizon_cosmetics::{preview, Item, Texture};
use horizon_launcher::wardrobe::{
    Catalogue, Claim, ClaimState, Entry, Error, Me, Service, Session,
};
use std::collections::HashMap;
use std::sync::Arc;

/// The still picture's size, in points.
const W: f32 = 150.;
const H: f32 = 220.;

/// What a background request brings back.
struct Loaded {
    catalogue: Catalogue,
    items: HashMap<String, Arc<Item>>,
    session: Arc<Session>,
    me: Me,
    skin: Option<(Texture, bool)>,
}

pub(super) struct WardrobePanel {
    /// The Microsoft account it is for.
    pub(super) uuid: String,
    loaded: Option<Loaded>,
    busy: bool,
    error: Option<String>,
    /// Quarter turns of the picture.
    turn: u8,
    /// The picture, baked at (window scale, turn, what is worn).
    seen: Option<(f32, u8, Vec<String>, Arc<RenderImage>)>,
}

/// The game language whose item names the launcher's language reads.
fn game_language() -> &'static str {
    match horizon_launcher::i18n::current().code() {
        "fr" => "fr_fr",
        "de" => "de_de",
        _ => "en_us",
    }
}

/// The account's skin as the preview takes it: from the profile heads' cache, 64×64 only (older
/// 64×32 skins and missing ones show a plain figure).
fn skin(account: &account::Account) -> Option<(Texture, bool)> {
    let (url, model) = account.skin.as_ref()?;
    let png =
        horizon_launcher::heads::cached_skin(url, &backend::data_dir().join("profile-heads"))?;
    let image = image::load_from_memory(&png).ok()?.to_rgba8();
    (image.dimensions() == (64, 64)).then(|| {
        (
            Texture {
                width: 64,
                height: 64,
                rgba: image.into_raw(),
            },
            model == "slim",
        )
    })
}

/// A plain figure's texture, for an account without a skin the preview can read: head and arms,
/// shirt and trousers in three greys, the outer layers empty.
fn plain() -> Texture {
    let mut rgba = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let c: [u8; 4] = match (x, y) {
                (0..32, 0..16) | (40..56, 16..32) | (32..48, 48..64) => [196, 196, 204, 255],
                (16..40, 16..32) => [128, 132, 148, 255],
                (0..16, 16..32) | (16..32, 48..64) => [92, 94, 108, 255],
                _ => [0, 0, 0, 0],
            };
            rgba[(y * 64 + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    Texture {
        width: 64,
        height: 64,
        rgba,
    }
}

impl Launcher {
    pub(super) fn open_wardrobe(&mut self, uuid: String, cx: &mut Context<Self>) {
        if self.wardrobe.as_ref().is_some_and(|w| w.uuid == uuid) {
            self.wardrobe = None;
            return;
        }
        let Some(account) = self.accounts.find(&uuid).cloned() else {
            return;
        };
        self.wardrobe = Some(WardrobePanel {
            uuid,
            loaded: None,
            busy: true,
            error: None,
            turn: 0,
            seen: None,
        });
        let task = cx.background_executor().spawn(async move {
            let service = Service::live();
            let catalogue = service.catalogue()?;
            let items = catalogue
                .items
                .iter()
                .filter_map(|e| Some((e.id.clone(), Arc::new(service.item(e).ok()?))))
                .collect();
            let session = service.sign_in(&account.name, |id| {
                account::session::join_server(&account.uuid, id)
            })?;
            let me = service.me(&session)?;
            Ok::<_, Error>(Loaded {
                catalogue,
                items,
                session: Arc::new(session),
                me,
                skin: skin(&account),
            })
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(panel) = &mut this.wardrobe {
                    panel.busy = false;
                    match done {
                        Ok(loaded) => panel.loaded = Some(loaded),
                        Err(e) => panel.error = Some(e.message()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `HORIZON_LAUNCHER_PAGE=wardrobe` in a development build: Settings with a made-up account's
    /// Wardrobe open, wearing the items of `HORIZON_COSMETICS_LOCAL`, for screenshots. Nothing is
    /// asked of Mojang or the service, and nothing is saved.
    pub(super) fn wardrobe_preview(&mut self) {
        let Some(dir) = std::env::var_os("HORIZON_COSMETICS_LOCAL") else {
            return;
        };
        let mut items: Vec<Item> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Item::parse(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        items.sort_by(|a, b| a.id.cmp(&b.id));
        let slot = |i: &Item| format!("{:?}", i.slot).to_lowercase();
        let account = account::Account {
            uuid: "00000000000000000000000000000001".into(),
            name: "Preview".into(),
            skin: None,
            cape: None,
        };
        let mut equipped = std::collections::BTreeMap::new();
        for i in &items {
            equipped.entry(slot(i)).or_insert_with(|| i.id.clone());
        }
        let claim = Claim {
            state: ClaimState::Open,
            items: items.iter().map(|i| i.id.clone()).collect(),
            starts_at: None,
            ends_at: None,
            claimed: Some(false),
        };
        let loaded = Loaded {
            catalogue: Catalogue {
                items: items
                    .iter()
                    .map(|i| Entry {
                        id: i.id.clone(),
                        hash: String::new(),
                        slot: slot(i),
                        names: [("en_us".to_owned(), i.id.replace('_', " "))].into(),
                    })
                    .collect(),
                claim: claim.clone(),
            },
            me: Me {
                uuid: account.uuid.clone(),
                name: account.name.clone(),
                owned: items.iter().map(|i| i.id.clone()).collect(),
                equipped,
                claim,
            },
            session: Arc::new(Session::preview(&account.uuid, &account.name)),
            items: items
                .into_iter()
                .map(|i| (i.id.clone(), Arc::new(i)))
                .collect(),
            skin: None,
        };
        // Settings first: a page switch reads accounts.json again, which doesn't hold this one.
        self.go(Page::Settings);
        self.accounts.accounts.push(account.clone());
        self.owner = Some(true);
        self.wardrobe = Some(WardrobePanel {
            uuid: account.uuid,
            loaded: Some(loaded),
            busy: false,
            error: None,
            turn: 0,
            seen: None,
        });
    }

    /// Claim the early-player set, or wear `item` in `slot` (None: nothing there).
    fn wardrobe_change(
        &mut self,
        change: Option<(String, Option<String>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &mut self.wardrobe else {
            return;
        };
        let Some(session) = panel.loaded.as_ref().map(|l| l.session.clone()) else {
            return;
        };
        if panel.busy {
            return;
        }
        panel.busy = true;
        panel.error = None;
        let uuid = panel.uuid.clone();
        let task = cx.background_executor().spawn(async move {
            let service = Service::live();
            match &change {
                None => service.claim(&session),
                Some((slot, item)) => service.equip(&session, slot, item.as_deref()),
            }
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(panel) = this.wardrobe.as_mut().filter(|p| p.uuid == uuid) {
                    panel.busy = false;
                    match done {
                        Ok(me) => {
                            if let Some(l) = &mut panel.loaded {
                                l.me = me;
                            }
                        }
                        Err(e) => panel.error = Some(e.message()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The account's skin wearing what it wears now, baked once per change.
    fn wardrobe_picture(&mut self) -> Option<Arc<RenderImage>> {
        let scale = self.scale;
        let panel = self.wardrobe.as_mut()?;
        let loaded = panel.loaded.as_ref()?;
        let worn: Vec<String> = loaded.me.equipped.values().cloned().collect();
        if let Some((s, turn, w, image)) = &panel.seen {
            if *s == scale && *turn == panel.turn && *w == worn {
                return Some(image.clone());
            }
        }
        let fallback = plain();
        let (texture, slim) = match &loaded.skin {
            Some((t, slim)) => (t, *slim),
            None => (&fallback, false),
        };
        let body = preview::player_model(slim);
        let mut draws = vec![preview::Draw {
            root: &body,
            base: Default::default(),
            texture,
            poses: Vec::new(),
        }];
        draws.extend(
            worn.iter()
                .filter_map(|id| loaded.items.get(id))
                .filter_map(|item| preview::Draw::item(item, &body, 0.0)),
        );
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        // Degrees: a little turned, then a quarter more per click.
        let yaw = 30. + 90. * f32::from(panel.turn);
        let image = preview::render(&draws, w, h, yaw, 10.);
        let texels: Vec<[u8; 4]> = image.rgba.as_chunks::<4>().0.to_vec();
        let baked = art::bake(&texels, w as usize, h as usize, 1. / scale, scale);
        panel.seen = Some((scale, panel.turn, worn, baked.clone()));
        Some(baked)
    }

    /// The panel, under the Microsoft accounts while open.
    pub(super) fn wardrobe_panel(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let uuid = self.wardrobe.as_ref()?.uuid.clone();
        let Some(name) = self.accounts.find(&uuid).map(|a| a.name.clone()) else {
            self.wardrobe = None;
            return None;
        };
        let picture = self.wardrobe_picture();
        let panel = self.wardrobe.as_ref()?;
        let busy = panel.busy;
        let mut side = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(12.))
            .child(caps(trf("{name}'s Wardrobe", &[("name", &name)])).text_color(t.muted));
        if let Some(l) = &panel.loaded {
            side = side.child(self.claim_row(l, busy, cx));
            let lang = game_language();
            for (slot, label) in [("head", "Head"), ("back", "Back"), ("shoulder", "Shoulder")] {
                let owned: Vec<_> = l
                    .catalogue
                    .items
                    .iter()
                    .filter(|e| e.slot == slot && l.me.owned.contains(&e.id))
                    .collect();
                if owned.is_empty() {
                    continue;
                }
                let wearing = l.me.equipped.get(slot);
                let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(
                    text(theme::LABEL)
                        .w(px(80.))
                        .text_color(t.ink)
                        .child(tr(label)),
                );
                let pick = |id: String, label: String, item: Option<String>, on: bool| {
                    let b = button(id, label, if on { Kind::Primary } else { Kind::Ghost }, &t);
                    if busy || on {
                        return disabled(b);
                    }
                    let slot = slot.to_owned();
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.wardrobe_change(Some((slot.clone(), item.clone())), cx);
                        cx.notify();
                    }))
                };
                row = row.child(pick(
                    format!("wardrobe-{slot}-none"),
                    tr("Nothing").into(),
                    None,
                    wearing.is_none(),
                ));
                for e in owned {
                    row = row.child(pick(
                        format!("wardrobe-{slot}-{}", e.id),
                        e.name(lang).to_owned(),
                        Some(e.id.clone()),
                        wearing == Some(&e.id),
                    ));
                }
                side = side.child(row);
            }
        } else if busy {
            side = side.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(tr("Checking your account with Mojang…")),
            );
        }
        if let Some(e) = &panel.error {
            side = side.child(text(theme::CAPTION).text_color(t.danger).child(e.clone()));
        }
        side = side.child(text(theme::CAPTION).text_color(t.muted).child(tr(
            "Only Horizon players see what you wear; other games and servers don't. \
             Items change nothing in the game.",
        )));
        let turn = button("wardrobe-turn", "Turn", Kind::Ghost, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                if let Some(p) = &mut this.wardrobe {
                    p.turn = (p.turn + 1) % 4;
                }
                cx.notify();
            },
        ));
        Some(
            plate(&t)
                .flex()
                .gap(px(20.))
                .px(px(16.))
                .py(px(14.))
                .ml(px(40.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(W + 24.))
                                .h(px(H + 24.))
                                .bg(t.well)
                                .border_2()
                                .border_color(t.outline)
                                .children(picture.map(|i| gpui::img(i).w(px(W)).h(px(H)))),
                        )
                        .child(turn),
                )
                .child(side),
        )
    }

    /// The early-player set's line: claim it while open, or why not.
    fn claim_row(&self, l: &Loaded, busy: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let claim = &l.me.claim;
        let (said, can) = match (claim.state, claim.claimed == Some(true)) {
            (_, true) => (tr("You have the early-player set."), false),
            (ClaimState::Open, false) => (
                tr("The early-player set is free to claim for a limited time."),
                true,
            ),
            (ClaimState::NotYet, false) => {
                (tr("The early-player set can't be claimed yet."), false)
            }
            (ClaimState::Closed, false) => {
                (tr("The early-player set can no longer be claimed."), false)
            }
        };
        let mut row = div().flex().items_center().gap(px(10.)).child(
            text(theme::CAPTION)
                .flex_1()
                .text_color(t.muted)
                .child(said),
        );
        if can {
            let b = button("wardrobe-claim", "Claim", Kind::Primary, &t);
            row = row.child(if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.wardrobe_change(None, cx);
                    cx.notify();
                }))
            });
        }
        row
    }
}
