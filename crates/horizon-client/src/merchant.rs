//! Villager trading: the data-driven `trade_set` / `villager_trade` registries,
//! offer generation (`AbstractVillager.addOffersFromTradeSet` with each
//! set's `RandomSequence`), `MerchantOffer` pricing, demand and stock, and the
//! `VillagerData` level thresholds. The villager entity itself is not part of
//! this module; anything that can trade calls [`Merchant`].
use crate::{
    enchant::Enchantments,
    items::{ItemStack, Items},
};
use horizon_core::random::{RandomSource, XoroshiroRandom, mix_stafford13, seed_from_hash};
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

/// `VillagerData.NEXT_LEVEL_XP_THRESHOLDS`.
pub const LEVEL_XP: [i32; 5] = [0, 10, 70, 150, 250];
pub fn can_level_up(level: i32) -> bool {
    (1..5).contains(&level)
}
pub fn min_xp(level: i32) -> i32 {
    if can_level_up(level) {
        LEVEL_XP[level as usize - 1]
    } else {
        0
    }
}
pub fn max_xp(level: i32) -> i32 {
    if can_level_up(level) {
        LEVEL_XP[level as usize]
    } else {
        0
    }
}
/// The professions with trades, as in `VillagerProfession`.
pub const PROFESSIONS: &[&str] = &[
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];

/// A villager's or wandering trader's trading state (`AbstractVillager.offers`
/// and `Villager.villagerXp`); `None` on the mob until its offers are first asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trader {
    pub xp: i32,
    pub offers: Vec<MerchantOffer>,
    /// `lastRestockGameTime`, `numberOfRestocksToday` and `lastRestockCheckDay`.
    pub last_restock: i64,
    pub restocks_today: i32,
    pub check_day: i64,
    /// The offers are not made yet (vanilla's `offers` is null): they are
    /// generated when first asked for, keeping the experience.
    pub unset: bool,
}
impl Trader {
    fn update_demand(&mut self) {
        for o in &mut self.offers {
            o.update_demand();
        }
    }
    /// `Villager.shouldRestock`.
    pub fn should_restock(&mut self, game_time: i64, day: i64) -> bool {
        let mut new_day = game_time > self.last_restock + 12000;
        new_day |= self.check_day > 0 && day > self.check_day;
        self.check_day = day;
        if new_day {
            self.last_restock = game_time;
            // resetNumberOfRestocks / catchUpDemand.
            let missed = 2 - self.restocks_today;
            if missed > 0 {
                for o in &mut self.offers {
                    o.uses = 0;
                }
            }
            for _ in 0..missed {
                self.update_demand();
            }
            self.restocks_today = 0;
        }
        let allowed = self.restocks_today == 0
            || self.restocks_today < 2 && game_time > self.last_restock + 2400;
        allowed && self.offers.iter().any(|o| o.uses > 0)
    }
    /// `Villager.restock`.
    pub fn restock(&mut self, game_time: i64) {
        self.update_demand();
        for o in &mut self.offers {
            o.uses = 0;
        }
        self.last_restock = game_time;
        self.restocks_today += 1;
    }
}

