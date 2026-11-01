//! Screen narration: what the narrator reads of the open screen and when, as
//! `Screen` does (`init`'s `triggerImmediateNarration`, `afterMouseMove`,
//! `afterMouseAction`, `afterKeyboardAction`, `updateNarratorStatus`,
//! `handleDelayedNarration`, `updateNarrationState`).
//!
//! Horizon's screens are drawn, not built from widget objects, so they record
//! their narration as they draw (`widgets::begin_screen`, `widgets::narratable`)
//! and [`ScreenNarrator::frame`] reads the last drawn frame's: a new screen id
//! is a screen opening (`init`), a new GUI size the same screen resized. The
//! input events the game hands it schedule the narration, and the frames run
//! it when due. The frames only record while the narrator can speak
//! (`shouldRunNarration`), whatever its status.
use crate::widgets::{self, ElementType, ListKind, Narratable, Narration, ScreenInfo, What};
use horizon_narrator::screen::{Output, Priority, ScreenNarration, Trigger, find_narratable};
use std::collections::HashMap;
use std::time::Instant;

/// `InputType`: what the player used last on a screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputType {
    #[default]
    None,
    Mouse,
    KeyboardArrow,
    KeyboardTab,
}

impl InputType {
    pub fn is_keyboard(self) -> bool {
        matches!(self, InputType::KeyboardArrow | InputType::KeyboardTab)
    }
}

/// A translation key and its arguments to text (`Component.translatable`).
pub type Translate<'a> = &'a dyn Fn(&str, &[&str]) -> String;

/// `Component.translatable(key, args)` in the current language.
pub fn translate(key: &str, args: &[&str]) -> String {
    let args: Vec<&dyn std::fmt::Display> =
        args.iter().map(|a| a as &dyn std::fmt::Display).collect();
    crate::lang::tr(key, &args)
}

/// The open screen's narration.
#[derive(Debug)]
struct Open {
    id: String,
    /// The GUI size it was laid out for: a new one is `init` again.
    gui: (f32, f32),
    state: ScreenNarration,
    /// Each container list row's `lastNarratable`, by list and row.
    row_last: HashMap<(u64, usize), u64>,
    /// `LevelLoadingScreen.lastNarration`.
    last_repeat: i64,
}

/// The screens' narration: the open screen's state, the player's last input
/// type (`Minecraft.lastInputType`) and the widget a click focused.
#[derive(Debug)]
pub struct ScreenNarrator {
    epoch: Instant,
    /// The clock tests set; `None` reads `epoch`.
    clock: Option<i64>,
    open: Option<Open>,
    input: InputType,
    /// `ContainerEventHandler.mouseClicked`'s `setFocused`: the widget last
    /// clicked keeps the focus until the screen changes.
    click_focus: Option<u64>,
    /// The last drawn frame's records.
    frame: Narration,
    /// `widgets::generation` when `frame` was read.
    generation: u64,
    /// The frames record their narration.
    recording: bool,
    /// The first frame was read: the narrator was there from the start.
    started: bool,
    /// The narrator came on while the game ran: the frame of this generation
    /// is the screen already open, which vanilla narrated silently all along.
    adopt: Option<u64>,
    /// `updateNarratorStatus` before that screen was read.
    pending_status: Option<(i64, bool, Trigger)>,
}

impl Default for ScreenNarrator {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            clock: None,
            open: None,
            input: InputType::None,
            click_focus: None,
            frame: Narration::default(),
            generation: 0,
            recording: false,
            started: false,
            adopt: None,
            pending_status: None,
        }
    }
}

/// The context `updateNarrationState` reads: the frame, the mouse, focus.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    frame: &'a Narration,
    info: &'a ScreenInfo,
    cursor: (f32, f32),
    click_focus: Option<u64>,
    /// The controller's focus ring (GUI pixels).
    pad_focus: Option<[f32; 4]>,
    input: InputType,
    /// `init`'s narration runs before the screen renders: nothing is hovered.
    init: bool,
    /// The frame's tooltip is the hovered widget's (HINT). Not inside a
    /// group, whose tooltips (a recipe's item) aren't its widgets'.
    tooltips: bool,
    tr: Translate<'a>,
}

fn contains([x, y, w, h]: [f32; 4], (cx, cy): (f32, f32)) -> bool {
    cx >= x && cy >= y && cx < x + w && cy < y + h
}

fn same_rect(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(p, q)| (p - q).abs() < 0.5)
}

fn num(n: usize) -> String {
    n.to_string()
}

impl Ctx<'_> {
    fn tr(&self, key: &str, args: &[&str]) -> String {
        (self.tr)(key, args)
    }
    /// The row widgets of the container list keyed `list`.
    fn children(&self, list: u64) -> impl Iterator<Item = &Narratable> {
        self.frame
            .items
            .iter()
            .filter(move |n| n.parent.is_some_and(|(l, _)| l == list))
    }
    /// `isFocused`: the drawer's focus, a click's, or the controller's ring.
    /// A container list is focused with one of its widgets.
    fn focused(&self, n: &Narratable) -> bool {
        n.focused
            || self.click_focus == Some(n.identity())
            || self.pad_focus.is_some_and(|r| same_rect(r, n.rect))
            || matches!(n.what, What::ContainerList { .. })
                && self.children(n.identity()).any(|c| self.focused(c))
    }
    /// `isHovered`; a list's is a row under the mouse.
    fn hovered(&self, n: &Narratable) -> bool {
        !self.init
            && match &n.what {
                What::List { hovered, .. } => hovered.is_some(),
                What::ContainerList { hovered_row, .. } => hovered_row.is_some(),
                _ => n.hovered.unwrap_or_else(|| contains(n.rect, self.cursor)),
            }
    }
    /// `narrationPriority`. A group's is HOVERED while it is visible.
    fn priority(&self, n: &Narratable) -> Priority {
        if let What::Group { visible } = n.what {
            return if visible {
                Priority::Hovered
            } else {
                Priority::None
            };
        }
        if self.focused(n) {
            Priority::Focused
        } else if self.hovered(n) {
            Priority::Hovered
        } else {
            Priority::None
        }
    }
}

