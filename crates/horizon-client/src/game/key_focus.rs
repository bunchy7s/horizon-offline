//! Vanilla's keyboard focus navigation in Horizon's menus
//! (`Screen.keyPressed`, `ContainerEventHandler.nextFocusPath`, 26.3): Tab
//! and Shift+Tab step the focus in tab order, the arrows move it the way they
//! point, Enter, Space and keypad Enter press the focused widget
//! (`AbstractButton.keyPressed`), and a focused slider's left and right move
//! its value once Enter (or Tab) let them (`AbstractSliderButton.keyPressed`).
//!
//! The focus lands on what the controller's focus mode reaches: the targets
//! each screen registers (`widgets::target_as`, docs/controllers.md). This
//! module is the pure part: which key does what, the tab order, the slider
//! rules. `pad_menu.rs` applies it in the game (it shares the controller's
//! focus), `startup_menu.rs` on the title shown while resources load.
use horizon_client::{
    gamepad::{self, Button, Rect},
    hud::{HudAtlas, HudBatch, rgba},
    widgets::{self, Kind, Target},
};
use winit::keyboard::KeyCode;

/// A navigation key, as `Screen.keyPressed` and the widgets read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Nav {
    /// Tab (forward) or Shift+Tab (`FocusNavigationEvent.TabNavigation`).
    Tab { forward: bool },
    /// An arrow (`ArrowNavigation`), as the D-pad direction it steps.
    Arrow(Button),
    /// Enter, Space or keypad Enter (`InputWithModifiers.isSelection`).
    Select,
}

/// The navigation a key press is, if any.
pub(super) fn nav_of(key: KeyCode, shift: bool) -> Option<Nav> {
    Some(match key {
        KeyCode::Tab => Nav::Tab { forward: !shift },
        KeyCode::ArrowUp => Nav::Arrow(Button::DPadUp),
        KeyCode::ArrowDown => Nav::Arrow(Button::DPadDown),
        KeyCode::ArrowLeft => Nav::Arrow(Button::DPadLeft),
        KeyCode::ArrowRight => Nav::Arrow(Button::DPadRight),
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Nav::Select,
        _ => return None,
    })
}

/// What a navigation key does on the screen as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Act {
    /// The screen takes the key itself: a text field's caret, Enter on a
    /// list whose screen joins its selected row, nothing focused.
    Pass,
    /// Focus `targets[i]` (Tab, or an arrow from no focus).
    Focus(usize),
    /// Move the focus that way (the controller's step, which scrolls lists).
    Step(Button),
    /// Press the focused widget (a click on it).
    Press,
    /// Enter on a focused slider: whether the arrows change its value flips.
    ToggleSlider,
    /// Left (-1) or right (1) on a slider that can change: one step.
    Nudge(f32),
}

/// Where the screen is: what `act` reads besides the focus.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Context {
    /// The focused slider's arrows change its value (`canChangeValue`).
    pub slider_edit: bool,
    /// A text field has the keyboard (a click focused it): with nothing
    /// focused, its caret takes left and right.
    pub typing: bool,
    /// The screen joins its list's selected row on Enter itself
    /// (`WorldListEntry.keyPressed`): a focused row passes Enter on.
    pub rows_take_enter: bool,
    /// The GUI size: an arrow from no focus starts at the screen's edge.
    pub gui: (f32, f32),
}

fn horizontal(button: Button) -> bool {
    matches!(button, Button::DPadLeft | Button::DPadRight)
}

