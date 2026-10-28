#!/bin/sh
# Generates gpui's Windows shaders_bytes.rs on Linux, where gpui's build script can't run its
# Windows branch (it is compiled for the host). Compiles each HLSL module with the fxc shim
# (tools/fxc, D3DCompile under Wine) and writes the binding gpui's directx_renderer includes.
# Usage: gen_gpui_shaders.sh <gpui-src-dir> <out-dir>
set -e
GPUI="$1"
OUT="$2"
FXC="$(dirname "$0")/fxc"

binding="$OUT/shaders_bytes.rs"
: > "$binding"

emit() { # module hlsl target entry
    local module="$1" hlsl="$2" target="$3" entry="$4"
    local name
    name="$(printf '%s_%s_BYTES' "$(printf '%s' "$module" | tr '[:lower:]' '[:upper:]')" \
        "$(printf '%s' "$target" | cut -c1-2 | tr '[:lower:]' '[:upper:]')")"
    case "$target" in
        vs_*) name="${module^^}_VERTEX_BYTES" ;;
        ps_*) name="${module^^}_FRAGMENT_BYTES" ;;
    esac
    "$FXC" /T "$target" /E "$entry" /Fh "$OUT/$module.h" /Vn "$name" /O3 "$hlsl" 2>/dev/null
    # The header's "const BYTE name[] = {0x..,...};" becomes "const NAME: &[u8] = &[...];"
    awk -v name="$name" '
        /const BYTE/ { found = 1 }
        found {
            line = $0
            gsub(/const BYTE[^=]*=/, "", line)
            gsub(/\{/, "[", line)
            gsub(/\}/, "]", line)
            gsub(/;/, "", line)
            buffer = buffer line
        }
        found && /\}/ {
            printf "const %s: &[u8] = &%s\n", name, buffer
            found = 0
            buffer = ""
        }
    ' "$OUT/$module.h" >> "$binding"
    rm -f "$OUT/$module.h"
}

for module in quad shadow path_rasterization path_sprite underline monochrome_sprite polychrome_sprite; do
    emit "$module" "$GPUI/src/platform/windows/shaders.hlsl" vs_4_1 "${module}_vertex"
    emit "$module" "$GPUI/src/platform/windows/shaders.hlsl" ps_4_1 "${module}_fragment"
done
emit emoji_rasterization "$GPUI/src/platform/windows/color_text_raster.hlsl" vs_4_1 emoji_rasterization_vertex
emit emoji_rasterization "$GPUI/src/platform/windows/color_text_raster.hlsl" ps_4_1 emoji_rasterization_fragment

echo "wrote $binding ($(wc -l < "$binding") lines)"
