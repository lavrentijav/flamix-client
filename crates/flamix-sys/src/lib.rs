//! Flamix OS integration (desktop): system proxy, external core processes, autostart,
//! privilege elevation for TUN. Mobile platforms do not use this crate.

pub mod autostart;
pub mod elevation;
pub mod runner;
pub mod sysproxy;

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
