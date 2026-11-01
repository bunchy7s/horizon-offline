# Horizon's server against Pumpkin

[Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) is a Minecraft server written in Rust, and the one Horizon's
server is measured against. It is GPL-3.0: we build it and run it as a black box, and never read or copy its
source. Only its binary, its log and the configuration file it writes run here.

Both servers speak the same protocol: Pumpkin 0.2.1+26.3-26.60 says "Java Minecraft (Protocol 777)", the
protocol of 26.3 that Horizon and `horizon-bots` speak. Each server gets the same bots, the same world seed and
the same settings.

## Result (2026-10-31)

playhorizon (AMD EPYC 7662, 8 vCPUs). The server and the bots ran on CPUs 0-3, with no other benchmark there;
the CI runner works on CPUs 4-7. Both servers are release builds: Horizon main d91cab68d, Pumpkin 55cb804cc.
`mixed` bots (60 % walk, 20 % explore, 20 % idle), seed 12345, view and simulation distance 10, creative,
offline. One bot joined every 0.5 s, then 60 s of warm-up and a 120 s window. Three rounds, the servers
alternated; the median of the rounds. Every run: [benchmarks/server-vs-pumpkin-2026-10-31.json](../../benchmarks/server-vs-pumpkin-2026-10-31.json).

| Bots | Server | MSPT P50 | MSPT avg | MSPT P99 | worst P99 | CPU % | RSS MB | chunks/s sent | new chunks/s | bots waited (ticks/s) | join P50 ms | join max ms | start s |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | Horizon | 0.0 | 0.0 | 0.1 | 0.1 | 0.8 | **320** | – | – | – | – | – | **2.2** |
| 0 | Pumpkin | 0.2 | 0.2 | 0.4 | 0.8 | 0.8 | 69 | – | – | – | – | – | 0.1 |
| 20 | Horizon | **35.1** | **36.0** | 54.9 | 67.3 | 305 | 1179 | 236 | 112 | 0 | **68** | 85 | 2.2 |
| 20 | Pumpkin | 18.6 | 20.0 | 59.9 | 104.8 | 255 | 1139 | 190 | 69 | 40 | 26 | 1192 | 0.1 |
| 60 | Horizon | 61.1 | 62.0 | 98.4 | 115.6 | 328 | 1765 | 436 | 87 | 93 | 65 | 109 | 2.2 |
| 60 | Pumpkin | 62.4 | 63.7 | 113.0 | 218.9 | 332 | 1771 | 425 | 69 | 206 | 33 | 1347 | 0.1 |
| 100 | Horizon | 80.5 | 82.0 | 120.2 | 138.7 | 314 | 2099 | 650 | 77 | 246 | 71 | 139 | 2.2 |
| 100 | Pumpkin | 152.7 | 156.6 | 299.0 | 548.5 | 345 | 1848 | 671 | 81 | 367 | 37 | 1657 | 0.1 |

MSPT comes from each server's own `tick query`, asked every 5 s. The averages per round:
- 20 bots: Horizon 37.9 / 36.0 / 34.7, Pumpkin 18.7 / 20.0 / 23.5.
- 60 bots: Horizon 62.0 / 61.9 / 70.8, Pumpkin 75.0 / 50.0 / 63.7.
- 100 bots: Horizon 76.3 / 86.2 / 82.0, Pumpkin 156.6 / 177.2 / 138.6.

How to read the columns:
- **chunks/s sent** and **new chunks/s**: the chunks the bots received each second over the window, and those
  they had never had before. The explore bots fly as fast as chunks come, so new chunks a second is the
  terrain the server generated for them.
- **bots waited**: ticks a second the bots stood still for want of their own chunk.
- **join**: from a bot's first packet to its spawn position.

Neither server held 20 TPS at 60 or 100 bots on 4 cores.

### Where Horizon wins

- **Load: Horizon scales better.** At 100 bots Horizon's tick takes half Pumpkin's time (82 against
  157 ms). Its P99 is 120 against 299 ms, and its worst 5-second P99 is 139 against 549 ms. At 60 the averages
  are equal and Horizon's P99 is lower.
- **More terrain, less waiting.** At 20 bots Horizon generated 1.6 times the new chunks a second, and its bots
  never waited for one; Pumpkin's waited 40 ticks a second. At 60 bots Horizon's bots waited half as long.
- **Join tail.** Every bot is in within 85–139 ms on Horizon. On Pumpkin some wait 1.2–1.7 s, though its
  median join is faster.

### Where Horizon loses, ranked

1. **Tick time at light load: 36 against 20 ms at 20 bots.** The work is not equal: Horizon generated 63 %
   more new chunks a second and runs vanilla's whole mob simulation (natural spawning, AI, pathfinding: 1 600
   to 2 000 mobs with 60 bots on this seed in the tick-time runs of #5191). Pumpkin's entity count isn't
   measured here. Real players weigh this case most: the official server's ticks are half mobs
   (`tick_mobs` 16.2 of 32.7 ms with 28 players). So the mobs come first (#5191, #5237), then chunk generation.
2. **Idle memory: 320 against 69 MB**, before any player joins.
3. **Start: 2.2 against 0.1 s** to the first tick (the worldgen data alone takes about 0.5 s).
4. **Median join: 65–71 against 26–37 ms**, though Horizon's worst join is shorter.
5. **Memory under load: 2.1 against 1.85 GB at 100 bots.**

Not compared:
- **Movement checks.** Horizon pulls back the moves vanilla refuses (3 400 to 11 200 pull-backs in a window);
  Pumpkin pulled back none.
- **Online mode and encryption** are off on both: the bots are offline players.

## Run it again

On a Linux machine with the vanilla data prepared (`python3 tools/lab.py prepare`, docs/parity-workflow.md):

```sh
# Pumpkin: your own build, run as a black box (GPL-3.0: don't read or copy its source into Horizon)
git clone https://github.com/Pumpkin-MC/Pumpkin && cd Pumpkin && git checkout 55cb804cc && cargo build --release && cd -
# Horizon
cargo build --release -p horizon-client --bin horizon-client
cargo build --release -p horizon-server -p horizon-bots
# The comparison: 0, 20, 60 and 100 bots, three rounds, the servers alternated
taskset -c 0-3 python3 tools/server_bench.py --servers horizon,pumpkin --pumpkin Pumpkin/target/release/pumpkin \
  --bots 0,20,60,100 --behaviour mixed --join-every 0.5 --warmup 60 --seconds 120 --repeat 3 \
  --out benchmarks/server-vs-pumpkin-$(date -u +%F).json
```

`tools/server_bench.py` sets Pumpkin up from its own defaults. Its first start writes them, in a scratch folder,
on 127.0.0.1 only and without telemetry. Each run then gets that file with the benchmark's settings, and with
Pumpkin's Bedrock and LAN listeners, query, plugins and telemetry off. On playhorizon the run above is
`tools/remote.sh --name <name> --cpus 0-3 --exclusive . bash -c ': horizon-prepare; python3 tools/server_bench.py …'`.
