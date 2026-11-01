//! The labelled widgets and focus targets drawn in the last frame, for the
//! control socket: a script clicks a button by its label (`horizon-ctl
//! click-button Respawn`) or any widget by its id in [`tree`] (`horizon-ctl
//! click done`), whatever screen drew it. Also the mouse cursor the widget
//! under the mouse asks for.
use std::cell::{Cell, RefCell};

/// The mouse cursor a frame's widgets asked for (`CursorTypes`; the others
/// aren't asked for yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Arrow,
    PointingHand,
    NotAllowed,
}

thread_local! {
    /// `GuiGraphicsExtractor.requestCursor`: the last request since [`take_cursor`].
    static CURSOR: Cell<Cursor> = const { Cell::new(Cursor::Arrow) };
}

/// `GuiGraphicsExtractor.requestCursor`: the last request of a frame wins.
pub fn request_cursor(cursor: Cursor) {
    CURSOR.set(cursor);
}

/// `AbstractWidget.handleCursor`: a hovered widget asks for the pointing
/// hand, or for not-allowed when it is inactive.
pub fn widget_cursor(hovered: bool, active: bool) {
    if hovered {
        request_cursor(if active {
            Cursor::PointingHand
        } else {
            Cursor::NotAllowed
        });
    }
}

/// The cursor the frame asked for, and back to the arrow for the next one
/// (the window shows the arrow unless a widget asks).
pub fn take_cursor() -> Cursor {
    CURSOR.replace(Cursor::Arrow)
}

/// A labelled widget in GUI pixels (the coordinates screens take input in).
#[derive(Clone, Debug)]
pub struct Widget {
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub enabled: bool,
    /// Buttons for [`record`]; sliders and tabs carry their labels too.
    pub kind: Kind,
}

/// What a controller does with a focus target (docs/controllers.md, "Focus
/// navigation").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Clicked with A (buttons, icon buttons, keys).
    Button,
    /// Left and right move the value instead of focus; A does nothing (a
    /// click would jump the handle to the middle). `value` is 0..1 as drawn;
    /// `track` is where a click sets 0 and the width a click spans to 1.
    Slider { value: f32, track: [f32; 2] },
    /// A text field: A focuses it and opens an on-screen keyboard.
    Field,
    /// A list row (worlds, servers, packs, languages): A selects it.
    Row,
    /// An inventory slot: A/X/Y click, right click and quick-move it.
    Slot,
    /// A tab (`true` is the selected one): the bumpers step through them.
    Tab(bool),
}

/// A focus target: a rectangle in GUI pixels and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub rect: [f32; 4],
    pub kind: Kind,
}
impl Target {
    pub fn center(&self) -> (f32, f32) {
        let [x, y, w, h] = self.rect;
        (x + w / 2.0, y + h / 2.0)
    }
}

thread_local! {
    /// Buttons drawn since the last `end_frame`, and the last frame's.
    static WIDGETS: RefCell<(Vec<Widget>, Vec<Widget>, u64)> = const { RefCell::new((Vec::new(), Vec::new(), 0)) };
    /// Every interactive rectangle (sliders, fields, list rows, the
    /// on-screen keyboard's keys), labelled or not: where a controller can move focus. Drawn since
    /// `end_frame`, the last frame's, and how many frames rotated.
    static TARGETS: RefCell<(Vec<Target>, Vec<Target>, u64)> = const { RefCell::new((Vec::new(), Vec::new(), 0)) };
}

/// Splitscreen: one player's buttons and focus targets, kept aside while another player's game
/// runs on this thread ([`take`] and [`put`] around each turn).
#[derive(Default)]
pub struct Saved(
    (Vec<Widget>, Vec<Widget>, u64),
    (Vec<Target>, Vec<Target>, u64),
);

/// This thread's buttons and targets, leaving none.
pub fn take() -> Saved {
    Saved(WIDGETS.take(), TARGETS.take())
}

/// Bring back what [`take`] kept.
pub fn put(saved: Saved) {
    WIDGETS.set(saved.0);
    TARGETS.set(saved.1);
}

/// An interactive rectangle (GUI pixels) a controller can focus and click.
pub fn target(rect: [f32; 4]) {
    target_as(rect, Kind::Button);
}

