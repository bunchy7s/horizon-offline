# Benchmark report format (version 1)

Every benchmark Horizon writes uses this one JSON format: `horizon-client --benchmark`, the launcher's own
measurements, and `tools/perf_bench.py` (which also writes it for vanilla and vanilla with mods). The same
file is the local log a player can attach to an issue and, only with the player's consent, what the
telemetry upload sends (minus the local-only fields below). The types, the statistics and the hardware
detection are in `crates/horizon-bench`; nothing else builds a report by hand.

A report holds summaries and histogram counts, never raw frame streams: a 10-minute run is a few
kilobytes whatever the frame rate. Measured numbers are rounded to 3 decimals.

## Example

```json
{
  "format": "horizon-benchmark-report",
  "version": 1,
  "kind": "game",
  "created": "2026-10-15T14:03:27Z",
  "app": { "name": "horizon-client", "version": "0.21.0", "commit": "e96e1571" },
  "hardware": {
    "os": "macos", "os_version": "27.0", "arch": "aarch64",
    "cpu": "Apple M4 Pro", "cpu_threads": 14, "cpu_cores": 14, "ram_mb": 65536, "storage": "nvme", "form": "desktop",
    "gpu": { "name": "Apple M4 Pro", "vendor": "apple", "driver": "", "backend": "metal", "kind": "integrated" },
    "display": { "width": 2560, "height": 1440, "scale": 2.0, "refresh_hz": 60 }
  },
  "conditions": {
    "scenario": "flight", "seed": "1234", "render_distance": 16, "simulation_distance": 12,
    "window": [1280, 720], "vsync": false, "repeats": 3, "warmup_s": 2.0, "power": "ac", "load_avg_1m": 1.2
  },
  "metrics": {
    "startup.title_ms": { "n": 3, "mean": 812.0, "stdev": 21.5, "min": 790.0, "max": 833.0, "ci95": 53.4 },
    "memory.rss_peak_mb": { "n": 3, "mean": 1210.0, "stdev": 4.0, "min": 1206.0, "max": 1214.0, "ci95": 9.9 }
  },
  "frames": [
    {
      "phase": "village", "repeat": 0, "frames": 4920, "seconds": 12.0,
      "avg_fps": 410.0, "low_1pct_fps": 120.3, "low_01pct_fps": 61.0,
      "p50_ms": 2.31, "p95_ms": 3.8, "p99_ms": 6.9, "p999_ms": 14.2, "max_ms": 21.0,
      "cpu_p50_ms": 1.4, "gpu_p50_ms": 1.9,
      "histogram": [0, 0, 3011, 1520, 301, 62, 15, 7, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    }
  ]
}
```

## Fields

**`format`, `version`.** Always `"horizon-benchmark-report"` and `1`. A reader rejects other formats and
newer versions. Adding an optional field keeps the version; renaming, removing or changing the meaning of
one (histogram edges included) makes it 2.

**`kind`.** `"game"` (a game run: Horizon, or vanilla from `tools/perf_bench.py`) or `"launcher"`.

**`created`.** When the run ended, UTC, `YYYY-MM-DDTHH:MM:SSZ`.

**`app`.** What was measured: `name` (`horizon-client`, `horizon-launcher`, `minecraft`), `version`,
optionally `commit` (Horizon builds) and `mods` (`[{ "name", "version" }]`, vanilla with mods).

