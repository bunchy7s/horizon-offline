//! Resident memory of each resource loader, in the game's start order: what a
//! larger registry (content mods, docs/modpacks.md) costs, loader by loader.
//!
//! resource_memory RESOURCES
//!
//! RESOURCES is a prepared root or a content-mod root (`.horizon-cache/mod-content`).
//! Prints, after each step, the live heap (a counting allocator: what the
//! step keeps), the heap's peak during the step (what it parsed on the way)
//! and the process RSS (what the allocator holds on to). On macOS it ends
//! with `footprint`'s physical footprint, the number the OS charges: RSS also
//! counts freed pages the allocator marked reusable.
use horizon_client::{
    assets::Assets, biome::Biomes, blocks::BlockFacts, collisions, crafting::Recipes, items::Items,
    light::LightTables, mesher::MeshTables, models, motion::MotionResources,
    vanillagen::VanillaTerrain,
};
use horizon_world::loot::PackOverlay;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};
use std::{path::PathBuf, process::Command};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
// SAFETY: forwards to the system allocator, only counting sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new) };
        if !p.is_null() {
            if new >= layout.size() {
                let now = LIVE.fetch_add(new - layout.size(), Relaxed) + new - layout.size();
                PEAK.fetch_max(now, Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new, Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static ALLOC: Counting = Counting;
const MB: f64 = 1048576.0;

fn rss_mb() -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: resource_memory RESOURCES")?,
    );
    println!(
        "{:<24} {:>8} {:>8} {:>10} {:>8}",
        "step", "live MB", "+live", "step peak", "RSS MB"
    );
    let mut last = 0usize;
    let mut step = |name: &str| {
        let live = LIVE.load(Relaxed);
        let peak = PEAK.swap(live, Relaxed);
        println!(
            "{name:<24} {:>8.0} {:>+8.0} {:>10.0} {:>8.0}",
            live as f64 / MB,
            (live as f64 - last as f64) / MB,
            peak as f64 / MB,
            rss_mb()
        );
        last = live;
    };
    step("start");
    let registry = horizon_world::level::data::load_registry(&root)?;
    step("registry alone");
    drop(registry);
    step("registry dropped");
    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    step("assets");
    println!(
        "  sprites: {} layers of {}px, {:.0} MB of pixels",
        render.texture_layers,
        render.sprite_size,
        render.pixels.len() as f64 / MB
    );
    // The game decodes gameplay facts once for both tables (BlockFacts::load_pair).
    let (facts, behaviours) = BlockFacts::load_pair(&root, assets.registry)?;
    step("block facts+behaviours");
    let items = Items::load(&root, assets.registry, &facts, &PackOverlay::NONE)?;
    step("items");
    let recipes = Recipes::load(&root, &items, &PackOverlay::NONE)?;
    step("recipes");
    let biomes = Biomes::load(&root)?;
    step("biomes");
    let light = LightTables::load(&root, assets.registry, &facts)?;
    step("light tables");
    let tables = MeshTables::new(
        &assets,
        &render,
        &facts,
        models::load_visibility(&root, assets.registry)?,
    )?;
    step("mesh tables");
    let collision = collisions::load(&root, assets.registry)?;
    step("collision");
    let motion = MotionResources::load_with_facts(&root, &assets, &facts)?;
    step("motion");
    let terrain = VanillaTerrain::new(&root.join("worldgen"), assets.registry, &biomes, 1234)?;
    step("worldgen");
    println!(
        "{} block states; live {:.0} MB, RSS {:.0} MB",
        facts.states.len(),
        LIVE.load(Relaxed) as f64 / MB,
        rss_mb()
    );
    if let Ok(out) = Command::new("footprint")
        .arg(std::process::id().to_string())
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text
            .lines()
            .filter(|l| l.contains("Footprint") || l.contains("phys_footprint"))
        {
            println!("  {}", line.trim());
        }
    }
    // Keep everything alive until the end.
    drop((
        terrain, behaviours, motion, collision, tables, light, biomes, recipes, items, facts,
        assets,
    ));
    Ok(())
}
