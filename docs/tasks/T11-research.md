# T11 — research: features of Happ, v2RayTun, v2rayN (+ Hiddify, v2rayNG for reference)

Read `AGENTS.md` first. No Rust build. This is a **research** task: the output is documentation that the lead
turns into feature tasks for Flamix, so precision and sources matter more than volume.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T11 -b oc/T11
    cd /srv/dev/projects/flamix-client-wt/T11

## Allowed network access (read-only web fetches / git clone --depth 1 into /srv/dev/tmp/flamix-research/)
- Happ: https://www.happ.su (and its docs/FAQ/dev pages, e.g. subscription headers, routing, deep links), App Store / Google Play pages
- v2RayTun: https://v2raytun.com (docs, FAQ), store pages
- v2rayN: https://github.com/2dust/v2rayN (README, wiki, source: subscription handling, deep links, settings)
- v2rayNG: https://github.com/2dust/v2rayNG ; Hiddify: https://github.com/hiddify/hiddify-app and https://hiddify.com docs
- Remnawave / Marzban docs about client headers (HWID, `subscription-userinfo`, Happ-specific headers): https://docs.rw , https://github.com/remnawave , https://github.com/Gozargah/Marzban
Nothing else; no logins, no installing apps, no POST requests.

## Files you own
`docs/research/**` only.

## Output
1. `docs/research/<app>.md` for happ, v2raytun, v2rayn (and short ones for v2rayng, hiddify): platforms; protocols & transports;
   modes (proxy/TUN/system-proxy/per-app); subscription formats and **every subscription header / body metadata key** the app
   understands (exact names, value formats, examples — e.g. Happ's `profile-title`, `subscription-userinfo`, `announce`,
   `routing`, `hide-settings`, `providerid`, …, whatever is really documented); **deep link / URL schemes** (exact syntax);
   routing (presets, rule formats, geo files); DNS; ping/speed test types; UI features (server list grouping, sorting, search,
   per-server traffic, statistics, widgets, tray, autostart, themes, languages, QR scan/export, backup/restore, auto-update of
   subs and of the app); security features (encrypted links like `happ://crypt…` — describe format only if publicly
   documented). Every claim gets a source URL; mark unverified claims as such.
2. `docs/research/screenshots.md`: links (URLs only — do NOT download or commit third-party images) to official
   screenshots of each app's main screen, server list, settings, routing — with one line describing the UI pattern each shows.
3. `docs/research/feature-matrix.md`: one table, rows = features, columns = Happ, v2RayTun, v2rayN, v2rayNG, Hiddify,
   Flamix (planned per `docs/subscription-headers.md`, `crates/flamix-core/src/model.rs`, `docs/tasks/*.md`). ✅/❌/?.
4. `docs/research/gaps.md`: features the others have that Flamix's current plan lacks, each with: what it is, which app(s),
   user value (high/med/low), rough complexity (S/M/L), and a suggested place in our architecture (core / sys / app / packaging).
   Sorted by value. Also list any subscription headers/deep links we should accept for compatibility but don't yet.

## Done
Commit on `oc/T11`. Report: branch, commit hash, top-10 gaps with one line each.