/// `Screen.updateNarrationState`: the title (or the screen's own elements),
/// then the narrated widget.
fn update_narration_state(
    out: &mut Output<'_>,
    ctx: &Ctx<'_>,
    last: &mut Option<u64>,
    row_last: &mut HashMap<(u64, usize), u64>,
) {
    let info = ctx.info;
    let replaces_title =
        |e: &&(ElementType, u32, Vec<String>)| e.0 == ElementType::Title && e.1 == 0;
    match info.extra.iter().find(replaces_title) {
        Some((_, _, lines)) => out.add_lines(ElementType::Title, lines.clone()),
        None => out.add(ElementType::Title, info.title.clone()),
    }
    for (kind, depth, lines) in info.extra.iter().filter(|e| !replaces_title(e)) {
        add_at(out, *depth, *kind, lines.clone());
    }
    if info.widgets {
        update_narrated_widget(out, ctx, last, row_last);
    }
}

fn add_at(out: &mut Output<'_>, depth: u32, kind: ElementType, lines: Vec<String>) {
    if depth == 0 {
        out.add_lines(kind, lines);
    } else {
        add_at(&mut out.nest(), depth - 1, kind, lines);
    }
}

/// `Screen.updateNarratedWidget`: the active narratables in tab order, the
/// one `findNarratableWidget` picks with its position, or how to navigate.
fn update_narrated_widget(
    out: &mut Output<'_>,
    ctx: &Ctx<'_>,
    last: &mut Option<u64>,
    row_last: &mut HashMap<(u64, usize), u64>,
) {
    let mut active: Vec<&Narratable> = ctx
        .frame
        .items
        .iter()
        .filter(|n| n.parent.is_none() && n.is_active())
        .collect();
    active.sort_by_key(|n| n.order);
    let entries: Vec<(u64, Priority)> = active
        .iter()
        .map(|n| (n.identity(), ctx.priority(n)))
        .collect();
    match find_narratable(&entries, *last) {
        Some((i, priority)) => {
            if priority.is_terminal() {
                *last = Some(entries[i].0);
            }
            if active.len() > 1 {
                out.add(
                    ElementType::Position,
                    ctx.tr(
                        "narrator.position.screen",
                        &[&num(i + 1), &num(active.len())],
                    ),
                );
                if priority == Priority::Focused {
                    let usage = ctx.info.usage.clone();
                    out.add(
                        ElementType::Usage,
                        usage.unwrap_or_else(|| ctx.tr("narration.component_list.usage", &[])),
                    );
                }
            }
            narrate(&mut out.nest(), active[i], ctx, row_last);
        }
        None if ctx.info.navigation => {
            out.add(ElementType::Usage, ctx.tr("narrator.screen.usage", &[]));
        }
        None => {}
    }
}

/// `AbstractWidget.updateNarration`: the widget's `updateWidgetNarration`,
/// then its tooltip's (HINT).
fn narrate(
    out: &mut Output<'_>,
    n: &Narratable,
    ctx: &Ctx<'_>,
    row_last: &mut HashMap<(u64, usize), u64>,
) {
    let focused = ctx.focused(n);
    // AbstractWidget.defaultButtonNarrationText's usage.
    let button_usage = |out: &mut Output<'_>, active: bool| {
        if active {
            let key = if focused {
                "narration.button.usage.focused"
            } else {
                "narration.button.usage.hovered"
            };
            out.add(ElementType::Usage, ctx.tr(key, &[]));
        }
    };
    // CycleButton.updateWidgetNarration's usage: the value a click moves to.
    let cycle_usage = |out: &mut Output<'_>, next: &str, active: bool| {
        if active {
            let key = if focused {
                "narration.cycle_button.usage.focused"
            } else {
                "narration.cycle_button.usage.hovered"
            };
            out.add(ElementType::Usage, ctx.tr(key, &[next]));
        }
    };
    match &n.what {
        What::Button { message, active } => {
            out.add(ElementType::Title, ctx.tr("gui.narrate.button", &[message]));
            button_usage(out, *active);
        }
        What::Named { title, active } => {
            out.add(ElementType::Title, title.clone());
            button_usage(out, *active);
        }
        What::Slider { message, active } => {
            out.add(ElementType::Title, ctx.tr("gui.narrate.slider", &[message]));
            if *active {
                // AbstractSliderButton.setFocused: `canChangeValue` when the
                // focus came from the mouse or Tab.
                let key = if !focused {
                    "narration.slider.usage.hovered"
                } else if matches!(ctx.input, InputType::Mouse | InputType::KeyboardTab) {
                    "narration.slider.usage.focused"
                } else {
                    "narration.slider.usage.focused.keyboard_cannot_change_value"
                };
                out.add(ElementType::Usage, ctx.tr(key, &[]));
            }
        }
        What::Cycle {
            message,
            next,
            active,
        } => {
            out.add(ElementType::Title, ctx.tr("gui.narrate.button", &[message]));
            cycle_usage(out, next, *active);
        }
        What::NamedCycle {
            title,
            next,
            active,
        } => {
            out.add(ElementType::Title, title.clone());
            cycle_usage(out, next, *active);
        }
        What::Checkbox {
            message,
            selected,
            active,
        } => {
            out.add(ElementType::Title, ctx.tr("gui.narrate.button", &[message]));
            if *active {
                let key = match (focused, selected) {
                    (true, true) => "narration.checkbox.usage.focused.uncheck",
                    (true, false) => "narration.checkbox.usage.focused.check",
                    (false, true) => "narration.checkbox.usage.hovered.uncheck",
                    (false, false) => "narration.checkbox.usage.hovered.check",
                };
                out.add(ElementType::Usage, ctx.tr(key, &[]));
            }
        }
        What::EditBox { message, value, .. } => {
            out.add(
                ElementType::Title,
                ctx.tr("gui.narrate.editBox", &[message, value]),
            );
        }
        What::Text { message } => out.add(ElementType::Title, message.clone()),
        What::Recipe { item, more } => {
            out.add(ElementType::Title, ctx.tr("narration.recipe", &[item]));
            let mut lines = vec![ctx.tr("narration.button.usage.hovered", &[])];
            if *more {
                lines.push(ctx.tr("narration.recipe.usage.more", &[]));
            }
            out.add_lines(ElementType::Usage, lines);
        }
        What::List {
            kind,
            total,
            hovered,
            selected,
        } => {
            // ObjectSelectionList / TabNavigationBar.updateWidgetNarration:
            // the hovered row, else the selected one.
            let hovered = hovered.as_ref().filter(|_| !ctx.init);
            if let Some(row) = hovered.or(selected.as_ref()) {
                if *total > 1 {
                    let key = match kind {
                        ListKind::Object => "narrator.position.list",
                        ListKind::Tabs => "narrator.position.tab",
                    };
                    out.nest().add(
                        ElementType::Position,
                        ctx.tr(key, &[&num(row.index + 1), &num(*total)]),
                    );
                }
                match kind {
                    ListKind::Object => out.add(ElementType::Title, row.narration.clone()),
                    ListKind::Tabs => {
                        out.add(
                            ElementType::Title,
                            ctx.tr("gui.narrate.tab", &[&row.narration]),
                        );
                        if let Some(hint) = &row.hint {
                            out.add(ElementType::Hint, hint.clone());
                        }
                    }
                }
            }
            if focused {
                let key = match kind {
                    ListKind::Object => "narration.selection.usage",
                    ListKind::Tabs => "narration.tab_navigation.usage",
                };
                out.add(ElementType::Usage, ctx.tr(key, &[]));
            }
            return;
        }
        What::Group { .. } => {
            // RecipeBookComponent.updateNarration: its active widgets'
            // pick (no last one: a focused widget is always it), one in.
            let group = n.identity();
            let children: Vec<&Narratable> =
                ctx.children(group).filter(|c| c.is_active()).collect();
            let entries: Vec<(u64, Priority)> = children
                .iter()
                .map(|c| (c.identity(), ctx.priority(c)))
                .collect();
            if let Some((i, _)) = find_narratable(&entries, None) {
                let ctx = Ctx {
                    tooltips: false,
                    ..*ctx
                };
                narrate(&mut out.nest(), children[i], &ctx, row_last);
            }
            return;
        }
        What::ContainerList { rows, hovered_row } => {
            // ContainerObjectSelectionList.updateWidgetNarration: the row
            // under the mouse for the mouse and the system, else the focused
            // one; its widget one level deeper, the row's position here.
            let list = n.identity();
            let hovered_row = hovered_row.filter(|_| !ctx.init);
            let row = match (out.trigger(), hovered_row) {
                (Trigger::Mouse | Trigger::System, Some(row)) => Some(row),
                _ => ctx
                    .children(list)
                    .find(|c| ctx.focused(c))
                    .and_then(|c| c.parent.map(|(_, row)| row)),
            };
            if let Some(row) = row {
                let children: Vec<&Narratable> = ctx
                    .children(list)
                    .filter(|c| c.parent == Some((list, row)))
                    .collect();
                narrate_row(&mut out.nest(), (list, row), &children, ctx, row_last);
                if *rows > 1 {
                    out.add(
                        ElementType::Position,
                        ctx.tr("narrator.position.list", &[&num(row + 1), &num(*rows)]),
                    );
                }
            }
            return;
        }
    }
    let hint = n.hint.clone().or_else(|| {
        (ctx.tooltips && ctx.hovered(n))
            .then(|| ctx.frame.tooltip.clone())
            .flatten()
    });
    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        out.add(ElementType::Hint, hint);
    }
}

