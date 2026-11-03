# Build times

`tools/build_times.py` measures how long Horizon takes to build, the same way on any checkout, so that two
machines or two commits can be compared. It exists for [#3916](https://github.com/horizon-rust/horizon/issues/3916)
(a leaner codebase that builds faster): the crate split needs a number before and a number after.
It only measures. It does not change the build.

## What it measures

Every build is the one a developer runs, `cargo build --locked -p horizon-client --profile <profile> --timings`
(the game and `horizon-ctl`), in a target folder of its own.

- **Cold:** a build from an empty target folder, for the `dev` and `fast` profiles. Its timings page gives the
  time of each crate and the critical path: the chain of units, each one waiting for the one before it, that
  ended last. Shortening any other unit does not shorten the build.
- **Nothing changed:** the same build again. This is the least any build costs.
- **Incremental:** a one-line change in one file of `horizon-world`, `horizon-client-render`,
  `horizon-client` (`game.rs`, the game binary) and `horizon-items` (a small crate low in the tree), then
  the build. The report lists the crates cargo recompiled and the slowest of them.

## Run it

```sh
python3 tools/build_times.py run                       # everything: 20 to 40 minutes
python3 tools/build_times.py run --profiles dev        # one profile
python3 tools/build_times.py run --only incremental --crates horizon-items --profiles dev   # a quick check
python3 tools/build_times.py run --only cold --profiles dev
```

It writes `.local/build-times/reports/<date>-<commit>-<os>.json` (with `-2`, `-3`... when an earlier run took
that name) and a `.md` table beside it, and prints the table. Logs of every cargo call are in
`.local/build-times/logs/`.

Before you run it:

- Close other builds, tests and heavy programs. A cargo or rustc process running beside it, or a CPU that is
  more than 15% busy before a build, is warned about and kept with that number in the report. A measurement
  under load is a worse one; do it again when the machine is quiet.
- The working tree must be clean: the numbers are about a commit. `--allow-dirty` overrides this and the report
  says so.
- `CARGO_BUILD_JOBS` is respected (`--jobs N` overrides it); the report records the jobs and where they
  came from. `CARGO_TARGET_DIR` is not used: the tool builds in `.local/build-times/target-<profile>` and
  leaves yours alone. It only ever empties a folder it made itself, and never `target/`.
- `--repeats 3` runs each incremental measurement three times and reports the median.
- `--file horizon-items=crates/horizon-items/src/enchant.rs` edits another file. Any crate of the workspace
  can go in `--crates`; one that is not in the game's dependencies is reported as not recompiled.

## The edit

The incremental builds add one comment line (`// build-times probe ...`) to a file. A comment cannot change
behaviour, yet the file really changes. `--edit` says where it goes:

- `middle` (the default): before the top-level item nearest the middle of the file that follows a blank line
  (not a doc comment or an attribute). At `78043f09a` that is line 708 of 1149 in `level/mod.rs`, 7434 of 8436
  in `gfx.rs` and 17519 of 20333 in `game.rs`; `items.rs` has no such item, so its comment goes on line 1, as
  with `top`, and the report warns. Every line below the comment moves, which is what an edit there does to
  rustc's incremental cache: the code below must be compiled again. The report gives the line (`file:line`).
- `top`: the first line. All the file's code moves.
- `append`: the last line. Almost nothing moves, so rustc has little to do. It shows how cheap a build can be.
- `touch`: no change to the bytes, only a new modification time.

Cargo decides on the file's modification time, not its content: after any of the four it runs rustc for the
crate, and for every crate that depends on it, because their input was written again. What rustc then
recompiles depends on what moved.

For a fair start, each measurement is preceded by a build of the unedited file in the same target folder. After the
measured build the file is put back byte for byte and built once more (not timed), then its old modification
time is restored. A build that fails is followed by that build of the unedited file too, so the next measurement
starts from it. The file and its modification time are put back on Ctrl+C, on an error, on SIGTERM and on
Ctrl+Break (Windows). If the process is killed outright, a note in `.local/build-times/` lets the next run
restore the file first. Either way, a file that something else wrote in the meantime (your editor) is left as it
is, and the run stops and says so: putting the old bytes back would undo that work.

## Compare

```sh
python3 tools/build_times.py compare before.json after.json
```

A table of seconds before, after and the change, for the cold builds, the nothing-changed build, every
incremental build, every crate of the cold builds' slowest list, and the two critical paths. When the
CPU, OS, RAM, jobs, rustc, edit mode, edited files or build variables (`RUSTFLAGS`, `RUSTC_WRAPPER`...) differ
between the reports, it says so first: those seconds are not comparable.

## First numbers

Measured on 2026-11-01 at `78043f09a` (main) plus the tool, with `rustc 1.99.1` and the default jobs (16, the logical
CPUs). Seconds, wall clock. Incremental builds use the default `middle` edit; the median of 3 runs for `dev`
and of 2 (Windows) or 1 (Linux) for `fast`.

| Machine | Windows 11 (26300), native | Linux: Ubuntu 26.04 in WSL2 on the same PC |
|---|---|---|
| CPU | AMD Ryzen 7 7700X, 8 cores, 16 threads | the same, 16 threads |
| RAM | 32 GB | 8 GB seen by the VM |
| Disk | SATA SSD (C:), repository on NVMe (D:) | the VM's ext4 disk |
| Linker | MSVC `link.exe` | the default `cc` |

| Build | Windows dev | Linux dev | Windows fast | Linux fast |
|---|---|---|---|---|
| Cold | 106.6 | 99.9 | 235.6 | 184.7 |
| Nothing changed | 0.4 | 0.2 | 0.4 | 0.2 |
| Edit in `horizon-world` (`level/mod.rs`) | 21.4 | 22.0 | 29.5 | 60.5 |
| Edit in `horizon-client-render` (`gfx.rs`) | 15.8 | 12.2 | 78.3 | 51.6 |
| Edit in `horizon-client` (`game.rs`) | 14.4 | 10.6 | 92.3 | 60.6 |
| Edit in `horizon-items` (`items.rs`) | 23.1 | 19.0 | 127.5 | 83.8 |
| Crates recompiled by those four edits | 9, 2, 1, 3 | the same | the same | the same |

Crates recompiled: an edit in `horizon-world` recompiles `horizon-world` and the eight crates above it
(`horizon-items`, `horizon-protocol`, `horizon-command`, `horizon-client-core`, `horizon-client-input`,
`horizon-server-files`, `horizon-client-render` and `horizon-client`); one in `horizon-items` recompiles it,
`horizon-client-render` and `horizon-client`; one in `horizon-client-render` recompiles it and `horizon-client`;
one in `game.rs` only `horizon-client`.

What the cold build's timings page shows (`dev`, Windows):

- 361 units. The critical path starts with `proc-macro2`, `quote` and `syn`, goes through the `windows` crates
  (`windows` itself 20 s), `wgpu-hal` (7 s), `wgpu-core` (7 s) and `horizon-client-render` (14 s), and ends with
  `horizon-client` the library (21 s, from second 50 to 72) and then `horizon-client` the binary (35 s, from 72
  to 107). Beside the library only `horizon-client-render` runs, for its last 4 s; beside the binary nothing runs
  but `horizon-ctl` for 3 s. A third of the build is one rustc on one crate.
- The crates that cost most compiler time added up: `horizon-client` 60 s, `windows` 35 s, `horizon-world` 27 s,
  `naga` 17 s, `windows-sys` 15 s, `ash` 15 s, `horizon-client-render` 14 s.
- In `fast`, `horizon-client` costs 160 s added up, `horizon-world` 67 s, `windows` 57 s, `naga` 54 s,
  `horizon-client-render` 34 s. The critical path is nearly the same chain, longer: `windows` 33 s, `wgpu-hal` 31 s,
  `wgpu-core` 27 s, `horizon-client-render` 34 s, the library 43 s, the binary 114 s.

Which edit is used changes the incremental numbers (`dev`, Windows, 2 runs each, 3 for `middle`; in `items.rs`
`middle` is the `top` edit, see above):

| Edit | `horizon-world` | `horizon-client-render` | `horizon-client` | `horizon-items` |
|---|---|---|---|---|
| `touch` (no change to the bytes) | 19.2 | 14.6 | 8.7 | 14.9 |
| `append` (comment on the last line) | 19.2 | 14.9 | 9.0 | 15.1 |
| `middle` (comment between items) | 21.4 | 15.8 | 14.4 | 23.1 |
| `top` (comment on the first line) | 22.4 | 24.4 | 16.8 | 23.1 |

`touch` and `append` recompile exactly the same crates as the other two: cargo goes by the modification time, and
rustc finds nothing new in the code. What they cost is the floor: rustc's startup and analysis of each crate
on the way up, plus the link. A comment further up moves the lines below it, so rustc compiles that code again;
that is the 1 to 10 s more.

Load. These runs were done while no other build or test was running. The CPU was 7% to 17% busy before the `dev`
builds of the main Windows run (the Unreal Editor and background services are open on that PC); one measurement,
at 16.6%, was warned about. In the `touch`, `append` and `top` runs it was 5% to 26%, and two measurements were
warned about (25.6% and 21.5%). The `fast` runs on Windows and all Linux runs had no warning. Two earlier
attempts, made while another job was running tests on the PC, were thrown away. The Linux runs are in a WSL2
virtual machine: they show the shape on Linux, not what a bare-metal Linux machine on this hardware does.
Repeated incremental runs differ by 1.5 s at most in `dev`; for `fast` on Windows, 2 runs agreed to within 2 s.
