//! Makes a world that holds chunks "an older version generated": copies the
//! overworld chunks of a rectangle from one world into another, each with a
//! `blending_data` compound as the 1.18 data fixer leaves on a pre-1.18
//! chunk, and drops every other chunk of the destination, so that the chunks
//! generated around them blend with them (`Blender.of`).
//!
//! old_world_fixture <source world> <destination world> <x0,z0,x1,z1>
extern crate horizon_client_lib as horizon_client;
use horizon_core::position::ChunkPos;
use horizon_world::{
    nbt::{Limits, Tag},
    region::{ChunkRecord, RegionFile},
};
use std::{fs, path::Path};

type Error = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let source = args.next().ok_or("source world")?;
    let destination = args.next().ok_or("destination world")?;
    let rect: Vec<i32> = args
        .next()
        .ok_or("x0,z0,x1,z1")?
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    let [x0, z0, x1, z1]: [i32; 4] = rect.try_into().map_err(|_| "x0,z0,x1,z1")?;
    let overworld = |world: &str| {
        Path::new(world)
            .join("dimensions")
            .join("minecraft")
            .join("overworld")
            .join("region")
    };
    let (source, destination) = (overworld(&source), overworld(&destination));
    fs::create_dir_all(&destination)?;
    for entry in fs::read_dir(&destination)?.flatten() {
        if entry.path().extension().is_some_and(|e| e == "mca") {
            fs::remove_file(entry.path())?;
        }
    }
    let mut copied = 0;
    for z in z0..=z1 {
        for x in x0..=x1 {
            let pos = ChunkPos::new(x, z);
            let path = source.join(format!("r.{}.{}.mca", x >> 5, z >> 5));
            let Ok(mut region) = RegionFile::open(&path) else {
                continue;
            };
            let Some(record) = region.read(pos)? else {
                continue;
            };
            let mut root = record.nbt(Limits::saved())?;
            if !matches!(root.tag.get("Status"), Some(Tag::String(s)) if s.to_string_lossy().ends_with("full"))
            {
                continue;
            }
            if let Tag::Compound(chunk) = &mut root.tag {
                chunk.insert(
                    "blending_data".into(),
                    Tag::compound([("min_section", Tag::Int(-4)), ("max_section", Tag::Int(20))]),
                );
            }
            let out = destination.join(format!("r.{}.{}.mca", x >> 5, z >> 5));
            let mut target = RegionFile::open_writable(&out, true)?;
            target.write(
                pos,
                &ChunkRecord::from_nbt(
                    &root,
                    record.compression,
                    record.timestamp,
                    Limits::saved(),
                )?,
            )?;
            copied += 1;
        }
    }
    println!("{copied} chunks copied");
    Ok(())
}
