//! Flamix core: platform-independent logic of the Flamix VPN client.
//!
//! - [`model`]: data types (the contract between modules)
//! - [`link`]: share links (`vless://`, `vmess://`, ...) <-> [`model::Server`]
//! - [`subscription`]: fetching/parsing subscriptions, headers, backup URLs
//! - [`config`]: generating Xray / sing-box configs
//! - [`geo`]: geo rule-database source -> download URLs
//! - [`store`]: persisting [`model::AppState`]
//! - [`ping`]: latency tests
//! - [`country`]: flag emoji <-> ISO country code
//! - [`deeplink`]: `flamix://`, `happ://`, `v2raytun://` import links
#![forbid(unsafe_code)]

pub mod backup;
pub mod config;
pub mod country;
pub mod deeplink;
pub mod error;
pub mod geo;
pub mod link;
pub mod model;
pub mod ping;
pub mod provider_routing;
pub mod store;
pub mod subscription;

pub use error::{Error, Result};
