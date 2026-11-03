#!/usr/bin/env bash
# The vanilla data for this checkout on the self-hosted runner. Horizon never downloads the game: the runner
# holds the maintainer's own copy, imported and decompiled once (HORIZON_SHARED_SOURCE, default
# ~/code/horizon-parity/shared-local; tools/ci/linux-runner.sh sets it on Linux). It is cloned once into the runner's
# work folder ($RUNNER_WORKSPACE/shared-local; APFS clones, or reflinks where the file system has them, so no second
# copy of the bytes), and .local/ links to it as in a parity worktree.
# .local/horizon-client and the exporter outputs beside it, what horizon-prepare makes, are prepared by this
# checkout's own preparer once per preparer version (its sources, the exporters and research-lock.json), so a
# change to the preparer is checked with what it prepares; the four newest preparations are kept.
set -euo pipefail
shared="${RUNNER_WORKSPACE:?}/shared-local"
source="${HORIZON_SHARED_SOURCE:-$HOME/code/horizon-parity/shared-local}"
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
if [ ! -f "$shared/minecraft/$version/prepared.json" ]; then
  [ -f "$source/minecraft/$version/prepared.json" ] || { echo "::error::no imported Minecraft $version in $source"; exit 1; }
fi
# Whatever the source holds and this runner's copy doesn't yet: all of it the first time, and later additions (the
# sounds came after the release runner's first clone, and its audio harnesses failed for want of them).
mkdir -p "$shared"
for entry in "$source"/*; do
  name=$(basename "$entry")
  [ "$name" = horizon-client ] || [ -e "$shared/$name" ] && continue
  if [ "$(uname)" = Darwin ]; then cp -cR "$entry" "$shared/"; else cp -R --reflink=auto "$entry" "$shared/"; fi
done
mkdir -p .local "$shared/prepared"
for entry in "$shared"/*; do
  name=$(basename "$entry")
  case "$name" in prepared|sounds|languages|*.log) continue ;; esac
  if [ -L ".local/$name" ] || [ ! -e ".local/$name" ]; then ln -sfn "$entry" ".local/$name"; fi
done

# HORIZON_PREPARE_KEY: the same key, computed where the Git history is (tools/pr_ready.py on another machine).
sha256() { if command -v shasum > /dev/null; then shasum -a 256; else sha256sum; fi; }
key=${HORIZON_PREPARE_KEY:-$(git ls-files -s crates/horizon-prepare mods/horizon/src/test/java research-lock.json | sha256 | cut -c1-16)}
prepared="$shared/prepared/$key"
# Two runners share the vanilla data (~/.actions-runners/horizon-2/_work/horizon/shared-local links to the first
# one's): one preparation at a time. A lock older than 30 minutes belongs to a job that died.
lock="$shared/prepared/.lock"
until mkdir "$lock" 2>/dev/null; do
  if [ -n "$(find "$lock" -maxdepth 0 -mmin +30 2>/dev/null)" ]; then rmdir "$lock" 2>/dev/null || true; fi
  sleep 5
done
trap 'rmdir "$lock" 2>/dev/null || true' EXIT
# horizon-prepare writes .local/horizon-client and its exporters' outputs beside it: all of them live in
# $prepared, listed in $prepared/.entries, and .local/ links to them. A real folder of that kind in .local/ is
# what an older failed preparation left: it would hide the preparations (and a new one would not count it as its
# own output), so it goes.
outputs=$({ cat "$shared"/prepared/*/.entries; cd .local && ls -d horizon-client*; } 2>/dev/null || true)
for name in $(echo "$outputs" | LC_ALL=C sort -u); do
  if [ -e ".local/$name" ] && [ ! -L ".local/$name" ]; then
    rm -rf ".local/${name:?}"
    echo "Removed .local/$name, left by a failed preparation"
  fi
done
# A preparation made while such a folder was there did not list it (horizon-client above all), so it is made again.
if [ -f "$prepared/.entries" ] && ! grep -qx horizon-client "$prepared/.entries"; then
  echo "Preparation $key lacks horizon-client: preparing again"
  rm -rf "$prepared"
