//! Block entity data: what a chunk stores for its block entities (`BlockEntity`
//! and its payloads). Plain data, as vanilla's block entities are world data;
//! their behaviour lives with the features that use them (`containers`,
//! `brewing`, `signs`, ...), which re-export these types under their old paths.
use horizon_client_core::{text::Text, who::Who};
use horizon_items::{banner_layer::BannerLayer, items::ItemStack};
use horizon_world::{
    level::sculk::SculkSpreader,
    nbt::{Compound, Tag},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// An entity's UUID as its four ints (`UUIDUtil.CODEC`).
pub type Uuid = [i32; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnaceKind {
    Furnace,
    BlastFurnace,
    Smoker,
}
#[derive(Clone, Debug)]
pub struct Furnace {
    pub kind: FurnaceKind,
    /// input, fuel, result
    pub items: [Option<ItemStack>; 3],
    pub lit_remaining: i32,
    pub lit_total: i32,
    pub cooking_timer: i32,
    pub cooking_total: i32,
    pub speed: f32,
    /// `AbstractFurnaceBlockEntity.recipesUsed`: how many of each recipe this furnace has
    /// made since a player last took from its output. The experience is owed per recipe
    /// rather than per item, and a save keeps it as the `RecipesUsed` compound.
    pub recipes_used: HashMap<String, u32>,
}
#[derive(Clone, Debug)]
pub enum BlockEntity {
    Chest(Vec<Option<ItemStack>>),
    Furnace(Furnace),
    Brewing(Box<BrewingStand>),
    /// Signs and hanging signs (`SignBlockEntity`).
    Sign(SignData),
    /// `JukeboxBlockEntity`: the record and its `JukeboxSongPlayer`.
    Jukebox(Jukebox),
    /// `BeaconBlockEntity`.
    Beacon(Box<Beacon>),
    /// `LecternBlockEntity`: the book and the open page.
    Lectern(Lectern),
    /// Sculk sensors, shriekers and catalysts (`crate::sculk`).
    Sculk(Box<SculkEntity>),
    /// `CreakingHeartBlockEntity`: its protector and the hurt emitter.
    CreakingHeart(Box<CreakingHeart>),
    /// `BannerBlockEntity`: the pattern layers.
    Banner(Banner),
    /// `CrafterBlockEntity`.
    Crafter(Crafter),
    /// `SpawnerBlockEntity` (`crate::mobs::spawner`).
    Spawner(Box<Spawner>),
    /// `TrialSpawnerBlockEntity` (`crate::mobs::trial_spawner`).
    TrialSpawner(Box<TrialSpawner>),
    /// `VaultBlockEntity` (`crate::vault`).
    Vault(Box<Vault>),
    /// `BeehiveBlockEntity` of beehives and bee nests (`crate::beehive`).
    Beehive(Box<Beehive>),
    /// `CommandBlockEntity` of the three command blocks (`crate::command_block`).
    CommandBlock(Box<CommandBlock>),
    /// `CampfireBlockEntity` of campfires and soul campfires.
    Campfire(Box<Campfire>),
    /// `SkullBlockEntity` of every skull and head block.
    Skull(Skull),
    /// `TheEndGatewayBlockEntity`: its age and configured destination.
    EndGateway(EndGateway),
    /// `DaylightDetectorBlockEntity`: no data, only the ticker
    /// (`DaylightDetectorBlock.tickEntity`).
    DaylightDetector,
    /// Suspicious sand and gravel: saved loot and the transient brushed face.
    Brushable(Box<Brushable>),
    /// A shelf holds three displayed item stacks.
    Shelf(Box<Shelf>),
    /// `ChiseledBookShelfBlockEntity`: a `Container` of six one-book slots.
    ChiseledBookshelf(Box<ChiseledBookshelf>),
    /// A block entity of a mod (`native_block_entity`), kept as its saved
    /// compound for the mod to read and change.
    Mod(Box<ModBlockEntity>),
    /// `PotentSulfurBlockEntity` (`game/potent_sulfur.rs`).
    PotentSulfur(PotentSulfur),
    /// `StructureBlockEntity` (`structure_block.rs`).
    StructureBlock(Box<crate::structure_block::StructureBlock>),
    /// `JigsawBlockEntity` (`jigsaw_block.rs`).
    JigsawBlock(Box<crate::jigsaw_block::JigsawBlock>),
    /// `TestBlockEntity` (`test_block.rs`).
    TestBlock(Box<crate::test_block::TestBlock>),
    /// `TestInstanceBlockEntity` (`test_instance_block.rs`).
    TestInstanceBlock(Box<crate::test_instance_block::TestInstanceBlock>),
}
/// A mod's block entity, read and changed through its fields
/// (`ModBlockEntityFields`) as a struct's: `entity.nbt`. Each change goes
/// through `DerefMut` and gives it a new `revision`, so a client tells
/// whether what it kept of it (`BlockEntityDraw::keep`) is still right
/// without comparing the whole entity, and no write can skip that.
pub struct ModBlockEntity {
    fields: ModBlockEntityFields,
    revision: ModRevision,
}
/// What a `ModBlockEntity` holds: its type id (`mymod:machine`), its saved
/// compound without `id`, `x`, `y`, `z` and `Items`, its item slots
/// (`Items`) when its block has a container (`ModContainer`), and the data
/// slots its menu shows (`ContainerData`: a furnace menu's flame and arrow),
/// which the mod sets and nothing saves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModBlockEntityFields {
    pub id: String,
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}
/// Which contents a `ModBlockEntity` has had: one per entity made (a clone
/// is another), counted on by each change. Equal revisions, equal contents.
/// Neither saved nor compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModRevision {
    born: u64,
    changes: u64,
}
static MOD_BLOCK_ENTITIES_MADE: AtomicU64 = AtomicU64::new(0);
impl ModBlockEntity {
    pub fn new(fields: ModBlockEntityFields) -> Self {
        Self {
            fields,
            revision: ModRevision {
                born: MOD_BLOCK_ENTITIES_MADE.fetch_add(1, Ordering::Relaxed),
                changes: 0,
            },
        }
    }
    pub fn revision(&self) -> ModRevision {
        self.revision
    }
    /// A client's copy loads the tag the server sent (`loadWithComponents`
    /// in `ClientPacketListener.handleBlockEntityData`): `tag`'s fields
    /// replace its own of the same names. One it already has as sent is
    /// left alone, so a tag sent again unchanged keeps its revision.
    pub fn merge_update_tag(&mut self, tag: &Compound) {
        for (k, v) in tag {
            if self.nbt.get(k) != Some(v) {
                self.nbt.insert(k.clone(), v.clone());
            }
        }
    }
}
impl From<ModBlockEntityFields> for ModBlockEntity {
    fn from(fields: ModBlockEntityFields) -> Self {
        Self::new(fields)
    }
}
impl Default for ModBlockEntity {
    fn default() -> Self {
        Self::new(ModBlockEntityFields::default())
    }
}
impl Clone for ModBlockEntity {
    fn clone(&self) -> Self {
        Self::new(self.fields.clone())
    }
}
impl PartialEq for ModBlockEntity {
    fn eq(&self, other: &Self) -> bool {
        self.fields == other.fields
    }
}
impl std::fmt::Debug for ModBlockEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fields.fmt(f)
    }
}
impl std::ops::Deref for ModBlockEntity {
    type Target = ModBlockEntityFields;
    fn deref(&self) -> &ModBlockEntityFields {
        &self.fields
    }
}
impl std::ops::DerefMut for ModBlockEntity {
    fn deref_mut(&mut self) -> &mut ModBlockEntityFields {
        self.revision.changes += 1;
        &mut self.fields
    }
}
/// The container of a mod block's block entity, as a `WorldlyContainer`
/// says it: its slot count, the slots each face reaches
/// (`getSlotsForFace`, by `Direction`: down, up, north, south, west, east),
/// and the slots hoppers may put into (`canPlaceItemThroughFace`) and take
/// from (`canTakeItemThroughFace`); and the vanilla menu that shows its
/// slots when the block is used, if any (`MOD_MENUS`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModContainer {
    pub slots: usize,
    pub faces: [Vec<usize>; 6],
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub menu: Option<String>,
}
/// The fluid tanks of a mod block's block entity, in the container's shape:
/// each tank's capacity in millibuckets, the tanks each face reaches, and
/// the tanks a fill may reach and a drain may take. The tanks' contents are
/// not here: they live in the entity's saved compound, under `fluids`, so
/// they survive restarts and the block's own tick reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModFluidTanks {
    /// Each tank's capacity in millibuckets, in tank order.
    pub tanks: Vec<i32>,
    pub faces: [Vec<usize>; 6],
    /// The tanks a fill may reach (`canPlaceItemThroughFace`).
    pub inputs: Vec<usize>,
    /// The tanks a drain may take from (`canTakeItemThroughFace`).
    pub outputs: Vec<usize>,
}
/// The energy buffer of a mod block's block entity (NeoForge's
/// `SimpleEnergyHandler`): its capacity, the most one insert and one extract
/// moves, and the faces a fill may come through and a drain may leave by.
/// The amount is not here: it lives in the entity's saved compound, under
/// `energy`, so it survives restarts and the block's own tick reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModEnergyBuffer {
    pub capacity: i32,
    /// The most one insert moves; 0 takes none.
    pub max_insert: i32,
    /// The most one extract moves; 0 gives none.
    pub max_extract: i32,
    /// The faces, by `Direction` number, a fill may come through.
    pub insert_faces: [bool; 6],
    /// The faces a drain may leave by.
    pub extract_faces: [bool; 6],
}
/// The vanilla menus a mod container can open, with the slot count each
/// shows: slot grids, and the furnaces' (input, fuel, result), whose four
/// data slots draw the flame (`data[0]` of `data[1]`) and the arrow
/// (`data[2]` of `data[3]`), as `AbstractFurnaceBlockEntity.dataAccess`.
pub const MOD_MENUS: [(&str, usize); 11] = [
    ("minecraft:furnace", 3),
    ("minecraft:blast_furnace", 3),
    ("minecraft:smoker", 3),
    ("minecraft:hopper", 5),
    ("minecraft:generic_3x3", 9),
    ("minecraft:generic_9x1", 9),
    ("minecraft:generic_9x2", 18),
    ("minecraft:generic_9x3", 27),
    ("minecraft:generic_9x4", 36),
    ("minecraft:generic_9x5", 45),
    ("minecraft:generic_9x6", 54),
];
/// `PotentSulfurBlockEntity`: the seconds left to the geyser's next change of
/// state (`waitingCountdown`, saved as `countdown`) and the game time of its
/// last eruption block event, which the client's plume counts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PotentSulfur {
    pub waiting_countdown: i32,
    pub eruption_tick: i64,
}
impl Default for PotentSulfur {
    fn default() -> Self {
        Self {
            waiting_countdown: -1,
            eruption_tick: -1,
        }
    }
}
/// `BrushableBlockEntity`: the loot table is resolved on the first brush.
#[derive(Clone, Debug, Default)]
pub struct Brushable {
    pub loot_table: Option<horizon_world::nbt::Tag>,
    pub loot_seed: Option<horizon_world::nbt::Tag>,
    pub item: Option<ItemStack>,
    /// Direction's legacy network id; not saved to disk by vanilla.
    pub hit_direction: Option<i32>,
}
/// `ShelfBlockEntity`'s displayed slots and alignment flag.
#[derive(Clone, Debug, Default)]
pub struct Shelf {
    pub items: [Option<ItemStack>; 3],
    pub align_to_bottom: bool,
}
/// `ChiseledBookShelfBlockEntity`: six slots and `lastInteractedSlot`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChiseledBookshelf {
    pub items: [Option<ItemStack>; 6],
    pub last_slot: i32,
}
impl Default for ChiseledBookshelf {
    fn default() -> Self {
        Self {
            items: Default::default(),
            last_slot: -1,
        }
    }
}
/// `TheEndGatewayBlockEntity`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndGateway {
    pub age: i64,
    pub exit_portal: Option<[i32; 3]>,
    pub exact_teleport: bool,
}
/// `CampfireBlockEntity`: four cooking slots, each with its
/// `cookingProgress` and `cookingTime` (the recipe's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Campfire {
    pub items: [Option<ItemStack>; 4],
    pub progress: [i32; 4],
    pub total: [i32; 4],
}
/// `SkullBlockEntity`: the player profile the head shows (kept as loaded:
/// a compound, or a string when it names a player only), the note block
/// sound and `custom_name`.
#[derive(Clone, Debug, Default)]
pub struct Skull {
    pub profile: Option<horizon_world::nbt::Tag>,
    pub note_block_sound: Option<String>,
    pub custom_name: Option<horizon_world::nbt::Tag>,
    /// What `profile` draws, read from it once rather than every frame.
    pub skin: Option<SkullSkin>,
}
/// `SkullBlockRenderer.getPlayerSkin` of a head's profile: its skin's URL
/// with the `skins/<sha1>` sprite it downloads to, and the
/// `DefaultPlayerSkin` drawn until that sprite is there (steve for a
/// profile with no id).
#[derive(Clone, Debug)]
pub struct SkullSkin {
    pub download: Option<(String, String)>,
    /// `ResolvableProfile.nameOrId` when the profile is `Dynamic` (a name or
    /// an id only): the key `skins::Resolver` resolves it by.
    pub dynamic: Option<NameOrId>,
    /// `PlayerSkin.Patch.body`: the profile's `texture`, a resource pack
    /// texture drawn instead of any skin.
    pub texture: Option<String>,
    pub fallback: &'static str,
}
/// `Either<String, UUID>`: the key a `Dynamic` `ResolvableProfile` resolves
/// by (`ResolvableProfile.nameOrId`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NameOrId {
    Name(String),
    Id(u128),
}
/// `CrafterBlockEntity`: nine slots that can be disabled, the powered flag
/// and the ticks left in the crafting animation.
#[derive(Clone, Debug, Default)]
pub struct Crafter {
    pub items: [Option<ItemStack>; 9],
    pub disabled: [bool; 9],
    pub triggered: bool,
    pub crafting_ticks: i32,
    /// The block state's `triggered` when last ticked (not saved): the block
    /// entity flag follows it only when it changes (`CrafterBlock.neighborChanged`).
    pub state_triggered: Option<bool>,
}
#[derive(Clone, Debug, Default)]
pub struct Banner {
    pub layers: Vec<BannerLayer>,
    pub name: Option<String>,
    /// Components not consumed by BannerBlockEntity's implicit fields.
    pub components: horizon_world::nbt::Compound,
}
/// `CreakingHeartBlockEntity` (the logic is in `game/creaking.rs`).
#[derive(Clone, Debug, Default)]
pub struct CreakingHeart {
    /// The protector: its entity id once resolved, and its UUID (the saved form).
    pub creaking: Option<(Option<u32>, [i32; 4])>,
    pub ticks_existed: i64,
    pub ticker: i32,
    /// Ticks left of the hurt effect (`emitter`) and where it heads.
    pub emitter: i32,
    pub emitter_target: Option<[f64; 3]>,
    pub output_signal: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Lectern {
    pub book: Option<ItemStack>,
    pub page: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Beacon {
    pub levels: i32,
    /// Effect ids (`minecraft:speed`).
    pub primary: Option<String>,
    pub secondary: Option<String>,
    /// Beam sections (RGB colour, height) and the ones being rebuilt.
    pub sections: Vec<(u32, i32)>,
    pub checking: Vec<(u32, i32)>,
    pub last_check_y: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Jukebox {
    /// `JukeboxBlockEntity`'s one slot (`ContainerSingleItem`), so a hopper or
    /// a dropper reaches it as it reaches any container.
    pub items: [Option<ItemStack>; 1],
    /// `ticks_since_song_started`, while a song plays.
    pub ticks: i64,
    pub playing: bool,
}
impl Jukebox {
    /// `JukeboxBlockEntity.getTheItem`: the disc it holds.
    pub fn record(&self) -> Option<&ItemStack> {
        self.items[0].as_ref()
    }
}
/// `BrewingStandBlockEntity` state. Slots: 0-2 bottles, 3 ingredient, 4 fuel.
#[derive(Clone, Debug, Default)]
pub struct BrewingStand {
    pub items: [Option<ItemStack>; 5],
    pub brew_time: i32,
    pub total_brew_time: i32,
    pub fuel: i32,
    pub total_fuel: i32,
    pub speed: f32,
    pub ingredient: Option<Arc<str>>,
    pub last_bottles: Option<[bool; 3]>,
}
/// One sign side (`SignText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignText {
    pub lines: [String; 4],
    /// `DyeColor` id.
    pub color: u8,
    pub glowing: bool,
    /// The `click_event` in each line's component style, as saved.
    pub click: [Option<Tag>; 4],
}
/// `SignBlockEntity` / `HangingSignBlockEntity` data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignData {
    pub hanging: bool,
    pub front: SignText,
    pub back: SignText,
    pub waxed: bool,
    /// `allow_op_features`: the sign's click events may run commands.
    pub allow_op_features: bool,
}
/// `VibrationInfo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub event: &'static str,
    pub distance: f32,
    pub pos: [f64; 3],
    pub source: Option<Who>,
    pub owner: Option<Who>,
}
/// `VibrationSelector`: the closest, then highest-frequency candidate of a tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    pub current: Option<(Info, i64)>,
}
/// `VibrationSystem.Data`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub current: Option<Info>,
    pub travel: i32,
    pub selector: Selector,
}
/// Which sculk block entity this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SculkKind {
    Sensor,
    Calibrated,
    Shrieker,
    Catalyst,
}
/// `SculkSensorBlockEntity`, `CalibratedSculkSensorBlockEntity`,
/// `SculkShriekerBlockEntity` and `SculkCatalystBlockEntity`.
#[derive(Clone, Debug)]
pub struct SculkEntity {
    pub kind: SculkKind,
    pub vibration: Data,
    /// `last_vibration_frequency` (sensors).
    pub last_frequency: i32,
    /// `warning_level` (shriekers).
    pub warning_level: i32,
    /// The catalyst's charge cursors.
    pub spreader: SculkSpreader,
}
/// `SpawnData.CustomSpawnRules`: the block and sky light the spawn block
/// must have (inclusive ranges within 0..=15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustomSpawnRules {
    pub block: (i32, i32),
    pub sky: (i32, i32),
}
/// `SpawnData`: the entity to spawn, optional light rules and an equipment
/// table (kept as loaded).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnData {
    pub entity: Compound,
    pub custom_spawn_rules: Option<CustomSpawnRules>,
    pub equipment: Option<Tag>,
}
/// Which `SpawnData` is next: one of the potentials (the same object in
/// vanilla, so changing it changes the potential) or its own.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    Potential(usize),
    Own(SpawnData),
}
/// `BaseSpawner`.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawner {
    pub spawn_delay: i32,
    /// `SpawnPotentials`: data and weight.
    pub potentials: Vec<(SpawnData, i32)>,
    pub next: Option<Next>,
    pub min_spawn_delay: i32,
    pub max_spawn_delay: i32,
    pub spawn_count: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
    pub spawn_range: i32,
    /// The client copy's delay (`spawnDelay` on the client level), which
    /// speeds the spin up as the next spawn nears.
    pub client_delay: i32,
    pub spin: f64,
    pub o_spin: f64,
}
impl Default for Spawner {
    fn default() -> Self {
        Self {
            spawn_delay: 20,
            potentials: Vec::new(),
            next: None,
            min_spawn_delay: 200,
            max_spawn_delay: 800,
            spawn_count: 4,
            max_nearby_entities: 6,
            required_player_range: 16,
            spawn_range: 4,
            client_delay: 20,
            spin: 0.0,
            o_spin: 0.0,
        }
    }
}
/// `TrialSpawnerConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrialConfig {
    pub spawn_range: i32,
    pub total_mobs: f32,
    pub simultaneous_mobs: f32,
    pub total_mobs_added_per_player: f32,
    pub simultaneous_mobs_added_per_player: f32,
    pub ticks_between_spawn: i32,
    pub spawn_potentials: Vec<(SpawnData, i32)>,
    pub loot_tables_to_eject: Vec<(String, i32)>,
    pub items_to_drop_when_ominous: String,
}
impl Default for TrialConfig {
    /// `TrialSpawnerConfig.Builder`'s defaults.
    fn default() -> Self {
        Self {
            spawn_range: 4,
            total_mobs: 6.0,
            simultaneous_mobs: 2.0,
            total_mobs_added_per_player: 2.0,
            simultaneous_mobs_added_per_player: 1.0,
            ticks_between_spawn: 40,
            spawn_potentials: Vec::new(),
            loot_tables_to_eject: vec![
                ("minecraft:spawners/trial_chamber/consumables".into(), 1),
                ("minecraft:spawners/trial_chamber/key".into(), 1),
            ],
            items_to_drop_when_ominous:
                "minecraft:spawners/trial_chamber/items_to_drop_when_ominous".into(),
        }
    }
}
/// A `Holder<TrialSpawnerConfig>` as saved: a registry id
/// (`minecraft:trial_chamber/breeze/normal`) or an inline config.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigSlot {
    /// The value as loaded, written back as is; None for the default.
    pub saved: Option<Tag>,
    pub config: TrialConfig,
}
/// `TrialSpawner` with its `TrialSpawnerStateData`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrialSpawner {
    pub normal: ConfigSlot,
    pub ominous: ConfigSlot,
    pub target_cooldown_length: i32,
    pub required_player_range: i32,
    pub detected_players: Vec<Uuid>,
    pub current_mobs: Vec<Uuid>,
    pub cooldown_ends_at: i64,
    pub next_mob_spawns_at: i64,
    pub total_mobs_spawned: i32,
    pub next_spawn_data: Option<SpawnData>,
    pub ejecting_loot_table: Option<String>,
    /// `isOminous`, set from the block on every server tick.
    pub is_ominous: bool,
    /// The server's display entity exists (`getOrCreateDisplayEntity`): it is
    /// made once per loaded block entity and kept.
    pub has_display: bool,
    pub spin: f64,
    pub o_spin: f64,
    /// `getDispensingItems`, kept once rolled (not saved): the ominous items
    /// and their weights (the rolled counts).
    pub dispensing: Option<Vec<ItemStack>>,
}
impl Default for TrialSpawner {
    fn default() -> Self {
        Self {
            normal: ConfigSlot::default(),
            ominous: ConfigSlot::default(),
            target_cooldown_length: 36000,
            required_player_range: 14,
            detected_players: Vec::new(),
            current_mobs: Vec::new(),
            cooldown_ends_at: 0,
            next_mob_spawns_at: 0,
            total_mobs_spawned: 0,
            next_spawn_data: None,
            ejecting_loot_table: None,
            is_ominous: false,
            has_display: false,
            spin: 0.0,
            o_spin: 0.0,
            dispensing: None,
        }
    }
}
/// `VaultConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct VaultConfig {
    pub loot_table: String,
    pub activation_range: f64,
    pub deactivation_range: f64,
    /// None when the config has no key (`ItemStack.EMPTY`): the vault never opens.
    pub key_item: Option<ItemStack>,
    pub override_loot_table_to_display: Option<String>,
}
/// `VaultBlockEntity`: its config, server data and shared data.
#[derive(Clone, Debug, PartialEq)]
pub struct Vault {
    pub config: VaultConfig,
    /// `VaultServerData`, oldest rewarded player first.
    pub rewarded_players: Vec<Uuid>,
    pub state_updating_resumes_at: i64,
    /// Popped from the end.
    pub items_to_eject: Vec<ItemStack>,
    pub total_ejections_needed: i32,
    pub last_insert_fail_timestamp: i64,
    /// `VaultSharedData`.
    pub display_item: Option<ItemStack>,
    pub connected_players: Vec<Uuid>,
    pub connected_particles_range: f64,
    /// The shared data changed since the last tick sent it.
    pub shared_dirty: bool,
    /// `VaultClientData`: the display item's spin and the previous tick's, in degrees.
    pub spin: f32,
    pub o_spin: f32,
}
/// `BeehiveBlockEntity`: the bees inside and the flower one of them knew.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beehive {
    pub bees: Vec<Occupant>,
    pub flower: Option<[i32; 3]>,
}
/// `BeehiveBlockEntity.Occupant`: a bee's tag (with its `id`), its time in
/// the hive and the time it stays at least.
#[derive(Clone, Debug, PartialEq)]
pub struct Occupant {
    pub entity_data: Tag,
    pub ticks_in_hive: i32,
    pub min_ticks_in_hive: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBlock {
    pub command: String,
    pub success_count: i32,
    /// `CustomName` as saved (a text component).
    pub custom_name: Option<Tag>,
    pub track_output: bool,
    pub last_output: Option<Text>,
    pub update_last_execution: bool,
    /// The game time of the last run, -1 for none.
    pub last_execution: i64,
    pub powered: bool,
    pub auto: bool,
    pub condition_met: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_block_entitys_revision_moves_with_each_change_and_only_then() {
        let mut machine = ModBlockEntity::new(ModBlockEntityFields {
            id: "mymod:machine".into(),
            data: vec![0],
            ..Default::default()
        });
        let made = machine.revision();
        // Reads leave it.
        assert_eq!(machine.data[0] + machine.nbt.len() as i32, 0);
        assert_eq!(machine.revision(), made);
        // Every way to change a field moves it.
        let mut seen = vec![made];
        let changes: [fn(&mut ModBlockEntity); 4] = [
            |m| m.data[0] = 1,
            |m| _ = m.nbt.insert("progress".into(), Tag::Int(3)),
            |m| m.items.push(None),
            |m| m.id.push('s'),
        ];
        for change in changes {
            change(&mut machine);
            assert!(!seen.contains(&machine.revision()));
            seen.push(machine.revision());
        }
        // Another entity, or a clone, never shares one, even with equal
        // contents; equality is the contents' alone.
        let copy = machine.clone();
        assert_eq!(copy, machine);
        assert!(!seen.contains(&copy.revision()));
        let fresh = ModBlockEntity::default();
        assert_ne!(fresh.revision(), ModBlockEntity::default().revision());
    }

    #[test]
    fn an_update_tag_sent_again_unchanged_keeps_the_revision() {
        let mut machine = ModBlockEntity::default();
        let tag = Compound::from([
            ("progress".into(), Tag::Int(3)),
            ("fuel".into(), Tag::Int(9)),
        ]);
        machine.merge_update_tag(&tag);
        let merged = machine.revision();
        machine.merge_update_tag(&tag);
        assert_eq!(machine.revision(), merged);
        // One field changed: replaced, the other kept, the revision moved.
        machine.merge_update_tag(&Compound::from([("progress".into(), Tag::Int(4))]));
        assert_ne!(machine.revision(), merged);
        assert_eq!(
            machine.nbt,
            Compound::from([
                ("progress".into(), Tag::Int(4)),
                ("fuel".into(), Tag::Int(9)),
            ])
        );
    }
}
