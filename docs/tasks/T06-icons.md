# T06 — icon & flag assets from the Alfa-Bank open icon pack

Read `AGENTS.md` first. This task needs NO Rust build.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T06 -b oc/T06
    cd /srv/dev/projects/flamix-client-wt/T06

## Source (MIT licensed, npm)
- `@alfalab/icons-glyph` — UI glyphs. Each icon is a CommonJS file `<Name>Icon.js` exporting a React
  component that does `createElement("svg", {fill:"currentColor", width, height, viewBox, ...}, createElement("path", {...}), ...)`.
- `@alfalab/icons-flag` — country flags `<Country>MColorIcon.js` (same React format, coloured).
Download tarballs with `npm pack @alfalab/icons-glyph @alfalab/icons-flag` (node/npm are in `/srv/dev/tools/bin`)
into a temp dir under `/srv/dev/tmp/flamix-icons/` (NOT into the repo).

## Write `scripts/icons/extract.mjs` (committed, re-runnable)
A Node script that `require`s each icon module with a **fake `react`** module (inject via
`Module._resolveFilename`/`require.cache`, or `createRequire` + a stub in `node_modules/react` of the temp dir) whose
`createElement(tag, props, ...children)` returns a tree, then serialises that tree to a clean SVG string:
camelCase attrs -> kebab-case (`fillRule` -> `fill-rule`, `clipRule`, `strokeWidth`, ...), drop `role`, `focusable`,
React-only props, keep `viewBox`, `fill`, `d`, etc. Handles `tslib` `__assign`, nested `g`, `defs`, `clipPath`, `mask`
(ids must stay unique per file). Usage: `node scripts/icons/extract.mjs <glyph-pkg-dir> <flag-pkg-dir> <out-dir>`.

## Output (committed)
1. `app/flamix/assets/icons/<semantic-name>.svg` — glyphs, **24×24 M-size** variant (`...MIcon`), monochrome. Replace
   `fill="currentColor"` with `fill="#000000"` (Slint recolours monochrome SVGs with `colorize`). Semantic names needed:

| file | meaning | suggested source glyph (pick the closest existing one; check the list) |
|---|---|---|
| sidebar-expand | expand sidebar arrow → | ArrowRight |
| sidebar-collapse | ← | ArrowLeft |
| add | add server/subscription | PlusCircle |
| routing | routing page | World / WorldSegmented |
| settings | settings | Gear |
| stats | statistics | ChartColumnThreeAsc / StatsChart |
| info | about | InformationCircle |
| search | search | Magnifier |
| edit | edit | Pencil |
| more | more menu | DotsHorizontal |
| chevron-down / chevron-right / chevron-up | expanders | ChevronDown / ChevronRight / ChevronUp |
| pin | pin | Pin |
| refresh | update subscription | something like ArrowsCw/Repeat/Update/Refresh — search the list |
| power | connect button | PowerButton |
| ping | latency | Lightning / Speed |
| copy, paste, trash, qr, link, check, close, download, upload, clock, shield, key, lock, sun, moon, globe, filter, sort, list, warning, support, web | — | obvious ones |

   Write `app/flamix/assets/icons/SOURCES.md`: table semantic name -> source component name.
2. `app/flamix/assets/flags/<ISO>.svg` — every flag the package has, file name = ISO 3166-1 alpha-2 upper case
   (`RU.svg`, `DE.svg`, `NL.svg`, ...). Build the English-name -> ISO map yourself in the script (a complete
   alpha-2 table with English short names; normalise names: `UnitedKingdom` -> GB, `UnitedStates` -> US,
   `SouthKorea` -> KR, `Czechia`/`CzechRepublic` -> CZ, ...). Print unmatched names; there should be very few (list them
   in SOURCES.md). Keep colours as is. Also `app/flamix/assets/flags/UN.svg` = a neutral grey globe/flag for unknown.
3. `app/flamix/assets/icons/LICENSE-alfalab.txt` — the MIT license text from the packages (`package/LICENSE` or
   the `license` field + copyright from package.json/README), attribution "Icons: Alfa-Bank core-ds (@alfalab/icons-*), MIT".
4. `app/flamix/assets/app-icon.svg` — app icon: a power-button glyph (white) on a rounded square
   (rx 22%) with a vertical gradient `#C8862E` -> `#7A4A12`, 1024 viewBox. Also render PNGs 16,32,48,64,128,256,512,1024
   into `app/flamix/assets/app-icon/` if `rsvg-convert` or ImageMagick exists on this box (check `which`); if not,
   skip the PNGs and say so in the report.

## Checks
- Every output SVG parses: `node -e` with a tiny check, or `python3 -c "import xml.dom.minidom as m; m.parse(...)"` for all files.
- Spot-check 5 icons by printing their SVG text in your report.
- Total size of `assets/` < 3 MB.

## Done
Commit on `oc/T06` (script + assets). Report: branch, commit hash, icon count, flag count, unmatched flags.
