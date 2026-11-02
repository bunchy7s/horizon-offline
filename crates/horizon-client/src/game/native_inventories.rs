//! Another player's inventories for server mods (API 132):
//! what an operator's `/invsee` or `/endersee` needs. A player online is read
//! and changed live; a player who isn't on the server is read from, and
//! written to, their file (`players/data/<uuid>.dat`, as the server saves
//! it), never while they are logging in, dead or swapped in. One stack moves
//! at a time, checked against what the mod last showed, so a menu built on
//! these never copies or loses an item. Server menus are in `native_menus.rs`;
//! maps a mod draws in `native_send_map` below.
use super::Game;
use horizon_client::{
    items::ItemStack,
    lan::{guest::Guest, playerdata},
    save::{Level, read_dat, write_dat},
};
use std::path::Path;

/// Which of a player's inventories (`native_inventory`).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeInventoryKind {
    /// 41 slots: 0..9 the hotbar, 9..36 the rest (as `Inventory`), 36..40
    /// the armour (feet, legs, chest, head), 40 the off hand.
    Player,
    /// The ender chest's 27 slots.
    Ender,
}

#[allow(dead_code)]
impl NativeInventoryKind {
    pub(crate) fn size(self) -> usize {
        match self {
            Self::Player => 41,
            Self::Ender => 27,
        }
    }
}

/// A player's inventory as `native_inventory` read it: its slots, and
/// whether it is the live one (the player is on the server) or their file's.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeInventory {
    pub slots: Vec<Option<ItemStack>>,
    pub online: bool,
}

/// `Level.inventory`'s slot numbers (the file's) for our 41.
fn file_slot(i: usize) -> Option<u8> {
    match i {
        0..36 => Some(i as u8),
        36..40 => Some(100 + (i - 36) as u8),
        40 => Some(150),
        _ => None,
    }
}

fn guest_slots(g: &Guest, kind: NativeInventoryKind) -> Vec<Option<ItemStack>> {
    match kind {
        NativeInventoryKind::Player => {
            let mut s = g.inventory.clone();
            s.resize(36, None);
            // `armor` is head first.
            s.extend([3, 2, 1, 0].map(|i| g.armor[i].clone()));
            s.push(g.offhand.clone());
            s
        }
        NativeInventoryKind::Ender => {
            let mut s = g.ender_items.clone();
            s.resize(27, None);
            s
        }
    }
}

fn set_guest_slots(g: &mut Guest, kind: NativeInventoryKind, mut s: Vec<Option<ItemStack>>) {
    match kind {
        NativeInventoryKind::Player => {
            g.offhand = s.pop().flatten();
            for (k, i) in [3, 2, 1, 0].into_iter().enumerate() {
                g.armor[i] = s[36 + k].take();
            }
            s.truncate(36);
            g.inventory = s;
        }
        NativeInventoryKind::Ender => g.ender_items = s,
    }
}

fn level_slots(level: &Level, kind: NativeInventoryKind) -> Vec<Option<ItemStack>> {
    match kind {
        NativeInventoryKind::Player => {
            let mut s = vec![None; 41];
            for (slot, item, count, damage) in &level.inventory {
                if let Some(i) = (0..41).find(|&i| file_slot(i) == Some(*slot)) {
                    s[i] = Some(ItemStack {
                        item: item.as_str().into(),
                        count: (*count).max(1),
                        damage: *damage,
                        extra: level.inventory_components.get(slot).cloned(),
                    });
                }
            }
            s
        }
        NativeInventoryKind::Ender => {
            let mut s = level.ender_items.clone();
            s.resize(27, None);
            s
        }
    }
}

fn set_level_slots(level: &mut Level, kind: NativeInventoryKind, s: Vec<Option<ItemStack>>) {
    match kind {
        NativeInventoryKind::Player => {
            level.inventory.clear();
            level.inventory_components.clear();
            for (i, stack) in s.into_iter().enumerate() {
                if let (Some(slot), Some(st)) = (file_slot(i), stack) {
                    level
                        .inventory
                        .push((slot, st.item.to_string(), st.count, st.damage));
                    if let Some(extra) = st.extra {
                        level.inventory_components.insert(slot, extra);
                    }
                }
            }
        }
        NativeInventoryKind::Ender => level.ender_items = s,
    }
}

