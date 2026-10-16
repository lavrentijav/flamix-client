//! Geo rule-database download URLs. OWNER: task T29.
//!
//! Resolves [`GeoData`] (which source to use, plus the custom URLs) to the concrete download
//! URLs: the Xray `geosite.dat`/`geoip.dat` files and the sing-box `.srs` rule-set **bases**.
//! [`urls`] itself is pure — no network, no filesystem. This module also provides the runtime
//! download/cache helpers ([`download_file`], [`is_fresh`], [`ensure`]) that fetch the `.dat`
//! files into a local directory.
//!
//! ## Pinned versions
//!
//! The `.dat` assets are pinned by release tag; the sing-box `.srs` bases are rolling branches
//! (upstream publishes `.srs` only on a branch / inside a per-tag zip), so the Xray `.dat` and
//! the sing-box `.srs` can drift over time — accepted. `scripts/fetch-cores.sh` bundles its own
//! `.dat` for the shipped packages and is a separate concern (not kept in sync with this file).
//!
//! - "Freedom for Russia" (`runetfreedom/russia-v2ray-rules-dat`) — the default source, the
//!   RU-focused lists: Xray `.dat` assets from release tag **202610161855**.
//! - Loyalsoldier (`Loyalsoldier/v2ray-rules-dat`): Xray `.dat` assets from release tag
//!   **202610152206**.
//! - sing-box `.srs` rule-sets: SagerNet `sing-geosite`/`sing-geoip` publish only `.db`
//!   release assets, not the `.srs` files, so their rolling `rule-set` branch is used (this is
//!   also the branch the old hardcoded constants used). `runetfreedom` does publish `.srs`,
//!   but only inside a single `sing-box.zip` release asset / on its rolling `release` branch,
//!   so those bases are not tag-pinnable either.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::model::{GeoData, GeoSource};

/// Xray `geosite.dat` — "Freedom for Russia" lists, release tag 202610161855.
const FFR_GEOSITE_DAT: &str =
    "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/download/202610161855/geosite.dat";
/// Xray `geoip.dat` — "Freedom for Russia" lists, release tag 202610161855.
const FFR_GEOIP_DAT: &str =
    "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/download/202610161855/geoip.dat";
/// sing-box geosite `.srs` base — `runetfreedom` publishes these only on the rolling
/// `release` branch (`.../sing-box/rule-set-geosite/geosite-<name>.srs`), not per tag.
const FFR_GEOSITE_SRS_BASE: &str =
    "https://raw.githubusercontent.com/runetfreedom/russia-v2ray-rules-dat/release/sing-box/rule-set-geosite";
/// sing-box geoip `.srs` base — same rolling `release` branch
/// (`.../sing-box/rule-set-geoip/geoip-<name>.srs`).
const FFR_GEOIP_SRS_BASE: &str =
    "https://raw.githubusercontent.com/runetfreedom/russia-v2ray-rules-dat/release/sing-box/rule-set-geoip";

/// Xray `geosite.dat` — Loyalsoldier, release tag 202610152206.
const LOYALSOLDIER_GEOSITE_DAT: &str =
    "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/download/202610152206/geosite.dat";
/// Xray `geoip.dat` — Loyalsoldier, release tag 202610152206.
const LOYALSOLDIER_GEOIP_DAT: &str =
    "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/download/202610152206/geoip.dat";

/// sing-box geosite `.srs` base — SagerNet `sing-geosite`, rolling `rule-set` branch
/// (`geosite-<name>.srs`). Used by every source that does not ship its own `.srs` files.
const SAGERNET_GEOSITE_SRS_BASE: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set";
/// sing-box geoip `.srs` base — SagerNet `sing-geoip`, rolling `rule-set` branch
/// (`geoip-<name>.srs`).
const SAGERNET_GEOIP_SRS_BASE: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set";

/// Concrete download URLs for a [`GeoData`] setting.
///
/// `xray_geosite`/`xray_geoip` are full file URLs consumed by Xray; the sing-box rule-sets are
/// addressed by appending `geosite-<name>.srs` / `geoip-<name>.srs` to the two bases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoUrls {
    pub xray_geosite: String,
    pub xray_geoip: String,
    pub singbox_geosite_base: String,
    pub singbox_geoip_base: String,
}