/// What `nav` does, `focus` being the focused target (the one a click
/// focused counts) among `targets`.
pub(super) fn act(nav: Nav, focus: Option<Target>, targets: &[Target], ctx: Context) -> Act {
    match (nav, focus) {
        (Nav::Tab { forward }, _) => {
            tab_step(targets, focus.map(|t| t.rect), forward).map_or(Act::Pass, Act::Focus)
        }
        // EditBox.keyPressed takes left and right for its caret.
        (
            Nav::Arrow(b),
            Some(Target {
                kind: Kind::Field, ..
            }),
        ) if horizontal(b) => Act::Pass,
        (Nav::Arrow(b), None) if horizontal(b) && ctx.typing => Act::Pass,
        (
            Nav::Arrow(b),
            Some(Target {
                kind: Kind::Slider { .. },
                ..
            }),
        ) if horizontal(b) && ctx.slider_edit => {
            Act::Nudge(if b == Button::DPadLeft { -1.0 } else { 1.0 })
        }
        (Nav::Arrow(b), Some(_)) => Act::Step(b),
        // No focus: from the screen's edge behind the arrow
        // (`getBorderForArrowNavigation(direction.getOpposite())`).
        (Nav::Arrow(b), None) => {
            let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
            gamepad::focus_step_rect(edge(ctx.gui, b), b, &rects).map_or(Act::Pass, Act::Focus)
        }
        (Nav::Select, None) => Act::Pass,
        (Nav::Select, Some(t)) => match t.kind {
            Kind::Field => Act::Pass,
            Kind::Row if ctx.rows_take_enter => Act::Pass,
            Kind::Slider { .. } => Act::ToggleSlider,
            _ => Act::Press,
        },
    }
}

/// The strip just outside the screen on the side an arrow comes from.
fn edge((w, h): (f32, f32), button: Button) -> Rect {
    match button {
        Button::DPadUp => [0.0, h, w, 1.0],
        Button::DPadLeft => [w, 0.0, 1.0, h],
        Button::DPadRight => [-1.0, 0.0, 1.0, h],
        _ => [0.0, -1.0, w, 1.0],
    }
}

/// `AbstractSliderButton.setFocused`: the arrows change a slider's value
/// when the focus came from the mouse or Tab, not from an arrow.
pub(super) fn slider_edit_on_focus(nav: Nav) -> bool {
    matches!(nav, Nav::Tab { .. })
}

fn same(a: Rect, b: Rect) -> bool {
    a.iter().zip(b).all(|(p, q)| (p - q).abs() < 0.5)
}

/// The tab order (`children()` sorted by `getTabOrderGroup`): the order
/// vanilla added the widgets in, which its layouts do row by row. Horizon
/// has no widget tree (a screen registers buttons and other targets
/// separately), so this is reading order: top to bottom, then left to right,
/// ties in the order they were drawn.
pub(super) fn tab_order(targets: &[Target]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..targets.len()).collect();
    let key = |t: &Target| (t.rect[1].round() as i32, t.rect[0].round() as i32);
    order.sort_by_key(|&i| key(&targets[i]));
    order
}

/// Tab or Shift+Tab from `from` (none focused: from the start or the end),
/// wrapping as `Screen.keyPressed` does (no next one: `clearFocus`, then
/// again). A list's rows are one stop, as vanilla's list is one widget: Tab
/// enters it on its first row (Shift+Tab on its last) and leaves it at once.
pub(super) fn tab_step(targets: &[Target], from: Option<Rect>, forward: bool) -> Option<usize> {
    // The stops in tab order: a run of rows is one.
    let mut stops: Vec<Vec<usize>> = Vec::new();
    for i in tab_order(targets) {
        let row = targets[i].kind == Kind::Row;
        match stops.last_mut() {
            Some(stop) if row && targets[stop[0]].kind == Kind::Row => stop.push(i),
            _ => stops.push(vec![i]),
        }
    }
    if stops.is_empty() {
        return None;
    }
    let at = from.and_then(|r| {
        stops
            .iter()
            .position(|s| s.iter().any(|&i| same(targets[i].rect, r)))
    });
    let n = stops.len();
    let next = match at {
        Some(k) if forward => (k + 1) % n,
        Some(k) => (k + n - 1) % n,
        None if forward => 0,
        None => n - 1,
    };
    let stop = &stops[next];
    Some(if forward {
        stop[0]
    } else {
        stop[stop.len() - 1]
    })
}

