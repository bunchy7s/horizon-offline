//! Settings' AI rows: the No AI switch (`horizon_core::no_ai`, shared with the game) and "Connect
//! an AI agent", the ready-made MCP config of each common client with Copy buttons, a one-click
//! add where the client has a CLI (run only on that click, its command shown first), and a test
//! that starts the MCP server and lists its tools. With No AI on, only the switch shows.
use super::*;
use horizon_launcher::agents;

#[derive(Default)]
pub(crate) struct AgentsUi {
    /// The client shown, an index into `agents::clients`.
    client: usize,
    /// Which Copy button just copied.
    copied: Option<&'static str>,
    /// A command or test running.
    busy: bool,
    /// What the last add or test said, and whether it went well.
    status: Option<(bool, String)>,
}

impl Launcher {
    /// The No AI row: always shown, so it can be turned off again.
    pub(crate) fn no_ai_row(&mut self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = backend::no_ai();
        let toggle =
            switch("no-ai", on, &t).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.agents.status = backend::set_no_ai(!on).err().map(|e| (false, e));
                cx.notify();
            }));
        index_row(
            n,
            tr("No AI"),
            Some(
                div()
                    .child(tr(if on {
                        "On: every AI feature is hidden and off. The game opens no control socket, and horizon-ctl and the MCP servers refuse."
                    } else {
                        "Hides and turns off every AI feature: agent control of the launcher and the game, horizon-ctl and the MCP servers."
                    }))
                    .into_any_element(),
            ),
            Some(toggle.into_any_element()),
            &t,
        )
    }

    /// "Connect an AI agent"; None while No AI is on.
    pub(crate) fn agents_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if backend::no_ai() {
            return None;
        }
        let t = self.theme;
        let exe = &backend::update_target();
        let clients = agents::clients(exe);
        let pick = self.agents.client.min(clients.len() - 1);
        let client = &clients[pick];
        let this = cx.entity().downgrade();
        let options: Vec<(usize, &'static str)> = clients
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.name))
            .collect();
        let picker = segments("agent-client", &options, pick, &t, move |i, _, cx| {
            let _ = this.update(cx, |this, cx| {
                this.agents.client = i;
                this.agents.copied = None;
                this.agents.status = None;
                cx.notify();
            });
        });
        let copy = |id: &'static str, text: String, cx: &mut Context<Self>| {
            let copied = self.agents.copied == Some(id);
            button(
                id,
                if copied { "Copied" } else { "Copy" },
                Kind::Secondary,
                &t,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text.clone()));
                this.agents.copied = Some(id);
                cx.notify();
            }))
        };
        let code = |s: &str| {
            div()
                .p(px(10.))
                .bg(t.well)
                .border_2()
                .border_color(t.outline)
                .font_family(theme::MONO)
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(t.ink)
                .children(s.lines().map(|l| div().child(l.to_owned())))
        };
        let mut section = plate(&t)
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Connect an AI agent")),
            )
            .child(div().max_w(px(640.)).text_color(t.muted).child(tr(
                "Your own agent (Claude Code, Codex, Cursor or any MCP client) can drive the launcher and the game it starts, with the same commands as the terminal. It runs on your computer and talks to Horizon there; it asks before deleting anything.",
            )))
            .child(picker);
        if let Some((program, args)) = client.add.clone() {
            let line = client.add_line().unwrap_or_default();
            let label = trf("Add to {client}", &[("client", client.name)]);
            let busy = self.agents.busy;
            let add = button(
                "agent-add",
                if busy {
                    tr("Running…").to_owned()
                } else {
                    label
                },
                Kind::Primary,
                &t,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.agents.busy {
                    return;
                }
                this.agents.busy = true;
                this.agents.status = None;
                let (program, args) = (program, args.clone());
                let task = cx
                    .background_executor()
                    .spawn(async move { run_add(program, &args) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        this.agents.busy = false;
                        this.agents.status = Some(result);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            }));
            section = section
                .child(div().text_color(t.muted).child(trf(
                    "{client} adds it with this command (it runs only when you click):",
                    &[("client", client.name)],
                )))
                .child(code(&line))
                .child(div().flex().gap(px(8.)).child(add).child(copy(
                    "agent-copy-command",
                    line.clone(),
                    cx,
                )));
        }
        section = section
            .child(div().text_color(t.muted).child(trf(
                if client.add.is_some() {
                    "Or put this in {file}:"
                } else {
                    "Put this in {file}:"
                },
                &[("file", tr(client.file))],
            )))
            .child(code(&client.config))
            .child(div().flex().gap(px(8.)).child(copy("agent-copy-config", client.config.clone(), cx)).child(
                button("agent-test", "Test the connection", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        if this.agents.busy {
                            return;
                        }
                        this.agents.busy = true;
                        this.agents.status = None;
                        let exe = backend::update_target();
                        let task = cx.background_executor().spawn(async move {
                            match agents::self_test(&exe) {
                                Ok(n) => (true, trf("The MCP server answered with {count} tools. Now ask your agent: \"Say hello from Horizon\".", &[("count", &n.to_string())])),
                                Err(e) => (false, trf("The MCP server did not answer: {error}", &[("error", &e)])),
                            }
                        });
                        cx.spawn(async move |this, cx| {
                            let result = task.await;
                            let _ = this.update(cx, |this, cx| {
                                this.agents.busy = false;
                                this.agents.status = Some(result);
                                cx.notify();
                            });
                        })
                        .detach();
                        cx.notify();
                    },
                )),
            ));
        if let Some((ok, message)) = &self.agents.status {
            section = section.child(
                div()
                    .text_color(if *ok { t.ink } else { t.danger })
                    .child(message.clone()),
            );
        }
        Some(
            section.child(
                div()
                    .id("agent-docs")
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child(tr("What an agent can do, and the commands"))
                    .on_click(|_, _, cx| cx.open_url(backend::AGENT_DOCS_URL)),
            ),
        )
    }
}

/// Runs a client's add command; (went well, what it said).
fn run_add(program: &str, args: &[String]) -> (bool, String) {
    match horizon_launcher::process::command(agents::locate(program))
        .args(args)
        .output()
    {
        Ok(out) if out.status.success() => (
            true,
            trf(
                "Added. Restart {program} if it is open, then ask it: \"Say hello from Horizon\".",
                &[("program", program)],
            ),
        ),
        Ok(out) => (
            false,
            trf(
                "{program} said: {output}",
                &[
                    ("program", program),
                    (
                        "output",
                        String::from_utf8_lossy(if out.stderr.is_empty() {
                            &out.stdout
                        } else {
                            &out.stderr
                        })
                        .trim(),
                    ),
                ],
            ),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            false,
            trf(
                "{program} is not installed here (or not on the PATH): copy the config instead.",
                &[("program", program)],
            ),
        ),
        Err(e) => (
            false,
            trf(
                "Could not run {program}: {error}",
                &[("program", program), ("error", &e.to_string())],
            ),
        ),
    }
}
