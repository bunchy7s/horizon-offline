//! Client tutorial progression (`net.minecraft.client.tutorial`).

/// The numeric values are the order in `options_file::TUTORIAL_STEPS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Movement = 0,
    FindTree = 1,
    PunchTree = 2,
    OpenInventory = 3,
    CraftPlanks = 4,
    None = 5,
}

impl Step {
    pub(super) fn from_option(value: i32) -> Self {
        match value {
            0 => Self::Movement,
            1 => Self::FindTree,
            2 => Self::PunchTree,
            3 => Self::OpenInventory,
            4 => Self::CraftPlanks,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hint {
    Move,
    Look,
    FindTree,
    PunchTree,
    OpenInventory,
    CraftPlanks,
}

/// The observations needed on the first tick of each step.
#[derive(Default)]
pub(super) struct Previous {
    pub(super) tree_item: bool,
    pub(super) log: bool,
    pub(super) plank: bool,
    pub(super) mined_tree: bool,
    pub(super) crafted_plank: bool,
}

pub(super) struct Tutorial {
    pub(super) step: Step,
    pub(super) waiting: u32,
    moved: bool,
    turned: bool,
    move_ticks: u32,
    look_ticks: u32,
    move_finished: Option<u32>,
    look_finished: bool,
    reset_count: u32,
    pub(super) break_progress: f32,
}

impl Tutorial {
    pub(super) fn new(step: Step) -> Self {
        Self {
            step,
            waiting: 0,
            moved: false,
            turned: false,
            move_ticks: 0,
            look_ticks: 0,
            move_finished: None,
            look_finished: false,
            reset_count: 0,
            break_progress: 0.0,
        }
    }

    /// Returns whether the persisted step changed.
    fn set_step(&mut self, step: Step) -> bool {
        if self.step == step {
            return false;
        }
        *self = Self::new(step);
        true
    }

    pub(super) fn input(&mut self) {
        self.moved = true;
    }
    pub(super) fn mouse(&mut self, x: f64, y: f64) {
        self.turned |= x.abs() > 0.01 || y.abs() > 0.01;
    }

    pub(super) fn tick(&mut self, survival: bool, previous: &Previous) -> bool {
        self.waiting = self.waiting.saturating_add(1);
        if self.step == Step::Movement {
            self.move_ticks += u32::from(std::mem::take(&mut self.moved));
            self.look_ticks += u32::from(std::mem::take(&mut self.turned));
            if self.move_ticks > 40 && self.move_finished.is_none() {
                self.move_finished = Some(self.waiting);
            }
            self.look_finished |= self.look_ticks > 40;
            if self.move_finished.is_some() && self.look_finished {
                return self.set_step(if survival { Step::FindTree } else { Step::None });
            }
        } else if self.step != Step::None && !survival {
            return self.set_step(Step::None);
        } else if self.waiting == 1 {
            match self.step {
                Step::FindTree if previous.tree_item || previous.mined_tree => {
                    return self.set_step(Step::CraftPlanks);
                }
                Step::PunchTree if previous.log || previous.mined_tree => {
                    return self.set_step(Step::CraftPlanks);
                }
                Step::CraftPlanks if previous.plank || previous.crafted_plank => {
                    return self.set_step(Step::None);
                }
                _ => {}
            }
        }
        false
    }

    pub(super) fn look_at_tree(&mut self) -> bool {
        self.step == Step::FindTree && self.set_step(Step::PunchTree)
    }
    pub(super) fn destroy(&mut self, is_log: bool, progress: f32) -> bool {
        if self.step != Step::PunchTree {
            return false;
        }
        if is_log && progress > 0.0 {
            self.break_progress = progress;
            if progress >= 1.0 {
                return self.set_step(Step::OpenInventory);
            }
        } else {
            self.break_progress = 0.0;
            if is_log {
                self.reset_count = self.reset_count.saturating_add(1);
            }
        }
        false
    }
    pub(super) fn open_inventory(&mut self) -> bool {
        self.step == Step::OpenInventory && self.set_step(Step::CraftPlanks)
    }
    pub(super) fn get_item(&mut self, tree: bool, log: bool, plank: bool) -> bool {
        match self.step {
            Step::FindTree if tree => self.set_step(Step::CraftPlanks),
            Step::PunchTree if log => self.set_step(Step::CraftPlanks),
            Step::CraftPlanks if plank => self.set_step(Step::None),
            _ => false,
        }
    }
    pub(super) fn hint(&self) -> Option<(Hint, Option<f32>)> {
        match self.step {
            Step::Movement if self.waiting >= 100 && self.move_finished.is_none() => {
                Some((Hint::Move, Some(self.move_ticks as f32 / 40.0)))
            }
            Step::Movement
                if !self.look_finished
                    && self
                        .move_finished
                        .is_some_and(|t| self.waiting.saturating_sub(t) >= 20)
                    && self.waiting >= 100 =>
            {
                Some((Hint::Look, Some(self.look_ticks as f32 / 40.0)))
            }
            Step::FindTree if self.waiting >= 6000 => Some((Hint::FindTree, None)),
            Step::PunchTree if self.waiting >= 600 || self.reset_count > 3 => {
                Some((Hint::PunchTree, Some(self.break_progress)))
            }
            Step::OpenInventory if self.waiting >= 600 => Some((Hint::OpenInventory, None)),
            Step::CraftPlanks if self.waiting >= 1200 => Some((Hint::CraftPlanks, None)),
            _ => None,
        }
    }
}

impl super::Game<'_> {
    /// `Tutorial.setStep` persists immediately so it survives leaving a world.
    pub(super) fn persist_tutorial_step(&mut self) {
        self.settings.file.set(
            "tutorialStep",
            horizon_client::options_file::V::Int(self.tutorial.step as i32),
        );
        self.settings.save(&Self::options_path());
    }
}

impl super::Game<'_> {
    pub(super) fn reset_tutorial(&mut self) {
        self.tutorial = Tutorial::new(Step::from_option(self.settings.file.int("tutorialStep")));
        self.tutorial_toasts.clear();
    }

    fn tutorial_tagged(&self, item: bool, tag: &str, id: &str) -> bool {
        let items = self.res.items();
        let tags = if item {
            &items.item_tags
        } else {
            &items.block_tags
        };
        tags.get(tag).is_some_and(|members| members.contains(id))
    }

    pub(super) fn tutorial_get_item(&mut self, stack: Option<&horizon_client::items::ItemStack>) {
        // The server thread's settings are a stale copy of the window's, and an
        // in-frame host acting for a guest is not its own player: a guest's
        // tutorial is their own client's to record.
        if self.options.dedicated.is_some()
            || self.serving.is_some()
            || self.lan_acting.is_some()
            || self.title.is_some()
            || self.player.is_none()
        {
            return;
        }
        let Some(stack) = stack else { return };
        let id = &*stack.item;
        let tree = self.tutorial_tagged(true, "minecraft:completes_find_tree_tutorial", id);
        let log = self.tutorial_tagged(true, "minecraft:logs", id);
        let plank = self.tutorial_tagged(true, "minecraft:planks", id);
        if self.tutorial.get_item(tree, log, plank) {
            self.persist_tutorial_step();
        }
    }

    pub(super) fn tutorial_destroy(
        &mut self,
        state: horizon_world::registry::BlockStateId,
        progress: f32,
    ) {
        // The server thread's settings are a stale copy of the window's, and an
        // in-frame host acting for a guest is not its own player: a guest's
        // tutorial is their own client's to record.
        if self.options.dedicated.is_some()
            || self.serving.is_some()
            || self.lan_acting.is_some()
            || self.title.is_some()
            || self.player.is_none()
        {
            return;
        }
        let name = &self.res.facts().get(state).name;
        let is_log = self.tutorial_tagged(false, "minecraft:logs", name);
        if self.tutorial.destroy(is_log, progress) {
            self.persist_tutorial_step();
        }
    }

    pub(super) fn tick_tutorial(&mut self) {
        use winit::keyboard::KeyCode;
        if self.title.is_some()
            || self.player.is_none()
            || self.options.dedicated.is_some()
            || self.serving.is_some()
        {
            return;
        }
        if self.screen == super::Screen::Playing
            && ([
                KeyCode::KeyW,
                KeyCode::KeyA,
                KeyCode::KeyS,
                KeyCode::KeyD,
                KeyCode::Space,
            ]
            .iter()
            .any(|key| self.keys.contains(key))
                || self
                    .pad_move()
                    .is_some_and(|[left, forward]| left.abs() > 0.3 || forward.abs() > 0.3))
        {
            self.tutorial.input();
        }
        let items = self.res.items();
        let in_tag =
            |tag: &str, id: &str| items.item_tags.get(tag).is_some_and(|set| set.contains(id));
        let mut previous = Previous::default();
        if self.tutorial.waiting == 0 && !matches!(self.tutorial.step, Step::Movement | Step::None)
        {
            for stack in self
                .inventory
                .iter()
                .chain(self.armor.iter())
                .flatten()
                .chain(self.offhand.iter())
            {
                previous.tree_item |= in_tag("minecraft:completes_find_tree_tutorial", &stack.item);
                previous.log |= in_tag("minecraft:logs", &stack.item);
                previous.plank |= in_tag("minecraft:planks", &stack.item);
            }
            if let Some(mined) = self.stats.values.get("minecraft:mined")
                && let Some(tree) = items
                    .block_tags
                    .get("minecraft:completes_find_tree_tutorial")
            {
                previous.mined_tree = mined
                    .iter()
                    .any(|(id, count)| *count > 0 && tree.contains(id));
            }
            if let Some(crafted) = self.stats.values.get("minecraft:crafted")
                && let Some(planks) = items.item_tags.get("minecraft:planks")
            {
                previous.crafted_plank = crafted
                    .iter()
                    .any(|(id, count)| *count > 0 && planks.contains(id));
            }
        }
        let survival = self.mode == horizon_client::command::args::GameMode::Survival;
        if self.tutorial.tick(survival, &previous) {
            self.persist_tutorial_step();
        }
        if self.tutorial.step == Step::FindTree
            && self.pick_entity_hit().is_none()
            && let Some(hit) = self.pick()
        {
            let name = &self
                .res
                .facts()
                .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                .name;
            if self.tutorial_tagged(false, "minecraft:completes_find_tree_tutorial", name)
                && self.tutorial.look_at_tree()
            {
                self.persist_tutorial_step();
            }
        }
    }
}

pub(super) struct HintToast {
    hint: Hint,
    lines: Vec<(horizon_client::text::Text, bool)>,
    title_lines: usize,
    progress: Option<f32>,
    smooth: f32,
    last_smooth: i64,
    wanted: bool,
    slot: Option<usize>,
    slots: usize,
    slide: horizon_client::toasts::Slide,
}

impl super::Game<'_> {
    pub(super) fn tutorial_toast_slots(&self) -> u32 {
        self.tutorial_toasts
            .iter()
            .filter_map(|t| t.slot.map(|s| ((1u32 << t.slots) - 1) << s))
            .fold(0, |a, b| a | b)
    }

