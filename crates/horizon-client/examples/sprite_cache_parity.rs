//! M-11 gate: block assets loaded through the sprite disk cache are byte-identical
//! to the ones decoded from the PNGs. Deletes the cache file in ROOT, loads (decodes
//! and writes it), loads again (reads it) and compares the sprite arrays.
//! cargo run --profile fast -p horizon-client --example sprite_cache_parity -- ROOT
use horizon_client::assets::{Assets, RenderAssets};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
    time::Instant,
};

fn digest(render: &RenderAssets) -> u64 {
    let mut h = DefaultHasher::new();
    render.pixels.hash(&mut h);
    (
        render.sprite_size,
        render.sprite_minimum_size,
        render.texture_layers,
    )
        .hash(&mut h);
    render.sprite_layers.hash(&mut h);
    render.translucent_layers.hash(&mut h);
    format!("{:?}", render.sprite_mips).hash(&mut h);
    for animation in &render.animations {
        animation.layer.hash(&mut h);
        for frame in 0..animation.frame_count() {
            animation.pixels(frame).hash(&mut h);
        }
        for tick in 0..400 {
            format!("{:?}", animation.blend(tick)).hash(&mut h);
        }
    }
    h.finish()
}

fn main() -> horizon_client::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: sprite_cache_parity ROOT")?;
    let root = Path::new(&root);
    let assets = Assets::load(root)?;
    let _ = std::fs::remove_file(root.join("horizon-sprites.cache"));
    let start = Instant::now();
    let cold = digest(&assets.reload(root)?);
    let cold_ms = start.elapsed().as_secs_f64() * 1000.;
    let size = std::fs::metadata(root.join("horizon-sprites.cache"))?.len();
    let start = Instant::now();
    let warm = digest(&assets.reload(root)?);
    let warm_ms = start.elapsed().as_secs_f64() * 1000.;
    println!(
        "cold={cold:016x} ({cold_ms:.1} ms) warm={warm:016x} ({warm_ms:.1} ms) cache_bytes={size}"
    );
    if cold != warm {
        return Err("cached sprites differ from decoded sprites".into());
    }
    println!("PARITY OK");
    Ok(())
}
