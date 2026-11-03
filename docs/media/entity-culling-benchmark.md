# Entity culling benchmark

Run date: 2026-11-01T14:09:25Z.

## Setup

- Hardware: Intel Core i7-3770S (8 threads); Intel HD Graphics 4000 (IVB GT2) with Intel Mesa 26.2.4-arch1.1; Vulkan; 1366x768 @ 60 Hz; Linux / Hyprland Wayland.
- Build: `cargo build --profile fast -p horizon-client (successful)`.
- Scene: 300 static armor stands behind a 33-wide x 31-high stone wall at 3.5 blocks from camera, with a 1-wide x 4-high center opening.
- Same seed-1234 scene per pass; 1366x768 fullscreen, render distance 2, immediate presentation, VSync off, 260 FPS cap.
- Camera: Fixed at the initial spawn X/Z, feet y=100, yaw 0 and pitch 0. Time was set to 6000 at scene setup and then advanced normally.
- Six 10 second samples per mode across both orders: Off → Sync → Async, then Async → Sync → Off. At least 5 seconds to settle after changes.

## Median results

| Mode | Average FPS | Change vs Off | 1% low FPS | CPU average | P99 frame time |
|---|---:|---:|---:|---:|---:|
| Off | 83.59 | — | 60.48 | 8.66 ms | 14.99 ms |
| Sync | 96.90 | +15.9% | 62.83 | 6.82 ms | 12.99 ms |
| Async | 99.67 | +19.2% | 64.55 | 5.99 ms | 13.02 ms |

## Individual samples

| Pass | Mode | Average FPS | 1% low FPS | CPU average | P99 frame time |
|---|---|---:|---:|---:|---:|
| Forward | Off | 83.37 | 46.68 | 8.88 ms | 19.27 ms |
| Forward | Sync | 96.55 | 62.56 | 7.26 ms | 13.52 ms |
| Forward | Async | 103.77 | 81.52 | 5.83 ms | 11.93 ms |
| Reverse | Async | 98.55 | 45.93 | 6.17 ms | 18.66 ms |
| Reverse | Sync | 97.26 | 56.59 | 6.45 ms | 13.04 ms |
| Reverse | Off | 83.98 | 57.35 | 8.60 ms | 14.60 ms |

GPU timing was unavailable because this driver returned zero for all timestamp queries. This is one synthetic scene on one Intel HD 4000 system; results are workload-specific.
