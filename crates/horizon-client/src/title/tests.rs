use super::*;
use horizon_world::nbt::Tag;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    root: PathBuf,
    title: Title,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-world-menu-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("saves")).unwrap();
        Self {
            title: Title::new(root.join("saves"), &root),
            root,
        }
    }
    fn world(&self, folder: &str, name: &str) {
        crate::save::write_dat(
            &self.title.saves.join(folder).join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([
                        ("LevelName", Tag::String(name.into())),
                        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
                        ("GameType", Tag::Int(1)),
                        ("allowCommands", Tag::Byte(1)),
                        (
                            "WorldGenSettings",
                            Tag::compound([("seed", Tag::Long(123456))]),
                        ),
                        (
                            "Version",
                            Tag::compound([("Name", Tag::String("26.3".into()))]),
                        ),
                        ("UnmodeledData", Tag::Long(987654321)),
                    ]),
                ),
                ("OtherRootData", Tag::String("preserve me".into())),
            ]),
        )
        .unwrap();
    }
    fn click(&mut self, label: &str) -> Action {
        self.click_with_sound(label).0
    }
    fn click_with_sound(&mut self, label: &str) -> (Action, bool) {
        let button = self
            .title
            .buttons(480.0, 262.0)
            .into_iter()
            .find(|b| b.label == label)
            .unwrap();
        self.title
            .click(button.x + button.w / 2.0, button.y + 10.0, (480.0, 262.0))
    }
    fn select(&mut self) {
        self.title.enter_singleplayer();
        self.title.click(240.0, 60.0, (480.0, 262.0));
    }
    fn finish_backup(&mut self) {
        self.title.backup_shown.set(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.title.mode, Mode::Backup { .. }) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.title.poll_backup();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn direct_connection_remembers_the_address_on_cancel_escape_and_join() {
    let mut f = Fixture::new();
    f.title.last_server = "localhost:25566".into();
    f.click("Multiplayer");
    // SafetyScreen comes first while its box was never ticked.
    assert!(matches!(f.title.mode, Mode::MultiplayerWarning { .. }));
    f.click("Proceed");
    f.click("Direct Connection");
    assert_eq!(f.title.focused_field().unwrap().1, "localhost:25566");

    *f.title.focused_field().unwrap().1 = "example.org:25567".into();
    f.click("Cancel");
    f.click("Direct Connection");
    assert_eq!(f.title.focused_field().unwrap().1, "example.org:25567");

    *f.title.focused_field().unwrap().1 = "[::1]:25568".into();
    f.title.escape();
    f.title.escape();
    f.click("Multiplayer");
    f.click("Proceed");
    f.click("Direct Connection");
    assert_eq!(f.title.focused_field().unwrap().1, "[::1]:25568");

    *f.title.focused_field().unwrap().1 = "localhost:25569".into();
    assert_eq!(
        f.click("Join Server"),
        Action::Connect {
            address: "localhost:25569".into(),
        }
    );
    assert_eq!(f.title.last_server, "localhost:25569");

    f.title.focused_field().unwrap().1.clear();
    f.title.escape();
    f.click("Direct Connection");
    assert!(f.title.focused_field().unwrap().1.is_empty());
}

#[test]
fn f5_reloads_the_server_list() {
    let mut f = Fixture::new();
    f.click("Multiplayer");
    f.click("Proceed");
    let file = crate::multiplayer::servers_file(&f.title.saves);
    horizon_protocol::servers::save(
        &file,
        &[horizon_protocol::servers::ServerEntry::new(
            "Local",
            "127.0.0.1:1",
        )],
    )
    .unwrap();
    let listed = |f: &Fixture| match &f.title.mode {
        Mode::Multiplayer(crate::multiplayer::Screen::List { servers, .. }) => servers.len(),
        _ => usize::MAX,
    };
    assert_eq!(listed(&f), 0);
    f.title.refresh_servers();
    assert_eq!(listed(&f), 1);
    // Only the list refreshes.
    f.title.mode = Mode::Main;
    f.title.refresh_servers();
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
fn empty_singleplayer_opens_creation_and_cancel_returns_to_title() {
    let mut f = Fixture::new();
    f.click("Singleplayer");
    assert!(matches!(f.title.mode, Mode::Create { .. }));
    f.click("Cancel");
    assert!(matches!(f.title.mode, Mode::Main));
    f.click("Singleplayer");
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
    // Filtering an existing list down to zero results must not create a world.
    f.world("One", "One");
    f.title.enter_singleplayer();
    f.title.type_text("no match");
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn selector_controls_require_selection_and_search_uses_filtered_indices() {
    let mut f = Fixture::new();
    f.world("A", "Alpha");
    f.world("B", "Beta");
    f.title.enter_singleplayer();
    let Mode::Select { worlds, .. } = &mut f.title.mode else {
        panic!("existing worlds should open the selector");
    };
    // Beta must have a different index in the full list and filtered list.
    // Filesystem timestamp precision must not decide which world comes first.
    worlds.sort_by(|a, b| a.folder.cmp(&b.folder));
    for label in ["Play Selected World", "Edit", "Delete", "Re-Create"] {
        assert!(
            !f.title
                .buttons(480.0, 262.0)
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .enabled
        );
        assert_eq!(f.click(label), Action::None);
    }
    // Clicking the disabled footer controls leaves the search box unfocused.
    f.title.click(240.0, 31.0, (480.0, 262.0));
    f.title.type_text("bEtA");
    assert_eq!(f.title.search, "bEtA");
    f.title.click(240.0, 60.0, (480.0, 262.0));
    f.click("Edit");
    assert!(matches!(&f.title.mode, Mode::Edit { world, .. } if world.folder == "B"));
    f.title.escape();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "B")
    );
    f.title.search_focused = true;
    f.title.backspace();
    assert!(matches!(f.title.mode, Mode::Select { selected: None, .. }));
}

#[test]
fn edit_layout_disables_unsupported_optimization_and_missing_icon() {
    let mut f = Fixture::new();
    f.world("World", "New World");
    f.select();
    f.click("Edit");
    let buttons = f.title.buttons(480.0, 262.0);
    let reset = buttons.iter().find(|b| b.label == "Reset Icon").unwrap();
    assert_eq!(
        (reset.x, reset.y, reset.w, reset.enabled),
        (140.0, 78.0, 200.0, false)
    );
    // A 1920x1046 client area has a fractional height at GUI scale 4;
    // vanilla rounds up to 262 GUI pixels before positioning the layout.
    assert_eq!(f.title.buttons(480.0, 261.5)[0].y, 78.0);
    assert!(
        !buttons
            .iter()
            .find(|b| b.label == "Optimize World")
            .unwrap()
            .enabled
    );
    f.click("Optimize World");
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
    fs::write(f.title.saves.join("World/icon.png"), b"icon").unwrap();
    assert!(
        f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Reset Icon")
            .unwrap()
            .enabled
    );
    f.click("Reset Icon");
    assert!(!f.title.saves.join("World/icon.png").exists());
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Reset Icon")
            .unwrap()
            .enabled
    );
    f.title.click(240.0, 60.0, (480.0, 262.0));
    for _ in 0..32 {
        f.title.backspace();
    }
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Save")
            .unwrap()
            .enabled
    );
}

