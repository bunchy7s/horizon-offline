//! The vault (`VaultBlockEntity` with its `VaultConfig`, `VaultServerData`
//! and `VaultSharedData`): the server tick that walks `VaultState` (inactive,
//! active while players who were not rewarded yet are near, unlocking when one
//! inserts the key, ejecting the rewards one by one) and the key insertion.
//!
//! The state lives in the block (`vault_state`); the tick reads it and reports
//! the next one. The level's random draws the display item and throws the
//! rewards; the loot tables go through the caller, so the game and a
//! differential test world roll them the same way.
pub use crate::chunk::block_entity::{Vault, VaultConfig};
use crate::items::ItemStack;
use horizon_core::random::RandomSource;
use horizon_world::nbt::{Compound, Tag};

/// A player's UUID as its four ints (`UUIDUtil.CODEC`).
pub type Uuid = [i32; 4];

/// `BuiltInLootTables.TRIAL_CHAMBERS_REWARD`, the default config's table.
pub const DEFAULT_LOOT_TABLE: &str = "minecraft:chests/trial_chambers/reward";
const DEFAULT_ACTIVATION_RANGE: f64 = 4.0;
const DEFAULT_DEACTIVATION_RANGE: f64 = 4.5;
/// `VaultServerData.MAX_REWARD_PLAYERS`.
const MAX_REWARD_PLAYERS: usize = 128;
/// `UNLOCKING_DELAY_TICKS`, `INSERT_FAIL_SOUND_BUFFER_TICKS` and the 20 ticks
/// of `VaultState` between player checks and ejections.
const UNLOCKING_DELAY: i64 = 14;
const INSERT_FAIL_SOUND_BUFFER: i64 = 15;
const STATE_DELAY: i64 = 20;

impl Default for VaultConfig {
    /// `VaultConfig.DEFAULT`: the trial key and the trial chambers' reward.
    fn default() -> Self {
        Self {
            loot_table: DEFAULT_LOOT_TABLE.into(),
            activation_range: DEFAULT_ACTIVATION_RANGE,
            deactivation_range: DEFAULT_DEACTIVATION_RANGE,
            key_item: Some(ItemStack::new("minecraft:trial_key", 1)),
            override_loot_table_to_display: None,
        }
    }
}

impl VaultConfig {
    /// `VaultConfig.CODEC`: a field that fails to decode takes its default
    /// (the key's is none); an activation range past the deactivation range
    /// fails the whole config.
    fn decode(tag: &Tag) -> Option<Self> {
        let Tag::Compound(_) = tag else {
            return None;
        };
        let range = |k: &str, default: f64| {
            horizon_client_core::nbt_value::float(tag.get(k)).unwrap_or(default)
        };
        let config = Self {
            loot_table: identifier(tag.get("loot_table"))
                .unwrap_or_else(|| DEFAULT_LOOT_TABLE.into()),
            activation_range: range("activation_range", DEFAULT_ACTIVATION_RANGE),
            deactivation_range: range("deactivation_range", DEFAULT_DEACTIVATION_RANGE),
            key_item: tag.get("key_item").and_then(stack),
            override_loot_table_to_display: identifier(tag.get("override_loot_table_to_display")),
        };
        (config.activation_range <= config.deactivation_range).then_some(config)
    }
    /// The fields that differ from their defaults.
    fn encode(&self) -> Tag {
        let mut out = Compound::new();
        if self.loot_table != DEFAULT_LOOT_TABLE {
            out.insert(
                "loot_table".into(),
                Tag::String(self.loot_table.as_str().into()),
            );
        }
        if self.activation_range != DEFAULT_ACTIVATION_RANGE {
            out.insert(
                "activation_range".into(),
                Tag::Double(self.activation_range.to_bits()),
            );
        }
        if self.deactivation_range != DEFAULT_DEACTIVATION_RANGE {
            out.insert(
                "deactivation_range".into(),
                Tag::Double(self.deactivation_range.to_bits()),
            );
        }
        if let Some(key) = &self.key_item {
            out.insert("key_item".into(), horizon_items::item_nbt::item_to_nbt(key));
        }
        if let Some(table) = &self.override_loot_table_to_display {
            out.insert(
                "override_loot_table_to_display".into(),
                Tag::String(table.as_str().into()),
            );
        }
        Tag::Compound(out)
    }
}

