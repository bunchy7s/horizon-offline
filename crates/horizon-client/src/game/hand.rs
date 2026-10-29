//! The local player's hand: clicks and held attack through
//! `horizon_client::first_person` (`Minecraft.startAttack` /
//! `continueAttack`, `MultiPlayerGameMode` block breaking), the arm swing
//! (`LivingEntity.SwingState`) and the first-person equip heights
//! (`FirstPersonHandsAndItems`) with the item swap ticker they read.
use super::{Game, Mining, Screen, minecarts::CartKind};
use horizon_client::{
    Result,
    attack_strength::HeldStack,
    first_person::{Hand, HandWorld, Hit, SwingAnimation, UseResult, swing_duration},
    items::ItemStack,
};
use horizon_core::geometry::{Aabb, Vec3};

/// The off hand's place in `visible_hands` (the main hand uses its hotbar slot).
const OFFHAND: usize = usize::MAX;

/// `AttackRange.effectiveMinRange`, `effectiveMaxRange` (for a player) and
/// `hitboxMargin` of an `attack_range` component.
fn attack_reach(range: &serde_json::Value, creative: bool) -> (f32, f32, f32) {
    let value = |key: &str, default: f32| range[key].as_f64().map_or(default, |v| v as f32);
    let (min, max) = if creative {
        (
            value("min_creative_reach", 0.0),
            value("max_creative_reach", 5.0),
        )
    } else {
        (value("min_reach", 0.0), value("max_reach", 3.0))
    };
    (min, max, value("hitbox_margin", 0.3))
}

