//! Geo rule-database download URLs. OWNER: task T29.
//!
//! Resolves [`GeoData`] (which source to use, plus the custom URLs) to the concrete download
//! URLs: the Xray `geosite.dat`/`geoip.dat` files and the sing-box `.srs` rule-set **bases**.
//! This is pure — no network, no filesystem; the app performs the actual downloads.
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
}
