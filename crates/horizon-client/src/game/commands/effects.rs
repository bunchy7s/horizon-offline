//! `/effect` (EffectCommands) and `/enchant` (EnchantCommand).
use super::{Arg, Context, Game, R, Target, err, texts};
use horizon_client::{
    brewing::{self, namespaced},
    effects::EffectInstance,
    text::{self, Style, Text},
};

/// `CommandResponseTracker` over targets: elements with a non-zero result.
#[derive(Default)]
struct Tracker {
    total: i32,
    changed: Vec<Target>,
}
impl Tracker {
    fn track(&mut self, t: Target, changed: bool) {
        if changed {
            self.total += 1;
            self.changed.push(t);
        }
    }
}
fn effect_name(id: &str) -> Text {
    Text::translate(&brewing::description_id(id), &[])
}
impl Game<'_> {
    fn effect_arg(ctx: &Context) -> R<&'static brewing::EffectDef> {
        let Some(Arg::Resource(id)) = ctx.get("effect") else {
            return Err(err("argument.resource.invalid_type", &[]));
        };
        brewing::effect(id).ok_or_else(|| {
            err(
                "argument.resource.not_found",
                &[id, &"minecraft:mob_effect"],
            )
        })
    }
    /// The executing entity for argument-less forms (the player).
    fn living_targets(&mut self, ctx: &Context) -> R<Vec<Target>> {
        if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", false)
        } else {
            Ok(vec![self.implied_player()])
        }
    }
    /// `EffectCommands.giveEffect`.
    pub(super) fn cmd_effect_give(&mut self, ctx: &Context) -> R<i32> {
        let effect = Self::effect_arg(ctx)?;
        let seconds = if ctx.has("infinite") {
            Some(-1)
        } else {
            Self::int(ctx, "seconds")
        };
        let amplifier = Self::int(ctx, "amplifier").unwrap_or(0);
        let particles = !matches!(ctx.get("hideParticles"), Some(Arg::Bool(true)));
        // computeDurationInTicks
        let duration = match seconds {
            Some(s) if effect.instant => s,
            Some(-1) => -1,
            Some(s) => s * 20,
            None if effect.instant => 1,
            None => 600,
        };
        let mut tracker = Tracker::default();
        for t in self.living_targets(ctx)? {
            let changed = match t {
                Target::Player | Target::Guest(_) => {
                    let mut instance = EffectInstance::new(effect.id, duration, amplifier);
                    instance.visible = particles;
                    self.for_player(t, |g| g.add_player_effect(instance))
                        .unwrap_or(false)
                }
                Target::Mob(id) => match self.mobs.iter_mut().find(|m| m.id == id) {
                    Some(m) => {
                        let name = namespaced(effect.id);
                        let new = !m.has_effect(name);
                        // `new MobEffectInstance(effect, duration, amplifier, false, particles)`.
                        m.add_effect_with(name, duration, amplifier, false, particles);
                        self.mod_effect_added(name, Some(id as i32), amplifier, new);
                        true
                    }
                    None => false,
                },
                _ => continue,
            };
            tracker.track(t, changed);
        }
        let message = match tracker.changed.as_slice() {
            [] => return Err(err("commands.effect.give.failed", &[])),
            [one] => Text::translate(
                "commands.effect.give.success.single",
                &[
                    effect_name(effect.id),
                    self.target_name(*one),
                    Text::literal((duration / 20).to_string()),
                ],
            ),
            many => Text::translate(
                "commands.effect.give.success.multiple",
                &texts(&[
                    effect_name(effect.id),
                    Text::literal(many.len().to_string()),
                    Text::literal((duration / 20).to_string()),
                ]),
            ),
        };
        self.feedback(message);
        Ok(tracker.total)
    }
    /// `EffectCommands.clearEffects` / `clearEffect`.
    pub(super) fn cmd_effect_clear(&mut self, ctx: &Context) -> R<i32> {
        let effect = if ctx.get("effect").is_some() {
            Some(Self::effect_arg(ctx)?)
        } else {
            None
        };
        let mut tracker = Tracker::default();
        for t in self.living_targets(ctx)? {
            let changed = match (t, effect) {
                (Target::Player | Target::Guest(_), None) => self
                    .for_player(t, |g| {
                        let any = !g.potions.effects.list.is_empty();
                        g.potions.effects.clear();
                        g.apply_effect_status();
                        any
                    })
                    .unwrap_or(false),
                (Target::Player | Target::Guest(_), Some(e)) => self
                    .for_player(t, |g| {
                        let before = g.potions.effects.list.len();
                        g.potions.effects.list.retain(|x| x.id != e.id);
                        g.apply_effect_status();
                        g.potions.effects.list.len() != before
                    })
                    .unwrap_or(false),
                (Target::Mob(id), e) => match self.mobs.iter_mut().find(|m| m.id == id) {
                    Some(m) => {
                        let before = m.extra.effects.len();
                        m.extra
                            .effects
                            .retain(|x| e.is_some_and(|e| x.0 != namespaced(e.id)));
                        let left = &m.extra.effects;
                        m.extra
                            .hidden_effects
                            .retain(|h| left.iter().any(|x| x.0 == *h));
                        m.extra
                            .ambient_effects
                            .retain(|h| left.iter().any(|x| x.0 == *h));
                        m.extra.effects.len() != before
                    }
                    None => false,
                },
                _ => continue,
            };
            tracker.track(t, changed);
        }
        let message = match (tracker.changed.as_slice(), effect) {
            ([], None) => return Err(err("commands.effect.clear.everything.failed", &[])),
            ([], Some(_)) => return Err(err("commands.effect.clear.specific.failed", &[])),
            ([one], None) => Text::translate(
                "commands.effect.clear.everything.success.single",
                &[self.target_name(*one)],
            ),
            (many, None) => Text::translate(
                "commands.effect.clear.everything.success.multiple",
                &[Text::literal(many.len().to_string())],
            ),
            ([one], Some(e)) => Text::translate(
                "commands.effect.clear.specific.success.single",
                &[effect_name(e.id), self.target_name(*one)],
            ),
            (many, Some(e)) => Text::translate(
                "commands.effect.clear.specific.success.multiple",
                &texts(&[effect_name(e.id), Text::literal(many.len().to_string())]),
            ),
        };
        self.feedback(message);
        Ok(tracker.total)
    }
    /// `EnchantCommand.enchant` (mobs hold bare item ids, so only the player's
    /// main hand item can carry the enchantment).
    pub(super) fn cmd_enchant(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(id)) = ctx.get("enchantment").cloned() else {
            return Ok(0);
        };
        let level = Self::int(ctx, "level").unwrap_or(1);
        let enchantments = &self.stations().enchantments;
        let Some(def) = enchantments.get(&id) else {
            return Err(err(
                "argument.resource.not_found",
                &[&id, &"minecraft:enchantment"],
            ));
        };
        let max = def.max_level as i32;
        if level > max {
            return Err(err("commands.enchant.failed.level", &[&level, &max]));
        }
        let targets = self.entities(ctx, "targets", false)?;
        let single = targets.len() == 1;
        let mut tracker = Tracker::default();
        for t in targets {
            match t {
                Target::Player => {
                    let Some(stack) = self.inventory[self.selected].clone() else {
                        if single {
                            return Err(err(
                                "commands.enchant.failed.itemless",
                                &[&self.target_name(t).plain()],
                            ));
                        }
                        continue;
                    };
                    let e = &self.stations().enchantments;
                    let compatible = e.can_enchant(self.items(), &id, &stack.item)
                        && stack
                            .crafting_enchantments()
                            .iter()
                            .all(|(other, _)| e.compatible(other, &id));
                    if compatible {
                        if let Some(s) = self.inventory[self.selected].as_mut() {
                            s.enchant(&id, level.max(0) as u32);
                        }
                        tracker.track(t, true);
                    } else if single {
                        // getHoverName().getString(): the custom or item name, no brackets.
                        return Err(err(
                            "commands.enchant.failed.incompatible",
                            &[&stack
                                .custom_name()
                                .unwrap_or_else(|| self.pack.names.item(&stack.item).to_string())],
                        ));
                    }
                }
                Target::Mob(mob) => {
                    let held = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .and_then(|m| m.extra.main_hand.clone());
                    if single {
                        return Err(match held {
                            None => err(
                                "commands.enchant.failed.itemless",
                                &[&self.target_name(t).plain()],
                            ),
                            Some(item) => err(
                                "commands.enchant.failed.incompatible",
                                &[&self.pack.names.item(&item)],
                            ),
                        });
                    }
                }
                _ if single => {
                    return Err(err(
                        "commands.enchant.failed.entity",
                        &[&self.target_name(t).plain()],
                    ));
                }
                _ => {}
            }
        }
        let (name, curse) = self
            .res
            .stations()
            .enchantments
            .full_name(&id, level.max(0) as u32);
        let style = Style {
            color: Some(if curse { text::RED } else { text::GRAY }),
            ..Style::default()
        };
        let full = Text::styled(name, style);
        let message = match tracker.changed.as_slice() {
            [] => return Err(err("commands.enchant.failed", &[])),
            [one] => Text::translate(
                "commands.enchant.success.single",
                &[full, self.target_name(*one)],
            ),
            many => Text::translate(
                "commands.enchant.success.multiple",
                &texts(&[full, Text::literal(many.len().to_string())]),
            ),
        };
        self.feedback(message);
        Ok(tracker.total)
    }
}
