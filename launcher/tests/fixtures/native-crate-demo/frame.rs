// The first frame: shows that the crate, both data folders and the post-build step reached the
// game, in the log and in the mod's folder (`horizon-mods/.horizon-mod-data/crate_demo/proof.txt`).
fn crate_demo_frame(game: &mut crate::game::Game<'_>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    // The bundle's data/greeting.txt, compiled in.
    let greeting = horizon_client::native_data!("crate_demo", "greeting.txt");
    // What the post-build step wrote when the launcher built this game.
    let baked = match game.native_built_data("crate_demo") {
        Some(dir) => std::fs::read_to_string(dir.join("baked.txt"))
            .unwrap_or_else(|e| format!("missing ({e})")),
        None => "missing (the game's folder is unknown)".to_owned(),
    };
    let line = format!(
        "crate_demo: {} checksum {:08x}, baked {}",
        String::from_utf8_lossy(greeting).trim(),
        crate_demo_engine::checksum(greeting),
        baked.trim()
    );
    eprintln!("{line}");
    let dir = game.native_data_dir("crate_demo");
    let _ =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join("proof.txt"), &line));
}
