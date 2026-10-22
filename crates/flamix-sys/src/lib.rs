//! Flamix OS integration (desktop): system proxy, external core processes, autostart,
//! privilege elevation for TUN. Mobile platforms do not use this crate.

pub mod autostart;
pub mod elevation;
pub mod runner;
pub mod stats;
pub mod sysproxy;
/// Linux transparent-proxy helper (nftables TPROXY + policy routing). Public API lives on
/// every target so the UI layer can read [`transparent::TproxyPlan`] uniformly; actual
/// install/remove runs only on Linux (added in M2 alongside the elevated watchdog).
pub mod transparent;

/// Error type of flamix-sys.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("core binary not found: {0}")]
    CoreNotFound(String),
    #[error("core exited: {0}")]
    CoreExited(String),
    #[error("command failed: {0}")]
    Command(String),
    #[error("not supported on this platform")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, Error>;
