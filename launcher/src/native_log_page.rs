//! The top bar's Build log: the native-mods build's own log (`native_mods::log_path`), read on
//! a background thread when the panel opens or Refresh is pressed, never per frame. Copy log
//! copies the whole file; Open full log and Show in folder point at the file itself, and only
//! a bounded tail of it is shown, with a notice when there is more. Each build attempt starts
//! its own log before its preflight can fail (`native_log_reset`, `native_log_preflight`), so
//! the panel never shows the last attempt as the latest, and an open panel reads again when a
//! build starts or ends.
use super::*;
use gpui::{ClipboardItem, ScrollHandle, SharedString};
use horizon_launcher::native_mods;

/// How much of the log the panel shows: its last lines. Copy log still copies all of it.
const SHOWN: usize = LOG_LINES;

/// The complete copyable snapshot and the bounded tail shown in the panel.
pub(super) struct Log {
    /// The whole file as it was read.
    full: String,
    /// How many lines the whole file has.
    lines: usize,
    /// Its last [`SHOWN`] lines, as the panel lays them out without copying per frame.
    preview: Vec<SharedString>,
}

/// What the panel shows in place of the log.
pub(super) enum Missing {
    /// Nothing was ever built (or the log was deleted).
    None,
    /// It could not be read, or could not be reset for this attempt.
    Failed(String),
}

pub(super) struct NativeLog {
    /// Whether the panel is open over the window.
    pub(super) open: bool,
    /// A read is wanted: the panel opened, or a build attempt began or ended. `drain` picks it
    /// up between frames, so `render` never reads the file.
    pub(super) stale: bool,
    /// Whether a log exists at all, for the top bar's Build log button (checked in `drain`).
    pub(super) available: bool,
    /// A read is under way.
    pub(super) reading: bool,
    /// The log as it was last read: `None` while the first read of this opening is under way.
    pub(super) loaded: Option<Result<Log, Missing>>,
    /// Why this attempt's log could not be reset, shown in place of the last attempt's file.
    pub(super) reset: Option<String>,
    /// The panel's scroll, kept to land on the newest output.
    pub(super) scroll: ScrollHandle,
    /// What the last Copy log did.
    pub(super) note: Option<String>,
}

impl Default for NativeLog {
    fn default() -> Self {
        NativeLog {
            open: false,
            stale: false,
            available: native_mods::log_path().is_file(),
            reading: false,
            loaded: None,
            reset: None,
            scroll: ScrollHandle::new(),
            note: None,
        }
    }
}

impl Log {
    fn read(path: &std::path::Path) -> Result<Self, Missing> {
        match std::fs::read_to_string(path) {
            Ok(full) => {
                let lines = full.lines().count();
                let preview = full
                    .lines()
                    .skip(lines.saturating_sub(SHOWN))
                    .map(SharedString::from)
                    .collect();
                Ok(Log {
                    full,
                    lines,
                    preview,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Missing::None),
            Err(e) => Err(Missing::Failed(e.to_string())),
        }
    }
}

impl NativeLog {
    fn begin_read(&mut self) -> bool {
        if !self.open || !self.stale || self.reading {
            return false;
        }
        self.stale = false;
        self.reading = true;
        self.loaded = None;
        self.note = None;
        true
    }

    fn finish_read(&mut self, result: Result<Log, Missing>) {
        self.reading = false;
        // A refresh or new attempt queued during this read supersedes its result.
        if !self.stale {
            self.loaded = Some(result);
        }
    }
}

impl Launcher {
    /// The top bar's Build log opens the panel; its button shows only while a log exists.
    pub(super) fn native_log_open(&mut self) {
        self.native_log.open = true;
        self.native_log.note = None;
        self.native_log.stale = true;
    }

    /// Escape closes the panel. Whether it was open.
    pub(super) fn native_log_escape(&mut self) -> bool {
        self.native_log.note = None;
        std::mem::take(&mut self.native_log.open)
    }

    /// Begins a build attempt's own log, before anything of it can fail: the last attempt's
    /// file is emptied, or the reason it could not be is shown in its place. An open panel
    /// reads again rather than keep showing the last attempt as the latest.
    pub(super) fn native_log_reset(&mut self) {
        self.native_log.reset = native_mods::clear_log().err();
        self.native_log.available = true;
        self.native_log.loaded = None;
        self.native_log.note = None;
        self.native_log.stale = true;
    }

    /// Records a failure from before the build itself started (no Horizon to build on, no mods
    /// to build), where the build's own output would have gone.
    pub(super) fn native_log_preflight(&mut self, error: &str) {
        if self.native_log.reset.is_none() {
            self.native_log.reset = native_mods::append_log(error).err();
        }
        self.native_log.stale = true;
    }

    /// Reads the log again on a background thread, for an open panel: the newest output lands
    /// in view. A read already under way is not restarted; the next `drain` picks this up.
    pub(super) fn native_log_read(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.native_log.begin_read() {
            return false;
        }
        let reset = self.native_log.reset.clone();
        let task = cx.background_executor().spawn(async move {
            if let Some(e) = reset {
                // This attempt never got a fresh log: its own error replaces the file.
                return Err(Missing::Failed(e));
            }
            Log::read(&native_mods::log_path())
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.native_log.finish_read(done);
                this.native_log.scroll.scroll_to_bottom();
                cx.notify();
            });
        })
        .detach();
        true
    }

