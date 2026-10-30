#!/usr/bin/env python3
"""Bakes the static Bricolage Grotesque files in crates/horizon-theme/fonts from upstream's variable font.

GPUI picks a font file by family and weight, so the launcher can't use the variable font itself:
this cuts the instances it uses (docs/design-system.md, "Night"): the UI family at optical size 14
in Regular, Medium, SemiBold and Bold, and the display family (Bold at optical size 72) under its
own family name. Upstream: github.com/ateliertriay/bricolage, via google/fonts (SIL OFL 1.1, no
reserved font name).

Bricolage has no Cyrillic, so each cut also takes Geist's (github.com/vercel/geist-font via
google/fonts, SIL OFL 1.1, no reserved font name; fonts/OFL-Geist.txt): the letters drawn like
Latin ones (А, В, Е, О, Р, С, у...) use Bricolage's own Latin glyphs; the others are Geist's
outlines, capitals scaled to Bricolage's cap height and small letters to its x-height, each case
from the Geist weight whose stems come out as thick as Bricolage's after scaling. The result is a
modified version of both fonts under the OFL, which allows it.

    python3 -m venv /tmp/ft && /tmp/ft/bin/pip install fonttools
    /tmp/ft/bin/python tools/bake_bricolage.py 'BricolageGrotesque[opsz,wdth,wght].ttf' 'Geist[wght].ttf'
"""

import sys
import unicodedata
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

OUT = Path(__file__).resolve().parent.parent / "crates/horizon-theme/fonts"
UI = "Bricolage Grotesque"
DISPLAY = "Bricolage Grotesque Display"
CUTS = [
    (UI, "Regular", 14, 400),
    (UI, "Medium", 14, 500),
    (UI, "SemiBold", 14, 600),
    (UI, "Bold", 14, 700),
    (DISPLAY, "Bold", 72, 700),
]
GEIST_COPYRIGHT = "Copyright 2024 The Geist Project Authors (https://github.com/vercel/geist-font)"
# The Cyrillic block and its supplement.
CYRILLIC = range(0x400, 0x530)
# Cyrillic letters drawn like a Latin letter (or a Latin letter with a diaeresis): Bricolage's own.
LATIN_LOOKING = dict(
    zip(
        map(ord, "АВЕКМНОРСТХЅІЈаеорсухѕіјЁёЇї"),
        map(ord, "ABEKMHOPCTXSIJaeopcyxsijËëÏï"),
    )
)


def rename(font: TTFont, family: str, style: str) -> None:
    """Names the instance as a plain static family: GPUI matches the family (1, 16) and weight."""
    full = f"{family} {style}"
    ps = f"{family.replace(' ', '')}-{style}"
    names = font["name"]
    names.names = [n for n in names.names if n.nameID not in (16, 17, 21, 22, 25)]
    for nid, value in [(1, family), (2, style), (3, ps), (4, full), (6, ps), (16, family), (17, style)]:
        names.setName(value, nid, 3, 1, 0x409)
        names.removeNames(nameID=nid, platformID=1)
    for table in ("STAT", "fvar", "avar"):
        if table in font:
            del font[table]


def bounds(font: TTFont, char: str):
    glyphs = font.getGlyphSet()
    pen = BoundsPen(glyphs)
    glyphs[font.getBestCmap()[ord(char)]].draw(pen)
    return pen.bounds


def stem(font: TTFont) -> float:
    x_min, _, x_max, _ = bounds(font, "I")
    return x_max - x_min


def geist_for(geist: Path, want: float, scale: float) -> TTFont:
    """Geist at the weight whose `I` stem, scaled by `scale`, is `want` units thick."""
    lo, hi = 100.0, 900.0
    for _ in range(12):
        mid = (lo + hi) / 2
        if stem(instantiateVariableFont(TTFont(geist), {"wght": mid})) * scale < want:
            lo = mid
        else:
            hi = mid
    return instantiateVariableFont(TTFont(geist), {"wght": (lo + hi) / 2})


def add_cyrillic(font: TTFont, geist: Path) -> int:
    """Adds Cyrillic to the instance `font` from Geist (see the module's doc); returns the count."""
    cap = bounds(font, "H")[3]
    x_height = bounds(font, "x")[3]
    want = stem(font)
    sources = {}
    for case, char, height in (("upper", "H", cap), ("lower", "x", x_height)):
        probe = TTFont(geist)
        scale = height / bounds(instantiateVariableFont(probe, {"wght": 400}), char)[3]
        sources[case] = (geist_for(geist, want, scale), scale)
    cmap = font.getBestCmap()
    glyf, hmtx = font["glyf"], font["hmtx"]
    added = {}
    for cp in CYRILLIC:
        if cp in cmap:
            continue
        if LATIN_LOOKING.get(cp) in cmap:
            added[cp] = cmap[LATIN_LOOKING[cp]]
            continue
        source, scale = sources["upper" if unicodedata.category(chr(cp)) == "Lu" else "lower"]
        source_name = source.getBestCmap().get(cp)
        if source_name is None:
            continue
        glyphs = source.getGlyphSet()
        outline = DecomposingRecordingPen(glyphs)
        glyphs[source_name].draw(outline)
        pen = TTGlyphPen(None)
        outline.replay(TransformPen(pen, (scale, 0, 0, scale, 0, 0)))
        name = f"uni{cp:04X}"
        glyph = pen.glyph()
        glyf[name] = glyph
        glyph.recalcBounds(glyf)
        advance = round(source["hmtx"][source_name][0] * scale)
        hmtx[name] = (advance, getattr(glyph, "xMin", 0))
        added[cp] = name
    font.setGlyphOrder(glyf.glyphOrder)
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap.update(added)
    return len(added)


def main() -> None:
    source, geist = Path(sys.argv[1]), Path(sys.argv[2])
    for family, style, opsz, wght in CUTS:
        font = TTFont(source)
        font = instantiateVariableFont(font, {"opsz": opsz, "wdth": 100, "wght": wght})
        rename(font, family, style)
        cyrillic = add_cyrillic(font, geist)
        # The OFL keeps every source's copyright notice.
        notice = f"{font['name'].getDebugName(0)}; Cyrillic: {GEIST_COPYRIGHT}"
        font["name"].setName(notice, 0, 3, 1, 0x409)
        font["name"].removeNames(nameID=0, platformID=1)
        os2 = font["OS/2"]
        os2.usWeightClass = wght
        # fsSelection: bit 5 bold, bit 6 regular; macStyle bit 0 bold.
        os2.fsSelection = os2.fsSelection & ~0x61 | (0x20 if wght == 700 else 0x40 if wght == 400 else 0)
        font["head"].macStyle = 1 if wght == 700 else 0
        # The font now covers Cyrillic: Unicode range bit 9, code page 1251 (bit 2).
        os2.ulUnicodeRange1 |= 1 << 9
        os2.ulCodePageRange1 |= 1 << 2
        out = OUT / f"{family.replace(' ', '')}-{style}.ttf"
        font.save(out)
        print(f"{out.relative_to(OUT.parent.parent.parent)}: {out.stat().st_size // 1024} KiB, {cyrillic} Cyrillic")


if __name__ == "__main__":
    main()
