# Upgrading a 26.2 data pack to 26.3

26.3 reworked the world generation registries, and vanilla 26.3 refuses a
worldgen data pack written for 26.2 (registry errors on load). Horizon
targets 26.3, so such a pack has to be rewritten first:

```sh
cargo run --release -p horizon-port-pack -- upgrade-datapack PACK.zip --world <world folder>
# or into any folder:
cargo run --release -p horizon-port-pack -- upgrade-datapack PACK.zip OUT/
```

`--world` writes the upgraded pack to `<world>/datapacks/<pack name>-26.3/`.
The input is a zip or a folder; nothing is downloaded, and the pack stays on
your machine (packs are often All Rights Reserved: do not commit or share an
upgraded copy).

Horizon does not apply a data pack's world generation yet (see
[the mod browser](mod-browser.md)); the upgraded pack loads in vanilla 26.3,
and the checks below run it there.

The command prints what it rewrote, what it could not rewrite faithfully
(`not faithful`) and the types it does not know (`left as it is`).

## What changes

Each rewrite is the one vanilla's own data went through between 26.2 and
26.3 (checked by `tools/datapack_upgrade_rosetta.py`, below), and the parts
26.2 hard-coded are spelled out as data so the 26.3 pack generates what the
26.2 pack did.

| 26.2 | 26.3 | How it is checked |
|---|---|---|
| noise `firstOctave`, `amplitudes` | `base_octave`, `octave_count`, `amplitude_modifiers` and the `base_amplitude` of vanilla's `NormalNoise.createParity` (keeps 26.2's normalisation) | all 63 vanilla noises identical to 26.3's, bit for bit |
| density `argument`, `argument1`/`argument2` | `input`, `left`/`right` | Rosetta, and generated terrain |
| `invert` | `reciprocal` | codec |
| `y_clamped_gradient` (`from_y`, `to_y`) | `gradient` on axis `y` | Rosetta |
| `shifted_noise` | `noise` with `shift_x`/`shift_y`/`shift_z` (same sampling) | Rosetta |
| `flat_cache` | `cache`, around `slice` at y = 0 when the input depends on y (26.2's flat cache held each column's value at y = 0) | unit test |
| `cache_2d`, `cache_once`, `cache_all_in_cell` | `cache` (26.3's caches only memoise) | Rosetta |
| `end_islands` | `end_outer_islands` | Rosetta |
| `constant` `{argument}` | the number | codec |
| `interpolated` (cell from the noise settings' `size_horizontal * 4`, `size_vertical * 4`) | `interpolated` with `cell_size_xz`/`cell_size_y`, from the settings whose router reaches the function | Rosetta: vanilla's 4 x 8 |
| noise settings `noise.size_*` | dropped (on each `interpolated`) | Rosetta |
| router `final_density` (26.2 added the beardifier in `NoiseChunk`) | `add(final_density, beardifier)` | as vanilla's `overworld/final_density` |
| router `barrier`, `fluid_level_*`, `lava`, `aquifers_enabled` | `aquifers` object; `exclusion` spells out 26.2's `isDeepDarkRegion` (erosion < -0.225 and depth > 0.9) from the pack's own erosion and depth; `surface_level` is the preliminary surface | as vanilla's |
| router `preliminary_surface_level` | named function, `aquifers.surface_level`, and `chunk_surface_level` = `interpolated` 16 x 1 of it (26.2's per-chunk surface interpolation) | as vanilla's |
| router `vein_toggle`, `vein_ridged`, `vein_gap`, `ore_veins_enabled` | two `ore_vein` material rules (copper, iron) that reproduce 26.2's `OreVeinifier`: same heights, thresholds, edge roundoff, solidness, richness and gap | written from `OreVeinifier`; terrain |
| `surface_rule` | `material_rule` (same node types), named `<settings>/upgraded/material_rule`, after the ore veins: 26.2 placed veins while filling the noise and its surface pass only replaced the default block | terrain |
| `spawn_target` climate names | the router's density functions by id (`depth` and `offset` dropped as vanilla did) | as vanilla's |
| block states `{Name, Properties}` | `{id, properties}`, or the id alone | Rosetta |
| `configured_carver/` | `carver/`, `config` unwrapped; the cave carvers' hard-coded count, thickness and room scale written out (`nether_cave` is a `cave`); canyon `yScale` into `shape.y_scale`; `lava_level`, `replaceable`, `debug_settings` dropped | all 4 vanilla carvers identical to 26.3's |
| `configured_feature/` | `feature/`, `config` unwrapped, block state providers renamed (`simple_state_provider` is its state) | Rosetta |
| placement `random_offset` (`xz_spread`, `y_spread`) | `offset` with `x`, `y`, `z` | Rosetta |
| biome `spawners`, `spawn_costs` | attribute `minecraft:gameplay/natural_mob_spawns` (overlay), counts as int providers; empty categories kept so none comes from the dimension's defaults | Rosetta (vanilla leaves empty ones out) |
| biome `creature_spawn_probability` | attribute `minecraft:gameplay/creature_world_gen_spawn_probability` (left out at the default 0.1) | Rosetta |
| structure `spawn_overrides` `minCount`/`maxCount` | `count` | Rosetta |
| dimension type `bed_rule.explodes` | `bed_rule.destroy_on_use` | Rosetta |
| `pack.mcmeta` | pack format 121 | vanilla loads it |

Not rewritten: the feature types 26.3 replaced by compositions of other
features (`seagrass`, `kelp`, `basalt_columns`, `nether_forest_vegetation`
and others; listed by the command), a `cache_2d` around a function of y, a
`y_clamped_gradient` with `from_y == to_y` (a step instead of 26.2's NaN at
that y), carvers' custom `lava_level`/`replaceable`, and noises with more
than 32 octaves or negative amplitudes.

## Checking an upgrade

```sh
python3 tools/datapack_upgrade_rosetta.py
python3 tools/datapack_upgrade_fidelity.py                              # calibration: vanilla 26.2 against 26.3
python3 tools/datapack_upgrade_fidelity.py --old-pack PACK.zip --new-pack OUT/
```

The Rosetta check writes vanilla 26.2's own worldgen out as a pack, upgrades
it and compares it with 26.3's file by file. Files whose format changed and
whose content did not must come out the same. It needs the 26.2 server jar
(`.local/vanilla-26.2/server-game.jar`, from the launcher's 26.2 version
manifest).

The fidelity check starts vanilla's 26.2 and 26.3 dedicated servers on the
same seed, the first with the original pack and the second with the upgrade,
force loads the same chunks and compares the saved blocks. Without packs it
measures what 26.3 itself changed, which the upgrade cannot be blamed for.
The server logs must hold no data pack or registry errors.