/// A saved player's file read as `playerdata::load` reads it, kept whole
/// (`player_extra`) so writing it back changes only what was edited.
fn read_level(world: &Path, uuid: u128) -> Option<Level> {
    let file = playerdata::path(world, uuid);
    let tag = read_dat(&file)?;
    let mut level = Level {
        health: 20.0,
        food: 20,
        saturation: 5.0,
        ..Level::default()
    };
    level.read_player(&tag);
    level.player_extra = Some(tag);
    Some(level)
}

/// Where one side of a move lives while it is made.
enum Side {
    /// A guest's index in the hub.
    Live(usize),
    /// A player's file, read.
    File(Box<Level>),
}

/// Another player's inventories, for server mods. API 132.
#[allow(dead_code)]
impl Game<'_> {
    /// The player `uuid`'s inventory `kind`: the live one when they are on
    /// the server, else their file's. Err while they are logging in, dead,
    /// swapped in (inside a player's hook: call from `native_menu_clicked` or
    /// `native_server_tick`), or when they have never played here.
    pub(super) fn native_inventory(
        &self,
        uuid: u128,
        kind: NativeInventoryKind,
    ) -> Result<NativeInventory, String> {
        match self.native_inventory_side(uuid)? {
            Side::Live(i) => {
                let host = self.lan.as_ref().expect("live");
                Ok(NativeInventory {
                    slots: guest_slots(&host.hub.guests[i], kind),
                    online: true,
                })
            }
            Side::File(level) => Ok(NativeInventory {
                slots: level_slots(&level, kind),
                online: false,
            }),
        }
    }

    fn native_inventory_side(&self, uuid: u128) -> Result<Side, String> {
        if self.lan_acting.is_some() {
            return Err("inside a player's turn".into());
        }
        let Some(host) = self.lan.as_ref() else {
            return Err("no server".into());
        };
        if let Some(i) = host.hub.guests.iter().position(|g| g.uuid == uuid) {
            let g = &host.hub.guests[i];
            return if !g.placed {
                Err("the player is logging in".into())
            } else if g.dead {
                Err("the player is dead".into())
            } else {
                Ok(Side::Live(i))
            };
        }
        if self.world_root.as_os_str().is_empty() {
            return Err("no world".into());
        }
        read_level(&self.world_root, uuid)
            .map(|l| Side::File(Box::new(l)))
            .ok_or_else(|| "no saved player".into())
    }

    /// Moves the stack in slot `from_slot` of `from`'s inventory `from_kind`,
    /// which must still be exactly `expected`, into `to`'s inventory
    /// `to_kind` where it fits whole (onto equal stacks first, then empty
    /// slots; for `Player`, the 36 main slots, never armour or off hand), or
    /// nothing moves and the Err says why. Each side is live or its file
    /// (`native_inventory`); at most one may be a file, which is written
    /// before the live side changes. Their screens follow with the tick's
    /// sync.
    pub(super) fn native_move_stack(
        &mut self,
        from: (u128, NativeInventoryKind, usize),
        expected: &ItemStack,
        to: (u128, NativeInventoryKind),
    ) -> Result<(), String> {
        let (from_uuid, from_kind, from_slot) = from;
        let (to_uuid, to_kind) = to;
        if from_uuid == to_uuid && from_kind == to_kind {
            return Err("the same inventory".into());
        }
        let a = self.native_inventory_side(from_uuid)?;
        let b = if from_uuid == to_uuid {
            None
        } else {
            Some(self.native_inventory_side(to_uuid)?)
        };
        if matches!(a, Side::File(_)) && matches!(b, Some(Side::File(_))) {
            return Err("both players are away".into());
        }
        let items = self.res.items();
        let max = |s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let read = |side: &Side, kind| match side {
            Side::Live(i) => guest_slots(&self.lan.as_ref().expect("live").hub.guests[*i], kind),
            Side::File(level) => level_slots(level, kind),
        };
        let mut src = read(&a, from_kind);
        let mut dst = read(b.as_ref().unwrap_or(&a), to_kind);
        match src.get_mut(from_slot) {
            Some(s) if s.as_ref() == Some(expected) => *s = None,
            _ => return Err("that stack changed".into()),
        }
        let room = match to_kind {
            NativeInventoryKind::Player => 36,
            NativeInventoryKind::Ender => 27,
        };
        if !super::native_menus::fit(&mut dst[..room], std::slice::from_ref(expected), &max) {
            return Err("no room".into());
        }
        // The file first: when it can't be written, nothing has moved.
        let world = self.world_root.clone();
        let write =
            |side: Side, kind, slots: Vec<Option<ItemStack>>, uuid| -> Result<Side, String> {
                if let Side::File(mut level) = side {
                    set_level_slots(&mut level, kind, slots);
                    write_dat(&playerdata::path(&world, uuid), level.player_tag())
                        .map_err(|e| format!("the player's file: {e}"))?;
                    return Ok(Side::File(level));
                }
                Ok(side)
            };
        let (a, b) = match b {
            None => {
                // Two inventories of one player: their file or live.
                let mut both = a;
                if let Side::File(level) = &mut both {
                    set_level_slots(level, from_kind, src.clone());
                }
                let both = write(both, to_kind, dst.clone(), from_uuid)?;
                (both, None)
            }
            Some(b) if matches!(b, Side::File(_)) => {
                let b = write(b, to_kind, dst.clone(), to_uuid)?;
                (a, Some(b))
            }
            Some(b) => {
                let a = write(a, from_kind, src.clone(), from_uuid)?;
                (a, Some(b))
            }
        };
        let host = self.lan.as_mut().expect("checked");
        if let Side::Live(i) = a {
            set_guest_slots(&mut host.hub.guests[i], from_kind, src);
            if b.is_none() {
                set_guest_slots(&mut host.hub.guests[i], to_kind, dst);
                return Ok(());
            }
        }
        if let Some(Side::Live(i)) = b {
            set_guest_slots(&mut host.hub.guests[i], to_kind, dst);
        }
        Ok(())
    }

    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    /// API 132.
    pub(super) fn native_send_map(&mut self, uuid: u128, map_id: i32, colors: &[u8]) -> bool {
        use horizon_protocol::packets::play::{Clientbound, MapItemData, MapPatch};
        if colors.len() != 128 * 128 {
            return false;
        }
        self.native_send(
            uuid,
            Clientbound::MapItemData(MapItemData {
                map_id,
                scale: 0,
                locked: true,
                decorations: None,
                patch: Some(MapPatch {
                    width: 128,
                    height: 128,
                    start_x: 0,
                    start_y: 0,
                    colors: colors.to_vec(),
                }),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn a_files_slots_map_to_the_41_and_back() {
        let mut level = Level::default();
        let mut s = vec![None; 41];
        s[0] = Some(stack("minecraft:stone", 5));
        s[35] = Some(stack("minecraft:dirt", 1));
        s[36] = Some(stack("minecraft:iron_boots", 1));
        s[39] = Some(stack("minecraft:iron_helmet", 1));
        s[40] = Some(stack("minecraft:shield", 1));
        set_level_slots(&mut level, NativeInventoryKind::Player, s.clone());
        let slots: Vec<u8> = level.inventory.iter().map(|e| e.0).collect();
        assert_eq!(slots, [0, 35, 100, 103, 150]);
        assert_eq!(level_slots(&level, NativeInventoryKind::Player), s);
    }

    #[test]
    fn a_saved_player_file_round_trips_its_inventory_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("native-inv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let uuid = 0x1234_u128;
        let file = playerdata::path(&dir, uuid);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut level = Level {
            health: 13.0,
            xp_level: 7,
            ..Level::default()
        };
        level.ender_items = vec![None; 27];
        level.ender_items[2] = Some(stack("minecraft:diamond", 3));
        let mut s = vec![None; 41];
        s[4] = Some(stack("minecraft:bread", 12));
        set_level_slots(&mut level, NativeInventoryKind::Player, s);
        write_dat(&file, level.player_tag()).unwrap();
        let mut back = read_level(&dir, uuid).unwrap();
        assert_eq!(back.health, 13.0);
        assert_eq!(back.xp_level, 7);
        assert_eq!(
            level_slots(&back, NativeInventoryKind::Ender)[2],
            Some(stack("minecraft:diamond", 3))
        );
        let mut inv = level_slots(&back, NativeInventoryKind::Player);
        assert_eq!(inv[4], Some(stack("minecraft:bread", 12)));
        inv[4] = None;
        set_level_slots(&mut back, NativeInventoryKind::Player, inv);
        write_dat(&file, back.player_tag()).unwrap();
        let again = read_level(&dir, uuid).unwrap();
        assert!(
            level_slots(&again, NativeInventoryKind::Player)
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(again.xp_level, 7);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
