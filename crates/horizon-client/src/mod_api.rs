//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Trait mods are process-wide, like mixin module statics.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
use std::sync::{Arc, OnceLock};

pub use crate::containers::{ModBlockEntity, ModContainer, ModFluidTanks};
pub use crate::crafting::{CustomRecipe, Ingredient};
pub use crate::extras::Corner;
pub use crate::extras_hud::Corners;
pub use crate::game::native_block_entity_draw::{BlockEntityDraw, CubeFace};
pub use crate::gfx::native_render::{
    RenderHooks, RenderNeeds, RenderNeedsEvent, WorldDrawn, WorldDrawnEvent, wgpu,
};
pub use crate::hud::HudBatch;
pub use crate::hud_layout::{GuiDraw, HealthInputs};
pub use crate::items::ItemStack;
pub use crate::text::Text;
pub use horizon_assets::item_model::DisplayContext;
pub use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
pub use horizon_world::{
    attachments::{
        AttachmentCodec, AttachmentError, AttachmentHost, AttachmentType, Attachments, Holder,
    },
    fluid::FluidState,
    level::{BlockId, Dimension as LevelDimension, PlacementContext},
    nbt::{Compound, Tag},
    registry::BlockStateId,
};
pub use serde_json;
use serde_json::{Map, Value};
use std::time::Duration;

/// A dispenser hook's answer: the slot's remainder (None empties it) and the
/// level event to play, `CLICK` (1000), `FAIL` (1001) or 0 for none.
pub type NativeDispensed = (Option<ItemStack>, i32);

/// What dropped the items heard by the `loot_drops` hook.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeLootDrops<'a> {
    /// The block's or mob's own loot table (`minecraft:blocks/stone`, `minecraft:entities/zombie`);
    /// empty when none rolled: the block or mob has none, the `mob_drops` rule is off, a baby
    /// animal. A mob's drops may hold more: a charged creeper's head, a fox's mouth item.
    pub table: &'a str,
    /// The level they drop in (`minecraft:overworld`).
    pub dimension: &'a str,
    /// `LootContextParams.ORIGIN`: a broken block's centre, a mob's position.
    pub origin: [f64; 3],
    /// The block broken: its position and state.
    pub block: Option<([i32; 3], BlockStateId)>,
    /// The mob that died, or the entity a player's use dropped them from (the
    /// sheep sheared, the item frame broken): its entity ID and type
    /// (`minecraft:zombie`).
    pub entity: Option<(u32, &'a str)>,
    /// The player who broke the block or killed the mob, when known. A mob
    /// names one only while it remembers the player's hit (100 ticks), so one
    /// that dies to lava long after names none.
    pub player: Option<u128>,
    /// What broke the block (`LootContextParams.TOOL`).
    pub tool: Option<&'a ItemStack>,
    /// The radius of the explosion that broke the block.
    pub explosion: Option<f32>,
    /// A player's use dropped them rather than a break or a death: shears on a
    /// sheep, a mooshroom, a snow golem or a bogged, a brush on an armadillo or
    /// a suspicious block, shears carving a pumpkin, picking sweet berries or
    /// cave vines, a hit taking an item frame or its item, or untying a lead
    /// or cutting it with shears. `player` names them; `block` or `entity` says
    /// what dropped them. API 100. A mob that a player killed while
    /// leashed drops its lead on its next tick as a call of its own (`entity`
    /// the mob, `player` its killer, `table` empty, `used` false). API
    /// 102.
    pub used: bool,
}

