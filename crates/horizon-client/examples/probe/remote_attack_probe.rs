//! The attacking client's side of a hit, as `Game::remote_player_attack` decides it, against
//! VanillaRemoteAttackOracle's real `Player.attack` (tools/test_remote_attack_parity.py).
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    attack_strength::{ATTACKER_SLOWDOWN, AttackStrength, attacker_slows_after_hit},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn double(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let mut out = BufWriter::new(File::create(&args[1])?);
    for line in BufReader::new(File::open(&args[0])?).lines() {
        let c: Value = serde_json::from_str(&line?)?;
        let mut attack = AttackStrength::default();
        attack.ticker = c["ticker"].as_i64().unwrap() as i32;
        let strength = attack.scale(0.5, double(&c["speed"]));
        let damage = match c["spin"].as_f64() {
            Some(spin) => spin as f32,
            None => double(&c["damage"]) as f32,
        };
        let mut sprinting = c["sprint"].as_bool().unwrap();
        let velocity: Vec<f64> = c["vel"].as_array().unwrap().iter().map(double).collect();
        let (mut x, y, mut z) = (velocity[0], velocity[1], velocity[2]);
        // `level_players` keeps a mob out of remote_player_attack: Entity.hurtClient is false.
        if c["target"] == "player"
            && attacker_slows_after_hit(damage, double(&c["knockback"]) as f32, strength, sprinting)
        {
            x *= ATTACKER_SLOWDOWN;
            z *= ATTACKER_SLOWDOWN;
            sprinting = false;
        }
        writeln!(
            out,
            "{:08x} {:016x} {:016x} {:016x} {}",
            strength.to_bits(),
            x.to_bits(),
            y.to_bits(),
            z.to_bits(),
            sprinting
        )?;
    }
    Ok(())
}
