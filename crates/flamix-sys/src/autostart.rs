//! Launch at login. OWNER: task T05.
//!
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value "Flamix".
//! - macOS: `~/Library/LaunchAgents/online.flamix.Flamix.plist`.
//! - Linux: `~/.config/autostart/flamix.desktop`.

#[cfg(target_os = "windows")]
const VALUE_NAME: &str = "Flamix";
#[cfg(target_os = "macos")]
const PLIST_NAME: &str = "online.flamix.Flamix.plist";
#[cfg(target_os = "linux")]
const DESKTOP_NAME: &str = "flamix.desktop";

/// The executable path plus `--minimized`, as used by the Windows and Linux backends.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn command() -> crate::Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" --minimized", exe.display()))
}

/// Enable/disable launch at login for the current executable (started with `--minimized`).
pub fn set_enabled(enabled: bool) -> crate::Result<()> {
    platform::set_enabled(enabled)
}

pub fn is_enabled() -> bool {
    platform::is_enabled()
}

#[cfg(target_os = "windows")]
mod platform {
    use std::io::ErrorKind;

    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    use super::{command, VALUE_NAME};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn set_enabled(enabled: bool) -> crate::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
        if enabled {
            key.set_value(VALUE_NAME, &command()?)?;
        } else {
            match key.delete_value(VALUE_NAME) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(RUN_KEY) else {
            return false;
        };
        let value: std::io::Result<String> = key.get_value(VALUE_NAME);
        value.is_ok()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    use super::PLIST_NAME;

    fn plist_path() -> crate::Result<PathBuf> {
        let home = directories::BaseDirs::new()
            .ok_or_else(|| crate::Error::Command("cannot determine home directory".to_string()))?;
        Ok(home
            .home_dir()
            .join("Library/LaunchAgents")
            .join(PLIST_NAME))
    }

    fn plist_body() -> crate::Result<String> {
        let exe = std::env::current_exe()?;
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>online.flamix.Flamix</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{}</string>\n\
             \t\t<string>--minimized</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             </dict>\n\
             </plist>\n",
            exe.display()
        ))
    }

    pub fn set_enabled(enabled: bool) -> crate::Result<()> {
        let path = plist_path()?;
        if enabled {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, plist_body()?)?;
        } else if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        plist_path().map(|p| p.is_file()).unwrap_or(false)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    use super::DESKTOP_NAME;

    fn desktop_path() -> crate::Result<PathBuf> {
        let home = directories::BaseDirs::new().ok_or_else(|| {
            crate::Error::Command("cannot determine config directory".to_string())
        })?;
        Ok(home.config_dir().join("autostart").join(DESKTOP_NAME))
    }

    fn desktop_body() -> crate::Result<String> {
        Ok(format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Flamix\n\
             Comment=Flamix VPN client\n\
             Exec={}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            super::command()?
        ))
    }

    pub fn set_enabled(enabled: bool) -> crate::Result<()> {
        let path = desktop_path()?;
        if enabled {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, desktop_body()?)?;
        } else if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        desktop_path().map(|p| p.is_file()).unwrap_or(false)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn set_enabled(_enabled: bool) -> crate::Result<()> {
        Err(crate::Error::Unsupported)
    }

    pub fn is_enabled() -> bool {
        false
    }
}