/// An interactive rectangle of a given kind.
pub fn target_as(rect: [f32; 4], kind: Kind) {
    if !rect.iter().all(|v| v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    TARGETS.with_borrow_mut(|(current, _, _)| {
        if current.len() < 1024 {
            current.push(Target { rect, kind });
        }
    });
}

/// How many targets the frame has so far, for [`clip_targets`].
pub fn targets_mark() -> usize {
    TARGETS.with_borrow(|(current, _, _)| current.len())
}

/// Drop the targets registered since `mark` that aren't inside `y0..y1`
/// (GUI pixels): rows a scrolling list clipped away.
pub fn clip_targets(mark: usize, y0: f32, y1: f32) {
    TARGETS.with_borrow_mut(|(current, _, _)| {
        let mut i = mark.min(current.len());
        while i < current.len() {
            let [_, y, _, h] = current[i].rect;
            if y < y0 - 0.5 || y + h > y1 + 0.5 {
                current.remove(i);
            } else {
                i += 1;
            }
        }
    });
}

/// The focus targets of the last drawn frame.
pub fn last_targets() -> Vec<Target> {
    TARGETS.with_borrow(|(_, last, _)| last.clone())
}

/// Counts drawn frames: focus waits for a new one after scrolling a list.
pub fn generation() -> u64 {
    TARGETS.with_borrow(|(_, _, generation)| *generation)
}

/// Called by the button drawers.
pub fn record(label: &str, rect: [f32; 4], enabled: bool) {
    record_as(label, rect, enabled, Kind::Button);
}

/// The label of a widget of another kind (a slider's "FOV: 70", a tab).
pub fn record_as(label: &str, [x, y, w, h]: [f32; 4], enabled: bool, kind: Kind) {
    let label = label.trim();
    if label.is_empty() {
        return;
    }
    WIDGETS.with_borrow_mut(|(current, _, _)| {
        if current.len() < 512 {
            current.push(Widget {
                label: label.to_owned(),
                x,
                y,
                w,
                h,
                enabled,
                kind,
            });
        }
    });
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn hovered_widgets_ask_for_the_hand_or_not_allowed() {
        let _ = take_cursor();
        widget_cursor(false, true);
        assert_eq!(take_cursor(), Cursor::Arrow);
        widget_cursor(true, true);
        assert_eq!(take_cursor(), Cursor::PointingHand);
        // Taken: the next frame starts from the arrow.
        assert_eq!(take_cursor(), Cursor::Arrow);
        widget_cursor(true, false);
        assert_eq!(take_cursor(), Cursor::NotAllowed);
        // The last request of a frame wins.
        widget_cursor(true, false);
        widget_cursor(false, true);
        widget_cursor(true, true);
        assert_eq!(take_cursor(), Cursor::PointingHand);
    }
}

/// Once per event loop turn with the count of rendered frames: when a new
/// frame was drawn since the last call, its buttons become the last frame's.
pub fn end_frame(frames: u64) {
    WIDGETS.with_borrow_mut(|(current, last, seen)| {
        if *seen != frames {
            *seen = frames;
            *last = std::mem::take(current);
            TARGETS.with_borrow_mut(|(current, last, generation)| {
                *last = std::mem::take(current);
                *generation += 1;
            });
            NARRATION.with_borrow_mut(|(current, last)| {
                *last = std::mem::replace(current, Narration::EMPTY);
            });
        }
    });
}

/// The buttons of the last drawn frame, top-most last.
pub fn last() -> Vec<Widget> {
    WIDGETS.with_borrow(|(_, last, _)| {
        last.iter()
            .filter(|w| w.kind == Kind::Button)
            .cloned()
            .collect()
    })
}

/// Every labelled widget of the last drawn frame, sliders and tabs included.
pub fn last_labelled() -> Vec<Widget> {
    WIDGETS.with_borrow(|(_, last, _)| last.clone())
}

// Screen narration: what a frame's screen and widgets would narrate
// (`Screen.updateNarrationState` and each `NarratableEntry`), recorded by the
// drawers as they draw, for horizon-client's `screen_narrator` to read once the
// frame is done. Nothing is recorded (nor built: the record calls take
// closures) unless the narrator asked for it with `set_narrating`.

pub use horizon_narrator::screen::ElementType;

/// The screen a frame drew, as `Screen` narrates it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenInfo {
    /// Which screen this is: a new id is a new `Screen` (narrated as it
    /// opens), the same id a screen still open.
    pub id: String,
    /// `getNarrationMessage` (TITLE at depth 0): the title, or its
    /// override (`joinForNarration(title, message)` for confirm screens).
    /// Empty for `CommonComponents.EMPTY`, which is narrated all the same.
    pub title: String,
    /// `shouldNarrateNavigation`: "Use mouse cursor or Tab button to select
    /// element" when no widget is narrated.
    pub navigation: bool,
    /// `updateNarratedWidget` runs (screens that override
    /// `updateNarrationState`, the chat, don't).
    pub widgets: bool,
    /// The screen's own elements beyond its title, at their depth (an
    /// element of the title's kind and depth replaces it, as
    /// `LevelLoadingScreen`'s progress does).
    pub extra: Vec<(ElementType, u32, Vec<String>)>,
    /// `getUsageNarration`, when the screen overrides it.
    pub usage: Option<String>,
    /// `triggerImmediateNarration(true)` every this many milliseconds
    /// (`LevelLoadingScreen`'s progress).
    pub repeat_ms: Option<u32>,
}

