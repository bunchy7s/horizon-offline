//! The Servers page's "Friends over the internet" panel: Tailscale on this computer, the address friends
//! type, inviting a friend to this one computer, and keeping shared friends to the server's port
//! (`horizon_launcher::tailscale`). Signing in is always the player's, in Tailscale's own app or page;
//! the launcher reads `tailscale status` and, with the player's own access token, calls the API.
use super::servers_page::Field;
use super::*;
use horizon_launcher::tailscale::{self, Api, Invite, Reach, State};
use serde_json::Value;

/// How often Tailscale's state is read while the panel shows.
const POLL: Duration = Duration::from_secs(10);
/// How often the invitations and access controls are read again while the panel shows, so an
/// accepted invitation appears without a click.
const RELOAD: Duration = Duration::from_secs(30);

pub(super) enum Event {
    State(State),
    Token(Option<String>),
    Invites(Result<Vec<Invite>, String>),
    Policy(Result<(Value, Option<String>), String>),
    /// An action finished: what to tell the player, and whether to read invites and policy again.
    Done(Result<String, String>),
    Invited(Result<Invite, String>),
}

pub(super) struct Internet {
    state: Option<State>,
    reading: bool,
    read: Option<Instant>,
    /// The access token, once read from the keychain (None: not read yet; Some(None): none).
    token: Option<Option<String>>,
    asking_token: bool,
    invites: Option<Result<Vec<Invite>, String>>,
    policy: Option<Result<(Value, Option<String>), String>>,
    loaded: Option<Instant>,
    busy: bool,
    message: Option<Result<String, String>>,
    /// The invitation just made, its link shown to copy.
    fresh: Option<Invite>,
    confirm_limit: bool,
    copied: Option<(String, Instant)>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Internet {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            state: None,
            reading: false,
            read: None,
            token: None,
            asking_token: false,
            invites: None,
            policy: None,
            loaded: None,
            busy: false,
            message: None,
            fresh: None,
            confirm_limit: false,
            copied: None,
            tx,
            rx,
        }
    }

    fn node(&self) -> Option<&tailscale::Node> {
        match &self.state {
            Some(State::Running(n)) => Some(n),
            _ => None,
        }
    }

    fn token(&self) -> Option<String> {
        self.token.clone().flatten()
    }
}

impl Launcher {
    /// What the background sent, and the next look at Tailscale while the panel shows.
    pub(super) fn internet_tick(&mut self, showing: bool) -> bool {
        let mut changed = false;
        while let Ok(event) = self.internet.rx.try_recv() {
            changed = true;
            self.internet_event(event);
        }
        let i = &mut self.internet;
        if showing && !i.reading && i.read.is_none_or(|at| at.elapsed() >= POLL) {
            self.internet_refresh();
        }
        changed
    }

    fn internet_refresh(&mut self) {
        let i = &mut self.internet;
        i.reading = true;
        i.read = Some(Instant::now());
        let tx = i.tx.clone();
        let want_token = i.token.is_none();
        std::thread::spawn(move || {
            let _ = tx.send(Event::State(tailscale::status()));
            if want_token {
                let token = account::keychain().get(tailscale::TOKEN_KEY).ok().flatten();
                let _ = tx.send(Event::Token(token));
            }
        });
    }

    fn internet_event(&mut self, event: Event) {
        let i = &mut self.internet;
        match event {
            Event::State(state) => {
                i.reading = false;
                let was_running = i.node().map(|n| n.id.clone());
                i.state = Some(state);
                let now = i.node().map(|n| n.id.clone());
                let stale = i.loaded.is_none_or(|at| at.elapsed() >= RELOAD);
                if now.is_some() && (now != was_running || (stale && !i.busy)) {
                    self.internet_load();
                }
            }
            Event::Token(token) => {
                i.token = Some(token);
                self.internet_load();
            }
            Event::Invites(r) => i.invites = Some(r),
            Event::Policy(r) => i.policy = Some(r),
            Event::Done(r) => {
                i.busy = false;
                i.message = Some(r);
                self.internet_load();
            }
            Event::Invited(r) => {
                i.busy = false;
                match r {
                    Ok(invite) => {
                        i.message = None;
                        i.fresh = Some(invite);
                        self.servers.text(Field::Email).set(String::new());
                    }
                    Err(e) => i.message = Some(Err(e)),
                }
                self.internet_load();
            }
        }
    }