/// The open screen, for the screen hooks (`native_screen_render_post`, the
/// mouse ones `native_screen_mouse_button_pressed_pre`, `..._released_pre` and
/// `native_screen_mouse_scrolled_pre`, and the key ones
/// `native_screen_key_pressed_pre` and `native_screen_char_typed_pre`): what
/// it is, its menu and title, where its container panel is, how many slots its
/// menu has, and where the mouse and the frame's partial tick are.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeScreenView<'a> {
    /// One id per `Screen` (`Screen::id`): `inventory`, `creative`, `paused`,
    /// `stats`, `options`, `win`… Stable across releases, so a mod can match
    /// on it. Every container, station and mod window draws on the inventory
    /// screen and so reports `inventory`: tell them apart by `menu` and
    /// `title` (both None on a window the client opened itself) and by
    /// `slots`.
    pub id: &'static str,
    /// The type id of the menu the server opened for this screen
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, a mod's own id; a mount's
    /// inventory is `horizon:mount<n>`). None on a screen without one (the
    /// client's own inventory, creative, the pause menu, stats…) and for a
    /// menu type the client does not know.
    pub menu: Option<&'a str>,
    /// The title the server gave the open menu, if it has one: the component
    /// itself, not a copy (`.plain()` it to draw, `chars()` for the styles).
    /// None when the server gave none, on a screen the client opened, and
    /// while playing.
    pub title: Option<&'a Text>,
    /// The screen's container panel in GUI pixels, `[x, y, width, height]`;
    /// the whole window for a screen with none. The panel is where the game
    /// draws the window: 176 wide for most, wider for the merchant and the
    /// beacon, a mod screen's own size for a mod's.
    pub rect: [f32; 4],
    /// The slots the open menu has, as `Game::slots` counts them: the player
    /// inventory's 36, then the container's, the station's, the mod screen's
    /// or the crafting grid's (a screen with no menu of its own still counts
    /// the inventory menu's). Counted, not listed.
    pub slots: usize,
    /// The pointer in GUI pixels (`Screen.mouseX`, `mouseY`, from the
    /// window's top left), as `native_cursor` gives it: for hover highlights.
    pub mouse: [f32; 2],
    /// This frame's partial tick (`getGameTimeDeltaPartialTick(false)`, 0 to
    /// 1): what the screen draws animation against.
    pub partial_tick: f32,
}

/// A depth-tested outline of the client's block selection shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeBlockRim {
    pub pos: [i32; 3],
    pub argb: u32,
    pub width: f32,
}

/// A camera-local visual light, with no gameplay light or shadow writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeColoredLight {
    pub at: [f64; 3],
    pub rgb: u32,
    pub radius: f32,
    pub intensity: f32,
}

/// The minimap a native mod shows (`native_minimap`), drawn by Horizon's
/// minimap: its corner, its side in GUI pixels, blocks per map pixel,
/// turning with the player or north up, round or square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeMinimap {
    pub corner: Corner,
    pub size: f32,
    pub blocks_per_pixel: f32,
    pub rotate: bool,
    pub round: bool,
}

/// A mod block's screen over its menu (`native_screen`): its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`; none hides the
/// slot, and so do slots past the list; the server's menu does not have a
/// hidden slot either, so clicks and shift-clicks skip it), and the player inventory's first
/// row (vanilla's is 84; the hotbar is 58 below it) and left column, from
/// its top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeScreen {
    /// Vanilla's container screens are 176 wide.
    pub width: f32,
    pub height: f32,
    pub slots: Vec<Option<[f32; 2]>>,
    pub inventory_y: f32,
    /// The inventory's left column (vanilla's is 8).
    pub inventory_x: f32,
    /// The mod draws the title and the "Inventory" label itself
    /// (`native_screen_draw`); Horizon leaves them out.
    pub own_labels: bool,
}

/// A native mod as Options > Mods lists it (`native_mods`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeMod {
    /// Its id: its settings folder and its keys' lines.
    pub id: String,
    /// `Name 1.0`.
    pub title: String,
    pub description: String,
    /// Its keys: name, label and default key (`key.keyboard.z`). The page
    /// lists them first; `native_mod_key` reads the player's choice.
    pub keys: Vec<[String; 3]>,
}

/// An item the panel shows (`native_panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeStack {
    pub item: String,
    pub count: u32,
    pub components: String,
}

/// What the player did on a native mod's panel (`native_panel_event`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativePanelEvent {
    /// A click on an item of the panel, by its index in
    /// `native_panel_items`' list.
    Clicked { index: u32, right: bool },
    /// The search box's text, each time the player changes it.
    Search(String),
    /// A key (`key.keyboard.r`; compare it with `native_mod_key`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key { key: String, stack: NativeStack },
    /// A click on a slot of a recipe the panel shows: the recipe's index in
    /// `native_panel_recipes`' list and the slot's in it (the results come
    /// after the slots), and the stack it showed then.
    SlotClicked {
        recipe: u32,
        slot: u32,
        stack: NativeStack,
        right: bool,
    },
    /// The action button of the recipe at this index.
    Action(u32),
    /// The player turned the recipes' page: the index of the first recipe
    /// now shown.
    Page(u32),
    /// The recipe page's back button.
    Back,
    /// The player picked the tab at this index.
    Tab(u32),
}

