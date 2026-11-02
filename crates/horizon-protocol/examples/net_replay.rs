//! Replays the play packets of a `horizon-bots --capture` file through the client's receive path on
//! one thread, and prints the CPU each stage takes and what each packet type costs to decode:
//!
//!   horizon-bots --server 127.0.0.1:25565 --bots 1 --behaviour explore --view-distance 32 \
//!       --seconds 60 --capture join.cap
//!   cargo run --release -p horizon-protocol --example net_replay -- join.cap
//!
//! The packets go through a connection three times, as they reach the client:
//! - `memory`: single player (an integrated server's pipe: no compression, no encryption);
//! - `tcp`: a server's default compression threshold (256), as offline servers send it;
//! - `tcp+aes`: the same encrypted, as online-mode servers send it.
//!
//! Each pass sends every packet into an in-memory pipe first, then times `ConnReader::recv` for all of
//! them (pipe read, decryption, framing, inflate). `decode` is `play::Clientbound::decode`, and
//! `sections` parses chunk packets' sections (overworld: 24 sections; `--biome-bits` from the server's
//! biome registry, 7 for vanilla's 65 biomes). `--repeat N` keeps each stage's fastest of N runs.
use horizon_protocol::{
    Direction, State,
    buf::write_varint,
    connection::{Connection, Pipe},
    ids,
    packets::play::Clientbound,
};
use std::{collections::BTreeMap, error::Error, time::Instant};

struct Packet {
    id: i32,
    body: Vec<u8>,
}

fn load(path: &str) -> Result<Vec<Packet>, Box<dyn Error>> {
    let data = std::fs::read(path)?;
    let mut packets = Vec::new();
    let mut at = 0;
    while at + 12 <= data.len() {
        let id = i32::from_le_bytes(data[at + 4..at + 8].try_into()?);
        let len = u32::from_le_bytes(data[at + 8..at + 12].try_into()?) as usize;
        let Some(payload) = data.get(at + 12..at + 12 + len) else {
            break;
        };
        let mut body = Vec::with_capacity(len + 3);
        write_varint(&mut body, id);
        body.extend_from_slice(payload);
        packets.push(Packet { id, body });
        at += 12 + len;
    }
    Ok(packets)
}

/// Seconds to receive every packet through a connection with this threshold and encryption,
/// and the bytes the connection read.
fn receive(packets: &[Packet], threshold: i32, encrypt: bool) -> (f64, i64, Vec<(i32, Vec<u8>)>) {
    let (a, b) = Pipe::pair();
    let (mut server, mut client) = (Connection::local(b), Connection::local(a));
    server.set_compression(threshold);
    client.set_compression(threshold);
    if encrypt {
        let key = [0x5a; 16];
        server.enable_encryption(&key);
        client.enable_encryption(&key);
    }
    for p in packets {
        server.send(&p.body).expect("the pipe is open");
    }
    let start = Instant::now();
    let received: Vec<_> = (0..packets.len())
        .map(|_| client.recv().expect("every packet sent arrives"))
        .collect();
    let seconds = start.elapsed().as_secs_f64();
    let bytes = client
        .reader
        .counters
        .bytes_in
        .load(std::sync::atomic::Ordering::Relaxed);
    (seconds, bytes, received)
}

#[derive(Default, Clone, Copy)]
struct Cost {
    count: u64,
    bytes: u64,
    nanos: u64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .ok_or("usage: net_replay CAPTURE [--repeat N] [--biome-bits B]")?;
    let flag = |name: &str, default: usize| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let repeat = flag("--repeat", 3).max(1);
    let biome_bits = flag("--biome-bits", 7) as u8;
    let packets = load(path)?;
    let body_bytes: usize = packets.iter().map(|p| p.body.len()).sum();
    let chunk_id = ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT;
    let chunks = packets.iter().filter(|p| p.id == chunk_id).count();
    println!(
        "{path}: {} packets, {:.1} MB of packet bodies, {chunks} chunk packets",
        packets.len(),
        body_bytes as f64 / 1e6
    );

    let mut received = Vec::new();
    println!("\nreceive (ConnReader::recv for every packet, fastest of {repeat}):");
    println!(
        "  {:<8} {:>10} {:>10} {:>12} {:>14}",
        "pass", "wire MB", "ms", "ns/packet", "ns/wire byte"
    );
    for (name, threshold, encrypt) in [
        ("memory", -1, false),
        ("tcp", 256, false),
        ("tcp+aes", 256, true),
    ] {
        let mut best = f64::MAX;
        let mut wire = 0;
        for _ in 0..repeat {
            let (seconds, bytes, got) = receive(&packets, threshold, encrypt);
            best = best.min(seconds);
            wire = bytes;
            received = got;
        }
        println!(
            "  {name:<8} {:>10.1} {:>10.1} {:>12.0} {:>14.2}",
            wire as f64 / 1e6,
            best * 1e3,
            best * 1e9 / packets.len() as f64,
            best * 1e9 / wire.max(1) as f64
        );
    }

    // Decode, per packet type: the fastest of the runs for each type.
    let mut costs: BTreeMap<i32, Cost> = BTreeMap::new();
    let mut sections = Cost::default();
    let mut errors = 0;
    for run in 0..repeat {
        let mut this: BTreeMap<i32, Cost> = BTreeMap::new();
        let mut this_sections = Cost::default();
        for (id, payload) in &received {
            let start = Instant::now();
            let packet = Clientbound::decode(*id, payload);
            let nanos = start.elapsed().as_nanos() as u64;
            let c = this.entry(*id).or_default();
            c.count += 1;
            c.bytes += payload.len() as u64;
            c.nanos += nanos;
            match packet {
                Ok(Clientbound::LevelChunkWithLight(p)) => {
                    let start = Instant::now();
                    if p.chunk.sections(24, 16, biome_bits).is_err() && run == 0 {
                        errors += 1;
                    }
                    this_sections.count += 1;
                    this_sections.bytes += p.chunk.buffer.len() as u64;
                    this_sections.nanos += start.elapsed().as_nanos() as u64;
                }
                Err(_) if run == 0 => errors += 1,
                _ => {}
            }
        }
        for (id, c) in this {
            let best = costs.entry(id).or_insert(c);
            if c.nanos < best.nanos {
                *best = c;
            }
        }
        if run == 0 || this_sections.nanos < sections.nanos {
            sections = this_sections;
        }
    }
    let mut rows: Vec<_> = costs.into_iter().collect();
    rows.sort_by_key(|(_, c)| std::cmp::Reverse(c.nanos));
    let total: u64 = rows.iter().map(|(_, c)| c.nanos).sum();
    println!(
        "\ndecode (play::Clientbound::decode): {:.1} ms for {} packets, {errors} errors",
        total as f64 / 1e6,
        packets.len()
    );
    println!(
        "  {:<34} {:>8} {:>10} {:>10} {:>10}",
        "packet", "count", "MB", "ms", "ns/packet"
    );
    for (id, c) in rows.iter().take(20) {
        let name = ids::name(State::Play, Direction::Clientbound, *id).unwrap_or("?");
        println!(
            "  {name:<34} {:>8} {:>10.2} {:>10.2} {:>10.0}",
            c.count,
            c.bytes as f64 / 1e6,
            c.nanos as f64 / 1e6,
            c.nanos as f64 / c.count as f64
        );
    }
    println!(
        "\nsections (LevelChunkPacketData.sections): {} chunks, {:.1} ms, {:.0} us a chunk",
        sections.count,
        sections.nanos as f64 / 1e6,
        sections.nanos as f64 / 1e3 / sections.count.max(1) as f64
    );
    Ok(())
}