impl ScreenInfo {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            navigation: true,
            widgets: true,
            extra: Vec::new(),
            usage: None,
            repeat_ms: None,
        }
    }
    /// One more element of the screen's own.
    pub fn element(self, kind: ElementType, depth: u32, text: impl Into<String>) -> Self {
        self.lines(kind, depth, vec![text.into()])
    }
    /// One more element of several lines, read as sentences.
    pub fn lines(mut self, kind: ElementType, depth: u32, lines: Vec<String>) -> Self {
        self.extra.push((kind, depth, lines));
        self
    }
    pub fn without_widgets(mut self) -> Self {
        self.widgets = false;
        self
    }
    pub fn without_navigation(mut self) -> Self {
        self.navigation = false;
        self
    }
    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = Some(usage.into());
        self
    }
    pub fn repeat_every(mut self, ms: u32) -> Self {
        self.repeat_ms = Some(ms);
        self
    }
}

/// What a list narrates its rows as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// `ObjectSelectionList`: "Selected list row i out of n", the row's
    /// narration, and the selection's usage when focused.
    Object,
    /// `TabNavigationBar`: "Selected tab i out of n", "<title> tab", and
    /// the tab navigation's usage when focused.
    Tabs,
}

/// A row of an `ObjectSelectionList` (its `getNarration`) or a tab (its
/// title and `getTabExtraNarration`).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub index: usize,
    pub narration: String,
    pub hint: Option<String>,
}

impl Row {
    pub fn new(index: usize, narration: impl Into<String>) -> Self {
        Self {
            index,
            narration: narration.into(),
            hint: None,
        }
    }
}

/// A narratable widget's kind and what its narration reads.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// `Button`: "<message> button".
    Button { message: String, active: bool },
    /// A button whose `createNarrationMessage` is its own (TITLE as is), with
    /// a button's usage.
    Named { title: String, active: bool },
    /// `AbstractSliderButton`: "<message> slider".
    Slider { message: String, active: bool },
    /// `CycleButton`: "<message> button" and the value a click moves to.
    Cycle {
        message: String,
        next: String,
        active: bool,
    },
    /// A `CycleButton` with its own narration (`withCustomNarration`):
    /// `title` as is, and the value a click moves to.
    NamedCycle {
        title: String,
        next: String,
        active: bool,
    },
    /// `Checkbox`: "<message> button" and whether a click checks it.
    Checkbox {
        message: String,
        selected: bool,
        active: bool,
    },
    /// `EditBox`: "<message> edit box: <value>"; an inactive one (a box
    /// that can't be edited now) isn't narrated.
    EditBox {
        message: String,
        value: String,
        active: bool,
    },
    /// A text widget that narrates its message and nothing else
    /// (`FittingMultiLineTextWidget`, a list's `FocusableTextWidget` header).
    Text { message: String },
    /// An `ObjectSelectionList` or a `TabNavigationBar`: `total` rows, the one
    /// under the mouse and the selected one.
    List {
        kind: ListKind,
        total: usize,
        hovered: Option<Row>,
        selected: Option<Row>,
    },
    /// A `ContainerObjectSelectionList` of `rows` rows (options, key binds):
    /// its rows' widgets are the narratables recorded with it as their
    /// parent. `hovered_row` is the row under the mouse.
    ContainerList {
        rows: usize,
        hovered_row: Option<usize>,
    },
    /// `RecipeButton`: "Recipe for <item>".
    Recipe { item: String, more: bool },
    /// `RecipeBookComponent`: one narratable, HOVERED whenever `visible`
    /// (whatever the mouse and the focus), that narrates the widget
    /// `findNarratableWidget` picks among its own one level in, with no
    /// position of its own. Its widgets are the narratables recorded with it
    /// as their parent (row 0).
    Group { visible: bool },
}

/// A `NarratableEntry` as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Narratable {
    /// The widget's identity across frames (`lastNarratable`); 0 derives
    /// one from its kind and place. A list's children need stable keys.
    pub key: u64,
    /// GUI pixels.
    pub rect: [f32; 4],
    /// `isHovered`, as the drawer saw it; `None` tests the mouse against
    /// `rect`.
    pub hovered: Option<bool>,
    /// `isFocused` (a text field with the caret, a focused list).
    pub focused: bool,
    pub what: What,
    /// Its tooltip's narration (HINT); empty for none. Without one, the
    /// tooltip drawn this frame is the hovered widget's.
    pub hint: Option<String>,
    /// A row widget of a `ContainerList`: that list's key and the row.
    pub parent: Option<(u64, usize)>,
    /// `getTabOrderGroup`: narratables are numbered in this order, then in
    /// the order they were drawn.
    pub order: i32,
}

