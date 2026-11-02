use super::*;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-shortcut-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn names_fit_every_system() {
    assert_eq!(
        file_name("Hardcore", System::Linux),
        "Hardcore (Horizon).desktop"
    );
    assert_eq!(
        file_name("A/B: C?", System::Windows),
        "A-B- C- (Horizon).lnk"
    );
    assert_eq!(file_name(" .. ", System::Mac), "Instance (Horizon).app");
    assert_eq!(slug("port:all the mods"), "port-all-the-mods");
}

#[test]
fn a_linux_entry_plays_its_instance() {
    let entry = desktop_entry(
        Path::new("/opt/My Horizon/horizon-launcher"),
        "port:atm",
        "ATM",
    );
    assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=ATM\n"));
    assert!(entry
        .contains("\nExec=\"/opt/My Horizon/horizon-launcher\" play --instance \"port:atm\"\n"));
    assert!(entry.contains("\nTryExec=/opt/My Horizon/horizon-launcher\n"));
    assert!(entry.ends_with("\nX-Horizon-Instance=port:atm\n"));
    let odd = desktop_entry(Path::new("/x"), "a", "Two\nlines");
    assert!(odd.contains("Name=Two lines\n"));
}

#[test]
fn a_mac_app_runs_a_script_that_plays_its_instance() {
    let files = mac_app(
        Path::new("/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher"),
        "it's",
        "Tom & Jerry",
    );
    let (_, plist, _) = &files[0];
    let plist = String::from_utf8(plist.clone()).unwrap();
    assert!(plist.contains("<string>Tom &amp; Jerry</string>"));
    assert!(plist.contains("<string>dev.horizon.instance.it-s</string>"));
    assert!(plist.contains("<key>LSUIElement</key>\n\t<true/>"));
    let (path, script, program) = &files[1];
    assert_eq!((*path, *program), ("Contents/MacOS/play", true));
    assert!(String::from_utf8(script.clone()).unwrap().ends_with(
        "exec '/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher' play \
         --instance 'it'\\''s'\n"
    ));
    assert_eq!(files[2].1, MAC_ICON);
}

