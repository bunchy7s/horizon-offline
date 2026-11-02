//! How long a big resource pack takes to load and reload, on generated files only (no game
//! resources): a stand-in vanilla root, a server-style zip pack of many namespaces, and a
//! high-resolution pack over vanilla's own textures. Each stage runs a few times; the median
//! is printed as a Markdown table.
//! `cargo run --profile fast -p horizon-client-render --example pack_load_bench [-- --scale 1]`
//! (`.github/workflows/pack-bench.yml` runs it on a branch and on its base, side by side).
use horizon_client_render::{
    hud::HudAtlas,
    model_library::ParsedModels,
    packs::{self, Pack},
    sprites::Sprites,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

/// A small deterministic generator: the same files on every run and every branch.
struct Noise(u64);
impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A `side`x`side` RGBA PNG (`frames` tall when animated) of blotchy pixels, which compress
/// about as well as real textures do.
fn png(noise: &mut Noise, side: u32, frames: u32, alpha: bool) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, side, side * frames);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let mut pixels = Vec::with_capacity((side * side * frames * 4) as usize);
        let mut colour = [0u8; 4];
        for i in 0..side * side * frames {
            if i % 3 == 0 {
                let n = noise.next();
                colour = [n as u8, (n >> 8) as u8, (n >> 16) as u8, 255];
                if alpha && n >> 40 & 3 == 0 {
                    colour[3] = 0;
                }
            }
            pixels.extend_from_slice(&colour);
        }
        writer.write_image_data(&pixels).unwrap();
    }
    out
}