impl Narratable {
    pub fn new(what: What, rect: [f32; 4]) -> Self {
        Self {
            key: 0,
            rect,
            hovered: None,
            focused: false,
            what,
            hint: None,
            parent: None,
            order: 0,
        }
    }
    pub fn button(message: impl Into<String>, rect: [f32; 4], active: bool) -> Self {
        Self::new(
            What::Button {
                message: message.into(),
                active,
            },
            rect,
        )
    }
    pub fn slider(message: impl Into<String>, rect: [f32; 4], active: bool) -> Self {
        Self::new(
            What::Slider {
                message: message.into(),
                active,
            },
            rect,
        )
    }
    pub fn edit_box(message: impl Into<String>, value: impl Into<String>, rect: [f32; 4]) -> Self {
        Self::new(
            What::EditBox {
                message: message.into(),
                value: value.into(),
                active: true,
            },
            rect,
        )
    }
    /// A button narrated by its own `createNarrationMessage`.
    pub fn named(title: impl Into<String>, rect: [f32; 4], active: bool) -> Self {
        Self::new(
            What::Named {
                title: title.into(),
                active,
            },
            rect,
        )
    }
    /// A `CycleButton` showing `message`; a click moves it to `next`.
    pub fn cycle(
        message: impl Into<String>,
        next: impl Into<String>,
        rect: [f32; 4],
        active: bool,
    ) -> Self {
        Self::new(
            What::Cycle {
                message: message.into(),
                next: next.into(),
                active,
            },
            rect,
        )
    }
    pub fn checkbox(
        message: impl Into<String>,
        selected: bool,
        rect: [f32; 4],
        active: bool,
    ) -> Self {
        Self::new(
            What::Checkbox {
                message: message.into(),
                selected,
                active,
            },
            rect,
        )
    }
    pub fn text(message: impl Into<String>, rect: [f32; 4]) -> Self {
        Self::new(
            What::Text {
                message: message.into(),
            },
            rect,
        )
    }
    /// An `ObjectSelectionList` (or tabs) of `total` rows over `rect`.
    pub fn list(
        kind: ListKind,
        total: usize,
        hovered: Option<Row>,
        selected: Option<Row>,
        rect: [f32; 4],
    ) -> Self {
        Self::new(
            What::List {
                kind,
                total,
                hovered,
                selected,
            },
            rect,
        )
    }
    /// A `ContainerObjectSelectionList` of `rows` rows over `rect`.
    pub fn container_list(rows: usize, hovered_row: Option<usize>, rect: [f32; 4]) -> Self {
        Self::new(What::ContainerList { rows, hovered_row }, rect)
    }
    /// A `RecipeBookComponent`-like group over `rect`, narrated while
    /// `visible`; its widgets are recorded with [`Self::in_group`].
    pub fn group(visible: bool, rect: [f32; 4]) -> Self {
        Self::new(What::Group { visible }, rect)
    }
    /// `RecipeButton`: the recipe's result `item`, and whether it has more
    /// recipes than one.
    pub fn recipe(item: impl Into<String>, more: bool, rect: [f32; 4]) -> Self {
        Self::new(
            What::Recipe {
                item: item.into(),
                more,
            },
            rect,
        )
    }
    pub fn key(mut self, key: u64) -> Self {
        self.key = key;
        self
    }
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = Some(hovered);
        self
    }
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// No HINT, whatever tooltip the frame draws: the tooltip isn't the
    /// widget's (a list's row tooltip, drawn by the list).
    pub fn without_tooltip(self) -> Self {
        self.hint("")
    }
    /// A widget of row `row` of the `ContainerList` keyed `list`.
    pub fn child_of(mut self, list: u64, row: usize) -> Self {
        self.parent = Some((list, row));
        self
    }
    /// A widget of the [`What::Group`] keyed `group`.
    pub fn in_group(self, group: u64) -> Self {
        self.child_of(group, 0)
    }
    pub fn order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
    /// `isActive`: visible and active. Inactive buttons and sliders are left
    /// out of a screen's narratables.
    pub fn is_active(&self) -> bool {
        match &self.what {
            What::Button { active, .. }
            | What::Named { active, .. }
            | What::Slider { active, .. }
            | What::Cycle { active, .. }
            | What::NamedCycle { active, .. }
            | What::Checkbox { active, .. }
            | What::EditBox { active, .. } => *active,
            What::Text { .. }
            | What::List { .. }
            | What::ContainerList { .. }
            | What::Recipe { .. }
            | What::Group { .. } => true,
        }
    }
    /// `key`, or one from the widget's kind and place (never 0).
    pub fn identity(&self) -> u64 {
        if self.key != 0 {
            return self.key;
        }
        let place = self.rect.map(|v| v.round() as i32);
        key_of(&(std::mem::discriminant(&self.what), place))
    }
}