fn v3(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// `LocalPlayer.raycastHitResult` with an item that has an attack range
/// (`AttackRange.getClosesetHit`): the distance from the eye to the nearest
/// box hit along the item's own `reach`, which goes past the plain pick's
/// 3 blocks (a spear's 4.5). A miss falls back to the plain pick (`None`).
fn reach_pick(
    boxes: &[(usize, Aabb)],
    eye: Vec3,
    look: Vec3,
    movement: Vec3,
    reach: (f32, f32, f32),
    clip: impl Fn(Vec3, Vec3) -> Option<Vec3>,
) -> Option<f64> {
    super::spear::hits_in_reach(boxes, eye, look, movement, reach, clip)
        .into_iter()
        .map(|(_, at)| eye.distance_squared(at).sqrt())
        .reduce(f64::min)
}

/// `AttackRange.isInRange`: float bounds, then inclusive double comparison.
fn attack_range_contains(range: &serde_json::Value, distance: f64, creative: bool) -> bool {
    let (min, max, margin) = attack_reach(range, creative);
    distance >= f64::from(min - margin) && distance <= f64::from(max + margin)
}

impl Game<'_> {
    /// Opening a screen stops breaking before the next game tick (which may pause).
    pub(super) fn stop_mining(&mut self) {
        if self.clicks.destroying.active {
            let pos = self.clicks.destroying.pos;
            let state = self.world.block(pos[0], pos[1], pos[2]);
            self.tutorial_destroy(state, -1.0);
        }
        let mut destroying = std::mem::take(&mut self.clicks.destroying);
        destroying.stop(&mut GameHands {
            game: self,
            error: None,
        });
        self.clicks.destroying = destroying;
        self.attack_clicks = 0;
        self.request_remote_mining_stop();
        self.mining = None;
    }
    /// `LivingEntity.swing` with `getModifiedSwingDuration`.
    pub(super) fn swing_hand(&mut self, hand: Hand, animation: SwingAnimation) {
        let effects = &self.potions.effects;
        let dig_speed = effects
            .amplifier("haste")
            .max(effects.amplifier("conduit_power"));
        let fatigue = effects.amplifier("mining_fatigue");
        let duration = swing_duration(animation, dig_speed, fatigue);
        if self.swing.start_if_able(hand, animation, duration) {
            self.swings_started += 1;
        }
    }
    /// `swing(hand)` for the hand that acts, with the default animation.
    pub(super) fn swing_arm(&mut self) {
        self.swing_hand(self.hand, SwingAnimation::DEFAULT);
    }
    /// `Player.resetAttackStrengthTicker`.
    pub(super) fn reset_attack_strength(&mut self) {
        self.attack_ticks = 0;
        self.hand_strength.reset();
    }
    /// The attack and use part of `Minecraft.tick` for the local player:
    /// attack clicks since the last tick and the held attack button.
    pub(super) fn tick_local_hand(&mut self) -> Result<()> {
        let attack_clicks = std::mem::take(&mut self.attack_clicks);
        if self.screen == Screen::Playing && self.title.is_none() && self.sleeping.is_none() {
            let mut clicks = std::mem::take(&mut self.clicks);
            // keyAttack.isDown(); playing implies the grabbed mouse here
            // (scripts under --control hold buttons without grabbing it).
            let down = self.left;
            let ticks = self.clicks.destroying.ticks;
            let mut world = GameHands {
                game: self,
                error: None,
            };
            clicks.tick(&mut world, attack_clicks, 0, down, false);
            let error = world.error.take();
            self.clicks = clicks;
            if let Some(e) = error {
                return Err(e);
            }
            // A tick of breaking: the hit sound every fourth tick and the
            // cracking particles.
            let d = &self.clicks.destroying;
            if d.active && d.ticks > ticks {
                let (pos, first) = (d.pos, (d.ticks as u32 - 1).is_multiple_of(4));
                let state = self.world.block(pos[0], pos[1], pos[2]);
                if first {
                    let sounds = self.res.facts().get(state).sounds.clone();
                    self.audio.play(
                        &sounds.hit,
                        Some(pos.map(|v| f64::from(v) + 0.5)),
                        (sounds.volume + 1.0) / 8.0,
                        sounds.pitch * 0.5,
                    );
                }
                if let Some(hit) = self.pick() {
                    self.crack_particle(pos, hit.face, state);
                }
            }
        } else {
            // Minecraft.tick: a screen keeps the attack from starting.
            self.clicks.miss_time = 10000;
        }
        let d = &self.clicks.destroying;
        self.mining = d.active.then_some(Mining {
            pos: d.pos,
            progress: d.progress,
            ticks: d.ticks as u32,
        });
        if let Some(mining) = &self.mining {
            let pos = mining.pos;
            let progress = mining.progress;
            let state = self.world.block(pos[0], pos[1], pos[2]);
            self.tutorial_destroy(state, progress);
        }
        Ok(())
    }
    /// `BlockBehaviour.getDestroyProgress` with `Player.getDestroySpeed`.
    pub(super) fn destroy_progress(&self, pos: [i32; 3]) -> f32 {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts().get(state);
        if facts.hardness == -1.0 {
            return 0.0;
        }
        let held = self.held();
        let mut speed = self.res.items().mining_speed(held, &facts.name);
        speed = self.modify_destroy_speed(speed);
        if let Some(eye) = self.eye_position()
            && self
                .res
                .facts()
                .get(self.world.block(
                    eye[0].floor() as i32,
                    eye[1].floor() as i32,
                    eye[2].floor() as i32,
                ))
                .fluid
                == horizon_client::blocks::Fluid::Water
        {
            speed *= self.submerged_mining_speed();
        }
        if self.player.as_ref().is_some_and(|p| !p.ground.on_ground) {
            speed /= 5.0;
        }
        let harvest = !facts.requires_tool || self.res.items().correct_for_drops(held, &facts.name);
        speed / facts.hardness / if harvest { 30.0 } else { 100.0 }
    }
    /// A key for the main-hand stack (`destroyingItem`): its item and components.
    pub(super) fn main_hand_key(&self) -> String {
        self.held()
            .map_or(String::new(), |s| format!("{} {:?}", s.item, s.extra))
    }
    /// Whether an attack would hit an entity rather than the block behind
    /// it: the crosshair's `EntityHitResult`.
    pub(super) fn entity_targeted(&self) -> bool {
        let before = |d: Option<f64>| d.is_some_and(|d| self.before_block(d));
        before(self.painting_hit().map(|h| h.1))
            || before(self.frame_hit().map(|h| h.1))
            || before(self.armor_stand_hit().map(|h| h.1))
            || before(self.cushion_hit().map(|h| h.1))
            || before(self.boat_hit().map(|h| h.1))
            || before(self.cart_hit().map(|h| h.1))
            || before(self.dragon_hit().map(|h| h.1))
            || before(self.mob_hit().map(|h| h.1))
            || before(self.guest_hit().map(|h| h.1))
            || before(self.redirectable_hit().map(|h| h.1))
    }
    /// `LocalPlayer.isHandsBusy`: paddling a boat.
    fn hands_busy(&self) -> bool {
        self.riding
            .is_some_and(|id| self.boats.iter().any(|b| b.id == id))
            && [
                winit::keyboard::KeyCode::KeyW,
                winit::keyboard::KeyCode::KeyS,
                winit::keyboard::KeyCode::KeyA,
                winit::keyboard::KeyCode::KeyD,
            ]
            .iter()
            .any(|k| self.keys.contains(k))
    }
    /// `MultiPlayerGameMode.startUseItem`'s bookkeeping around the game's
    /// use: nothing starts while a block is being broken or the hands are
    /// busy, and `LocalPlayer.itemUsed` drops the hand after a use that
    /// swung and spent (or, with infinite materials, kept) the item, or that
    /// started using it.
    pub(super) fn use_with_hand(&mut self) -> Result<()> {
        if self.clicks.destroying.active || self.hands_busy() {
            return Ok(());
        }
        let before = [self.stack_in(self.selected).cloned(), self.offhand.clone()];
        let swings = self.swings_started;
        let using = self.using.is_some();
        let Some(hand) = self.use_item()? else {
            return Ok(());
        };
        let (before, after) = match hand {
            Hand::Main => (&before[0], self.stack_in(self.selected).cloned()),
            Hand::Off => (&before[1], self.offhand.clone()),
        };
        let swung = self.swings_started != swings;
        let spent = match (before, &after) {
            (Some(a), Some(b)) => a.count != b.count || a.item != b.item || self.creative,
            _ => false,
        };
        if (swung && spent) || (!using && self.using.is_some() && after.is_some()) {
            self.heights.item_used(hand);
        }
        Ok(())
    }
    /// `LocalPlayer.tick`: the swing (`LivingEntity.baseTick`), the attack
    /// strength and item swap tickers (`Player.tick`), then
    /// `FirstPersonHandsAndItems.tick`.
    pub(super) fn tick_hand(&mut self) {
        // LocalPlayer.applyInput (the camera is the player's own).
        if self.player.is_some() {
            self.sway.tick(self.pitch, self.yaw);
        }
        self.swing.tick();
        let main = self.inventory[self.selected].clone();
        // Player.tick: a different item resets the attack strength, and
        // detectEquipmentUpdates gives the player its modifiers.
        if self.hand_strength.tick(main.as_ref().map(|s| HeldStack {
            item: s.item.to_string(),
            rest: format!("{} {} {:?}", s.count, s.damage, s.extra),
        })) {
            self.attack_ticks = 0;
        }
        self.attack_item = main.as_ref().map(|s| s.item.clone());
        let next = [(self.selected, main), (OFFHAND, self.offhand.clone())];
        // shouldInstantlyReplaceVisibleItem: the same stack, or an equal one
        // (matchesIgnoringComponents; every item plays the swap animation).
        // A stack that stays in its slot with the same item is the same
        // stack, whatever its count (vanilla shrinks and grows it in place).
        let mut changed = [false; 2];
        for h in 0..2 {
            let same = match (&self.visible_hands[h], &next[h]) {
                ((_, None), (_, None)) => true,
                ((slot, Some(a)), (next_slot, Some(b))) => {
                    a.item == b.item
                        && (slot == next_slot || (a.count == b.count && a.extra == b.extra))
                }
                _ => false,
            };
            if same {
                self.visible_hands[h] = next[h].clone();
            }
            changed[h] = !same;
        }
        let speed = self.main_hand_attack_speed();
        let replace = self.heights.tick(
            self.hands_busy(),
            self.hand_strength.swap_scale(1.0, speed),
            changed,
        );
        for h in 0..2 {
            if replace[h] {
                self.visible_hands[h] = next[h].clone();
            }
        }
    }
    /// `Minecraft.crosshairPickEntity` when it is a `LivingEntity`: whether
    /// it is alive (the nearest entity under the crosshair, before any block).
    pub(super) fn crosshair_living(&self) -> Option<bool> {
        let nearest = |hits: &[Option<f64>]| hits.iter().flatten().copied().reduce(f64::min);
        let other = nearest(&[
            self.painting_hit().map(|h| h.1),
            self.frame_hit().map(|h| h.1),
            self.boat_hit().map(|h| h.1),
            self.cart_hit().map(|h| h.1),
            self.dragon_hit().map(|h| h.1),
        ]);
        let active = self
            .using
            .and_then(|(slot, _)| self.inventory.get(slot).and_then(Option::as_ref))
            .or_else(|| self.held());
        let range = active.and_then(|stack| {
            self.pack
                .item_renderer
                .models
                .components(&horizon_client::item_render::stack_view(stack))
                .get("minecraft:attack_range")
                .cloned()
        });
        let spear = range.as_ref().and_then(|r| {
            let eye = self.eye_position()?;
            let look = super::view_vector(self.yaw, self.pitch);
            let reach = attack_reach(r, self.creative);
            let boxes = self.hittable_mob_boxes();
            let clip = |from: Vec3, to: Vec3| {
                let length = from.distance_squared(to).sqrt();
                let d = to.subtract(from).scale(1.0 / length);
                (length > 0.0)
                    .then(|| self.clip_ray([from.x, from.y, from.z], [d.x, d.y, d.z], length))
                    .flatten()
                    .map(|hit| Vec3::new(hit.point[0], hit.point[1], hit.point[2]))
            };
            let movement = self.known_player_movement();
            reach_pick(&boxes, v3(eye), v3(look), movement, reach, clip)
        });
        let living = match spear {
            Some(distance) => distance,
            None => nearest(&[
                self.mob_hit().map(|h| h.1),
                self.armor_stand_hit().map(|h| h.1),
            ])?,
        };
        if (spear.is_none() && !self.before_block(living)) || other.is_some_and(|o| o <= living) {
            return None;
        }
        // Hud.extractCrosshair: with an attack range, the hit must be in it.
        if let Some(range) = &range
            && !attack_range_contains(range, living, self.creative)
        {
            return None;
        }
        // Only alive mobs are hit, so a picked living entity is alive.
        Some(true)
    }
    /// `Hud.canRenderCrosshairForSpectator` for the nearest hit in reach: an
    /// entity that is a `MenuProvider` (a chest or hopper minecart, a chest
    /// boat), or a block whose `showAsInteractableInSpectatorMode` holds.
    pub(super) fn spectator_crosshair(&self) -> bool {
        let cart = self.cart_hit().map(|(i, d)| {
            let kind = &self.carts[i].kind;
            (d, matches!(kind, CartKind::Chest | CartKind::Hopper))
        });
        let boat = self.boat_hit().map(|(i, d)| (d, self.boats[i].chest));
        let others = [
            self.mob_hit().map(|h| h.1),
            self.armor_stand_hit().map(|h| h.1),
            self.cushion_hit().map(|h| h.1),
            self.painting_hit().map(|h| h.1),
            self.frame_hit().map(|h| h.1),
            self.dragon_hit().map(|h| h.1),
        ];
        let nearest = cart
            .into_iter()
            .chain(boat)
            .chain(others.into_iter().flatten().map(|d| (d, false)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((distance, menu)) = nearest
            && self.before_block(distance)
        {
            return menu;
        }
        let Some(hit) = self.pick() else {
            return false;
        };
        let id = &*self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
            .name;
        let name = id.trim_start_matches("minecraft:");
        match name {
            // BlockBehaviour.getMenuProvider and `instanceof Portal`.
            "crafting_table" | "enchanting_table" | "anvil" | "chipped_anvil" | "damaged_anvil"
            | "stonecutter" | "loom" | "cartography_table" | "grindstone" | "smithing_table"
            | "barrel" | "furnace" | "blast_furnace" | "smoker" | "brewing_stand" | "hopper"
            | "dispenser" | "dropper" | "crafter" | "lectern" | "beacon" | "nether_portal"
            | "end_portal" | "end_gateway" => true,
            // ChestBlock.getMenuProvider: none while a redstone conductor
            // sits on top of either half (a sitting cat aside). Copper
            // chests are `ChestBlock`s too.
            _ if horizon_client::containers::is_chest_block(id) => {
                let key = (hit.pos[0], hit.pos[1], hit.pos[2]);
                let blocked = |p: (i32, i32, i32)| {
                    let above = self.world.block(p.0, p.1 + 1, p.2);
                    self.res.behaviours().facts(above).flags
                        & horizon_world::level::data::state_flags::CONDUCTOR
                        != 0
                };
                !blocked(key) && self.chest_other_half(key).is_none_or(|o| !blocked(o))
            }
            _ => name.ends_with("shulker_box"),
        }
    }
    /// `Attributes.ATTACK_SPEED` with the main-hand item's modifier and the
    /// Haste and Mining Fatigue modifiers (`ADD_MULTIPLIED_TOTAL`), clamped to
    /// the attribute's range.
    pub(super) fn main_hand_attack_speed(&self) -> f64 {
        let value = horizon_client::attack_strength::BASE_ATTACK_SPEED
            + self
                .attack_item_info()
                .map_or(0.0, |i| f64::from(i.attack_speed));
        self.potions
            .effects
            .modifiers()
            .into_iter()
            .filter(|m| m.0 == "minecraft:attack_speed")
            .fold(value, |v, m| v * (1.0 + m.2))
            .clamp(0.0, 1024.0)
    }
    /// The item whose attack damage and speed modifiers the player has (`attack_item`).
    pub(super) fn attack_item_info(&self) -> Option<&horizon_client::items::ItemInfo> {
        self.res.items().get(self.attack_item.as_deref()?)
    }
    /// `evaluateWhichHandsToRender`: which of the main and off hand show.
    pub(super) fn hands_to_render(&self) -> [bool; 2] {
        let held = |slot| {
            self.stack_in(slot)
                .map(|s: &ItemStack| (&*s.item, s.data().is_some_and(|d| !d.charged.is_empty())))
        };
        let using = self.used_hand().zip(
            self.using
                .and_then(|(slot, _)| self.stack_in(slot))
                .map(|s| &*s.item),
        );
        horizon_client::first_person::hands_to_render(
            [held(self.selected), held(super::shield::OFFHAND_SLOT)],
            using,
        )
    }
    /// The visible main- and off-hand items.
    pub(super) fn visible_hand_items(&self) -> [Option<ItemStack>; 2] {
        [
            self.visible_hands[0].1.clone(),
            self.visible_hands[1].1.clone(),
        ]
    }
}

/// The game as `first_person`'s world: the crosshair, breaking and attacking.
struct GameHands<'g, 'a> {
    game: &'g mut Game<'a>,
    error: Option<Box<dyn std::error::Error>>,
}
impl GameHands<'_, '_> {
    /// The main-hand item's `kinetic_weapon` and `piercing_weapon`, a spear's.
    fn attack_kinetic(&self) -> Option<horizon_client::spear::Kinetic> {
        self.game
            .held()
            .and_then(|s| horizon_client::spear::Kinetic::of(&s.item))
    }
    fn keep(&mut self, r: Result<()>) {
        if let Err(e) = r
            && self.error.is_none()
        {
            self.error = Some(e);
        }
    }
}
impl HandWorld for GameHands<'_, '_> {
    fn hit(&mut self) -> Hit {
        if self.game.entity_targeted() {
            return Hit::Entity;
        }
        match self.game.pick() {
            Some(h) => Hit::Block {
                pos: h.pos,
                face: match h.face {
                    [0, -1, 0] => 0,
                    [0, 1, 0] => 1,
                    [0, 0, -1] => 2,
                    [0, 0, 1] => 3,
                    [-1, 0, 0] => 4,
                    _ => 5,
                },
            },
            None => Hit::Miss,
        }
    }
    fn is_air(&mut self, pos: [i32; 3]) -> bool {
        let state = self.game.world.block(pos[0], pos[1], pos[2]);
        self.game.res.facts().get(state).name.ends_with("air")
    }
    fn creative(&self) -> bool {
        self.game.creative
    }
    fn spectator(&self) -> bool {
        self.game.spectator()
    }
    fn instabuild(&self) -> bool {
        self.game.creative
    }
    fn hands_busy(&self) -> bool {
        self.game.hands_busy()
    }
    fn using_item(&self) -> bool {
        self.game.using.is_some()
    }
    fn item_enabled(&mut self, _: Hand) -> bool {
        true
    }
    fn held_empty(&mut self, hand: Hand) -> bool {
        match hand {
            Hand::Main => self.game.held().is_none(),
            Hand::Off => self.game.offhand.is_none(),
        }
    }
    fn held_count(&mut self, hand: Hand) -> i32 {
        let stack = match hand {
            Hand::Main => self.game.held(),
            Hand::Off => self.game.offhand.as_ref(),
        };
        stack.map_or(0, |s| s.count as i32)
    }
    fn infinite_materials(&self) -> bool {
        self.game.creative
    }
    fn cannot_attack(&mut self) -> bool {
        let ticker = self.game.hand_strength.ticker;
        self.game.cannot_attack_with_item(ticker, 0)
    }
    fn piercing_weapon(&mut self) -> bool {
        self.attack_kinetic().is_some()
    }
    fn attack_animation(&mut self) -> SwingAnimation {
        self.attack_kinetic()
            .map_or(SwingAnimation::DEFAULT, |k| k.attack_animation())
    }
    fn interact_animation(&mut self, _: Hand) -> SwingAnimation {
        SwingAnimation::DEFAULT
    }
    fn piercing_attack(&mut self, _: SwingAnimation) {
        // The server's part (`PiercingWeapon.attack`), then the client's:
        // the sound `MultiPlayerGameMode.piercingAttack` plays.
        if let Some(sound) = self.game.jab() {
            self.game.play_at_feet(sound);
        }
        self.game.hand_strength.reset_attack();
    }
    fn attack_entity(&mut self) -> bool {
        self.game.attack()
    }
    fn block_action_restricted(&mut self, _: [i32; 3]) -> bool {
        !self.game.may_build()
    }
    fn within_border(&mut self, _: [i32; 3]) -> bool {
        true
    }
    fn attack_block(&mut self, pos: [i32; 3]) {
        let result = self.game.attack_block(pos);
        self.keep(result);
    }
    fn destroy_progress(&mut self, pos: [i32; 3]) -> f32 {
        self.game.destroy_progress(pos)
    }
    fn destroy_block(&mut self, pos: [i32; 3]) -> bool {
        if self.is_air(pos) {
            return false;
        }
        // Item.canDestroyBlock: swords, tridents and maces cannot break
        // blocks in creative.
        let held = self.game.held().map(|s| s.item.to_string());
        if self.game.creative
            && held.is_some_and(|i| {
                i.ends_with("_sword") || i == "minecraft:trident" || i == "minecraft:mace"
            })
        {
            return false;
        }
        let state = self.game.world.block(pos[0], pos[1], pos[2]);
        self.game.tutorial_destroy(state, 1.0);
        let r = self.game.break_block(pos);
        self.keep(r);
        true
    }
    fn main_hand_key(&mut self) -> String {
        self.game.main_hand_key()
    }
    fn destroy_stage(&mut self, _: [i32; 3], _: Option<i32>) {}
    fn swing(&mut self, hand: Hand, animation: SwingAnimation) {
        self.game.swing_hand(hand, animation);
    }
    fn reset_attack_strength(&mut self) {
        self.game.reset_attack_strength();
    }
    fn entity_within_border(&mut self) -> bool {
        true
    }
    fn use_on_entity(&mut self, _: Hand) -> Option<UseResult> {
        None
    }
    fn use_on_block(&mut self, _: Hand) -> UseResult {
        UseResult::Pass
    }
    fn use_item(&mut self, _: Hand) -> UseResult {
        UseResult::Pass
    }
    fn item_used(&mut self, hand: Hand) {
        self.game.heights.item_used(hand);
    }
    fn release_using(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zombie-sized box whose near face is `distance` blocks ahead of an
    /// eye at the origin looking along +z.
    fn mob_ahead(distance: f64) -> Vec<(usize, Aabb)> {
        let min = Vec3::new(-0.3, -1.0, distance);
        vec![(0, Aabb::new(min, min.add_vector(Vec3::new(0.6, 1.95, 0.6))))]
    }

    #[test]
    fn a_spear_picks_a_mob_past_the_plain_reach() {
        // #3209: a mob 4 blocks ahead is out of the plain pick's 3 blocks but
        // within a spear's attack range (min 2, max 4.5, margin 0.125), so the
        // crosshair picks it and the attack indicator shows.
        let spear = serde_json::json!({
            "min_reach": 2.0, "max_reach": 4.5,
            "min_creative_reach": 2.0, "max_creative_reach": 6.5,
            "hitbox_margin": 0.125,
        });
        let (eye, look) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        let boxes = mob_ahead(4.0);
        let reach = attack_reach(&spear, false);
        let pick = reach_pick(&boxes, eye, look, Vec3::ZERO, reach, |_, _| None);
        assert_eq!(pick, Some(4.0));
        assert!(attack_range_contains(&spear, 4.0, false));
        // Past the maximum plus the margin: no pick.
        let far = mob_ahead(4.7);
        assert_eq!(
            reach_pick(&far, eye, look, Vec3::ZERO, reach, |_, _| None),
            None
        );
        // A block nearer than the minimum reach takes the pick.
        let wall = |from: Vec3, _: Vec3| Some(from.add_vector(Vec3::new(0.0, 0.0, 1.0)));
        assert_eq!(reach_pick(&boxes, eye, look, Vec3::ZERO, reach, wall), None);
    }

    #[test]
    fn attack_range_uses_inclusive_float_bounds() {
        let range = serde_json::json!({
            "min_reach": 2.0, "max_reach": 4.5,
            "min_creative_reach": 2.0, "max_creative_reach": 6.5,
            "hitbox_margin": 0.125,
        });
        for (creative, min, max) in [(false, 1.875, 4.625), (true, 1.875, 6.625)] {
            for distance in [min, max] {
                assert!(attack_range_contains(&range, distance, creative));
            }
            for distance in [min - 0.0001, max + 0.0001] {
                assert!(!attack_range_contains(&range, distance, creative));
            }
        }
        let defaults = serde_json::json!({});
        for (creative, reach) in [(false, 3.0f32), (true, 5.0f32)] {
            let max = f64::from(reach + 0.3f32);
            assert!(attack_range_contains(&defaults, max, creative));
            assert!(!attack_range_contains(&defaults, max + 1e-8, creative));
        }
    }
}
