use horizon_core::position::BlockPos;
use horizon_world::{
    chunk::{BlockState, ChunkData},
    compression::Compression,
    nbt::{self, Limits},
    region::{ChunkRecord, RegionFile, RegionWriter},
};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

fn codec(name: &str) -> Result<Compression, Box<dyn Error>> {
    match name {
        "gzip" | "1" => Ok(Compression::Gzip),
        "zlib" | "2" => Ok(Compression::Zlib),
        "none" | "3" => Ok(Compression::None),
        "lz4" | "4" => Ok(Compression::Lz4),
        _ => Err(format!("unknown codec {name}; choose gzip, zlib, none or lz4").into()),
    }
}

fn read_bounded(path: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let max = Limits::default().max_bytes;
    let mut bytes = Vec::new();
    File::open(path)?
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err("input exceeds 64 MiB limit".into());
    }
    Ok(bytes)
}

fn write_new(path: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err("use --help for commands".into());
    };
    match command {
        "--help" | "-h" if args.len() == 1 => println!(
            "Horizon world data tools — Java Edition {}\n\
             inspect <region.mca>\n\
             copy-region <source.mca> <new-directory/r.x.z.mca> [gzip|zlib|none|lz4]\n\
             get-block <region.mca> <x> <y> <z>\n\
             edit-block-data <source.mca> <new-directory/r.x.z.mca> <x> <y> <z> <name> [property=value ...]\n\
             nbt-copy <input> <new-output> <input-codec> <output-codec>\n\
             nbt-network-copy <input> <new-output> <input-codec> <output-codec>\n\
             Existing output files are refused. edit-block-data changes saved block data only;\n\
             it does not run lighting, heightmaps, ticks or neighbor updates.",
            horizon_core::VANILLA_VERSION
        ),
        "nbt-copy" | "nbt-network-copy" if args.len() == 5 => {
            let bytes =
                codec(&args[3])?.decode(&read_bounded(&args[1])?, Limits::default().max_bytes)?;
            let encoded = if command == "nbt-copy" {
                let (root, used) = nbt::read_named(&bytes, Limits::default())?;
                if used != bytes.len() {
                    return Err("trailing bytes after NBT root".into());
                }
                nbt::write_named(&root, Limits::default())?
            } else {
                let (root, used) = nbt::read_network(&bytes, Limits::default())?;
                if used != bytes.len() {
                    return Err("trailing bytes after NBT root".into());
                }
                nbt::write_network(&root, Limits::default())?
            };
            write_new(&args[2], &codec(&args[4])?.encode(&encoded)?)?;
            println!("Wrote {}", args[2]);
        }
        "inspect" if args.len() == 2 => {
            let mut region = RegionFile::open(&args[1])?;
            let chunks: Vec<_> = region.chunks().collect();
            println!(
                "Horizon region {},{}: {} chunks",
                region.position().x,
                region.position().z,
                chunks.len()
            );
            for (pos, timestamp) in chunks {
                let record = region.read(pos)?.unwrap();
                let root = record.nbt(Limits::default())?;
                println!(
                    "chunk {},{} timestamp={} codec={:?} compressed={} root_type={}",
                    pos.x,
                    pos.z,
                    timestamp,
                    record.compression,
                    record.compressed.len(),
                    root.tag.id()
                );
            }
        }
        "copy-region" if args.len() == 3 || args.len() == 4 => {
            let mut region = RegionFile::open(&args[1])?;
            if args.len() == 3 {
                region.export(&args[2], &[])?;
            } else {
                let compression = codec(&args[3])?;
                if horizon_world::region::position_from_path(Path::new(&args[2]))?
                    != region.position()
                {
                    return Err("export region coordinates differ".into());
                }
                let mut writer = RegionWriter::create(&args[2])?;
                let positions: Vec<_> = region.chunks().map(|(p, _)| p).collect();
                for pos in positions {
                    let old = region.read(pos)?.unwrap();
                    let record = ChunkRecord::from_nbt(
                        &old.nbt(Limits::default())?,
                        compression,
                        old.timestamp,
                        Limits::default(),
                    )?;
                    writer.write(pos, &record)?;
                }
                writer.finish()?;
            }
            println!("Wrote {}", args[2]);
        }
        "get-block" if args.len() == 5 => {
            let block = BlockPos::new(args[2].parse()?, args[3].parse()?, args[4].parse()?);
            let record = RegionFile::open(&args[1])?
                .read(block.chunk())?
                .ok_or("chunk is absent")?;
            let chunk = ChunkData::new(record.nbt(Limits::default())?)?;
            let state = chunk
                .block(block)?
                .ok_or("section has no block-state data")?;
            print!("{}", state.name);
            for (name, value) in state.properties {
                print!(" {name}={value}");
            }
            println!();
        }
        "edit-block-data" if args.len() >= 7 => {
            let block = BlockPos::new(args[3].parse()?, args[4].parse()?, args[5].parse()?);
            let mut properties = BTreeMap::new();
            for arg in &args[7..] {
                let (key, value) = arg.split_once('=').ok_or("expected property=value")?;
                if key.is_empty()
                    || value.is_empty()
                    || properties
                        .insert(key.to_owned(), value.to_owned())
                        .is_some()
                {
                    return Err("invalid or duplicate block property".into());
                }
            }
            let mut region = RegionFile::open(&args[1])?;
            let old = region.read(block.chunk())?.ok_or("chunk is absent")?;
            let mut chunk = ChunkData::new(old.nbt(Limits::default())?)?;
            chunk.set_block_data(
                block,
                BlockState {
                    name: args[6].clone(),
                    properties: properties.into(),
                },
            )?;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as u32;
            let next = ChunkRecord::from_nbt(
                &chunk.into_nbt(),
                old.compression,
                timestamp,
                Limits::default(),
            )?;
            region.export(&args[2], &[(block.chunk(), Some(next))])?;
            println!(
                "Wrote saved block data to {}; simulation-derived data has not been updated.",
                args[2]
            );
        }
        _ => return Err("invalid command or arguments; use --help".into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("Horizon: {error}");
        std::process::exit(2);
    }
}