/// What the last drawn frame's screen can focus: its enabled buttons and
/// every target it registered (sliders, fields, rows, slots...).
pub(super) fn drawn_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = widgets::last()
        .iter()
        .filter(|w| w.enabled)
        .map(|w| Target {
            rect: [w.x, w.y, w.w, w.h],
            kind: Kind::Button,
        })
        .collect();
    for t in widgets::last_targets() {
        // A button can be both a labelled button and a target.
        if let Some(same) = targets.iter_mut().find(|o| o.rect == t.rect) {
            same.kind = t.kind;
        } else {
            targets.push(t);
        }
    }
    targets
}

/// The targets inside the GUI (`gui`: its size in GUI pixels).
pub(super) fn on_screen(mut targets: Vec<Target>, (w, h): (f32, f32)) -> Vec<Target> {
    let (w, h) = (w + 0.5, h + 0.5);
    targets.retain(|t| {
        let [x, y, tw, th] = t.rect;
        x >= -0.5 && y >= -0.5 && x + tw <= w && y + th <= h
    });
    targets
}

/// Which targets the keyboard reaches: vanilla focuses widgets only, never
/// an inventory's slots, and the creative and recipe-book tabs aren't the
/// screen's widgets either (`inventory`).
pub(super) fn keyboard_targets(targets: Vec<Target>, inventory: bool) -> Vec<Target> {
    targets
        .into_iter()
        .filter(|t| match t.kind {
            Kind::Slot => false,
            Kind::Tab(_) => !inventory,
            _ => true,
        })
        .collect()
}

/// The focus in a newly drawn frame: the same widget, or one the screen
/// moved a little (a list scrolled); else the screen changed and the focus
/// starts again on the first widget (`Screen.setInitialFocus` after a key).
pub(super) fn follow(focus: Target, targets: &[Target]) -> Option<Target> {
    let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
    gamepad::refind(focus.rect, &rects, 12.0)
        .or_else(|| tab_step(targets, None, true))
        .map(|i| targets[i])
}

/// Where a click focuses a text field with its caret at the end of the
/// text, as Tab leaves it (`EditBox.setFocused` keeps the cursor, which
/// `setValue` put at the end): the box's right end.
pub(super) fn field_click(rect: Rect) -> (f32, f32) {
    let [x, y, w, h] = rect;
    (x + w - 2.0, y + h / 2.0)
}