/// The zoom a native mod asks for (`native_zoom`): the factor (1 is none), eased through the
/// field of view like the spyglass when `smooth`, and the scale of mouse turning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeZoom {
    pub factor: f32,
    pub smooth: bool,
    pub turn: f32,
}

/// A player on the server, as a server hook sees them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeServerPlayer {
    pub uuid: u128,
    pub name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: String,
    pub position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Health points, 20 at full; 0 once dead. API 21.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server. API 49.
    pub yaw: f32,
    pub pitch: f32,
}

/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`,
/// `item`, `block_pos`. Usages of one command share their words; the
/// server merges them into one tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeCommand {
    pub usage: String,
    /// The operator level a player needs to see and run it (0: everyone).
    pub level: u8,
}

impl NativeCommand {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
        }
    }
}

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeAuthority {
    LocalAuthoritative,
    IntegratedServer,
    DedicatedServer,
    RemoteClient,
}

/// One open attempt of one world; it rejects a previous open's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWorldEpoch(u64);

impl NativeWorldEpoch {
    /// Create an epoch for the game.
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the epoch number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTickId(u64);

impl NativeTickId {
    /// Create a tick ID for the game.
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the tick number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The world phases (`native_world_lifecycle`'s `phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The phase names are the hook log's stable vocabulary (the Debug text
/// the mixins write); keep them despite the shared prefix.
#[allow(clippy::enum_variant_names)]
#[non_exhaustive]
pub enum WorldPhase {
    WorldOpenReady,
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    WorldCloseFinal,
    /// The server loop or final save failed; `error_text` carries the error.
    WorldCloseFailed,
}

/// The tick phases carry whether the world runs, including frozen steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerTickPhase {
    ServerTickStart { world_running: bool },
    ServerTickEnd { world_running: bool },
}

/// Context for every world lifecycle phase.
/// `authority` is the server role. LocalAuthoritative and RemoteClient never
/// fire today. `epoch` identifies the open attempt, including failed opens;
/// the same epoch identifies its ready, final-close, or failed-close event.
/// `tick` counts non-paused server ticks since the world opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeLifecycleContext {
    pub authority: NativeAuthority,
    pub epoch: NativeWorldEpoch,
    pub tick: NativeTickId,
}

/// The ordered list of mods shared by the client and server.
pub type Mods = Arc<[Box<dyn Hooks>]>;

pub(crate) static MODS: OnceLock<Mods> = OnceLock::new();

/// The process-wide list in dispatch order, or an empty list before installation.
pub fn mods() -> &'static [Box<dyn Hooks>] {
    MODS.get().map_or(&[], |mods| mods.as_ref())
}

/// Whether a mod claims a cancellable hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Later mods and the game continue.
    Continue,
    /// Set the hook flag and skip later mods.
    Cancel,
}

mod generated_host;
pub use generated_host::Host;

/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

impl AttachmentHost for dyn Host + '_ {
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError> {
        Host::attachments(self, holder)
    }
}

/// Read-only level services, like vanilla's `LevelReader`.
pub trait LevelHost {
    /// Read a block through `LevelAccess::block_state`.
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId;
    /// `Level.isClientSide`, through `LevelAccess::is_client_side`.
    fn is_client_side(&self) -> bool;
    /// `BlockState.setValue`, through `native_state_with_property`; None if invalid.
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId>;
}

/// Access vanilla's `Level.random`, so draws come in vanilla's order.
pub trait LevelRandomHost: LevelHost {
    /// Access a holder's server-owned attachment map.
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError>;
    /// The level's random stream through `native_level_random`, or None on a non-authoritative level.
    fn random(&mut self) -> Option<&mut LegacyRandom>;
}

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool;
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool;
}

/// Mutable level services, like vanilla's `ServerLevel`. Whole-block writes wait for block entity support.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Change a block property through `native_set_block_property`.
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool;
}

macro_rules! level_attachment_host {
    ($($host:ident),+) => { $(
        impl AttachmentHost for dyn $host + '_ {
            fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError> {
                LevelRandomHost::attachments(self, holder)
            }
        }
    )+ };
}
level_attachment_host!(LevelRandomHost, LevelWriteHost);

mod generated;
pub use generated::Hooks;
#[doc(hidden)]
pub use generated::*;