#[test]
fn rename_preserves_unknown_metadata_and_cancel_discards_edits() {
    let mut f = Fixture::new();
    f.world("World", "Original");
    let path = f.title.saves.join("World/level.dat");
    let original = crate::save::read_dat(&path).unwrap();
    f.select();
    f.click("Edit");
    f.title.type_text(" discarded");
    f.click("Cancel");
    assert_eq!(crate::save::read_dat(&path).unwrap(), original);
    f.click("Edit");
    for _ in 0..32 {
        f.title.backspace();
    }
    f.title.type_text("  Renamed 世界  ");
    f.click("Save");
    let mut expected = original;
    let Some(Tag::Compound(data)) = expected.get_mut("Data") else {
        panic!()
    };
    data.insert("LevelName".into(), Tag::String("Renamed 世界".into()));
    assert_eq!(crate::save::read_dat(&path).unwrap(), expected);
    assert!(path.with_file_name("level.dat_old").is_file());
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].name == "Renamed 世界")
    );
    f.click("Edit");
    f.title.type_text(" via Enter");
    f.title.confirm_edit();
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "Renamed 世界 via Enter"
    );
}

#[test]
fn delete_requires_confirmation_and_both_cancel_paths_preserve_world() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Delete");
    assert!(matches!(f.title.mode, Mode::Delete { .. }));
    assert!(f.title.saves.join("World/level.dat").is_file());
    f.click("Cancel");
    f.click("Delete");
    f.title.escape();
    assert!(f.title.saves.join("World/level.dat").is_file());
    f.click("Delete");
    let delete = f
        .title
        .buttons(480.0, 262.0)
        .into_iter()
        .find(|b| b.label == "Delete")
        .unwrap();
    assert_eq!((delete.x, delete.y, delete.w), (88.0, 146.0, 150.0));
    f.click("Delete");
    assert!(!f.title.saves.join("World").exists());
    // The last world gone, vanilla opens a fresh Create New World, not an empty list.
    assert!(matches!(&f.title.mode, Mode::Create { name, .. } if name == "New World"));
    f.click("Cancel");
    assert!(matches!(f.title.mode, Mode::Main));
    f.click("Singleplayer");
    assert!(matches!(f.title.mode, Mode::Create { .. }));
}

