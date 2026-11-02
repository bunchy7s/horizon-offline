# Horizon main vs vanilla 26.3 on a loaded Mac mini M4 Pro (2026-11-01)

`tools/perf_bench.py` reports for Horizon main 1bd24e4042, vanilla 26.3 and vanilla with performance mods, render distances
8, 16 and 32, three games each, every game from a fresh world. **These are not player numbers:** the Mac mini was running other
work the whole time (1-minute load average 15-114 on 14 cores), and every game rendered without presenting. They are kept because
they're the first Mac reports and show how each game behaves under heavy CPU contention. The reports are the
`*-m4pro-1080p-rd*.json` files next to this page.

## Setup

- **Machine:** Mac mini, Apple M4 Pro (10 performance + 4 efficiency cores, 20-core GPU), 64 GB, macOS 27.0.1. Screen
  1920x1080; the session was locked, so every game ran `offscreen` (Horizon renders into a texture; vanilla draws into a
  window nobody sees).
- **Horizon:** main 1bd24e4042, `cargo build --release -p horizon-client`, Metal.
- **Vanilla:** the 26.3 client jar with Fabric Loader and the driver mod, Java 25 (Temurin), `-Xmx2G` with the launcher's G1
  flags, OpenGL 4.1 (Apple's Metal-backed driver).
- **Vanilla + mods:** the same with Sodium mc26.3-0.9.2, Lithium mc26.3-0.26.2, FerriteCore 9.0.0, ImmediatelyFast 1.17.1 and
  Fabric API (`tools/benchmark-deps/mods.lock.json`).
- **Order:** the games took turns inside each repeat (vanilla, mods, Horizon; then mods, Horizon, vanilla; then Horizon, vanilla,
  mods), one game at a time.

## Why these numbers aren't a fair comparison

- **Load.** Load before each game: RD 8 46-82, RD 16 43-114, RD 32 15-59 (the later RD 32 games ran at 15-30). Horizon raises its
  render and server threads to the user-interactive QoS class, and the JVM's threads stay at the default, so under this
  contention macOS runs Horizon's frame first. Vanilla's 1% lows of 2-15 FPS show that, more than how fast either game is.
- **Resolution.** Vanilla's 1920x1080 window was shrunk to **1920x930** by the 1080p screen (menu and title bars); Horizon,
  offscreen, rendered 1920x1080. Vanilla drew 14% fewer pixels, which favours it in GPU-bound scenes. Because the recorded
  windows differ, `perf_bench.py --table` refuses to compare these reports. The tables below were made with the window check
  skipped. A rerun at a size that fits the screen (1600x900) would avoid it.

## Results

Mean of 3 games ± 95% confidence interval. FPS over the 12-second flight (average / 1% low / 0.1% low):

| RD | scene | Horizon main | Vanilla 26.3 | Vanilla + mods |
| --- | --- | --- | --- | --- |
| 8 | forest | 364 ±22 / 145 ±54 / 97 ±38 | 66 ±22 / 8 ±10 / 6 ±8 | 47 ±42 / 4 ±2 / 3 ±1 |
| 8 | mountain | 466 ±7 / 165 ±54 / 91 ±37 | 46 ±37 / 5 ±7 / 4 ±6 | 67 ±35 / 8 ±5 / 4 ±4 |
| 8 | ocean | 410 ±88 / 161 ±39 / 112 ±41 | 50 ±44 / 6 ±13 / 5 ±11 | 35 ±16 / 5 ±4 / 4 ±4 |
| 8 | village | 540 ±453 / 196 ±189 / 135 ±184 | 42 ±16 / 4 ±3 / 3 ±0 | 33 ±46 / 4 ±6 / 3 ±4 |
| 16 | forest | 333 ±4 / 193 ±29 / 137 ±71 | 23 ±9 / 2 ±1 / 2 ±1 | 62 ±94 / 10 ±21 / 7 ±14 |
| 16 | mountain | 357 ±6 / 200 ±30 / 154 ±33 | 44 ±12 / 7 ±2 / 4 ±1 | 36 ±44 / 4 ±5 / 3 ±5 |
| 16 | ocean | 412 ±4 / 209 ±26 / 127 ±44 | 51 ±16 / 6 ±8 / 4 ±7 | 75 ±159 / 13 ±33 / 7 ±16 |
| 16 | village | 568 ±30 / 271 ±25 / 210 ±75 | 46 ±110 / 9 ±27 / 7 ±17 | 46 ±45 / 7 ±9 / 5 ±4 |
| 32 | forest | 165 ±11 / 104 ±51 / 95 ±60 | 68 ±122 / 18 ±49 / 14 ±42 | 114 ±132 / 29 ±44 / 21 ±34 |
| 32 | mountain | 190 ±5 / 109 ±35 / 91 ±32 | 29 ±19 / 3 ±2 / 2 ±3 | 57 ±49 / 7 ±2 / 3 ±3 |
| 32 | ocean | 210 ±13 / 129 ±77 / 116 ±98 | 44 ±80 / 5 ±4 / 2 ±1 | 119 ±176 / 32 ±66 / 17 ±37 |
| 32 | village | 326 ±17 / 152 ±85 / 123 ±84 | 81 ±99 / 15 ±20 / 11 ±11 | 92 ±28 / 13 ±6 / 9 ±3 |

Start to title / in-world, mean scene load, RSS steady / peak, physical footprint steady / peak (macOS `footprint`, which on Apple
Silicon includes the game's GPU allocations), load average before the first game:

| RD | Horizon main | Vanilla 26.3 | Vanilla + mods |
| --- | --- | --- | --- |
| 8 | 7.2 / 11.4, 6.5, 1379 / 1537, 1883 / 2094, 73 | 49.9 / 89.1, 18.2, 1811 / 1877, 2635 / 2709, 66 | 56.3 / 97.7, 17.9, 1927 / 2019, 2396 / 2441, 74 |
| 16 | 1.5 / 14.0, 9.5, 1617 / 1892, 2495 / 3084, 114 | 19.4 / 78.9, 33.3, 2431 / 2555, 3450 / 3713, 76 | 23.8 / 76.9, 27.1, 2355 / 2566, 3212 / 3431, 70 |
| 32 | 2.2 / 14.3, 11.1, 2196 / 2537, 4373 / 5560, 28 | 20.6 / 124.2, 44.2, 2838 / 2958, 4265 / 4618, 59 | 13.5 / 63.2, 29.9, 3705 / 4078, 4647 / 5250, 37 |

## What it shows

- Horizon is GPU-bound on the M4 Pro in every scene at every distance: its median GPU time per frame is 1.6-2.5 ms at RD 8 and
  2.9-5.7 ms at RD 32, its render thread's CPU time 0.6-2.2 ms.
- In the least-loaded RD 32 games (load 15-22), vanilla + mods matched Horizon's average FPS over the forest (162 vs 163-170) and
  the ocean (187 vs 209-216); Horizon's 1% lows were 2.4 times the modded client's there, and 15-17 times over the village and the
  mountain.
- Horizon's resident memory is 1.2-1.7 times smaller than both clients'. Its physical footprint, GPU memory included, is level
  with vanilla's at RD 32 (4373 vs 4265 MB steady) and above it at peak (5560 vs 4618 MB).
