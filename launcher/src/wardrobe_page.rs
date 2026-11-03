//! The Wardrobe page (top bar, between Servers and Versions), shown only with the cosmetics flag
//! on (`horizon_cosmetics::enabled`): the signed-in Microsoft account's skin wearing its items, a
//! row per slot, what the account owns for the chosen slot, the founder set to claim while its
//! window is open, and three switches: Show my cosmetics (the account's, on the service), Hide
//! other players' cosmetics and Show other players' auras and trails (the game's
//! `horizon-extras.txt`, `horizon_cosmetics::prefs`, also in its settings).
//!
//! It opens on the outfit kept on disk (`wardrobe::Saved`), so it shows offline and when the
//! service fails; a change shows at once and waits on disk until the service takes it. The talking
//! is `horizon_launcher::wardrobe`'s, on a background thread.
use super::*;
use gpui::RenderImage;
use horizon_cosmetics::prefs::{Others, Prefs};
use horizon_cosmetics::{preview, Item, Texture};
use horizon_launcher::wardrobe::{Catalogue, Change, ClaimState, Error, Saved, Service, Session};
use std::collections::HashMap;
use std::sync::Arc;

/// The still picture's size, in points.
const W: f32 = 180.;
const H: f32 = 264.;
/// How long a line like "Equipped Sunlet." stays.
const TOAST: Duration = Duration::from_secs(4);

/// The slots a row each, with their names. Themes are the launcher's own; name tags have no model.
const SLOTS: [(&str, &str); 5] = [
    ("head", "Head"),
    ("back", "Back"),
    ("shoulder", "Shoulder pet"),
    ("name_tag", "Name tag"),
    ("theme", "Launcher theme"),
];

/// Where the service stands, as the page tells it.
#[derive(Clone, PartialEq)]
enum Status {
    Loading,
    Ready,
    /// Not reached: changes wait on disk.
    Offline,
    /// It answered with a failure; the details line to copy.
    Failed(String),
    /// Mojang didn't confirm the account, in words.
    Account(String),
}

pub(super) struct Wardrobe {
    /// The Microsoft account it is for.
    uuid: String,
    saved: Saved,
    items: HashMap<String, Arc<Item>>,
    session: Option<Arc<Session>>,
    skin: Option<(Texture, bool)>,
    status: Status,
    busy: bool,
    slot: &'static str,
    /// Quarter turns of the picture.
    turn: u8,
    /// The picture, baked at (window scale, turn, what is worn).
    seen: Option<(f32, u8, Vec<String>, Arc<RenderImage>)>,
    toast: Option<(String, Instant)>,
}

/// What a background trip to the service brings back.
struct Trip {
    saved: Saved,
    items: HashMap<String, Arc<Item>>,
    session: Option<Arc<Session>>,
    result: Result<(), Error>,
}

