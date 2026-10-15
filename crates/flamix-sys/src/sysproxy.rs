//! OS system proxy. OWNER: task T05.
//!
//! - Windows: registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
//!   (ProxyEnable, ProxyServer, ProxyOverride) + `InternetSetOptionW` refresh.
//! - macOS: `networksetup -set{web,secureweb,socksfirewall}proxy` for every enabled service.
//! - Linux: GNOME `gsettings org.gnome.system.proxy`, KDE `kwriteconfig5/6` (kioslaverc) +
//!   `dbus-send` reparse; whichever tools exist. Unknown DE = Ok(()) with a warning.

/// Proxy endpoints to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySpec {
    pub host: String,
    pub http_port: u16,
    pub socks_port: u16,
    /// Bypass list in Windows syntax ("localhost;127.*;<local>"), converted per OS.
    pub bypass: Vec<String>,
}

/// Set the system proxy.
pub fn set(spec: &ProxySpec) -> crate::Result<()> {
    let _ = spec;
    todo!("T05")
}

/// Clear the system proxy (only if it still points to us is not required: always clear).
pub fn clear() -> crate::Result<()> {
    todo!("T05")
}
