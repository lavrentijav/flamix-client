# T27 — About page: fix the container length / scrollbars

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
Owner feedback on the "О программе" page: the container length is wrong — content "goes under" and
scrollbars appear (a horizontal bar at the bottom is overlapping the content). Make the page fit
cleanly: vertical scrolling only, content never hidden under a scrollbar, and compact enough to mostly
fit the default window.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T27 -b oc/T27
    cd /srv/dev/projects/flamix-client-wt/T27

## Files you own
`app/flamix/ui/pages/about-page.slint`. Nothing else.

## Diagnosis to act on
The `ScrollView` sizes its content to the content's **preferred width**. Long `Text` items with
`wrap: word-wrap` that have no width constraint (the header sentence, the backup description) and the
fixed card `height: X.preferred-height` pattern can make the content wider than the viewport, so a
horizontal scrollbar appears at the bottom and overlaps/cuts the last card.

## Do
1. **No horizontal overflow.** Force the inner content to the viewport width and turn off horizontal
   scrolling:
   - import `ScrollBarPolicy` from `std-widgets.slint` and set the `ScrollView`'s
     `horizontal-scrollbar-policy: ScrollBarPolicy.always-off;`
   - constrain the inner content: set the `ScrollView`'s child `VerticalLayout` to the viewport width
     (e.g. `width: scroll.content-width;` using the ScrollView `id`), and/or give every wrapping `Text`
     `horizontal-stretch: 1` and `wrap: word-wrap` so nothing claims its unwrapped width.
   - Verify there is exactly one vertical scrollbar and no horizontal one.
2. **No content under the scrollbar.** Add bottom padding inside the scroll content (e.g. widen the
   trailing spacer to ~12px) so the last card's bottom edge is fully visible above the vertical
   scrollbar.
3. **Compact layout.** Reduce vertical footprint where cheap (spacing, the header icon size or padding)
   so the page fits a 960×600 window without scrolling when possible; it may still scroll on small
   windows. Do not remove any content (cores, licences, backup).
4. Keep the card style (radius/stroke), Russian labels, and all existing bridge wiring/callbacks
   (export/import/confirm/refresh-cores/links) unchanged.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'mkdir -p shots && cargo run -p flamix -- --demo --screenshot shots/about.png --page 5 && cargo run -p flamix -- --demo --light --screenshot shots/about-light.png --page 5'
Regenerate `docs/design/screenshot-about.png` and `-about-light.png`. In the report state exactly what
caused the horizontal overflow and how you verified the fix (height math / policy). Commit on `oc/T27`.
Do not push, do not merge.
