//! The player's advancements in the game: criteria triggered from game
//! events (`CriteriaTriggers`), awarding with rewards, the toast and the
//! chat announcement, and `players/advancements/<uuid>.json`.
use super::{Dim, Game};
use horizon_client::{
    advancements::{self as adv, EntityView, Frame, ItemView, Progress, Tree},
    hud::HudBatch,
    items::ItemStack,
    text::{self, Style, Text},
    toasts::Slide,
};
use horizon_core::geometry::Vec3;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Instant,
};

pub(super) struct Toast {
    icon: String,
    frame: Frame,
    title: Text,
    slide: Slide,
    /// `ToastInstance.firstSlotIndex`: the 32-pixel slot it stacks in once it
    /// leaves the queue.
    slot: Option<usize>,
    /// A `RecipeToast`: the crafting station and result of each display
    /// unlocked while it lasts (`recipeItems`); empty for an advancement.
    recipes: Vec<(String, ItemStack)>,
    /// `RecipeToast.changed` and `lastChanged`.
    changed: bool,
    last_changed: i64,
}
impl Toast {
    /// `RecipeToast.addOrUpdate` on a new toast.
    pub(super) fn recipe(items: Vec<(String, ItemStack)>) -> Self {
        Self {
            icon: String::new(),
            frame: Frame::Task,
            title: Text::literal(""),
            slide: Slide::default(),
            slot: None,
            recipes: items,
            changed: true,
            last_changed: 0,
        }
    }
    pub(super) fn is_recipe(&self) -> bool {
        !self.recipes.is_empty()
    }
    /// `RecipeToast.addItem`: more displays on a toast still queued or shown.
    pub(super) fn add_recipes(&mut self, items: Vec<(String, ItemStack)>) {
        self.recipes.extend(items);
        self.changed = true;
    }
    /// `Toast.update` with `fullyVisibleForMs`: whether it wants to be shown.
    fn wants_show(&mut self, fully_for: i64, multiplier: f64) -> bool {
        let time = 5000.0 * multiplier;
        if self.recipes.is_empty() {
            // AdvancementToast.
            return (fully_for as f64) < time;
        }
        if self.changed {
            self.last_changed = fully_for;
            self.changed = false;
        }
        ((fully_for - self.last_changed) as f64) < time
    }
    /// `RecipeToast.displayedRecipeIndex`: the displays take turns over the
    /// toast's display time.
    fn displayed_recipe(&self, fully_for: i64, multiplier: f64) -> usize {
        let n = self.recipes.len() as f64;
        (fully_for as f64 / (5000.0 * multiplier / n).max(1.0) % n) as usize
    }
}
/// Where the player's fall, lava ride and Nether trip began, the blast that
/// last hit the player, and the effects the last tick saw (`ServerPlayer`'s
/// fields of the same names). A guest keeps their own, as `levitation` does.
#[derive(Clone, Debug, Default)]
pub(super) struct TravelMarks {
    /// `startingToFallPosition`.
    pub(super) fall: Option<[f64; 3]>,
    /// `currentExplosionImpactPos`, and whether a wind charge made the blast.
    pub(super) blast: Option<([f64; 3], bool)>,
    /// `enteredLavaOnVehiclePosition`.
    pub(super) lava_ride: Option<[f64; 3]>,
    /// `enteredNetherPosition`.
    pub(super) nether: Option<[f64; 3]>,
    /// The (effect, amplifier) pairs `EffectsChangedTrigger` last saw.
    pub(super) effects: Vec<(&'static str, i32)>,
}
#[derive(Default)]
pub(super) struct AdvState {
    pub(super) tree: Arc<Tree>,
    /// `ClientAdvancements`: a server's tree carries each advancement's
    /// place on its tab (`DisplayInfo` x, y), in tree order.
    pub(super) positions: Option<Vec<(f32, f32)>>,
    pub(super) progress: Progress,
    pub(super) loaded: bool,
    pub(super) toasts: VecDeque<Toast>,
    /// Awards skip the toast (`/advancement grant ... everything`).
    pub(super) quiet: bool,
    /// The inventory last seen by `inventory_changed`.
    pub(super) last_inventory: u64,
    /// `ServerPlayer.levitationStartPos` and `levitationStartTime`.
    pub(super) levitation: Option<([f64; 3], u64)>,
    /// The fall, lava ride, Nether trip, blast and effects `tick_marks` follows.
    pub(super) marks: TravelMarks,
    /// While the game acts for a network player (`as_guest`): the
    /// advancements whose progress changed (`progressChanged`), the recipes
    /// unlocked and the announcements, for their client and everyone's.
    pub(super) changed: BTreeSet<usize>,
    pub(super) unlocked: Vec<String>,
    pub(super) announced: Vec<Text>,
    /// The tree being read on another thread since the world opened
    /// (`prefetch_advancement_tree`), taken by the first `advancement_tree`.
    pub(super) pending_tree: Option<std::thread::JoinHandle<Tree>>,
    /// `inventory_changed`'s criteria by item, built for `tree`.
    inventory_index: Option<(Arc<Tree>, Arc<adv::InventoryIndex>)>,
}
/// The game's world for predicates.
struct View<'a, 'g>(&'a Game<'g>);
impl adv::World for View<'_, '_> {
    fn dimension(&self) -> &str {
        self.0.dim.id()
    }
    fn biome(&self, p: [i32; 3]) -> String {
        self.0
            .world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map(|c| {
                self.0
                    .res
                    .biomes()
                    .get(c.biome(p[0], p[1], p[2]))
                    .name
                    .clone()
            })
            .unwrap_or_default()
    }
    fn structures(&self, p: [i32; 3]) -> Vec<String> {
        self.0.structures_at(p)
    }
    fn block(&self, p: [i32; 3]) -> (String, BTreeMap<String, String>) {
        let state = self.0.world.block(p[0], p[1], p[2]);
        let name = self.0.res.facts().get(state).name.to_string();
        let props = self
            .0
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| {
                s.properties
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (name, props)
    }
    fn fluid(&self, p: [i32; 3]) -> Option<String> {
        match self
            .0
            .res
            .facts()
            .get(self.0.world.block(p[0], p[1], p[2]))
            .fluid
        {
            horizon_client::blocks::Fluid::Water => Some("minecraft:water".into()),
            horizon_client::blocks::Fluid::Lava => Some("minecraft:lava".into()),
            _ => None,
        }
    }
    fn can_see_sky(&self, p: [i32; 3]) -> bool {
        (p[1] + 1..p[1] + 200).all(|y| {
            !self
                .0
                .res
                .facts()
                .get(self.0.world.block(p[0], y, p[2]))
                .full_collision
        })
    }
    fn light(&self, p: [i32; 3]) -> i32 {
        let l = self.0.world.light(p[0], p[1], p[2]);
        i32::from((l >> 4).max(l & 15))
    }
    fn smokey(&self, p: [i32; 3]) -> bool {
        (1..=5).any(|d| {
            self.0
                .res
                .facts()
                .get(self.0.world.block(p[0], p[1] - d, p[2]))
                .name
                .ends_with("campfire")
        })
    }
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        let items = &self.0.res.items();
        match registry {
            "item" => items.item_tags.get(tag).is_some_and(|t| t.contains(id)),
            "block" => items.block_tags.get(tag).is_some_and(|t| t.contains(id)),
            "entity_type" => self.0.res.mob_data().entity_has(tag, id),
            "worldgen/biome" => self.0.res.mob_data().biome_has(tag, id),
            "damage_type" => self
                .0
                .res
                .stations()
                .enchantments
                .damage_tags
                .get(tag)
                .is_some_and(|t| t.contains(id)),
            _ => false,
        }
    }
}
fn now_text() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    adv::date_text(secs)
}
impl Game<'_> {
    /// `ItemPredicate.test` (the `item_filter` of a filtered item modifier).
    pub(super) fn loot_item_filter(&self, predicate: &Value, s: &ItemStack) -> bool {
        adv::item(predicate, &item_view(s), &View(self))
    }
}
pub(super) fn item_view(s: &ItemStack) -> ItemView {
    ItemView {
        id: s.item.to_string(),
        count: s.count,
        damage: s.damage,
        enchantments: s
            .enchantments()
            .iter()
            .map(|(id, l)| (id.to_string(), *l))
            .collect(),
        potion: s.potion().map(str::to_owned),
        custom_name: s.custom_name().is_some(),
    }
}
impl Game<'_> {
    fn adv_path(&self) -> Option<std::path::PathBuf> {
        Some(super::stats::player_json(
            &self.world_root,
            "advancements",
            self.level.player_uuid?,
        ))
    }
    /// A server with no player of its own and nobody it acts for: there is
    /// no player for advancements and statistics.
    pub(super) fn no_player_here(&self) -> bool {
        self.options.dedicated.is_some() && self.lan_acting.is_none()
    }
    /// The advancement tree, loaded once.
    pub(super) fn advancement_tree(&mut self) -> Arc<Tree> {
        if self.adv.tree.list.is_empty() {
            let tree = match self.adv.pending_tree.take().map(|h| h.join()) {
                Some(Ok(tree)) => tree,
                _ => Tree::load(&self.options.resources.join("data")),
            };
            self.adv.tree = Arc::new(tree);
            // With the tree, while the world loads: built on the first
            // inventory change it cost that tick ~10 ms.
            self.inventory_index();
        }
        self.adv.tree.clone()
    }
    /// `inventory_changed`'s criteria by item, for the tree in place.
    fn inventory_index(&mut self) -> Arc<adv::InventoryIndex> {
        let tree = self.adv.tree.clone();
        if let Some((built, index)) = &self.adv.inventory_index
            && Arc::ptr_eq(built, &tree)
        {
            return index.clone();
        }
        let items = self.res.items();
        let index = Arc::new(adv::InventoryIndex::build(&tree, |tag| {
            items
                .item_tags
                .get(tag)
                .map(|t| t.iter().map(String::as_str).collect())
                .unwrap_or_default()
        }));
        self.adv.inventory_index = Some((tree, index.clone()));
        index
    }
    /// Start reading the advancement files on another thread while the world
    /// loads. The first tick's first criterion trigger used to read and parse
    /// all of them on the game thread (about 60 ms).
    pub(super) fn prefetch_advancement_tree(&mut self) {
        if !self.adv.tree.list.is_empty() || self.adv.pending_tree.is_some() {
            return;
        }
        let data = self.options.resources.join("data");
        self.adv.pending_tree = std::thread::Builder::new()
            .name("horizon-advancements".into())
            .spawn(move || Tree::load(&data))
            .ok();
    }
    pub(super) fn load_advancements(&mut self) {
        if self.adv.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.adv.loaded = true;
        self.advancement_tree();
        let progress = self
            .adv_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Progress::from_json(&v))
            .unwrap_or_default();
        // The known recipes are the player's own (`recipeBook`), not the
        // rewards of done advancements: /recipe take sticks.
        let tree = self.adv.tree.clone();
        self.adv.progress = progress;
        self.ui.registries.advancements = tree.list.iter().map(|a| a.id.clone()).collect();
    }
    pub(super) fn save_advancements(&self) {
        if !self.adv.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        let Some(path) = self.adv_path() else {
            return;
        };
        let json = self
            .adv
            .progress
            .to_json(&self.adv.tree, horizon_world::DATA_VERSION);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
    }
    /// The player as predicates see it.
    /// The predicate view of a command target.
    pub(super) fn entity_view_of(&self, t: super::commands::Target) -> EntityView {
        use super::commands::Target;
        match t {
            Target::Player => self.player_entity(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m))
                .unwrap_or_default(),
            other => EntityView {
                kind: self.target_type(other),
                position: self.target_position(other).unwrap_or_default(),
                ..EntityView::default()
            },
        }
    }
    /// A `ContextAwarePredicate` for `this` at `origin`.
    pub(super) fn conditions_hold(
        &self,
        v: &Value,
        this: Option<&EntityView>,
        origin: [f64; 3],
    ) -> bool {
        adv::conditions(
            v,
            &adv::Entities {
                this,
                origin,
                tool: None,
                block_pos: None,
            },
            &View(self),
        )
    }
    pub(super) fn player_entity(&self) -> EntityView {
        let Some(p) = self.player.as_ref() else {
            return EntityView::default();
        };
        let mut eq: [Option<ItemView>; 7] = Default::default();
        eq[0] = self.held().map(item_view);
        eq[1] = self.offhand.as_ref().map(item_view);
        for (i, a) in self.armor.iter().enumerate() {
            eq[2 + i] = a.as_ref().map(item_view);
        }
        let vehicle = self.riding.and_then(|id| {
            self.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| Box::new(self.mob_entity(m)))
        });
        EntityView {
            kind: "minecraft:player".into(),
            position: [p.position.x, p.position.y, p.position.z],
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
            swimming: p.pose == horizon_client::pose::Pose::Swimming,
            flying: p.flying,
            on_ground: p.ground.on_ground,
            equipment: eq,
            vehicle,
            effects: self
                .potions
                .effects
                .list
                .iter()
                .map(|e| (format!("minecraft:{}", e.id), e.amplifier, e.duration))
                .collect(),
            gamemode: Some(self.mode.name().to_owned()),
            ..Default::default()
        }
    }
    pub(super) fn mob_entity(&self, m: &horizon_client::mobs::Mob) -> EntityView {
        // mainhand, offhand, head, chest, legs, feet.
        let slot = |i: &Option<std::sync::Arc<str>>| {
            i.as_ref().map(|id| item_view(&ItemStack::new(id, 1)))
        };
        let mut equipment: [Option<ItemView>; 7] = Default::default();
        if !horizon_client::mobs::equine::is_equine(m.species()) {
            equipment[0] = slot(&m.extra.main_hand);
            equipment[1] = slot(&m.extra.off_hand);
            equipment[2] = slot(&m.extra.head);
            equipment[3] = slot(&m.extra.chest);
            equipment[4] = slot(&m.extra.legs);
            equipment[5] = slot(&m.extra.feet);
        }
        // A wolf's armor is its body slot, with its durability.
        if m.species() == horizon_client::mobs::Species::Wolf {
            equipment[6] = m.extra.head.as_ref().map(|id| {
                let mut armor = ItemStack::new(id, 1);
                armor.damage = m.extra.body_damage.max(0) as u32;
                item_view(&armor)
            });
        }
        EntityView {
            kind: m.kind.id.clone(),
            position: [m.position.x, m.position.y, m.position.z],
            on_fire: m.fire_ticks > 0,
            baby: m.is_baby(),
            on_ground: m.on_ground,
            equipment,
            components: horizon_client::mobs::finalize::variant_component(m)
                .map(|(id, value)| (id.to_owned(), value))
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }
    /// `SimpleCriterionTrigger.trigger`: test each open criterion of the
    /// trigger and award what it completes.
    pub(super) fn trigger(&mut self, name: &str, test: impl Fn(&Value, &dyn adv::World) -> bool) {
        if self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let Some(list) = tree.by_trigger.get(name) else {
            return;
        };
        self.trigger_criteria(&tree, list, test);
    }
    /// `trigger` for some of a trigger's criteria: (advancement, criterion)
    /// indexes in `tree`, in tree order.
    fn trigger_criteria(
        &mut self,
        tree: &Tree,
        list: &[(usize, usize)],
        test: impl Fn(&Value, &dyn adv::World) -> bool,
    ) {
        let mut hits = Vec::new();
        {
            let view = View(self);
            for &(ai, ci) in list {
                let a = &tree.list[ai];
                let c = &a.criteria[ci];
                if self.adv.progress.has(&a.id, &c.name) || self.adv.progress.done(a) {
                    continue;
                }
                if test(&c.conditions, &view) {
                    hits.push((ai, ci));
                }
            }
        }
        if hits.is_empty() {
            return;
        }
        let date = now_text();
        for (ai, ci) in hits {
            let a = &tree.list[ai];
            let was = self.adv.progress.done(a);
            self.adv.progress.grant(&a.id, &a.criteria[ci].name, &date);
            self.progress_changed(ai);
            if !was && self.adv.progress.done(a) {
                self.award(ai);
            }
        }
    }
    /// `PlayerAdvancements.progressChanged`, kept for a network player's client.
    fn progress_changed(&mut self, index: usize) {
        if self.lan_acting.is_some() {
            self.adv.changed.insert(index);
        }
    }
    /// After `ServerRecipeBook.addRecipes`: the toast and the
    /// `recipe_unlocked` trigger of each recipe now known.
    pub(super) fn recipe_unlocked(&mut self, unlocked: &[String]) {
        if self.lan_acting.is_some() {
            // A network player's client gets them in ClientboundRecipeBookAddPacket.
            self.adv.unlocked.extend_from_slice(unlocked);
        } else {
            self.recipe_toast(unlocked);
        }
        if unlocked.is_empty() || self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.adv.tree.clone();
        for r in unlocked {
            let list = tree.recipe_criteria(r);
            self.trigger_criteria(&tree, &list, |c, _| {
                c["recipes"].as_str() == Some(r.as_str())
            });
        }
    }
    /// `PlayerAdvancements.award` once done: rewards, toast, announcement.
    fn award(&mut self, index: usize) {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        // AdvancementRewards.grant
        if a.rewards.experience > 0 {
            self.experience.give(a.rewards.experience);
        }
        // ServerPlayer.awardRecipesByKey: the rewards that name a recipe.
        let rewards: Vec<String> = a
            .rewards
            .recipes
            .iter()
            .filter(|r| self.res.stations().recipe_index.contains(r))
            .cloned()
            .collect();
        let (_, unlocked) = self
            .level
            .recipe_book
            .add_recipes(&self.res.stations().recipe_index, &rewards);
        for table in &a.rewards.loot {
            let items = self
                .res
                .items()
                .loot_table(table, false, None, &mut self.world.random);
            for s in items {
                if let Some(rest) = self.give(s)
                    && let Some(eye) = self.eye_position()
                {
                    self.spawn_item(rest, eye, [0.0; 3], 0);
                }
            }
        }
        if let Some(d) = &a.display {
            // A network player's client shows its own toast.
            if d.show_toast && !self.adv.quiet && self.lan_acting.is_none() {
                self.adv.toasts.push_back(Toast {
                    icon: d.icon.clone(),
                    frame: d.frame,
                    title: text_of(&d.title),
                    slide: Slide::default(),
                    slot: None,
                    recipes: Vec::new(),
                    changed: false,
                    last_changed: 0,
                });
            }
            if d.announce && self.ui.rules.bool("show_advancement_messages") {
                let key = match d.frame {
                    Frame::Task => "chat.type.advancement.task",
                    Frame::Goal => "chat.type.advancement.goal",
                    Frame::Challenge => "chat.type.advancement.challenge",
                };
                let name = adv_name(a);
                // PlayerList.broadcastSystemMessage: everyone hears of a
                // network player's advancement.
                match self.lan_acting_name.clone() {
                    // Player.getDisplayName.
                    Some(_) => {
                        let player = self.target_name(super::commands::Target::Player);
                        self.adv
                            .announced
                            .push(Text::translate(key, &[player, name]));
                    }
                    None => {
                        let text = Text::translate(key, &[self.player_display_text(), name]);
                        // The guests hear of the host's advancement too.
                        if let Some(host) = &self.lan {
                            host.hub
                                .system_message(horizon_client::lan::data::component_nbt(
                                    &text.component(),
                                ));
                        }
                        self.system_message(text);
                    }
                }
            }
        }
        self.recipe_unlocked(&unlocked);
    }
    /// `InventoryChangeTrigger`, checked when the inventory changes.
    pub(super) fn check_inventory_trigger(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
        {
            s.as_ref().map(|s| (&s.item, s.count)).hash(&mut h);
        }
        let key = h.finish();
        if key == self.adv.last_inventory {
            return;
        }
        self.adv.last_inventory = key;
        let stacks: Vec<ItemView> = self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
            .flatten()
            .map(item_view)
            .collect();
        // InventoryChangeTrigger slots: occupied, full and empty counts.
        let all: Vec<&Option<ItemStack>> = self
            .inventory
            .iter()
            .chain(self.armor.iter())
            .chain(std::iter::once(&self.offhand))
            .collect();
        let occupied = all.iter().filter(|s| s.is_some()).count() as f64;
        let empty = all.len() as f64 - occupied;
        let full = all
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|s| {
                self.res
                    .items()
                    .get(&s.item)
                    .is_some_and(|i| s.count >= i.max_stack)
            })
            .count() as f64;
        // Only the criteria the items held could pass (`InventoryIndex`).
        if self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let index = self.inventory_index();
        let list = index.candidates(stacks.iter().map(|s| s.id.as_str()));
        self.trigger_criteria(&tree, &list, move |c, w| {
            let slots = &c["slots"];
            let range = |k: &str, v: f64| {
                let r = &slots[k];
                r.is_null()
                    || r.as_f64().is_some_and(|n| (n - v).abs() < 0.5)
                    || r.is_object()
                        && r["min"].as_f64().is_none_or(|m| v >= m)
                        && r["max"].as_f64().is_none_or(|m| v <= m)
            };
            if !(range("occupied", occupied) && range("full", full) && range("empty", empty)) {
                return false;
            }
            let mut wanted: Vec<&Value> = c["items"]
                .as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            for s in &stacks {
                wanted.retain(|p| !adv::item(p, s, w));
            }
            wanted.is_empty()
        });
    }
    /// `PlayerTrigger.LOCATION` every second, and the other per-tick checks.
    pub(super) fn tick_advancements(&mut self) {
        if self.player.is_none() || self.remote.is_some() {
            return;
        }
        self.check_inventory_trigger();
        // ServerPlayer.tick: PlayerTrigger.TICK every tick.
        let me = self.player_entity();
        self.trigger("minecraft:tick", |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &ctx, w)
        });
        // ServerPlayer.onEffectAdded and onEffectsRemoved keep the start of a
        // levitation; tick fires LEVITATION while there is one.
        let levitating = self
            .potions
            .effects
            .list
            .iter()
            .any(|e| e.id == "levitation");
        match (levitating, self.adv.levitation) {
            (true, None) => {
                let at = self
                    .player
                    .as_ref()
                    .map(|p| [p.position.x, p.position.y, p.position.z]);
                self.adv.levitation = at.map(|at| (at, self.ticks));
            }
            (false, Some(_)) => self.adv.levitation = None,
            _ => {}
        }
        if let Some((start, since)) = self.adv.levitation {
            self.trigger_levitation(start, (self.ticks - since) as i64);
        }
        self.tick_marks();
        if self.ticks.is_multiple_of(20) {
            let me = self.player_entity();
            self.trigger("minecraft:location", |c, w| {
                let ctx = adv::Entities {
                    this: Some(&me),
                    origin: me.position,
                    tool: None,
                    block_pos: None,
                };
                adv::conditions(&c["player"], &ctx, w)
            });
        }
    }
    /// A player condition plus one entity (`player`, `entity` keys).
    pub(super) fn trigger_entity(&mut self, name: &str, key: &str, target: &EntityView) {
        let me = self.player_entity();
        let target = target.clone();
        let key = key.to_owned();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let ectx = adv::Entities {
                this: Some(&target),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::conditions(&c[&key], &ectx, w)
        });
    }
    /// `KilledTrigger` (`player_killed_entity`): the entity killed and the
    /// killing blow's damage source (none: any `killing_blow` fails).
    pub(super) fn trigger_killed(&mut self, target: &EntityView, blow: Option<&adv::DamageView>) {
        let me = self.player_entity();
        let target = target.clone();
        let blow = blow.cloned();
        self.trigger("minecraft:player_killed_entity", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let ectx = adv::Entities {
                this: Some(&target),
                ..pctx
            };
            let blow_ok = c["killing_blow"].is_null()
                || blow
                    .as_ref()
                    .is_some_and(|d| adv::damage_source(&c["killing_blow"], d, w, me.position));
            adv::conditions(&c["player"], &pctx, w)
                && adv::conditions(&c["entity"], &ectx, w)
                && blow_ok
        });
    }
    /// `KilledByArrowTrigger`: what the arrow killed and the weapon it came from.
    pub(super) fn trigger_killed_by_arrow(
        &mut self,
        victims: &[EntityView],
        weapon: Option<&ItemStack>,
    ) {
        let me = self.player_entity();
        let victims = victims.to_vec();
        let weapon = weapon.map(item_view);
        self.trigger("minecraft:killed_by_arrow", move |c, w| {
            adv::killed_by_arrow(c, &me, &victims, weapon.as_ref(), w)
        });
    }
    /// `LevitationTrigger`: how far the player rose from where it started
    /// levitating, and for how long.
    fn trigger_levitation(&mut self, start: [f64; 3], duration: i64) {
        let me = self.player_entity();
        self.trigger("minecraft:levitation", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], start, me.position))
                && adv::range_i(&c["duration"], duration)
        });
    }
    /// `ServerPlayer.tick`: `trackStartFallingPosition` and the end of a fall
    /// (`resetFallDistance`), `trackEnteredOrExitedLavaOnVehicle`, the way back
    /// from the Nether, and `onEffectAdded`, `onEffectUpdated` and
    /// `onEffectsRemoved`, which are seen as each tick ends.
    fn tick_marks(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let falling = p.fall_distance > 0.0;
        let dead = p.health.dead_or_dying();
        let lava = self
            .riding
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .is_some_and(|m| m.fluids.in_lava);
        match (falling, self.adv.marks.fall) {
            (true, None) => {
                self.adv.marks.fall = Some(at);
                // The blast a wind charge made is spent by the fall, and counts
                // only if the player was above where it hit.
                if let Some((impact, wind_charge)) = self.adv.marks.blast.take()
                    && wind_charge
                    && impact[1] <= at[1]
                {
                    self.trigger_fall_after_explosion(impact);
                }
            }
            // `resetFallDistance`: a fall ends by landing, climbing or water.
            (false, Some(start)) => {
                self.adv.marks.fall = None;
                if !dead {
                    self.trigger_distance("minecraft:fall_from_height", start);
                }
            }
            _ => {}
        }
        match (lava, self.adv.marks.lava_ride) {
            (true, None) => self.adv.marks.lava_ride = Some(at),
            (true, Some(start)) => self.trigger_distance("minecraft:ride_entity_in_lava", start),
            (false, Some(_)) => self.adv.marks.lava_ride = None,
            (false, None) => {}
        }
        // Back in the Overworld from the Nether: measured from where it was entered.
        if let Some(start) = self.adv.marks.nether
            && self.dim == Dim::Overworld
        {
            self.adv.marks.nether = None;
            self.trigger_distance("minecraft:nether_travel", start);
        }
        let effects: Vec<(&'static str, i32)> = self
            .potions
            .effects
            .list
            .iter()
            .map(|e| (e.id, e.amplifier))
            .collect();
        if effects != self.adv.marks.effects {
            self.adv.marks.effects = effects;
            self.trigger_effects_changed(None);
        }
    }
    /// `DistanceTrigger`: the player's `player` condition, `start_position`
    /// at `start`, and the `distance` from `start` to the player.
    fn trigger_distance(&mut self, name: &str, start: [f64; 3]) {
        let me = self.player_entity();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::location(&c["start_position"], start, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], start, me.position))
        });
    }
    /// `FallAfterExplosionTrigger` for a wind charge's blast that hit the
    /// player at `impact`, measured to where the player is now.
    fn trigger_fall_after_explosion(&mut self, impact: [f64; 3]) {
        let me = self.player_entity();
        let cause = EntityView {
            kind: "minecraft:wind_charge".into(),
            position: impact,
            ..EntityView::default()
        };
        self.trigger("minecraft:fall_after_explosion", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let cctx = adv::Entities {
                this: Some(&cause),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::location(&c["start_position"], impact, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], impact, me.position))
                && adv::conditions(&c["cause"], &cctx, w)
        });
    }
    /// `EffectsChangedTrigger`: the player's `effects`, and for an effect that
    /// `source` gave, that entity (`source`).
    pub(super) fn trigger_effects_changed(&mut self, source: Option<EntityView>) {
        let me = self.player_entity();
        self.trigger("minecraft:effects_changed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let effects = c["effects"].is_null()
                || adv::entity(
                    &serde_json::json!({ "minecraft:effects": c["effects"] }),
                    &me,
                    w,
                    me.position,
                );
            let source = c["source"].is_null()
                || source.as_ref().is_some_and(|s| {
                    let sctx = adv::Entities {
                        this: Some(s),
                        origin: me.position,
                        tool: None,
                        block_pos: None,
                    };
                    adv::conditions(&c["source"], &sctx, w)
                });
            adv::conditions(&c["player"], &pctx, w) && effects && source
        });
    }
    /// `ItemDurabilityTrigger`: a player's item takes damage (`ItemStack.applyDamage`),
    /// `stack` as it is before the damage. The durability and delta ranges are not
    /// read: no advancement in the data uses them.
    pub(super) fn trigger_item_durability(&mut self, stack: &ItemStack) {
        if self.remote.is_some() || self.no_player_here() {
            return;
        }
        let me = self.player_entity();
        let item = item_view(stack);
        self.trigger("minecraft:item_durability_changed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && (c["item"].is_null() || adv::item(&c["item"], &item, w))
        });
    }
    /// `ServerPlayer.updateUsingItem`: USING_ITEM for the item in use, with
    /// what the player looks at (`PlayerPredicate.lookingAt`).
    pub(super) fn trigger_using_item(&mut self, stack: &ItemStack) {
        if self.remote.is_some() || self.no_player_here() {
            return;
        }
        let mut me = self.player_entity();
        me.looking_at = self.looking_at_entity().map(Box::new);
        let it = item_view(stack);
        self.trigger("minecraft:using_item", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::item(&c["item"], &it, w)
        });
    }
    /// The mob a 100 block ray from the eyes meets first, if no block is in
    /// the way (`ProjectileUtil.getEntityHitResult` and `hasLineOfSight`).
    fn looking_at_entity(&self) -> Option<EntityView> {
        let eye = self.eye_position()?;
        let dir = super::view_vector(self.yaw, self.pitch);
        let (m, _) = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .filter_map(|m| Some((m, super::ray_box(eye, dir, 100.0, m.bounds())?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        // `LivingEntity.hasLineOfSight`: eye to the target's eye height,
        // `ClipContext.Block.COLLIDER`, no fluids, within 128 blocks.
        let from = Vec3::new(eye[0], eye[1], eye[2]);
        let to = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let world = self.mob_world_base(&[]);
        let clear = to.distance(from) <= 128.0 && world.clear_between(from, to, false);
        clear.then(|| self.mob_entity(m))
    }
    /// `PlayerHurtEntityTrigger`: the player, the damage and the victim.
    pub(super) fn trigger_player_hurt_entity(
        &mut self,
        victim: &EntityView,
        damage: &adv::DamageView,
    ) {
        let me = self.player_entity();
        let (victim, damage) = (victim.clone(), damage.clone());
        self.trigger("minecraft:player_hurt_entity", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let vctx = adv::Entities {
                this: Some(&victim),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::damage(&c["damage"], &damage, w, me.position)
                && adv::conditions(&c["entity"], &vctx, w)
        });
    }
    /// The player's hit on `victim` as `LivingEntity.hurtServer` reports it:
    /// `kind` damage of `amount` from `direct` (the player's own hand when
    /// `None`), nothing blocked.
    pub(super) fn player_hit(
        &mut self,
        victim: &EntityView,
        kind: &str,
        direct: Option<EntityView>,
        amount: f32,
    ) {
        let me = self.player_entity();
        self.trigger_player_hurt_entity(
            victim,
            &adv::DamageView {
                kind: kind.to_owned(),
                is_direct: direct.is_none(),
                direct: direct.or_else(|| Some(me.clone())),
                source: Some(me),
                original: amount,
                actual: amount,
                blocked: false,
            },
        );
    }
    /// `EntityHurtPlayerTrigger`: the player and the damage it took.
    pub(super) fn trigger_entity_hurt_player(&mut self, damage: &adv::DamageView) {
        let me = self.player_entity();
        let damage = damage.clone();
        self.trigger("minecraft:entity_hurt_player", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::damage(&c["damage"], &damage, w, me.position)
        });
    }
    /// A player condition plus one item (`item` key).
    pub(super) fn trigger_item(&mut self, name: &str, stack: &ItemStack) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::item(&c["item"], &it, w)
        });
    }
    pub(super) fn trigger_item_entity(
        &mut self,
        name: &str,
        stack: &ItemStack,
        target: &EntityView,
    ) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger(name, |c, w| adv::item_entity(c, &me, target, &it, w));
    }
    /// `ConstructBeaconTrigger`: the pyramid levels of the new beacon.
    pub(super) fn trigger_construct_beacon(&mut self, levels: i32) {
        let me = self.player_entity();
        self.trigger("minecraft:construct_beacon", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::range_i(&c["level"], i64::from(levels))
        });
    }
    /// `EnchantedItemTrigger`: the enchanted item and the levels it cost.
    pub(super) fn trigger_enchanted_item(&mut self, stack: &ItemStack, levels: i32) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger("minecraft:enchanted_item", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::item(&c["item"], &it, w)
                && adv::range_i(&c["levels"], i64::from(levels))
        });
    }
    /// `BrewedPotionTrigger`: the potion contents of a stack taken from a
    /// brewing stand.
    pub(super) fn trigger_brewed_potion(&mut self, stack: &ItemStack) {
        let me = self.player_entity();
        let potion = stack.potion().map(str::to_owned);
        self.trigger("minecraft:brewed_potion", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::potions(&c["potion"], potion.as_deref(), w)
        });
    }
    /// `TradeTrigger`: the villager and the item the trade gave.
    pub(super) fn trigger_trade(&mut self, villager: &EntityView, stack: &ItemStack) {
        let me = self.player_entity();
        let villager = villager.clone();
        let it = item_view(stack);
        self.trigger("minecraft:villager_trade", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let vctx = adv::Entities {
                this: Some(&villager),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::conditions(&c["villager"], &vctx, w)
                && adv::item(&c["item"], &it, w)
        });
    }
    /// `FishingRodHookedTrigger`: the rod, what the hook is in (or the hook)
    /// and the items it caught.
    pub(super) fn trigger_fishing_rod_hooked(
        &mut self,
        rod: &ItemStack,
        hooked: &EntityView,
        items: &[ItemStack],
    ) {
        let me = self.player_entity();
        let rod = item_view(rod);
        let hooked = hooked.clone();
        let items: Vec<ItemView> = items.iter().map(item_view).collect();
        self.trigger("minecraft:fishing_rod_hooked", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let ectx = adv::Entities {
                this: Some(&hooked),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::item(&c["rod"], &rod, w)
                && adv::conditions(&c["entity"], &ectx, w)
                && (c["item"].is_null() || items.iter().any(|i| adv::item(&c["item"], i, w)))
        });
    }
    /// `EnterBlockTrigger`: the block, its state properties and the player.
    pub(super) fn trigger_enter_block(&mut self, state: horizon_world::registry::BlockStateId) {
        let Some(definition) = self.pack.assets.registry.state(state).cloned() else {
            return;
        };
        let me = self.player_entity();
        self.trigger("minecraft:enter_block", move |c, w| {
            let block_ok = adv::id_matches(&c["blocks"], "block", &definition.name, w);
            let state_ok = c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .all(|(k, v)| definition.properties.get(k).map(String::as_str) == v.as_str());
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            block_ok && state_ok && adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `SlideDownBlockTrigger`: match the actual block and its state.
    pub(super) fn trigger_honey_slide(&mut self, pos: [i32; 3]) {
        let me = self.player_entity();
        self.trigger("minecraft:slide_down_block", move |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &ctx, w) && adv::block(c, pos, w)
        });
    }

    /// `TargetBlockTrigger`: signal and projectile conditions are evaluated in
    /// the shooter's context, including distance from the shooter.
    pub(super) fn trigger_target_hit(&mut self, projectile: &EntityView, strength: i32) {
        let me = self.player_entity();
        let projectile = projectile.clone();
        self.trigger("minecraft:target_hit", move |c, w| {
            let player = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let target = adv::Entities {
                this: Some(&projectile),
                ..player
            };
            adv::conditions(&c["player"], &player, w)
                && adv::range_i(&c["signal_strength"], i64::from(strength))
                && adv::conditions(&c["projectile"], &target, w)
        });
    }

    /// `ItemUsedOnLocationTrigger` (placed_block, item_used_on_block).
    pub(super) fn trigger_block(&mut self, name: &str, pos: [i32; 3], stack: &ItemStack) {
        let me = self.player_entity();
        let it = item_view(stack);
        self.trigger(name, move |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: Some(&it),
                block_pos: Some(pos),
            };
            adv::conditions(&c["location"], &ctx, w)
        });
    }
    /// `LightningStrikeTrigger`: the bolt, and the mobs around it that it did
    /// not hit (`bystander`), each tested from this player's position.
    pub(super) fn trigger_lightning_strike(&mut self, bolt: &EntityView, viewers: &[EntityView]) {
        let me = self.player_entity();
        let (bolt, viewers) = (bolt.clone(), viewers.to_vec());
        self.trigger("minecraft:lightning_strike", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let bctx = adv::Entities {
                this: Some(&bolt),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::conditions(&c["lightning"], &bctx, w)
                && (c["bystander"].is_null()
                    || viewers.iter().any(|v| {
                        adv::conditions(
                            &c["bystander"],
                            &adv::Entities {
                                this: Some(v),
                                ..pctx
                            },
                            w,
                        )
                    }))
        });
    }
    /// `ChanneledLightningTrigger`: each victim condition matched by some
    /// mob the bolt hit, from the player who channelled it.
    pub(super) fn trigger_channeled_lightning(&mut self, victims: &[EntityView]) {
        let me = self.player_entity();
        let victims = victims.to_vec();
        self.trigger("minecraft:channeled_lightning", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && c["victims"].as_array().into_iter().flatten().all(|want| {
                    victims.iter().any(|v| {
                        adv::conditions(
                            want,
                            &adv::Entities {
                                this: Some(v),
                                ..pctx
                            },
                            w,
                        )
                    })
                })
        });
    }
    /// `SpearMobsTrigger`: the living entities the spear stabbed in this use.
    pub(super) fn trigger_spear_mobs(&mut self, stabbed: usize) {
        let me = self.player_entity();
        self.trigger("minecraft:spear_mobs", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && c["count"].as_u64().is_none_or(|n| stabbed as u64 >= n)
        });
    }
    /// `BeeNestDestroyedTrigger`: the nest's block state, the tool that broke
    /// it and the bees left inside.
    pub(super) fn trigger_bee_nest_destroyed(
        &mut self,
        state: horizon_world::registry::BlockStateId,
        tool: Option<&ItemStack>,
        bees: i64,
    ) {
        let Some(definition) = self.pack.assets.registry.state(state).cloned() else {
            return;
        };
        let me = self.player_entity();
        let tool = tool.map(item_view).unwrap_or_default();
        self.trigger("minecraft:bee_nest_destroyed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            let state_ok = c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .all(|(k, v)| definition.properties.get(k).map(String::as_str) == v.as_str());
            adv::id_matches(&c["blocks"], "block", &definition.name, w)
                && state_ok
                && adv::item(&c["item"], &tool, w)
                && adv::range_i(&c["num_bees_inside"], bees)
                && adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// A trigger with only the player condition (slept_in_bed, started_riding).
    pub(super) fn trigger_player(&mut self, name: &str) {
        let me = self.player_entity();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `RecipeCraftedTrigger`: the recipe and the stacks that went into it
    /// (`ServerPlayer.triggerRecipeCrafted`); an empty slot is air.
    pub(super) fn trigger_recipe_crafted(
        &mut self,
        recipe: &str,
        ingredients: &[Option<ItemStack>],
    ) {
        let me = self.player_entity();
        let used: Vec<ItemView> = ingredients
            .iter()
            .map(|s| {
                s.as_ref().map_or_else(
                    || ItemView {
                        id: "minecraft:air".to_owned(),
                        ..Default::default()
                    },
                    item_view,
                )
            })
            .collect();
        let recipe = recipe.to_owned();
        self.trigger("minecraft:recipe_crafted", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::id_matches(&c["recipes"], "recipe", &recipe, w)
                && adv::recipe_ingredients(&c["ingredients"], &used, w)
        });
    }
    /// `LootTableTrigger`: a container's loot table unpacked for this player
    /// (`RandomizableContainer.unpackLootTable`).
    pub(super) fn trigger_loot_table(&mut self, table: &str) {
        let me = self.player_entity();
        let table = table.to_owned();
        self.trigger("minecraft:player_generates_container_loot", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::id_matches(&c["loot_tables"], "loot_table", &table, w)
        });
    }
    /// `BredAnimalsTrigger`.
    pub(super) fn trigger_bred(
        &mut self,
        parent: &EntityView,
        partner: &EntityView,
        child: Option<&EntityView>,
    ) {
        let me = self.player_entity();
        let (parent, partner, child) = (parent.clone(), partner.clone(), child.cloned());
        self.trigger("minecraft:bred_animals", move |c, w| {
            let ctx = |e: &EntityView| {
                adv::Entities {
                    this: Some(e),
                    origin: me.position,
                    tool: None,
                    block_pos: None,
                }
                .this
                .map(|_| ())
            };
            let _ = ctx;
            let test = |key: &str, e: Option<&EntityView>| {
                c[key].is_null()
                    || e.is_some_and(|e| {
                        adv::conditions(
                            &c[key],
                            &adv::Entities {
                                this: Some(e),
                                origin: me.position,
                                tool: None,
                                block_pos: None,
                            },
                            w,
                        )
                    })
            };
            // matches(parent, partner, child) or (partner, parent, child).
            (test("parent", Some(&parent)) && test("partner", Some(&partner))
                || test("parent", Some(&partner)) && test("partner", Some(&parent)))
                && test("child", child.as_ref())
        });
    }
    /// `ChangeDimensionTrigger`, and `ServerPlayer.enteredNetherPosition`: the
    /// Overworld position the player went into the Nether from (`at`) is kept
    /// for the way back, and any other trip drops it.
    pub(super) fn trigger_dimension(&mut self, from: Dim, to: Dim, at: [f64; 3]) {
        let (from_id, to_id) = (from.id().to_owned(), to.id().to_owned());
        self.trigger("minecraft:changed_dimension", move |c, _| {
            c["from"].as_str().is_none_or(|f| f == from_id)
                && c["to"].as_str().is_none_or(|t| t == to_id)
        });
        if from == Dim::Overworld && to == Dim::Nether {
            self.adv.marks.nether = Some(at);
        } else if !(from == Dim::Nether && to == Dim::Overworld) {
            self.adv.marks.nether = None;
        }
    }
    /// `ToastManager.SLOT_COUNT`.
    pub(super) const TOAST_SLOTS: usize = 5;
    /// `ToastManager.update` and `extractRenderState` for the `AdvancementToast`s and
    /// `RecipeToast`s: up to five stack in 32-pixel slots, the rest queue. `hidden` (F1)
    /// keeps them timing out without drawing them.
    pub(super) fn draw_toasts(&mut self, batch: &mut HudBatch, hidden: bool) {
        let multiplier = f64::from(self.settings.notification_display_time);
        let mut sound_played = false;
        let mut i = 0;
        while i < self.adv.toasts.len() {
            let t = &mut self.adv.toasts[i];
            if t.slot.is_none() {
                i += 1;
                continue;
            }
            let origin = *t.slide.origin.get_or_insert_with(Instant::now);
            let now = origin.elapsed().as_millis() as i64;
            let mut slide = std::mem::take(&mut t.slide);
            let (sound, finished) = slide.update(now, |fully| t.wants_show(fully, multiplier));
            t.slide = slide;
            if finished {
                self.adv.toasts.remove(i);
                continue;
            }
            // Only the first visibility change of an update plays its sound.
            if let Some(show) = sound
                && !std::mem::replace(&mut sound_played, true)
            {
                let event = if show { "ui.toast.in" } else { "ui.toast.out" };
                self.audio
                    .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
            }
            i += 1;
        }
        // The queued toasts take the free slots in order; a challenge's own sound
        // plays once as it leaves the queue.
        let mut used = self.toast_slots_used();
        let mut challenge_sound = false;
        for t in &mut self.adv.toasts {
            if t.slot.is_some() {
                continue;
            }
            let Some(slot) = (0..Self::TOAST_SLOTS).find(|s| used & (1 << s) == 0) else {
                break;
            };
            used |= 1 << slot;
            t.slot = Some(slot);
            challenge_sound |= !t.is_recipe() && t.frame == Frame::Challenge;
        }
        if challenge_sound {
            self.audio
                .play("minecraft:ui.toast.challenge_complete", None, 1.0, 1.0);
        }
        if hidden {
            return;
        }
        for i in 0..self.adv.toasts.len() {
            if let Some(slot) = self.adv.toasts[i].slot {
                self.draw_toast(batch, i, 32.0 * slot as f32, multiplier);
            }
        }
    }
    /// The slots held by shown advancement, recipe and pack toasts, as bits.
    pub(super) fn toast_slots_used(&self) -> u32 {
        let packs = self.server_packs.toast_slots();
        self.adv
            .toasts
            .iter()
            .filter_map(|t| t.slot)
            .chain(packs)
            .fold(self.tutorial_toast_slots(), |used, slot| used | 1 << slot)
    }
    /// The `AdvancementToast` or `RecipeToast` at `index` in its slot at `y`.
    fn draw_toast(&mut self, batch: &mut HudBatch, index: usize, y: f32, multiplier: f64) {
        let t = &self.adv.toasts[index];
        let x = batch.width - 160.0 * t.slide.portion;
        let fully_for = t.slide.fully_for;
        let (title, frame, icon) = (t.title.clone(), t.frame, t.icon.clone());
        let recipe = t
            .is_recipe()
            .then(|| t.recipes[t.displayed_recipe(fully_for, multiplier)].clone());
        if let Some((station, item)) = recipe {
            // RecipeToast: black description under a purple title, the
            // station at 0.6 scale in the corner over the cycling result.
            let atlas = &self.gui.atlas;
            batch.sprite(atlas, "gui/sprites/toast/recipe", x, y);
            let title = self.translate("recipe.toast.title", "New Recipe(s) Unlocked!");
            let description = self.translate("recipe.toast.description", "Check your recipe book");
            let atlas = &self.gui.atlas;
            batch.text(atlas, &title, x + 30.0, y + 7.0, 0xFF50_0050, false);
            batch.text(atlas, &description, x + 30.0, y + 18.0, 0xFF00_0000, false);
            let start = batch.vertices.len();
            self.draw_item_model(batch, &ItemStack::new(&station, 1), x + 3.0, y + 3.0);
            batch.scale_since(start, x, y, 0.6, 0.6);
            self.draw_item_model(batch, &item, x + 8.0, y + 8.0);
            return;
        }
        let atlas = &self.gui.atlas;
        batch.sprite(atlas, "gui/sprites/toast/advancement", x, y);
        let (key, color) = match frame {
            Frame::Task => ("advancements.toast.task", 0xFF00_FFFF),
            Frame::Goal => ("advancements.toast.goal", 0xFF00_FFFF),
            Frame::Challenge => ("advancements.toast.challenge", 0xFFFF_88FF),
        };
        batch.text_styled(
            atlas,
            &Text::translate(key, &[]),
            x + 30.0,
            y + 7.0,
            color,
            false,
        );
        batch.text_styled(atlas, &title, x + 30.0, y + 18.0, u32::MAX, false);
        self.draw_item(batch, &ItemStack::new(&icon, 1), x + 8.0, y + 8.0);
    }
}
/// `Advancement.name`: the bracketed title in the frame's colour, with the
/// description on hover; the id when there is no display.
pub(super) fn adv_name(a: &adv::Advancement) -> Text {
    let Some(d) = &a.display else {
        return Text::literal(a.id.clone());
    };
    let color = frame_color(d.frame);
    let title = text_of(&d.title).with_style(&Style {
        hover: Some(Box::new(decorate_name(d))),
        ..Style::default()
    });
    Text::translate("chat.square_brackets", &[title]).with_color(color)
}
fn frame_color(frame: Frame) -> u32 {
    match frame {
        Frame::Challenge => text::DARK_PURPLE,
        _ => text::GREEN,
    }
}
/// `Advancement.decorateName`: a copy of the title with the frame's colour
/// under its own style (mergeStyles), then a new line and the description
/// after the title's own siblings, so both inherit the title's root style.
/// Built on the title's JSON (a list as its first element with the rest as
/// siblings): a parsed Text moves a styled root's style down a level once it
/// has siblings.
fn decorate_name(d: &adv::Display) -> Text {
    let mut component = match horizon_client::scoreboard::canonical_component(&d.title) {
        Value::String(s) => serde_json::json!({ "text": s }),
        c => c,
    };
    if let Some(o) = component.as_object_mut() {
        let frame = Text::empty().with_color(frame_color(d.frame)).component();
        o.entry("color").or_insert_with(|| frame["color"].clone());
        let extra = o.entry("extra").or_insert_with(|| serde_json::json!([]));
        if let Some(extra) = extra.as_array_mut() {
            extra.extend(["\n".into(), d.description.clone()]);
        }
    }
    text_of(&component).with_component(component)
}
impl Game<'_> {
    /// `AdvancementCommands.Action.GRANT` on one advancement (or one of
    /// its criteria): false when nothing changed.
    pub(super) fn grant_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        if criterion.is_none() && self.adv.progress.done(a) {
            return false;
        }
        let names: Vec<&str> = match criterion {
            Some(c) => vec![c],
            None => a.criteria.iter().map(|c| c.name.as_str()).collect(),
        };
        let date = now_text();
        let mut changed = false;
        for c in names {
            let was = self.adv.progress.done(a);
            if self.adv.progress.grant(&a.id, c, &date) {
                changed = true;
                self.progress_changed(index);
                if !was && self.adv.progress.done(a) {
                    self.award(index);
                }
            }
        }
        changed || criterion.is_none()
    }
    /// `AdvancementCommands.Action.REVOKE`.
    pub(super) fn revoke_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        let tree = self.adv.tree.clone();
        let a = &tree.list[index];
        self.progress_changed(index);
        match criterion {
            Some(c) => self.adv.progress.revoke(&a.id, c),
            None => {
                let mut any = false;
                for c in &a.criteria {
                    any |= self.adv.progress.revoke(&a.id, &c.name);
                }
                any
            }
        }
    }
}
impl Game<'_> {
    /// `ClientAdvancements.update` (ClientboundUpdateAdvancementsPacket):
    /// the server's advancements replace or join the tree, each changed one
    /// takes the progress sent, and one just done shows its toast. `icons`
    /// are the added advancements' icon items by name.
    pub(super) fn remote_advancements(
        &mut self,
        p: horizon_protocol::packets::play::UpdateAdvancements,
        icons: Vec<String>,
    ) {
        use horizon_client::scoreboard::tag_to_json;
        let mut list: Vec<adv::Advancement> = Vec::new();
        let mut positions: Vec<(f32, f32)> = Vec::new();
        if !p.reset {
            let old = self.adv.positions.take().unwrap_or_default();
            for (i, a) in self.adv.tree.list.iter().enumerate() {
                if !p.removed.contains(&a.id) {
                    list.push(a.clone());
                    positions.push(old.get(i).copied().unwrap_or((0.0, 0.0)));
                }
            }
        } else {
            self.adv.progress = Progress::default();
        }
        for id in &p.removed {
            self.adv.progress.criteria.remove(id);
        }
        for (a, icon) in p.added.into_iter().zip(icons) {
            list.retain(|o| o.id != a.id);
            let display = a.display.map(|d| adv::Display {
                icon,
                title: tag_to_json(&d.title),
                description: tag_to_json(&d.description),
                frame: match d.frame {
                    1 => Frame::Challenge,
                    2 => Frame::Goal,
                    _ => Frame::Task,
                },
                background: d.background,
                show_toast: d.show_toast,
                announce: false,
                hidden: d.hidden,
            });
            let mut criteria: Vec<adv::Criterion> = Vec::new();
            for name in a.requirements.iter().flatten() {
                if !criteria.iter().any(|c| &c.name == name) {
                    criteria.push(adv::Criterion {
                        name: name.clone(),
                        trigger: String::new(),
                        conditions: Value::Null,
                    });
                }
            }
            list.push(adv::Advancement {
                id: a.id,
                parent: a.parent,
                display,
                criteria,
                requirements: a.requirements,
                rewards: Default::default(),
            });
            positions.push((a.x, a.y));
        }
        let mut tree = Tree::default();
        tree.list = list;
        for (i, a) in tree.list.iter().enumerate() {
            tree.by_id.insert(a.id.clone(), i);
        }
        self.ui.registries.advancements = tree.list.iter().map(|a| a.id.clone()).collect();
        self.adv.tree = Arc::new(tree);
        self.adv.positions = Some(positions);
        let tree = self.adv.tree.clone();
        for (id, criteria) in p.progress {
            let Some(&i) = tree.by_id.get(&id) else {
                continue;
            };
            // The client only shows whether a criterion is obtained, not when.
            let obtained: BTreeMap<String, String> = criteria
                .into_iter()
                .filter(|(_, at)| at.is_some())
                .map(|(name, _)| (name, String::new()))
                .collect();
            self.adv.progress.criteria.insert(id, obtained);
            let a = &tree.list[i];
            if !p.reset
                && p.show
                && self.adv.progress.done(a)
                && let Some(d) = a.display.as_ref().filter(|d| d.show_toast)
            {
                self.adv.toasts.push_back(Toast {
                    icon: d.icon.clone(),
                    frame: d.frame,
                    title: text_of(&d.title),
                    slide: Slide::default(),
                    slot: None,
                    recipes: Vec::new(),
                    changed: false,
                    last_changed: 0,
                });
            }
        }
    }
}
/// An advancement's title or description: a whole component (ComponentSerialization.CODEC),
/// with its siblings and styles, as a plugin's `{"text":"","extra":[...]}` needs.
pub(super) fn text_of(v: &Value) -> Text {
    horizon_client::scoreboard::comp_text(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipes(n: usize) -> Toast {
        Toast::recipe(
            (0..n)
                .map(|i| {
                    let item = ItemStack::new(&format!("minecraft:item_{i}"), 1);
                    ("minecraft:crafting_table".to_owned(), item)
                })
                .collect(),
        )
    }
    /// Drives a toast frame by frame (every `step` ms) and returns the slide
    /// portions at the given times and when it finished.
    fn run(toast: &mut Toast, step: i64, until: i64, at: &[i64]) -> (Vec<f32>, Option<i64>) {
        let mut slide = Slide::default();
        let mut seen = Vec::new();
        for now in (0..=until).step_by(step as usize) {
            let (_, finished) = slide.update(now, |f| toast.wants_show(f, 1.0));
            if finished {
                return (seen, Some(now));
            }
            if at.contains(&now) {
                seen.push(slide.portion);
            }
        }
        (seen, None)
    }

    #[test]
    fn titles_keep_their_siblings_and_styles() {
        let title = text_of(&serde_json::json!({
            "text": "",
            "extra": [{"text": "Quest ", "color": "gold"}, "Board"],
        }));
        assert_eq!(title.plain(), "Quest Board");
        assert_eq!(title.spans[0].style.color, Some(text::GOLD));
        assert_eq!(title.spans[1].style.color, None);
        let vanilla = text_of(&serde_json::json!({"translate": "advancements.story.root.title"}));
        assert_eq!(
            vanilla,
            Text::translate("advancements.story.root.title", &[])
        );
        assert_eq!(
            vanilla.translation_key(),
            Some("advancements.story.root.title")
        );
    }

    fn announced(title: Value, description: Value) -> adv::Advancement {
        adv::Advancement {
            id: "quests:board".into(),
            parent: None,
            display: Some(adv::Display {
                icon: "minecraft:book".into(),
                title,
                description,
                frame: Frame::Task,
                background: None,
                show_toast: true,
                announce: true,
                hidden: false,
            }),
            criteria: Vec::new(),
            requirements: Vec::new(),
            rewards: adv::Rewards::default(),
        }
    }
    /// The chat hover: the JSON a client receives, and the spans drawn here.
    fn hover(a: &adv::Advancement) -> (Value, Text) {
        let local = decorate_name(a.display.as_ref().unwrap());
        let sent = adv_name(a).component()["with"][0]["hover_event"]["value"].clone();
        (sent, local)
    }

    #[test]
    fn chat_hover_keeps_the_titles_siblings_before_the_description() {
        let a = announced(
            serde_json::json!({"text": "", "extra": [{"text": "Quest ", "color": "gold"}, "Board"]}),
            serde_json::json!("Open the board"),
        );
        let (sent, local) = hover(&a);
        let sent = horizon_client::scoreboard::comp_text(&sent);
        assert_eq!(sent.plain(), "Quest Board\nOpen the board");
        let colors = |t: &Text| t.spans.iter().map(|s| s.style.color).collect::<Vec<_>>();
        let (gold, green) = (Some(text::GOLD), Some(text::GREEN));
        assert_eq!(colors(&sent), [gold, green, green, green]);
        // The spans drawn here come from the same JSON a client receives.
        assert_eq!(colors(&local), colors(&sent));
    }

    #[test]
    fn chat_hover_description_takes_a_styled_titles_own_style() {
        // mergeStyles keeps the title's gold; the description inherits it.
        let object = announced(
            serde_json::json!({"text": "Quest", "color": "gold", "bold": true, "extra": [" Board"]}),
            serde_json::json!("Open the board"),
        );
        let list = announced(
            serde_json::json!([{"text": "Quest", "color": "gold", "bold": true}, " Board"]),
            serde_json::json!("Open the board"),
        );
        for a in [object, list] {
            let (sent, local) = hover(&a);
            let sent = horizon_client::scoreboard::comp_text(&sent);
            assert_eq!(sent.plain(), "Quest Board\nOpen the board");
            let last = &sent.spans.last().unwrap().style;
            assert_eq!((last.color, last.bold), (Some(text::GOLD), Some(true)));
            assert_eq!(local.spans.last().unwrap().style, *last);
        }
    }

    #[test]
    fn chat_hover_of_a_vanilla_advancement() {
        let a = announced(
            serde_json::json!({"translate": "advancements.story.root.title"}),
            serde_json::json!({"translate": "advancements.story.root.description"}),
        );
        assert_eq!(
            hover(&a).0,
            serde_json::json!({
                "translate": "advancements.story.root.title",
                "color": "green",
                "extra": ["\n", {"translate": "advancements.story.root.description"}],
            })
        );
    }

    #[test]
    fn toasts_slide_in_squared_and_out_after_five_seconds() {
        let mut toast = recipes(1);
        let (portions, finished) = run(&mut toast, 10, 10_000, &[0, 300, 600, 3000]);
        // (300 / 600)² = 0.25: ToastInstance.calculateVisiblePortion.
        assert_eq!(portions, [0.0, 0.25, 1.0, 1.0]);
        // Fully in at 600, shown 5000 ms, out over 600 more.
        assert!(
            finished.is_some_and(|t| (6200..=6220).contains(&t)),
            "{finished:?}"
        );
    }

    #[test]
    fn the_first_update_plays_the_show_sound_and_the_hide_sound_follows() {
        let mut toast = recipes(1);
        let mut slide = Slide::default();
        assert_eq!(slide.update(0, |f| toast.wants_show(f, 1.0)).0, Some(true));
        assert_eq!(slide.update(100, |f| toast.wants_show(f, 1.0)).0, None);
        assert_eq!(
            slide.update(5600, |f| toast.wants_show(f, 1.0)).0,
            Some(false)
        );
    }

    #[test]
    fn recipe_toasts_cycle_through_their_displays_over_the_display_time() {
        let toast = recipes(4);
        let shown: Vec<usize> = [0, 1249, 1250, 2500, 3750, 4999, 5000]
            .iter()
            .map(|&t| toast.displayed_recipe(t, 1.0))
            .collect();
        assert_eq!(shown, [0, 0, 1, 2, 3, 3, 0]);
        // notificationDisplayTime doubles each turn.
        assert_eq!(toast.displayed_recipe(2500, 2.0), 1);
    }

    #[test]
    fn unlocking_more_recipes_keeps_the_toast_up_five_more_seconds() {
        let mut toast = recipes(1);
        let mut slide = Slide::default();
        let mut finished = None;
        for now in (0..=20_000).step_by(10) {
            if now == 4000 {
                toast.add_recipes(recipes(1).recipes);
            }
            if slide.update(now, |f| toast.wants_show(f, 1.0)).1 {
                finished = Some(now);
                break;
            }
        }
        assert_eq!(toast.recipes.len(), 2);
        // lastChanged moved to 3400 ms fully shown: hidden at 8400 + 600.
        assert!(
            finished.is_some_and(|t| (9600..=9620).contains(&t)),
            "{finished:?}"
        );
    }
}
