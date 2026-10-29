//! The system clipboard and the text-editing shortcuts of 26.3:
//! `KeyboardHandler.getClipboard`/`setClipboard`, `InputWithModifiers.isSelectAll`,
//! `isCopy`, `isPaste` and `isCut`, `StringUtil.filterText` and
//! `ChatFormatting.stripFormatting`.

use winit::keyboard::{Key, KeyCode, PhysicalKey};

/// A text-editing shortcut (`InputWithModifiers.isSelectAll`, `isCopy`,
/// `isPaste`, `isCut`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditShortcut {
    SelectAll,
    Copy,
    Paste,
    Cut,
}

/// The letter a key stands for in shortcuts (`KeyEvent.shortcutKey`, SDL's
/// keycode): the layout's letter, or the key's US-layout letter when the
/// layout's is not a Latin one (SDL's default `latin_letters` keycode option).
pub fn shortcut_letter(logical: &Key, physical: PhysicalKey) -> Option<char> {
    if let Key::Character(s) = logical {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii_alphabetic()
        {
            return Some(c.to_ascii_lowercase());
        }
    }
    let PhysicalKey::Code(code) = physical else {
        return None;
    };
    Some(match code {
        KeyCode::KeyA => 'a',
        KeyCode::KeyC => 'c',
        KeyCode::KeyV => 'v',
        KeyCode::KeyX => 'x',
        _ => return None,
    })
}

/// `InputWithModifiers.isSelectAll`/`isCopy`/`isPaste`/`isCut`: A, C, V or X
/// with the edit modifier (`InputQuirks.EDIT_SHORTCUT_KEY_MODIFIER`: Cmd on
/// macOS, Ctrl elsewhere) and neither Shift nor Alt.
pub fn edit_shortcut(
    letter: Option<char>,
    edit_modifier: bool,
    shift: bool,
    alt: bool,
) -> Option<EditShortcut> {
    if !edit_modifier || shift || alt {
        return None;
    }
    Some(match letter? {
        'a' => EditShortcut::SelectAll,
        'c' => EditShortcut::Copy,
        'v' => EditShortcut::Paste,
        'x' => EditShortcut::Cut,
        _ => return None,
    })
}

/// Windows reports AltGr as Ctrl plus Alt: the system presses a left Control
/// in front of the right Alt.
pub const ALTGR_IS_CTRL_ALT: bool = cfg!(windows);

/// Whether a key's text reaches the screen (`KeyboardHandler.textInput`, then
/// `charTyped` per code point). The edit modifier's shortcuts type nothing,
/// but `textInput` takes no modifiers: with AltGr held (Ctrl plus Alt where
/// `altgr_is_ctrl_alt`) the layout's third-level characters are text, as `/`
/// on ABNT2, `@`, `{` and `[` on German or the Polish letters.
pub fn types_text(edit_modifier: bool, alt: bool, altgr_is_ctrl_alt: bool) -> bool {
    !edit_modifier || (altgr_is_ctrl_alt && alt)
}

/// `StringUtil.isAllowedChatCharacter`.
pub fn is_allowed_chat_character(c: char) -> bool {
    c != '§' && c >= ' ' && c != '\u{7f}'
}

/// `StringUtil.filterText`: the allowed characters, and line breaks where
/// the field takes several lines.
pub fn filter_text(input: &str, multiline: bool) -> String {
    input
        .chars()
        .filter(|&c| is_allowed_chat_character(c) || (multiline && c == '\n'))
        .collect()
}