/// A stable key for a widget from anything that names it (a list's name, an
/// option's id), never 0.
pub fn key_of(name: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish() | 1
}

/// `CommonComponents.joinForNarration`: the parts, ". " between them.
pub fn join_for_narration(parts: &[&str]) -> String {
    parts.join(". ")
}

/// A frame's narration records.
#[derive(Debug, Default)]
pub struct Narration {
    pub screen: Option<ScreenInfo>,
    pub items: Vec<Narratable>,
    /// The tooltip drawn this frame, its lines joined.
    pub tooltip: Option<String>,
}

impl Narration {
    const EMPTY: Narration = Narration {
        screen: None,
        items: Vec::new(),
        tooltip: None,
    };
}

thread_local! {
    /// The narrator wants the frames' narration (`Screen.shouldRunNarration`).
    static NARRATING: Cell<bool> = const { Cell::new(false) };
    /// Narration recorded since the last `end_frame`, and the last frame's.
    static NARRATION: RefCell<(Narration, Narration)> =
        const { RefCell::new((Narration::EMPTY, Narration::EMPTY)) };
    /// A screen asked for `triggerImmediateNarration(true)`.
    static NARRATE_NOW: Cell<bool> = const { Cell::new(false) };
    /// Where the narrator's focused widget was last drawn.
    static FOCUS: Cell<Option<[f32; 4]>> = const { Cell::new(None) };
}

/// The narrator tells the drawers where its focused widget is (a click's
/// or the controller's), so what only a hovered or focused widget narrates
/// (a cycle button's next value) is worked out for it alone.
pub fn set_narration_focus(rect: Option<[f32; 4]>) {
    FOCUS.set(rect);
}

/// Whether the widget at `rect` has the narrator's focus.
pub fn narration_focused(rect: [f32; 4]) -> bool {
    FOCUS
        .get()
        .is_some_and(|f| f.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5))
}

/// Whether the frames record their narration; the narrator switches it.
pub fn set_narrating(on: bool) {
    NARRATING.set(on);
    if !on {
        NARRATION.with_borrow_mut(|(current, _)| *current = Narration::EMPTY);
    }
}

pub fn narrating() -> bool {
    NARRATING.get()
}

/// The screen being drawn: what it narrates and which one it is. The
/// narratables recorded before it in the frame are dropped: the last screen
/// drawn (an overlay over the screen under it) is the one narrated.
pub fn begin_screen(info: impl FnOnce() -> ScreenInfo) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            current.screen = Some(info());
            current.items.clear();
            current.tooltip = None;
        });
    }
}

/// Changes the screen begun this frame, if one was.
pub fn screen_mut(f: impl FnOnce(&mut ScreenInfo)) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            if let Some(screen) = &mut current.screen {
                f(screen);
            }
        });
    }
}

/// A narratable widget of the screen begun this frame (nothing without one).
pub fn narratable(narratable: impl FnOnce() -> Narratable) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            if current.screen.is_some() && current.items.len() < 512 {
                current.items.push(narratable());
            }
        });
    }
}

/// Changes the narratable recorded last this frame (one a shared drawer
/// recorded: an edit box's message, a button's key or row).
pub fn last_narratable(f: impl FnOnce(&mut Narratable)) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            if let Some(last) = current.items.last_mut() {
                f(last);
            }
        });
    }
}

/// Changes the narratable recorded last this frame and moves it to `index`
/// among the screen's: a widget the screen adds before others it draws first.
pub fn last_narratable_at(index: usize, f: impl FnOnce(&mut Narratable)) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            if let Some(mut last) = current.items.pop() {
                f(&mut last);
                let index = index.min(current.items.len());
                current.items.insert(index, last);
            }
        });
    }
}

/// How many narratables the frame has so far, for [`move_narratables`].
pub fn narratables_mark() -> usize {
    if !NARRATING.get() {
        return 0;
    }
    NARRATION.with_borrow(|(current, _)| current.items.len())
}

/// Moves the narratables recorded since `mark` to `index`, in their order:
/// widgets a screen adds before others it draws first.
pub fn move_narratables(mark: usize, index: usize) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            let items = &mut current.items;
            let mark = mark.min(items.len());
            let index = index.min(mark);
            let moved = items.len() - mark;
            items[index..].rotate_right(moved);
        });
    }
}

