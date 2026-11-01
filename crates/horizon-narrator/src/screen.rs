//! A screen's narration: `ScreenNarrationCollector` (what to say, and what
//! was said already) and `Screen`'s timing (`scheduleNarration`,
//! `handleDelayedNarration`, the suppression after `init`).
//!
//! The game builds a screen's narration each time it runs ([`Output`]): the
//! screen's title, then the narrated widget's own elements one level deeper
//! ([`Output::nest`]). Only what changed since the last run is said, unless a
//! run is forced; elements are read title first, then positions, hints and
//! usage, each outer before inner, joined with ". ".
use std::collections::BTreeMap;

/// `NarratedElementType`, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementType {
    Title,
    Position,
    Hint,
    Usage,
}

/// `NarrationTrigger`: what scheduled the narration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    Mouse,
    Keyboard,
    #[default]
    System,
}

/// `NarratableEntry.NarrationPriority`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    #[default]
    None,
    Hovered,
    Focused,
}

impl Priority {
    pub fn is_terminal(self) -> bool {
        self == Priority::Focused
    }
}

/// `ScreenNarrationCollector.NarrationEntry`. `contents` is the thunk's text:
/// `None` is `NarrationThunk.EMPTY`, which equals no text, not even an empty
/// one.
#[derive(Debug, Default)]
struct Entry {
    contents: Option<Vec<String>>,
    generation: i64,
    already_narrated: bool,
}

/// `ScreenNarrationCollector`.
#[derive(Debug, Default)]
pub struct Collector {
    generation: i64,
    entries: BTreeMap<(ElementType, u32), Entry>,
}

impl Collector {
    /// `update`: a new generation, filled by `updater`.
    pub fn update(&mut self, trigger: Trigger, updater: impl FnOnce(&mut Output<'_>)) {
        self.generation += 1;
        updater(&mut Output {
            collector: self,
            depth: 0,
            trigger,
        });
    }

    /// `collectNarrationText`: this generation's elements not narrated yet
    /// (all of them when `force`), marked narrated.
    pub fn collect(&mut self, force: bool) -> String {
        let mut result = String::new();
        let mut first = true;
        for entry in self.entries.values_mut() {
            if entry.generation == self.generation && (force || !entry.already_narrated) {
                for line in entry.contents.iter().flatten() {
                    if !first {
                        result.push_str(". ");
                    }
                    first = false;
                    result.push_str(line);
                }
                entry.already_narrated = true;
            }
        }
        result
    }

    fn add(&mut self, key: (ElementType, u32), contents: Vec<String>) {
        let generation = self.generation;
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            generation: -1,
            ..Entry::default()
        });
        // NarrationEntry.update: new text is narrated again, and so is text
        // that was missing from the generation before.
        if entry.contents.as_ref() != Some(&contents) {
            entry.contents = Some(contents);
            entry.already_narrated = false;
        } else if entry.generation + 1 != generation {
            entry.already_narrated = false;
        }
        entry.generation = generation;
    }
}

/// `NarrationElementOutput` at a depth.
pub struct Output<'a> {
    collector: &'a mut Collector,
    depth: u32,
    trigger: Trigger,
}

impl Output<'_> {
    /// `add(type, NarrationThunk.from(text))`.
    pub fn add(&mut self, kind: ElementType, text: impl Into<String>) {
        self.collector.add((kind, self.depth), vec![text.into()]);
    }
    /// `add(type, NarrationThunk.from(lines))`: one element, each line read
    /// as its own sentence.
    pub fn add_lines(&mut self, kind: ElementType, lines: Vec<String>) {
        self.collector.add((kind, self.depth), lines);
    }
    /// `nest`: the narrated element's own elements, read after the screen's
    /// of the same type.
    pub fn nest(&mut self) -> Output<'_> {
        Output {
            collector: self.collector,
            depth: self.depth + 1,
            trigger: self.trigger,
        }
    }
    pub fn trigger(&self) -> Trigger {
        self.trigger
    }
}

/// `Screen.NARRATE_SUPPRESS_AFTER_INIT_TIME`, and the delay after the narrator
/// is turned on.
pub const SUPPRESS_AFTER_INIT_MS: i64 = 2000;
pub const DELAY_MOUSE_MOVE_MS: i64 = 750;
pub const DELAY_MOUSE_ACTION_MS: i64 = 200;
pub const DELAY_KEYBOARD_ACTION_MS: i64 = 200;

