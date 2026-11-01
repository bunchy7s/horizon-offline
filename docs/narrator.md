# Narrator

The narrator reads the game aloud as vanilla 26.3's does: the chat, system
messages, the toggle toast, and the open screen with the widget under the
mouse or with the focus. It is vanilla's feature (`GameNarrator` over Mojang's
text2speech library, and `Screen`'s narration), ported 1:1: the same option,
the same hotkey, the same voices and the same sentences, in the game's
language.

## Using it

- **Options → Accessibility Settings... → Narrator** (also on the Chat
  Settings page and the title screen's accessibility button). `options.txt`
  key `narrator`, by id:

  | Id | Status | Reads |
  |---|---|---|
  | 0 | OFF | nothing |
  | 1 | Narrates All | chat and system messages, and the screens |
  | 2 | Narrates Chat | chat messages |
  | 3 | Narrates System | system messages and the screens |

- **Ctrl+B** (Cmd+B on macOS) cycles the status anywhere in the game, with
  the "Narrator Enabled/Disabled" toast, while **Narrator Hotkey** is on
  (`narratorHotkey`, on by default), as `KeyboardHandler` does.
- The volume is the **Voice/Speech** slider times **Master Volume**.
- Where text-to-speech can't start, the option reads "Not Available" and the
  toast says so; the game goes on without it (vanilla asks in a message box
  first; Horizon logs vanilla's message instead).

## Voices per platform

The same backends text2speech uses, opened on a speech thread of their own so
speaking never blocks a frame:

| Platform | Speech | Voice |
|---|---|---|
| Windows | SAPI `SpVoice` (COM) | the voice and rate chosen in Windows' speech settings |
| macOS | AppKit `NSSpeechSynthesizer` | the system's spoken content voice and rate |
| Linux | flite, `cmu_us_kal16` | `libflite` and `libflite_cmu_us_kal16` (`.so` or `.so.1`), loaded at run time |

On Linux the narrator needs flite installed, as vanilla's does (Debian and
Ubuntu: `sudo apt install flite`; Fedora: `flite`; Arch: `flite`). Without it
the game runs and the narrator is "Not Available".

## Where the code lives

| Part | Code |
|---|---|
| `GameNarrator`: status, queued and interrupting messages, volume, the toast's text, the last 32 messages | `crates/horizon-narrator/src/lib.rs` |
| The speech thread and the backends | `crates/horizon-narrator/src/speech.rs`, `speech/{windows,mac,linux}.rs` |
| `ScreenNarrationCollector` (what changed since the last run), `Screen`'s timing (`afterMouseMove`, `afterKeyboardAction`, the suppression after `init`), `findNarratableWidget` | `crates/horizon-narrator/src/screen.rs` |
| What a frame's screen and widgets narrate (`ScreenInfo`, `Narratable`, `What`), recorded while drawing | `crates/horizon-client-render/src/widgets.rs` |
| The driver: reads the last drawn frame, runs `updateNarrationState` and says what is due | `crates/horizon-client/src/screen_narrator.rs` |
| Game glue: start and stop, the options, Ctrl+B, the toast, the input events the driver hears | `crates/horizon-client/src/game/narrator.rs` |
| Screens whose narration is their own (chat, level loading, books, confirm screens) | `crates/horizon-client/src/game/screen_info.rs` |
| Chat and system messages | `game.rs`, `game/remote.rs` and the other callers of `horizon_narrator::say_*`, where vanilla narrates |

## How screens record their narration

Vanilla screens are trees of widget objects that narrate themselves.
Horizon's screens are drawn each frame, so they record what they would
narrate as they draw, and `ScreenNarrator::frame` reads the finished frame:

- `widgets::begin_screen(|| ScreenInfo::new(id, title))` starts the screen.
  The `id` says which screen it is: a new id is a screen opening (`init`'s
  narration), the same id a screen still open, a new GUI size the screen
  resized. The last screen begun in a frame wins (an overlay over the screen
  under it). `ScreenInfo` also carries `getUsageNarration`, whether the
  widgets and the navigation hint are narrated, extra elements (the chat's
  message, the loading progress) and a repeat period.
- `widgets::narratable(|| Narratable::...)` records a widget in the order
  vanilla adds it: `button`, `slider`, `cycle`, `checkbox`, `edit_box`,
  `named` (a button narrated by its own message), `text`, `recipe`, `list`
  (`ObjectSelectionList` and `TabNavigationBar`), `container_list` (its row
  widgets recorded with `child_of(list, row)`), `group` (`RecipeBookComponent`:
  its widgets recorded with `in_group(group)`). `.key()` gives a widget a
  stable identity, `.hint()` its tooltip, `.without_tooltip()` none,
  `.focused()` and `.hovered()` what the drawer knows.
- The shared drawers record for you: `ui::button`, `ui::slider`,
  `ui::EditBox::draw`, the `screen_draw` buttons and fields. A button that
  vanilla narrates as another widget goes through `ui::button_as(.., |rect|
  Narratable::cycle(..))`. `widgets::last_narratable` and
  `narratables_mark`/`move_narratables` fix a recorded widget or its place
  when the draw order isn't vanilla's add order.
- Text is built with `screen_narrator::translate(key, args)` (vanilla's keys:
  `gui.narrate.button`, `narrator.position.screen`, ...) and joined with
  `widgets::join_for_narration` (`CommonComponents.joinForNarration`).

Everything is recorded through closures, and only while the narrator can
speak: with text-to-speech unavailable nothing is built or allocated.

Adding a screen, for example a confirm screen with two buttons:

```rust
horizon_client::widgets::begin_screen(|| {
    // ConfirmScreen.getNarrationMessage: its title and its message.
    ScreenInfo::new("my_confirm", join_for_narration(&[&title, &message]))
});
// ... draw the title and message ...
ui::button(batch, atlas, x, y, 150.0, 20.0, &yes, true, hover_yes); // recorded
ui::button(batch, atlas, x + 160.0, y, 150.0, 20.0, &no, true, hover_no);
```

Hovering "No" then reads "Title. Message" when the screen opens and, after
the mouse moves, "No button. Screen element 2 out of 2. Left click to
activate". Driver tests (`cargo test -p horizon-client --lib screen_narrator`)
show the expected sentences as translation keys.

## Scripted runs

- `HORIZON_NO_AUDIO=1` keeps the narrator active but silent: everything it
  would say still goes through its status and into its recent messages.
- `horizon-ctl narration` prints the status, whether text-to-speech is
  there, and the last 32 messages its status let through, oldest first
  ([horizon-ctl commands](horizon-ctl-commands.md)).

## Known differences from vanilla

The narration is built from what the screens draw, which comes close to
vanilla's widget trees but not always all the way:

- Focus comes from a click (as `mouseClicked` focuses), a text field's caret,
  keyboard navigation (Tab, the arrows and Enter, [controllers.md](controllers.md)),
  the controller's focus ring and the screens that track focus (lists, tabs).
  Tab follows reading order rather than vanilla's add order.
- "Screen element i out of n" counts widgets in the order screens record
  them; most follow vanilla's add order, a few approximate it.
- Some screens narrate less than vanilla's: a dialog's body text, the
  command box's suggestion list (`narration.suggestion`), the stats screen
  while it loads (and the focus it gives its tabs once loaded), and the
  server list's "incompatible version" status.
- A click focuses the widget under the mouse even where a scrolled list
  hides it behind the header or footer.
- The macOS backend has no `crashed` state: a failing synthesizer message
  would end vanilla's narration for good, and raises nothing in Horizon.
- The first narration of a screen runs after its first frame (vanilla's runs
  in `init`, before it); what `init` couldn't see then (the hovered widget)
  is left out, as in vanilla.
- The first launch's accessibility onboarding narrates its title, but its
  Narrator button stays inactive ("Not Available") and it doesn't say
  "Press Enter to enable the narrator" (`AccessibilityOnboardingScreen`'s
  `handleInitialNarrationDelay`): use Ctrl+B or the Accessibility Settings.