/// Names the edit box recorded last, which a shared drawer recorded without
/// its `getMessage`.
pub fn name_last_edit_box(name: impl FnOnce() -> String) {
    last_narratable(|n| {
        if let What::EditBox { message, .. } = &mut n.what {
            *message = name();
        }
    });
}

/// The tooltip drawn this frame: the hovered widget's narration hint.
pub fn frame_tooltip(lines: &[crate::text::Text]) {
    if NARRATING.get() {
        NARRATION.with_borrow_mut(|(current, _)| {
            let text: Vec<String> = lines.iter().map(|l| l.plain()).collect();
            current.tooltip = Some(text.join(" "));
        });
    }
}

/// `triggerImmediateNarration(true)`: narrate what changed at once.
pub fn narrate_now() {
    if NARRATING.get() {
        NARRATE_NOW.set(true);
    }
}

/// Whether a screen asked to narrate at once since the last call.
pub fn take_narrate_now() -> bool {
    NARRATE_NOW.replace(false)
}

/// The last drawn frame's narration, taken (the narrator is its only reader).
pub fn take_last_narration() -> Narration {
    NARRATION.with_borrow_mut(|(_, last)| std::mem::replace(last, Narration::EMPTY))
}

#[cfg(test)]
mod narration_tests {
    use super::*;

    fn frame(n: u64) -> Narration {
        end_frame(n);
        take_last_narration()
    }

    #[test]
    fn nothing_is_recorded_or_built_unless_narrating() {
        set_narrating(false);
        let _ = frame(1000);
        begin_screen(|| unreachable!("not built while the narrator is off"));
        narratable(|| unreachable!());
        narrate_now();
        let last = frame(1001);
        assert!(last.screen.is_none() && last.items.is_empty());
        assert!(!take_narrate_now());
    }

    #[test]
    fn a_frame_keeps_the_last_screen_begun_and_its_widgets() {
        set_narrating(true);
        let _ = frame(2000);
        // Without a screen, widgets aren't kept.
        narratable(|| Narratable::button("Stray", [0.0; 4], true));
        begin_screen(|| ScreenInfo::new("pause", "Game Menu"));
        narratable(|| Narratable::button("Back to Game", [0.0, 0.0, 200.0, 20.0], true));
        frame_tooltip(&[crate::text::Text::literal("Under")]);
        // An overlay over the pause menu: its own screen and widgets.
        begin_screen(|| ScreenInfo::new("confirm", "Sure?"));
        narratable(|| Narratable::button("Yes", [0.0, 30.0, 100.0, 20.0], true));
        screen_mut(|s| s.title.push('!'));
        frame_tooltip(&[
            crate::text::Text::literal("Two"),
            crate::text::Text::literal("lines"),
        ]);
        let last = frame(2001);
        assert_eq!(last.screen.unwrap().title, "Sure?!");
        assert_eq!(last.items.len(), 1);
        assert_eq!(
            last.items[0].what,
            What::Button {
                message: "Yes".into(),
                active: true
            }
        );
        assert_eq!(last.tooltip.as_deref(), Some("Two lines"));
        // Taken: read once.
        assert!(take_last_narration().screen.is_none());
        // A frame not drawn doesn't rotate.
        begin_screen(|| ScreenInfo::new("title", ""));
        assert!(frame(2001).screen.is_none());
        assert_eq!(frame(2002).screen.unwrap().id, "title");
        // Switching off drops what was recorded.
        begin_screen(|| ScreenInfo::new("title", ""));
        set_narrating(false);
        assert!(frame(2003).screen.is_none());
    }

    #[test]
    fn the_last_widget_recorded_can_be_named_after_the_fact() {
        set_narrating(true);
        let _ = frame(3000);
        begin_screen(|| ScreenInfo::new("select_world", "Select World"));
        // A shared drawer records the box without its message.
        narratable(|| Narratable::edit_box("", "abc", [0.0; 4]));
        last_narratable(|n| {
            if let What::EditBox { message, .. } = &mut n.what {
                *message = "Search for worlds".into();
            }
            n.order = -1;
        });
        let last = frame(3001);
        assert_eq!(
            last.items[0].what,
            What::EditBox {
                message: "Search for worlds".into(),
                value: "abc".into(),
                active: true,
            }
        );
        assert_eq!(last.items[0].order, -1);
        set_narrating(false);
        last_narratable(|_| unreachable!("nothing recorded while off"));
    }

