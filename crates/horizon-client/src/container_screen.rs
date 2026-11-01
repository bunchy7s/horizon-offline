//! The mouse handling every survival and container screen shares
//! (`AbstractContainerScreen.mouseClicked` / `mouseReleased`): which slot id
//! a click targets (-999 outside the window, `hasClickedOutside` with the
//! recipe book's own rule) and which `ContainerInput` it sends. Shift is
//! the event's own modifiers (`InputWithModifiers.hasShiftDown`: the
//! physical Shift keys, never the remapped or toggled sneak key).

use crate::menu::Input;

/// The screen's window: `leftPos`, `topPos`, `imageWidth`, `imageHeight`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Image {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

/// The recipe book of an `AbstractRecipeBookScreen`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Book {
    /// `RecipeBookComponent.isVisible`.
    pub visible: bool,
    /// `selectedTab.isHoveredOrFocused()`.
    pub selected_tab_hovered: bool,
}

/// `RecipeBookComponent.IMAGE_WIDTH`.
const BOOK_WIDTH: i32 = 147;

fn outside(mx: f64, my: f64, image: Image) -> bool {
    mx < f64::from(image.left)
        || my < f64::from(image.top)
        || mx >= f64::from(image.left + image.width)
        || my >= f64::from(image.top + image.height)
}

/// `AbstractContainerScreen.hasClickedOutside`, or
/// `AbstractRecipeBookScreen.hasClickedOutside` when the screen has a book
/// (`RecipeBookComponent.hasClickedOutside`: a click on the book, or with
/// its selected tab hovered, is not outside).
pub fn clicked_outside(mx: f64, my: f64, image: Image, book: Option<Book>) -> bool {
    let out = outside(mx, my, image);
    let Some(book) = book else {
        return out;
    };
    let book_outside = !book.visible || {
        let on_book = f64::from(image.left - BOOK_WIDTH) < mx
            && mx < f64::from(image.left)
            && f64::from(image.top) < my
            && my < f64::from(image.top + image.height);
        out && !on_book && !book.selected_tab_hovered
    };
    book_outside && out
}

/// What a click points at: a slot, outside the window (slot id -999), or
/// neither (slot id -1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target<S> {
    Slot(S),
    Outside,
    Nothing,
}

impl<S> Target<S> {
    /// Outside wins over a hovered slot, as in `mouseClicked`.
    pub fn of(slot: Option<S>, outside: bool) -> Self {
        match slot {
            _ if outside => Self::Outside,
            Some(s) => Self::Slot(s),
            None => Self::Nothing,
        }
    }
}

/// The left, right and middle buttons (SDL's 1, 3 and 2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// `getContainerClickButton`: the button a menu click carries.
    pub fn menu_button(self) -> i32 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
        }
    }
}

/// The input state `mouseClicked`/`mouseReleased` keep between events.
#[derive(Clone, Debug, Default)]
pub struct Clicks {
    /// `isQuickCrafting` with `quickCraftingButton`.
    quick_crafting: Option<Button>,
    skip_next_release: bool,
}

/// What a release does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Release {
    /// `slotClicked` with this input on the release's target.
    Click(Input),
    /// `quickCraftToSlots` over the dragged slots.
    QuickCraft,
    Nothing,
}

impl Clicks {
    /// A quick craft drag in progress, and the button that started it.
    pub fn quick_crafting(&self) -> Option<Button> {
        self.quick_crafting
    }

    /// A press another screen control handled (such as a book button or
    /// double click's `PICKUP_ALL`): its release clicks nothing more.
    pub fn skip_release(&mut self) {
        self.quick_crafting = None;
        self.skip_next_release = true;
    }

    /// The screen that takes the pointer over starts with its own state:
    /// `AbstractContainerScreen` holds `isQuickCrafting` and the drag it
    /// belongs to on the screen, so a screen that opens (or closes) drops
    /// them. A press whose release never arrived -- the pointer left the
    /// window -- would otherwise keep the drag running, and `press` answers
    /// `None` for every later press, so the quick move and the clicks go
    /// dead on every screen after it.
    pub fn cancel_drag(&mut self) {
        self.quick_crafting = None;
        self.skip_next_release = false;
    }