    fn tutorial_toast(&self, hint: Hint, progress: Option<f32>) -> HintToast {
        use horizon_client::{keybinds, text::Text};
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping)).plain();
        let (title_key, fallback, description_key, description, icon_args) = match hint {
            Hint::Move => (
                "tutorial.move.title",
                "Move with %1$s, %2$s, %3$s, %4$s",
                "tutorial.move.description",
                "Jump with %1$s",
                vec![
                    key("key.forward"),
                    key("key.left"),
                    key("key.back"),
                    key("key.right"),
                ],
            ),
            Hint::Look => (
                "tutorial.look.title",
                "Look around",
                "tutorial.look.description",
                "Use your mouse to turn",
                vec![],
            ),
            Hint::FindTree => (
                "tutorial.find_tree.title",
                "Find a tree",
                "tutorial.find_tree.description",
                "Punch it to collect wood",
                vec![],
            ),
            Hint::PunchTree => (
                "tutorial.punch_tree.title",
                "Punch the tree",
                "tutorial.punch_tree.description",
                "Hold %1$s to break",
                vec![],
            ),
            Hint::OpenInventory => (
                "tutorial.open_inventory.title",
                "Open inventory",
                "tutorial.open_inventory.description",
                "Press %1$s",
                vec![],
            ),
            Hint::CraftPlanks => (
                "tutorial.craft_planks.title",
                "Craft wooden planks",
                "tutorial.craft_planks.description",
                "The recipe book can help",
                vec![],
            ),
        };
        let substitute = |mut text: String, values: &[String]| {
            for (i, value) in values.iter().enumerate() {
                text = text.replace(&format!("%{}$s", i + 1), value);
                text = text.replacen("%s", value, 1);
            }
            text
        };
        let title = substitute(self.translate(title_key, fallback), &icon_args);
        let desc_args = match hint {
            Hint::Move => vec![key("key.jump")],
            Hint::PunchTree => vec![key("key.attack")],
            Hint::OpenInventory => vec![key("key.inventory")],
            _ => vec![],
        };
        let description = substitute(self.translate(description_key, description), &desc_args);
        let font = &self.gui.atlas.font;
        let mut lines = font.split_lines(&Text::literal(title), 126.0);
        let title_lines = lines.len();
        lines.extend(font.split_lines(&Text::literal(description), 126.0));
        let height = 10 + 11 * lines.len().max(2);
        let slots = height.div_ceil(32);
        HintToast {
            hint,
            lines,
            title_lines,
            progress,
            smooth: 0.0,
            last_smooth: 0,
            wanted: true,
            slot: None,
            slots,
            slide: Default::default(),
        }
    }

    pub(super) fn draw_tutorial_toasts(
        &mut self,
        batch: &mut horizon_client::hud::HudBatch,
        hidden: bool,
    ) {
        let wanted = self.tutorial.hint();
        for t in &mut self.tutorial_toasts {
            t.wanted = wanted.is_some_and(|(hint, _)| hint == t.hint);
            if t.wanted {
                t.progress = wanted.and_then(|(_, progress)| progress);
            }
        }
        if let Some((hint, progress)) = wanted
            && !self
                .tutorial_toasts
                .iter()
                .any(|t| t.wanted && t.hint == hint)
        {
            let toast = self.tutorial_toast(hint, progress);
            self.tutorial_toasts.push(toast);
        }
        let mut used = self.toast_slots_used();
        for t in &mut self.tutorial_toasts {
            if t.slot.is_none() && t.wanted && t.slots <= Self::TOAST_SLOTS {
                let mask = (1u32 << t.slots) - 1;
                if let Some(slot) =
                    (0..=Self::TOAST_SLOTS - t.slots).find(|s| used & (mask << s) == 0)
                {
                    t.slot = Some(slot);
                    used |= mask << slot;
                }
            }
        }
        let atlas = &self.gui.atlas;
        let mut i = 0;
        while i < self.tutorial_toasts.len() {
            let t = &mut self.tutorial_toasts[i];
            if t.slot.is_none() {
                if !t.wanted {
                    self.tutorial_toasts.remove(i);
                } else {
                    i += 1;
                }
                continue;
            }
            let (sound, finished) = t.slide.update_now(|_| t.wanted);
            if let Some(show) = sound {
                let event = if show { "ui.toast.in" } else { "ui.toast.out" };
                self.audio
                    .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
            }
            if finished {
                self.tutorial_toasts.remove(i);
                continue;
            }
            if let Some(progress) = t.progress {
                let delta = ((t.slide.fully_for - t.last_smooth) as f32 / 100.0).clamp(0.0, 1.0);
                t.smooth += (progress - t.smooth) * delta;
                t.last_smooth = t.slide.fully_for;
            }
            if !hidden {
                let height = (10 + 11 * t.lines.len().max(2)) as f32;
                let x = batch.width - 160.0 * t.slide.portion;
                let y = 32.0 * t.slot.unwrap_or_default() as f32;
                horizon_client::ui::nine_slice(
                    batch,
                    atlas,
                    "gui/sprites/toast/tutorial",
                    x,
                    y,
                    160.0,
                    height,
                    u32::MAX,
                );
                let icon = match t.hint {
                    Hint::Move => "movement_keys",
                    Hint::Look => "mouse",
                    Hint::FindTree | Hint::PunchTree => "tree",
                    Hint::OpenInventory => "recipe_book",
                    Hint::CraftPlanks => "wooden_planks",
                };
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/toast/{icon}"),
                    x + 6.0,
                    y + 6.0,
                    20.0,
                    20.0,
                    u32::MAX,
                );
                let text_y = y + 7.0 + ((t.lines.len().max(2) - t.lines.len()) * 11) as f32 / 2.0;
                for (n, (line, _)) in t.lines.iter().enumerate() {
                    batch.text_styled(
                        atlas,
                        line,
                        x + 30.0,
                        text_y + 11.0 * n as f32,
                        if n < t.title_lines {
                            0xFF50_0050
                        } else {
                            0xFF00_0000
                        },
                        false,
                    );
                }
                if let Some(progress) = t.progress {
                    batch.fill(atlas, x + 3.0, y + height - 4.0, 154.0, 1.0, u32::MAX);
                    let color = if progress >= t.smooth {
                        0xFF00_5500
                    } else {
                        0xFF55_0000
                    };
                    batch.fill(
                        atlas,
                        x + 3.0,
                        y + height - 4.0,
                        154.0 * t.smooth.clamp(0.0, 1.0),
                        1.0,
                        color,
                    );
                }
            }
            i += 1;
        }
    }
}