/// Resolve the download URLs for `geo`.
///
/// - [`GeoSource::FreedomForRussia`] (default): the "Freedom for Russia" RU lists — `.dat`
///   from the pinned release tag, sing-box `.srs` from the project's own rolling
///   `release` branch.
/// - [`GeoSource::Loyalsoldier`]: Loyalsoldier `.dat` assets; sing-box `.srs` from SagerNet
///   (Loyalsoldier does not publish `.srs`).
/// - [`GeoSource::Custom`]: the user's `geosite_url`/`geoip_url` for Xray; sing-box cannot turn
///   a custom `.dat` into `.srs`, so it falls back to the SagerNet bases with a warning.
pub fn urls(geo: &GeoData) -> GeoUrls {
    match geo.source {
        GeoSource::FreedomForRussia => GeoUrls {
            xray_geosite: FFR_GEOSITE_DAT.to_string(),
            xray_geoip: FFR_GEOIP_DAT.to_string(),
            singbox_geosite_base: FFR_GEOSITE_SRS_BASE.to_string(),
            singbox_geoip_base: FFR_GEOIP_SRS_BASE.to_string(),
        },
        GeoSource::Loyalsoldier => GeoUrls {
            xray_geosite: LOYALSOLDIER_GEOSITE_DAT.to_string(),
            xray_geoip: LOYALSOLDIER_GEOIP_DAT.to_string(),
            singbox_geosite_base: SAGERNET_GEOSITE_SRS_BASE.to_string(),
            singbox_geoip_base: SAGERNET_GEOIP_SRS_BASE.to_string(),
        },
        GeoSource::Custom => {
            tracing::warn!(
                "geo source is custom: sing-box cannot convert a custom .dat to .srs; \
                 using the SagerNet rule-set bases for sing-box"
            );
            GeoUrls {
                xray_geosite: geo.geosite_url.clone(),
                xray_geoip: geo.geoip_url.clone(),
                singbox_geosite_base: SAGERNET_GEOSITE_SRS_BASE.to_string(),
                singbox_geoip_base: SAGERNET_GEOIP_SRS_BASE.to_string(),
            }
        }
    }
}

/// Timeout for a single geo database download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Local paths of the two Xray databases inside `dir`: `(geosite.dat, geoip.dat)`.
pub fn file_paths(dir: &Path) -> (PathBuf, PathBuf) {
    (dir.join("geosite.dat"), dir.join("geoip.dat"))
}

/// Age of a file's modification time in seconds, or `None` when it cannot be read.
///
/// A modification time in the future (clock skew) is treated as age zero, i.e. brand new.
fn mtime_age_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    match modified.elapsed() {
        Ok(age) => Some(age.as_secs() as i64),
        Err(_) => Some(0),
    }
}

/// `true` when both databases exist and were modified less than `max_age_secs` ago.
///
/// Uses the file mtime — no timestamp is stored separately.
pub fn is_fresh(dir: &Path, max_age_secs: i64) -> bool {
    let (geosite, geoip) = file_paths(dir);
    let fresh = |path: &Path| matches!(mtime_age_secs(path), Some(age) if age < max_age_secs);
    fresh(&geosite) && fresh(&geoip)
}

/// `<dest>.part` — the temporary file a download is written to before the atomic rename.
fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// A client for database downloads: a 60 s timeout and normal redirects, but no redirects to
/// non-http(s) schemes (e.g. `file://`).
fn download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if matches!(attempt.url().scheme(), "http" | "https") {
                attempt.follow()
            } else {
                attempt.error("redirect to a non-http(s) scheme is not allowed")
            }
        }))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}