    /// Reads the invitations and the access controls (needs the token and a running Tailscale).
    fn internet_load(&mut self) {
        let i = &mut self.internet;
        let (Some(token), Some(node)) = (i.token(), i.node().cloned()) else {
            return;
        };
        i.loaded = Some(Instant::now());
        let tx = i.tx.clone();
        std::thread::spawn(move || {
            let api = Api::new(&token);
            let _ = tx.send(Event::Invites(api.invites(&node.id)));
            let _ = tx.send(Event::Policy(api.policy()));
        });
    }

    pub(super) fn save_token(&mut self) {
        let token = self.servers.value(Field::Token).trim().to_string();
        if !token.starts_with("tskey-") {
            self.internet.message = Some(Err(
                "Paste the access token Tailscale showed (it begins with tskey-).".into(),
            ));
            return;
        }
        self.servers.text(Field::Token).set(String::new());
        self.internet.asking_token = false;
        self.internet.message = match account::keychain().set(tailscale::TOKEN_KEY, &token) {
            Ok(()) if account::keychain().persistent() => None,
            Ok(()) => Some(Ok(
                "No keychain here: the token is kept until the launcher closes.".into(),
            )),
            Err(e) => Some(Err(e)),
        };
        self.internet.token = Some(Some(token));
        self.internet_load();
    }

    fn forget_token(&mut self) {
        let _ = account::keychain().delete(tailscale::TOKEN_KEY);
        let i = &mut self.internet;
        i.token = Some(None);
        i.invites = None;
        i.policy = None;
        i.fresh = None;
        i.message = Some(Ok(
            "The access token is forgotten. Delete it in Tailscale's admin \
                             console too (Settings, Keys) if you won't use it again."
                .into(),
        ));
    }

    pub(super) fn invite_friend(&mut self) {
        let i = &mut self.internet;
        let (Some(token), Some(node)) = (i.token(), i.node().cloned()) else {
            return;
        };
        if i.busy {
            return;
        }
        let email = self.servers.value(Field::Email).trim().to_string();
        if !email.is_empty() && !email.contains('@') {
            i.message = Some(Err("That isn't an email address.".into()));
            return;
        }
        i.busy = true;
        i.fresh = None;
        let tx = i.tx.clone();
        std::thread::spawn(move || {
            let email = Some(email.as_str()).filter(|e| !e.is_empty());
            let _ = tx.send(Event::Invited(Api::new(&token).invite(&node.id, email)));
        });
    }

