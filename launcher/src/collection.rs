//! Data collection in the window: the question asked once (and again when what is collected
//! changes), and its section in Settings. Everything goes through `horizon_launcher::telemetry`.
use super::*;
use horizon_launcher::telemetry;
use horizon_telemetry::{Choices, Consent, Kind as DataKind};

/// What the window keeps about data collection.
pub(crate) struct Collection {
    pub consent: Consent,
    body_scroll: Scrollbar,
    json_scroll: Scrollbar,
    /// The question's switches, until the player answers.
    pub draft: Choices,
    /// "View what would be sent": a line on what it is, and the JSON.
    pub view: Option<(String, String)>,
    /// What "Delete my data" did, or why it could not.
    pub note: Option<String>,
    pub confirm_delete: bool,
}

impl Collection {
    pub fn load() -> Self {
        Self {
            consent: telemetry::consent(),
            body_scroll: Scrollbar::default(),
            json_scroll: Scrollbar::default(),
            draft: Choices::NONE,
            view: None,
            note: None,
            confirm_delete: false,
        }
    }
}

fn kind_id(prefix: &str, kind: DataKind) -> &'static str {
    match (prefix, kind) {
        ("ask", DataKind::Perf) => "ask-perf",
        ("ask", DataKind::Crash) => "ask-crash",
        ("ask", DataKind::Bug) => "ask-bug",
        ("ask", DataKind::Benchmark) => "ask-benchmark",
        (_, DataKind::Perf) => "data-perf",
        (_, DataKind::Crash) => "data-crash",
        (_, DataKind::Bug) => "data-bug",
        (_, DataKind::Benchmark) => "data-benchmark",
    }
}

/// Where a switch would be: the ping has none.
fn always_on(t: &Theme) -> Div {
    caps("Always on").flex_none().text_color(t.muted)
}

/// The "I'm playing" ping on the first-run question, laid out as the switches' rows.
fn ping_row(t: &Theme) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(16.))
        .py(px(10.))
        .border_b_1()
        .border_color(t.rule)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    text(theme::UI)
                        .text_color(t.ink)
                        .child(telemetry::PING_NAME),
                )
                .child(
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(telemetry::PING),
                ),
        )
        .child(always_on(t))
}

/// The exact JSON, in mono, scrolling inside a plate.
fn json_view(view: &(String, String), t: &Theme, scroll: &Scrollbar) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            text(theme::CAPTION)
                .text_color(t.muted)
                .child(view.0.clone()),
        )
        .child(
            scroll.wrap(
                div()
                    .id("data-json")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .p(px(16.))
                    .rounded(px(theme::RADIUS_PLATE))
                    .bg(t.plate)
                    .font_family(theme::MONO)
                    .text_size(px(theme::CAPTION.px))
                    .line_height(px(18.))
                    .text_color(t.ink_soft)
                    .children(view.1.lines().map(|l| div().child(l.to_string()))),
                t,
            ),
        )
}

impl Launcher {
    /// Whether the question covers the window: not answered for this consent version, and the
    /// opening is over.
    pub(crate) fn asking_consent(&self) -> bool {
        self.opening.is_none() && self.collection.consent.needs_asking()
    }

    fn answer_consent(&mut self, choices: Choices) {
        match telemetry::answer(choices) {
            Ok(consent) => self.collection.consent = consent,
            Err(e) => self.collection.note = Some(format!("Could not save your choice: {e}")),
        }
        // The question and Settings say what the "I'm playing" ping holds, beside the switches.
        if !self.settings.ping_notice_seen {
            self.settings.ping_notice_seen = true;
            self.settings.save();
        }
        self.collection.view = None;
    }

    fn toggle_view(&mut self) {
        self.collection.view = match self.collection.view {
            Some(_) => None,
            None => Some(telemetry::preview()),
        };
    }