/// `VaultState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VaultState {
    #[default]
    Inactive,
    Active,
    Unlocking,
    Ejecting,
}

impl VaultState {
    const ALL: [Self; 4] = [
        Self::Inactive,
        Self::Active,
        Self::Unlocking,
        Self::Ejecting,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Active => "active",
            Self::Unlocking => "unlocking",
            Self::Ejecting => "ejecting",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// A player the vault can detect (`PlayerDetector.INCLUDING_CREATIVE_PLAYERS`).
#[derive(Clone, Copy, Debug)]
pub struct VaultPlayer {
    pub uuid: Uuid,
    pub block: [i32; 3],
    pub spectator: bool,
}

/// What a vault did that the level shows or sends.
#[derive(Clone, Debug, PartialEq)]
pub enum VaultEvent {
    /// `setBlockAndUpdate` with this `vault_state`.
    State(VaultState),
    /// `sendBlockUpdated`: the clients load the new shared data.
    BlockUpdated,
    /// `levelEvent(id, pos, data)`.
    LevelEvent(i32, i32),
    /// `playSound(null, vault, sound, BLOCKS, volume, pitch)`.
    Sound(&'static str, f32, f32),
    /// `DefaultDispenseItemBehavior.spawnItem` from the top of the vault:
    /// the item and its velocity.
    Eject(ItemStack, [f64; 3]),
}

/// What a vault reads from the level.
pub trait VaultLevel {
    fn game_time(&self) -> i64;
    /// `level.getPlayers()`, in the server's order.
    fn players(&self) -> Vec<VaultPlayer>;
}

/// Rolls a loot table (`LootTable.getRandomItems`, stacks split) with the
/// level random, or with the table's random sequence when the flag is set and
/// the table has one.
pub type Roll<'a, R> = dyn FnMut(&str, &mut R, bool) -> Vec<ItemStack> + 'a;

impl Default for Vault {
    fn default() -> Self {
        Self {
            config: VaultConfig::default(),
            rewarded_players: Vec::new(),
            state_updating_resumes_at: 0,
            items_to_eject: Vec::new(),
            total_ejections_needed: 0,
            last_insert_fail_timestamp: 0,
            display_item: None,
            connected_players: Vec::new(),
            connected_particles_range: DEFAULT_DEACTIVATION_RANGE,
            shared_dirty: false,
            spin: 0.0,
            o_spin: 0.0,
        }
    }
}

/// An `Identifier` (`minecraft` when the namespace is left out).
fn identifier(t: Option<&Tag>) -> Option<String> {
    match t? {
        Tag::String(s) => horizon_client_core::identifier::normalize_id(&s.to_string_lossy()),
        _ => None,
    }
}
/// `ItemStack.CODEC`: an item other than air, 1 to 99 of it.
fn stack(tag: &Tag) -> Option<ItemStack> {
    let id = identifier(tag.get("id"))?;
    if let Some(count) = tag.get("count")
        && !horizon_client_core::nbt_value::number(Some(count))
            .is_some_and(|c| (1..=99).contains(&c))
    {
        return None;
    }
    let mut item = horizon_items::item_nbt::item_from_nbt(tag)?;
    item.item = id.as_str().into();
    (&*item.item != "minecraft:air").then_some(item)
}
/// `UUIDUtil.CODEC_LINKED_SET`: every entry a UUID, duplicates dropped.
fn uuids(tag: Option<&Tag>) -> Option<Vec<Uuid>> {
    let Tag::List(list) = tag? else {
        return None;
    };
    let mut out: Vec<Uuid> = Vec::new();
    for t in list {
        let Tag::IntArray(v) = t else {
            return None;
        };
        let u = <Uuid>::try_from(v.as_slice()).ok()?;
        if !out.contains(&u) {
            out.push(u);
        }
    }
    Some(out)
}
fn encode_uuids(list: &[Uuid]) -> Tag {
    Tag::List(list.iter().map(|u| Tag::IntArray(u.to_vec())).collect())
}
/// `ItemStack.matches`, with None as the empty stack.
fn matches(a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.count == b.count && a.same_item_same_components(b),
        _ => false,
    }
}
fn same_set(a: &[Uuid], b: &[Uuid]) -> bool {
    a.len() == b.len() && a.iter().all(|u| b.contains(u))
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(degrees: f32) -> f32 {
    let t = degrees % 360.0;
    if t >= 180.0 {
        t - 360.0
    } else if t < -180.0 {
        t + 360.0
    } else {
        t
    }
}

impl Vault {
    /// `VaultClientData.updateDisplayItemSpin`, every client tick.
    pub fn update_display_item_spin(&mut self) {
        self.o_spin = self.spin;
        self.spin = wrap_degrees(self.spin + 10.0);
    }
    /// `VaultRenderer`'s spin: `Mth.rotLerp` between the last two ticks.
    pub fn display_spin(&self, partial: f32) -> f32 {
        self.o_spin + partial * wrap_degrees(self.spin - self.o_spin)
    }
    /// `loadAdditional`. `VaultServerData.set` leaves the total of ejections
    /// out, so a vault loaded mid-ejection counts from 0.
    pub fn load(tag: &Tag) -> Self {
        let mut v = Self {
            config: tag
                .get("config")
                .map_or_else(|| Some(VaultConfig::default()), VaultConfig::decode)
                .unwrap_or_default(),
            ..Self::default()
        };
        if let Some(Tag::Compound(_)) = tag.get("server_data") {
            let server = tag.get("server_data");
            let get = |k: &str| server.and_then(|s| s.get(k));
            v.rewarded_players = uuids(get("rewarded_players")).unwrap_or_default();
            v.state_updating_resumes_at =
                horizon_client_core::nbt_value::number(get("state_updating_resumes_at"))
                    .unwrap_or(0);
            v.items_to_eject = match get("items_to_eject") {
                Some(Tag::List(list)) => list
                    .iter()
                    .map(stack)
                    .collect::<Option<_>>()
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
        }
        if let Some(Tag::Compound(_)) = tag.get("shared_data") {
            let shared = tag.get("shared_data");
            let get = |k: &str| shared.and_then(|s| s.get(k));
            v.display_item = get("display_item").and_then(stack);
            v.connected_players = uuids(get("connected_players")).unwrap_or_default();
            v.connected_particles_range =
                horizon_client_core::nbt_value::float(get("connected_particles_range"))
                    .unwrap_or(DEFAULT_DEACTIVATION_RANGE);
        }
        v
    }
    /// `saveAdditional`: the config, shared data and server data.
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        let mut server = Compound::new();
        if !self.rewarded_players.is_empty() {
            server.insert(
                "rewarded_players".into(),
                encode_uuids(&self.rewarded_players),
            );
        }
        if self.state_updating_resumes_at != 0 {
            server.insert(
                "state_updating_resumes_at".into(),
                Tag::Long(self.state_updating_resumes_at),
            );
        }
        if !self.items_to_eject.is_empty() {
            let items = self
                .items_to_eject
                .iter()
                .map(horizon_items::item_nbt::item_to_nbt)
                .collect();
            server.insert("items_to_eject".into(), Tag::List(items));
        }
        if self.total_ejections_needed != 0 {
            server.insert(
                "total_ejections_needed".into(),
                Tag::Int(self.total_ejections_needed),
            );
        }
        vec![
            ("config", self.config.encode()),
            ("shared_data", self.update_shared()),
            ("server_data", Tag::Compound(server)),
        ]
    }
    /// `VaultSharedData.CODEC`.
    fn update_shared(&self) -> Tag {
        let mut out = Compound::new();
        if let Some(item) = &self.display_item {
            out.insert(
                "display_item".into(),
                horizon_items::item_nbt::item_to_nbt(item),
            );
        }
        if !self.connected_players.is_empty() {
            out.insert(
                "connected_players".into(),
                encode_uuids(&self.connected_players),
            );
        }
        if self.connected_particles_range != DEFAULT_DEACTIVATION_RANGE {
            out.insert(
                "connected_particles_range".into(),
                Tag::Double(self.connected_particles_range.to_bits()),
            );
        }
        Tag::Compound(out)
    }
    /// `getUpdateTag`: the shared data the clients see.
    pub fn update_tag(&self) -> Tag {
        Tag::compound([("shared_data", self.update_shared())])
    }
    /// `VaultSharedData.setDisplayItem`.
    fn set_display_item(&mut self, item: Option<ItemStack>) {
        if !matches(self.display_item.as_ref(), item.as_ref()) {
            self.display_item = item;
            self.shared_dirty = true;
        }
    }
    /// `canEjectReward`.
    fn can_eject_reward(&self, state: VaultState) -> bool {
        self.config.key_item.is_some() && state != VaultState::Inactive
    }
    /// `cycleDisplayItemFromLootTable`: one item of the display table, rolled
    /// with the level random (`Util.getRandom`).
    fn cycle_display_item<R: RandomSource>(
        &mut self,
        state: VaultState,
        random: &mut R,
        roll: &mut Roll<R>,
    ) {
        if !self.can_eject_reward(state) {
            self.set_display_item(None);
            return;
        }
        let table = self
            .config
            .override_loot_table_to_display
            .as_ref()
            .unwrap_or(&self.config.loot_table);
        let mut items = roll(table, random, false);
        let item = if items.is_empty() {
            None
        } else {
            let i = random.next_int_bound(items.len() as i32).unwrap_or(0) as usize;
            Some(items.swap_remove(i))
        };
        self.set_display_item(item);
    }
    /// `VaultSharedData.updateConnectedPlayersWithinRange` with
    /// `PlayerDetector.INCLUDING_CREATIVE_PLAYERS`.
    fn update_connected_players<L: VaultLevel>(&mut self, l: &L, pos: [i32; 3], range: f64) {
        let found: Vec<Uuid> = l
            .players()
            .into_iter()
            .filter(|p| {
                let d = [0, 1, 2].map(|i| f64::from(p.block[i] - pos[i]));
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range && !p.spectator
            })
            .map(|p| p.uuid)
            .filter(|u| !self.rewarded_players.contains(u))
            .collect();
        if !same_set(&self.connected_players, &found) {
            self.connected_players = found;
            self.shared_dirty = true;
        }
    }
    /// `updateStateForConnectedPlayers`.
    fn update_state_for_connected_players<L: VaultLevel>(
        &mut self,
        l: &L,
        pos: [i32; 3],
        range: f64,
    ) -> VaultState {
        self.update_connected_players(l, pos, range);
        self.state_updating_resumes_at = l.game_time() + STATE_DELAY;
        if self.connected_players.is_empty() {
            VaultState::Inactive
        } else {
            VaultState::Active
        }
    }
    /// `VaultServerData.ejectionProgress`.
    fn ejection_progress(&self) -> f32 {
        if self.total_ejections_needed == 1 {
            return 1.0;
        }
        // Mth.inverseLerp(size, 1, total), in floats.
        let size = self.items_to_eject.len() as f32;
        1.0 - (size - 1.0) / (self.total_ejections_needed as f32 - 1.0)
    }
    /// `VaultState.tickAndGetNext`.
    fn tick_and_get_next<L: VaultLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: VaultState,
        events: &mut Vec<VaultEvent>,
    ) -> VaultState {
        match state {
            VaultState::Inactive => {
                self.update_state_for_connected_players(l, pos, self.config.activation_range)
            }
            VaultState::Active => {
                self.update_state_for_connected_players(l, pos, self.config.deactivation_range)
            }
            VaultState::Unlocking => {
                self.state_updating_resumes_at = l.game_time() + STATE_DELAY;
                VaultState::Ejecting
            }
            VaultState::Ejecting => {
                // The progress counts the item being ejected.
                let progress = self.ejection_progress();
                let Some(item) = self.items_to_eject.pop() else {
                    self.total_ejections_needed = 0;
                    return self.update_state_for_connected_players(
                        l,
                        pos,
                        self.config.deactivation_range,
                    );
                };
                // DefaultDispenseItemBehavior.spawnItem going up, accuracy 2: the
                // power only scales x and z, which are 0.
                random.next_double();
                let spread = 0.017_227_5 * 2.0;
                let v = [
                    random.triangle(0.0, spread),
                    random.triangle(0.2, spread),
                    random.triangle(0.0, spread),
                ];
                events.push(VaultEvent::Eject(item, v));
                events.push(VaultEvent::LevelEvent(3017, 0));
                events.push(VaultEvent::Sound(
                    "block.vault.eject_item",
                    1.0,
                    0.8 + 0.4 * progress,
                ));
                self.set_display_item(self.items_to_eject.last().cloned());
                self.state_updating_resumes_at = l.game_time() + STATE_DELAY;
                VaultState::Ejecting
            }
        }
    }
    /// `VaultState.onTransition`: `from`'s exit, then `to`'s entry.
    #[allow(clippy::too_many_arguments)]
    fn on_transition<R: RandomSource>(
        &mut self,
        from: VaultState,
        to: VaultState,
        ominous: bool,
        random: &mut R,
        roll: &mut Roll<R>,
        events: &mut Vec<VaultEvent>,
    ) {
        events.push(VaultEvent::State(to));
        if from == VaultState::Ejecting {
            events.push(VaultEvent::Sound("block.vault.close_shutter", 1.0, 1.0));
        }
        match to {
            VaultState::Inactive => {
                self.set_display_item(None);
                events.push(VaultEvent::LevelEvent(3016, i32::from(ominous)));
            }
            VaultState::Active => {
                if self.display_item.is_none() {
                    self.cycle_display_item(to, random, roll);
                }
                events.push(VaultEvent::LevelEvent(3015, i32::from(ominous)));
            }
            VaultState::Unlocking => {
                events.push(VaultEvent::Sound("block.vault.insert_item", 1.0, 1.0))
            }
            VaultState::Ejecting => {
                events.push(VaultEvent::Sound("block.vault.open_shutter", 1.0, 1.0))
            }
        }
    }
    /// `VaultBlockEntity.Server.tick`: the next state of the block.
    #[allow(clippy::too_many_arguments)]
    pub fn server_tick<L: VaultLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: VaultState,
        ominous: bool,
        roll: &mut Roll<R>,
        events: &mut Vec<VaultEvent>,
    ) -> VaultState {
        let now = l.game_time();
        if now % 20 == 0 && state == VaultState::Active {
            self.cycle_display_item(state, random, roll);
        }
        let mut next = state;
        if now >= self.state_updating_resumes_at {
            next = self.tick_and_get_next(l, random, pos, state, events);
            if next != state {
                self.on_transition(state, next, ominous, random, roll, events);
            }
        }
        if std::mem::take(&mut self.shared_dirty) {
            events.push(VaultEvent::BlockUpdated);
        }
        next
    }
    /// `VaultBlock.useItemOn`: None when the item passes to the empty hand (an
    /// empty hand or a vault that is not active), else how many of `stack`
    /// the vault takes (none from a player with infinite materials).
    #[allow(clippy::too_many_arguments)]
    pub fn use_item_on<L: VaultLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: VaultState,
        ominous: bool,
        player: Uuid,
        stack: Option<&ItemStack>,
        roll: &mut Roll<R>,
        events: &mut Vec<VaultEvent>,
    ) -> Option<u32> {
        let stack = stack.filter(|s| s.count > 0)?;
        (state == VaultState::Active).then(|| {
            self.try_insert_key(l, random, pos, state, ominous, player, stack, roll, events)
        })
    }
    /// `VaultBlockEntity.Server.tryInsertKey`: how many of `stack` the vault
    /// takes.
    #[allow(clippy::too_many_arguments)]
    fn try_insert_key<L: VaultLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: VaultState,
        ominous: bool,
        player: Uuid,
        stack: &ItemStack,
        roll: &mut Roll<R>,
        events: &mut Vec<VaultEvent>,
    ) -> u32 {
        if !self.can_eject_reward(state) {
            return 0;
        }
        let Some(key) = &self.config.key_item else {
            return 0;
        };
        let key_count = key.count;
        if !(stack.same_item_same_components(key) && stack.count >= key_count) {
            self.play_insert_fail_sound(l, "block.vault.insert_item_fail", events);
            return 0;
        }
        if self.rewarded_players.contains(&player) {
            self.play_insert_fail_sound(l, "block.vault.reject_rewarded_player", events);
            return 0;
        }
        let items = roll(&self.config.loot_table.clone(), random, true);
        if items.is_empty() {
            return 0;
        }
        // unlock
        self.total_ejections_needed = items.len() as i32;
        self.items_to_eject = items;
        self.set_display_item(self.items_to_eject.last().cloned());
        self.state_updating_resumes_at = l.game_time() + UNLOCKING_DELAY;
        self.on_transition(state, VaultState::Unlocking, ominous, random, roll, events);
        // addToRewardedPlayers
        if !self.rewarded_players.contains(&player) {
            self.rewarded_players.push(player);
            if self.rewarded_players.len() > MAX_REWARD_PLAYERS {
                self.rewarded_players.remove(0);
            }
        }
        self.update_connected_players(l, pos, self.config.deactivation_range);
        key_count
    }
    /// `playInsertFailSound`: at most once every 15 ticks.
    fn play_insert_fail_sound<L: VaultLevel>(
        &mut self,
        l: &L,
        sound: &'static str,
        events: &mut Vec<VaultEvent>,
    ) {
        if l.game_time() >= self.last_insert_fail_timestamp + INSERT_FAIL_SOUND_BUFFER {
            events.push(VaultEvent::Sound(sound, 1.0, 1.0));
            self.last_insert_fail_timestamp = l.game_time();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    fn tag(snbt: &str) -> Tag {
        horizon_world::nbt::parse_snbt(snbt).expect("snbt").0
    }
    struct Level {
        time: i64,
        players: Vec<VaultPlayer>,
    }
    impl VaultLevel for Level {
        fn game_time(&self) -> i64 {
            self.time
        }
        fn players(&self) -> Vec<VaultPlayer> {
            self.players.clone()
        }
    }

    #[test]
    fn a_missing_config_is_the_trial_key_and_an_invalid_one_too() {
        assert_eq!(Vault::load(&tag("{}")).config, VaultConfig::default());
        let v = Vault::load(&tag(
            "{config:{activation_range:5.0d,deactivation_range:4.0d}}",
        ));
        assert_eq!(v.config, VaultConfig::default());
        // A config without a key never opens.
        let v = Vault::load(&tag(
            r#"{config:{loot_table:"minecraft:chests/trial_chambers/reward_ominous"}}"#,
        ));
        assert_eq!(v.config.key_item, None);
        assert!(!v.can_eject_reward(VaultState::Active));
    }

    #[test]
    fn a_rewarded_player_is_refused_and_a_new_one_unlocks() {
        let player = [0, 0, 0, 1];
        let l = Level {
            time: 100,
            players: vec![VaultPlayer {
                uuid: player,
                block: [1, 0, 0],
                spectator: false,
            }],
        };
        let mut v = Vault::default();
        let mut random = LegacyRandom::new(1);
        let mut events = Vec::new();
        let key = ItemStack::new("minecraft:trial_key", 1);
        let mut roll =
            |_: &str, _: &mut LegacyRandom, _: bool| vec![ItemStack::new("minecraft:diamond", 2)];
        let pos = [0, 0, 0];
        let taken = v.use_item_on(
            &l,
            &mut random,
            pos,
            VaultState::Inactive,
            false,
            player,
            Some(&key),
            &mut roll,
            &mut events,
        );
        assert_eq!(taken, None);
        let taken = v.use_item_on(
            &l,
            &mut random,
            pos,
            VaultState::Active,
            false,
            player,
            Some(&key),
            &mut roll,
            &mut events,
        );
        assert_eq!(taken, Some(1));
        assert_eq!(events[0], VaultEvent::State(VaultState::Unlocking));
        assert_eq!(v.rewarded_players, [player]);
        assert_eq!(v.state_updating_resumes_at, 114);
        events.clear();
        let taken = v.use_item_on(
            &l,
            &mut random,
            pos,
            VaultState::Active,
            false,
            player,
            Some(&key),
            &mut roll,
            &mut events,
        );
        assert_eq!(taken, Some(0));
        assert_eq!(
            events,
            [VaultEvent::Sound(
                "block.vault.reject_rewarded_player",
                1.0,
                1.0
            )]
        );
    }

    #[test]
    fn saved_data_round_trips() {
        let v = Vault::load(&tag(
            r#"{server_data:{rewarded_players:[[I;0,0,0,1]],state_updating_resumes_at:40L,items_to_eject:[{id:"minecraft:emerald",count:3}]},shared_data:{display_item:{id:"minecraft:emerald",count:3}}}"#,
        ));
        let saved = Tag::compound(v.save());
        assert_eq!(Vault::load(&saved), v);
    }
}
