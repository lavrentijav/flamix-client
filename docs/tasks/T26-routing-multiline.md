# T26 — routing page: multi-line rule values (one entry per line)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
Owner feedback on the "Маршрутизация" page: inside a rule the values must be a **multi-line** input —
enter each domain / IP / process on its own line, and commas typed by the user should be turned into
line breaks automatically. The input must span the full width of the rule card and its text is
left-aligned.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T26 -b oc/T26
    cd /srv/dev/projects/flamix-client-wt/T26

## Files you own
`app/flamix/ui/pages/routing-page.slint`, `app/flamix/src/pages/routing.rs`. Nothing else.

## Do
1. **Model/parse.** In `routing.rs`, make `split_values` split on commas **and** newlines (handle `\r`),
   trim each token, drop empties (keep de-duplication off — order matters). Make `join_values` join with
   `"\n"` (one value per line) instead of `", "`. This is what "commas become line breaks" means:
   whatever the user types, the field is re-rendered as one entry per line on the next refresh, and the
   `Vec<String>` written to the model is identical either way. Update the existing tests
   (`split_and_join_values`, `rule_item_reflects_model`, etc.) and add cases: comma → newline
   normalization, mixed commas+newlines+blank lines, CRLF, and that switching kind still moves values.
2. **Input widget.** In `RuleRow` replace the single-line `LineEdit` for `values` with a `TextEdit`
   (import `TextEdit` from `std-widgets.slint`), ~**64px** tall (about 3–4 lines), `horizontal-stretch: 1`
   so it fills the card width, text left-aligned (default), placeholder showing one entry per line
   (e.g. `"geosite:category-ru\ndomain:example.com\ndomain:example.org"`). Wire `edited(t)` to the same
   `values-changed(root.kind, t)` callback. Debounce is not needed (the page already applies on change).
3. **Row layout.** Keep the action/kind chips row on top (left-aligned). Put the multi-line field on its
   own full-width row; move the up/down/delete `IconButton`s to a compact row (either to the right of, or
   below, the field — your choice, but the field must be full width, not squeezed by the buttons). The
   rule card must not overflow horizontally: no horizontal scrollbar on the page.
4. **No regressions.** The existing rule add/reorder/enable/delete/kind-switch behaviour, validation
   ("Правило N: укажите значения") and Xray value conversion must keep working; empty values still show
   the chosen kind. `set_rule_values` keeps exactly one kind populated.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'mkdir -p shots && cargo run -p flamix -- --demo --screenshot shots/routing.png --page 2 && cargo run -p flamix -- --demo --light --screenshot shots/routing-light.png --page 2'
Regenerate `docs/design/screenshot-routing.png` and `-routing-light.png` (the demo has rules with several
values so the multi-line field is visible). Commit on `oc/T26`. Report: branch, commit, test counts, the
exact parse/join rules. Do not push, do not merge.
