//! Running a command with admin/root rights (TUN mode). OWNER: task T05.
//!
//! - Linux: `pkexec` (polkit dialog); if the binary already has `cap_net_admin` run directly.
//! - macOS: `osascript -e 'do shell script "..." with administrator privileges'`.
//! - Windows: if the process is already elevated run directly, otherwise `ShellExecuteW`
//!   with verb "runas".

/// True if the current process already has admin/root rights.
pub fn is_elevated() -> bool {
    todo!("T05")
}