/// `ContainerObjectSelectionList.Entry.updateNarration`: the row's widget
/// `findNarratableWidget` picks (inactive ones too), with its place in the row.
fn narrate_row(
    out: &mut Output<'_>,
    row: (u64, usize),
    children: &[&Narratable],
    ctx: &Ctx<'_>,
    row_last: &mut HashMap<(u64, usize), u64>,
) {
    let entries: Vec<(u64, Priority)> = children
        .iter()
        .map(|c| (c.identity(), ctx.priority(c)))
        .collect();
    let Some((i, priority)) = find_narratable(&entries, row_last.get(&row).copied()) else {
        return;
    };
    if priority.is_terminal() {
        row_last.insert(row, entries[i].0);
    }
    if children.len() > 1 {
        out.add(
            ElementType::Position,
            ctx.tr(
                "narrator.position.object_list",
                &[&num(i + 1), &num(children.len())],
            ),
        );
    }
    narrate(&mut out.nest(), children[i], ctx, row_last);
}

/// `Screen.runNarration`: what to say now (empty for nothing).
fn run(open: &mut Open, ctx: &Ctx<'_>, only_changed: bool, trigger: Trigger) -> String {
    let Open {
        state, row_last, ..
    } = open;
    let mut last = state.last_narratable;
    let said = state.run(only_changed, trigger, |out| {
        update_narration_state(out, ctx, &mut last, row_last);
    });
    state.last_narratable = last;
    said
}

impl ScreenNarrator {
    fn now(&self) -> i64 {
        self.clock
            .unwrap_or_else(|| self.epoch.elapsed().as_millis() as i64)
    }

    /// `Minecraft.setLastInputType`: the mouse's buttons, Tab and the arrows.
    /// Tab and the arrows move the focus off what a click focused.
    pub fn set_input(&mut self, input: InputType) {
        self.input = input;
        if input.is_keyboard() {
            self.click_focus = None;
        }
    }
    /// `MouseHandler.handleAccumulatedMovement`: `afterMouseMove`.
    pub fn mouse_moved(&mut self) {
        let now = self.now();
        if let Some(open) = &mut self.open {
            open.state.after_mouse_move(now);
        }
    }
    /// A scroll (`MouseHandler.onScroll`): `afterMouseAction`.
    pub fn mouse_action(&mut self) {
        let now = self.now();
        if let Some(open) = &mut self.open {
            open.state.after_mouse_action(now);
        }
    }
    /// A mouse button pressed at `cursor` (GUI pixels), before the screen
    /// takes it (`MouseHandler.onButton`): `afterMouseAction`, and a left
    /// click focuses the active widget under the mouse (a container list's
    /// row widget, else the list), as `mouseClicked` does. A click on nothing
    /// keeps the focus.
    pub fn mouse_pressed(&mut self, cursor: (f32, f32), left: bool) {
        if self.open.is_none() {
            return;
        }
        if left && let Some(key) = self.clicked(cursor) {
            self.click_focus = Some(key);
        }
        self.mouse_action();
    }
    fn clicked(&self, cursor: (f32, f32)) -> Option<u64> {
        let items = &self.frame.items;
        let top = items
            .iter()
            .find(|n| n.parent.is_none() && n.is_active() && contains(n.rect, cursor))?;
        let key = top.identity();
        if !matches!(top.what, What::ContainerList { .. }) {
            return Some(key);
        }
        let child = items.iter().find(|c| {
            c.parent.is_some_and(|(list, _)| list == key)
                && c.is_active()
                && contains(c.rect, cursor)
        });
        Some(child.map_or(key, Narratable::identity))
    }
    /// A key pressed before the screen takes it (`KeyboardHandler.keyPress`):
    /// `afterKeyboardAction`.
    pub fn keyboard_action(&mut self) {
        let now = self.now();
        if let Some(open) = &mut self.open {
            open.state.after_keyboard_action(now);
        }
    }
    /// `Screen.updateNarratorStatus(wasDisabled, trigger)`: once the narrator
    /// is turned on, the open screen is narrated two seconds later.
    pub fn status_changed(&mut self, was_disabled: bool, trigger: Trigger) {
        let now = self.now();
        match &mut self.open {
            Some(open) => open.state.after_status_change(now, was_disabled, trigger),
            None => self.pending_status = Some((now, was_disabled, trigger)),
        }
    }

