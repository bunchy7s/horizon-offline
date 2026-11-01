#!/usr/bin/env bash
# The checks runners on playhorizon (the partner's Linux server, docs/ci-runners.md): GitHub's 20 hosted jobs at a
# time are shared by every pull request, so the same-repository ones and main's pushes run here instead.
#
#   bash tools/ci/checks-runners.sh install < token   # register (token on stdin), install, start; again to update
#   bash tools/ci/checks-runners.sh status
#   bash tools/ci/checks-runners.sh uninstall < token  # stop and unregister (a remove-token)
#
# The token is made on a machine whose gh has admin on the repository, never printed or stored here:
#   gh api -X POST repos/horizon-rust/horizon/actions/runners/registration-token --jq .token |
#     ssh ludovic@playhorizon 'bash ~/horizon-install/checks-runners.sh install'
#
# Two kinds of runner, each a systemd user service in ~/.actions-runners/<name>:
# - horizon-ci-<n> (label playhorizon-ci, CI_RUNNERS of them, default 2): ci.yml's verify and launcher jobs. Each has
#   its own work folder, rustup home and cargo target folder (outside the checkout, so the checkout's clean leaves it),
#   and they share one cargo home (registry and git sources).
# - horizon-light-<n> (label playhorizon-light, LIGHT_RUNNERS, default 2): the bots that run main's own scripts on
#   pull_request_target and comment events (pr labels, overlap note, cancel-on-close, claim command). No Rust.
# All of them run nice, pinned to CPUs 4-7 inside horizon-ci.slice (linux-runner.sh: CPU quota and memory cap for
# all CI on the machine), so CPUs 0-3 stay for tools/remote.sh runs, benchmarks and the servers.
# Before every job, ci-shared/job-started.sh fails a job from a fork's pull request (whatever its workflow file says)
# and empties a cargo target folder grown past MAX_TARGET_GB.
set -euo pipefail
repo=horizon-rust/horizon
base=$HOME/.actions-runners
shared=$base/ci-shared
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
version=2.338.0
sha=af4b794c1bc41d73d40535e3fe092a39f9679cd8d965954c2aca25a05ca41d32
ci=${CI_RUNNERS:-2} light=${LIGHT_RUNNERS:-2} cpus=${RUNNER_CPU_LIST:-4-7} jobs=${CI_JOBS:-4}
packages="build-essential pkg-config git curl jq gh libasound2-dev libudev-dev libxkbcommon-x11-dev libwayland-dev
  libx11-xcb-dev libfontconfig-dev libvulkan-dev"

runners() {
  for i in $(seq 1 "$ci"); do echo "horizon-ci-$i playhorizon-ci"; done
  for i in $(seq 1 "$light"); do echo "horizon-light-$i playhorizon-light"; done
}

shared_files() {
  mkdir -p "$shared/bin" "$shared/cargo"
  local python
  python=$(command -v python3.13 || echo "$HOME/.local/bin/python3.13")
  [ -x "$python" ] || { echo "No python3.13: uv python install 3.13, then link it into ~/.local/bin"; exit 1; }
  ln -sfn "$python" "$shared/bin/python3"
  ln -sfn "$python" "$shared/bin/python"
  cat > "$shared/job-started.sh" <<'EOF'
#!/usr/bin/env bash
# ACTIONS_RUNNER_HOOK_JOB_STARTED of the checks runners (tools/ci/checks-runners.sh): runs before each job's steps;
# a failure fails the job. A fork's pull request runs its own copy of the workflows, so its runs-on can name these
# runners whatever ci.yml says: such a job stops here, before any of its code is on disk.
set -euo pipefail
case ${GITHUB_EVENT_NAME:-} in
  pull_request|pull_request_review|pull_request_review_comment|merge_group)
    head=$(jq -r '.pull_request.head.repo.full_name // "unknown"' "${GITHUB_EVENT_PATH:-/dev/null}" 2>/dev/null || echo unknown)
    if [ "$head" != "${GITHUB_REPOSITORY:-}" ]; then
      echo "::error::A pull request from $head never runs on a self-hosted runner: use runs-on ubuntu-latest for forks"
      exit 1
    fi;;
esac
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -d "$CARGO_TARGET_DIR" ]; then
  size=$(du -s --block-size=1G "$CARGO_TARGET_DIR" | cut -f1)
  if [ "$size" -gt "${MAX_TARGET_GB:-40}" ]; then
    echo "Cargo target folder at ${size} GB (over ${MAX_TARGET_GB:-40}): emptied, this job builds from scratch"
    rm -rf "${CARGO_TARGET_DIR:?}"
  fi
fi
EOF
  chmod +x "$shared/job-started.sh"
}

unpack() {
  local dir=$1 cache=$HOME/.cache/gha-runner/actions-runner-linux-x64-$version.tar.gz
  if ! echo "$sha  $cache" | sha256sum -c --quiet - > /dev/null 2>&1; then
    mkdir -p "$(dirname "$cache")"
    curl -fsSL -o "$cache" "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz"
    echo "$sha  $cache" | sha256sum -c --quiet -
  fi
  mkdir -p "$dir"
  tar xzf "$cache" -C "$dir"
}