    #[test]
    fn widgets_drawn_late_can_be_moved_to_where_they_were_added() {
        set_narrating(true);
        let _ = frame(4000);
        begin_screen(|| ScreenInfo::new("command_block", ""));
        narratable(|| Narratable::button("Done", [0.0; 4], true));
        narratable(|| Narratable::button("Cancel", [0.0; 4], true));
        let mark = narratables_mark();
        narratable(|| Narratable::edit_box("Command", "", [0.0; 4]));
        narratable(|| Narratable::button("Late", [0.0; 4], true));
        move_narratables(mark, 0);
        let order: Vec<String> = frame(4001)
            .items
            .iter()
            .map(|n| match &n.what {
                What::Button { message, .. } | What::EditBox { message, .. } => message.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(order, ["Command", "Late", "Done", "Cancel"]);
        set_narrating(false);
        assert_eq!(narratables_mark(), 0);
    }

    #[test]
    fn widgets_know_who_they_are_and_whether_they_are_narrated() {
        let a = Narratable::button("A", [10.0, 20.0, 200.0, 20.0], true);
        let moved = Narratable::button("A", [10.0, 44.0, 200.0, 20.0], true);
        let slider = Narratable::slider("A", [10.0, 20.0, 200.0, 20.0], true);
        assert_eq!(
            a.identity(),
            Narratable::button("B", a.rect, false).identity()
        );
        assert_ne!(a.identity(), moved.identity());
        assert_ne!(a.identity(), slider.identity());
        assert_eq!(a.clone().key(7).identity(), 7);
        assert_ne!(key_of(&"options"), 0);
        assert!(a.is_active());
        assert!(!Narratable::button("B", a.rect, false).is_active());
        assert!(Narratable::edit_box("Search", "", a.rect).is_active());
        let mut port = Narratable::edit_box("Port", "", a.rect);
        if let What::EditBox { active, .. } = &mut port.what {
            *active = false;
        }
        assert!(!port.is_active());
        assert!(Narratable::group(false, a.rect).is_active());
        let recipe = Narratable::recipe("Torch", true, a.rect).in_group(9);
        assert_eq!(recipe.parent, Some((9, 0)));
        assert_eq!(join_for_narration(&["Title", "Message"]), "Title. Message");
    }
}

impl Kind {
    /// The kind's name in the control socket's `ui` reply.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Button => "button",
            Kind::Slider { .. } => "slider",
            Kind::Field => "field",
            Kind::Row => "row",
            Kind::Slot => "slot",
            Kind::Tab(_) => "tab",
        }
    }
}

/// A widget of the `ui` tree: a labelled widget and its focus target merged.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The label as a slug up to its `:` (`FOV: 70` is `fov`), `-2`, `-3`
    /// for repeats; `<kind>-<n>` in draw order without a label.
    pub id: String,
    pub kind: Kind,
    pub label: Option<String>,
    pub rect: [f32; 4],
    pub enabled: bool,
}

/// `Video Settings...` → `video-settings`, `FOV: 70` → `fov`.
pub fn slug(label: &str) -> String {
    let name = label.split(':').next().unwrap_or(label);
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// The labelled widgets and the focus targets of a frame as one list with
/// ids: a target drawn at a labelled widget's rectangle is that widget
/// (its kind wins, a slider's value, unless the label says it's a tab:
/// Horizon's segments); other targets follow, unlabelled. With the GUI
/// size, what lies off screen (layout probes) is left out before naming.
pub fn tree(widgets: &[Widget], targets: &[Target], screen: Option<[f32; 2]>) -> Vec<Node> {
    let same = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(p, q)| (p - q).abs() < 0.5);
    let mut nodes: Vec<Node> = widgets
        .iter()
        .map(|w| Node {
            id: String::new(),
            kind: w.kind,
            label: Some(w.label.clone()),
            rect: [w.x, w.y, w.w, w.h],
            enabled: w.enabled,
        })
        .collect();
    for t in targets {
        if let Some(node) = nodes.iter_mut().find(|n| same(n.rect, t.rect)) {
            if !matches!(node.kind, Kind::Tab(_)) {
                node.kind = t.kind;
            }
        } else {
            nodes.push(Node {
                id: String::new(),
                kind: t.kind,
                label: None,
                rect: t.rect,
                enabled: true,
            });
        }
    }
    if let Some([w, h]) = screen {
        nodes.retain(|n| {
            let [x, y, nw, nh] = n.rect;
            x >= -0.5 && y >= -0.5 && x + nw <= w + 0.5 && y + nh <= h + 0.5
        });
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for node in &mut nodes {
        let base = node
            .label
            .as_deref()
            .map(slug)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| node.kind.name().to_owned());
        let n = seen.entry(base.clone()).or_default();
        *n += 1;
        node.id = if node.label.is_none() {
            format!("{base}-{}", *n - 1)
        } else if *n > 1 {
            format!("{base}-{n}")
        } else {
            base
        };
    }
    nodes
}