/// A screen's narration state (`Screen`'s `narrationState`,
/// `narrationSuppressTime`, `nextNarrationTime`, `nextNarrationTriggeredBy`
/// and `lastNarratable`). A new screen starts a new one. Times are
/// milliseconds on any steady clock (`Util.getMillis`).
#[derive(Debug)]
pub struct ScreenNarration {
    pub collector: Collector,
    suppress_time: i64,
    next_time: i64,
    next_trigger: Trigger,
    /// The widget narrated last, by the caller's identity for it.
    pub last_narratable: Option<u64>,
}

impl Default for ScreenNarration {
    fn default() -> Self {
        Self {
            collector: Collector::default(),
            suppress_time: i64::MIN,
            next_time: i64::MAX,
            next_trigger: Trigger::System,
            last_narratable: None,
        }
    }
}

impl ScreenNarration {
    /// The end of `init` (after its `triggerImmediateNarration(false)`):
    /// screens opened from the keyboard wait for the next key before
    /// narrating again; the others stay quiet for two seconds.
    pub fn after_init(&mut self, now: i64, keyboard: bool) {
        self.suppress_time = if keyboard {
            i64::MAX
        } else {
            now.saturating_add(SUPPRESS_AFTER_INIT_MS)
        };
    }

    /// `scheduleNarration(delay, ignoreSuppression, trigger)`.
    pub fn schedule(&mut self, now: i64, delay: i64, ignore_suppression: bool, trigger: Trigger) {
        self.next_time = now.saturating_add(delay);
        self.next_trigger = trigger;
        if ignore_suppression {
            self.suppress_time = i64::MIN;
        }
    }
    /// `scheduleNarration()`: as soon as the screen next ticks.
    pub fn schedule_now(&mut self, now: i64) {
        self.schedule(now, 0, true, Trigger::System);
    }
    pub fn after_mouse_move(&mut self, now: i64) {
        self.schedule(now, DELAY_MOUSE_MOVE_MS, false, Trigger::Mouse);
    }
    pub fn after_mouse_action(&mut self, now: i64) {
        self.schedule(now, DELAY_MOUSE_ACTION_MS, true, Trigger::Mouse);
    }
    pub fn after_keyboard_action(&mut self, now: i64) {
        self.schedule(now, DELAY_KEYBOARD_ACTION_MS, true, Trigger::Keyboard);
    }
    /// `updateNarratorStatus`'s scheduling: once the narrator is turned on,
    /// the screen is narrated two seconds later.
    pub fn after_status_change(&mut self, now: i64, was_disabled: bool, trigger: Trigger) {
        if was_disabled {
            self.schedule(now, SUPPRESS_AFTER_INIT_MS, false, trigger);
        }
    }

    /// `handleDelayedNarration`'s test: the trigger of a narration due now
    /// (only changed elements are said), which is then no longer pending.
    pub fn due(&mut self, now: i64) -> Option<Trigger> {
        if now > self.next_time && now > self.suppress_time {
            self.next_time = i64::MAX;
            Some(self.next_trigger)
        } else {
            None
        }
    }