    /// Once per event loop turn, after `widgets::end_frame`: switches the
    /// frames' recording with the narrator, reads the last drawn frame and
    /// says what is due (`Gui.update`'s `handleDelayedNarration`, `init`'s
    /// narration). `gui` is the GUI size, `cursor` the mouse in it,
    /// `pad_focus` the controller's focus ring.
    pub fn frame(&mut self, gui: (f32, f32), cursor: (f32, f32), pad_focus: Option<[f32; 4]>) {
        let active = horizon_narrator::is_active();
        for said in self.frame_with(active, gui, cursor, pad_focus, &translate) {
            horizon_narrator::say_system_now(&said);
        }
    }

    /// [`frame`](Self::frame) with the narrator there (`active`) or not: what
    /// to say. `Screen.shouldRunNarration` is `GameNarrator.isActive`, whether
    /// the narrator *can* speak, not its status: while it is off the screens'
    /// narration still runs and is marked said (`saySystemNow` drops it), so
    /// the click that turns it on in the options says only what it changed,
    /// 200 ms later, after the status announcement.
    fn frame_with(
        &mut self,
        active: bool,
        gui: (f32, f32),
        cursor: (f32, f32),
        pad_focus: Option<[f32; 4]>,
        tr: Translate<'_>,
    ) -> Vec<String> {
        if !active {
            if self.recording {
                widgets::set_narrating(false);
                self.recording = false;
                self.open = None;
                self.frame = Narration::default();
                self.adopt = None;
            }
            self.pending_status = None;
            self.started = true;
            return Vec::new();
        }
        let generation = widgets::generation();
        if !self.recording {
            widgets::set_narrating(true);
            self.recording = true;
            self.turned_on(generation);
            return Vec::new();
        }
        // A screen's `triggerImmediateNarration(true)` (a list refilled)
        // reads the frame drawn after it.
        let mut narrate_now = false;
        if generation != self.generation {
            self.generation = generation;
            self.frame = widgets::take_last_narration();
            narrate_now = widgets::take_narrate_now();
        }
        let now = self.now();
        let said = self.step(now, gui, cursor, pad_focus, narrate_now, tr);
        // The drawers work out what only a focused widget narrates for it.
        let clicked = self.click_focus.and_then(|key| {
            let items = &self.frame.items;
            items.iter().find(|n| n.identity() == key).map(|n| n.rect)
        });
        widgets::set_narration_focus(clicked.or(pad_focus));
        said
    }

    /// The frames record from the next one drawn, of `generation + 1`: at
    /// startup that is the first screen opening; later it is the screen open
    /// when the narrator came on.
    fn turned_on(&mut self, generation: u64) {
        self.generation = generation;
        self.adopt = self.started.then_some(generation + 1);
        self.started = true;
    }

    /// One turn over the frame read: the open screen's `init` when it is new
    /// or resized, its repeated narration and the narration due.
    fn step(
        &mut self,
        now: i64,
        gui: (f32, f32),
        cursor: (f32, f32),
        pad_focus: Option<[f32; 4]>,
        narrate_now: bool,
        tr: Translate<'_>,
    ) -> Vec<String> {
        let adopting = self.adopt.is_some_and(|g| self.generation >= g);
        let pending = if adopting {
            self.adopt = None;
            self.pending_status.take()
        } else {
            None
        };
        let Some(info) = &self.frame.screen else {
            self.open = None;
            return Vec::new();
        };
        let opened = self.open.as_ref().is_none_or(|o| o.id != info.id);
        let resized = !opened && self.open.as_ref().is_some_and(|o| o.gui != gui);
        if opened {
            // Minecraft.setScreen over no screen: `setLastInputType(NONE)`,
            // so a screen opened from the game is never a keyboard one.
            if self.open.is_none() && !adopting {
                self.input = InputType::None;
            }
            self.click_focus = None;
            self.open = Some(Open {
                id: info.id.clone(),
                gui,
                state: ScreenNarration::default(),
                row_last: HashMap::new(),
                last_repeat: now,
            });
        } else if resized && let Some(open) = &mut self.open {
            // init(width, height) again: rebuildWidgets and clearFocus.
            open.gui = gui;
            open.state.last_narratable = None;
            open.row_last.clear();
            self.click_focus = None;
        }
        let ctx = |init| Ctx {
            frame: &self.frame,
            info,
            cursor,
            click_focus: self.click_focus,
            pad_focus,
            input: self.input,
            init,
            tooltips: true,
            tr,
        };
        let Some(open) = &mut self.open else {
            return Vec::new();
        };
        let mut said = Vec::new();
        if opened || resized {
            // Screen.init: `triggerImmediateNarration(false)`, then the
            // suppression (until the next key for a keyboard user).
            let text = run(open, &ctx(true), false, Trigger::System);
            if adopting {
                if let Some((at, was_disabled, trigger)) = pending {
                    open.state.after_status_change(at, was_disabled, trigger);
                }
            } else {
                said.push(text);
                open.state.after_init(now, self.input.is_keyboard());
            }
        }
        if narrate_now
            || info
                .repeat_ms
                .is_some_and(|ms| now - open.last_repeat > i64::from(ms))
        {
            open.last_repeat = now;
            said.push(run(open, &ctx(false), true, Trigger::System));
        }
        if let Some(trigger) = open.state.due(now) {
            said.push(run(open, &ctx(false), true, trigger));
        }
        said.retain(|s| !s.is_empty());
        said
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::Row;
    use ElementType::*;

    /// Keys and their arguments as they are, so the tests read what the
    /// narration is made of without a language loaded.
    fn keys(key: &str, args: &[&str]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            format!("{key}({})", args.join(", "))
        }
    }

    const GUI: (f32, f32) = (427.0, 240.0);

    struct Harness {
        n: ScreenNarrator,
        cursor: (f32, f32),
        pad_focus: Option<[f32; 4]>,
        gui: (f32, f32),
    }

