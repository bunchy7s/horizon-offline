//! Noise parameters: 26.2's `{firstOctave, amplitudes}` as 26.3's `NormalNoise` parameters.
use super::Report;
use serde_json::{Map, Value, json};

/// `{firstOctave, amplitudes}` to 26.3's `NormalNoise` parameters, as
/// vanilla's `NormalNoise.createParity` builds them: the same octaves and
/// the base amplitude that keeps 26.2's normalisation.
pub(super) fn noise(v: &Value, report: &mut Report) -> Value {
    let (Some(first), Some(amps)) = (
        v.get("firstOctave").and_then(Value::as_i64),
        v.get("amplitudes").and_then(Value::as_array),
    ) else {
        return v.clone();
    };
    let amps: Vec<f64> = amps.iter().filter_map(Value::as_f64).collect();
    if amps.is_empty() || amps.len() > 32 || amps.iter().any(|a| *a < 0.0) {
        report.lossy("noise: more than 32 octaves or negative amplitudes (left as they are)");
        return v.clone();
    }
    let mut o = Map::new();
    let base = parity_base_amplitude(first as i32, &amps);
    if base != 1.0 {
        o.insert("base_amplitude".into(), json!(base));
    }
    o.insert("base_octave".into(), json!(first));
    if amps.len() != 1 {
        o.insert("octave_count".into(), json!(amps.len()));
    }
    if amps.iter().any(|a| *a != 1.0) {
        o.insert("amplitude_modifiers".into(), json!(amps));
    }
    Value::Object(o)
}

/// `NormalNoise.computeParityBaseAmplitude`.
pub fn parity_base_amplitude(base_octave: i32, amps: &[f64]) -> f64 {
    let octaves = build_octaves(base_octave, 1.0, amps.len(), amps);
    let target: f64 = java_sum(octaves.iter().map(|a| a.abs()));
    let new = normalization_factor(target, &octaves);
    if new == 0.0 {
        return 1.0;
    }
    parity_normalization_factor(1.0, amps) / new
}

/// `NormalNoise.buildOctaves` with normalisation on: the octaves' signed
/// amplitudes (zero-modifier octaves left out).
fn build_octaves(_base_octave: i32, base_amplitude: f64, count: usize, mods: &[f64]) -> Vec<f64> {
    let n = count as i32;
    let mut amplitude = base_amplitude * (0.5f64.powi(-(n - 1)) / (0.5f64.powi(-n) - 1.0));
    let mut out = Vec::new();
    for i in 0..count {
        let m = mods.get(i).copied().unwrap_or(1.0);
        if m != 0.0 {
            out.push(amplitude * m);
        }
        amplitude *= 0.5;
    }
    out
}

fn normalization_factor(target: f64, octaves: &[f64]) -> f64 {
    let mut variance = 0.0;
    for a in octaves {
        let layer = 0.2702247831245211 * a.abs();
        variance += layer * layer;
    }
    let deviation = variance.sqrt();
    if deviation == 0.0 {
        return 0.0;
    }
    target * 0.3333333333333333 / (deviation * 2f64.sqrt())
}

fn parity_normalization_factor(base_amplitude: f64, mods: &[f64]) -> f64 {
    let (mut min, mut max) = (i32::MAX, i32::MIN);
    for (i, m) in mods.iter().enumerate() {
        if *m != 0.0 {
            min = min.min(i as i32);
            max = max.max(i as i32);
        }
    }
    let span = max.wrapping_sub(min);
    base_amplitude * 0.5 * 0.3333333333333333 / (0.1 * (1.0 + 1.0 / f64::from(span + 1)))
}

/// `DoubleStream.sum()`: Kahan summation as `Collectors.sumWithCompensation`
/// and `computeFinalSum` do it.
fn java_sum(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut c, mut simple) = (0.0f64, 0.0f64, 0.0f64);
    for v in values {
        simple += v;
        let tmp = v - c;
        let velvel = sum + tmp;
        c = (velvel - sum) - tmp;
        sum = velvel;
    }
    let tmp = sum - c;
    if tmp.is_nan() && simple.is_infinite() {
        simple
    } else {
        tmp
    }
}

/// A noise holder: an id, or inline parameters to upgrade.
pub(super) fn noise_holder(v: &Value, report: &mut Report) -> Value {
    match v {
        Value::Object(_) => noise(v, report),
        other => other.clone(),
    }
}
