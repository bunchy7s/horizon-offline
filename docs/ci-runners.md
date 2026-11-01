# CI runners

GitHub gives the organisation 20 hosted jobs at a time, shared by every pull request, every push to main and the
bots. On 2026-10-31 that pool ran ~12 jobs on average with 400+ runs queued and every job waited 20–25 minutes for a
runner, while its work took 0.3–4 minutes. A quarter of that pool's time went to bots that run for seconds; those
run on our own machines now, and the heavy checks can follow when the pool is full.

| Runner | Machine | Labels | Takes |
| --- | --- | --- | --- |
| `playhorizon-ci-1`, `playhorizon-ci-2` | playhorizon (partner's server, 8 vCPU) | `playhorizon-ci` | Only with `SELF_HOSTED_CI=all`: `checks`' `verify` and `launcher` for main's pushes and this repository's own pull requests |
| `playhorizon-light-1`, `playhorizon-light-2` | playhorizon | `playhorizon-light` | The bots that run main's own scripts: `pr labels`, `overlap note`, `cancel-on-close`, `claim command`, and `checks`' `parity-scope` |
| `playhorizon-horizon` | playhorizon | `horizon-game`, `playhorizon` | Parity ([release-flow.md](release-flow.md#the-parity-job)) |
| `ludovics-mac-mini*` | Mac mini | `horizon-game`, `horizon-parity-main`, `horizon-release`, `horizon-nightly` | Parity, parity-main, macOS builds, releases |

Everything else stays on GitHub's runners: `checks` by default, a fork's pull request always, the Windows and Linux
builds, and every job that holds a secret (signing, publishing: the partner keeps hypervisor access to playhorizon).

## Which jobs, and when

The repository variable `SELF_HOSTED_CI` decides:

| `SELF_HOSTED_CI` | Bots (`playhorizon-light`) | `checks` of main and same-repository pull requests (`playhorizon-ci`) |
| --- | --- | --- |
| unset (default) | playhorizon | GitHub |
| `all` | playhorizon | playhorizon |
| `off` | GitHub | GitHub |

`gh variable set SELF_HOSTED_CI --body all --repo horizon-rust/horizon` (or `off`; `gh variable delete` for the
default) takes effect from the next run; queued jobs keep the runner they asked for.

Why the checks stay on GitHub by default: the checks runners have CPUs 4–7, shared with the parity runner, about
one GitHub runner's worth. With every same-repository pull request on them (#5415, 2026-10-31 20:25–20:45 UTC), 19
jobs waited there, the oldest 18 minutes, while GitHub's pool, rid of the bots, ran 9–12 of its 20 jobs with nothing
waiting. Use `all` when GitHub's pool is full again (hosted jobs waiting tens of minutes) and playhorizon is quiet.
Set `off` when playhorizon is down or moves: the bots would wait for it.

## How long

Measured 2026-10-31 on #5415, with playhorizon's parity runner busy on the same CPUs (load ~15–20):

| Job | GitHub's runners: wait + work | playhorizon: wait + work |
| --- | --- | --- |
| `verify` (Rust changed) | 20–25 min + ~8 min (clippy 2.4, tests 4.2, Python 1.3) | ~0 + 10.5 min (clippy 3.7, tests 5.0, Python 1.7) |
| `launcher` (launcher changed) | 20–25 min + 2.1 min | ~0 + 2.5 min |
| a bot (`pr labels`, `overlap note`...) | 20–26 min + 0.3–0.9 min | ~0 + seconds |

The work is ~30% slower here (4 CPUs shared with parity, against a GitHub runner's 4 of its own), up to 16 minutes
for `verify` when the machine is loaded (load ~30). That pays only while GitHub's pool makes jobs wait longer than
that. A runner's first build of a workspace is cold: ~14 min for `verify`, ~11 for `launcher`.

## Forks never run here

A fork's pull request is code nobody here wrote. Three locks keep it off our runners:

1. The workflows pick the runner by event: `checks` sends a `pull_request` to playhorizon only when
   `github.event.pull_request.head.repo.full_name == github.repository` (and `SELF_HOSTED_CI=all`), else to
   `ubuntu-latest`. The bots run on
   `pull_request_target`, `issue_comment` and `schedule`, which always run main's workflow and main's scripts, never
   the pull request's (each workflow's header says how). `tests/test_ci_runners.py` pins both.
2. A fork's pull request runs its *own* copy of the workflows, so it could name these labels anyway. Before every
   job, the runners' `job-started.sh` (`ACTIONS_RUNNER_HOOK_JOB_STARTED`) fails any pull request event whose head
   repository isn't this one, before a line of it is on disk.
3. The repository asks a maintainer to approve the runs of every outside contributor's pull request (Settings →
   Actions → "Require approval for all external contributors").

## On playhorizon

Installed by `tools/ci/checks-runners.sh` (run it there; it reads the registration token on stdin, so the token never
touches the disk or the screen):

```sh
gh api -X POST repos/horizon-rust/horizon/actions/runners/registration-token --jq .token |
  ssh ludovic@playhorizon 'bash ~/horizon-install/checks-runners.sh install'
ssh ludovic@playhorizon 'bash ~/horizon-install/checks-runners.sh status'
```

- Each runner is a systemd user service (`horizon-runner-ci-<n>`, `horizon-runner-light-<n>`) in
  `~/.actions-runners/horizon-{ci,light}-<n>`, with its own work folder and `HOME`.
- All of them run nice and pinned to CPUs 4–7, inside `horizon-ci.slice` (the CPU quota and memory cap of all CI on
  the machine, `linux-runner.sh`). The checks runners have their own `horizon-ci-checks.slice` (memory ≤ 16 GB).
  CPUs 0–3 stay for `tools/remote.sh` runs, benchmarks and the servers.
- The checks runners share one cargo home (`~/.actions-runners/ci-shared/cargo`: registry and git sources). Each has
  its own rustup home and its own build folder (`CARGO_TARGET_DIR`, outside the checkout, so the checkout's clean
  keeps it): a run rebuilds only what changed. `CARGO_BUILD_JOBS=4`. `job-started.sh` empties a build folder over
  40 GB (`MAX_TARGET_GB`) before the job.
- No sccache: each build folder already holds the dependencies, and the host's shared cache is the remote runs'.
- Python is the host's 3.13 (`ci-shared/bin/python3`); the apt packages the jobs need are installed by the script, so
  the jobs skip their `apt-get`, `setup-python` and Rust cache steps there (`runner.environment == 'github-hosted'`).
