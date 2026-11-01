//! crate_demo's post-build step: the launcher runs it once the game is built, in the new build's
//! folder. What it writes in HORIZON_NATIVE_DATA_DIR, the game reads (`native_built_data`);
//! HORIZON_NATIVE_CACHE_DIR stays from one build to the next.
use std::{env, fs, path::PathBuf};

fn main() {
    let dir =
        |name: &str| PathBuf::from(env::var_os(name).unwrap_or_else(|| panic!("{name} unset")));
    let (data, cache) = (
        dir("HORIZON_NATIVE_DATA_DIR"),
        dir("HORIZON_NATIVE_CACHE_DIR"),
    );
    let runs = fs::read_to_string(cache.join("runs"))
        .ok()
        .and_then(|r| r.trim().parse().ok())
        .unwrap_or(0u32)
        + 1;
    fs::write(cache.join("runs"), runs.to_string()).expect("the cache folder");
    let baked = format!("{:08x} (run {runs})", crate_demo_engine::checksum(b"baked"));
    fs::write(data.join("baked.txt"), &baked).expect("the data folder");
    println!("crate_demo baked {baked}");
}