fi
if [ ! -f "$prepared/.entries" ]; then
  echo "Preparing resources with this checkout's horizon-prepare ($key)"
  rm -rf "$prepared" && mkdir -p "$prepared"
  # Links to another preparation's outputs go first, so what this one writes shows up as new entries.
  for entry in .local/*; do
    if [ -L "$entry" ] && [[ "$(readlink "$entry")" == "$shared/prepared/"* ]]; then rm "$entry"; fi
  done
  before=$(ls -1A .local | LC_ALL=C sort)
  started=$SECONDS
  cargo build --release --locked -p horizon-prepare
  if ! target/release/horizon-prepare --root "$PWD" > "$RUNNER_TEMP/prepare.log" 2>&1; then
    tail -40 "$RUNNER_TEMP/prepare.log"
    # Nothing of a failed preparation stays in .local/: the next job prepares again.
    LC_ALL=C comm -13 <(echo "$before") <(ls -1A .local | LC_ALL=C sort) |
      while read -r name; do rm -rf ".local/${name:?}"; done
    rm -rf "$prepared"
    exit 1
  fi
  LC_ALL=C comm -13 <(echo "$before") <(ls -1A .local | LC_ALL=C sort) > "$RUNNER_TEMP/prepared-entries"
  while read -r name; do mv ".local/$name" "$prepared/$name"; done < "$RUNNER_TEMP/prepared-entries"
  mv "$RUNNER_TEMP/prepared-entries" "$prepared/.entries"
  echo "Prepared in $((SECONDS - started)) s: $(tr '\n' ' ' < "$prepared/.entries")"
fi
# The game finds some data relative to its resources (<resources>/../../datagen): beside the prepared folders,
# the same neighbours as in the shared folder.
for entry in "$shared"/*; do
  name=$(basename "$entry")
  case "$name" in prepared|sounds|languages|*.log) continue ;; esac
  [ -e "$prepared/$name" ] || ln -sfn "$entry" "$prepared/$name"
done
while read -r name; do
  if [ -L ".local/$name" ] || [ ! -e ".local/$name" ]; then ln -sfn "$prepared/$name" ".local/$name"; else
    echo "::error::.local/$name is not a link to the prepared resources"; exit 1; fi
done < "$prepared/.entries"
# The game's sounds (horizon-prepare sounds copies them from the official launcher's asset store) depend only on the
# Minecraft version: kept once in $shared/sounds/<version>/ (sounds.json and sounds/) and copied into each preparation
# as clones (APFS, or reflinks on Linux: no second copy of the bytes). Not links: vanilla's resource packs refuse
# symbolic links inside a pack, so its sound oracles would find no sound files.
assets="$prepared/horizon-client/$version/assets/minecraft"
if [ -f "$shared/sounds/$version/sounds.json" ] && [ -d "$assets" ]; then
  for entry in sounds.json sounds; do
    [ -L "$assets/$entry" ] && rm "$assets/$entry"
    [ -e "$assets/$entry" ] && continue
    if [ "$(uname)" = Darwin ]; then cp -cR "$shared/sounds/$version/$entry" "$assets/"
    else cp -R --reflink=auto "$shared/sounds/$version/$entry" "$assets/"; fi
  done
fi
# The languages (horizon-prepare languages copies them from the official launcher's asset store, which no runner
# has) depend only on the Minecraft version too: kept once in $shared/languages/<version>/ (assets/pack.mcmeta, the
# language list, and assets/minecraft/lang/) and cloned into each preparation to fill missing files. Without them a
# preparation has en_us alone, and pack_screen's harness, which draws ten languages, fails.
languages="$shared/languages/$version/assets"
if [ -d "$languages/minecraft/lang" ] && [ -d "$assets" ]; then
  mkdir -p "$assets/lang"
  # File by file: `cp -n` exits 1 on macOS (and on some coreutils) when it skips one, which ends the script.
  for file in "$languages/minecraft/lang/"*; do
    [ -e "$assets/lang/${file##*/}" ] && continue
    if [ "$(uname)" = Darwin ]; then cp -c "$file" "$assets/lang/"; else cp --reflink=auto "$file" "$assets/lang/"; fi
  done
  if [ ! -e "$assets/../pack.mcmeta" ]; then
    if [ "$(uname)" = Darwin ]; then
      cp -c "$languages/pack.mcmeta" "$assets/../"
    else
      cp --reflink=auto "$languages/pack.mcmeta" "$assets/../"
    fi
  fi
fi
touch "$prepared"
# The four newest preparations stay.
ls -1dt "$shared/prepared"/*/ 2>/dev/null | tail -n +5 | while read -r old; do rm -rf "$old"; done
