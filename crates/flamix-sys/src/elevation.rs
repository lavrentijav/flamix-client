//! Running a command with admin/root rights (TUN mode). OWNER: task T05.
//!
//! - Linux: `pkexec` (polkit dialog); if the binary already has `cap_net_admin` run directly.
//! - macOS: `osascript -e 'do shell script "..." with administrator privileges'`.
//! - Windows: if the process is already elevated run directly, otherwise `ShellExecuteW`
//!   with verb "runas".

/// True if the current process already has admin/root rights.
pub fn is_elevated() -> bool {
    #[cfg(target_os = "linux")]
    {
        parse_proc_status_uid()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        // SAFETY: IsUserAnAdmin takes no arguments, dereferences nothing and only inspects the
        // current process token.
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

/// Linux: read the effective uid from `/proc/self/status` without linking libc.
#[cfg(target_os = "linux")]
fn parse_proc_status_uid() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            // `Uid: <real> <effective> <saved> <fs>`
            return rest
                .split_whitespace()
                .nth(1)
                .map(|uid| uid == "0")
                .unwrap_or(false);
        }
    }
    false
}

/// Re-launch this executable with admin rights (Windows only). `args` are passed through.
///
/// On Windows a UAC prompt is shown via `ShellExecuteW` verb "runas". Other platforms return
/// [`crate::Error::Unsupported`].
pub fn restart_elevated(args: &[String]) -> crate::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::restart_elevated(args)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = args;
        Err(crate::Error::Unsupported)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn restart_elevated(args: &[String]) -> crate::Result<()> {
        let exe = std::env::current_exe()?;
        let exe = exe.to_string_lossy();
        let params = args
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(" ");
        let file = wide(&exe);
        let params = wide(&params);
        let verb = wide("runas");
        // SAFETY: all pointers reference null-terminated UTF-16 buffers that stay alive for the
        // duration of the call; `nShowCmd` is SW_SHOWNORMAL. ShellExecuteW copies the strings and
        // does not retain them.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                1,
            )
        };
        if (result as usize) <= 32 {
            return Err(crate::Error::Command(
                "failed to relaunch with administrator rights".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_elevated_does_not_panic() {
        let _ = is_elevated();
    }
}