#[test]
fn deleting_one_of_two_worlds_keeps_the_list() {
    let mut f = Fixture::new();
    f.world("One", "One");
    f.world("Two", "Two");
    f.select();
    f.click("Delete");
    f.click("Delete");
    assert!(matches!(&f.title.mode, Mode::Select { worlds, .. } if worlds.len() == 1));
}

#[test]
fn a_list_emptied_elsewhere_opens_create_world() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    world_management::delete(&f.title.saves, "World").unwrap();
    f.title.worlds_changed();
    assert!(matches!(f.title.mode, Mode::Create { .. }));
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
fn backup_zip_preserves_nested_files_excludes_lock_and_keeps_duplicate_backups() {
    let f = Fixture::new();
    f.world("World", "World");
    let world = f.title.saves.join("World");
    fs::create_dir_all(world.join("dimensions/minecraft/overworld/region")).unwrap();
    fs::write(world.join("session.lock"), b"locked").unwrap();
    fs::write(
        world.join("dimensions/minecraft/overworld/region/r.0.0.mca"),
        b"chunk bytes",
    )
    .unwrap();
    for _ in 0..2 {
        assert!(world_management::backup(&f.title.saves, "World").unwrap() > 0);
    }
    let archives: Vec<_> = fs::read_dir(f.root.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(archives.len(), 2);
    for archive in archives {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        assert!(zip.by_name("World/session.lock").is_err());
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("World/level.dat").unwrap(), &mut bytes)
            .unwrap();
        assert_eq!(bytes, fs::read(world.join("level.dat")).unwrap());
        bytes.clear();
        std::io::Read::read_to_end(
            &mut zip
                .by_name("World/dimensions/minecraft/overworld/region/r.0.0.mca")
                .unwrap(),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(bytes, b"chunk bytes");
    }
}

#[test]
fn backup_screen_blocks_navigation_and_returns_to_the_selected_world() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    f.title.type_text(" unsaved");
    f.click("Make Backup");
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    assert!(f.title.buttons(480.0, 262.0).is_empty());
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    // The first frame must be shown even when the backup finishes immediately.
    f.title.poll_backup();
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    f.finish_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    assert!(
        matches!(&f.title.toast, Some((_, WorldToast::Created { folder, bytes })) if folder == "World" && *bytes > 0)
    );
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "World"
    );
}

#[test]
fn failed_backup_returns_to_the_selected_world_with_an_error_toast() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    f.title.type_text(" unsaved");
    // A file at the backup-directory path makes the real worker fail reliably.
    fs::write(f.root.join("backups"), b"not a directory").unwrap();
    f.click("Make Backup");
    f.finish_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    let Some((_, WorldToast::Failed(error))) = &f.title.toast else {
        panic!("backup failures should produce an error toast");
    };
    assert!(!error.is_empty());
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "World"
    );
    assert_eq!(
        fs::read(f.root.join("backups")).unwrap(),
        b"not a directory"
    );
}

#[test]
fn stopped_backup_worker_returns_to_the_list_with_an_error_toast() {
    let mut f = Fixture::new();
    f.world("World", "World");
    let world = list_worlds(&f.title.saves).remove(0);
    let (send, receive) = mpsc::channel();
    f.title.mode = Mode::Backup { world };
    f.title.backup = Some(receive);
    f.title.backup_shown.set(true);
    drop(send);
    f.title.poll_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    assert!(
        matches!(&f.title.toast, Some((_, WorldToast::Failed(error))) if error == "Backup worker stopped")
    );
}

#[test]
fn backup_toast_slides_in_and_out_with_its_sounds() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    f.click("Make Backup");
    f.title.poll_backup();
    f.finish_backup();
    assert_eq!(f.title.update_toast(1.0), Some("minecraft:ui.toast.in"));
    assert_eq!(f.title.update_toast(1.0), None);
    let age = |f: &mut Fixture, ms: u64| {
        let (slide, _) = f.title.toast.as_mut().unwrap();
        let origin = slide.origin.as_mut().unwrap();
        *origin -= std::time::Duration::from_millis(ms);
    };
    // Half way in: (300 / 600)² of its width shows.
    age(&mut f, 300);
    assert_eq!(f.title.update_toast(1.0), None);
    let portion = f.title.toast.as_ref().unwrap().0.portion;
    assert!((portion - 0.25).abs() < 0.02, "{portion}");
    // Shown in full for the display time, then out.
    age(&mut f, 300);
    f.title.update_toast(1.0);
    age(&mut f, 5100);
    assert_eq!(f.title.update_toast(1.0), Some("minecraft:ui.toast.out"));
    assert!(f.title.toast.is_some());
    age(&mut f, 700);
    assert_eq!(f.title.update_toast(1.0), None);
    assert!(f.title.toast.is_none());
}

