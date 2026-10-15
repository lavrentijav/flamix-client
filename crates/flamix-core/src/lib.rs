//! Flamix core: platform-independent logic of the Flamix VPN client.
//!
//! - [`model`]: data types (the contract between modules)
//! - [`link`]: share links (`vless://`, `vmess://`, ...) <-> [`model::Server`]
//! - [`subscription`]: fetching/parsing subscriptions, headers, backup URLs
//! - [`config`]: generating Xray / sing-box configs
//! - [`store`]: persisting [`model::AppState`]
//! - [`ping`]: latency tests
//! - [`country`]: flag emoji <-> ISO country code
//! - [`deeplink`]: `flamix://`, `happ://`, `v2raytun://` import links
#![forbid(unsafe_code)]

pub mod config;
pub mod country;
pub mod deeplink;
pub mod error;
pub mod link;
pub mod model;
pub mod ping;
pub mod store;
pub mod subscription;

pub use error::{Error, Result};