/// Reads a Shell Link back by its spec: the header, the LinkInfo's paths, and the strings.
fn read_lnk(b: &[u8]) -> (u32, String, String, Vec<String>) {
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    assert_eq!(u32_at(0), 0x4C);
    assert_eq!(
        &b[4..20],
        &[1, 0x14, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46]
    );
    let flags = u32_at(20);
    assert_eq!(u32_at(60), 1, "show normal");
    let info = 76;
    let size = u32_at(info) as usize;
    assert_eq!(u32_at(info + 4), 0x24);
    assert_eq!(u32_at(info + 8), 1);
    let volume = info + u32_at(info + 12) as usize;
    assert_eq!((u32_at(volume), u32_at(volume + 4)), (0x11, 3));
    let cstr = |at: usize| {
        let end = b[at..].iter().position(|&c| c == 0).unwrap();
        String::from_utf8(b[at..at + end].to_vec()).unwrap()
    };
    let ansi = cstr(info + u32_at(info + 16) as usize);
    assert_eq!(cstr(info + u32_at(info + 24) as usize), "");
    let wide_at = info + u32_at(info + 28) as usize;
    let wide: Vec<u16> = b[wide_at..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let wide = String::from_utf16(&wide).unwrap();
    assert_eq!(
        ansi.replace('?', ""),
        wide.replace(|c: char| !c.is_ascii(), "")
    );
    let mut at = info + size;
    let mut strings = Vec::new();
    for _ in 0..4 {
        let n = u16::from_le_bytes([b[at], b[at + 1]]) as usize;
        let units: Vec<u16> = b[at + 2..at + 2 + 2 * n]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        strings.push(String::from_utf16(&units).unwrap());
        at += 2 + 2 * n;
    }
    assert_eq!(&b[at..], &[0, 0, 0, 0], "the terminal block ends it");
    (flags, ansi, wide, strings)
}

#[test]
fn a_windows_shortcut_is_a_shell_link_to_the_launcher() {
    let exe = r"C:\Users\Zoé\AppData\Local\Horizon\horizon-launcher.exe";
    let b = lnk(exe, &windows_args("port:atm"), "Play ATM in Horizon");
    let (flags, ansi, wide, strings) = read_lnk(&b);
    assert_eq!(flags, 0x02 | 0x04 | 0x10 | 0x20 | 0x40 | 0x80);
    assert_eq!(
        ansi,
        r"C:\Users\Zo?\AppData\Local\Horizon\horizon-launcher.exe"
    );
    assert_eq!(wide, exe);
    assert_eq!(
        strings,
        [
            "Play ATM in Horizon",
            r"C:\Users\Zoé\AppData\Local\Horizon",
            "play --instance \"port:atm\"",
            exe,
        ]
    );
}

/// The shell-written shortcut keeps the instance's arguments, so an uninstall finds it: `plays`
/// reads them from the file's Unicode strings.
#[cfg(windows)]
#[test]
fn the_shell_written_link_keeps_the_instance_it_plays() {
    let root = scratch("plays");
    let exe = root.join("Horizon Launcher").join("horizon-launcher.exe");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, b"x").unwrap();
    let link = root.join("Hardcore (Horizon).lnk");
    write_windows_link(
        &link,
        &exe,
        &windows_args("hardcore"),
        "Play Hardcore in Horizon",
    )
    .unwrap();
    assert!(plays(&link, "hardcore"));
    assert!(!plays(&link, "another"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn shortcuts_are_made_found_renamed_and_removed() {
    let root = scratch("make");
    let (desktop, menu, trash) = (
        root.join("Desktop"),
        root.join("applications"),
        root.join("trash"),
    );
    let bin = trash.clone();
    let trash_fn = move |p: &Path| desktop_seams::to_folder(p, &bin);
    let t: Trash = &trash_fn;
    // A path the shell accepts on the system running the test: Windows writes its shortcuts through
    // IShellLinkW, which refuses a Unix path.
    let exe = if cfg!(windows) {
        Path::new(r"C:\Horizon\horizon-launcher.exe")
    } else {
        Path::new("/opt/horizon-launcher")
    };
    for system in [System::Linux, System::Mac, System::Windows] {
        let made = make_in(
            system,
            &desktop,
            Some(&menu),
            exe,
            "hardcore",
            "Hardcore",
            t,
        )
        .unwrap();
        assert_eq!(made[0], desktop.join(file_name("Hardcore", system)));
        assert_eq!(find_in(&desktop, Some(&menu), "hardcore"), {
            let mut m = made.clone();
            m.sort();
            m
        });
        // Again: rewritten in place, nothing in the trash.
        make_in(
            system,
            &desktop,
            Some(&menu),
            exe,
            "hardcore",
            "Hardcore",
            t,
        )
        .unwrap();
        assert!(!trash.exists());
        // Renamed: the old one goes, the new one is there.
        let made = make_in(system, &desktop, Some(&menu), exe, "hardcore", "Hard", t).unwrap();
        assert!(made[0].ends_with(file_name("Hard", system)));
        assert!(!desktop.join(file_name("Hardcore", system)).exists());
        assert!(find_in(&desktop, Some(&menu), "skyblock").is_empty());
        let gone = remove_in(&desktop, Some(&menu), "hardcore", t).unwrap();
        assert_eq!(gone.len(), made.len());
        assert!(find_in(&desktop, Some(&menu), "hardcore").is_empty());
        fs::remove_dir_all(&trash).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let made = make_in(System::Mac, &desktop, None, exe, "a", "A", t).unwrap();
        let mode = fs::metadata(made[0].join("Contents/MacOS/play"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
    // Someone else's file of that name stays.
    fs::write(desktop.join("Mine (Horizon).desktop"), "not ours").unwrap();
    let err = make_in(System::Linux, &desktop, None, exe, "mine", "Mine", t).unwrap_err();
    assert!(err.contains("isn't this instance's shortcut"), "{err}");
    fs::remove_dir_all(&root).unwrap();
}
