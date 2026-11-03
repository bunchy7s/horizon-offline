# Build environment for the launcher and for building native mods on NixOS:
# `nix-shell` in this folder, then `cargo build`, or press Build in the launcher started from it.
# The launcher's Linux check reads /etc/os-release and names this shell on NixOS (#5248).
{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ pkg-config clang mold ];
  buildInputs = with pkgs; [
    libxkbcommon wayland vulkan-loader fontconfig freetype alsa-lib openssl
    udev libx11 libxcb libxcursor libxrandr libxi
  ];
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [ libxkbcommon wayland vulkan-loader fontconfig freetype alsa-lib udev libx11 libxcb libxcursor libxrandr libxi ];
}