#[test]
fn failed_operations_show_error_and_leave_world_intact() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    fs::remove_file(f.title.saves.join("World/level.dat")).unwrap();
    f.click("Save");
    assert!(matches!(f.title.mode, Mode::Error { .. }));
    f.click("Back");
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
    assert!(f.title.saves.join("World").is_dir());
    for bad in ["../World", "..", ".", "/World"] {
        assert!(world_management::delete(&f.title.saves, bad).is_err());
    }
}

#[cfg(unix)]
#[test]
fn backup_rejects_symlinks_and_removes_incomplete_archive() {
    let f = Fixture::new();
    f.world("World", "World");
    std::os::unix::fs::symlink(f.root.join("outside"), f.title.saves.join("World/link")).unwrap();
    assert!(world_management::backup(&f.title.saves, "World").is_err());
    assert_eq!(fs::read_dir(f.root.join("backups")).unwrap().count(), 0);
}

#[test]
fn recreate_uses_existing_name_seed_and_mode_and_scroll_reaches_later_rows() {
    let mut f = Fixture::new();
    for i in 0..10 {
        f.world(&format!("World{i}"), &format!("World {i}"));
    }
    f.select();
    f.click("Re-Create");
    assert!(
        matches!(&f.title.mode,Mode::Create { name,seed,creative:true,.. } if name.starts_with("World ") && seed == "123456")
    );
    f.click("Cancel");
    f.title
        .scroll_worlds(-100.0, (240.0, 100.0), (480.0, 262.0));
    assert!(f.title.scroll.get().amount > 0.0);
    let Mode::Select { worlds, .. } = &f.title.mode else {
        panic!()
    };
    let last = worlds.last().unwrap().folder.clone();
    f.title.click(240.0, 180.0, (480.0, 262.0));
    assert!(
        matches!(&f.title.mode,Mode::Select { worlds,selected:Some(i) } if worlds[*i].folder == last)
    );
}

#[test]
fn classic_navigation_buttons_sound_even_without_a_game_action() {
    let mut f = Fixture::new();
    f.world("World", "World");
    assert_eq!(
        f.title.click(0.0, 0.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert_eq!(
        f.click_with_sound("Minecraft Realms"),
        (Action::None, false)
    );
    assert_eq!(f.click_with_sound("Singleplayer"), (Action::None, true));
    assert_eq!(
        f.click_with_sound("Play Selected World"),
        (Action::None, false)
    );
    assert_eq!(f.click_with_sound("Create New World"), (Action::None, true));
    assert_eq!(
        f.title.click(240.0, 55.0, (480.0, 262.0)),
        (Action::None, false)
    );
    // The Game tab's game mode cycle button.
    let mode = f
        .title
        .buttons(480.0, 262.0)
        .into_iter()
        .find(|b| b.id == 20)
        .unwrap();
    assert_eq!(
        f.title.click(mode.x + 1.0, mode.y + 1.0, (480.0, 262.0)),
        (Action::None, true)
    );
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Back"), (Action::None, true));
    assert_eq!(f.click_with_sound("Options..."), (Action::Options, true));
}

#[test]
fn classic_world_selection_is_silent_until_the_entry_launches() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    assert_eq!(
        f.title.click(240.0, 60.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert_eq!(
        f.title.click(240.0, 60.0, (480.0, 262.0)),
        (
            Action::Play {
                folder: "World".into()
            },
            true
        )
    );
}

#[test]
fn a_world_open_elsewhere_stays_closed_behind_a_toast() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    let open = crate::save::SessionLock::acquire(&f.title.saves.join("World")).unwrap();
    let (icon_x, icon_y) = (240.0 - 135.0 + 2.0, LIST_TOP + 4.0);
    assert_eq!(
        f.title
            .click(icon_x + 16.0, icon_y + 16.0, (480.0, 262.0))
            .0,
        Action::None
    );
    assert!(
        matches!(&f.title.toast, Some((_, WorldToast::AccessFailure(folder))) if folder == "World")
    );
    assert!(matches!(f.title.mode, Mode::Select { .. }));
    drop(open);
    assert_eq!(
        f.title
            .click(icon_x + 16.0, icon_y + 16.0, (480.0, 262.0))
            .0,
        Action::Play {
            folder: "World".into()
        }
    );
}

#[test]
fn clicking_a_world_icon_joins_it_at_once() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    // WorldListEntry.mouseOverIcon: the row's 32x32 icon, left of its text.
    let (icon_x, icon_y) = (240.0 - 135.0 + 2.0, LIST_TOP + 4.0);
    assert_eq!(
        f.title.click(icon_x + 31.0, icon_y + 16.0, (480.0, 262.0)),
        (
            Action::Play {
                folder: "World".into()
            },
            true
        )
    );
    // Right of the icon only selects the row.
    f.title.enter_singleplayer();
    assert_eq!(
        f.title.click(icon_x + 33.0, icon_y + 16.0, (480.0, 262.0)),
        (Action::None, false)
    );
}

#[test]
fn world_management_buttons_sound_when_enabled() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    assert_eq!(f.click_with_sound("Edit"), (Action::None, true));
    assert_eq!(f.click_with_sound("Reset Icon"), (Action::None, false));
    assert_eq!(f.click_with_sound("Optimize World"), (Action::None, false));
    fs::write(f.title.saves.join("World/icon.png"), b"icon").unwrap();
    assert_eq!(f.click_with_sound("Reset Icon"), (Action::None, true));
    let Mode::Edit { name, .. } = &mut f.title.mode else {
        panic!("edit screen")
    };
    name.clear();
    assert_eq!(f.click_with_sound("Save"), (Action::None, false));
    let Mode::Edit { name, .. } = &mut f.title.mode else {
        panic!("edit screen")
    };
    *name = "Renamed".into();
    assert_eq!(f.click_with_sound("Save"), (Action::None, true));
    assert_eq!(f.click_with_sound("Edit"), (Action::None, true));
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Create { .. }));
}

