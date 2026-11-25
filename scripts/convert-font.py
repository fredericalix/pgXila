#!/usr/bin/env python3
"""Converts a pixel font stored as outlines (TrueType) into a BDF bitmap font.

The graphical client draws its own pixels and only reads bitmap fonts. Pixel
fonts are usually published as TrueType files whose outlines are squares on a
regular grid. This script samples each glyph at the centre of every cell of
that grid, which gives back the original bitmap exactly, and writes it as BDF.

It refuses to write anything if a glyph does not sit on the grid.

Usage:
    scripts/convert-font.py SOURCE.ttf OUTPUT.bdf --pixels-per-em 9 \
        --family "Xila Sans" --weight Medium --notice "..."

Requires Python 3 with fontTools and Pillow. It is only needed to regenerate
the files in crates/xila-gui/assets/fonts; building pgXila does not run it.
"""

import argparse
import sys

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

# Each grid cell is rendered on OVERSAMPLE x OVERSAMPLE pixels before sampling.
OVERSAMPLE = 8
# Room around the origin, in cells, large enough for any glyph.
LEFT, RIGHT, ABOVE, BELOW = 4, 20, 14, 6


def sample(font, char):
    """Returns the set of (column, row) cells of a glyph; rows grow upwards
    from the baseline, columns grow rightwards from the origin."""
    k = OVERSAMPLE
    image = Image.new("L", ((LEFT + RIGHT) * k, (ABOVE + BELOW) * k), 0)
    ImageDraw.Draw(image).text((LEFT * k, ABOVE * k), char, font=font, fill=255, anchor="ls")
    pixels = image.load()
    cells = set()
    for row in range(ABOVE + BELOW):
        for column in range(LEFT + RIGHT):
            lit = sum(
                pixels[column * k + x, row * k + y] > 127 for y in range(k) for x in range(k)
            )
            coverage = lit / (k * k)
            if 0.1 < coverage < 0.9:
                raise ValueError(
                    f"U+{ord(char):04X} is not on the pixel grid "
                    f"(cell {column - LEFT},{ABOVE - 1 - row} is {coverage:.0%} covered)"
                )
            if coverage >= 0.9:
                cells.add((column - LEFT, ABOVE - 1 - row))
    return cells


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("source")
    parser.add_argument("output")
    parser.add_argument("--pixels-per-em", type=int, required=True)
    parser.add_argument("--family", required=True)
    parser.add_argument("--weight", default="Medium")
    parser.add_argument("--notice", required=True, help="copyright and origin of the glyphs")
    args = parser.parse_args()

    ttfont = TTFont(args.source)
    units_per_em = ttfont["head"].unitsPerEm
    if units_per_em % args.pixels_per_em:
        sys.exit(f"{units_per_em} units per em is not a multiple of {args.pixels_per_em} pixels")
    unit = units_per_em // args.pixels_per_em
    ascent = round(ttfont["hhea"].ascent / unit)
    descent = round(-ttfont["hhea"].descent / unit)
    cmap = ttfont.getBestCmap()
    advances = ttfont["hmtx"]
    font = ImageFont.truetype(args.source, args.pixels_per_em * OVERSAMPLE)

    glyphs = []
    for codepoint in sorted(cmap):
        if codepoint < 0x20 or codepoint > 0xFFFF:
            continue
        advance, _ = advances[cmap[codepoint]]
        if advance % unit:
            sys.exit(f"U+{codepoint:04X}: advance {advance} is not a whole number of pixels")
        try:
            cells = sample(font, chr(codepoint))
        except ValueError as error:
            sys.exit(str(error))
        glyphs.append((codepoint, advance // unit, cells))

    lines = [
        "STARTFONT 2.1",
        f"FONT -pgxila-{args.family}-{args.weight}-R-Normal--{args.pixels_per_em}-"
        f"{args.pixels_per_em * 10}-75-75-P-0-ISO10646-1",
        f"SIZE {args.pixels_per_em} 75 75",
        f"FONTBOUNDINGBOX {LEFT + RIGHT} {ascent + descent} 0 {-descent}",
        "STARTPROPERTIES 5",
        f'FAMILY_NAME "{args.family}"',
        f'WEIGHT_NAME "{args.weight}"',
        f"FONT_ASCENT {ascent}",
        f"FONT_DESCENT {descent}",
        f'COPYRIGHT "{args.notice}"',
        "ENDPROPERTIES",
        f"CHARS {len(glyphs)}",
    ]
    for codepoint, advance, cells in glyphs:
        if cells:
            left = min(c for c, _ in cells)
            right = max(c for c, _ in cells)
            bottom = min(r for _, r in cells)
            top = max(r for _, r in cells)
        else:
            left, right, bottom, top = 0, -1, 0, -1
        width, height = right - left + 1, top - bottom + 1
        lines += [
            f"STARTCHAR U+{codepoint:04X}",
            f"ENCODING {codepoint}",
            f"SWIDTH {advance * 1000 // args.pixels_per_em} 0",
            f"DWIDTH {advance} 0",
            f"BBX {width} {height} {left if cells else 0} {bottom if cells else 0}",
            "BITMAP",
        ]
        row_bytes = (width + 7) // 8
        for row in range(top, bottom - 1, -1):
            bits = 0
            for column in range(left, right + 1):
                bits = (bits << 1) | ((column, row) in cells)
            bits <<= row_bytes * 8 - width
            lines.append(f"{bits:0{row_bytes * 2}X}")
        lines.append("ENDCHAR")
    lines.append("ENDFONT")

    with open(args.output, "w", encoding="ascii") as out:
        out.write("\n".join(lines) + "\n")
    print(f"{args.output}: {len(glyphs)} glyphs, {args.pixels_per_em} pixels per em, "
          f"ascent {ascent}, descent {descent}")


if __name__ == "__main__":
    main()