rust() {
  # Each runner's own rustup home (two updating one at once collide), the shared cargo home's proxies. No
  # self-update: it would rewrite those shared proxies under the other runner.
  local dir=$1
  export RUSTUP_HOME=$dir/rustup CARGO_HOME=$shared/cargo
  if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none > /dev/null
  fi
  "$CARGO_HOME/bin/rustup" set auto-self-update disable > /dev/null
  "$CARGO_HOME/bin/rustup" toolchain install stable --profile minimal -c rustfmt -c clippy > /dev/null
  "$CARGO_HOME/bin/rustup" default stable > /dev/null
}

install() {
  local token="" missing name label dir slice
  token=$(cat)
  [ -n "$token" ] || { echo "No registration token on stdin"; exit 1; }
  missing=$( (dpkg-query -W -f='${Package} ${Status}\n' $packages 2>&1 || true) |
    awk '/^dpkg-query: no packages found matching/ {print $NF; next} !/ installed$/ {print $1}' | tr '\n' ' ')
  if [ -n "${missing// /}" ]; then
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $missing > /dev/null
  fi
  [ -f "$units/horizon-ci.slice" ] || { echo "No horizon-ci.slice: install tools/ci/linux-runner.sh first"; exit 1; }
  shared_files
  cp "$0" "$shared/checks-runners.sh"
  # The checks' share of the CI slice's memory: a build that runs out stops there, not the parity runner's jobs.
  cat > "$units/horizon-ci-checks.slice" <<EOF
[Unit]
Description=Horizon CI: the checks runners' jobs (tools/ci/checks-runners.sh)
[Slice]
MemoryHigh=12G
MemoryMax=16G
EOF
  while read -r name label; do
    dir=$base/$name
    if [ ! -f "$dir/.runner" ]; then
      unpack "$dir"
      (cd "$dir" && ./config.sh --unattended --replace --url "https://github.com/$repo" --token "$token" \
        --name "playhorizon-${name#horizon-}" --labels "$label" --work _work > /dev/null)
    fi
    mkdir -p "$dir/home"
    {
      echo "LANG=C.UTF-8"
      echo "HOME=$dir/home"
      echo "ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh"
      echo "HORIZON_TELEMETRY_URL=off"
      echo "HORIZON_NO_AUDIO=1"
      echo "HORIZON_NO_HEARTBEAT=1"
      echo "HORIZON_LAUNCHER_NO_SELF_UPDATE=1"
      echo "GH_PROMPT_DISABLED=1"
      if [ "$label" = playhorizon-ci ]; then
        rust "$dir"
        echo "RUSTUP_HOME=$dir/rustup"
        echo "CARGO_HOME=$shared/cargo"
        echo "CARGO_TARGET_DIR=$dir/target"
        echo "CARGO_BUILD_JOBS=$jobs"
        echo "MAX_TARGET_GB=${MAX_TARGET_GB:-40}"
      fi
    } > "$dir/.env"
    echo "$shared/bin:$shared/cargo/bin:/usr/local/bin:/usr/bin:/bin" > "$dir/.path"
    slice=horizon-ci.slice
    [ "$label" = playhorizon-ci ] && slice=horizon-ci-checks.slice
    cat > "$units/horizon-runner-${name#horizon-}.service" <<EOF
[Unit]
Description=GitHub Actions runner playhorizon-${name#horizon-} for $repo ($label, tools/ci/checks-runners.sh)
After=network-online.target
[Service]
Slice=$slice
WorkingDirectory=$dir
ExecStart=/usr/bin/taskset -c $cpus $dir/bin/runsvc.sh
Nice=19
# GitHub's runners' umask: a user service's is 0002, and tests check the modes of the files they write.
UMask=0022
# runsvc.sh hands SIGTERM to the runner, which stops after cancelling its job.
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=5min
Restart=always
RestartSec=30
[Install]
WantedBy=default.target
EOF
  done < <(runners)
  systemctl --user daemon-reload
  while read -r name label; do
    local unit=horizon-runner-${name#horizon-}.service
    systemctl --user enable "$unit" > /dev/null 2>&1
    # The runner reads .env when it starts: restart only an idle one (a restart cancels its job).
    if pgrep -f "$base/$name/bin/Runner.Worker" > /dev/null; then echo "$name in a job: restart $unit after it"
    else systemctl --user restart "$unit"; fi
  done < <(runners)
  status
}

status() {
  systemctl --user --no-pager list-units 'horizon-runner-ci-*' 'horizon-runner-light-*'
  local dir
  for dir in "$base"/horizon-ci-* "$base"/horizon-light-*; do
    [ -d "$dir" ] || continue
    printf '%-16s %s, target %s\n' "$(basename "$dir")" \
      "$(pgrep -f "$dir/bin/Runner.Worker" > /dev/null && echo busy || echo idle)" \
      "$(du -sh "$dir/target" 2>/dev/null | cut -f1 || echo -)"
  done
}

uninstall() {
  local token name label
  token=$(cat)
  while read -r name label; do
    systemctl --user disable --now "horizon-runner-${name#horizon-}.service" 2>/dev/null || true
    [ -f "$base/$name/.runner" ] && (cd "$base/$name" && ./config.sh remove --token "$token" > /dev/null)
    rm -f "$units/horizon-runner-${name#horizon-}.service"
  done < <(runners)
  rm -f "$units/horizon-ci-checks.slice"
  systemctl --user daemon-reload
}

case ${1:-} in
  install|status|uninstall) "$1" ;;
  *) sed -n '2,10p' "$0"; exit 2 ;;
esac