/// `ChatFormatting.stripFormatting`: `§` followed by a format code removed.
pub fn strip_formatting(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§'
            && chars.peek().is_some_and(
                |n| matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r'),
            )
        {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// `TextFieldHelper.getClipboardContents`: formatting and carriage returns
/// removed (signs).
pub fn helper_clipboard_contents(clipboard: &str) -> String {
    strip_formatting(&clipboard.replace('\r', ""))
}

/// A clipboard: the system's, or an in-memory one in tests.
pub trait Clipboard {
    /// `KeyboardHandler.getClipboard`: empty when there is no text.
    fn get(&mut self) -> String;
    /// `KeyboardHandler.setClipboard`: empty text is not copied.
    fn set(&mut self, text: &str);
}

/// The system clipboard (`ClipboardManager`). On Wayland, arboard uses the
/// data-control protocol and falls back to X11 (XWayland) on compositors
/// without it. When no system clipboard can be reached, text copied in the
/// game can still be pasted in the game.
#[derive(Default)]
pub struct SystemClipboard {
    system: Option<arboard::Clipboard>,
    /// The text Horizon last copied (`horizon-ctl status`).
    copied: Option<String>,
}

impl SystemClipboard {
    /// The text last copied by the game.
    pub fn copied(&self) -> Option<&str> {
        self.copied.as_deref()
    }
    fn system(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        self.system.as_mut()
    }
}

impl Clipboard for SystemClipboard {
    fn get(&mut self) -> String {
        let Some(system) = self.system() else {
            return self.copied.clone().unwrap_or_default();
        };
        match system.get_text() {
            Ok(text) => text,
            Err(arboard::Error::ContentNotAvailable) => String::new(),
            Err(_) => {
                // A lost connection (a restarted compositor): reconnect next time.
                self.system = None;
                String::new()
            }
        }
    }
    fn set(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.copied = Some(text.to_owned());
        if let Some(system) = self.system()
            && system.set_text(text.to_owned()).is_err()
        {
            self.system = None;
        }
    }
}

/// A clipboard in memory.
#[derive(Default, Debug)]
pub struct MemoryClipboard(pub String);

impl Clipboard for MemoryClipboard {
    fn get(&mut self) -> String {
        self.0.clone()
    }
    fn set(&mut self, text: &str) {
        if !text.is_empty() {
            self.0 = text.to_owned();
        }
    }
}

/// A text field with a cursor and a selection (`EditBox`).
pub trait Editable {
    /// Select All: the cursor at the end, the selection from the start.
    fn select_all(&mut self);
    /// `getHighlighted`.
    fn selected_text(&self) -> String;
    /// `insertText`, with the field's own filter and length limit. True when
    /// the value changed.
    fn insert(&mut self, text: &str) -> bool;
}

/// `EditBox.keyPressed`'s shortcuts. True when the value changed.
pub fn apply(
    field: &mut dyn Editable,
    shortcut: EditShortcut,
    clipboard: &mut dyn Clipboard,
) -> bool {
    match shortcut {
        EditShortcut::SelectAll => {
            field.select_all();
            false
        }
        EditShortcut::Copy => {
            clipboard.set(&field.selected_text());
            false
        }
        EditShortcut::Paste => field.insert(&clipboard.get()),
        EditShortcut::Cut => {
            clipboard.set(&field.selected_text());
            field.insert("")
        }
    }
}

/// Which append-only field a Select All covers: a name and an index (a
/// sign line, a book page).
pub type FieldId = (&'static str, usize);

/// The selection of Horizon's append-only fields, whose cursor stays at the
/// end: Select All selects the whole value, which the next copy, cut, paste,
/// typed text or backspace replaces. It is forgotten as soon as the field or
/// its value changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppendSelection(Option<(FieldId, String)>);

impl AppendSelection {
    /// Whether `value` of field `id` is wholly selected.
    pub fn covers(&self, id: FieldId, value: &str) -> bool {
        self.0.as_ref().is_some_and(|(i, v)| *i == id && v == value)
    }
    /// Select the whole `value` of field `id`, as Select All does.
    pub fn select(&mut self, id: FieldId, value: &str) {
        self.0 = Some((id, value.to_owned()));
    }
    /// Before typing or a backspace: an all-selected value is cleared (true).
    pub fn take(&mut self, id: FieldId, value: &mut String) -> bool {
        let covered = self.covers(id, value);
        self.0 = None;
        if covered {
            value.clear();
        }
        covered
    }
    /// A shortcut in an append-only field. `insert` adds pasted text to a
    /// value the field's own way and says whether it accepted it (a field
    /// that rejects it keeps its value, as `TextFieldHelper.insertText`).
    /// True when the value changed.
    pub fn apply(
        &mut self,
        id: FieldId,
        value: &mut String,
        shortcut: EditShortcut,
        clipboard: &mut dyn Clipboard,
        insert: impl FnOnce(&mut String, &str) -> bool,
    ) -> bool {
        let covered = self.covers(id, value);
        match shortcut {
            EditShortcut::SelectAll => {
                self.select(id, value);
                false
            }
            EditShortcut::Copy => {
                if covered {
                    clipboard.set(value);
                }
                false
            }
            EditShortcut::Cut => {
                self.0 = None;
                if !covered {
                    return false;
                }
                clipboard.set(value);
                let changed = !value.is_empty();
                value.clear();
                changed
            }
            EditShortcut::Paste => {
                self.0 = None;
                let text = clipboard.get();
                let mut next = if covered {
                    String::new()
                } else {
                    value.clone()
                };
                if !insert(&mut next, &text) || next == *value {
                    return false;
                }
                *value = next;
                true
            }
        }
    }
}

/// `EditBox.insertText` for a field without a cursor: the filtered text,
/// cut to the room left by `max_chars`.
pub fn append_limited(value: &mut String, text: &str, max_chars: usize) -> bool {
    let room = max_chars.saturating_sub(value.chars().count());
    if room == 0 {
        return false;
    }
    value.extend(filter_text(text, false).chars().take(room));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altgr_types_text_where_it_is_ctrl_alt() {
        // Plain keys and Shift type.
        assert!(types_text(false, false, true));
        assert!(types_text(false, false, false));
        // Ctrl (Cmd) shortcuts do not.
        assert!(!types_text(true, false, true));
        assert!(!types_text(true, false, false));
        // AltGr on Windows is Ctrl+Alt: ABNT2 `/`, German `@` type.
        assert!(types_text(true, true, true));
        // Where AltGr is its own modifier, Ctrl+Alt is a shortcut.
        assert!(!types_text(true, true, false));
        // Alt alone (macOS Option, Linux AltGr) types.
        assert!(types_text(false, true, false));
    }

    #[test]
    fn shortcuts_need_the_edit_modifier_alone() {
        let c = Some('c');
        assert_eq!(
            edit_shortcut(c, true, false, false),
            Some(EditShortcut::Copy)
        );
        assert_eq!(
            edit_shortcut(Some('v'), true, false, false),
            Some(EditShortcut::Paste)
        );
        assert_eq!(
            edit_shortcut(Some('x'), true, false, false),
            Some(EditShortcut::Cut)
        );
        assert_eq!(
            edit_shortcut(Some('a'), true, false, false),
            Some(EditShortcut::SelectAll)
        );
        assert_eq!(edit_shortcut(c, false, false, false), None);
        assert_eq!(edit_shortcut(c, true, true, false), None);
        assert_eq!(edit_shortcut(c, true, false, true), None);
        assert_eq!(edit_shortcut(Some('b'), true, false, false), None);
    }

    #[test]
    fn the_layout_letter_decides_and_non_latin_layouts_use_the_position() {
        let q = PhysicalKey::Code(KeyCode::KeyQ);
        // AZERTY: the key at Q's position types an A.
        assert_eq!(shortcut_letter(&Key::Character("a".into()), q), Some('a'));
        // Russian: the key at C's position types с.
        let c = PhysicalKey::Code(KeyCode::KeyC);
        assert_eq!(shortcut_letter(&Key::Character("с".into()), c), Some('c'));
        assert_eq!(shortcut_letter(&Key::Character("C".into()), q), Some('c'));
    }

    #[test]
    fn filter_text_keeps_line_breaks_only_for_multiline_fields() {
        let s = "a\u{7}b§c\nd\re\u{80}";
        assert_eq!(filter_text(s, false), "abcde\u{80}");
        assert_eq!(filter_text(s, true), "abc\nde\u{80}");
        assert_eq!(strip_formatting("§aRed§r §zx§"), "Red §zx§");
        assert_eq!(helper_clipboard_contents("§lone\r\ntwo"), "one\ntwo");
        // TextFieldHelper.getClipboardContents runs before insertText's filter.
        assert_eq!(
            filter_text(&helper_clipboard_contents("§aHello"), false),
            "Hello"
        );
    }

    struct Field {
        value: String,
        cursor: usize,
        highlight: usize,
    }
    impl Editable for Field {
        fn select_all(&mut self) {
            self.cursor = self.value.len();
            self.highlight = 0;
        }
        fn selected_text(&self) -> String {
            let (a, b) = (
                self.cursor.min(self.highlight),
                self.cursor.max(self.highlight),
            );
            self.value[a..b].to_owned()
        }
        fn insert(&mut self, text: &str) -> bool {
            let (a, b) = (
                self.cursor.min(self.highlight),
                self.cursor.max(self.highlight),
            );
            self.value.replace_range(a..b, text);
            self.cursor = a + text.len();
            self.highlight = self.cursor;
            true
        }
    }

    #[test]
    fn edit_box_shortcuts() {
        let mut clip = MemoryClipboard("old".into());
        let mut f = Field {
            value: "hello".into(),
            cursor: 5,
            highlight: 5,
        };
        // Nothing selected: copying leaves the clipboard alone (setClipboard skips "").
        apply(&mut f, EditShortcut::Copy, &mut clip);
        assert_eq!(clip.0, "old");
        apply(&mut f, EditShortcut::SelectAll, &mut clip);
        apply(&mut f, EditShortcut::Copy, &mut clip);
        assert_eq!(clip.0, "hello");
        assert!(apply(&mut f, EditShortcut::Cut, &mut clip));
        assert_eq!(f.value, "");
        assert!(apply(&mut f, EditShortcut::Paste, &mut clip));
        assert!(apply(&mut f, EditShortcut::Paste, &mut clip));
        assert_eq!(f.value, "hellohello");
    }

    #[test]
    fn append_only_fields_select_everything_or_nothing() {
        let id = ("search", 0);
        let mut clip = MemoryClipboard("stone".into());
        let mut sel = AppendSelection::default();
        let mut value = String::from("dirt");
        let insert = |v: &mut String, t: &str| append_limited(v, t, 6);
        // Without Select All, copy and cut do nothing and paste appends.
        sel.apply(id, &mut value, EditShortcut::Copy, &mut clip, insert);
        assert_eq!(clip.0, "stone");
        assert!(!sel.apply(id, &mut value, EditShortcut::Cut, &mut clip, insert));
        assert!(sel.apply(id, &mut value, EditShortcut::Paste, &mut clip, insert));
        assert_eq!(value, "dirtst");
        // Select All then paste replaces the value.
        sel.apply(id, &mut value, EditShortcut::SelectAll, &mut clip, insert);
        assert!(sel.apply(id, &mut value, EditShortcut::Paste, &mut clip, insert));
        assert_eq!(value, "stone");
        // Select All then cut empties it and copies it.
        sel.apply(id, &mut value, EditShortcut::SelectAll, &mut clip, insert);
        clip.0 = "x".into();
        assert!(sel.apply(id, &mut value, EditShortcut::Cut, &mut clip, insert));
        assert_eq!((value.as_str(), clip.0.as_str()), ("", "stone"));
        // A selection is forgotten once the value changes or another field is used.
        value = "ab".into();
        sel.apply(id, &mut value, EditShortcut::SelectAll, &mut clip, insert);
        assert!(!sel.covers(("other", 0), &value));
        value.push('c');
        assert!(!sel.take(id, &mut value));
        assert_eq!(value, "abc");
        sel.apply(id, &mut value, EditShortcut::SelectAll, &mut clip, insert);
        assert!(sel.take(id, &mut value));
        assert_eq!(value, "");
    }

    #[test]
    fn a_rejected_paste_keeps_the_selected_value() {
        // TextFieldHelper.insertText: a line too wide is not set at all.
        let id = ("sign", 1);
        let mut clip = MemoryClipboard("far too long".into());
        let mut sel = AppendSelection::default();
        let mut value = String::from("ok");
        sel.apply(
            id,
            &mut value,
            EditShortcut::SelectAll,
            &mut clip,
            |_, _| true,
        );
        let fits = |v: &mut String, t: &str| {
            let next = format!("{v}{t}");
            next.len() <= 5 && {
                *v = next;
                true
            }
        };
        assert!(!sel.apply(id, &mut value, EditShortcut::Paste, &mut clip, fits));
        assert_eq!(value, "ok");
    }
}
