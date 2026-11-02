#![forbid(unsafe_code)]
//! Runs tools/test_fire_starter_sound_parity.py's cases through the fire
//! starter's use sound (`horizon_client::items::fire_starter_pitch`).
use horizon_client::{
    Result,
    items::{fire_starter_pitch, fire_starter_sound},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let line = line?;
        let mut parts = line.trim().split(' ');
        let fire_charge = parts.next() == Some("fire_charge");
        parts.next(); // the block the use is made on: the sound does not read it
        let mut random = LegacyRandom::new(parts.next().unwrap_or_default().parse()?);
        let pitch = fire_starter_pitch(fire_charge, &mut random);
        writeln!(
            out,
            "{} {} {} {}",
            fire_starter_sound(fire_charge),
            1.0f32.to_bits() as i32,
            pitch.to_bits() as i32,
            random.next_long()
        )?;
    }
    out.flush()?;
    Ok(())
}
