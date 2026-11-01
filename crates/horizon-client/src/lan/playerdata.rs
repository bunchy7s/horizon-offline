//! Guests' saved state: `players/data/<uuid>.dat` in the vanilla player
//! compound, through the same code that saves the host.
use super::guest::{Guest, SavedBody};
use crate::{
    items::ItemStack,
    save::{Level, put, read_dat, uuid_from_tag, write_dat},
};
use horizon_protocol::crypto::uuid_string;
use horizon_world::nbt::Tag;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn path(world: &Path, uuid: u128) -> PathBuf {
    world
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_string(uuid)))
}

/// `UUIDUtil.uuidToIntArray`.
pub fn uuid_ints(uuid: u128) -> [i32; 4] {
    [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ]
}
/// `UUIDUtil.uuidFromIntArray`.
pub fn ints_uuid(ints: [i32; 4]) -> u128 {
    ints.iter()
        .fold(0, |acc, &i| acc << 32 | u128::from(i as u32))
}

/// Whether a saved file exists for this player.
pub fn exists(world: &Path, uuid: u128) -> bool {
    path(world, uuid).is_file()
}

/// `PlayerDataStorage.load`: a guest's saved state, if they have been here
/// before. The player's file, else its `.dat_old` (what a save cut off
/// between its two renames leaves, `write_dat`).
pub fn load(guest: &mut Guest, world: &Path) -> bool {
    let file = path(world, guest.uuid);
    let Some(tag) = read_dat(&file).or_else(|| read_dat(&file.with_extension("dat_old"))) else {
        return false;
    };
    let mut level = Level {
        health: 20.0,
        food: 20,
        saturation: 5.0,
        ..Level::default()
    };
    level.read_player(&tag);
    level.player_extra = Some(tag);
    apply(guest, &level);
    true
}

/// `PlayerList.loadPlayerData` for the world's single-player owner: the file
/// level.dat names (`singleplayer_uuid`), as the world opening read it into
/// `saved`, before the owner's own file. Builds that save the owner to the
/// named file alone (0.2.93) leave the own file behind, stale (#5351). The
/// own file is for a world whose named file is missing; then the player the
/// world opening found instead (the newest player file, the legacy
/// `Data.Player`, or nothing in a new world). Returns whether the player has
/// been in this world before.
pub fn load_owner(guest: &mut Guest, world: &Path, saved: Option<&Level>) -> bool {
    if let Some(level) = saved.filter(|l| l.singleplayer_file) {
        apply(guest, level);
        return level.position.is_some();
    }
    if load(guest, world) {
        return true;
    }
    saved.is_some_and(|level| {
        apply(guest, level);
        level.position.is_some()
    })
}

/// Save the world's single-player owner: its state into `level` (`store`),
/// then to the file level.dat names, which it loads ([`load_owner`]), and to
/// its own file, so that the one it loads is never the older of the two.
pub fn save_owner(guest: &Guest, level: &mut Level, world: &Path) {
    store(guest, level, None);
    let _ = level.write_player(world);
    save(guest, world);
}