    fn internet_action(
        &mut self,
        job: impl FnOnce(&Api) -> Result<String, String> + Send + 'static,
    ) {
        let i = &mut self.internet;
        let Some(token) = i.token() else {
            return;
        };
        if i.busy {
            return;
        }
        i.busy = true;
        i.message = None;
        let tx = i.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Done(job(&Api::new(&token))));
        });
    }

    /// Applies `change` to the access controls read at `etag`, checked by Tailscale first.
    fn change_policy(&mut self, new: Value, etag: Option<String>, said: &'static str) {
        self.internet.confirm_limit = false;
        self.internet_action(move |api| {
            api.validate(&new)?;
            api.set_policy(&new, etag.as_deref())?;
            Ok(said.to_string())
        });
    }

    fn copy_button(
        &self,
        id: &'static str,
        text: String,
        kind: Kind,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let copied = self
            .internet
            .copied
            .as_ref()
            .is_some_and(|(what, at)| what == &text && at.elapsed() < Duration::from_secs(2));
        button(
            id,
            if copied { "Copied" } else { "Copy" },
            kind,
            &self.theme,
        )
        .bg(self.theme.plate)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text.clone()));
            this.internet.copied = Some((text.clone(), Instant::now()));
            cx.notify();
        }))
    }

    pub(super) fn internet_panel(&mut self, port: u16, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut panel = plate(&t)
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .child(caps(tr("Friends over the internet · Tailscale")).text_color(t.muted));
        let line = |s: String| text(theme::CAPTION).text_color(t.muted).child(s);
        let check = button("ts-check", "Check again", Kind::Ghost, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.internet_refresh();
                cx.notify();
            },
        ));
        match self.internet.state.clone() {
            None => return panel.child(line(tr("Looking for Tailscale…").into())),
            Some(State::Missing) => {
                return panel
                    .child(text(theme::UI).text_color(t.ink).child(tr(
                        "Tailscale lets friends anywhere join without opening your router to the internet. It's free: install it, sign in, then invite them here.",
                    )))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button("ts-install", "Install Tailscale", Kind::Secondary, &t)
                                    .bg(t.plate)
                                    .on_click(|_, _, cx| cx.open_url(tailscale::install_url())),
                            )
                            .child(check),
                    );
            }
            Some(State::Off(why)) => {
                let open = button("ts-open", "Open Tailscale", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Err(e) = tailscale::open_app() {
                            this.internet.message = Some(Err(e));
                        }
                        this.internet.read = None;
                        cx.notify();
                    }));
                panel = panel
                    .child(text(theme::UI).text_color(t.ink).child(tr(
                        "Tailscale is installed but not running. Open it and switch it on.",
                    )))
                    .when(!why.is_empty(), |d| d.child(line(why)))
                    .child(div().flex().gap(px(8.)).child(open).child(check));
                return self.internet_message(panel);
            }
            Some(State::SignedOut { auth_url }) => {
                let sign_in = button("ts-login", "Sign in to Tailscale", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        match &auth_url {
                            Some(url) => cx.open_url(url),
                            None => {
                                if let Err(e) = tailscale::open_app() {
                                    this.internet.message = Some(Err(e));
                                }
                            }
                        }
                        this.internet.read = None;
                        cx.notify();
                    }));
                panel = panel
                    .child(text(theme::UI).text_color(t.ink).child(tr(
                        "Tailscale is installed. Sign in with your own account in Tailscale (Horizon never sees it), then come back here.",
                    )))
                    .when(cfg!(target_os = "linux"), |d| {
                        d.child(line(trf(
                            "On Linux, run `{cmd}` in a terminal and open the link it prints.",
                            &[("cmd", tailscale::LINUX_LOGIN)],
                        )))
                    })
                    .child(div().flex().gap(px(8.)).child(sign_in).child(check));
                return self.internet_message(panel);
            }
            Some(State::Running(_)) => {}
        }
        let node = self.internet.node().cloned().unwrap_or_default();
        let address = node.address(port);
        let ip = node.ipv4().unwrap_or_default().to_string();
        let mut where_ = div().flex().flex_col().gap(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    text(theme::TITLE)
                        .font_family(theme::MONO)
                        .text_color(t.ink)
                        .child(address.clone()),
                )
                .child(self.copy_button("ts-copy", address, Kind::Secondary, cx)),
        );
        if node.magic_dns && !ip.is_empty() {
            let ip_port = if port == 25565 {
                ip.clone()
            } else {
                format!("{ip}:{port}")
            };
            where_ = where_.child(line(trf(
                "Or {address}. Friends type it in Multiplayer, Add Server, once this computer is shared with them and Tailscale runs on theirs.",
                &[("address", &ip_port)],
            )));
        } else {
            where_ = where_.child(line(
                tr("Friends type it in Multiplayer, Add Server, once this computer is shared with them and Tailscale runs on theirs.")
                    .into(),
            ));
        }
        panel = panel.child(where_);
        panel = panel.child(
            div()
                .border_t_1()
                .border_color(t.rule)
                .pt(px(12.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    text(theme::UI)
                        .text_color(t.ink)
                        .child(tr("Invite a friend")),
                )
                .child(self.invite_section(&ip, port, cx)),
        );
        panel = panel.child(line(
            tr("Horizon never turns on Funnel: the server isn't put on the public internet, only shared with the people you invite.")
                .into(),
        ));
        self.internet_message(panel)
    }

    fn internet_message(&self, panel: Div) -> Div {
        let t = self.theme;
        match &self.internet.message {
            Some(Ok(m)) => {
                panel.child(text(theme::CAPTION).text_color(t.ink_soft).child(m.clone()))
            }
            Some(Err(e)) => panel.child(text(theme::CAPTION).text_color(t.danger).child(e.clone())),
            None => panel,
        }
    }

    fn invite_section(&mut self, ip: &str, port: u16, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let line = |s: String| text(theme::CAPTION).text_color(t.muted).child(s);
        let link = |id: &'static str, label: &'static str, url: &'static str| {
            button(id, label, Kind::Ghost, &t).on_click(move |_, _, cx| cx.open_url(url))
        };
        let token = self.internet.token();
        if token.is_none() {
            let mut d = div().flex().flex_col().gap(px(10.)).child(line(
                tr("Share this computer with a friend: they get it in their own Tailscale, and nothing else of yours. Do it in Tailscale's admin console (Machines, this computer, Share), or here with an access token you make there.")
                    .into(),
            ));
            let machines = link(
                "ts-console",
                "Open the admin console",
                "https://login.tailscale.com/admin/machines",
            );
            if self.internet.asking_token {
                let save = button("ts-token-save", "Save", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_token();
                        cx.notify();
                    }));
                d = d
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(self.field(Field::Token, "tskey-api-…", 300., cx))
                            .child(save)
                            .child(link("ts-keys", "Make a token", tailscale::KEYS)),
                    )
                    .child(line(
                        tr("Settings, Keys, Generate access token. It's kept in this computer's keychain and only sent to Tailscale.")
                            .into(),
                    ));
            } else {
                d = d.child(div().flex().gap(px(8.)).child(machines).child(
                    button("ts-token", "Use an access token…", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.internet.asking_token = true;
                            this.internet.message = None;
                            cx.notify();
                        }),
                    ),
                ));
            }
            return d;
        }

        // Invite.
        let busy = self.internet.busy;
        let invite = button("ts-invite", "Make an invitation", Kind::Secondary, &t).bg(t.plate);
        let invite = if busy {
            disabled(invite)
        } else {
            invite.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.invite_friend();
                cx.notify();
            }))
        };
        let mut d = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.field(Field::Email, "Their email (optional)", 260., cx))
                    .child(invite),
            )
            .child(line(
                tr("One invitation per friend. With an email Tailscale sends it; without, send them the link. They accept it in their own Tailscale account.")
                    .into(),
            ));
        if let Some(fresh) = self.internet.fresh.clone() {
            d = d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.ink)
                            .truncate()
                            .child(fresh.url.clone()),
                    )
                    .child(self.copy_button("ts-fresh-copy", fresh.url, Kind::Secondary, cx)),
            );
        }
        match self.internet.invites.clone() {
            Some(Ok(list)) if !list.is_empty() => {
                let mut rows = div().flex().flex_col().border_t_1().border_color(t.rule);
                for (n, inv) in list.into_iter().enumerate() {
                    let who = match (&inv.accepted_by, &inv.email) {
                        (Some(by), _) => format!("Shared with {by}"),
                        (None, Some(email)) => format!("Sent to {email}, not accepted yet"),
                        (None, None) => "Link not used yet".to_string(),
                    };
                    let id = inv.id.clone();
                    let withdraw = button(
                        SharedString::from(format!("ts-revoke-{n}")),
                        if inv.accepted {
                            "Stop sharing"
                        } else {
                            "Withdraw"
                        },
                        Kind::Ghost,
                        &t,
                    );
                    let withdraw = if busy {
                        disabled(withdraw)
                    } else {
                        withdraw.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let id = id.clone();
                            this.internet.fresh = None;
                            this.internet_action(move |api| {
                                api.revoke(&id)?;
                                Ok("Withdrawn: that friend can't reach this computer any more."
                                    .into())
                            });
                            cx.notify();
                        }))
                    };
                    let date = inv.created.get(..10).unwrap_or_default().to_string();
                    rows = rows.child(index_row(
                        n + 1,
                        who,
                        Some(format!("Made {date}").into_any_element()),
                        Some(withdraw.into_any_element()),
                        &t,
                    ));
                }
                d = d.child(rows);
            }
            Some(Err(e)) => d = d.child(text(theme::CAPTION).text_color(t.danger).child(e)),
            _ => {}
        }

        // What shared friends reach.
        let reach = match &self.internet.policy {
            Some(Ok((policy, etag))) => Some((
                tailscale::reach(policy, ip, port),
                policy.clone(),
                etag.clone(),
            )),
            Some(Err(e)) => {
                d = d.child(line(format!("Couldn't read your access controls: {e}")));
                None
            }
            None => None,
        };
        if let Some((reach, policy, etag)) = reach {
            let mut safety = div().flex().flex_col().gap(px(8.)).pt(px(4.));
            match reach {
                Reach::Everything => {
                    safety = safety.child(text(theme::UI).text_color(t.ink).child(
                        "Friends you share this computer with can reach all of it, not only \
                         Minecraft (Tailscale's default).",
                    ));
                    if self.internet.confirm_limit {
                        let new = tailscale::limit_shared(&policy, ip, port);
                        safety = safety
                            .child(line(format!(
                                "This changes your tailnet's access controls: the rule that lets \
                                 everyone reach everything becomes one for your own devices and \
                                 members, and friends you share with reach only {ip}:{port}."
                            )))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(
                                        button("ts-limit-go", "Change the access controls", Kind::Primary, &t)
                                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                                if let Some(new) = new.clone() {
                                                    this.change_policy(
                                                        new,
                                                        etag.clone(),
                                                        "Done: friends you share this computer with reach only the Minecraft server.",
                                                    );
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(button("ts-limit-cancel", "Cancel", Kind::Ghost, &t).on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.internet.confirm_limit = false;
                                            cx.notify();
                                        }),
                                    )),
                            );
                    } else {
                        let b = button("ts-limit", "Limit them to Minecraft", Kind::Secondary, &t)
                            .bg(t.plate);
                        safety = safety.child(div().flex().child(if busy {
                            disabled(b)
                        } else {
                            b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.internet.confirm_limit = true;
                                cx.notify();
                            }))
                        }));
                    }
                }
                Reach::ServerOnly => {
                    let undo = tailscale::unlimit_shared(&policy, ip, port);
                    let b = button("ts-unlimit", "Remove the limit", Kind::Ghost, &t);
                    safety = safety.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(div().flex_none().size(px(6.)).rounded_full().bg(t.success))
                            .child(
                                text(theme::UI)
                                    .flex_1()
                                    .text_color(t.ink)
                                    .child(format!(
                                        "Friends you share with reach only the Minecraft server (port {port})."
                                    )),
                            )
                            .child(if busy {
                                disabled(b)
                            } else {
                                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if let Some(undo) = undo.clone() {
                                        this.change_policy(
                                            undo,
                                            etag.clone(),
                                            "Tailscale's default access controls are back.",
                                        );
                                    }
                                    cx.notify();
                                }))
                            }),
                    );
                }
                Reach::Custom => {
                    let snippet = tailscale::snippet(ip, port);
                    safety = safety
                        .child(text(theme::UI).text_color(t.ink).child(
                            "Your access controls are your own, so Horizon leaves them alone. To \
                             keep shared friends to Minecraft, add this rule in the admin console:",
                        ))
                        .child(
                            div()
                                .p(px(12.))
                                .rounded(px(theme::RADIUS))
                                .bg(t.plate)
                                .font_family(theme::MONO)
                                .text_size(px(theme::CAPTION.px))
                                .text_color(t.ink_soft)
                                .child(snippet.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(self.copy_button(
                                    "ts-snippet-copy",
                                    snippet,
                                    Kind::Secondary,
                                    cx,
                                ))
                                .child(link("ts-acls", "Open access controls", tailscale::ACCESS)),
                        );
                }
            }
            d = d.child(safety);
        }
        d.child(div().flex().child(
            button("ts-forget", tr("Forget the access token"), Kind::Ghost, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.forget_token();
                    cx.notify();
                }),
            ),
        ))
    }
}