#[test]
fn backup_button_sounds_and_the_progress_screen_is_silent() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    assert_eq!(f.click_with_sound("Make Backup"), (Action::None, true));
    assert_eq!(
        f.title.click(240.0, 120.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    f.finish_backup();
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn failed_world_operation_back_button_plays_the_click_sound() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    fs::remove_file(f.title.saves.join("World/level.dat")).unwrap();
    assert_eq!(f.click_with_sound("Save"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Error { .. }));
    assert_eq!(f.click_with_sound("Back"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
}

#[test]
fn delete_refuses_an_open_world_then_removes_it_with_its_lock() {
    let f = Fixture::new();
    f.world("World", "World");
    let open = crate::save::SessionLock::acquire(&f.title.saves.join("World")).unwrap();
    assert!(world_management::delete(&f.title.saves, "World").is_err());
    assert!(f.title.saves.join("World/level.dat").is_file());
    drop(open);
    world_management::delete(&f.title.saves, "World").unwrap();
    assert!(!f.title.saves.join("World").exists());
}

#[cfg(unix)]
#[test]
fn delete_removes_links_without_following_them() {
    let f = Fixture::new();
    f.world("World", "World");
    let outside = f.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("World/link")).unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("Linked")).unwrap();
    assert!(world_management::delete(&f.title.saves, "Linked").is_err());
    world_management::delete(&f.title.saves, "World").unwrap();
    assert!(!f.title.saves.join("World").exists());
    assert!(outside.join("keep.txt").is_file());
}

#[test]
fn hovered_buttons_ask_for_the_hand_or_not_allowed() {
    use crate::widgets::Cursor;
    let mut f = Fixture::new();
    let atlas = crate::hud::HudAtlas::build(Path::new("/nonexistent/horizon-test-assets")).unwrap();
    let cursor_over = |title: &Title, label: Option<&str>| {
        let mouse = label.map_or((0.0, 0.0), |label| {
            let buttons = title.buttons(480.0, 262.0);
            let b = buttons.iter().find(|b| b.label == label).unwrap();
            (b.x + b.w / 2.0, b.y + 10.0)
        });
        let _ = crate::widgets::take_cursor();
        title.draw(&atlas, &mut HudBatch::new([480, 262]), mouse, true);
        crate::widgets::take_cursor()
    };
    assert_eq!(
        cursor_over(&f.title, Some("Singleplayer")),
        Cursor::PointingHand
    );
    assert_eq!(cursor_over(&f.title, None), Cursor::Arrow);
    // The world list's footer is inactive until a world is selected.
    f.world("A", "Alpha");
    f.title.enter_singleplayer();
    assert_eq!(cursor_over(&f.title, Some("Edit")), Cursor::NotAllowed);
}

#[test]
fn world_search_hint_is_gray_and_only_shown_empty_and_unfocused() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    let atlas = crate::hud::HudAtlas::build(Path::new("/nonexistent/horizon-test-assets")).unwrap();
    let draw = |title: &Title| {
        let mut batch = HudBatch::new([480, 262]);
        title.draw(&atlas, &mut batch, (0.0, 0.0), true);
        batch
    };
    let has_hint = |batch: &HudBatch| {
        batch
            .vertices
            .iter()
            .any(|v| v.color == rgba(170, 170, 170, 255) && (21.0..41.0).contains(&v.pos[1]))
    };
    assert!(has_hint(&draw(&f.title)));

    f.title.click(240.0, 31.0, (480.0, 262.0));
    assert!(f.title.search_focused);
    assert!(!has_hint(&draw(&f.title)));
    f.title.type_text("World");
    f.title.click(240.0, 60.0, (480.0, 262.0));
    assert!(!f.title.search_focused);
    assert!(!has_hint(&draw(&f.title)));
}