/// Give a guest the player state read into `level`.
pub fn apply(guest: &mut Guest, level: &Level) {
    if let Some(p) = level.position {
        guest.position = p;
        guest.previous_position = p;
    }
    guest.yaw = level.rotation[0];
    guest.pitch = level.rotation[1];
    guest.head_yaw = guest.yaw;
    guest.on_ground = level.on_ground;
    guest.health = level.health;
    guest.saved_absorption = Some(level.absorption);
    guest.food = level.food;
    guest.saturation = level.saturation;
    guest.xp_level = level.xp_level;
    guest.xp_progress = level.xp_progress;
    guest.xp_total = level.xp_total;
    guest.xp_seed = level.xp_seed;
    guest.effects = crate::effects::PlayerEffects::from_saved(&level.active_effects);
    guest.game_mode = level.mode();
    guest.flying = level.flying;
    guest.flying_speed = level.fly_speed.filter(|s| s.is_finite()).unwrap_or(0.05);
    guest.selected = (level.selected_slot.max(0) as usize).min(8);
    // A point that is the world spawn is none of the player's own: it
    // respawns wherever the world spawn is then (`setworldspawn`).
    guest.spawn = level
        .spawn
        .filter(|s| Some(s.map(|v| v.floor() as i32)) != level.world_spawn)
        .map(|s| [s[0] + 0.5, s[1], s[2] + 0.5]);
    guest.respawn_elsewhere = level.respawn_elsewhere.clone();
    guest.saved = level.player_extra.clone();
    guest.seen_credits = level.seen_credits;
    guest.post_effects.clone_from(&level.post_effects);
    guest.last_death = level.last_death.clone();
    let extra = level.player_extra.as_ref();
    let short = |key: &str, default: i64| {
        extra
            .and_then(|t| crate::save::number(t.get(key)))
            .unwrap_or(default) as i32
    };
    guest.loaded_body = Some(SavedBody {
        fire_ticks: short("Fire", -20),
        air: short("Air", 300),
        fall_distance: level.fall_distance,
    });
    guest.saved_vehicle = level.root_vehicle;
    guest.recipe_book = level.recipe_book.clone();
    guest.dimension = match level.dimension.as_str() {
        d @ ("minecraft:the_nether" | "minecraft:the_end") => d.to_owned(),
        _ => "minecraft:overworld".to_owned(),
    };
    if level.ender_items.len() == 27 {
        guest.ender_items = level.ender_items.clone();
    }
    for (slot, item, count, damage) in &level.inventory {
        let mut stack = ItemStack {
            item: Arc::from(item.as_str()),
            count: *count,
            damage: *damage,
            extra: level.inventory_components.get(slot).cloned(),
        };
        if stack.count == 0 {
            stack.count = 1;
        }
        match *slot {
            0..=35 => guest.inventory[usize::from(*slot)] = Some(stack),
            100 => guest.armor[3] = Some(stack),
            101 => guest.armor[2] = Some(stack),
            102 => guest.armor[1] = Some(stack),
            103 => guest.armor[0] = Some(stack),
            150 => guest.offhand = Some(stack),
            _ => {}
        }
    }
}

/// Save a guest (PlayerDataStorage.save) who has left: the vehicle it took
/// along is in its `saved` compound already (`take_parent_vehicle`).
pub fn save(guest: &Guest, world: &Path) {
    let mut level = Level {
        player_uuid: Some(uuid_ints(guest.uuid)),
        ..Level::default()
    };
    store(guest, &mut level, None);
    let file = path(world, guest.uuid);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = write_dat(&file, level.player_tag());
}

