#!/usr/bin/env bash
# Packages the Linux launcher as an AppImage: one file to download, make executable and run, with
# the libraries GPUI needs beside the binary. The C library, the graphics drivers and the session
# libraries stay the machine's (see `system_library`): a Vulkan driver or a glibc from another
# machine cannot be packaged.
#
#   packaging/linux/appimage.sh BINARY VERSION OUTDIR
#
# Needs patchelf, and appimagetool: the AppImage project's, 1.9.1 by default, downloaded here and checked against
# its SHA-256 ($APPIMAGETOOL names one already on the machine, as the .dmg build takes $DMGBUILD).
# It runs without FUSE (`--appimage-extract-and-run`), so a runner needs none.
#
# The launcher updates itself by replacing the file $APPIMAGE names (launcher/src/backend.rs,
# LAUNCHER_APPIMAGE_ASSET); its menu entry starts that file, not the copy in the image's mount
# (launcher/src/desktop.rs). Both only work for an AppImage built here.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
name=horizon-launcher-linux-x86_64.AppImage

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
appdir=$work/AppDir
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" \
  "$appdir/usr/share/applications" "$appdir/usr/share/icons"

# The program.
cp "$binary" "$appdir/usr/bin/horizon-launcher"
chmod 755 "$appdir/usr/bin/horizon-launcher"

# Both places: AppRun's, and the one desktop environments read once the image is unpacked.
cp "$here/horizon-launcher.desktop" "$appdir/horizon-launcher.desktop"
cp "$here/horizon-launcher.desktop" "$appdir/usr/share/applications/horizon-launcher.desktop"
cp -R "$here/../icons/hicolor/." "$appdir/usr/share/icons/hicolor/"
cp "$here/../icons/hicolor/512x512/apps/horizon.png" "$appdir/horizon.png"
ln -s horizon.png "$appdir/.DirIcon"

# Runs the packaged binary. It finds the packaged libraries through its own run path (set below),
# not LD_LIBRARY_PATH, which every program it starts (the game, a browser, a file manager) would
# inherit and load the launcher's libraries with.
cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# AppRun: the launcher; its libraries are beside it (usr/lib, through its run path).
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/horizon-launcher" "$@"
EOF
chmod 755 "$appdir/AppRun"

# Libraries that must come from the machine: bundling any of them breaks it, in one of two ways.
system_library() {
  case "$(basename "$1")" in
    # The C library: the loader has to be the system's, and two copies of glibc in one process
    # do not work. Every library here links it, so the AppImage runs where glibc is at least the
    # build machine's (ubuntu-22.04: 2.35, so Debian 12, Ubuntu 22.04, Fedora 36 and newer).
    libc.so.* | libm.so.* | libpthread.so.* | libdl.so.* | librt.so.* | libresolv.so.* | \
      libnss_*.so.* | libutil.so.* | libanl.so.* | ld-linux*.so.* | libgcc_s.so.*) return 0 ;;
    # The graphics stack: the Vulkan loader, the GL libraries, Mesa's and the vendors' drivers
    # are matched to the GPU and its driver package on this machine.
    libGL.so.* | libGLX.so.* | libEGL.so.* | libOpenGL.so.* | libGLdispatch.so.* | \
      libglapi.so.* | libvulkan.so.* | libdrm.so.* | libgbm.so.* | libva.so.* | \
      libvdpau.so.*) return 0 ;;
    # The session's: D-Bus talks to the running session's bus (the keychain is a service on it),
    # and systemd/udev are the machine's device and service managers.
    libdbus-1.so.* | libsystemd.so.* | libudev.so.*) return 0 ;;
    # Sound: ALSA's plugins are the machine's and are loaded by name.
    libasound.so.*) return 0 ;;
  esac
  return 1
}

# Everything else the launcher links, then everything those need, until nothing new is added.
todo=("$appdir/usr/bin/horizon-launcher")
seen=" "
while [[ ${#todo[@]} -gt 0 ]]; do
  elf=${todo[0]}
  todo=("${todo[@]:1}")
  while read -r soname arrow path _; do
    [[ "$arrow" == "=>" && -f "$path" ]] || continue
    base=$(basename "$path")
    system_library "$path" && continue
    [[ "$seen" == *" $base "* ]] && continue
    seen+="$base "
    cp -L "$path" "$appdir/usr/lib/$base"
    todo+=("$appdir/usr/lib/$base")
  done < <(ldd "$elf")
done

# Each packaged file looks for the others beside the launcher: the binary in ../lib, a library in
# its own folder. A run path is the file's own, so nothing the launcher starts inherits it.
command -v patchelf > /dev/null || { echo "error: patchelf is needed (apt install patchelf)" >&2; exit 1; }
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/horizon-launcher"
for lib in "$appdir"/usr/lib/*.so*; do
  if [[ -e "$lib" ]]; then patchelf --set-rpath '$ORIGIN' "$lib"; fi
done

# Everything the launcher links is either beside it or the machine's: a library missing on the
# machine that builds this would make an image that starts nowhere. No LD_LIBRARY_PATH: the run
# paths alone must find them.
missing=$(ldd "$appdir/usr/bin/horizon-launcher" | grep "not found" || true)
[[ -z "$missing" ]] || {
  echo "error: the launcher links libraries this machine does not have:" >&2
  echo "$missing" >&2
  exit 1
}

tool=${APPIMAGETOOL:-}
if [[ -z "$tool" ]]; then
  tool=$work/appimagetool
  # Pinned: the same tool for every build, whatever the AppImage project publishes next. 1.9.1's
  # runtime mounts the image where FUSE 2 is there and unpacks it where it is not.
  url=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
  sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
  curl -fsSL -o "$tool" "$url"
  chmod 755 "$tool"
  echo "$sha256  $tool" | sha256sum -c - > /dev/null
fi

rm -f "$out/$name"
# Unpacked first, so this works on a machine (or runner) without FUSE.
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$appdir" "$out/$name"

# It starts, and is the version it was built as, before any window or GPU is needed.
got=$("$out/$name" --appimage-extract-and-run --version)
[[ "$got" == "horizon-launcher $version" ]] || {
  echo "error: expected 'horizon-launcher $version', got '$got'" >&2
  exit 1
}

# The menu entry it installs starts the AppImage, not the copy inside its mount, which is gone
# once the launcher exits. A data folder of its own: never a player's.
data=$work/data
HORIZON_LAUNCHER_HOME=$work/home XDG_DATA_HOME=$data \
  "$out/$name" --appimage-extract-and-run desktop-entry > /dev/null
grep -Fq "Exec=\"$out/$name\"" "$data/applications/horizon-launcher.desktop" || {
  echo "error: the menu entry does not start $out/$name" >&2
  cat "$data/applications/horizon-launcher.desktop" >&2
  exit 1
}

file "$out/$name"
echo "Packaged $out/$name"
