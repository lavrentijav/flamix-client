//! Launch at login. OWNER: task T05.
//!
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value "Flamix".
//! - macOS: `~/Library/LaunchAgents/online.flamix.Flamix.plist`.
//! - Linux: `~/.config/autostart/flamix.desktop`.

/// Enable/disable launch at login for the current executable (started with `--minimized`).
pub fn set_enabled(enabled: bool) -> crate::Result<()> {
    let _ = enabled;
    todo!("T05")
}

pub fn is_enabled() -> bool {
    todo!("T05")
}
