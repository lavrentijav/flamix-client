# T28 — routing page: separate value fields with list editing + collapsible sections

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
Owner feedback on "Маршрутизация": inside a rule the values must be **three separate, titled fields**
(Домены / IP / Процессы) instead of one multi-line box, and each field is a **list of entries** edited
like a token list. Also the page sections must be **collapsible**.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T28 -b oc/T28
    cd /srv/dev/projects/flamix-client-wt/T28

## Files you own
`app/flamix/ui/pages/routing-page.slint`, `app/flamix/src/pages/routing.rs`. Nothing else (no `model.rs`,
no other pages, no controller).

## A. Separate titled value fields (replaces the single multi-line box and the kind chips)
The model already stores three lists per rule: `RoutingRule.domains`, `.ips`, `.processes`
(`Vec<String>`). Drop the "kind" concept from the UI: instead of action+kind chips + one field, show
**three labelled list fields** inside each rule card, each editing its own list:
- **Домены** → `domains` (Xray syntax: `geosite:category-ru`, `domain:example.com`, `full:x.y`, `regexp:…`)
- **IP / подсети** → `ips` (`geoip:ru`, `10.0.0.0/8`, `geoip:private`)
- **Процессы** → `processes` (`telegram.exe`), hint that it is TUN-only.
So `RoutingRuleItem` in the `.slint` changes from `{index, action, kind, values, enabled}` to
`{index, action, enabled, collapsed, domains: [string], ips: [string], processes: [string]}` (keep the
`for … : RuleRow` model). The action chips (Прокси/Напрямую/Блок) and the enable Switch stay.

## B. List-entry editing (per value row)
Each of the three fields is a vertical list of **one-line `LineEdit` rows**, one entry per value, plus a
trailing empty row. Behaviour:
- **Enter** in a row → insert a new empty row right below it and move focus there.
- **Backspace on an empty row** → delete that row and move focus to the row above (if any); on the first
  row, just clear/keep it.
- Typing in the last (empty) row keeps it as the input row (a fresh empty row is appended automatically
  after the last non-empty entry so the user always has a place to add).
- The list sent to Rust is the ordered list of **non-empty, trimmed** entries.
Implement the list mutation as **pure helpers in `routing.rs`** (`list_insert`, `list_remove`, empty-row
normalization) with unit tests; drive them from bridge callbacks. Focus handling is the tricky part:
expose from Rust which `(rule-index, field, row)` should get focus after a structural edit (e.g.
`in-out property <int> focus-rule`, `focus-field`, `focus-row`, and a `focus-token` that changes on each
request) and have each `LineEdit` call `self.focus()` when its coordinates match the token. If a simpler
robust Slint-native approach exists, use it and explain it in the report.

## C. Collapsible page sections
Make each top-level section collapsible with a chevron header (click toggles):
1. **Пресеты** (the three preset cards),
2. **Свои правила** (the rules list + add button),
3. **Дополнительно** (domain strategy, block QUIC),
4. **Маршрутизация провайдера**.
Collapse state is UI-local (`in-out property <bool> …-collapsed` on `RoutingBridge`, no persistence
needed). When collapsed, show a one-line summary (e.g. "Свои правила · 4"). Keep the existing card style.

## Constraints
- Keep all existing behaviour: preset selection, provider toggles, validation ("Правило N: укажите
  значения" — now "хотя бы одно значение"), add/reorder/enable/delete rule, domain strategy, block QUIC,
  debounced reconnect.
- The Rust↔config path is unchanged: rules still produce `RoutingRule {domains, ips, processes, action}`
  exactly as today (remove the single-kind `set_rule_values`/`change_rule_kind`/`kind_index` machinery and
  the `KindCache`, replacing them with straightforward per-list editing).
- No `unwrap`/`expect` on user data; comments in English; `-D warnings` clean.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'mkdir -p shots && cargo run -p flamix -- --demo --screenshot shots/routing.png --page 2 && cargo run -p flamix -- --demo --light --screenshot shots/routing-light.png --page 2'
Regenerate `docs/design/screenshot-routing.png` / `-routing-light.png` (demo must fill a couple of rules
with several domains/IPs/processes so the three fields are visible). Commit on `oc/T28`. Report: branch,
commit, test counts, how focus works, anything unfinished. Do not push, do not merge.
