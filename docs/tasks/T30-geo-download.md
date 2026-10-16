# T30 — geo databases: download + local cache (core)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
The owner wants the app to download the geo rule databases at runtime, defaulting to the
"Freedom for Russia" lists (`Settings.geo` already exists; `crates/flamix-core/src/geo.rs` already
resolves the URLs — extend it, do not rewrite it).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T30 -b oc/T30
    cd /srv/dev/projects/flamix-client-wt/T30

## Files you own
`crates/flamix-core/src/geo.rs`. Nothing else.

## Do (add to `geo.rs`)
1. `pub fn file_paths(dir: &Path) -> (PathBuf, PathBuf)` — `<dir>/geosite.dat`, `<dir>/geoip.dat`.
2. `pub fn is_fresh(dir: &Path, max_age_secs: i64) -> bool` — true when both files exist and were
   modified less than `max_age_secs` ago (use file mtime, not a clock we store).
3. `pub async fn download_file(url: &str, dest: &Path) -> flamix_core::Result<()>` — GET the URL
   with `reqwest` (reuse the client setup used by `subscription::fetch`, e.g. a 60 s timeout, no
   redirects to file://; follow normal redirects), stream/write to a temp file in the same dir
   (`<dest>.part`) then atomically `rename` over `dest` so a partial download never replaces a good
   file. Non-2xx → `Error::Http`. Never panic; create the parent dir.
4. `pub struct GeoStatus { pub downloaded: bool, pub dir: PathBuf }` and
   `pub async fn ensure(dir: &Path, geo: &GeoData, max_age_secs: i64) -> flamix_core::Result<GeoStatus>`
   — if `is_fresh(dir, max_age_secs)` return `{downloaded:false, dir}`; otherwise resolve
   `geo::urls(geo)` and download **both** `.dat` files (geosite then geoip), skipping a file whose
   URL is empty; return `{downloaded:true, dir}`. On any download error return the error (the caller
   decides; do not leave a half set — a failed second download still leaves the first good file,
   that is fine, just propagate the error).
5. Unit tests (no network): `file_paths`, `is_fresh` (fresh dir, one missing file, an old mtime via
   `filetime`-free approach — set mtime with `std::fs::File::set_modified`), `ensure` short-circuits
   when fresh (point it at a temp dir with fresh files and URLs that would fail), and empty-URL
   skipping. A real-network download test may be `#[ignore]`d and must hit a `file://`? No — use an
   ignored test against the pinned URL only if you can run it on ru1; otherwise skip.

## Constraints
- No `unwrap`/`expect` on network/file data; return `Result`. Comments in English.
- Do not change the existing `urls()`/`GeoUrls` API or its tests (add alongside).
- `-D warnings` clean.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core
Commit on `oc/T30`. Report: branch, commit, the exact signatures you added, the atomicity approach,
test counts, anything unfinished. Do not push, do not merge.
