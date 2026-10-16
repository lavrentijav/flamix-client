//! Fallback backend for platforms without a supported tray (never used on the three targets).

use super::{TrayCommand, TraySink, TrayState};
use std::sync::Arc;

pub fn create(
    _handler: Arc<dyn Fn(TrayCommand) + Send + Sync>,
    _initial: &TrayState,
    _rt: Option<tokio::runtime::Handle>,
) -> Option<Box<dyn TraySink>> {
    None
}
