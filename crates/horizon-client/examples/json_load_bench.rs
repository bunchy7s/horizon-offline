//! Compare the read + Value parse stages in BlockFacts::load and Assets::load.
//! Usage: json_load_bench RESOURCE_ROOT [REPETITIONS]; see tools/json_load_fixtures.py.
extern crate horizon_client_lib as horizon_client;
use serde_json::Value;
use std::{
    fs::{self, File},
    hint::black_box,
    io::BufReader,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn reader(path: &Path) -> Result<Value> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
}

fn slice(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn measure(path: &Path, load: fn(&Path) -> Result<Value>) -> Result<Duration> {
    let start = Instant::now();
    let value = black_box(load(black_box(path))?);
    let elapsed = start.elapsed();
    // Value destruction is excluded; the temporary byte buffer is released by slice().
    drop(value);
    Ok(elapsed)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .ok_or("usage: json_load_bench ROOT [REPETITIONS]")?;
    let repetitions: usize = args.next().map_or(Ok(15), |s| s.parse())?;
    if repetitions == 0 {
        return Err("REPETITIONS must be positive".into());
    }
    for name in ["gameplay-facts.json", "facts.json"] {
        let path = Path::new(&root).join(name);
        // Validate complete parsed trees on the exact files being timed.
        assert_eq!(reader(&path)?, slice(&path)?);
        for _ in 0..2 {
            measure(&path, reader)?;
            measure(&path, slice)?;
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for i in 0..repetitions {
            // Alternate order to reduce filesystem cache / frequency bias.
            if i % 2 == 0 {
                before.push(measure(&path, reader)?);
                after.push(measure(&path, slice)?);
            } else {
                after.push(measure(&path, slice)?);
                before.push(measure(&path, reader)?);
            }
        }
        before.sort_unstable();
        after.sort_unstable();
        let median = |samples: &[Duration]| {
            (samples[(samples.len() - 1) / 2].as_secs_f64()
                + samples[samples.len() / 2].as_secs_f64())
                * 500.0
        };
        let (before, after) = (median(&before), median(&after));
        println!(
            "{name}: {} bytes; {repetitions} samples; reader {before:.3} ms; slice {after:.3} ms; {:.2}x",
            fs::metadata(&path)?.len(),
            before / after
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_and_slice_preserve_values_and_error_categories() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("horizon-json-load-{}.json", std::process::id()));
        let check = || -> Result<()> {
            fs::write(
                &path,
                br#"{"states":[{"id":0,"hardness":-1.5,"name":"escaped\n\u2603","outline":[[0,0,0,1,1,1]],"enabled":true}],"defaults":null}"#,
            )?;
            assert_eq!(reader(&path)?, slice(&path)?);
            for invalid in [
                &b"{\"states\":["[..],
                &b"{\"states\":tru}"[..],
                &b"{} trailing"[..],
                &b"{\"name\":\"\xff\"}"[..],
            ] {
                fs::write(&path, invalid)?;
                let before = reader(&path).unwrap_err();
                let after = slice(&path).unwrap_err();
                assert_eq!(
                    before
                        .downcast_ref::<serde_json::Error>()
                        .unwrap()
                        .classify(),
                    after
                        .downcast_ref::<serde_json::Error>()
                        .unwrap()
                        .classify()
                );
            }
            fs::remove_file(&path)?;
            for load in [reader, slice] {
                let error = load(&path).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    std::io::ErrorKind::NotFound
                );
            }
            Ok(())
        };
        let result = check();
        let _ = fs::remove_file(path);
        result
    }
}