/// The launcher's data folder for the Wardrobe: per-account outfits, item files by hash.
fn dir() -> std::path::PathBuf {
    backend::data_dir().join("wardrobe")
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

/// The item files the catalogue names, kept or fetched (a missing one is left out).
fn fetch_items(service: &Service, catalogue: &Catalogue) -> HashMap<String, Arc<Item>> {
    let cache = dir().join("items");
    catalogue
        .items
        .iter()
        .filter(|e| e.theme.is_none())
        .filter_map(|e| Some((e.id.clone(), Arc::new(service.item_cached(e, &cache).ok()?))))
        .collect()
}

/// The catalogue (the kept one's items if the service isn't reached), a session proved with
/// Mojang, what waits sent, then the account's own answer with anything still waiting on top.
fn refresh(service: &Service, account: &account::Account, trip: &mut Trip) -> Result<(), Error> {
    let catalogue = match service.catalogue() {
        Ok(c) => c,
        Err(e) => {
            if let Some(c) = &trip.saved.catalogue {
                trip.items = fetch_items(service, c);
            }
            return Err(e);
        }
    };
    trip.items = fetch_items(service, &catalogue);
    trip.saved.catalogue = Some(catalogue);
    let session = Arc::new(service.sign_in(&account.name, |id| {
        account::session::join_server(&account.uuid, id)
    })?);
    trip.session = Some(session.clone());
    service.sync(&session, &mut trip.saved)?;
    let mut me = service.me(&session)?;
    for change in &trip.saved.pending {
        me.apply(change);
    }
    trip.saved.me = Some(me);
    Ok(())
}

/// `founder_visor` → "Founder Visor", for the preview's made-up names.
fn title_case(id: &str) -> String {
    id.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A day, in the player's time zone (Unix seconds).
fn day(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

impl Launcher {
    /// On every visit to the page: the selected Microsoft account's kept outfit at once, then the
    /// service's answer, sending what waits.
    pub(super) fn open_wardrobe(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.wardrobe_account() else {
            self.wardrobe = None;
            return;
        };
        if self
            .wardrobe
            .as_ref()
            .is_some_and(|w| w.uuid == account.uuid && w.busy)
        {
            return;
        }
        let keep = self.wardrobe.take().filter(|w| w.uuid == account.uuid);
        let saved = Saved::load(&dir(), &account.uuid);
        let (slot, turn) = keep.as_ref().map_or(("head", 0), |w| (w.slot, w.turn));
        self.wardrobe = Some(Wardrobe {
            uuid: account.uuid.clone(),
            saved: saved.clone(),
            items: keep.map(|w| w.items).unwrap_or_default(),
            session: None,
            skin: skin(&account),
            status: Status::Loading,
            busy: true,
            slot,
            turn,
            seen: None,
            toast: None,
        });
        let task = cx.background_executor().spawn(async move {
            let service = Service::live();
            let mut trip = Trip {
                saved,
                items: HashMap::new(),
                session: None,
                result: Ok(()),
            };
            trip.result = refresh(&service, &account, &mut trip);
            let _ = trip.saved.save(&dir(), &account.uuid);
            trip
        });
        cx.spawn(async move |this, cx| {
            let trip = task.await;
            let _ = this.update(cx, |this, cx| {
                this.wardrobe_back(trip, None);
                cx.notify();
            });
        })
        .detach();
    }

    /// The launcher theme the selected account wears (its sky and accent hue), from the outfit
    /// kept on disk; None with cosmetics off.
    pub(super) fn worn_theme(&self) -> Option<([u32; 6], f32)> {
        if !horizon_cosmetics::enabled() {
            return None;
        }
        let kept;
        let saved = match &self.wardrobe {
            Some(w) => &w.saved,
            None => {
                kept = Saved::load(&dir(), &self.wardrobe_account()?.uuid);
                &kept
            }
        };
        saved.theme()
    }

    /// The account the page is for: the selected one, if it signed in with Microsoft.
    fn wardrobe_account(&self) -> Option<account::Account> {
        match self.accounts.player() {
            account::Player::Microsoft(_) => self.accounts.selected().cloned(),
            _ => None,
        }
    }

    /// A trip came back: keep what it learnt, say how it went.
    fn wardrobe_back(&mut self, trip: Trip, done: Option<String>) {
        let Some(w) = self.wardrobe.as_mut() else {
            return;
        };
        w.busy = false;
        if !trip.items.is_empty() {
            w.items = trip.items;
        }
        if trip.session.is_some() {
            w.session = trip.session;
        }
        w.saved = trip.saved;
        w.status = match trip.result {
            Ok(()) => Status::Ready,
            Err(Error::Offline) => Status::Offline,
            Err(Error::Account(e)) => Status::Account(tr(&e).into()),
            Err(e) => {
                let host = Service::live().host().to_owned();
                let code = match e {
                    Error::Service(c) => c.to_string(),
                    other => format!("{other:?}"),
                };
                let at = chrono::Local::now().format("%Y-%m-%d %H:%M");
                Status::Failed(format!("{host} · {code} · {at}"))
            }
        };
        let said = match (&w.status, done) {
            (Status::Ready, Some(done)) => Some(done),
            (Status::Ready, None) => None,
            (_, Some(_)) => {
                Some(tr("Couldn't sync your outfit. It'll try again when you're online.").into())
            }
            (_, None) => None,
        };
        w.toast = said.map(|s| (s, Instant::now()));
        self.restyle = true;
    }

    /// Wear `item` in `slot` (None: nothing there), or turn Show my cosmetics on or off: at once
    /// here and on disk, then on the service.
    fn wardrobe_change(&mut self, change: Change, cx: &mut Context<Self>) {
        let lang = game_language();
        let Some(w) = self.wardrobe.as_mut() else {
            return;
        };
        let done: String = match &change {
            Change::Equip { item: Some(id), .. } => {
                let name = (w.saved.catalogue.iter())
                    .flat_map(|c| &c.items)
                    .find(|e| &e.id == id)
                    .map_or(id.as_str(), |e| e.name(lang));
                trf(
                    "Equipped {name}. Others see it the next time they meet you.",
                    &[("name", name)],
                )
            }
            Change::Equip { item: None, .. } => {
                tr("Taken off. Others see it the next time they meet you.").into()
            }
            Change::Visible(true) => tr("Other players see your cosmetics again.").into(),
            Change::Visible(false) => tr("Other players see you without your cosmetics.").into(),
        };
        w.saved.change(change);
        let _ = w.saved.save(&dir(), &w.uuid);
        w.seen = None;
        self.restyle = true;
        let Some(w) = self.wardrobe.as_mut() else {
            return;
        };
        let Some(session) = w.session.clone().filter(|s| s.live()) else {
            // Offline, or the session ended: it waits, and the next visit sends it.
            w.toast = Some((
                tr("Saved on this computer. It syncs when you're back online.").into(),
                Instant::now(),
            ));
            return;
        };
        if w.busy {
            return;
        }
        w.busy = true;
        let (uuid, mut saved) = (w.uuid.clone(), w.saved.clone());
        let task = cx.background_executor().spawn(async move {
            let result = Service::live().sync(&session, &mut saved);
            let _ = saved.save(&dir(), &uuid);
            Trip {
                saved,
                items: HashMap::new(),
                session: None,
                result,
            }
        });
        self.wardrobe_wait(task, done, cx);
    }

    /// Claim the founder set.
    fn wardrobe_claim(&mut self, cx: &mut Context<Self>) {
        let Some(w) = self.wardrobe.as_mut().filter(|w| !w.busy) else {
            return;
        };
        let Some(session) = w.session.clone() else {
            return;
        };
        w.busy = true;
        let (uuid, mut saved) = (w.uuid.clone(), w.saved.clone());
        let task = cx.background_executor().spawn(async move {
            let result = Service::live()
                .claim(&session)
                .map(|me| saved.me = Some(me));
            let _ = saved.save(&dir(), &uuid);
            Trip {
                saved,
                items: HashMap::new(),
                session: None,
                result,
            }
        });
        let done = tr("Founder set claimed. It's in your wardrobe.").into();
        self.wardrobe_wait(task, done, cx);
    }

    fn wardrobe_wait(&mut self, task: gpui::Task<Trip>, done: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let trip = task.await;
            let _ = this.update(cx, |this, cx| {
                this.wardrobe_back(trip, Some(done));
                cx.notify();
            });
            // Let the line go.
            cx.background_executor().timer(TOAST).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// `HORIZON_LAUNCHER_PAGE=wardrobe` in a development build: the page for a made-up account
    /// that owns the items of `HORIZON_COSMETICS_LOCAL` and wears the first of each slot, for
    /// screenshots. Nothing is asked of Mojang or the service, and nothing is saved.
    /// `HORIZON_WARDROBE_STATE=offline|failed|closed|empty` shows that state instead, `theme` wears
    /// First Light.
    pub(super) fn wardrobe_preview(&mut self) {
        use horizon_launcher::wardrobe::{Claim, Entry, Me};
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
        let slot = |i: &Item| match i.slot {
            horizon_cosmetics::Slot::Head => "head",
            horizon_cosmetics::Slot::Back => "back",
            horizon_cosmetics::Slot::Shoulder => "shoulder",
            horizon_cosmetics::Slot::NameTag => "name_tag",
        };
        let state = std::env::var("HORIZON_WARDROBE_STATE").unwrap_or_default();
        let mut equipped = std::collections::BTreeMap::new();
        for i in &items {
            equipped
                .entry(slot(i).to_owned())
                .or_insert_with(|| i.id.clone());
        }
        let mut ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        // A launcher theme to wear: the design brief's First Light.
        let dawn = Entry {
            id: "first_light".into(),
            hash: String::new(),
            slot: "theme".into(),
            names: [("en_us".to_owned(), "First Light".to_owned())].into(),
            theme: Some(horizon_launcher::wardrobe::Theme {
                sky: [
                    "#0b1030", "#1d2a63", "#3d4c94", "#7f78c0", "#e2a0b0", "#ffd6a0",
                ]
                .map(String::from),
                accent_hue: 246.,
            }),
        };
        ids.push(dawn.id.clone());
        if state == "theme" {
            equipped.insert("theme".to_owned(), dawn.id.clone());
        }
        let claim = Claim {
            state: if state == "closed" {
                ClaimState::Closed
            } else {
                ClaimState::Open
            },
            items: ids.clone(),
            starts_at: Some(1_793_000_000),
            ends_at: Some(1_795_000_000),
            claimed: Some(state != "empty" && state != "closed"),
        };
        let empty = state == "empty" || state == "closed";
        let catalogue = Catalogue {
            items: items
                .iter()
                .map(|i| Entry {
                    id: i.id.clone(),
                    hash: String::new(),
                    slot: slot(i).to_owned(),
                    names: [("en_us".to_owned(), title_case(&i.id))].into(),
                    theme: None,
                })
                .chain([dawn])
                .collect(),
            claim: claim.clone(),
        };
        let account = account::Account {
            uuid: "00000000000000000000000000000001".into(),
            name: "Preview".into(),
            skin: None,
            cape: None,
        };
        let mut saved = Saved {
            me: Some(Me {
                uuid: account.uuid.clone(),
                name: account.name.clone(),
                owned: if empty { Vec::new() } else { ids },
                equipped: if empty { Default::default() } else { equipped },
                visible: true,
                claim,
            }),
            catalogue: Some(catalogue),
            pending: Vec::new(),
        };
        if state == "offline" {
            saved.change(Change::Equip {
                slot: "head".into(),
                item: None,
            });
        }
        // A page switch reads accounts.json again, which doesn't hold this one: switch first.
        self.go(Page::Wardrobe);
        self.accounts.accounts.push(account.clone());
        self.owner = Some(true);
        self.wardrobe = Some(Wardrobe {
            uuid: account.uuid.clone(),
            saved,
            items: items
                .into_iter()
                .map(|i| (i.id.clone(), Arc::new(i)))
                .collect(),
            session: (!empty).then(|| Arc::new(Session::preview(&account.uuid, &account.name))),
            skin: None,
            status: match state.as_str() {
                "offline" => Status::Offline,
                "failed" => {
                    Status::Failed("cosmetics.playhorizon.dev · 503 · 2026-11-02 21:40".into())
                }
                _ => Status::Ready,
            },
            busy: false,
            slot: "head",
            turn: 0,
            seen: None,
            toast: None,
        });
        self.wardrobe_reload = false;
        self.restyle = true;
    }

    /// The account's skin wearing what it wears now, baked once per change.
    fn wardrobe_picture(&mut self) -> Option<Arc<RenderImage>> {
        let scale = self.scale;
        let w = self.wardrobe.as_mut()?;
        let me = w.saved.me.as_ref()?;
        let worn: Vec<String> = me.equipped.values().cloned().collect();
        if let Some((s, turn, seen, image)) = &w.seen {
            if *s == scale && *turn == w.turn && *seen == worn {
                return Some(image.clone());
            }
        }
        let fallback = plain();
        let (texture, slim) = match &w.skin {
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
                .filter_map(|id| w.items.get(id))
                .filter_map(|item| preview::Draw::item(item, &body, 0.0)),
        );
        let (pw, ph) = ((W * scale) as u32, (H * scale) as u32);
        // Degrees: a little turned, then a quarter more per click.
        let yaw = 30. + 90. * f32::from(w.turn);
        let image = preview::render(&draws, pw, ph, yaw, 10.);
        let texels: Vec<[u8; 4]> = image.rgba.as_chunks::<4>().0.to_vec();
        let baked = art::bake(&texels, pw as usize, ph as usize, 1. / scale, scale);
        w.seen = Some((scale, w.turn, worn, baked.clone()));
        Some(baked)
    }

    /// A name tag style's mark, baked once.
    fn wardrobe_mark(&self, item: &Item) -> Arc<RenderImage> {
        let t = &item.texture;
        let texels: Vec<[u8; 4]> = t.rgba.as_chunks::<4>().0.to_vec();
        art::bake(&texels, t.width as usize, t.height as usize, 2., self.scale)
    }

    pub(super) fn wardrobe_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = page_header(
            "Wardrobe",
            "What you wear in every Horizon world and server. Changes reach other players the next \
             time they see you.",
            &t,
        );
        let page = div().flex().flex_col().gap(px(24.)).child(header);
        if self.wardrobe.is_none() && self.wardrobe_account().is_none() {
            let sign_in = button(
                "wardrobe-sign-in",
                "Sign in with Microsoft",
                Kind::Primary,
                &t,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Settings);
                this.start_sign_in();
                cx.notify();
            }));
            return page.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .p(px(24.))
                    .child(
                        text(theme::UI)
                            .text_color(t.ink)
                            .child(tr("Sign in to use cosmetics.")),
                    )
                    .child(sign_in),
            );
        }
        if std::mem::take(&mut self.wardrobe_reload) || self.wardrobe.is_none() {
            self.open_wardrobe(cx);
        }
        let picture = self.wardrobe_picture();
        let Some(w) = self.wardrobe.as_ref() else {
            return page;
        };
        let page = page.children(self.wardrobe_banner(w, cx));
        let Some(me) = w.saved.me.clone() else {
            // Nothing kept yet: the first answer is on its way.
            let skeleton = |h: f32| div().h(px(h)).w_full().bg(t.well);
            return page.child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .w(px(260.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .children((0..SLOTS.len()).map(|_| skeleton(52.))),
                    )
                    .child(div().flex_1().child(skeleton(H + 80.))),
            );
        };
        let lang = game_language();
        let entries = w
            .saved
            .catalogue
            .as_ref()
            .map(|c| c.items.clone())
            .unwrap_or_default();
        let name_of = |id: &str| {
            entries
                .iter()
                .find(|e| e.id == id)
                .map_or(id.to_owned(), |e| e.name(lang).to_owned())
        };
        // The slots, one row each.
        let mut slots = plate(&t)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .w(px(260.))
            .flex_none();
        for (i, (slot, label)) in SLOTS.into_iter().enumerate() {
            let owns = entries
                .iter()
                .any(|e| e.slot == slot && me.owned.contains(&e.id));
            let under = match me.equipped.get(slot) {
                Some(id) => name_of(id),
                None if owns => tr("Nothing equipped").into(),
                None => tr("None owned yet").into(),
            };
            let chosen = w.slot == slot;
            slots = slots.child(
                keyboard(div().id(("wardrobe-slot", i)), &t)
                    .flex()
                    .flex_col()
                    .px(px(12.))
                    .py(px(8.))
                    .cursor_pointer()
                    .when(chosen, |d| d.bg(t.well))
                    .child(text(theme::LABEL).text_color(t.ink).child(tr(label)))
                    .child(text(theme::CAPTION).text_color(t.muted).child(under))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(w) = &mut this.wardrobe {
                            w.slot = slot;
                        }
                        cx.notify();
                    })),
            );
        }
        // The player, with the name tag style over the head.
        let tag = me
            .equipped
            .get("name_tag")
            .and_then(|id| w.items.get(id))
            .filter(|i| i.tag.is_some());
        let line = tag
            .and_then(|i| i.tag)
            .map(|g| gpui::rgb(u32::from_be_bytes([0, g.line[0], g.line[1], g.line[2]])));
        let mark = tag.map(|i| self.wardrobe_mark(i));
        let name_tag = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(3.))
            .bg(gpui::rgba(0x0000_0066))
            .children(mark.map(|m| gpui::img(m).size(px(16.))))
            .child(
                text(theme::UI)
                    .text_color(gpui::white())
                    .child(me.name.clone()),
            )
            .when_some(line, |d, c| d.border_b_2().border_color(c));
        let turn = button("wardrobe-turn", "Turn", Kind::Ghost, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                if let Some(w) = &mut this.wardrobe {
                    w.turn = (w.turn + 1) % 4;
                }
                cx.notify();
            },
        ));
        let player = plate(&t)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .flex_1()
            .min_w(px(240.))
            .p(px(20.))
            .child(name_tag)
            .child(
                div()
                    .w(px(W))
                    .h(px(H))
                    .children(picture.map(|i| gpui::img(i).w(px(W)).h(px(H)))),
            )
            .child(turn);
        // What the account owns for the chosen slot, "Nothing" first.
        let slot = w.slot;
        let label = SLOTS.iter().find(|(s, _)| *s == slot).map_or("", |s| s.1);
        let founder = &me.claim.items;
        let wearing = me.equipped.get(slot).cloned();
        let mut cards = div().flex().flex_wrap().gap(px(8.));
        let card = |id: SharedString, title: String, under: String, on: bool| {
            keyboard(div().id(id), &t)
                .flex()
                .flex_col()
                .gap(px(2.))
                .w(px(136.))
                .p(px(12.))
                .cursor_pointer()
                .bg(t.well)
                .border_2()
                .border_color(if on { t.accent } else { t.outline })
                .child(text(theme::LABEL).text_color(t.ink).child(title))
                .child(text(theme::CAPTION).text_color(t.muted).child(under))
        };
        cards = cards.child(
            card(
                format!("wardrobe-none-{slot}").into(),
                tr("Nothing").into(),
                tr("Take it off").into(),
                wearing.is_none(),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.wardrobe_change(
                    Change::Equip {
                        slot: slot.into(),
                        item: None,
                    },
                    cx,
                );
                cx.notify();
            })),
        );
        for e in entries
            .iter()
            .filter(|e| e.slot == slot && me.owned.contains(&e.id))
        {
            let on = wearing.as_deref() == Some(e.id.as_str());
            let under = match (on, founder.contains(&e.id)) {
                (true, _) => tr("Equipped"),
                (false, true) => tr("Founder set"),
                (false, false) => tr("Yours"),
            };
            let id = e.id.clone();
            cards = cards.child(
                card(
                    format!("wardrobe-item-{}", e.id).into(),
                    e.name(lang).to_owned(),
                    under.into(),
                    on,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.wardrobe_change(
                        Change::Equip {
                            slot: slot.into(),
                            item: Some(id.clone()),
                        },
                        cx,
                    );
                    cx.notify();
                })),
            );
        }
        let owned = plate(&t)
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .w(px(312.))
            .flex_none()
            .child(text(theme::HEADING).text_color(t.ink).child(tr(label)))
            .child(cards);
        let page = page
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(slots)
                    .child(player)
                    .child(owned),
            )
            .child(self.wardrobe_switches(me.visible, cx));
        let toast = w
            .toast
            .as_ref()
            .filter(|(_, at)| at.elapsed() < TOAST)
            .map(|(s, _)| s.clone());
        page.children(toast.map(|s| {
            plate(&t)
                .px(px(16.))
                .py(px(10.))
                .child(text(theme::UI).text_color(t.ink).child(s))
        }))
    }

    /// Over the page: the founder set's claim, offline, a failure.
    fn wardrobe_banner(&self, w: &Wardrobe, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let row = |said: String| {
            plate(&t)
                .flex()
                .items_center()
                .gap(px(16.))
                .p(px(16.))
                .child(
                    text(theme::UI)
                        .flex_1()
                        .min_w_0()
                        .text_color(t.ink)
                        .child(said),
                )
        };
        match &w.status {
            Status::Offline => {
                let waiting = w.saved.pending.len();
                let mut said = tr(
                    "You're offline. You can still change what you wear: it saves on this \
                     computer and syncs when you're back online.",
                )
                .to_string();
                if waiting > 0 {
                    said = format!(
                        "{said} {}",
                        trf(
                            "Changes waiting to sync: {n}.",
                            &[("n", &waiting.to_string())]
                        )
                    );
                }
                return Some(row(said).child(self.wardrobe_retry(cx)));
            }
            Status::Failed(details) => {
                let copy = details.clone();
                return Some(
                    row(tr(
                        "The Wardrobe didn't load. What you wear still shows in the game: the \
                         launcher kept your last outfit.",
                    )
                    .into())
                    .child(self.wardrobe_retry(cx))
                    .child(
                        button("wardrobe-details", "Copy details", Kind::Ghost, &t).on_click(
                            move |_: &ClickEvent, _, cx| {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(copy.clone()))
                            },
                        ),
                    ),
                );
            }
            Status::Account(e) => return Some(row(e.clone()).child(self.wardrobe_retry(cx))),
            Status::Loading | Status::Ready => {}
        }
        let me = w.saved.me.as_ref()?;
        let claim = &me.claim;
        if claim.claimed == Some(true) {
            return None;
        }
        let until = claim.ends_at.map(day).unwrap_or_default();
        match claim.state {
            ClaimState::Open => {
                let said = if me.owned.is_empty() {
                    trf(
                        "Nothing in your wardrobe yet. The founder set is free to claim until {day}.",
                        &[("day", &until)],
                    )
                } else {
                    trf(
                        "The founder set is free to claim until {day}.",
                        &[("day", &until)],
                    )
                };
                let b = button("wardrobe-claim", "Claim the founder set", Kind::Primary, &t);
                Some(row(said).child(if w.busy || w.session.is_none() {
                    disabled(b)
                } else {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.wardrobe_claim(cx);
                        cx.notify();
                    }))
                }))
            }
            ClaimState::Closed => Some(row(trf(
                "The founder set was free to claim until {day}. It isn't offered any more; \
                 players who claimed it keep it.",
                &[("day", &until)],
            ))),
            ClaimState::NotYet => me
                .owned
                .is_empty()
                .then(|| row(tr("Nothing in your wardrobe yet.").into())),
        }
    }

    fn wardrobe_retry(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let b = button("wardrobe-retry", "Try again", Kind::Secondary, &self.theme);
        if self.wardrobe.as_ref().is_some_and(|w| w.busy) {
            return disabled(b);
        }
        b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.open_wardrobe(cx);
            cx.notify();
        }))
    }

    /// Show my cosmetics (the account's, on the service), and the game's two switches about
    /// other players.
    fn wardrobe_switches(&self, visible: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let home = backend::game_home();
        let prefs = Prefs::load(&home);
        let item = |id: &'static str, on: bool, title: &'static str, under: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_1()
                .items_center()
                .gap(px(12.))
                .cursor_pointer()
                .child(switch(format!("{id}-switch"), on, &t))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(text(theme::LABEL).text_color(t.ink).child(tr(title)))
                        .child(text(theme::CAPTION).text_color(t.muted).child(tr(under))),
                )
        };
        let save = |prefs: Prefs| {
            let home = home.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = prefs.save(&home);
                cx.refresh_windows();
            }
        };
        let hidden = prefs.others == Others::Hidden;
        plate(&t)
            .flex()
            .flex_wrap()
            .gap(px(24.))
            .p(px(16.))
            .child(
                item(
                    "wardrobe-mine",
                    visible,
                    "Show my cosmetics",
                    "Off: everyone sees you without them.",
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.wardrobe_change(Change::Visible(!visible), cx);
                    cx.notify();
                })),
            )
            .child(
                item(
                    "wardrobe-others",
                    hidden,
                    "Hide other players' cosmetics",
                    "Others look like plain players to you. Also in the game's Horizon settings.",
                )
                .on_click(save(Prefs {
                    others: if hidden {
                        Others::Shown
                    } else {
                        Others::Hidden
                    },
                    ..prefs
                })),
            )
            .child(
                item(
                    "wardrobe-auras",
                    prefs.auras,
                    "Show other players' auras and trails",
                    "Off by default. Also in the game's Horizon settings.",
                )
                .on_click(save(Prefs {
                    auras: !prefs.auras,
                    ..prefs
                })),
            )
    }
}