#[test]
fn scrolled_world_list_draws_inside_the_list_and_drags_its_scroller() {
    let mut f = Fixture::new();
    for i in 0..10 {
        f.world(&format!("World{i}"), &format!("World {i}"));
    }
    f.select();
    let size = (480.0, 262.0);
    // SelectWorldScreen's list: 49 to h - 60, scroller at the row's right + 8.
    let bar = (480.0 / 2.0 - ROW_WIDTH / 2.0 + ROW_WIDTH + 8.0 + 3.0, 60.0);
    f.title.click(bar.0, bar.1, size);
    f.title.drag((bar.0, 120.0), size);
    let moved = f.title.scroll.get().amount;
    assert!(moved > 0.0);
    f.title.release();
    f.title.drag((bar.0, 60.0), size);
    assert_eq!(f.title.scroll.get().amount, moved);
    let atlas = crate::hud::HudAtlas::build(Path::new("/nonexistent/horizon-test-assets")).unwrap();
    let mut batch = HudBatch::new([480, 262]);
    f.title.draw(&atlas, &mut batch, (0.0, 0.0), true);
    assert!(!batch.vertices.is_empty());
}

#[test]
fn create_world_tabs_follow_vanilla_and_their_choices_reach_the_action() {
    let mut f = Fixture::new();
    f.click("Singleplayer");
    let size = (480.0, 262.0);
    let labels = |f: &Fixture| -> Vec<(String, f32, f32, bool)> {
        f.title
            .buttons(size.0, size.1)
            .into_iter()
            .map(|b| (b.label, b.x, b.y, b.enabled))
            .collect()
    };
    // Game tab: the grid (210 × 117) at (0.5, 1/6) of 24..h-33, footer centred.
    let game = labels(&f);
    assert_eq!(game[0].1, 135.0);
    assert_eq!(game[0].2, 24.0 + 14.0 + 41.0);
    assert_eq!((game[3].1, game[3].2), (86.0, 235.0));
    assert_eq!((game[4].1, game[4].2), (244.0, 235.0));
    // Survival, Hardcore (Hard, no cheats, both locked), Creative (cheats on).
    f.title.click(240.0, game[0].2 + 10.0, size);
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!()
    };
    assert!(options.hardcore && options.difficulty() == 3 && !options.commands(false));
    assert!(!labels(&f)[1].3 && !labels(&f)[2].3);
    f.title.click(240.0, game[0].2 + 10.0, size);
    let Mode::Create {
        creative, options, ..
    } = &f.title.mode
    else {
        panic!()
    };
    assert!(*creative && !options.hardcore && options.commands(true));
    // The World tab (second of three 124-wide tabs from x 54): its seed box
    // takes the focus, the world type cycles and the switches toggle.
    let (action, sound) = f.title.click(54.0 + 124.0 + 10.0, 10.0, size);
    assert!(sound && action == Action::None);
    let world = labels(&f);
    assert_eq!((world[0].1, world[0].2), (85.0, 38.0));
    assert_eq!((world[2].1, world[2].2), (85.0 + 266.0, 38.0 + 73.0));
    // The world-type button cycles through vanilla's world presets.
    let world_type = |f: &Fixture| -> crate::save::WorldPreset {
        let Mode::Create { options, .. } = &f.title.mode else {
            panic!()
        };
        options.world_type
    };
    assert_eq!(world_type(&f), crate::save::WorldPreset::Normal);
    for expected in [
        crate::save::WorldPreset::Flat,
        crate::save::WorldPreset::LargeBiomes,
        crate::save::WorldPreset::Amplified,
        crate::save::WorldPreset::SingleBiomeSurface,
        crate::save::WorldPreset::Normal,
    ] {
        f.title.click(world[0].1 + 10.0, world[0].2 + 10.0, size);
        assert_eq!(world_type(&f), expected);
    }
    // The amplified preset is the third one.
    for _ in 0..3 {
        f.title.click(world[0].1 + 10.0, world[0].2 + 10.0, size);
    }
    f.title.click(world[3].1 + 10.0, world[3].2 + 10.0, size);
    f.title.click(100.0, 38.0 + 41.0 + 10.0, size);
    let Mode::Create { focus, options, .. } = &f.title.mode else {
        panic!()
    };
    assert_eq!(*focus, 1);
    assert_eq!(options.world_type, crate::save::WorldPreset::Amplified);
    assert!(options.bonus_chest());
    let Action::Create { options, .. } = f.title.create_action() else {
        panic!()
    };
    assert_eq!(
        options.world_gen(),
        crate::save::WorldGen {
            preset: crate::save::WorldPreset::Amplified,
            structures: true,
            bonus_chest: true
        }
    );
    // The More tab: Experiments has no screen yet; Data
    // Packs opens on a new pack folder, and its selection reaches the action.
    f.title.click(54.0 + 248.0 + 10.0, 10.0, size);
    let more = labels(&f);
    assert_eq!(
        more[..3].iter().map(|b| b.3).collect::<Vec<_>>(),
        [true, false, true]
    );
    let (action, sound) = f.title.click(more[2].1 + 10.0, more[2].2 + 10.0, size);
    let Action::DataPacks { dir, selected } = action else {
        panic!("{action:?}")
    };
    assert!(sound && dir.is_dir() && selected == ["vanilla"]);
    f.title
        .set_data_packs(vec!["vanilla".into(), "file/ttm.zip".into()]);
    let Action::Create { options, .. } = f.title.create_action() else {
        panic!()
    };
    assert_eq!(options.data_packs, ["vanilla", "file/ttm.zip"]);
    assert_eq!(options.pack_dir.as_ref(), Some(&dir));
    // Cancel removes the folder (removeTempDataPackDir).
    f.title.escape();
    assert!(!dir.exists());
}