    /// `mouseClicked` (no double click): the `slotClicked` input it sends on
    /// `target`, if any. A press holding a stack starts a quick craft drag
    /// instead. `cloning` is `keyPickItem` with infinite materials.
    pub fn press<S>(
        &mut self,
        target: &Target<S>,
        button: Button,
        carried_empty: bool,
        shift: bool,
        cloning: bool,
    ) -> Option<Input> {
        self.skip_next_release = false;
        if button == Button::Middle && !cloning {
            return None;
        }
        if matches!(target, Target::Nothing) || self.quick_crafting.is_some() {
            return None;
        }
        if !carried_empty {
            self.quick_crafting = Some(button);
            return None;
        }
        self.skip_next_release = true;
        Some(if cloning {
            Input::Clone
        } else if shift && !matches!(target, Target::Outside) {
            Input::QuickMove
        } else if matches!(target, Target::Outside) {
            Input::Throw
        } else {
            Input::Pickup
        })
    }

    /// `mouseReleased` (no double click). `dragged` is whether the quick
    /// craft drag collected any slots; `pick_item` is the release of the
    /// `keyPickItem` button.
    pub fn release<S>(
        &mut self,
        target: &Target<S>,
        button: Button,
        carried_empty: bool,
        dragged: bool,
        shift: bool,
        pick_item: bool,
    ) -> Release {
        if self.quick_crafting.is_some_and(|b| b != button) {
            self.quick_crafting = None;
            self.skip_next_release = true;
            return Release::Nothing;
        }
        if self.skip_next_release {
            self.skip_next_release = false;
            return Release::Nothing;
        }
        let quick_crafting = self.quick_crafting.take().is_some();
        if quick_crafting && dragged {
            Release::QuickCraft
        } else if carried_empty {
            Release::Nothing
        } else if pick_item {
            Release::Click(Input::Clone)
        } else if shift && !matches!(target, Target::Outside) {
            Release::Click(Input::QuickMove)
        } else {
            Release::Click(Input::Pickup)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INV: Image = Image {
        left: 100,
        top: 50,
        width: 176,
        height: 166,
    };

    #[test]
    fn window_edges() {
        assert!(!clicked_outside(100.0, 50.0, INV, None));
        assert!(clicked_outside(99.9, 60.0, INV, None));
        assert!(clicked_outside(276.0, 60.0, INV, None));
        assert!(!clicked_outside(275.9, 215.9, INV, None));
    }

    #[test]
    fn the_open_book_is_not_outside() {
        let book = Book {
            visible: true,
            selected_tab_hovered: false,
        };
        assert!(!clicked_outside(10.0, 100.0, INV, Some(book)));
        assert!(clicked_outside(-60.0, 100.0, INV, Some(book)));
        assert!(!clicked_outside(
            -60.0,
            100.0,
            INV,
            Some(Book {
                selected_tab_hovered: true,
                ..book
            })
        ));
        let closed = Book {
            visible: false,
            ..book
        };
        assert!(clicked_outside(10.0, 100.0, INV, Some(closed)));
    }

    #[test]
    fn a_carried_stack_thrown_outside() {
        let mut clicks = Clicks::default();
        let out = Target::<usize>::Outside;
        assert_eq!(clicks.press(&out, Button::Left, false, false, false), None);
        assert_eq!(
            clicks.release(&out, Button::Left, false, false, false, false),
            Release::Click(Input::Pickup)
        );
        // Shift never quick-moves outside, and an empty hand throws nothing.
        assert_eq!(
            clicks.press(&out, Button::Right, true, true, false),
            Some(Input::Throw)
        );
        assert_eq!(
            clicks.release(&out, Button::Right, true, false, true, false),
            Release::Nothing
        );
    }

    #[test]
    fn a_recipe_book_click_does_not_throw_a_carried_stack_after_the_layout_shifts() {
        let mut clicks = Clicks::default();
        let button = Target::<usize>::Nothing;
        // The book button consumes the press while the player carries a stack.
        assert_eq!(
            clicks.press(&button, Button::Left, false, false, false),
            None
        );
        clicks.skip_release();

        // Closing the book shifts the inventory before mouseReleased tests the
        // pointer again; it may now look like an outside click.
        assert_eq!(
            clicks.release(
                &Target::<usize>::Outside,
                Button::Left,
                false,
                false,
                false,
                false,
            ),
            Release::Nothing
        );
    }

    #[test]
    fn a_creative_page_release_does_not_throw_a_carried_stack() {
        let mut clicks = Clicks::default();
        let out = Target::<usize>::Outside;
        clicks.skip_release();
        assert_eq!(
            clicks.release(&out, Button::Left, false, false, false, false),
            Release::Nothing
        );
    }

    #[test]
    fn another_button_cancels_the_drag() {
        let mut clicks = Clicks::default();
        let slot = Target::Slot(3);
        clicks.press(&slot, Button::Left, false, false, false);
        assert_eq!(
            clicks.release(&slot, Button::Right, false, true, false, false),
            Release::Nothing
        );
        assert_eq!(
            clicks.release(&slot, Button::Left, false, true, false, false),
            Release::Nothing
        );
        assert_eq!(
            clicks.release(&slot, Button::Left, false, true, true, false),
            Release::Click(Input::QuickMove)
        );
    }

    /// A press whose release never arrives (the pointer left the window, or
    /// the screen closed first) leaves `isQuickCrafting` set; the next screen
    /// starts with its own state, as `AbstractContainerScreen` does, so the
    /// reset the game runs on open/close makes clicks work again.
    #[test]
    fn a_lost_release_does_not_wedge_the_next_screen() {
        let mut clicks = Clicks::default();
        let slot = Target::<usize>::Slot(3);
        // Pressed with a stack carried: a quick craft drag starts, and its
        // release is lost.
        assert_eq!(clicks.press(&slot, Button::Left, false, false, false), None);
        assert!(clicks.quick_crafting().is_some());
        assert_eq!(
            clicks.press(&slot, Button::Left, true, false, false),
            None,
            "the stuck drag swallows the next press"
        );
        // A new screen: fresh state, so the press is a normal click again.
        clicks = Clicks::default();
        assert!(clicks.quick_crafting().is_none());
        assert_eq!(
            clicks.press(&slot, Button::Left, true, false, false),
            Some(Input::Pickup)
        );
    }

    /// #1737: a press whose release never arrives (the pointer left the
    /// window) leaves `isQuickCrafting` set, and `press` answers `None` for
    /// every later press, so the Shift quick move and the clicks on the
    /// inventory go dead. The drag belongs to the screen that has it
    /// (`AbstractContainerScreen` holds `isQuickCrafting`), so the screen
    /// that takes the pointer over drops it and the quick move works again.
    #[test]
    fn a_lost_release_does_not_wedge_the_quick_move() {
        let mut clicks = Clicks::default();
        let slot = Target::<usize>::Slot(3);
        // Pressed with a stack carried: a quick craft drag starts, and its
        // release never arrives.
        assert_eq!(clicks.press(&slot, Button::Left, false, false, false), None);
        assert!(clicks.quick_crafting().is_some());
        // The stuck drag swallows the next press: the Shift quick move stops
        // working and the inventory stops answering.
        assert_eq!(clicks.press(&slot, Button::Right, true, true, false), None);
        // The screen that takes the pointer over starts with its own state.
        clicks.cancel_drag();
        assert!(clicks.quick_crafting().is_none());
        // The quick move works again.
        assert_eq!(
            clicks.press(&slot, Button::Right, true, true, false),
            Some(Input::QuickMove)
        );
        assert_eq!(
            clicks.press(&slot, Button::Left, true, false, false),
            Some(Input::Pickup)
        );
    }
}