    impl Harness {
        fn new() -> Self {
            let mut n = ScreenNarrator {
                clock: Some(0),
                ..Default::default()
            };
            n.turned_on(0);
            Self {
                n,
                cursor: (-1.0, -1.0),
                pad_focus: None,
                gui: GUI,
            }
        }
        /// A frame of `screen` drawn and read at `at`.
        fn draw(&mut self, at: i64, screen: &ScreenInfo, items: &[Narratable]) -> Vec<String> {
            self.draw_with(at, screen, items, None)
        }
        fn draw_with(
            &mut self,
            at: i64,
            screen: &ScreenInfo,
            items: &[Narratable],
            tooltip: Option<&str>,
        ) -> Vec<String> {
            self.n.clock = Some(at);
            self.n.generation += 1;
            self.n.frame = Narration {
                screen: Some(screen.clone()),
                items: items.to_vec(),
                tooltip: tooltip.map(str::to_owned),
            };
            self.n
                .step(at, self.gui, self.cursor, self.pad_focus, false, &keys)
        }
        fn at(&mut self, at: i64) -> &mut ScreenNarrator {
            self.n.clock = Some(at);
            &mut self.n
        }
        /// A left click where the mouse is.
        fn press(&mut self, at: i64) {
            self.n.clock = Some(at);
            self.n.mouse_pressed(self.cursor, true);
        }
    }

    fn button(message: &str, y: f32) -> Narratable {
        Narratable::button(message, [100.0, y, 200.0, 20.0], true)
    }

    fn title_screen() -> (ScreenInfo, Vec<Narratable>) {
        (
            ScreenInfo::new("title", "Title Screen"),
            vec![
                button("Singleplayer", 100.0),
                button("Multiplayer", 124.0),
                Narratable::button("Minecraft Realms", [100.0, 148.0, 200.0, 20.0], false),
                button("Options...", 196.0),
            ],
        )
    }

    #[test]
    fn a_screen_opening_says_its_title_and_how_to_navigate() {
        let mut h = Harness::new();
        let (screen, items) = title_screen();
        // Under the mouse already, but `init` narrates before the render.
        h.cursor = (150.0, 110.0);
        assert_eq!(
            h.draw(0, &screen, &items),
            ["Title Screen. narrator.screen.usage"]
        );
        // The same screen drawn again says nothing new.
        assert!(h.draw(16, &screen, &items).is_empty());
        assert!(h.draw(5000, &screen, &items).is_empty());
    }

    #[test]
    fn hovering_a_button_names_it_once_the_suppression_ends() {
        let mut h = Harness::new();
        let (screen, items) = title_screen();
        h.draw(0, &screen, &items);
        h.cursor = (150.0, 130.0);
        h.at(100).mouse_moved();
        // Due at 850, but quiet for 2 s after init.
        assert!(h.draw(1000, &screen, &items).is_empty());
        // Realms is inactive: three narratables, Multiplayer the second.
        assert_eq!(
            h.draw(2001, &screen, &items),
            [
                "gui.narrate.button(Multiplayer). narrator.position.screen(2, 3). \
                 narration.button.usage.hovered"
            ]
        );
        // Moving to Options... says only what changed.
        h.cursor = (150.0, 200.0);
        h.at(3000).mouse_moved();
        assert!(h.draw(3700, &screen, &items).is_empty());
        assert_eq!(
            h.draw(3751, &screen, &items),
            ["gui.narrate.button(Options...). narrator.position.screen(3, 3)"]
        );
    }