#[test]
fn new_world_generation_settings_round_trip() {
    let f = Fixture::new();
    let dir = f.root.join("flat");
    let options = crate::save::WorldGen {
        preset: crate::save::WorldPreset::Flat,
        structures: false,
        bonus_chest: true,
    };
    crate::save::write_world_gen_settings(&dir, 42, options).unwrap();
    assert_eq!(crate::save::WorldGen::read(&dir), Some(options));
    let settings = crate::save::world_gen_settings(&dir).unwrap();
    let generator = settings
        .get("dimensions")
        .and_then(|d| d.get("minecraft:overworld"))
        .and_then(|o| o.get("generator"));
    assert_eq!(
        crate::save::text(generator.and_then(|g| g.get("type"))).as_deref(),
        Some("minecraft:flat")
    );
}

#[test]
fn each_world_preset_writes_its_own_generator() {
    let f = Fixture::new();
    let generator = |preset: crate::save::WorldPreset| {
        let dir = f.root.join(preset.id());
        let options = crate::save::WorldGen {
            preset,
            structures: true,
            bonus_chest: false,
        };
        crate::save::write_world_gen_settings(&dir, 42, options).unwrap();
        assert_eq!(crate::save::WorldGen::read(&dir), Some(options));
        let settings = crate::save::world_gen_settings(&dir).unwrap();
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"))
            .cloned()
            .unwrap();
        (
            crate::save::text(generator.get("type")),
            crate::save::text(generator.get("settings")),
            crate::save::text(generator.get("biome_source").and_then(|s| s.get("type"))),
        )
    };
    assert_eq!(
        generator(crate::save::WorldPreset::Normal),
        (
            Some("minecraft:noise".to_owned()),
            Some("minecraft:overworld".to_owned()),
            Some("minecraft:multi_noise".to_owned()),
        )
    );
    assert_eq!(
        generator(crate::save::WorldPreset::Amplified),
        (
            Some("minecraft:noise".to_owned()),
            Some("minecraft:amplified".to_owned()),
            Some("minecraft:multi_noise".to_owned()),
        )
    );
    assert_eq!(
        generator(crate::save::WorldPreset::LargeBiomes),
        (
            Some("minecraft:noise".to_owned()),
            Some("minecraft:large_biomes".to_owned()),
            Some("minecraft:multi_noise".to_owned()),
        )
    );
    assert_eq!(
        generator(crate::save::WorldPreset::SingleBiomeSurface),
        (
            Some("minecraft:noise".to_owned()),
            Some("minecraft:overworld".to_owned()),
            Some("minecraft:fixed".to_owned()),
        )
    );
    assert_eq!(
        generator(crate::save::WorldPreset::Flat).0,
        Some("minecraft:flat".to_owned())
    );
}