/// Write the response body to `part`, streaming chunk by chunk.
async fn write_response(mut response: reqwest::Response, part: &Path) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(part).await?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Error::Http(e.to_string()))?
    {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

/// Download `url` to `dest`, creating the parent directory if needed.
///
/// The body is streamed to `<dest>.part` and then atomically renamed over `dest`, so a partial
/// or failed download never replaces an existing good file. A non-2xx response is
/// [`Error::Http`]; the temporary file is removed on any failure.
pub async fn download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let response = download_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Http(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Http(format!("HTTP {}", status.as_u16())));
    }

    let part = part_path(dest);
    if let Err(err) = write_response(response, &part).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(err);
    }
    if let Err(err) = tokio::fs::rename(&part, dest).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(err.into());
    }
    Ok(())
}

/// Outcome of [`ensure`]: whether files were (re)downloaded and the target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoStatus {
    pub downloaded: bool,
    pub dir: PathBuf,
}

/// Ensure the Xray databases in `dir` are present and not older than `max_age_secs`.
///
/// When [`is_fresh`] already holds this is a no-op (`downloaded: false`). Otherwise the resolved
/// [`urls`] are fetched — geosite first, then geoip — skipping an empty URL. On a download error
/// the error is propagated; a previously downloaded good file is left in place.
pub async fn ensure(dir: &Path, geo: &GeoData, max_age_secs: i64) -> Result<GeoStatus> {
    if is_fresh(dir, max_age_secs) {
        return Ok(GeoStatus {
            downloaded: false,
            dir: dir.to_path_buf(),
        });
    }

    let urls = urls(geo);
    let (geosite_path, geoip_path) = file_paths(dir);
    for (url, dest) in [
        (&urls.xray_geosite, &geosite_path),
        (&urls.xray_geoip, &geoip_path),
    ] {
        if url.is_empty() {
            continue;
        }
        download_file(url, dest).await?;
    }

    Ok(GeoStatus {
        downloaded: true,
        dir: dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(geosite: &str, geoip: &str) -> GeoData {
        GeoData {
            source: GeoSource::Custom,
            geosite_url: geosite.into(),
            geoip_url: geoip.into(),
        }
    }

    #[test]
    fn default_source_is_freedom_for_russia() {
        assert_eq!(GeoData::default().source, GeoSource::FreedomForRussia);
        // ...and its URLs resolve to the runetfreedom release/branch.
        let urls = urls(&GeoData::default());
        assert!(urls
            .xray_geosite
            .contains("runetfreedom/russia-v2ray-rules-dat"));
        assert!(urls.xray_geosite.ends_with("202610161855/geosite.dat"));
        assert!(urls.xray_geoip.ends_with("202610161855/geoip.dat"));
        assert!(urls.singbox_geosite_base.contains("russia-v2ray-rules-dat"));
        assert!(urls
            .singbox_geosite_base
            .ends_with("sing-box/rule-set-geosite"));
        assert!(urls.singbox_geoip_base.ends_with("sing-box/rule-set-geoip"));
    }

    #[test]
    fn freedom_for_russia_urls_are_pinned_and_distinct() {
        let urls = urls(&GeoData {
            source: GeoSource::FreedomForRussia,
            ..GeoData::default()
        });
        assert_eq!(
            urls.xray_geosite,
            "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/download/202610161855/geosite.dat"
        );
        assert_eq!(
            urls.xray_geoip,
            "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/download/202610161855/geoip.dat"
        );
        assert_eq!(
            urls.singbox_geosite_base,
            "https://raw.githubusercontent.com/runetfreedom/russia-v2ray-rules-dat/release/sing-box/rule-set-geosite"
        );
        assert_eq!(
            urls.singbox_geoip_base,
            "https://raw.githubusercontent.com/runetfreedom/russia-v2ray-rules-dat/release/sing-box/rule-set-geoip"
        );
    }

    #[test]
    fn loyalsoldier_uses_dat_assets_and_sagernet_srs() {
        let urls = urls(&GeoData {
            source: GeoSource::Loyalsoldier,
            ..GeoData::default()
        });
        assert_eq!(
            urls.xray_geosite,
            "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/download/202610152206/geosite.dat"
        );
        assert_eq!(
            urls.xray_geoip,
            "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/download/202610152206/geoip.dat"
        );
        // Loyalsoldier publishes no `.srs`: the sing-box side falls back to SagerNet.
        assert!(urls.singbox_geosite_base.contains("SagerNet/sing-geosite"));
        assert!(urls.singbox_geoip_base.contains("SagerNet/sing-geoip"));
    }

    #[test]
    fn custom_urls_pass_through_with_sagernet_srs_fallback() {
        let urls = urls(&custom(
            "https://cdn.example/geosite.dat",
            "https://cdn.example/geoip.dat",
        ));
        assert_eq!(urls.xray_geosite, "https://cdn.example/geosite.dat");
        assert_eq!(urls.xray_geoip, "https://cdn.example/geoip.dat");
        // Custom `.dat` cannot become `.srs`; sing-box uses SagerNet.
        assert!(urls.singbox_geosite_base.contains("SagerNet/sing-geosite"));
        assert!(urls.singbox_geoip_base.contains("SagerNet/sing-geoip"));
    }

    #[test]
    fn custom_empty_urls_stay_empty_for_xray() {
        let urls = urls(&custom("", ""));
        assert!(urls.xray_geosite.is_empty());
        assert!(urls.xray_geoip.is_empty());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("flamix-geo-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, body: &[u8]) {
        std::fs::write(path, body).unwrap();
    }

    fn set_mtime(path: &Path, age_secs: u64) {
        let old = std::time::SystemTime::now() - Duration::from_secs(age_secs);
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(old).unwrap();
    }

    #[test]
    fn file_paths_are_named_inside_dir() {
        let dir = Path::new("/tmp/flamix-geo-paths");
        let (geosite, geoip) = file_paths(dir);
        assert_eq!(geosite, dir.join("geosite.dat"));
        assert_eq!(geoip, dir.join("geoip.dat"));
    }

    #[test]
    fn is_fresh_true_for_recent_files() {
        let dir = temp_dir("fresh");
        let (geosite, geoip) = file_paths(&dir);
        touch(&geosite, b"site");
        touch(&geoip, b"ip");
        assert!(is_fresh(&dir, 3600));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_fresh_false_when_one_file_missing() {
        let dir = temp_dir("missing");
        let (geosite, _geoip) = file_paths(&dir);
        touch(&geosite, b"site");
        assert!(!is_fresh(&dir, 3600));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_fresh_false_for_old_files() {
        let dir = temp_dir("old");
        let (geosite, geoip) = file_paths(&dir);
        touch(&geosite, b"site");
        touch(&geoip, b"ip");
        set_mtime(&geosite, 10_000);
        set_mtime(&geoip, 10_000);
        assert!(!is_fresh(&dir, 60));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn ensure_short_circuits_when_fresh() {
        let dir = temp_dir("ensure-fresh");
        let (geosite, geoip) = file_paths(&dir);
        touch(&geosite, b"site");
        touch(&geoip, b"ip");
        // URLs that would fail if `ensure` actually tried to download them.
        let geo = custom(
            "http://127.0.0.1:1/geosite.dat",
            "http://127.0.0.1:1/geoip.dat",
        );
        let status = ensure(&dir, &geo, 3600).await.unwrap();
        assert!(!status.downloaded);
        assert_eq!(status.dir, dir);
        std::fs::remove_dir_all(&status.dir).ok();
    }

    #[tokio::test]
    async fn ensure_skips_empty_urls() {
        let dir = temp_dir("ensure-empty");
        let geo = custom("", "");
        let status = ensure(&dir, &geo, 3600).await.unwrap();
        assert!(status.downloaded);
        let (geosite, geoip) = file_paths(&status.dir);
        assert!(!geosite.exists());
        assert!(!geoip.exists());
        std::fs::remove_dir_all(&status.dir).ok();
    }

    #[tokio::test]
    #[ignore = "requires network access to GitHub"]
    async fn download_file_downloads_geosite() {
        let dir = temp_dir("net");
        let (geosite, _geoip) = file_paths(&dir);
        let url = urls(&GeoData::default()).xray_geosite;
        download_file(&url, &geosite).await.unwrap();
        assert!(std::fs::metadata(&geosite).unwrap().len() > 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