    /// `runNarration`: rebuilds the narration and returns what to say now
    /// (`saySystemNow`), empty for nothing.
    pub fn run(
        &mut self,
        only_changed: bool,
        trigger: Trigger,
        updater: impl FnOnce(&mut Output<'_>),
    ) -> String {
        self.collector.update(trigger, updater);
        self.collector.collect(!only_changed)
    }
}

/// `Screen.findNarratableWidget` over the screen's active narratables, in
/// order, each with the caller's identity for it and its priority, and the
/// identity of the one narrated last: the first focused one that isn't the
/// last narrated, else the highest priority one (the first of equals), else
/// the last narrated focused one. The index and priority of the one found;
/// `None` when no narratable is focused or hovered.
pub fn find_narratable(
    entries: &[(u64, Priority)],
    last: Option<u64>,
) -> Option<(usize, Priority)> {
    let mut result: Option<(usize, Priority)> = None;
    let mut low_priority = None;
    for (i, &(id, priority)) in entries.iter().enumerate() {
        if priority.is_terminal() {
            if Some(id) != last {
                return Some((i, priority));
            }
            low_priority = Some((i, priority));
        } else if priority > result.map_or(Priority::None, |(_, p)| p) {
            result = Some((i, priority));
        }
    }
    result.or(low_priority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ElementType::*;

    fn run(
        n: &mut ScreenNarration,
        only_changed: bool,
        items: &[(ElementType, u32, &str)],
    ) -> String {
        n.run(only_changed, Trigger::System, |out| {
            for &(kind, depth, text) in items {
                let mut out = Output {
                    collector: out.collector,
                    depth,
                    trigger: Trigger::System,
                };
                out.add(kind, text);
            }
        })
    }

    #[test]
    fn elements_are_read_by_type_then_depth() {
        let mut n = ScreenNarration::default();
        let said = n.run(false, Trigger::System, |out| {
            out.add(Title, "Options");
            out.add(Position, "Screen element 2 out of 9");
            out.add(Usage, "Use Tab to navigate");
            let mut widget = out.nest();
            widget.add(Usage, "Press Enter to activate");
            widget.add(Title, "Button: Done");
        });
        assert_eq!(
            said,
            "Options. Button: Done. Screen element 2 out of 9. Use Tab to navigate. Press Enter to activate"
        );
    }

    #[test]
    fn only_what_changed_is_said_again() {
        let mut n = ScreenNarration::default();
        let first = [(Title, 0, "Options"), (Title, 1, "Button: Done")];
        assert_eq!(run(&mut n, true, &first), "Options. Button: Done");
        assert_eq!(run(&mut n, true, &first), "");
        let moved = [
            (Title, 0, "Options"),
            (Title, 1, "Button: Video Settings..."),
        ];
        assert_eq!(run(&mut n, true, &moved), "Button: Video Settings...");
        // Forced: everything of this generation.
        assert_eq!(
            run(&mut n, false, &moved),
            "Options. Button: Video Settings..."
        );
    }

    #[test]
    fn an_element_missing_for_a_generation_is_said_again_when_it_returns() {
        let mut n = ScreenNarration::default();
        assert_eq!(
            run(&mut n, true, &[(Title, 0, "Options"), (Usage, 0, "Tab")]),
            "Options. Tab"
        );
        assert_eq!(run(&mut n, true, &[(Title, 0, "Options")]), "");
        assert_eq!(
            run(&mut n, true, &[(Title, 0, "Options"), (Usage, 0, "Tab")]),
            "Tab"
        );
    }

    #[test]
    fn lines_are_one_element_read_as_sentences() {
        let mut n = ScreenNarration::default();
        let said = n.run(false, Trigger::System, |out| {
            out.add_lines(Title, vec!["First".into(), "".into(), "Third".into()]);
        });
        assert_eq!(said, "First. . Third");
    }

    #[test]
    fn narration_waits_for_its_delay_and_the_suppression() {
        let mut n = ScreenNarration::default();
        assert_eq!(n.due(0), None);
        n.after_init(1000, false);
        n.after_mouse_move(1000);
        // Due at 1750 but suppressed until 3000.
        assert_eq!(n.due(2000), None);
        assert_eq!(n.due(3001), Some(Trigger::Mouse));
        assert_eq!(n.due(4000), None);
        // A click lifts the suppression.
        n.after_init(5000, false);
        n.after_mouse_action(5000);
        assert_eq!(n.due(5200), None);
        assert_eq!(n.due(5201), Some(Trigger::Mouse));
    }

    #[test]
    fn a_screen_opened_by_keyboard_waits_for_the_next_key() {
        let mut n = ScreenNarration::default();
        n.after_init(0, true);
        n.after_mouse_move(0);
        assert_eq!(n.due(1_000_000), None);
        n.after_keyboard_action(10);
        assert_eq!(n.due(211), Some(Trigger::Keyboard));
    }

    #[test]
    fn turning_the_narrator_on_narrates_the_screen_two_seconds_later() {
        let mut n = ScreenNarration::default();
        n.after_status_change(0, false, Trigger::Keyboard);
        assert_eq!(n.due(10_000), None);
        n.after_status_change(0, true, Trigger::Keyboard);
        assert_eq!(n.due(2000), None);
        assert_eq!(n.due(2001), Some(Trigger::Keyboard));
    }

    #[test]
    fn the_focused_widget_wins_unless_it_was_just_narrated() {
        use Priority::*;
        let ids = |p: &[Priority]| {
            p.iter()
                .enumerate()
                .map(|(i, &p)| (i as u64 + 10, p))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            find_narratable(&ids(&[None, None]), Option::None),
            Option::None
        );
        assert_eq!(
            find_narratable(&ids(&[None, Hovered, Hovered]), Option::None),
            Some((1, Hovered))
        );
        assert_eq!(
            find_narratable(&ids(&[Hovered, Focused]), Option::None),
            Some((1, Focused))
        );
        // The focused one was narrated last: a hovered one goes first.
        assert_eq!(
            find_narratable(&ids(&[Hovered, Focused]), Some(11)),
            Some((0, Hovered))
        );
        assert_eq!(
            find_narratable(&ids(&[None, Focused]), Some(11)),
            Some((1, Focused))
        );
        assert_eq!(
            find_narratable(&ids(&[Focused, Focused]), Some(10)),
            Some((1, Focused))
        );
    }
}