    /// The Build log panel over the whole window, while it is open.
    pub(super) fn native_log_panel(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.native_log.open {
            return None;
        }
        let t = self.theme;
        let path = native_mods::log_path();
        // Rendering clones only the preview lines' shared text handles.
        let loaded = &self.native_log.loaded;
        let gone = self.native_log.reset.is_some() || matches!(loaded, Some(Err(Missing::None)));
        let open = path.clone();
        let reveal = path.clone();
        let copy = {
            let b = button("native-log-copy", "Copy log", Kind::Ghost, &t);
            match loaded {
                Some(Ok(_)) => b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    // The whole file as it was read, never a stale or missing one.
                    let Some(Ok(log)) = &this.native_log.loaded else {
                        return;
                    };
                    copy_to_clipboard(cx, ClipboardItem::new_string(log.full.as_str().to_string()));
                    this.native_log.note = Some(tr("Copied build log.").into());
                    cx.notify();
                })),
                _ => disabled(b),
            }
        };
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(copy)
            .child({
                let b = button("native-log-file", "Open full log", Kind::Ghost, &t);
                if gone {
                    disabled(b)
                } else {
                    b.on_click(move |_, _, cx| cx.open_with_system(&open))
                }
            })
            .child({
                let b = button("native-log-reveal", "Show in folder", Kind::Ghost, &t);
                if gone {
                    disabled(b)
                } else {
                    b.on_click(move |_, _, cx| cx.reveal_path(&reveal))
                }
            })
            .child(
                button("native-log-refresh", "Refresh", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.native_log.stale = true;
                        cx.notify();
                    },
                )),
            );
        if let Some(note) = &self.native_log.note {
            actions = actions.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .min_w_0()
                    .child(note.clone()),
            );
        }
        let top = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(17., FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .child(tr("Native build log")),
                    )
                    // The actual file the actions below work on, wrapping rather than cut off.
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.muted)
                            .min_w_0()
                            .line_height(px(18.))
                            .child(short(&path)),
                    ),
            )
            .child(
                button("native-log-close", "Close", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.native_log_escape();
                        cx.notify();
                    },
                )),
            );
        let body = match loaded {
            None => centered(tr("Loading build log…"), &t),
            Some(Err(Missing::None)) => centered(tr("No native build log is available yet."), &t),
            Some(Err(Missing::Failed(e))) => notice(
                trf(
                    "Could not read the build log: {error}",
                    &[("error", e.as_str())],
                ),
                &t,
            ),
            Some(Ok(log)) => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                // Only the tail is laid out; the whole log stays one Copy away.
                .children((log.lines > SHOWN).then(|| {
                    quiet_note(
                        trf(
                            "Only the last {lines} lines are shown. Copy or open the full \
                             log for all details.",
                            &[("lines", &SHOWN.to_string())],
                        ),
                        &t,
                    )
                }))
                .child(
                    div()
                        .id("native-log-lines")
                        .track_scroll(&self.native_log.scroll)
                        .flex_1()
                        .min_h_0()
                        .overflow_scroll()
                        .p(px(10.))
                        .border_2()
                        .border_color(t.outline)
                        .bg(t.well)
                        .font_family(theme::MONO)
                        .text_size(px(12.))
                        .children(
                            log.preview
                                .iter()
                                .map(|line| div().text_color(t.ink_soft).child(line.clone())),
                        ),
                ),
        };
        Some(
            div()
                .id("native-log")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .overflow_hidden()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(14.))
                .px(px(32.))
                .py(px(24.))
                .bg(t.page.opacity(0.96))
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .occlude()
                .child(top)
                .child(actions)
                .children(
                    self.native_step
                        .as_ref()
                        .map(|step| quiet_note(step.clone(), &t)),
                )
                .child(body.flex_1().min_h_0().min_w_0()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_complete_diagnostics_and_only_renders_the_tail() {
        let dir = std::env::temp_dir().join(format!("horizon-log-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("build.log");
        let full = "first build context\n".repeat(SHOWN + 5)
            + "error[E0308]: expected String, found &str\n";
        std::fs::write(&path, &full).unwrap();
        let log = Log::read(&path).unwrap_or_else(|_| panic!("read the diagnostic fixture"));
        assert_eq!(log.full, full);
        assert_eq!(log.lines, SHOWN + 6);
        assert_eq!(log.preview.len(), SHOWN);
        assert_eq!(
            log.preview.last().unwrap().as_ref(),
            "error[E0308]: expected String, found &str"
        );
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(Log::read(&path), Err(Missing::None)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_during_a_read_discards_old_output_and_retries() {
        let mut state = NativeLog {
            open: true,
            stale: true,
            ..Default::default()
        };
        assert!(state.begin_read());
        state.stale = true; // A new build, a refresh, or completion while I/O is pending.
        assert!(!state.begin_read());
        state.finish_read(Err(Missing::Failed("old build error".into())));
        assert!(state.loaded.is_none());
        assert!(state.begin_read());
        state.finish_read(Err(Missing::None));
        assert!(matches!(state.loaded, Some(Err(Missing::None))));
        state.open = false;
        state.stale = true;
        assert!(!state.begin_read());
        state.open = true;
        assert!(state.begin_read());
    }
}