#[test]
fn world_fields_edit_at_their_cursor() {
    use winit::keyboard::KeyCode;
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    // The search box takes the focus.
    f.title.click(240.0, 30.0, (480.0, 262.0));
    f.title.type_text("hello world");
    for _ in 0..5 {
        assert!(f.title.field_key(KeyCode::ArrowLeft, false, false));
    }
    f.title.type_text("X");
    assert_eq!(f.title.search, "hello Xworld");
    // Shift+Home selects back to the start; typing replaces the selection.
    f.title.field_key(KeyCode::Home, false, true);
    f.title.type_text("Y");
    assert_eq!(f.title.search, "Yworld");
    // Ctrl+Left jumps a word; Backspace deletes before the cursor.
    f.title.field_key(KeyCode::ArrowLeft, true, false);
    f.title.field_key(KeyCode::ArrowRight, false, false);
    f.title.field_key(KeyCode::Backspace, false, false);
    assert_eq!(f.title.search, "world");
}

#[test]
fn server_fields_edit_at_their_cursor() {
    use winit::keyboard::KeyCode;
    let mut f = Fixture::new();
    f.title.mode = Mode::Multiplayer(crate::multiplayer::Screen::Edit {
        index: None,
        name: "Minecraft Server".into(),
        address: String::new(),
        focus: 1,
        packs: None,
    });
    f.title.type_text("localhost");
    for _ in 0..4 {
        assert!(f.title.field_key(KeyCode::ArrowLeft, false, false));
    }
    f.title.type_text("X");
    let Mode::Multiplayer(crate::multiplayer::Screen::Edit { address, .. }) = &f.title.mode else {
        panic!("add server");
    };
    assert_eq!(address, "localXhost");
    // Shift+Home selects back to the start; typing replaces the selection.
    f.title.field_key(KeyCode::Home, false, true);
    f.title.type_text("Y");
    let Mode::Multiplayer(crate::multiplayer::Screen::Edit { address, .. }) = &f.title.mode else {
        panic!("add server");
    };
    assert_eq!(address, "Yhost");
    // Tab moves to the name, which edits at its own cursor.
    f.title.tab();
    f.title.field_key(KeyCode::ArrowLeft, false, false);
    f.title.type_text("X");
    let Mode::Multiplayer(crate::multiplayer::Screen::Edit { name, .. }) = &f.title.mode else {
        panic!("add server");
    };
    assert_eq!(name, "Minecraft ServeXr");
    f.title.mode = Mode::Multiplayer(crate::multiplayer::Screen::Direct {
        address: String::new(),
    });
    f.title.type_text("one two");
    // Ctrl+Backspace deletes the word before the caret.
    f.title.field_key(KeyCode::Backspace, true, false);
    let Mode::Multiplayer(crate::multiplayer::Screen::Direct { address }) = &f.title.mode else {
        panic!("direct");
    };
    assert_eq!(address, "one ");
}
#[test]
fn world_list_names_every_game_mode_and_hardcore_in_red() {
    let f = Fixture::new();
    f.world("Creative", "Creative");
    let level = |folder: &str, game_type: i32, hardcore: bool| {
        crate::save::write_dat(
            &f.title.saves.join(folder).join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("LevelName", Tag::String(folder.into())),
                    ("GameType", Tag::Int(game_type)),
                    (
                        "difficulty_settings",
                        Tag::compound([
                            ("difficulty", Tag::String("hard".into())),
                            ("hardcore", Tag::Byte(i8::from(hardcore))),
                        ]),
                    ),
                    (
                        "Version",
                        Tag::compound([("Name", Tag::String("26.3".into()))]),
                    ),
                ]),
            )]),
        )
        .unwrap();
    };
    level("Hardcore", 0, true);
    level("Adventure", 2, false);
    level("Spectator", 3, false);
    let worlds = list_worlds(&f.title.saves);
    let info = |folder: &str| world_info(worlds.iter().find(|w| w.folder == folder).unwrap());

    let hardcore = info("Hardcore");
    assert_eq!(hardcore.plain(), "Hardcore Mode, Version: 26.3");
    assert_eq!(hardcore.spans[0].text, "Hardcore Mode");
    assert_eq!(hardcore.spans[0].style.color, Some(0xFF0000));
    assert!(hardcore.spans[1..].iter().all(|s| s.style.color.is_none()));
    assert_eq!(info("Adventure").plain(), "Adventure Mode, Version: 26.3");
    assert_eq!(info("Spectator").plain(), "Spectator Mode, Version: 26.3");
    let creative = info("Creative");
    assert_eq!(creative.plain(), "Creative Mode, Commands, Version: 26.3");
    assert!(creative.spans.iter().all(|s| s.style.color.is_none()));
}
