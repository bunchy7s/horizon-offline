use horizon_client::{
    Result,
    mobs::{Mob, MobTypes, aquatic},
};
use horizon_core::geometry::Vec3;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    let types = MobTypes::load(&root)?;
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let kind = Arc::new(
            types.types[&format!("minecraft:{}", row["species"].as_str().unwrap())].clone(),
        );
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.fluids.in_water = row["water"].as_bool().unwrap();
        mob.velocity = Vec3::new(
            row["x"].as_f64().unwrap(),
            row["y"].as_f64().unwrap(),
            row["z"].as_f64().unwrap(),
        );
        mob.body_yaw = row["body"].as_f64().unwrap() as f32;
        for (i, value) in row["v"].as_array().unwrap().iter().enumerate() {
            mob.anim.v[i] = value.as_f64().unwrap() as f32;
        }
        if row["reset"].as_bool().unwrap() {
            aquatic::squid_cycle_event(&mut mob);
        }
        let velocity = mob.velocity;
        aquatic::squid_client_step(&mut mob, &mut Vec::new());
        assert_eq!(
            velocity, mob.velocity,
            "a remote animation must preserve packet velocity"
        );
        write!(out, "{:08x}", mob.body_yaw.to_bits())?;
        for value in &mob.anim.v[..6] {
            write!(out, " {:08x}", value.to_bits())?;
        }
        writeln!(out)?;
    }
    Ok(())
}