/// `Villager.updateSpecialPrices`: the reputation and Hero of the Village
/// discounts of a player, from scratch.
pub fn update_special_prices(offers: &mut [MerchantOffer], reputation: i32, hero: Option<i32>) {
    // 0.3F + 0.0625F * amplifier, widened from float.
    let hero_modifier = hero.map_or(0.0, |a| f64::from(0.3f32 + 0.0625f32 * a as f32));
    for offer in offers {
        offer.special_price_diff = 0;
        if reputation != 0 {
            offer.special_price_diff -= (reputation as f32 * offer.price_multiplier).floor() as i32;
        }
        if hero_modifier > 0.0 {
            let reduction =
                (hero_modifier * f64::from(offer.base_cost_a.stack.count)).floor() as i32;
            offer.special_price_diff -= reduction.max(1);
        }
    }
}
/// An `ItemCost`: item and count (component predicates are not used by the
/// 26.3 trade data except for one exact match, kept as the stack's data).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCost {
    pub stack: ItemStack,
}
impl ItemCost {
    pub fn test(&self, s: &ItemStack) -> bool {
        s.item == self.stack.item && (self.stack.extra.is_none() || s.data() == self.stack.data())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantOffer {
    pub base_cost_a: ItemCost,
    pub cost_b: Option<ItemCost>,
    pub result: ItemStack,
    pub uses: i32,
    pub max_uses: i32,
    pub reward_exp: bool,
    pub special_price_diff: i32,
    pub demand: i32,
    pub price_multiplier: f32,
    pub xp: i32,
}
impl MerchantOffer {
    fn max_stack(items: &Items, s: &ItemStack) -> u32 {
        items.get(&s.item).map_or(64, |i| i.max_stack)
    }
    /// `getModifiedCostCount` for cost A.
    pub fn cost_a_count(&self, items: &Items) -> u32 {
        let base = self.base_cost_a.stack.count as i32;
        // Mth.floor(basePrice * demand * priceMultiplier): an int product, then the float.
        let demand_diff =
            ((base.wrapping_mul(self.demand) as f32 * self.price_multiplier).floor() as i32).max(0);
        (base + demand_diff + self.special_price_diff)
            .clamp(1, Self::max_stack(items, &self.base_cost_a.stack) as i32) as u32
    }
    pub fn cost_a(&self, items: &Items) -> ItemStack {
        ItemStack {
            count: self.cost_a_count(items),
            ..self.base_cost_a.stack.clone()
        }
    }
    pub fn cost_b(&self) -> Option<ItemStack> {
        self.cost_b.as_ref().map(|c| c.stack.clone())
    }
    pub fn out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }
    /// `MerchantOffer.updateDemand` (on restock).
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }
    /// `satisfiedBy(buyA, buyB)`.
    pub fn satisfied_by(
        &self,
        items: &Items,
        a: Option<&ItemStack>,
        b: Option<&ItemStack>,
    ) -> bool {
        let Some(a) = a else { return false };
        if !self.base_cost_a.test(a) || a.count < self.cost_a_count(items) {
            return false;
        }
        match (&self.cost_b, b) {
            (None, b) => b.is_none(),
            (Some(c), Some(b)) => c.test(b) && b.count >= c.stack.count,
            (Some(_), None) => false,
        }
    }
    /// `MerchantOffer.take`: consume the payment; false when not satisfied.
    pub fn take(
        &self,
        items: &Items,
        a: &mut Option<ItemStack>,
        b: &mut Option<ItemStack>,
    ) -> bool {
        if !self.satisfied_by(items, a.as_ref(), b.as_ref()) {
            return false;
        }
        let n = self.cost_a_count(items);
        if let Some(s) = a.as_mut() {
            s.count -= n;
            if s.count == 0 {
                *a = None;
            }
        }
        if let (Some(c), Some(s)) = (&self.cost_b, b.as_mut()) {
            s.count -= c.stack.count;
            if s.count == 0 {
                *b = None;
            }
        }
        true
    }
}
/// `MerchantOffers.getRecipeFor`: a hint past the first offer names the only
/// offer that may match; otherwise the first offer that fits.
pub fn recipe_for(
    offers: &[MerchantOffer],
    items: &Items,
    a: Option<&ItemStack>,
    b: Option<&ItemStack>,
    hint: Option<usize>,
) -> Option<usize> {
    if let Some(h) = hint
        && h > 0
        && h < offers.len()
    {
        return offers[h].satisfied_by(items, a, b).then_some(h);
    }
    offers.iter().position(|o| o.satisfied_by(items, a, b))
}
/// `MerchantContainer.updateSellItem`: (active offer, result).
pub fn sell_item(
    offers: &[MerchantOffer],
    items: &Items,
    a: Option<&ItemStack>,
    b: Option<&ItemStack>,
    hint: Option<usize>,
) -> (Option<usize>, Option<ItemStack>) {
    let (a, b) = if a.is_none() { (b, None) } else { (a, b) };
    if a.is_none() {
        return (None, None);
    }
    let mut found = recipe_for(offers, items, a, b, hint);
    let mut active = found;
    if found.is_none_or(|i| offers[i].out_of_stock()) {
        active = found;
        found = recipe_for(offers, items, b, a, hint);
    }
    match found {
        Some(i) if !offers[i].out_of_stock() => (Some(i), Some(offers[i].result.clone())),
        _ => (active, None),
    }
}

/// `RandomSequence(worldSeed ^ salt, id)` with the default salt 0.
pub fn sequence_random(world_seed: i64, id: &str) -> XoroshiroRandom {
    const SILVER: u64 = 0x6a09_e667_f3bc_c909;
    const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;
    let lo = world_seed as u64 ^ SILVER;
    let hi = lo.wrapping_add(GOLDEN);
    let key = seed_from_hash(id);
    XoroshiroRandom::from_state(mix_stafford13(lo ^ key[0]), mix_stafford13(hi ^ key[1]))
}