/// Vanilla's focus look for what has no highlighted sprite: the white
/// one-pixel outline of a focused list's selection
/// (`AbstractSelectionList`) and of a focused `EditBox`. Buttons, sliders
/// and tabs show their highlighted sprite, drawn under the hidden pointer.
pub(super) fn draw_outline(atlas: &HudAtlas, batch: &mut HudBatch, focus: Target) {
    let [x, y, w, h] = focus.rect;
    if matches!(focus.kind, Kind::Row | Kind::Field) {
        let white = rgba(255, 255, 255, 255);
        batch.fill(atlas, x, y, w, 1.0, white);
        batch.fill(atlas, x, y + h - 1.0, w, 1.0, white);
        batch.fill(atlas, x, y, 1.0, h, white);
        batch.fill(atlas, x + w - 1.0, y, 1.0, h, white);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(x: f32, y: f32) -> Target {
        Target {
            rect: [x, y, 98.0, 20.0],
            kind: Kind::Button,
        }
    }
    fn row(y: f32) -> Target {
        Target {
            rect: [100.0, y, 220.0, 36.0],
            kind: Kind::Row,
        }
    }
    fn slider() -> Target {
        Target {
            rect: [0.0, 0.0, 150.0, 20.0],
            kind: Kind::Slider {
                value: 0.5,
                track: [4.0, 142.0],
            },
        }
    }
    fn field() -> Target {
        Target {
            rect: [0.0, 0.0, 200.0, 20.0],
            kind: Kind::Field,
        }
    }
    const CTX: Context = Context {
        slider_edit: false,
        typing: false,
        rows_take_enter: false,
        gui: (427.0, 240.0),
    };

    #[test]
    fn keys_map_to_vanillas_navigation() {
        assert_eq!(
            nav_of(KeyCode::Tab, false),
            Some(Nav::Tab { forward: true })
        );
        assert_eq!(
            nav_of(KeyCode::Tab, true),
            Some(Nav::Tab { forward: false })
        );
        assert_eq!(
            nav_of(KeyCode::ArrowUp, false),
            Some(Nav::Arrow(Button::DPadUp))
        );
        assert_eq!(
            nav_of(KeyCode::ArrowRight, true),
            Some(Nav::Arrow(Button::DPadRight))
        );
        // isSelection: Enter (40), Space (44), keypad Enter (88).
        for key in [KeyCode::Enter, KeyCode::Space, KeyCode::NumpadEnter] {
            assert_eq!(nav_of(key, false), Some(Nav::Select));
        }
        assert_eq!(nav_of(KeyCode::KeyA, false), None);
        assert_eq!(nav_of(KeyCode::Escape, false), None);
    }

    /// The title screen's rows (TitleScreen.init's add order).
    #[test]
    fn tab_order_reads_rows_top_to_bottom_then_left_to_right() {
        let targets = [
            button(164.0, 108.0), // Options
            button(266.0, 108.0), // Quit
            button(164.0, 60.0),  // Singleplayer
            Target {
                rect: [140.0, 84.0, 20.0, 20.0],
                kind: Kind::Button,
            }, // Friends
            button(164.0, 84.0),  // Realms
        ];
        assert_eq!(tab_order(&targets), vec![2, 3, 4, 0, 1]);
    }

    #[test]
    fn tab_wraps_both_ways_and_starts_at_either_end() {
        let targets = [button(0.0, 0.0), button(0.0, 30.0), button(0.0, 60.0)];
        assert_eq!(tab_step(&targets, None, true), Some(0));
        assert_eq!(tab_step(&targets, None, false), Some(2));
        assert_eq!(tab_step(&targets, Some(targets[0].rect), true), Some(1));
        assert_eq!(tab_step(&targets, Some(targets[2].rect), true), Some(0));
        assert_eq!(tab_step(&targets, Some(targets[0].rect), false), Some(2));
        // A rect no longer there: from the start.
        assert_eq!(tab_step(&targets, Some([9.0; 4]), true), Some(0));
        assert_eq!(tab_step(&[], None, true), None);
    }

    #[test]
    fn a_lists_rows_are_one_tab_stop() {
        let search = field();
        let targets = [search, row(40.0), row(76.0), row(112.0), button(0.0, 200.0)];
        // Into the list on its first row, then out of it.
        assert_eq!(tab_step(&targets, Some(search.rect), true), Some(1));
        assert_eq!(tab_step(&targets, Some(targets[2].rect), true), Some(4));
        // Backwards: in on the last row, out to the search box.
        assert_eq!(tab_step(&targets, Some(targets[4].rect), false), Some(3));
        assert_eq!(tab_step(&targets, Some(targets[1].rect), false), Some(0));
    }

    #[test]
    fn select_presses_buttons_and_toggles_sliders() {
        let b = button(0.0, 0.0);
        assert_eq!(act(Nav::Select, Some(b), &[b], CTX), Act::Press);
        let s = slider();
        assert_eq!(act(Nav::Select, Some(s), &[s], CTX), Act::ToggleSlider);
        // Nothing focused: the screen's own Enter (a world list joins).
        assert_eq!(act(Nav::Select, None, &[b], CTX), Act::Pass);
        let f = field();
        assert_eq!(act(Nav::Select, Some(f), &[f], CTX), Act::Pass);
        let r = row(40.0);
        assert_eq!(act(Nav::Select, Some(r), &[r], CTX), Act::Press);
        let joins = Context {
            rows_take_enter: true,
            ..CTX
        };
        assert_eq!(act(Nav::Select, Some(r), &[r], joins), Act::Pass);
    }

    /// AbstractSliderButton.keyPressed and setFocused.
    #[test]
    fn slider_arrows_change_the_value_only_once_allowed() {
        let s = slider();
        let left = Nav::Arrow(Button::DPadLeft);
        let right = Nav::Arrow(Button::DPadRight);
        assert_eq!(
            act(left, Some(s), &[s], CTX),
            Act::Step(Button::DPadLeft),
            "focused by an arrow: left moves the focus"
        );
        let edit = Context {
            slider_edit: true,
            ..CTX
        };
        assert_eq!(act(left, Some(s), &[s], edit), Act::Nudge(-1.0));
        assert_eq!(act(right, Some(s), &[s], edit), Act::Nudge(1.0));
        // Up and down always move the focus.
        assert_eq!(
            act(Nav::Arrow(Button::DPadDown), Some(s), &[s], edit),
            Act::Step(Button::DPadDown)
        );
        assert!(slider_edit_on_focus(Nav::Tab { forward: true }));
        assert!(slider_edit_on_focus(Nav::Tab { forward: false }));
        assert!(!slider_edit_on_focus(Nav::Arrow(Button::DPadDown)));
    }

    /// EditBox.keyPressed consumes left and right, not Tab, up or down.
    #[test]
    fn a_text_field_keeps_left_and_right() {
        let f = field();
        let b = button(0.0, 40.0);
        let targets = [f, b];
        for b in [Button::DPadLeft, Button::DPadRight] {
            assert_eq!(act(Nav::Arrow(b), Some(f), &targets, CTX), Act::Pass);
        }
        assert_eq!(
            act(Nav::Arrow(Button::DPadDown), Some(f), &targets, CTX),
            Act::Step(Button::DPadDown)
        );
        assert_eq!(
            act(Nav::Tab { forward: true }, Some(f), &targets, CTX),
            Act::Focus(1)
        );
        // A field a click focused (no keyboard focus) keeps them too.
        let typing = Context {
            typing: true,
            ..CTX
        };
        assert_eq!(
            act(Nav::Arrow(Button::DPadLeft), None, &targets, typing),
            Act::Pass
        );
    }

    #[test]
    fn an_arrow_from_no_focus_starts_at_the_screens_edge() {
        let targets = [
            button(164.0, 60.0),
            button(164.0, 84.0),
            button(10.0, 200.0),
        ];
        let down = act(Nav::Arrow(Button::DPadDown), None, &targets, CTX);
        assert_eq!(down, Act::Focus(0));
        let up = act(Nav::Arrow(Button::DPadUp), None, &targets, CTX);
        assert_eq!(up, Act::Focus(2));
        let right = act(Nav::Arrow(Button::DPadRight), None, &targets, CTX);
        assert_eq!(right, Act::Focus(2));
    }

    #[test]
    fn the_keyboard_skips_slots_and_inventory_tabs() {
        let slot = Target {
            rect: [0.0, 0.0, 16.0, 16.0],
            kind: Kind::Slot,
        };
        let tab = Target {
            rect: [0.0, 0.0, 26.0, 32.0],
            kind: Kind::Tab(false),
        };
        let b = button(0.0, 0.0);
        assert_eq!(keyboard_targets(vec![slot, tab, b], true), vec![b]);
        assert_eq!(keyboard_targets(vec![slot, tab, b], false), vec![tab, b]);
    }

    #[test]
    fn focus_follows_its_widget_or_starts_again() {
        let a = button(0.0, 0.0);
        let moved = button(0.0, 4.0);
        let far = button(200.0, 100.0);
        assert_eq!(follow(a, &[far, moved]), Some(moved));
        // Gone (another screen): the first in tab order.
        let first = button(0.0, 50.0);
        assert_eq!(follow(a, &[far, first]), Some(first));
        assert_eq!(follow(a, &[]), None);
    }

    #[test]
    fn a_field_is_clicked_at_its_end() {
        assert_eq!(field_click([10.0, 20.0, 200.0, 20.0]), (208.0, 30.0));
    }
}
