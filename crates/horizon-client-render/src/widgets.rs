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
