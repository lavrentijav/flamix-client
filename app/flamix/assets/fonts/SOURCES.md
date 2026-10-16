# Bundled fonts

The `ru1` build machine has no system fonts, so a font must be bundled for the
Slint software renderer (`--screenshot`). DejaVu Sans is imported from
`ui/app.slint`; Slint embeds the `.ttf` files in the binary and registers them
at runtime. On Windows the Fluent style's default (Segoe UI family) is used when
available; DejaVu is the fallback elsewhere.

| file | source |
|---|---|
| DejaVuSans.ttf | `/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf` (DejaVu Fonts, Bitstream Vera + Arev license) |
| DejaVuSans-Bold.ttf | `/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf` |
| LICENSE-DejaVu.txt | DejaVu font license (copy of the Debian `fonts-dejavu-core` copyright file) |
