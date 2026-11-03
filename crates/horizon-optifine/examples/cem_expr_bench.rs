//! How fast CEM animation expressions evaluate: a walk cycle, idle breathing, head tracking,
//! counters and keyframes in the style of animation packs, compiled once, then evaluated as
//! one entity's frame over and over. CPU time is the figure to compare: the wall clock counts
//! whatever else the machine runs.
//!
//! `cargo run --release -p horizon-optifine --example cem_expr_bench`

use horizon_optifine::cem::expr::compile;
use std::{hint::black_box, time::Instant};

const NAMES: &[&str] = &[
    "limb_swing",
    "limb_speed",
    "age",
    "head_yaw",
    "head_pitch",
    "frame_time",
    "swing_progress",
    "hurt_time",
    "is_in_water",
    "is_on_ground",
    "is_child",
    "is_riding",
    "is_sneaking",
    "head.rx",
    "head.ry",
    "body.rx",
    "left_arm.rx",
    "right_arm.rx",
    "left_leg.rx",
    "right_leg.rx",
    "var.water",
    "var.breath",
    "var.t",
    "var.step",
    "var.look",
];

const EXPRESSIONS: &[(&str, &str)] = &[
    ("var.t", "var.t + frame_time * 20"),
    ("var.breath", "sin(age * 0.1) * 0.03"),
    (
        "var.water",
        "if(is_in_water, min(20, var.water + 0.1 * frame_time * 20), max(0, var.water - 0.1 * frame_time * 20))",
    ),
    ("var.step", "sin(limb_swing * 0.6662) * limb_speed"),
    ("var.look", "clamp(head_yaw, -45, 45)"),
    (
        "head.rx",
        "torad(head_pitch) + var.breath + if(is_sneaking, 0.2, 0)",
    ),
    ("head.ry", "torad(var.look) * if(is_riding, 0.5, 1)"),
    ("body.rx", "if(is_sneaking, 0.5, 0) + var.breath * 0.5"),
    (
        "left_arm.rx",
        "-var.step * 1.4 - sin(swing_progress * pi) * 1.2 + sin(age * 0.067) * 0.05",
    ),
    (
        "right_arm.rx",
        "var.step * 1.4 + if(hurt_time > 0, -sin(hurt_time / pi) * 0.4, 0)",
    ),
    ("left_leg.rx", "var.step * 1.4 * if(is_on_ground, 1, 0.3)"),
    ("right_leg.rx", "-var.step * 1.4 * if(is_on_ground, 1, 0.3)"),
    (
        "head.rx",
        "head.rx + keyframeloop(var.t / 10, 0, 0.05, 0.1, 0.05, 0, -0.05)",
    ),
    (
        "body.rx",
        "body.rx + easeinoutsine(var.water / 20, 0, torad(30))",
    ),
    (
        "left_arm.rx",
        "if(is_child && !is_on_ground, left_arm.rx * 0.5, between(var.water, 1, 19), lerp(var.water / 20, left_arm.rx, -2.5), left_arm.rx)",
    ),
    (
        "right_arm.rx",
        "if(is_child && !is_on_ground, right_arm.rx * 0.5, between(var.water, 1, 19), lerp(var.water / 20, right_arm.rx, -2.5), right_arm.rx)",
    ),
    ("head.ry", "head.ry + wraprad(sin(var.t * 0.01) * 0.1)"),
    (
        "left_leg.rx",
        "left_leg.rx + abs(cos(limb_swing * 0.3331)) * 0.1 * limb_speed",
    ),
    (
        "right_leg.rx",
        "right_leg.rx + abs(cos(limb_swing * 0.3331 + pi)) * 0.1 * limb_speed",
    ),
    ("var.t", "fmod(var.t, 1000)"),
];

fn main() {
    let slot = |name: &str| NAMES.iter().position(|n| *n == name).map(|i| i as u32);
    let program: Vec<_> = EXPRESSIONS
        .iter()
        .map(|(target, src)| {
            let expr = compile(src, &mut |n| slot(n)).unwrap_or_else(|e| panic!("{src}: {e}"));
            (slot(target).unwrap() as usize, expr)
        })
        .collect();
    let mut slots = vec![0.0f32; NAMES.len()];
    let frames = 2_000_000u32;
    let pid = std::process::id();
    let cpu = horizon_bench::cpu_seconds(pid);
    let start = Instant::now();
    for frame in 0..frames {
        // An entity walking in and out of water.
        slots[0] = frame as f32 * 0.3;
        slots[1] = 0.8;
        slots[2] = frame as f32;
        slots[5] = 0.05;
        slots[8] = f32::from(frame % 400 < 200);
        slots[9] = 1.0;
        for (target, expr) in &program {
            slots[*target] = expr.eval(black_box(&slots));
        }
    }
    black_box(&slots);
    let evals = f64::from(frames) * program.len() as f64;
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{} expressions x {frames} frames: {:.1} ns per expression, {:.0} M evaluations/s, {:.2} us per entity frame",
        program.len(),
        elapsed * 1e9 / evals,
        evals / elapsed / 1e6,
        elapsed * 1e6 / f64::from(frames),
    );
    match cpu.zip(horizon_bench::cpu_seconds(pid)) {
        Some((before, after)) => println!(
            "  cpu time {:.1} ns per expression, {:.2} us per entity frame",
            (after - before) * 1e9 / evals,
            (after - before) * 1e6 / f64::from(frames),
        ),
        None => println!("  cpu time unavailable on this system"),
    }
}