**`hardware`.** `os` (`macos`, `linux`, `windows`), `os_version`, `arch`, `cpu` (brand string),
`cpu_threads`, `cpu_cores` (physical), `cpu_base_mhz` and `cpu_boost_mhz` (where the OS gives them; not on
Apple silicon), `ram_mb`, `storage` (the drive under the game folder: `nvme`, `ssd`, `hdd`), `form` (`laptop`,
`desktop`); `gpu` (`name`, `vendor`, `driver`, `backend`: `metal`, `vulkan`, `dx12`, `gl`;
`kind`: `integrated`, `discrete`, `virtual`, `cpu`, `other`; `vram_mb`, a discrete GPU's own memory) and
`display` (the screen the window was on: `width`, `height` in physical pixels, `scale`, `refresh_hz`) when
known. Any field can be missing. On mains or battery is `conditions.power`.

**`conditions`.** What has to match for two reports to be compared, all optional: `scenario`, `seed`,
`render_distance`, `simulation_distance`, `window` (`[width, height]` actually rendered), `vsync`,
`fps_cap` (absent: unlimited), `offscreen` (rendered without presenting: not a player number), `repeats`,
`warmup_s`, `power` (`ac`, `battery`), `load_avg_1m` (before the run; above about 1 the machine was busy),
`backend` (when forced). Readers compare reports only with equal `scenario`, `seed`, distances, `window`, `vsync`,
`fps_cap` and `offscreen`. `tools/perf_bench.py --table` refuses a comparison when these values differ or a required
value is unknown; an absent `fps_cap` means unlimited. A single report can still be shown without comparison.

**`metrics`.** Scalar measurements, by name; each is a summary over the run's repeats: `n`, `mean`,
`stdev` (sample standard deviation, null when `n` is 1), `min`, `max`, `ci95` (half-width of the 95%
confidence interval of the mean, Student's t, null when `n` is 1). The unit is the name's suffix: `_ms`,
`_s`, `_mb`, `_bytes`, `_fps`, `_pct`, `_mspt`, `_w`, `_c` (degrees Celsius), `_count`. Names in use:

| Name | What |
| --- | --- |
| `startup.title_ms` | process start to the first frame of the title screen |
| `startup.world_ms` | process start to in-world, every chunk around the spawn loaded and drawn |
| `chunks.<phase>.load_ms` | teleport to every chunk in view distance loaded and drawn |
| `chunks.<phase>.generated_count`, `chunks.<phase>.meshed_count`, `chunks.<phase>.generated_per_s` | chunks generated and section meshes built while loading, and the rate |
| `fps.<phase>.avg_fps`, `fps.<phase>.low_1pct_fps`, `fps.<phase>.low_01pct_fps` | per-phase figures across repeats |
| `tick.mspt`, `tick.tps` | integrated server tick time and rate (planned) |
| `memory.rss_peak_mb`, `memory.rss_steady_mb` | resident memory: peak over the run, median while flying |
| `memory.gpu_mb`, `memory.<phase>.gpu_mb` | video memory the game allocated (or NVIDIA attributes to it), where readable |
| `power.package_w`, `power.battery_drain_pct` | power draw, battery used per hour (laptops; planned) |
| `thermal.cpu_c`, `thermal.throttled_pct` | temperature, share of the run spent throttled (planned) |
| `launcher.start_ms` | launcher process start to its first frame (a session) |
| `launcher.cold_start_ms`, `launcher.warm_start_ms` | the same in `horizon-launcher benchmark`: its first start, then the others |
| `launcher.rss_mb`, `launcher.cpu_idle_pct` | launcher resident memory (sampled; `max` is the peak), CPU used while idle |
| `launcher.release_list_ms` | fetching the release list |
| `launcher.install_ms`, `launcher.update_ms` | downloading and unpacking a version: a new one, an update |
| `launcher.prepare_ms` | preparing a version's resources |
| `launcher.play_to_game_frame_ms` | Play pressed to the game's first frame |
| `launcher.binary_bytes`, `game.binary_bytes` | size of the executables |

New names can be added without a new version; the table lists the ones the tools write.

**`frames`.** One entry per measured phase and repeat (a scene's flight, the launcher's page switching):
`phase`, `repeat`, `frames`, `seconds`, `avg_fps`, `low_1pct_fps` and `low_01pct_fps` (the average frame
time of the slowest 1% or 0.1% of frames, as FPS), `p50_ms`, `p95_ms`, `p99_ms`, `p999_ms`, `max_ms`,
`cpu_p50_ms` and `gpu_p50_ms` when measured, and `histogram`: frame counts per bucket. The bucket edges
are fixed by the version, so histograms from any number of reports add up:

    0, 1, 2, 3, 4.17, 5, 6, 7, 8.33, 10, 12, 14, 16.67, 20, 25, 33.33, 40, 50, 66.67, 100, 150, 250, 500, 1000 ms

Bucket *i* counts frames from edge *i* (included) to edge *i* + 1; the last bucket is 1000 ms and over
(24 buckets). 4.17, 8.33, 16.67 and 33.33 ms are 240, 120, 60 and 30 FPS.

## Local only

`label` (a name the person running it chose) and `local` (paths of the logs the run wrote next to the
report) stay on the machine: the upload removes them and dates the report to the day only, as every
telemetry item is (`Report::for_upload`). The upload is `horizon-telemetry`'s `benchmark` item: this
report, as it is, only with the player's consent to benchmark uploads (docs/telemetry.md). Nothing in a report
identifies the player, the machine's name or the world: no host name, user name, path or address.

## Where reports go

- The game: `benchmarks/<kind>-<created>-<scenario>.json` in the game folder.
- The launcher: `benchmarks/` in the launcher's data folder.
- `tools/perf_bench.py`: one report per game and render distance, `benchmarks/perf/<label>-rd<N>.json` in the
  repository checkout.
