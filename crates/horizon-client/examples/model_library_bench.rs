//! Full block/assets + item startup. Compile with HORIZON_BENCH_COUNT_ALLOCATIONS=1
//! for live requested heap bytes; leave it unset for latency measurements.
//! cargo run --profile fast -p horizon-client --example model_library_bench -- ROOT [--parity]
//! Run fresh processes, warm up twice, then compare multiple alternating baseline/candidate runs.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{assets::Assets, item_render::ItemRenderer};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hash::{Hash, Hasher},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::Instant,
};

struct CountingAllocator;
const TRACK: bool = option_env!("HORIZON_BENCH_COUNT_ALLOCATIONS").is_some();
static LIVE: AtomicUsize = AtomicUsize::new(0);
// The allocator delegates every operation and layout unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if TRACK && !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if TRACK && !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let out = unsafe { System.realloc(ptr, layout, size) };
        if TRACK && !out.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            LIVE.fetch_add(size, Relaxed);
        }
        out
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() -> horizon_client::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: model_library_bench ROOT [--parity]")?;
    let root = Path::new(&root);
    let initial = LIVE.load(Relaxed);
    let start = Instant::now();
    let assets = Assets::load(root)?;
    let render = assets.reload(root)?;
    let block_ms = start.elapsed().as_secs_f64() * 1000.;
    let block_retained = LIVE.load(Relaxed) - initial;
    let start_items = Instant::now();
    let items = ItemRenderer::load_with_library(root, render.model_library.clone())?;
    let item_ms = start_items.elapsed().as_secs_f64() * 1000.;
    let total_ms = start.elapsed().as_secs_f64() * 1000.;
    let retained = LIVE.load(Relaxed) - initial;
    println!(
        "block_ms={block_ms:.3} item_ms={item_ms:.3} total_ms={total_ms:.3} retained_bytes={retained} block_retained_bytes={block_retained}"
    );
    if std::env::args().any(|a| a == "--parity") {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for id in 0..assets.registry.len() {
            let id = horizon_world::registry::BlockStateId(id as u32);
            for pos in [
                horizon_core::position::BlockPos::new(0, 0, 0),
                horizon_core::position::BlockPos::new(-17, 65, 31),
            ] {
                if let Ok(selected) = render.model_catalog.collect(id, pos) {
                    for (variant, model) in selected {
                        format!("{variant:?}").hash(&mut hash);
                        model.ambient_occlusion.hash(&mut hash);
                        for face in &model.faces {
                            format!("{:?}", face.quad).hash(&mut hash);
                            face.layer.hash(&mut hash);
                            face.opaque.hash(&mut hash);
                            face.translucent.hash(&mut hash);
                        }
                    }
                }
            }
        }
        let mut names = Vec::new();
        for ns in std::fs::read_dir(root.join("assets"))? {
            let ns = ns?;
            let base = ns.path().join("models");
            let mut stack = vec![base.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in entries {
                    let path = entry?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "json") {
                        names.push(format!(
                            "{}:{}",
                            ns.file_name().to_string_lossy(),
                            path.strip_prefix(&base)?
                                .with_extension("")
                                .to_string_lossy()
                                .replace('\\', "/")
                        ));
                    }
                }
            }
        }
        names.sort();
        for name in &names {
            format!("{:?}", items.bake(name)).hash(&mut hash);
        }
        println!(
            "parity_models={} fingerprint={:016x}",
            names.len(),
            hash.finish()
        );
    }
    std::hint::black_box((&assets, &items));
    Ok(())
}