/// Put a guest's state in `level`'s player fields; `vehicle` is the
/// RootVehicle compound of the ride (`ServerPlayer.saveParentVehicle`).
pub fn store(guest: &Guest, level: &mut Level, vehicle: Option<Tag>) {
    level.position = Some(guest.position);
    level.dimension = guest.dimension.clone();
    level.rotation = [guest.yaw, guest.pitch];
    level.health = guest.health;
    level.absorption = guest
        .motion
        .as_ref()
        .map_or(guest.saved_absorption.unwrap_or(0.0), |m| {
            m.health.absorption()
        });
    level.food = guest.food;
    level.saturation = guest.saturation;
    level.xp_level = guest.xp_level;
    level.xp_progress = guest.xp_progress;
    level.xp_total = guest.xp_total;
    level.xp_seed = guest.xp_seed;
    level.active_effects = guest.effects.saved();
    level.game_mode = guest.game_mode;
    level.creative = guest.game_mode == 1;
    level.flying = guest.flying;
    level.fly_speed = Some(guest.flying_speed);
    level.selected_slot = guest.selected as i32;
    level.spawn = guest.spawn.map(|s| [s[0] - 0.5, s[1], s[2] - 0.5]);
    level.respawn_elsewhere = guest.respawn_elsewhere.clone();
    level.on_ground = guest.on_ground;
    level.seen_credits = guest.seen_credits;
    level.post_effects.clone_from(&guest.post_effects);
    level.last_death = guest.last_death.clone();
    level.player_extra = guest.saved.clone();
    // The vehicle the player rides (`ServerPlayer.saveParentVehicle`), or
    // the one its file names that has not come back yet (kept in `saved`).
    level.root_vehicle = guest.saved_vehicle;
    if let Some(vehicle) = vehicle {
        level.root_vehicle = uuid_from_tag(vehicle.get("Attach"));
        let extra = level
            .player_extra
            .get_or_insert_with(|| Tag::Compound(Default::default()));
        put(extra, "RootVehicle", vehicle);
    }
    // The live body's fire, air and fall distance (Entity.saveWithoutId),
    // or what the file had while the player has no body yet.
    let body = guest
        .motion
        .as_deref()
        .map(SavedBody::of)
        .or(guest.loaded_body);
    if let Some(body) = body {
        level.fall_distance = body.fall_distance;
        let extra = level
            .player_extra
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(map) = extra {
            let short = |v: i32| horizon_world::nbt::Tag::Short(v.clamp(-32768, 32767) as i16);
            map.insert("Fire".into(), short(body.fire_ticks));
            map.insert("Air".into(), short(body.air));
        }
    }
    // LivingEntity.addAdditionalSaveData: the live body's AttributeMap.pack,
    // or the list the file had while the player has no body yet.
    if let Some(m) = guest.motion.as_deref() {
        let extra = level
            .player_extra
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(map) = extra {
            map.insert("attributes".into(), m.saved_attributes());
        }
    }
    level.recipe_book = guest.recipe_book.clone();
    level.ender_items = guest.ender_items.clone();
    level.inventory.clear();
    level.inventory_components.clear();
    let mut put = |slot: u8, stack: &Option<ItemStack>| {
        if let Some(s) = stack {
            level
                .inventory
                .push((slot, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = &s.extra {
                level.inventory_components.insert(slot, extra.clone());
            }
        }
    };
    for (i, s) in guest.inventory.iter().enumerate() {
        put(i as u8, s);
    }
    put(103, &guest.armor[0]);
    put(102, &guest.armor[1]);
    put(101, &guest.armor[2]);
    put(100, &guest.armor[3]);
    put(150, &guest.offhand);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::{Joined, PlayerSender};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    fn guest() -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "Alex".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
            },
            2,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    fn holding(uuid: u128, item: &str, count: u32) -> Guest {
        let mut g = guest();
        g.uuid = uuid;
        g.position = [10.5, 70.0, -3.5];
        g.inventory[0] = Some(ItemStack {
            item: Arc::from(item),
            count,
            damage: 0,
            extra: None,
        });
        g
    }

    fn world(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-owner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn first_item(g: &Guest) -> Option<(&str, u32)> {
        g.inventory[0].as_ref().map(|s| (&*s.item, s.count))
    }

    #[test]
    fn the_owner_loads_the_file_level_dat_names_over_an_older_own_file() {
        // #5351: a newer build saved the owner to both files, then 0.2.93 saved
        // it again to the file level.dat names alone. The owner's own file is
        // older; vanilla's loadPlayerData reads the named one for the owner.
        let owner = horizon_protocol::crypto::offline_uuid("Player46606");
        let dir = world("named");
        save(&holding(owner, "minecraft:diamond", 5), &dir);
        let mut level = Level::default();
        store(&holding(owner, "minecraft:gold_ingot", 7), &mut level, None);
        level.write(&dir).unwrap();
        let named = ints_uuid(level.player_uuid.unwrap());
        assert_ne!(
            named, owner,
            "level.dat names offline `Player`, not the owner"
        );
        let saved = Level::read(&dir).unwrap();
        let mut g = guest();
        g.uuid = owner;
        assert!(load_owner(&mut g, &dir, Some(&saved)));
        assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_owner_loads_its_own_file_when_the_named_one_is_missing() {
        let owner = horizon_protocol::crypto::offline_uuid("Player46606");
        let dir = world("own");
        let mut level = Level::default();
        level.write_without_player(&dir).unwrap();
        save(&holding(owner, "minecraft:diamond", 5), &dir);
        let saved = Level::read(&dir).unwrap();
        let mut g = guest();
        g.uuid = owner;
        assert!(load_owner(&mut g, &dir, Some(&saved)));
        assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_cut_off_between_its_renames_loads_the_dat_old() {
        // PlayerDataStorage.load: a file `write_dat` moved to `.dat_old` and
        // did not replace (the game stopped between the two renames) is read
        // instead, for the owner's named file and for anyone's own file.
        let owner = horizon_protocol::crypto::offline_uuid("Player46606");
        let dir = world("dat-old");
        let mut level = Level::default();
        for (item, count) in [("minecraft:diamond", 5), ("minecraft:gold_ingot", 7)] {
            store(&holding(owner, item, count), &mut level, None);
            level.write(&dir).unwrap();
            save(&holding(owner, item, count), &dir);
        }
        let named = path(&dir, ints_uuid(level.player_uuid.unwrap()));
        for file in [&named, &path(&dir, owner)] {
            std::fs::remove_file(file).unwrap();
            assert!(file.with_extension("dat_old").is_file());
        }
        let saved = Level::read(&dir).unwrap();
        assert!(saved.singleplayer_file, "the named file's .dat_old counts");
        let mut g = guest();
        g.uuid = owner;
        assert!(load_owner(&mut g, &dir, Some(&saved)));
        assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
        let mut g = guest();
        g.uuid = owner;
        assert!(load(&mut g, &dir), "a guest's own file falls back too");
        assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_owner_leaving_saves_the_file_it_loads() {
        // The last autosave wrote both files with a diamond; the owner leaves
        // with gold and the game stops before the world's own save. The file
        // the owner loads holds the gold, as its own file does.
        let owner = horizon_protocol::crypto::offline_uuid("Player46606");
        let dir = world("leave");
        let mut level = Level::default();
        store(&holding(owner, "minecraft:diamond", 5), &mut level, None);
        level.write(&dir).unwrap();
        save(&holding(owner, "minecraft:diamond", 5), &dir);
        save_owner(&holding(owner, "minecraft:gold_ingot", 7), &mut level, &dir);
        let saved = Level::read(&dir).unwrap();
        let mut g = guest();
        g.uuid = owner;
        assert!(load_owner(&mut g, &dir, Some(&saved)));
        assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
        let mut g = guest();
        g.uuid = owner;
        assert!(load(&mut g, &dir));
        assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_burn_and_fall_come_back() {
        // A player saved burning in mid-fall (#1323, #1327): Entity.saveWithoutId's
        // Fire, Air and fall_distance, read back for the next body.
        let mut g = guest();
        g.loaded_body = Some(SavedBody {
            fire_ticks: 96,
            air: 250,
            fall_distance: 9.5,
        });
        let mut level = Level::default();
        store(&g, &mut level, None);
        let tag = level.player_tag();
        let mut read = Level::default();
        read.read_player(&tag);
        read.player_extra = Some(tag);
        let mut back = guest();
        apply(&mut back, &read);
        assert_eq!(back.loaded_body, g.loaded_body);
    }

    #[test]
    fn post_effects_are_saved_with_the_player() {
        // ServerPlayer.addAdditionalSaveData's `post_effects`, in order; none
        // is an empty list, and a cleared list does not come back from an
        // older file's copy.
        let mut g = guest();
        g.post_effects = vec!["minecraft:invert".into(), "pack:glow".into()];
        let mut level = Level::default();
        store(&g, &mut level, None);
        let tag = level.player_tag();
        let mut read = Level::default();
        read.read_player(&tag);
        read.player_extra = Some(tag);
        let mut back = guest();
        apply(&mut back, &read);
        assert_eq!(back.post_effects, g.post_effects);
        // The cleared list is written over the loaded file.
        g.post_effects.clear();
        store(&g, &mut read, None);
        let tag = read.player_tag();
        assert!(matches!(
            tag.get("post_effects"),
            Some(horizon_world::nbt::Tag::List(l)) if l.is_empty()
        ));
    }

    /// The resources the prepared game data holds, for a body to breathe with.
    fn resources() -> Option<(crate::assets::Assets, crate::motion::MotionResources)> {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return None;
        }
        let assets = crate::assets::Assets::load(&root).ok()?;
        let resources = crate::motion::MotionResources::load(&root, &assets).ok()?;
        Some((assets, resources))
    }

    #[test]
    fn a_saved_air_comes_back() {
        // A player saved underwater (#1321): the file keeps the remaining air
        // (Entity.saveWithoutId's Air), and the body that comes back breathes
        // on it instead of starting with a full supply.
        let Some((_assets, resources)) = resources() else {
            eprintln!("skipped the air round trip: prepared resources unavailable");
            return;
        };
        let mut g = guest();
        let mut motion =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .expect("a body");
        motion.air = 99;
        g.motion = Some(Box::new(motion));
        let mut level = Level::default();
        store(&g, &mut level, None);
        let tag = level.player_tag();
        let mut read = Level::default();
        read.read_player(&tag);
        read.player_extra = Some(tag);
        let mut back = guest();
        apply(&mut back, &read);
        assert_eq!(back.loaded_body.expect("a body").air, 99);
    }

    #[test]
    fn a_saved_root_vehicle_comes_back() {
        // A player saved riding a horse (#1474): the file holds the vehicle
        // (ServerPlayer.saveParentVehicle's RootVehicle {Attach, Entity}),
        // and the next login spawns it and rides it again.
        let vehicle = [1, 2, 3, 4];
        let horse = Tag::compound([
            ("id", Tag::String("minecraft:horse".into())),
            ("UUID", Tag::IntArray(vehicle.to_vec())),
        ]);
        let root = Tag::compound([
            ("Attach", Tag::IntArray(vehicle.to_vec())),
            ("Entity", horse),
        ]);
        let g = guest();
        let mut level = Level::default();
        store(&g, &mut level, Some(root.clone()));
        let tag = level.player_tag();
        assert_eq!(tag.get("RootVehicle"), Some(&root));
        let mut read = Level::default();
        read.read_player(&tag);
        read.player_extra = Some(tag);
        let mut back = guest();
        apply(&mut back, &read);
        assert_eq!(back.saved_vehicle, Some(vehicle));
        // Saved again before the horse is back, the file keeps it whole.
        let mut again = Level::default();
        store(&back, &mut again, None);
        assert_eq!(again.player_tag().get("RootVehicle"), Some(&root));
    }

    fn abilities(g: &Guest) -> (i8, f32) {
        match g.abilities() {
            horizon_protocol::packets::play::Clientbound::PlayerAbilities(p) => {
                (p.flags, p.flying_speed)
            }
            other => panic!("not the abilities: {other:?}"),
        }
    }

    /// A flying speed a mod set goes out in every abilities packet (a game
    /// mode change's, a respawn's), is saved as vanilla's `abilities.flySpeed`
    /// and comes back when the player does; a file without one is vanilla's 0.05.
    #[test]
    fn a_set_flying_speed_is_sent_kept_and_saved() {
        let dir = world("fly-speed");
        let mut g = guest();
        assert_eq!(abilities(&g).1, 0.05);
        g.flying_speed = 2.5;
        g.game_mode = 1;
        assert_eq!(abilities(&g), (1 | 4 | 8, 2.5));
        g.game_mode = 0;
        assert_eq!(abilities(&g), (0, 2.5));
        save(&g, &dir);
        let tag = read_dat(&path(&dir, g.uuid)).unwrap();
        let saved = tag.get("abilities").and_then(|a| a.get("flySpeed"));
        assert_eq!(saved, Some(&Tag::Float(2.5f32.to_bits())));
        let mut back = guest();
        assert!(load(&mut back, &dir));
        assert_eq!(back.flying_speed, 2.5);
        // A vanilla file's own speed is read, and one without is 0.05.
        let mut level = Level::default();
        level.read_player(&Tag::compound([(
            "abilities",
            Tag::compound([("flySpeed", Tag::Float(0.1f32.to_bits()))]),
        )]));
        let mut other = guest();
        apply(&mut other, &level);
        assert_eq!(other.flying_speed, 0.1);
        level.read_player(&Tag::compound([(
            "abilities",
            Tag::Compound(Default::default()),
        )]));
        apply(&mut other, &level);
        assert_eq!(other.flying_speed, 0.05);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