    #[test]
    fn a_click_focuses_its_widget_and_the_focus_is_narrated_first() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("options", "Options");
        let items = [button("FOV: 70", 50.0), button("Done", 200.0)];
        h.draw(0, &screen, &items);
        h.cursor = (150.0, 60.0);
        h.press(10);
        // A click lifts the suppression: 200 ms later, focused.
        assert_eq!(
            h.draw(211, &screen, &items),
            [
                "gui.narrate.button(FOV: 70). narrator.position.screen(1, 2). \
                 narration.component_list.usage. narration.button.usage.focused"
            ]
        );
        // The focused one was narrated last: the hovered one goes first.
        h.cursor = (150.0, 205.0);
        h.at(300).mouse_moved();
        assert_eq!(
            h.draw(1051, &screen, &items),
            ["gui.narrate.button(Done). narrator.position.screen(2, 2). \
                 narration.button.usage.hovered"]
        );
        // A click on nothing keeps the focus; a new screen drops it.
        h.at(1100).mouse_pressed((0.0, 0.0), true);
        assert_eq!(h.n.click_focus, Some(items[0].identity()));
        h.draw(1200, &ScreenInfo::new("video", "Video Settings"), &items);
        assert_eq!(h.n.click_focus, None);
    }

    #[test]
    fn inactive_widgets_are_not_narrated_and_a_single_one_has_no_position() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("death", "You Died!");
        let inactive = [
            Narratable::button("Respawn", [100.0, 100.0, 200.0, 20.0], false),
            Narratable::button("Title Screen", [100.0, 124.0, 200.0, 20.0], false),
        ];
        assert_eq!(
            h.draw(0, &screen, &inactive),
            ["You Died!. narrator.screen.usage"]
        );
        h.cursor = (150.0, 110.0);
        let items = [
            Narratable::button("Respawn", [100.0, 100.0, 200.0, 20.0], true),
            Narratable::button("Title Screen", [100.0, 124.0, 200.0, 20.0], false),
        ];
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw(3751, &screen, &items),
            ["gui.narrate.button(Respawn). narration.button.usage.hovered"]
        );
    }

    #[test]
    fn sliders_cycle_buttons_and_checkboxes_say_how_to_use_them() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("video", "Video Settings");
        let items = [
            Narratable::slider("FOV: 70", [0.0, 0.0, 150.0, 20.0], true),
            Narratable::new(
                What::Cycle {
                    message: "Graphics: Fancy".into(),
                    next: "Fabulous!".into(),
                    active: true,
                },
                [0.0, 30.0, 150.0, 20.0],
            ),
            Narratable::new(
                What::Checkbox {
                    message: "Do not show this screen again".into(),
                    selected: true,
                    active: true,
                },
                [0.0, 60.0, 150.0, 20.0],
            ),
        ];
        h.draw(0, &screen, &items);
        // The controller's ring focuses the slider, as the arrows would:
        // the keyboard can't change it before Enter.
        h.pad_focus = Some(items[0].rect);
        h.at(3000).set_input(InputType::KeyboardArrow);
        h.n.keyboard_action();
        assert_eq!(
            h.draw(3201, &screen, &items),
            [
                "gui.narrate.slider(FOV: 70). narrator.position.screen(1, 3). \
                 narration.component_list.usage. \
                 narration.slider.usage.focused.keyboard_cannot_change_value"
            ]
        );
        h.pad_focus = None;
        h.cursor = (10.0, 40.0);
        h.at(4000).mouse_moved();
        assert_eq!(
            h.draw(4751, &screen, &items),
            [
                "gui.narrate.button(Graphics: Fancy). narrator.position.screen(2, 3). \
                 narration.cycle_button.usage.hovered(Fabulous!)"
            ]
        );
        h.cursor = (10.0, 70.0);
        h.at(5000).set_input(InputType::Mouse);
        h.n.mouse_pressed(h.cursor, true);
        assert_eq!(
            h.draw(5201, &screen, &items),
            ["gui.narrate.button(Do not show this screen again). \
                 narrator.position.screen(3, 3). narration.component_list.usage. \
                 narration.checkbox.usage.focused.uncheck"]
        );
        // A click focuses a slider that the keys can then change.
        h.cursor = (10.0, 10.0);
        h.press(6000);
        assert_eq!(
            h.draw(6201, &screen, &items),
            [
                "gui.narrate.slider(FOV: 70). narrator.position.screen(1, 3). \
                 narration.slider.usage.focused"
            ]
        );
    }

    #[test]
    fn a_hovered_widget_reads_its_tooltip_as_a_hint() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("options", "Options");
        let items = [
            button("Language...", 50.0),
            button("Done", 200.0).hint("Back to the game"),
        ];
        h.draw(0, &screen, &items);
        h.cursor = (150.0, 60.0);
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw_with(
                3751,
                &screen,
                &items,
                Some("Translations may not be accurate")
            ),
            [
                "gui.narrate.button(Language...). narrator.position.screen(1, 2). \
                 Translations may not be accurate. narration.button.usage.hovered"
            ]
        );
        // Its own hint, whatever the frame's tooltip.
        h.cursor = (150.0, 205.0);
        h.at(4000).mouse_moved();
        assert_eq!(
            h.draw_with(4751, &screen, &items, Some("Other")),
            ["gui.narrate.button(Done). narrator.position.screen(2, 2). Back to the game"]
        );
    }

    #[test]
    fn an_object_list_narrates_its_hovered_row_or_its_selection() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("select_world", "Select World");
        let list = |hovered: Option<Row>| {
            Narratable::new(
                What::List {
                    kind: ListKind::Object,
                    total: 3,
                    hovered,
                    selected: Some(Row::new(0, "Selected: World A")),
                },
                [0.0, 50.0, 427.0, 150.0],
            )
            .key(widgets::key_of(&"worlds"))
        };
        let items = vec![
            list(Some(Row::new(1, "Selected: World B"))),
            button("Play Selected World", 210.0),
        ];
        h.draw(0, &screen, &items);
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw(3751, &screen, &items),
            ["Selected: World B. narrator.position.screen(1, 2). \
                 narrator.position.list(2, 3)"]
        );
        // Clicked: focused, its selection and how to move it.
        h.cursor = (10.0, 60.0);
        h.press(4000);
        let items = vec![list(None), button("Play Selected World", 210.0)];
        assert_eq!(
            h.draw(4201, &screen, &items),
            ["Selected: World A. narrator.position.list(1, 3). \
                 narration.component_list.usage. narration.selection.usage"]
        );
    }

    #[test]
    fn tabs_name_the_hovered_or_current_tab() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("create_world", "Create New World");
        let mut current = Row::new(0, "Game");
        current.hint = Some(String::new());
        let tabs = Narratable::new(
            What::List {
                kind: ListKind::Tabs,
                total: 3,
                hovered: None,
                selected: Some(current),
            },
            [0.0, 0.0, 427.0, 24.0],
        )
        .focused(true);
        // Focused at init (a keyboard user's initial focus).
        assert_eq!(
            h.draw(0, &screen, &[tabs]),
            [
                "Create New World. gui.narrate.tab(Game). narrator.position.tab(1, 3). . \
                 narration.tab_navigation.usage"
            ]
        );
    }

    #[test]
    fn a_container_list_narrates_its_row_and_the_rows_widget() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("options/Video", "Video Settings");
        let list = widgets::key_of(&"options");
        let child = |message: &str, x: f32, y: f32, row: usize| {
            Narratable::button(message, [x, y, 150.0, 20.0], true)
                .key(widgets::key_of(&message))
                .child_of(list, row)
        };
        let items = vec![
            Narratable::new(
                What::ContainerList {
                    rows: 3,
                    hovered_row: Some(1),
                },
                [0.0, 30.0, 427.0, 150.0],
            )
            .key(list),
            child("Graphics: Fancy", 50.0, 40.0, 0),
            child("Render Distance: 12", 210.0, 40.0, 0),
            child("Max Framerate: 120", 50.0, 64.0, 1),
            child("VSync: ON", 210.0, 64.0, 1),
            button("Done", 200.0),
        ];
        h.draw(0, &screen, &items);
        h.cursor = (250.0, 70.0);
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw(3751, &screen, &items),
            [
                "gui.narrate.button(VSync: ON). narrator.position.screen(1, 2). \
                 narrator.position.list(2, 3). narrator.position.object_list(2, 2). \
                 narration.button.usage.hovered"
            ]
        );
        // A click on a row's widget focuses it, and the list with it: the
        // keyboard's narration reads the focused row.
        h.press(4000);
        assert_eq!(h.n.click_focus, Some(widgets::key_of(&"VSync: ON")));
        h.at(4100).keyboard_action();
        assert_eq!(
            h.draw(4301, &screen, &items),
            ["narration.component_list.usage. narration.button.usage.focused"]
        );
    }

    #[test]
    fn a_screen_of_its_own_elements_skips_the_widgets() {
        let mut h = Harness::new();
        let chat = ScreenInfo::new("chat", "Chat screen")
            .without_widgets()
            .without_navigation()
            .element(Usage, 0, "Input message and press Enter to send")
            .element(Title, 1, "Message to send: hi");
        let items = [Narratable::edit_box("", "hi", [2.0, 228.0, 423.0, 12.0]).focused(true)];
        assert_eq!(
            h.draw(0, &chat, &items),
            ["Chat screen. Message to send: hi. Input message and press Enter to send"]
        );
        // NO_TITLE still says its empty title.
        let connecting = ScreenInfo::new("connect", "");
        assert_eq!(h.draw(10, &connecting, &[]), [". narrator.screen.usage"]);
    }

    #[test]
    fn a_repeating_screen_says_what_changed_every_two_seconds() {
        let mut h = Harness::new();
        let loading = |p: u32| {
            ScreenInfo::new("level_loading", "")
                .without_navigation()
                .element(Title, 0, format!("Loading {p}%"))
                .repeat_every(2000)
        };
        assert_eq!(h.draw(0, &loading(0), &[]), ["Loading 0%"]);
        assert!(h.draw(1500, &loading(40), &[]).is_empty());
        assert_eq!(h.draw(2001, &loading(50), &[]), ["Loading 50%"]);
        assert!(h.draw(4002, &loading(50), &[]).is_empty());
        // `triggerImmediateNarration(true)` at once.
        h.n.frame.screen = Some(loading(60));
        assert_eq!(
            h.n.step(4100, GUI, (0.0, 0.0), None, true, &keys),
            ["Loading 60%"]
        );
    }

    #[test]
    fn a_resize_narrates_the_screen_again() {
        let mut h = Harness::new();
        let (screen, items) = title_screen();
        h.draw(0, &screen, &items);
        h.gui = (640.0, 360.0);
        assert_eq!(
            h.draw(5000, &screen, &items),
            ["Title Screen. narrator.screen.usage"]
        );
        // And is quiet for two seconds again.
        h.cursor = (150.0, 110.0);
        h.at(5100).mouse_moved();
        assert!(h.draw(6000, &screen, &items).is_empty());
    }

    #[test]
    fn a_keyboard_user_hears_nothing_more_until_the_next_key() {
        let mut h = Harness::new();
        let (screen, items) = title_screen();
        h.draw(0, &screen, &items);
        h.at(10).set_input(InputType::KeyboardTab);
        h.n.keyboard_action();
        // Options opened from the title screen by keyboard.
        let options = ScreenInfo::new("options", "Options");
        assert_eq!(
            h.draw(100, &options, &items),
            ["Options. narrator.screen.usage"]
        );
        h.cursor = (150.0, 110.0);
        h.at(200).mouse_moved();
        assert!(h.draw(100_000, &options, &items).is_empty());
        h.at(100_001).keyboard_action();
        assert_eq!(
            h.draw(100_202, &options, &items),
            [
                "gui.narrate.button(Singleplayer). narrator.position.screen(1, 3). \
                 narration.button.usage.hovered"
            ]
        );
        // A screen opened from the game forgets the keyboard.
        h.n.frame.screen = None;
        h.n.step(100_300, GUI, h.cursor, None, false, &keys);
        assert_eq!(
            h.draw(100_400, &options, &items),
            ["Options. narrator.screen.usage"]
        );
        assert_eq!(h.n.input, InputType::None);
    }

    #[test]
    fn the_narrator_turned_on_over_a_screen_narrates_it_two_seconds_later() {
        let mut h = Harness::new();
        let (screen, items) = title_screen();
        // Ctrl+B before the frames record: the screen open is taken as it
        // is, silently, then narrated 2 s after the key.
        h.n.recording = false;
        h.at(1000).status_changed(true, Trigger::Keyboard);
        h.n.turned_on(h.n.generation);
        h.cursor = (150.0, 110.0);
        assert!(h.draw(1016, &screen, &items).is_empty());
        assert!(h.draw(3000, &screen, &items).is_empty());
        // Only what changed since: the hovered button, which init didn't see.
        assert_eq!(
            h.draw(3001, &screen, &items),
            [
                "gui.narrate.button(Singleplayer). narrator.position.screen(1, 3). \
                 narration.button.usage.hovered"
            ]
        );
    }

    #[test]
    fn key_binds_rows_name_their_key_and_headers_their_category() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("options/KeyBinds", "Key Binds");
        let list = widgets::key_of(&"options/KeyBinds/list");
        let child = |n: Narratable, row: usize, x: f32| {
            n.key(widgets::key_of(&(list, row, x as i32)))
                .child_of(list, row)
        };
        let items = vec![
            Narratable::container_list(3, Some(1), [0.0, 38.0, 427.0, 160.0]).key(list),
            child(
                Narratable::text("Movement", [180.0, 40.0, 60.0, 17.0]),
                0,
                180.0,
            ),
            child(
                Narratable::named(
                    "Jump is bound to gui.narrate.button(Space)",
                    [200.0, 60.0, 75.0, 20.0],
                    true,
                ),
                1,
                200.0,
            ),
            child(
                Narratable::named("Reset Jump button", [280.0, 60.0, 50.0, 20.0], false),
                1,
                280.0,
            ),
            button("Done", 210.0),
        ];
        h.draw(0, &screen, &items);
        h.cursor = (210.0, 65.0);
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw(3751, &screen, &items),
            [
                "Jump is bound to gui.narrate.button(Space). narrator.position.screen(1, 2). \
                 narrator.position.list(2, 3). narrator.position.object_list(1, 2). \
                 narration.button.usage.hovered"
            ]
        );
        // The category header row narrates its text, nothing more.
        let mut items = items;
        items[0] = Narratable::container_list(3, Some(0), [0.0, 38.0, 427.0, 160.0]).key(list);
        h.cursor = (200.0, 45.0);
        h.at(4000).mouse_moved();
        assert_eq!(
            h.draw(4751, &screen, &items),
            ["Movement. narrator.position.list(1, 3)"]
        );
    }

    #[test]
    fn a_switch_narrates_its_own_message_and_next_value() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("create_world", "Create New World");
        let items = [Narratable::new(
            What::NamedCycle {
                title: "Generate Structures. gui.narrate.button(: ON). Villages".into(),
                next: "OFF".into(),
                active: true,
            },
            [100.0, 100.0, 44.0, 20.0],
        )
        .hint("Villages")];
        h.draw(0, &screen, &items);
        h.cursor = (110.0, 110.0);
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw(3751, &screen, &items),
            [
                "Generate Structures. gui.narrate.button(: ON). Villages. Villages. \
                 narration.cycle_button.usage.hovered(OFF)"
            ]
        );
    }

    #[test]
    fn the_recipe_book_narrates_its_hovered_widget_one_level_in() {
        let mut h = Harness::new();
        let screen = ScreenInfo::new("container", "Crafting");
        let book = widgets::key_of(&"recipe_book");
        let in_book = |n: Narratable, i: usize| n.key(widgets::key_of(&(book, i))).in_group(book);
        let items = |visible: bool, search_focused: bool| {
            vec![
                Narratable::button("", [10.0, 100.0, 20.0, 18.0], true),
                Narratable::group(visible, [40.0, 40.0, 147.0, 166.0]).key(book),
                in_book(
                    Narratable::recipe("Oak Planks", false, [51.0, 71.0, 25.0, 25.0]),
                    0,
                ),
                in_book(
                    Narratable::recipe("Torch", true, [76.0, 71.0, 25.0, 25.0]),
                    1,
                ),
                in_book(
                    Narratable::edit_box("Search Items", "to", [65.0, 53.0, 81.0, 14.0])
                        .focused(search_focused),
                    2,
                ),
                in_book(
                    Narratable::cycle(
                        ": Showing All",
                        ": Showing Craftable",
                        [150.0, 52.0, 26.0, 16.0],
                        true,
                    )
                    .hint("Showing All"),
                    3,
                ),
                in_book(Narratable::button("", [10.0, 43.0, 35.0, 27.0], true), 4),
            ]
        };
        // Visible, it is narrated at once, though nothing in it is hovered.
        h.cursor = (80.0, 80.0);
        assert_eq!(
            h.draw(0, &screen, &items(true, false)),
            ["Crafting. narrator.position.screen(2, 2)"]
        );
        // A hovered recipe: its result and usage, no tooltip, no position.
        h.at(3000).mouse_moved();
        assert_eq!(
            h.draw_with(3751, &screen, &items(true, false), Some("Torch")),
            [
                "narration.recipe(Torch). narration.button.usage.hovered. narration.recipe.usage.more"
            ]
        );
        // The filter reads its tooltip.
        h.cursor = (160.0, 60.0);
        h.at(4000).mouse_moved();
        assert_eq!(
            h.draw(4751, &screen, &items(true, false)),
            [
                "gui.narrate.button(: Showing All). Showing All. narration.cycle_button.usage.hovered(: Showing Craftable)"
            ]
        );
        // The focused search box wins over what the mouse is on.
        h.at(5000).mouse_moved();
        assert_eq!(
            h.draw(5751, &screen, &items(true, true)),
            ["gui.narrate.editBox(Search Items, to)"]
        );
        // The toggle button before it wins among the hovered.
        h.cursor = (15.0, 105.0);
        h.at(6000).mouse_moved();
        assert_eq!(
            h.draw(6751, &screen, &items(true, false)),
            [
                "gui.narrate.button(). narrator.position.screen(1, 2). narration.button.usage.hovered"
            ]
        );
        // Hidden: never narrated, though still counted.
        h.cursor = (80.0, 80.0);
        h.at(7000).mouse_moved();
        let hidden = &items(false, false)[..2];
        assert_eq!(h.draw(7751, &screen, hidden), ["narrator.screen.usage"]);
        assert_eq!(
            h.draw(8000, &ScreenInfo::new("container/2", "Crafting"), hidden),
            ["Crafting. narrator.screen.usage"]
        );
    }

    #[test]
    fn the_drawers_hear_which_widget_has_the_focus() {
        let mut n = ScreenNarrator {
            clock: Some(0),
            ..Default::default()
        };
        let rect = [100.0, 40.0, 150.0, 20.0];
        let tick = |n: &mut ScreenNarrator, frame: u64, at: i64| {
            widgets::begin_screen(|| ScreenInfo::new("options/Video", "Video"));
            widgets::narratable(|| Narratable::button("VSync: ON", rect, true));
            widgets::end_frame(frame);
            n.clock = Some(at);
            n.frame_with(true, GUI, (0.0, 0.0), None, &keys)
        };
        tick(&mut n, 50_000, 0);
        tick(&mut n, 50_001, 16);
        assert!(!widgets::narration_focused(rect));
        n.clock = Some(100);
        n.mouse_pressed((110.0, 50.0), true);
        tick(&mut n, 50_002, 116);
        assert!(widgets::narration_focused(rect));
    }

    #[test]
    fn turning_the_narrator_on_with_its_button_says_what_the_click_changed() {
        // The narrator can speak but is off: the screens' narration runs
        // all the same (`shouldRunNarration` is `isActive`).
        let mut n = ScreenNarrator {
            clock: Some(0),
            ..Default::default()
        };
        let mut frames = 40_000;
        let mut tick = |n: &mut ScreenNarrator, at: i64, label: &str, next: &str, cursor| {
            widgets::begin_screen(|| ScreenInfo::new("options/Accessibility", "Accessibility"));
            let (label, next) = (label.to_owned(), next.to_owned());
            widgets::narratable(|| {
                Narratable::cycle(label, next, [100.0, 40.0, 150.0, 20.0], true)
            });
            widgets::narratable(|| button("Done", 200.0));
            frames += 1;
            widgets::end_frame(frames);
            n.clock = Some(at);
            n.frame_with(true, GUI, cursor, None, &keys)
        };
        let over = (150.0, 50.0);
        // The first turn switches the frames' recording on.
        assert!(tick(&mut n, 0, "Narrator: OFF", "Narrates All", over).is_empty());
        assert!(widgets::narrating());
        // Built and said (saySystemNow drops it while the status is off).
        assert_eq!(
            tick(&mut n, 16, "Narrator: OFF", "Narrates All", over),
            ["Accessibility. narrator.screen.usage"]
        );
        n.clock = Some(100);
        n.mouse_moved();
        assert_eq!(
            tick(&mut n, 2017, "Narrator: OFF", "Narrates All", over),
            [
                "gui.narrate.button(Narrator: OFF). narrator.position.screen(1, 2). \
                 narration.cycle_button.usage.hovered(Narrates All)"
            ]
        );
        // The click turns it on (announced at once by the status update);
        // 200 ms later the screen says only what the click changed.
        n.clock = Some(3000);
        n.mouse_pressed(over, true);
        assert!(
            tick(
                &mut n,
                3016,
                "Narrator: Narrates All",
                "Narrates Chat",
                over
            )
            .is_empty()
        );
        assert_eq!(
            tick(
                &mut n,
                3201,
                "Narrator: Narrates All",
                "Narrates Chat",
                over
            ),
            [
                "gui.narrate.button(Narrator: Narrates All). narration.component_list.usage. \
                 narration.cycle_button.usage.focused(Narrates Chat)"
            ]
        );
        // Without speech nothing is recorded or built.
        assert!(n.frame_with(false, GUI, over, None, &keys).is_empty());
        assert!(!widgets::narrating());
    }

    #[test]
    fn the_game_translates_with_the_current_language() {
        let mut h = Harness::new();
        h.n.clock = Some(0);
        h.n.generation += 1;
        h.n.frame = Narration {
            screen: Some(ScreenInfo::new("pause", "Game Menu")),
            items: Vec::new(),
            tooltip: None,
        };
        assert_eq!(
            h.n.step(0, GUI, (0.0, 0.0), None, false, &translate),
            [format!(
                "Game Menu. {}",
                translate("narrator.screen.usage", &[])
            )]
        );
        assert_eq!(
            translate("gui.narrate.button", &["Done"]),
            crate::lang::tr("gui.narrate.button", &[&"Done"])
        );
    }
}
