//! Who plays: the top bar's quick switch (a menu under the account chip) and Settings' offline
//! names (`horizon_launcher::account::Accounts::offline`).
use super::servers_page::Field;
use super::*;
use horizon_launcher::account::Player;

impl Launcher {
    /// The chip's two lines: the name that plays, and how.
    pub(super) fn who_plays(&self) -> (String, &'static str) {
        match self.accounts.player() {
            Player::Microsoft(_) => (
                self.accounts
                    .selected()
                    .map_or_else(String::new, |a| a.name.clone()),
                tr("Microsoft account"),
            ),
            Player::Offline(name) => (name, tr("Offline")),
            Player::GameName => (
                self.offline_name.clone().unwrap_or_else(|| "Player".into()),
                tr("Offline"),
            ),
        }
    }

    pub(super) fn play_as(&mut self, who: Player) {
        let mut accounts = Accounts::load();
        let done = accounts
            .play_as_with(account::store(), who, self.settings.dev_offline)
            .and_then(|()| accounts.save());
        match done {
            Ok(()) => self.accounts = accounts,
            Err(e) => self.sign_in_state = Some(SignInState::Done(Err(e))),
        }
        self.account_menu = false;
    }

    /// Everyone the player can play as, with how each signs in.
    fn players(&self) -> Vec<(Player, String, &'static str)> {
        let mut out: Vec<_> = self
            .accounts
            .accounts
            .iter()
            .map(|a| {
                (
                    Player::Microsoft(a.uuid.clone()),
                    a.name.clone(),
                    tr("Microsoft"),
                )
            })
            .collect();
        out.extend(
            self.accounts
                .offline
                .iter()
                .map(|n| (Player::Offline(n.clone()), n.clone(), tr("Offline"))),
        );
        out.push((
            Player::GameName,
            self.offline_name.clone().unwrap_or_else(|| "Player".into()),
            tr("Offline, the game's name"),
        ));
        out
    }

    /// The menu under the account chip, while open.
    pub(super) fn account_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.account_menu {
            return None;
        }
        let t = self.theme;
        let now = self.accounts.player();
        let mut menu = div()
            .id("account-menu")
            .absolute()
            .top(px(46.))
            .right_0()
            .w(px(280.))
            .p(px(6.))
            .border_2()
            .border_color(t.outline)
            .bg(t.surface)
            .flex()
            .flex_col()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.account_menu = false;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }));
        for (i, (who, name, how)) in self.players().into_iter().enumerate() {
            let chosen = who == now;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("account-pick-{i}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .h(px(46.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft))
                    .child(self.profile_heads.draw(
                        match &who {
                            Player::Microsoft(uuid) => Some(uuid),
                            _ => None,
                        },
                        24.,
                        self.scale,
                        &t,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(text(theme::LABEL).truncate().text_color(t.ink).child(name))
                            .child(text(theme::CAPTION).text_color(t.muted).child(how)),
                    )
                    .when(chosen, |d| {
                        d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.play_as(who.clone());
                        cx.notify();
                    })),
            );
        }
        menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
            div()
                .id("account-manage")
                .flex()
                .items_center()
                .h(px(32.))
                .px(px(10.))
                .rounded(px(theme::RADIUS))
                .text_size(px(theme::LABEL.px))
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                .child(tr("Manage accounts"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.account_menu = false;
                    this.go(Page::Settings);
                    cx.notify();
                })),
        );
        Some(gpui::deferred(menu).with_priority(1))
    }

    fn add_offline_name(&mut self) {
        let name = self.servers.value(Field::OfflineName).trim().to_string();
        if name.is_empty() {
            return;
        }
        self.play_as(Player::Offline(name));
        if !matches!(self.sign_in_state, Some(SignInState::Done(Err(_)))) {
            self.servers.text(Field::OfflineName).set(String::new());
        }
    }

    /// Enter in the offline name field.
    pub(super) fn offline_name_enter(&mut self) {
        self.add_offline_name();
    }

    /// Settings' rows for the offline names, after the Microsoft accounts: each with Play as
    /// and Forget, then a field to add one.
    pub(super) fn offline_rows(&mut self, mut n: usize, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let mut rows = div().flex().flex_col();
        let now = self.accounts.player();
        for (i, name) in self.accounts.offline.clone().into_iter().enumerate() {
            let plays = now == Player::Offline(name.clone());
            let mut actions = div().flex().gap(px(4.));
            if !plays {
                let who = Player::Offline(name.clone());
                actions = actions.child(
                    button(format!("offline-play-{i}"), "Play as", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(who.clone());
                            cx.notify();
                        }),
                    ),
                );
            }
            let dressed = name.clone();
            actions = actions.child(
                button(format!("offline-skin-{i}"), "Skin…", Kind::Ghost, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_skin(dressed.clone());
                        cx.notify();
                    }),
                ),
            );
            let forget = name.clone();
            actions = actions.child(
                button(format!("offline-forget-{i}"), "Forget", Kind::Ghost, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let mut accounts = Accounts::load();
                        if accounts
                            .remove_offline(&forget, &horizon_launcher::skins::dir())
                            .is_ok()
                            && accounts.save().is_ok()
                        {
                            this.accounts = accounts;
                        }
                        cx.notify();
                    }),
                ),
            );
            n += 1;
            rows = rows.child(index_row(
                n,
                name,
                Some(
                    div()
                        .child(tr(if plays {
                            "Offline name · plays"
                        } else {
                            "Offline name"
                        }))
                        .into_any_element(),
                ),
                Some(actions.into_any_element()),
                &t,
            ));
        }
        rows = rows.children(self.skin_panel(cx));
        // The name the game made up can't be forgotten — it's what an offline player is —
        // only rolled for a new one.
        if let Some(made_up) = self.offline_name.clone() {
            let plays = now == Player::GameName;
            n += 1;
            rows = rows.child(index_row(
                n,
                made_up,
                Some(
                    div()
                        .child(tr(if plays {
                            "Offline name · the game's own · plays"
                        } else {
                            "Offline name · the game's own"
                        }))
                        .into_any_element(),
                ),
                Some(
                    button("game-name-new", "New name", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            match instances::new_player_name() {
                                Ok(name) => this.offline_name = Some(name),
                                Err(e) => this.sign_in_state = Some(SignInState::Done(Err(e))),
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
                &t,
            ));
        }
        n += 1;
        let add = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.field(Field::OfflineName, tr("A name, 3 to 16 letters"), 220., cx))
            .child(
                button("offline-add", "Add", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.add_offline_name();
                        cx.notify();
                    },
                )),
            );
        rows = rows.child(index_row(
            n,
            tr("Offline name"),
            Some(
                div()
                    .child(tr(
                        "Play offline under another name, and switch from the top bar. Online \
                         servers don't take offline names.",
                    ))
                    .into_any_element(),
            ),
            Some(add.into_any_element()),
            &t,
        ));
        (rows, n)
    }
}