    /// The question, centred over the launcher. Everything starts off; "Send nothing" is one
    /// click, as easy as saying yes.
    pub(crate) fn consent_screen(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let draft = self.collection.draft;
        let rows = telemetry::SWITCHES.iter().map(|&(kind, name, what)| {
            let on = draft.allows(kind);
            div()
                .flex()
                .items_start()
                .gap(px(16.))
                .py(px(10.))
                .border_b_1()
                .border_color(t.rule)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(text(theme::UI).text_color(t.ink).child(name))
                        .child(text(theme::CAPTION).text_color(t.muted).child(what)),
                )
                .child(switch(kind_id("ask", kind), on, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.collection.draft.set(kind, !on);
                        cx.notify();
                    },
                )))
        });
        let viewing = self.collection.view.is_some();
        // The body scrolls inside the card, so the answers stay at its foot however short the
        // window and however long what would be sent.
        let body = div()
            .id("ask-body")
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(12.))
            .px(px(28.))
            .pt(px(28.))
            .pb(px(20.))
            .overflow_y_scroll()
            .child(caps("Data collection").text_color(t.muted))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child("Help make Horizon faster and steadier?"),
            )
            .child(text(theme::UI).text_color(t.muted).child(
                "Horizon can send a few things to its developers. Each switch is off until you turn it on, and Settings can change it any time.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(t.rule)
                    .children(rows)
                    .child(ping_row(&t)),
            )
            .child(text(theme::CAPTION).text_color(t.muted).child(telemetry::NEVER))
            .child(
                div()
                    .id("ask-view")
                    .text_size(px(theme::UI.px))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child(if viewing { "Hide what would be sent" } else { "View what would be sent" })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_view();
                        cx.notify();
                    })),
            )
            .children(self.collection.view.as_ref().map(|v| json_view(v, &t, &self.collection.json_scroll)));
        let answers = div()
            .flex()
            .flex_none()
            .justify_end()
            .gap(px(12.))
            .px(px(28.))
            .pb(px(28.))
            .child(
                button("ask-none", "Send nothing", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.answer_consent(Choices::NONE);
                        cx.notify();
                    },
                )),
            )
            .child(
                button("ask-save", "Save choices", Kind::Primary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.answer_consent(this.collection.draft);
                        cx.notify();
                    },
                )),
            );
        let card = plate(&t)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(680.))
            .max_h_full()
            .bg(t.page)
            .border_1()
            .border_color(t.rule)
            .child(self.collection.body_scroll.wrap(body, &t))
            .child(answers);
        div()
            .id("consent")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(32.))
            .py(px(24.))
            .bg(t.page.opacity(0.92))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .occlude()
            .child(card)
    }

    /// Settings' data collection: a switch per kind, then the player's data.
    pub(crate) fn collection_section(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let consent = self.collection.consent.clone();
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, &(kind, name, what)) in telemetry::SWITCHES.iter().enumerate() {
            let on = !consent.needs_asking() && consent.choices.allows(kind);
            let toggle = switch(kind_id("data", kind), on, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let mut choices = if this.collection.consent.needs_asking() {
                        Choices::NONE
                    } else {
                        this.collection.consent.choices
                    };
                    choices.set(kind, !on);
                    this.answer_consent(choices);
                    cx.notify();
                },
            ));
            list = list.child(index_row(
                i + 1,
                name,
                Some(div().child(what).into_any_element()),
                Some(toggle.into_any_element()),
                &t,
            ));
        }
        list = list.child(index_row(
            telemetry::SWITCHES.len() + 1,
            telemetry::PING_NAME,
            Some(div().child(telemetry::PING).into_any_element()),
            Some(always_on(&t).into_any_element()),
            &t,
        ));
        let waiting = horizon_telemetry::outbox::pending(&telemetry::home()).len();
        let id = match consent.install_id.as_str() {
            "" => "No install ID: nothing has been collected".to_string(),
            id => format!("Install ID {}…{}", &id[..4], &id[id.len() - 4..]),
        };
        let server = if horizon_telemetry::endpoint().is_some() {
            format!("{waiting} waiting to be sent")
        } else {
            "Sending is turned off here (HORIZON_TELEMETRY_URL=off)".into()
        };
        let confirm = self.collection.confirm_delete;
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(
                button(
                    "data-view",
                    if self.collection.view.is_some() {
                        "Hide"
                    } else {
                        "View what would be sent"
                    },
                    Kind::Ghost,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_view();
                    cx.notify();
                })),
            )
            .child(
                button(
                    "data-delete",
                    if confirm {
                        "Delete everything?"
                    } else {
                        "Delete my data"
                    },
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if this.collection.confirm_delete {
                        this.collection.note = Some(match telemetry::delete_my_data() {
                            Ok(done) => done,
                            Err(e) => e,
                        });
                        this.collection.consent = telemetry::consent();
                        this.collection.view = None;
                    }
                    this.collection.confirm_delete = !this.collection.confirm_delete;
                    cx.notify();
                })),
            );
        list = list.child(index_row(
            telemetry::SWITCHES.len() + 2,
            "Your data",
            Some(
                div()
                    .font_family(theme::MONO)
                    .child(format!("{id}  ·  {server}"))
                    .into_any_element(),
            ),
            Some(actions.into_any_element()),
            &t,
        ));
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text(theme::HEADING).text_color(t.ink).child("Data collection"))
            .child(text(theme::UI).max_w(px(600.)).text_color(t.muted).child(
                "Each switch is off unless you turn it on. What it sends is below it; the privacy page on playhorizon.dev says how long it is kept.",
            ))
            .child(list)
            .child(text(theme::CAPTION).text_color(t.muted).child(telemetry::NEVER))
            .children(self.collection.note.clone().map(|n| text(theme::UI).text_color(t.ink).child(n)))
            .children(self.collection.view.as_ref().map(|v| json_view(v, &t, &self.collection.json_scroll)))
    }
}