/// What a trade screen's session changes besides the menu, for the caller.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An experience orb of this value at the merchant (`rewardTradeXp`).
    Orb(i32),
    /// The villager reached this level (regeneration for 200 ticks).
    LevelUp(i32),
    /// `notifyTradeUpdated`: the merchant says yes (a result) or no, at
    /// this pitch.
    TradeUpdated(bool, f32),
    /// `MerchantMenu.playTradeSound` (a shift-clicked trade).
    TradeSound,
    /// A trade of this offer was made (`notifyTrade`).
    Traded(usize),
}
/// What the world lends a trading session for offers made while it is open.
pub struct TradeEnv<'a> {
    pub trades: &'a Trades,
    pub enchantments: &'a Enchantments,
    pub maps: Option<&'a dyn MapMaker>,
}
/// The merchant side of an open trading screen: `AbstractVillager` /
/// `Villager` / `WanderingTrader` as `MerchantMenu` sees it (offers,
/// experience, level-ups, special prices for the trading player) and the
/// `MerchantContainer` state (selection hint, active offer, future xp).
#[derive(Clone, Debug)]
pub struct Session {
    pub offers: Vec<MerchantOffer>,
    pub xp: i32,
    pub level: i32,
    /// A villager (progress bar, restocks, level-ups), not a wandering trader.
    pub villager: bool,
    /// `minecraft:` ids of the profession and villager type.
    pub profession: String,
    pub variant: String,
    /// The merchant's own random (`rewardTradeXp`).
    pub random: horizon_core::random::LegacyRandom,
    pub origin: [f64; 3],
    /// The trading player's reputation and Hero of the Village amplifier.
    pub reputation: i32,
    pub hero: Option<i32>,
    /// Trade set sequences the session may draw from (the next levels'), lent
    /// by the world's `RandomSequences` and given back when it closes.
    pub sequences: std::collections::BTreeMap<String, XoroshiroRandom>,
    pub selection_hint: usize,
    pub active: Option<usize>,
    pub future_xp: i32,
    /// `Mob.ambientSoundTime`.
    pub ambient_sound_time: i32,
    /// `Villager.lastTradedPlayer` is set: the TRADE reputation event is due.
    pub last_traded: bool,
    pub events: Vec<Event>,
}
impl Session {
    pub fn new(offers: Vec<MerchantOffer>, villager: bool) -> Self {
        Self {
            offers,
            xp: 0,
            level: 1,
            villager,
            profession: String::new(),
            variant: "minecraft:plains".into(),
            random: horizon_core::random::LegacyRandom::new(0),
            origin: [0.0; 3],
            reputation: 0,
            hero: None,
            sequences: Default::default(),
            selection_hint: 0,
            active: None,
            future_xp: 0,
            ambient_sound_time: 0,
            last_traded: false,
            events: Vec::new(),
        }
    }
    /// `Merchant.showProgressBar` / `canRestock`.
    pub fn show_progress(&self) -> bool {
        self.villager
    }
    /// The trade sets whose sequences a villager of `profession` at `level`
    /// may draw while trading: those of the levels above it.
    pub fn sequences_needed(trades: &Trades, profession: &str, level: i32) -> Vec<String> {
        (level + 1..=5)
            .filter_map(|l| trades.sequence(&Trades::set_for(profession, l)))
            .collect()
    }
    /// `Villager.updateSpecialPrices` for the trading player.
    pub fn update_special_prices(&mut self) {
        if self.villager {
            update_special_prices(&mut self.offers, self.reputation, self.hero);
        }
    }
    /// `AbstractVillager.notifyTrade`: the offer's use, then `rewardTradeXp`
    /// (and a level-up with its offers). True when the special prices were
    /// recomputed (the menu then updates its result).
    pub fn notify_trade(
        &mut self,
        index: usize,
        env: Option<&TradeEnv<'_>>,
        items: &Items,
    ) -> bool {
        let Some(offer) = self.offers.get_mut(index) else {
            return false;
        };
        offer.uses += 1;
        let offer = offer.clone();
        self.ambient_sound_time = -80;
        self.events.push(Event::Traded(index));
        let mut prices = false;
        if self.villager {
            let mut pop = 3 + self.random.next_int_bound(4).unwrap_or(0);
            self.xp += offer.xp;
            self.last_traded = true;
            if can_level_up(self.level) && self.xp >= max_xp(self.level) {
                self.level += 1;
                if let Some(env) = env {
                    let set = Trades::set_for(&self.profession, self.level);
                    if env.trades.sets.contains_key(&set) {
                        let more = self.offers_from_set(env, items, &set);
                        self.offers.extend(more);
                        self.update_special_prices();
                        prices = true;
                    }
                }
                self.events.push(Event::LevelUp(self.level));
                pop += 5;
            }
            if offer.reward_exp {
                self.events.push(Event::Orb(pop));
            }
        } else if offer.reward_exp {
            let pop = 3 + self.random.next_int_bound(4).unwrap_or(0);
            self.events.push(Event::Orb(pop));
        }
        prices
    }
    fn offers_from_set(
        &mut self,
        env: &TradeEnv<'_>,
        items: &Items,
        set: &str,
    ) -> Vec<MerchantOffer> {
        let Some(sequence) = env.trades.sequence(set) else {
            return Vec::new();
        };
        let Some(mut random) = self.sequences.remove(&sequence) else {
            return Vec::new();
        };
        let ctx = TradeContext {
            items,
            enchantments: env.enchantments,
            variant: &self.variant,
            origin: self.origin,
            maps: env.maps,
        };
        let out = env.trades.offers_from_set(&ctx, set, &mut random);
        self.sequences.insert(sequence, random);
        out
    }
    /// `AbstractVillager.notifyTradeUpdated`.
    pub fn notify_trade_updated(&mut self, has_result: bool) {
        if self.ambient_sound_time > -80 + 20 {
            self.ambient_sound_time = -80;
            let pitch = self.voice_pitch();
            self.events.push(Event::TradeUpdated(has_result, pitch));
        }
    }
    /// `LivingEntity.getVoicePitch` of an adult merchant.
    pub fn voice_pitch(&mut self) -> f32 {
        (self.random.next_float() - self.random.next_float()) * 0.2 + 1.0
    }
}