/// The button `label` names: an exact match (case-insensitive), else the only
/// button whose label contains it. The top-most wins among equals.
pub fn find(widgets: &[Widget], label: &str) -> Result<Widget, String> {
    let want = label.trim().to_lowercase();
    let exact: Vec<&Widget> = widgets
        .iter()
        .filter(|w| w.label.to_lowercase() == want)
        .collect();
    if let Some(w) = exact.last() {
        return Ok((*w).clone());
    }
    let partial: Vec<&Widget> = widgets
        .iter()
        .filter(|w| w.label.to_lowercase().contains(&want))
        .collect();
    match partial.as_slice() {
        [w] => Ok((*w).clone()),
        [] => Err(format!(
            "no button labelled {label:?}; buttons: {:?}",
            widgets.iter().map(|w| w.label.as_str()).collect::<Vec<_>>()
        )),
        many => Err(format!(
            "{label:?} matches several buttons: {:?}",
            many.iter().map(|w| w.label.as_str()).collect::<Vec<_>>()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(label: &str) -> Widget {
        Widget {
            label: label.into(),
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            enabled: true,
            kind: Kind::Button,
        }
    }

    #[test]
    fn finds_exact_then_unique_partial() {
        let list = [w("Play Selected World"), w("Create New World"), w("Back")];
        assert_eq!(find(&list, "back").unwrap().label, "Back");
        assert_eq!(find(&list, "create").unwrap().label, "Create New World");
        assert!(find(&list, "world").is_err());
        assert!(find(&list, "options").is_err());
    }

    #[test]
    fn tree_merges_labels_with_targets_and_names_them() {
        let slider = Kind::Slider {
            value: 0.25,
            track: [4.0, 142.0],
        };
        let mut fov = w("FOV: 70");
        fov.kind = slider;
        let mut done = w("Done");
        done.y = 30.0;
        let mut done2 = w("Done");
        done2.y = 60.0;
        let widgets = [fov, w("Video Settings..."), done, done2];
        let targets = [
            Target {
                rect: [0.0, 0.0, 10.0, 10.0],
                kind: slider,
            },
            Target {
                rect: [0.0, 90.0, 10.0, 10.0],
                kind: Kind::Row,
            },
            Target {
                rect: [0.0, 100.0, 10.0, 10.0],
                kind: Kind::Row,
            },
        ];
        let mut probe = w("Done");
        probe.x = -10_000.0;
        let widgets = [&[probe][..], &widgets].concat();
        let nodes = tree(&widgets, &targets, Some([400.0, 200.0]));
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            ["fov", "video-settings", "done", "done-2", "row-0", "row-1"]
        );
        assert_eq!(nodes[0].kind, slider, "the target's kind carries the value");
        assert_eq!(nodes[4].label, None);
        assert_eq!(slug("Music & Sounds..."), "music-sounds");
        assert_eq!(slug("Difficulty: Normal"), "difficulty");
        assert_eq!(slug("Économie"), "économie");
    }

    #[test]
    fn last_lists_buttons_only() {
        end_frame(100);
        record("Back", [0.0, 0.0, 5.0, 5.0], true);
        record_as("FOV: 70", [0.0, 9.0, 5.0, 5.0], true, Kind::Tab(true));
        end_frame(101);
        assert_eq!(last().len(), 1);
        assert_eq!(last_labelled().len(), 2);
    }

    #[test]
    fn frames_rotate_only_when_drawn() {
        record("Respawn", [1.0, 2.0, 3.0, 4.0], true);
        target_as(
            [1.0, 2.0, 3.0, 4.0],
            Kind::Slider {
                value: 0.5,
                track: [1.0, 3.0],
            },
        );
        target_as([1.0, 2.0, 0.0, 4.0], Kind::Row);
        let before = generation();
        end_frame(1);
        assert_eq!(last().len(), 1);
        assert_eq!(last_targets().len(), 1, "an empty rectangle isn't a target");
        assert!(matches!(last_targets()[0].kind, Kind::Slider { value, .. } if value == 0.5));
        assert_eq!(last_targets()[0].center(), (2.5, 4.0));
        end_frame(1);
        assert_eq!(last().len(), 1);
        assert_eq!(generation(), before + 1);
        end_frame(2);
        assert!(last().is_empty() && last_targets().is_empty());
        target([0.0, 0.0, 5.0, 5.0]);
        let mark = targets_mark();
        target([0.0, 10.0, 5.0, 5.0]);
        target([0.0, 40.0, 5.0, 5.0]);
        clip_targets(mark, 8.0, 30.0);
        end_frame(3);
        assert_eq!(
            last_targets().len(),
            2,
            "the clipped row is gone, earlier ones stay"
        );
    }
}
