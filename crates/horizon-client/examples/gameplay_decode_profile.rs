//! Profile the two mandatory passes over gameplay-facts.json independently.
//! Build with `cargo build --release -p horizon-client --example gameplay_decode_profile`.
//! Each invocation runs one phase in a separate process for CPU/RSS measurements.
extern crate horizon_client_lib as horizon_client;
#[allow(dead_code)]
#[path = "../../horizon-items/src/blocks/decode.rs"]
mod decode;

use std::{hint::black_box, path::Path, time::Instant};

// The included private decoder expects the parent crate Result alias.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected resource directory")?;
    let mode = args.next().ok_or("expected validate or typed")?;
    if mode != "validate" && mode != "typed" {
        return Err("expected validate or typed".into());
    }
    let total = Instant::now();
    let bytes = std::fs::read(Path::new(&root).join("gameplay-facts.json"))?;
    let started = Instant::now();
    if mode == "validate" {
        let parsed: decode::Validate = serde_json::from_slice(&bytes)?;
        black_box(parsed);
    } else {
        let parsed: decode::Gameplay<'_> = serde_json::from_slice(&bytes)?;
        black_box(parsed);
    }
    println!("phase_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    println!("total_ms={:.3}", total.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
