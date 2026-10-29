//! `DialogScreen` and its kinds (`SimpleDialogScreen`, `MultiButtonDialogScreen`):
//! where a server dialog's title, body, inputs and buttons go in a
//! `HeaderAndFooterLayout` (a 33 high header, the body in a `ScrollableLayout`,
//! the footer), and the `WaitingForResponseScreen` an answer can leave open.
use crate::{
    dialog::{Body, Control, Dialog, Kind},
    font::Font,
    text::Text,
};

/// `HeaderAndFooterLayout.DEFAULT_HEADER_AND_FOOTER_HEIGHT`.
const BAR: f32 = 33.0;
/// `Checkbox.getBoxSize`.
pub const BOX: f32 = 17.0;
/// `DialogScreen.WARNING_BUTTON_SPRITES`: normal, disabled, highlighted.
pub const WARNING_SPRITES: [&str; 3] = [
    "gui/sprites/dialog/warning_button",
    "gui/sprites/dialog/warning_button_disabled",
    "gui/sprites/dialog/warning_button_highlighted",
];

/// One drawn piece; `y` is from the top of the body (before it scrolls) for
/// the body's parts, from the top of the screen for the footer's.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// A line of text and where it starts.
    Line(Text, f32, f32),
    /// A text input's box.
    Field { input: usize, rect: [f32; 4] },
    /// A boolean input: the box, its label right of it.
    Check { input: usize, rect: [f32; 4] },
    /// A button of `Dialog::buttons`.
    Button { index: usize, rect: [f32; 4] },
    /// A single option input's cycle button.
    Cycle { input: usize, rect: [f32; 4] },
    /// A number range input's slider.
    Slider { input: usize, rect: [f32; 4] },
    /// An item body's widget; `body` indexes `Dialog::body`.
    Item { body: usize, rect: [f32; 4] },
}
impl Part {
    /// The part moved by `(dx, dy)`.
    fn moved(self, dx: f32, dy: f32) -> Self {
        match self {
            Self::Line(t, x, y) => Self::Line(t, x + dx, y + dy),
            Self::Field { input, rect } => Self::Field {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Check { input, rect } => Self::Check {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Button { index, rect } => Self::Button {
                index,
                rect: shift(rect, dx, dy),
            },
            Self::Cycle { input, rect } => Self::Cycle {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Slider { input, rect } => Self::Slider {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Item { body, rect } => Self::Item {
                body,
                rect: shift(rect, dx, dy),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    /// The title and where it is drawn.
    pub title: (f32, f32),
    pub warning: [f32; 4],
    /// The body's visible area (`ScrollableLayout`'s container): x, y, w, h.
    pub area: [f32; 4],
    /// The body's full height: more than the area's scrolls.
    pub content: f32,
    pub body: Vec<Part>,
    pub footer: Vec<Part>,
}

/// A laid-out element of a `LinearLayout` before it is placed.
struct Element {
    w: f32,
    h: f32,
    parts: Vec<Part>,
}
impl Element {
    /// Its parts moved to `(x, y)`.
    fn at(self, x: f32, y: f32) -> impl Iterator<Item = Part> {
        self.parts.into_iter().map(move |p| p.moved(x, y))
    }
}
fn shift([x, y, w, h]: [f32; 4], dx: f32, dy: f32) -> [f32; 4] {
    [x + dx, y + dy, w, h]
}
/// `FrameLayout.alignInDimension` at 0.5: the offset that centres `inner`.
fn centre(outer: f32, inner: f32) -> f32 {
    ((outer - inner) * 0.5).trunc()
}

/// `FocusableTextWidget` with the body's settings: `width` wide, the text
/// wrapped to `width - 8` and centred, 4 of padding all round.
fn message(font: &Font, contents: &Text, width: f32) -> Element {
    let lines = font.split_lines(contents, width - 8.0);
    let mid = (width / 2.0).trunc();
    let parts = lines
        .into_iter()
        .enumerate()
        .map(|(i, (line, _))| {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            Part::Line(line, x, 4.0 + 9.0 * i as f32)
        })
        .collect::<Vec<_>>();
    Element {
        w: width,
        h: 9.0 * parts.len() as f32 + 8.0,
        parts,
    }
}

/// `DialogBodyHandlers.ItemHandler`: the item widget, with its description 2
/// to the right of it, both centred on each other vertically.
fn item(
    font: &Font,
    body: usize,
    description: Option<&(Text, i32)>,
    (w, h): (f32, f32),
) -> Element {
    let widget = |rect_h: f32, y: f32| Part::Item {
        body,
        rect: [0.0, y, w, rect_h],
    };
    let Some((contents, width)) = description else {
        return Element {
            w,
            h,
            parts: vec![widget(h, 0.0)],
        };
    };
    let text = message(font, contents, *width as f32);
    let height = h.max(text.h);
    let mut parts = vec![widget(h, centre(height, h))];
    parts.extend(
        text.parts
            .into_iter()
            .map(|p| p.moved(w + 2.0, centre(height, text.h))),
    );
    Element {
        w: w + 2.0 + text.w,
        h: height,
        parts,
    }
}

/// `InputControlHandlers`: the control for input `i`.
fn input(font: &Font, i: usize, control: &Control) -> Element {
    match control {
        Control::Text {
            width,
            label,
            label_visible,
            ..
        } => {
            let w = *width as f32;
            if !label_visible {
                return Element {
                    w,
                    h: 20.0,
                    parts: vec![Part::Field {
                        input: i,
                        rect: [0.0, 0.0, w, 20.0],
                    }],
                };
            }
            // CommonLayouts.labeledElement: the label, 4 below it the box,
            // both on the left.
            Element {
                w: w.max(font.width_text(label)),
                h: 9.0 + 4.0 + 20.0,
                parts: vec![
                    Part::Line(label.clone(), 0.0, 0.0),
                    Part::Field {
                        input: i,
                        rect: [0.0, 13.0, w, 20.0],
                    },
                ],
            }
        }
        Control::Boolean { label, .. } => {
            let w = BOX + 4.0 + font.width_text(label);
            Element {
                w,
                h: BOX,
                parts: vec![Part::Check {
                    input: i,
                    rect: [0.0, 0.0, w, BOX],
                }],
            }
        }
        // CycleButton and AbstractSliderButton: one 20 high button whose
        // message holds the label.
        Control::SingleOption { width, .. } => Element {
            w: *width as f32,
            h: 20.0,
            parts: vec![Part::Cycle {
                input: i,
                rect: [0.0, 0.0, *width as f32, 20.0],
            }],
        },
        Control::NumberRange { width, .. } => Element {
            w: *width as f32,
            h: 20.0,
            parts: vec![Part::Slider {
                input: i,
                rect: [0.0, 0.0, *width as f32, 20.0],
            }],
        },
    }
}

/// `DialogScreen.packControlsIntoColumns`: full rows in a grid 2 apart, what
/// is left in a centred row under them.
fn columns(widths: &[f32], columns: usize, first: usize) -> Element {
    let columns = columns.max(1);
    let full = widths.len() / columns * columns;
    let row_width = |ws: &[f32]| ws.iter().sum::<f32>() + 2.0 * (ws.len().max(1) - 1) as f32;
    let mut col = vec![0.0f32; columns];
    for (i, w) in widths[..full].iter().enumerate() {
        col[i % columns] = col[i % columns].max(*w);
    }
    let last = &widths[full..];
    if !last.is_empty() {
        // GridLayout spreads a cell over every column it spans (Divisor).
        let spread = row_width(last) - 2.0 * (columns - 1) as f32;
        let (each, extra) = ((spread / columns as f32).floor(), spread % columns as f32);
        for (c, w) in col.iter_mut().enumerate() {
            *w = w.max(each + f32::from(u8::from((c as f32) < extra)));
        }
    }
    let x_of = |c: usize| col[..c].iter().sum::<f32>() + 2.0 * c as f32;
    let mut parts = Vec::new();
    for (i, w) in widths[..full].iter().enumerate() {
        let (r, c) = (i / columns, i % columns);
        parts.push(Part::Button {
            index: first + i,
            rect: [x_of(c) + centre(col[c], *w), 22.0 * r as f32, *w, 20.0],
        });
    }
    let width = x_of(columns - 1) + col[columns - 1];
    if !last.is_empty() {
        let y = 22.0 * (full / columns) as f32;
        let mut x = centre(width, row_width(last));
        for (i, w) in last.iter().enumerate() {
            parts.push(Part::Button {
                index: first + full + i,
                rect: [x, y, *w, 20.0],
            });
            x += w + 2.0;
        }
    }
    let rows = full / columns + usize::from(!last.is_empty());
    Element {
        w: width,
        h: (22.0 * rows as f32 - 2.0).max(0.0),
        parts,
    }
}

impl Layout {
    /// `DialogScreen.init` and `repositionElements` on a `w` by `h` screen.
    pub fn new(dialog: &Dialog, font: &Font, (w, h): (f32, f32)) -> Self {
        // The header: the title, 10 right of it the warning button, both
        // centred together.
        let tw = font.width_text(&dialog.title);
        let (hx, hy) = (centre(w, tw + 30.0), centre(BAR, 20.0));
        let title = (hx, hy + centre(20.0, 9.0));
        let warning = [hx + tw + 10.0, hy, 20.0, 20.0];

        let mut elements: Vec<Element> = dialog
            .body
            .iter()
            .enumerate()
            .map(|(i, b)| match b {
                Body::Message { contents, width } => message(font, contents, *width as f32),
                Body::Item {
                    description,
                    width,
                    height,
                    ..
                } => item(
                    font,
                    i,
                    description.as_ref(),
                    (*width as f32, *height as f32),
                ),
            })
            .collect();
        elements.extend(
            dialog
                .inputs
                .iter()
                .enumerate()
                .map(|(i, input_)| input(font, i, &input_.control)),
        );
        let buttons = dialog.buttons();
        let (listed, footer_buttons, footer_h) = match &dialog.kind {
            Kind::MultiAction {
                exit, columns: n, ..
            }
            | Kind::DialogList {
                exit, columns: n, ..
            } => {
                let listed = buttons.len() - usize::from(exit.is_some());
                let widths: Vec<f32> = buttons[..listed].iter().map(|b| b.width as f32).collect();
                elements.push(columns(&widths, *n as usize, 0));
                // ButtonListDialogScreen: the exit button in the footer, else a 5 high one.
                let footer = exit.as_ref().map(|_| listed).into_iter().collect();
                (listed, footer, if exit.is_some() { BAR } else { 5.0 })
            }
            Kind::ServerLinks { exit, .. } => {
                let footer = exit.as_ref().map(|_| 0).into_iter().collect();
                (0, footer, if exit.is_some() { BAR } else { 5.0 })
            }
            // SimpleDialogScreen: the buttons side by side, 8 apart.
            Kind::Notice(_) | Kind::Confirmation { .. } => {
                (0, (0..buttons.len()).collect::<Vec<_>>(), BAR)
            }
        };
        debug_assert!(listed + footer_buttons.len() <= buttons.len());

        // The body: a column 10 apart, each element centred in it.
        let body_w = elements.iter().map(|e| e.w).fold(0.0, f32::max);
        let content = elements.iter().map(|e| e.h).sum::<f32>()
            + 10.0 * elements.len().saturating_sub(1) as f32;
        // ScrollableLayout: 10 each side for the scrollbar (ReserveStrategy.BOTH),
        // no higher than what the header and footer leave.
        let area_w = body_w + 20.0;
        let area_h = content.min(h - BAR - footer_h).max(0.0);
        let area_y = (BAR + 30.0).min(h - footer_h - area_h);
        let area_x = centre(w, area_w);
        let mut body = Vec::new();
        let mut y = 0.0;
        for e in elements {
            let (x, eh) = (area_x + 10.0 + centre(body_w, e.w), e.h);
            body.extend(e.at(x, y));
            y += eh + 10.0;
        }

        let widths: Vec<f32> = footer_buttons
            .iter()
            .map(|&i| buttons[i].width as f32)
            .collect();
        let row = widths.iter().sum::<f32>() + 8.0 * widths.len().saturating_sub(1) as f32;
        let (mut x, fy) = (centre(w, row), h - footer_h + centre(footer_h, 20.0));
        let mut footer = Vec::new();
        for (&i, bw) in footer_buttons.iter().zip(&widths) {
            footer.push(Part::Button {
                index: i,
                rect: [x, fy, *bw, 20.0],
            });
            x += bw + 8.0;
        }
        Self {
            title,
            warning: Self::in_bounds(warning, w, h),
            area: [area_x, area_y, area_w, area_h],
            content,
            body,
            footer,
        }
    }
    /// `makeSureWarningButtonIsInBounds`.
    fn in_bounds(r: [f32; 4], w: f32, h: f32) -> [f32; 4] {
        if r[0] < 0.0 || r[1] < 0.0 || r[0] > w - 20.0 || r[1] > h - 20.0 {
            [(w - 40.0).max(0.0), 5.0f32.min(h), 20.0, 20.0]
        } else {
            r
        }
    }
    /// Every part where it is drawn with the body scrolled by `scroll`, and
    /// whether it is in the body (clipped to its area).
    pub fn placed(&self, scroll: f32) -> impl Iterator<Item = (Part, bool)> + '_ {
        let dy = self.area[1] - scroll;
        let body = self
            .body
            .iter()
            .map(move |p| (p.clone().moved(0.0, dy), true));
        body.chain(self.footer.iter().map(|p| (p.clone(), false)))
    }
}

/// `WaitingForResponseScreen`: its Back button's label after `ticks`, and
/// whether it shows and works (it shows after a second, works after five).
pub fn waiting_button(ticks: u32) -> (Text, bool, bool) {
    let seconds = (ticks / 20).min(5);
    let label = match seconds {
        0 => Text::literal(""),
        5 => Text::translate("gui.back", &[]),
        s => Text::translate(
            "gui.waitingForResponse.button.inactive",
            &[Text::literal((5 - s).to_string())],
        ),
    };
    (label, seconds >= 1, seconds == 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_pack_full_rows_and_centre_the_rest() {
        // AuthMe's two buttons: one row, 2 apart.
        let two = columns(&[150.0, 150.0], 2, 0);
        assert_eq!((two.w, two.h), (302.0, 20.0));
        // Three in two columns: the third alone, centred under them.
        let three = columns(&[150.0, 150.0, 150.0], 2, 0);
        assert_eq!((three.w, three.h), (302.0, 42.0));
        assert_eq!(
            three.parts[2],
            Part::Button {
                index: 2,
                rect: [76.0, 22.0, 150.0, 20.0]
            }
        );
        // One in two columns spreads over both.
        let one = columns(&[100.0], 2, 0);
        assert_eq!(one.w, 100.0);
    }

    #[test]
    fn the_waiting_screens_button_counts_down() {
        assert!(!waiting_button(0).1);
        assert_eq!(
            waiting_button(20).0,
            Text::translate(
                "gui.waitingForResponse.button.inactive",
                &[Text::literal("4")]
            )
        );
        assert_eq!(
            waiting_button(100),
            (Text::translate("gui.back", &[]), true, true)
        );
    }
}