/// The trade registries.
pub struct Trades {
    pub trades: HashMap<String, Value>,
    /// villager_trade tags, ordered.
    pub tags: HashMap<String, Vec<String>>,
    pub sets: HashMap<String, Value>,
    /// potion tags, ordered (`set_random_potion` options).
    pub potion_tags: HashMap<String, Vec<String>>,
}
fn walk(dir: &Path, base: &Path, out: &mut HashMap<String, Value>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "json")
            && let Ok(rel) = p.strip_prefix(base)
            && let Ok(bytes) = fs::read(&p)
            && let Ok(v) = serde_json::from_slice(&bytes)
        {
            out.insert(
                format!(
                    "minecraft:{}",
                    rel.with_extension("").to_string_lossy().replace('\\', "/")
                ),
                v,
            );
        }
    }
}
impl Trades {
    /// Load from a resource root; `overlay` (the world's data packs'
    /// content files) merges over it, the way a pack stack layers
    /// (`SimpleJsonResourceReloadListener`, `TagLoader`).
    pub fn load(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> Self {
        let data = root.join("data/minecraft");
        let (mut trades, mut raw_tags, mut sets) = (HashMap::new(), HashMap::new(), HashMap::new());
        walk(
            &data.join("villager_trade"),
            &data.join("villager_trade"),
            &mut trades,
        );
        walk(
            &data.join("tags/villager_trade"),
            &data.join("tags/villager_trade"),
            &mut raw_tags,
        );
        walk(&data.join("trade_set"), &data.join("trade_set"), &mut sets);
        let mut raw_potions = HashMap::new();
        walk(
            &data.join("tags/potion"),
            &data.join("tags/potion"),
            &mut raw_potions,
        );
        // The packs' trade files: a trade or set's top pack file replaces
        // the game's own (`SimpleJsonResourceReloadListener` sees one
        // resource per id), a tag's entries join in pack order with a
        // `replace` file dropping the lower sources' (`TagLoader.load`).
        for (id, files) in overlay.kind("villager_trade") {
            if let Ok(v) = serde_json::from_slice(PackOverlay::top(files)) {
                trades.insert(id.to_owned(), v);
            }
        }
        for (id, files) in overlay.kind("trade_set") {
            if let Ok(v) = serde_json::from_slice(PackOverlay::top(files)) {
                sets.insert(id.to_owned(), v);
            }
        }
        let values = |v: Value| -> Vec<String> {
            v["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str().map(str::to_owned))
                .collect()
        };
        let mut raw_potions: HashMap<String, Vec<String>> = raw_potions
            .into_iter()
            .map(|(k, v)| (k, values(v)))
            .collect();
        let mut raw: HashMap<String, Vec<String>> = raw_tags
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v["values"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.as_str().map(str::to_owned))
                        .collect(),
                )
            })
            .collect();
        for (registry, raw) in [("villager_trade", &mut raw), ("potion", &mut raw_potions)] {
            for (id, files) in overlay.tag_kind(registry) {
                let slot = raw.entry(id).or_default();
                for bytes in files {
                    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                        continue;
                    };
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    if let Some(vs) = v["values"].as_array() {
                        slot.extend(vs.iter().filter_map(|e| e.as_str().map(str::to_owned)));
                    }
                }
            }
        }
        fn expand(
            name: &str,
            raw: &HashMap<String, Vec<String>>,
            out: &mut Vec<String>,
            depth: u32,
        ) {
            for v in raw.get(name).into_iter().flatten() {
                match v.strip_prefix('#') {
                    Some(t) if depth < 16 => expand(t, raw, out, depth + 1),
                    Some(_) => {}
                    None if !out.contains(v) => out.push(v.clone()),
                    None => {}
                }
            }
        }
        let expand_all = |raw: &HashMap<String, Vec<String>>| {
            raw.keys()
                .map(|k| {
                    let mut v = Vec::new();
                    expand(k, raw, &mut v, 0);
                    (k.clone(), v)
                })
                .collect()
        };
        Self {
            trades,
            tags: expand_all(&raw),
            sets,
            potion_tags: expand_all(&raw_potions),
        }
    }
    /// The random sequence of a trade set (`TradeSet.randomSequence`).
    pub fn sequence(&self, set_id: &str) -> Option<String> {
        self.sets
            .get(set_id)?
            .get("random_sequence")?
            .as_str()
            .map(str::to_owned)
    }
    /// Every trade set's random sequence, sorted.
    pub fn sequences(&self) -> Vec<String> {
        let mut v: Vec<String> = self.sets.keys().filter_map(|k| self.sequence(k)).collect();
        v.sort();
        v
    }
    /// `VillagerProfession.getTrades(level)`.
    pub fn set_for(profession: &str, level: i32) -> String {
        format!(
            "minecraft:{}/level_{level}",
            profession.trim_start_matches("minecraft:")
        )
    }
    fn trade_list(&self, spec: &Value) -> Vec<String> {
        match spec {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => vec![],
        }
    }
    /// `AbstractVillager.addOffersFromTradeSet`.
    pub fn offers_from_set<R: RandomSource>(
        &self,
        ctx: &TradeContext<'_>,
        set_id: &str,
        random: &mut R,
    ) -> Vec<MerchantOffer> {
        let Some(set) = self.sets.get(set_id) else {
            return vec![];
        };
        let mut potential = self.trade_list(&set["trades"]);
        let amount = int_provider(&set["amount"], random);
        let duplicates = set["allow_duplicates"].as_bool().unwrap_or(false);
        let mut out = Vec::new();
        while (out.len() as i32) < amount && !potential.is_empty() {
            let roll = random.next_int_bound(potential.len() as i32).unwrap_or(0) as usize;
            let id = if duplicates {
                potential[roll].clone()
            } else {
                potential.remove(roll)
            };
            match self
                .trades
                .get(&id)
                .and_then(|t| self.offer_from(ctx, t, random))
            {
                Some(o) => out.push(o),
                None if duplicates => {
                    potential.remove(roll);
                }
                None => {}
            }
        }
        out
    }
}
/// What a villager trade's loot context provides.
pub struct TradeContext<'a> {
    pub items: &'a Items,
    pub enchantments: &'a Enchantments,
    /// Villager type (`minecraft:plains`, ...), for `merchant_predicate`.
    pub variant: &'a str,
    /// The merchant's position (the loot context's origin).
    pub origin: [f64; 3],
    /// Finds structures for exploration maps; None finds nothing.
    pub maps: Option<&'a dyn MapMaker>,
}
/// `ExplorationMapFunction`: `ServerLevel.findNearestMapStructure` and the
/// map it makes, for the world the merchant is in.
pub trait MapMaker {
    /// The map for the nearest structure in `destination` (a structure tag)
    /// within `radius` chunks of `origin`, or the stack unchanged when none is
    /// found.
    fn exploration_map(
        &self,
        stack: ItemStack,
        map: &ExplorationMap,
        origin: [f64; 3],
    ) -> ItemStack;
}
/// An `exploration_map` function's settings.
pub use horizon_world::loot::ExplorationMap;
fn int_provider<R: RandomSource>(v: &Value, random: &mut R) -> i32 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:uniform" => {
                let (min, max) = (
                    o["min"].as_i64().unwrap_or(0) as i32,
                    o["max"].as_i64().unwrap_or(0) as i32,
                );
                if min >= max {
                    min
                } else {
                    min + random.next_int_bound(max - min + 1).unwrap_or(0)
                }
            }
            "minecraft:constant" => o["value"].as_i64().unwrap_or(0) as i32,
            // Sum: inputs in order, summed in a long and clamped.
            "minecraft:add" => {
                let mut total = 0i64;
                for input in o["inputs"].as_array().into_iter().flatten() {
                    total += i64::from(int_provider(input, random));
                }
                total.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
            }
            "minecraft:binomial" => {
                let n = int_provider(&o["n"], random);
                let p = float_provider(&o["p"], random);
                (0..n).filter(|_| random.next_float() < p).count() as i32
            }
            _ => o.get("value").and_then(Value::as_i64).unwrap_or(0) as i32,
        },
        _ => 0,
    }
}
fn float_provider<R: RandomSource>(v: &Value, random: &mut R) -> f32 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
        Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:uniform" => {
                let min = float_provider(&o["min"], random);
                let max = float_provider(&o["max"], random);
                // Mth.randomBetween
                random.next_float() * (max - min) + min
            }
            _ => o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        },
        _ => 0.0,
    }
}
/// `MobEffect.isInstantaneous`.
fn instantaneous(effect: &str) -> bool {
    matches!(
        effect,
        "minecraft:saturation" | "minecraft:instant_health" | "minecraft:instant_damage"
    )
}
fn cost(items: &Items, v: &Value, additional: i32) -> ItemCost {
    let id = v["id"].as_str().unwrap_or("minecraft:air");
    let max = items.get(id).map_or(64, |i| i.max_stack) as i32;
    let count = (v["count"].as_i64().unwrap_or(1) as i32 + additional).clamp(0, max);
    let mut stack = ItemStack::new(id, count as u32);
    if let Some(potion) = v["components"]["minecraft:potion_contents"]["potion"].as_str() {
        stack = stack.with_potion(potion);
    }
    ItemCost { stack }
}
fn variant_ok(pred: &Value, variant: &str) -> bool {
    match &pred["predicate"]["minecraft:predicates"]["minecraft:villager/variant"] {
        Value::String(s) => s == variant,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(variant)),
        _ => true,
    }
}
impl Trades {
    /// `VillagerTrade.getOffer`.
    pub fn offer_from<R: RandomSource>(
        &self,
        ctx: &TradeContext<'_>,
        t: &Value,
        random: &mut R,
    ) -> Option<MerchantOffer> {
        if let Some(p) = t.get("merchant_predicate")
            && !variant_ok(p, ctx.variant)
        {
            return None;
        }
        let gives = &t["gives"];
        let mut result = ItemStack::new(
            gives["id"].as_str()?,
            gives["count"].as_u64().unwrap_or(1) as u32,
        );
        let mut additional = 0;
        let modifiers: Vec<&Value> = match &t["given_item_modifier"] {
            Value::Array(a) => a.iter().collect(),
            v @ Value::Object(_) => vec![v],
            _ => vec![],
        };
        for m in modifiers {
            let (stack, extra) = self.apply_modifier(ctx, m, result, random)?;
            result = stack;
            additional += extra;
        }
        if let Some(Value::String(tag)) = t.get("double_trade_price_enchantments")
            && result
                .stored_enchantments()
                .iter()
                .any(|(e, _)| ctx.enchantments.in_tag(tag, e))
        {
            additional *= 2;
        }
        let a = cost(ctx.items, &t["wants"], additional);
        if a.stack.count < 1 {
            return None;
        }
        let b = t.get("additional_wants").map(|w| cost(ctx.items, w, 0));
        if b.as_ref().is_some_and(|b| b.stack.count < 1) {
            return None;
        }
        let max_uses = match t.get("max_uses") {
            Some(v) => int_provider(v, random),
            None => 4,
        };
        let xp = match t.get("xp") {
            Some(v) => int_provider(v, random),
            None => 1,
        };
        let discount = match t.get("reputation_discount") {
            Some(v) => float_provider(v, random),
            None => 0.0,
        };
        Some(MerchantOffer {
            base_cost_a: a,
            cost_b: b,
            result,
            uses: 0,
            max_uses: max_uses.max(1),
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier: discount.max(0.0),
            xp: xp.max(0),
        })
    }
    /// The loot functions trades use; None discards the offer.
    fn apply_modifier<R: RandomSource>(
        &self,
        ctx: &TradeContext<'_>,
        m: &Value,
        stack: ItemStack,
        random: &mut R,
    ) -> Option<(ItemStack, i32)> {
        let e = ctx.enchantments;
        let pool = |options: &Value| -> Vec<Arc<str>> {
            match options {
                Value::String(s) if s.starts_with('#') => e.tag(s).to_vec(),
                Value::String(s) => vec![Arc::from(s.as_str())],
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(Arc::from)).collect(),
                _ => e.list.iter().map(|x| x.id.clone()).collect(),
            }
        };
        match m["type"].as_str().or(m["function"].as_str()).unwrap_or("") {
            "minecraft:enchant_with_levels" => {
                let levels = int_provider(&m["levels"], random);
                // EnchantmentHelper.enchantItem on the loot context's random.
                let chosen = e.select(ctx.items, random, &stack.item, levels, &pool(&m["options"]));
                let mut out = if &*stack.item == "minecraft:book" {
                    ItemStack::new("minecraft:enchanted_book", stack.count)
                } else {
                    stack
                };
                for (id, l) in chosen {
                    out.enchant(&id, l);
                }
                let extra = if m["include_additional_cost_component"]
                    .as_bool()
                    .unwrap_or(false)
                    && levels > 0
                {
                    levels
                } else {
                    0
                };
                Some((out, extra))
            }
            "minecraft:enchant_randomly" => {
                let book = &*stack.item == "minecraft:book";
                let only_compatible = m["only_compatible"].as_bool().unwrap_or(true);
                let options: Vec<Arc<str>> = pool(&m["options"])
                    .into_iter()
                    .filter(|id| {
                        book || !only_compatible || e.can_enchant(ctx.items, id, &stack.item)
                    })
                    .collect();
                if options.is_empty() {
                    return Some((stack, 0));
                }
                let id = options[random.next_int_bound(options.len() as i32).unwrap_or(0) as usize]
                    .clone();
                let max = e.get(&id).map_or(1, |d| d.max_level) as i32;
                let level = if max <= 1 {
                    1
                } else {
                    1 + random.next_int_bound(max).unwrap_or(0)
                };
                let mut out = if book {
                    ItemStack::new("minecraft:enchanted_book", 1)
                } else {
                    stack
                };
                out.enchant(&id, level as u32);
                let extra = if m["include_additional_cost_component"]
                    .as_bool()
                    .unwrap_or(false)
                {
                    2 + random.next_int_bound(5 + level * 10).unwrap_or(0) + 3 * level
                } else {
                    0
                };
                Some((out, extra))
            }
            "minecraft:filtered" => {
                let filter = &m["item_filter"];
                let item_ok = filter["items"].as_str().is_none_or(|i| i == &*stack.item);
                let needs = |key: &str, have: bool| filter["predicates"].get(key).is_none() || have;
                let data = stack.data();
                let ok = item_ok
                    && needs("minecraft:enchantments", !stack.enchantments().is_empty())
                    && needs(
                        "minecraft:stored_enchantments",
                        !stack.stored_enchantments().is_empty(),
                    )
                    && needs(
                        "minecraft:dyed_color",
                        data.is_some_and(|d| d.dyed_color.is_some()),
                    )
                    && needs("minecraft:map_id", data.is_some_and(|d| d.map_id.is_some()));
                if ok {
                    Some((stack, 0))
                } else if m["on_fail"]["type"] == "minecraft:discard" {
                    None
                } else {
                    Some((stack, 0))
                }
            }
            "minecraft:set_potion" => Some((
                stack.with_potion(m["id"].as_str().unwrap_or("minecraft:water")),
                0,
            )),
            "minecraft:discard" => None,
            "minecraft:set_random_dyes" => {
                let rolls = int_provider(&m["number_of_dyes"], random);
                if rolls <= 0 {
                    return Some((stack, 0));
                }
                let dyes: Vec<usize> = (0..rolls)
                    .map(|_| random.next_int_bound(16).unwrap_or(0) as usize)
                    .collect();
                // DyedItemColor.applyDyes: one item, the colours mixed in.
                let current = stack.data().and_then(|d| d.dyed_color);
                let mut out = ItemStack { count: 1, ..stack };
                out.data_mut().dyed_color = Some(crate::crafting::apply_dyes(current, &dyes));
                Some((out, 0))
            }
            "minecraft:set_random_potion" => {
                let options: Vec<String> = match &m["options"] {
                    Value::String(s) => match s.strip_prefix('#') {
                        Some(tag) => self.potion_tags.get(tag).cloned().unwrap_or_default(),
                        None => vec![s.clone()],
                    },
                    Value::Array(a) => a
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    _ => vec![],
                };
                if options.is_empty() {
                    return Some((stack, 0));
                }
                let pick = random.next_int_bound(options.len() as i32).unwrap_or(0) as usize;
                Some((stack.with_potion(&options[pick]), 0))
            }
            "minecraft:set_stew_effect" => {
                let effects = m["effects"].as_array().cloned().unwrap_or_default();
                if &*stack.item != "minecraft:suspicious_stew" || effects.is_empty() {
                    return Some((stack, 0));
                }
                let entry =
                    &effects[random.next_int_bound(effects.len() as i32).unwrap_or(0) as usize];
                let effect = entry["type"].as_str().unwrap_or("").to_owned();
                let mut duration = int_provider(&entry["duration"], random);
                if !instantaneous(&effect) {
                    duration *= 20;
                }
                let mut out = stack;
                let d = out.data_mut();
                // SuspiciousStewEffects.withEffectAdded
                d.stew_effects.push((Arc::from(effect.as_str()), duration));
                Some((out, 0))
            }
            "minecraft:exploration_map" => {
                let Some(maps) = ctx.maps else {
                    return Some((stack, 0));
                };
                let map = ExplorationMap::from_json(m);
                Some((maps.exploration_map(stack, &map, ctx.origin), 0))
            }
            _ => Some((stack, 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn items() -> Items {
        Items::for_tests(HashMap::new())
    }
    fn offer(cost: u32, b: Option<u32>) -> MerchantOffer {
        MerchantOffer {
            base_cost_a: ItemCost {
                stack: ItemStack::new("minecraft:emerald", cost),
            },
            cost_b: b.map(|n| ItemCost {
                stack: ItemStack::new("minecraft:book", n),
            }),
            result: ItemStack::new("minecraft:iron_boots", 1),
            uses: 0,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier: 0.2,
            xp: 1,
        }
    }

    #[test]
    fn demand_raises_prices() {
        let items = items();
        let mut o = offer(4, None);
        assert_eq!(o.cost_a_count(&items), 4);
        o.uses = 12;
        o.update_demand();
        assert_eq!(o.demand, 12);
        // 4 + floor(4 * 12 * 0.2) = 13.
        assert_eq!(o.cost_a_count(&items), 13);
        o.special_price_diff = -20;
        assert_eq!(o.cost_a_count(&items), 1);
    }

    #[test]
    fn restocks_twice_a_day() {
        let mut t = Trader {
            offers: vec![offer(4, None)],
            ..Default::default()
        };
        assert!(!t.should_restock(100, 0), "nothing used yet");
        t.offers[0].uses = 12;
        assert!(t.should_restock(100, 0));
        t.restock(100);
        assert_eq!(
            (t.offers[0].uses, t.offers[0].demand, t.restocks_today),
            (0, 12, 1)
        );
        t.offers[0].uses = 1;
        // The second restock needs 2400 ticks since the first.
        assert!(!t.should_restock(2000, 0));
        assert!(t.should_restock(2600, 0));
        t.restock(2600);
        t.offers[0].uses = 1;
        assert!(!t.should_restock(6000, 0), "two a day");
        // A new day resets the count and catches up on demand.
        assert!(t.should_restock(30000, 1));
        assert_eq!(t.restocks_today, 0);
    }

    #[test]
    fn payment_and_selection() {
        let items = items();
        let offers = vec![offer(5, None), offer(1, Some(1))];
        let mut a = Some(ItemStack::new("minecraft:emerald", 6));
        let mut b = None;
        assert_eq!(
            sell_item(&offers, &items, a.as_ref(), b.as_ref(), None).0,
            Some(0)
        );
        assert!(offers[0].take(&items, &mut a, &mut b));
        assert_eq!(a.as_ref().unwrap().count, 1);
        // Payment in the second slot only still matches (A and B swap).
        let book = Some(ItemStack::new("minecraft:book", 1));
        let (i, r) = sell_item(
            &offers,
            &items,
            None,
            Some(&ItemStack::new("minecraft:emerald", 5)),
            None,
        );
        assert_eq!(
            (i, r.map(|r| r.item)),
            (Some(0), Some(Arc::from("minecraft:iron_boots")))
        );
        assert_eq!(
            sell_item(&offers, &items, a.as_ref(), book.as_ref(), Some(1)).0,
            Some(1)
        );
        let mut out = offers[0].clone();
        out.uses = 12;
        assert_eq!(
            sell_item(
                &[out],
                &items,
                Some(&ItemStack::new("minecraft:emerald", 5)),
                None,
                None
            )
            .1,
            None
        );
    }

    #[test]
    fn sequence_random_is_stable() {
        let mut a = sequence_random(12345, "minecraft:trade_set/armorer/level_1");
        let mut b = sequence_random(12345, "minecraft:trade_set/armorer/level_1");
        let mut c = sequence_random(12345, "minecraft:trade_set/butcher/level_1");
        let (x, y, z) = (a.next_long(), b.next_long(), c.next_long());
        assert_eq!(x, y);
        assert_ne!(x, z);
        assert_eq!(min_xp(2), 10);
        assert_eq!(max_xp(2), 70);
        assert!(!can_level_up(5));
    }

    #[test]
    fn reputation_and_heroes_lower_prices() {
        let items = items();
        let mut offers = vec![offer(10, None), offer(1, None)];
        // Reputation 50 with a 0.2 multiplier: 10 off, floored per offer.
        update_special_prices(&mut offers, 50, None);
        assert_eq!(offers[0].special_price_diff, -10);
        assert_eq!(offers[0].cost_a_count(&items), 1);
        assert_eq!(offers[1].special_price_diff, -10);
        // Bad reputation raises the price (-30 * 0.2 = -6 floors to -6).
        update_special_prices(&mut offers, -30, None);
        assert_eq!(offers[0].special_price_diff, 6);
        assert_eq!(offers[0].cost_a_count(&items), 16);
        // Hero of the Village I: 0.3 * 10 = 3 off, at least 1.
        update_special_prices(&mut offers, 0, Some(0));
        assert_eq!(offers[0].special_price_diff, -3);
        assert_eq!(offers[1].special_price_diff, -1);
        // Level III: 0.3 + 0.125 = 0.425 -> 4 off a ten emerald price.
        update_special_prices(&mut offers, 0, Some(2));
        assert_eq!(offers[0].special_price_diff, -4);
        // Both stack, from scratch each time.
        update_special_prices(&mut offers, 25, Some(0));
        assert_eq!(offers[0].special_price_diff, -5 - 3);
        update_special_prices(&mut offers, 0, None);
        assert_eq!(offers[0].special_price_diff, 0);
    }
    #[test]
    fn offers_follow_the_trade_data() {
        let items = items();
        let e = crate::enchant::tests::sample();
        let mut trades = HashMap::new();
        trades.insert("minecraft:t/boots".into(), json!({"gives":{"id":"minecraft:iron_boots"},"max_uses":12,"reputation_discount":0.2,"wants":{"count":4,"id":"minecraft:emerald"}}));
        trades.insert("minecraft:t/helmet".into(), json!({"gives":{"id":"minecraft:iron_helmet"},"max_uses":12,"reputation_discount":0.2,"wants":{"count":5,"id":"minecraft:emerald"}}));
        trades.insert("minecraft:t/desert".into(), json!({"gives":{"id":"minecraft:sand"},"wants":{"id":"minecraft:emerald"},"merchant_predicate":{"entity":"this","predicate":{"minecraft:predicates":{"minecraft:villager/variant":"minecraft:desert"}},"type":"minecraft:entity_properties"}}));
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:t/level_1".into(),
            vec![
                "minecraft:t/boots".into(),
                "minecraft:t/helmet".into(),
                "minecraft:t/desert".into(),
            ],
        );
        let mut sets = HashMap::new();
        sets.insert(
            "minecraft:t/level_1".into(),
            json!({"amount": 5, "trades": "#minecraft:t/level_1"}),
        );
        let t = Trades {
            trades,
            tags,
            sets,
            potion_tags: HashMap::new(),
        };
        let ctx = TradeContext {
            items: &items,
            enchantments: &e,
            variant: "minecraft:plains",
            origin: [0.0; 3],
            maps: None,
        };
        let offers = t.offers_from_set(&ctx, "minecraft:t/level_1", &mut sequence_random(1, "x"));
        // The desert-only trade is skipped for a plains villager; no duplicates.
        assert_eq!(offers.len(), 2);
        assert_ne!(offers[0].result.item, offers[1].result.item);
        assert!(
            offers
                .iter()
                .all(|o| o.max_uses == 12 && (o.price_multiplier - 0.2).abs() < 1e-6)
        );
    }
}
