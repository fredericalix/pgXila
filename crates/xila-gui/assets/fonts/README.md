# Fonts of the graphical client

The graphical client draws its own text, pixel by pixel, with two bitmap fonts
compiled into the program:

| File | Font |
|------|------|
| `xila-sans-9.bdf` | Xila Sans, regular |
| `xila-sans-bold-9.bdf` | Xila Sans, bold |

Both are 9 pixels per em: capitals are 7 pixels tall, lowercase letters 5, with
2 pixels above for accents and 2 below for tails. They cover Latin with all
French accents and ligatures, Greek and Cyrillic.

## Origin and licence

Xila Sans is a bitmap conversion of **Pixeloid Sans** and **Pixeloid Sans
Bold**, Copyright © 2020-2022 GGBotNet (<https://ggbot.net/fonts/>), published
under the SIL Open Font License, version 1.1. The glyphs are unchanged: only
the file format differs.

"Pixeloid" is a reserved font name, which the licence does not allow a modified
version to keep. That is why the converted fonts have another name.

These two font files are distributed under the SIL Open Font License 1.1, whose
text is in [`OFL.txt`](OFL.txt). The rest of pgXila is under the PostgreSQL
License.

## Regenerating the files

The original fonts are TrueType files whose outlines are squares on a regular
grid. `scripts/convert-font.py` samples each glyph at the centre of every cell
of that grid, which gives back the original bitmap exactly, and refuses to
write anything if a glyph does not sit on the grid.

```sh
NOTICE='Xila Sans is a bitmap conversion of Pixeloid Sans, Copyright (c) 2020-2022 GGBotNet (https://ggbot.net/fonts/), SIL Open Font License 1.1'

scripts/convert-font.py PixeloidSans.ttf crates/xila-gui/assets/fonts/xila-sans-9.bdf \
    --pixels-per-em 9 --family "Xila Sans" --weight Medium --notice "$NOTICE"
scripts/convert-font.py PixeloidSans-Bold.ttf crates/xila-gui/assets/fonts/xila-sans-bold-9.bdf \
    --pixels-per-em 9 --family "Xila Sans" --weight Bold --notice "$NOTICE"
```

The script needs Python 3 with fontTools and Pillow. Building pgXila does not
run it.