fn model(noise: &mut Noise, namespace: &str, texture: &str) -> Vec<u8> {
    let n = noise.next() % 4;
    format!(
        r##"{{"parent":"minecraft:block/cube_all","textures":{{"all":"{namespace}:{texture}"}},"elements":[{{"from":[0,0,0],"to":[16,{h},16],"faces":{{"north":{{"texture":"#all","uv":[0,0,16,16]}},"up":{{"texture":"#all"}}}}}}]}}"##,
        h = 4 + n * 4
    )
    .into_bytes()
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A deflated zip written by hand (the reader skips CRCs, so they stay zero), at most 65535
/// entries.
#[derive(Default)]
struct ZipOut {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}
impl ZipOut {
    fn put(&mut self, name: &str, body: &[u8]) {
        let data = miniz_oxide::deflate::compress_to_vec(body, 6);
        let offset = u32::try_from(self.out.len()).unwrap();
        let sizes = [data.len() as u32, body.len() as u32];
        self.out.extend(0x0403_4b50u32.to_le_bytes());
        self.out.extend([20, 0, 0, 0, 8, 0]);
        self.out.extend([0; 8]);
        sizes.iter().for_each(|s| self.out.extend(s.to_le_bytes()));
        self.out.extend((name.len() as u16).to_le_bytes());
        self.out.extend([0; 2]);
        self.out.extend(name.as_bytes());
        self.out.extend(&data);
        self.central.extend(0x0201_4b50u32.to_le_bytes());
        self.central.extend([20, 0, 20, 0, 0, 0, 8, 0]);
        self.central.extend([0; 8]);
        sizes
            .iter()
            .for_each(|s| self.central.extend(s.to_le_bytes()));
        self.central.extend((name.len() as u16).to_le_bytes());
        self.central.extend([0; 12]);
        self.central.extend(offset.to_le_bytes());
        self.central.extend(name.as_bytes());
        self.count = self.count.checked_add(1).expect("too many zip entries");
    }
    fn finish(mut self, path: &Path) {
        let start = self.out.len() as u32;
        self.out.extend(&self.central);
        self.out.extend(0x0605_4b50u32.to_le_bytes());
        self.out.extend([0; 4]);
        self.out.extend(self.count.to_le_bytes());
        self.out.extend(self.count.to_le_bytes());
        self.out.extend((self.central.len() as u32).to_le_bytes());
        self.out.extend(start.to_le_bytes());
        self.out.extend([0; 2]);
        fs::write(path, self.out).unwrap();
    }
}

const META: &[u8] = br#"{"pack":{"description":"bench","min_format":[69,0],"max_format":[999,0]}}"#;

/// The stand-in game resources: vanilla-sized texture folders, models and the stacked JSON
/// files (lang, sounds, fonts, atlases) every reload merges.
fn vanilla(root: &Path, scale: usize, noise: &mut Noise) -> Vec<String> {
    let mut blocks = Vec::new();
    for i in 0..1100 * scale {
        let name = format!("block/b{i}");
        let frames = if i % 50 == 0 { 8 } else { 1 };
        write(
            root,
            &format!("assets/minecraft/textures/{name}.png"),
            &png(noise, 16, frames, i % 7 == 0),
        );
        if frames > 1 {
            write(
                root,
                &format!("assets/minecraft/textures/{name}.png.mcmeta"),
                br#"{"animation":{"frametime":2}}"#,
            );
        }
        write(
            root,
            &format!("assets/minecraft/models/block/b{i}.json"),
            &model(noise, "minecraft", &name),
        );
        blocks.push(format!("minecraft:{name}"));
    }
    for (folder, count, side) in [("item", 1500, 16), ("entity", 900, 64), ("gui", 800, 16)] {
        for i in 0..count * scale {
            let sub = if folder == "gui" { "sprites/" } else { "" };
            write(
                root,
                &format!(
                    "assets/minecraft/textures/{folder}/{sub}{}/t{i}.png",
                    i % 20
                ),
                &png(noise, side, 1, true),
            );
        }
    }
    for i in 0..1500 * scale {
        write(
            root,
            &format!("assets/minecraft/models/item/i{i}.json"),
            &model(noise, "minecraft", &format!("item/{}/t{i}", i % 20)),
        );
    }
    let lang: String = (0..4000)
        .map(|i| format!("\"key.{i}\":\"Value {i}\""))
        .collect::<Vec<_>>()
        .join(",");
    write(
        root,
        "assets/minecraft/lang/en_us.json",
        format!("{{{lang}}}").as_bytes(),
    );
    write(
        root,
        "assets/minecraft/sounds.json",
        br#"{"a":{"sounds":["a"]}}"#,
    );
    write(
        root,
        "assets/minecraft/atlases/blocks.json",
        br#"{"sources":[{"type":"directory","source":"block","prefix":"block/"}]}"#,
    );
    write(root, "pack.mcmeta", META);
    blocks
}

/// A server pack the way ItemsAdder-style plugins make them: one zip of many namespaces, each
/// with its own items' textures and models, its sounds and its lang files.
fn server_zip(path: &Path, scale: usize, noise: &mut Noise) {
    let mut zip = ZipOut::default();
    zip.put("pack.mcmeta", META);
    for n in 0..190 {
        let ns = format!("plugin{n}");
        for i in 0..50 * scale {
            let texture = format!("item/{}/t{i}", i % 5);
            zip.put(
                &format!("assets/{ns}/textures/{texture}.png"),
                &png(noise, 32, 1, true),
            );
            zip.put(
                &format!("assets/{ns}/models/item/{}/m{i}.json", i % 5),
                &model(noise, &ns, &texture),
            );
            if i % 10 == 0 {
                zip.put(&format!("assets/{ns}/sounds/s{i}.ogg"), &[0; 2048]);
            }
        }
        zip.put(
            &format!("assets/{ns}/lang/en_us.json"),
            br#"{"item.custom":"Custom"}"#,
        );
    }
    let lang: String = (0..2000)
        .map(|i| format!("\"server.{i}\":\"Server {i}\""))
        .collect::<Vec<_>>()
        .join(",");
    zip.put(
        "assets/minecraft/lang/en_us.json",
        format!("{{{lang}}}").as_bytes(),
    );
    zip.finish(path);
}

/// A 64x pack over vanilla's own block, item and entity textures, with OptiFine CTM-style
/// tiles beside them: what the big OptiFine packs look like on disk.
fn hd_zip(path: &Path, scale: usize, noise: &mut Noise) {
    let mut zip = ZipOut::default();
    zip.put("pack.mcmeta", META);
    for i in 0..1100 * scale {
        zip.put(
            &format!("assets/minecraft/textures/block/b{i}.png"),
            &png(noise, 64, 1, i % 7 == 0),
        );
        if i % 4 == 0 {
            for tile in 0..12 {
                zip.put(
                    &format!("assets/minecraft/optifine/ctm/b{i}/{tile}.png"),
                    &png(noise, 64, 1, false),
                );
            }
            zip.put(
                &format!("assets/minecraft/optifine/ctm/b{i}/b{i}.properties"),
                format!("matchTiles=b{i}\nmethod=ctm\ntiles=0-11\n").as_bytes(),
            );
        }
    }
    for i in 0..750 * scale {
        zip.put(
            &format!("assets/minecraft/textures/item/{}/t{i}.png", i % 20),
            &png(noise, 64, 1, true),
        );
    }
    zip.finish(path);
}

fn median(mut ms: Vec<f64>) -> f64 {
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

/// `f`'s median time over `runs`, with `before` (not timed) ahead of each run.
fn time(runs: usize, mut before: impl FnMut(), mut f: impl FnMut()) -> f64 {
    median(
        (0..runs)
            .map(|_| {
                before();
                let started = Instant::now();
                f();
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}

fn zip_pack(id: &str, path: PathBuf) -> Pack {
    // Read the way `packs::folder` reads a `resourcepacks/` zip.
    let dir = path.parent().unwrap().join(format!("{id}-dir"));
    fs::create_dir_all(&dir).unwrap();
    let at = dir.join(path.file_name().unwrap());
    fs::rename(&path, &at).unwrap();
    packs::folder(&dir, packs::RESOURCE_FORMAT)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{id} is not a pack"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale = args
        .iter()
        .position(|a| a == "--scale")
        .and_then(|i| args.get(i + 1))
        .map_or(1, |s| s.parse().unwrap());
    let runs = 3;
    let scratch = std::env::temp_dir().join(format!("horizon-pack-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15);
    let vanilla_root = scratch.join("vanilla");
    let blocks = vanilla(&vanilla_root, scale, &mut noise);
    server_zip(&scratch.join("server.zip"), scale, &mut noise);
    hd_zip(&scratch.join("hd.zip"), scale, &mut noise);
    let server = zip_pack("server", scratch.join("server.zip"));
    let hd = zip_pack("hd", scratch.join("hd.zip"));
    let cache = scratch.join("cache");
    let both = [hd.clone(), server.clone()];

    let mut rows: Vec<(String, f64)> = Vec::new();
    let mut row = |name: &str, ms: f64| {
        eprintln!("{name}: {ms:.1} ms");
        rows.push((name.to_owned(), ms));
    };
    let fresh = || {
        let _ = fs::remove_dir_all(&cache);
    };
    row(
        "overlay: build, both packs (cold)",
        time(runs, fresh, || {
            packs::resource_root(&vanilla_root, &both, &cache).unwrap();
        }),
    );
    let root = packs::resource_root(&vanilla_root, &both, &cache).unwrap();
    row(
        "overlay: reuse, both packs (cache hit)",
        time(
            runs,
            || {},
            || {
                packs::resource_root(&vanilla_root, &both, &cache).unwrap();
            },
        ),
    );
    // The pack screen: one pack turned off, then on again.
    row(
        "overlay: toggle the server pack off (new selection)",
        time(
            runs,
            || {
                let only =
                    packs::resource_root(&vanilla_root, std::slice::from_ref(&hd), &cache).unwrap();
                fs::remove_dir_all(&only).unwrap();
            },
            || {
                packs::resource_root(&vanilla_root, std::slice::from_ref(&hd), &cache).unwrap();
            },
        ),
    );
    row(
        "models: parse every model",
        time(
            runs,
            || {},
            || {
                std::hint::black_box(ParsedModels::load(&root));
            },
        ),
    );
    let sprite_cache = root.join("horizon-sprites.cache");
    // OptiFine connected-texture tiles, inserted by their path as `ConnectedTextures` does.
    let tiles: Vec<String> = (0..1100 * scale)
        .step_by(4)
        .flat_map(|i| (0..12).map(move |t| format!("assets/minecraft/optifine/ctm/b{i}/{t}.png")))
        .collect();
    let sprites = |cold: bool, prefetch: bool, names: &[String], files: bool| {
        time(
            runs,
            || {
                if cold {
                    let _ = fs::remove_file(&sprite_cache);
                }
            },
            || {
                let mut sprites = Sprites::with_disk_cache(&root);
                if prefetch {
                    sprites.prefetch(&root, names.iter().map(String::as_str));
                }
                for name in names {
                    if files {
                        sprites.insert_file(&root, name).unwrap();
                    } else {
                        sprites.insert(&root, name).unwrap();
                    }
                }
                sprites.save_disk_cache();
                sprites.finish();
                std::hint::black_box(&sprites.pixels);
            },
        )
    };
    row(
        "block sprites: one by one (no cache)",
        sprites(true, false, &blocks, false),
    );
    row(
        "block sprites: prefetched (no cache)",
        sprites(true, true, &blocks, false),
    );
    row(
        "block sprites: from the disk cache",
        sprites(false, false, &blocks, false),
    );
    row("CTM tiles: one by one", sprites(false, false, &tiles, true));
    row("CTM tiles: prefetched", sprites(false, true, &tiles, true));
    row(
        "GUI atlas: decode and stitch, both packs",
        time(
            runs,
            || {},
            || {
                if let Err(e) = HudAtlas::build_over(&root, &vanilla_root) {
                    eprintln!("GUI atlas failed: {e}");
                }
            },
        ),
    );
    row(
        "GUI atlas: vanilla alone",
        time(
            runs,
            || {},
            || {
                let _ = HudAtlas::build(&vanilla_root);
            },
        ),
    );

    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("| Stage (scale {scale}, {cores} threads, median of {runs}) | ms |");
    println!("|---|---:|");
    for (name, ms) in &rows {
        println!("| {name} | {ms:.0} |");
    }
    let _ = fs::remove_dir_all(&scratch);
}
