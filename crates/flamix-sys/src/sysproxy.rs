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

/// Windows `ProxyServer` value: `http=host:P;https=host:P;socks=host:S`.
pub fn windows_proxy_server(spec: &ProxySpec) -> String {
    format!(
        "http={host}:{p};https={host}:{p};socks={host}:{s}",
        host = spec.host,
        p = spec.http_port,
        s = spec.socks_port
    )
}

/// Windows `ProxyOverride` value (already Windows syntax, `<local>` is kept).
pub fn windows_bypass(list: &[String]) -> String {
    list.join(";")
}

/// macOS bypass domain list: `<local>` is Windows-only and dropped; `127.*` -> `127.0.0.1`.
pub fn macos_bypass_domains(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|e| !is_local_marker(e))
        .map(|e| match e.as_str() {
            "127.*" => "127.0.0.1".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// GNOME `ignore-hosts` list: drop `<local>`, turn the Windows wildcard ranges into CIDR.
pub fn gnome_ignore_hosts(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|e| !is_local_marker(e))
        .map(|e| match e.as_str() {
            "127.*" => "127.0.0.0/8".to_string(),
            "10.*" => "10.0.0.0/8".to_string(),
            "192.168.*" => "192.168.0.0/16".to_string(),
            "172.16.*" => "172.16.0.0/12".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// KDE `NoProxyFor` list: comma-separated hosts, drop `<local>`, `127.*` -> `127.0.0.1`.
pub fn kde_no_proxy(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|e| !is_local_marker(e))
        .map(|e| match e.as_str() {
            "127.*" => "127.0.0.1".to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn is_local_marker(entry: &str) -> bool {
    entry.eq_ignore_ascii_case("<local>")
}

/// Parse `networksetup -listallnetworkservices`: skip the header (line 1) and disabled
/// services (leading `*`), trim empty lines.
pub fn parse_networksetup_services(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.starts_with("An asterisk"))
        .filter(|l| !l.starts_with('*'))
        .map(str::to_string)
        .collect()
}

/// Set the system proxy.
pub fn set(spec: &ProxySpec) -> crate::Result<()> {
    platform::set(spec)
}

/// Clear the system proxy (only if it still points to us is not required: always clear).
pub fn clear() -> crate::Result<()> {
    platform::clear()
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use windows_sys::Win32::Networking::WinInet::InternetSetOptionW;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    const INTERNET_OPTION_REFRESH: u32 = 37;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn refresh() {
        unsafe {
            // SAFETY: a null session handle and null buffer is the documented way to broadcast
            // the settings change; the call dereferences neither pointer.
            InternetSetOptionW(
                std::ptr::null(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null(),
                0,
            );
            // SAFETY: same as above for the refresh notification.
            InternetSetOptionW(
                std::ptr::null(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null(),
                0,
            );
        }
    }

    pub fn set(spec: &ProxySpec) -> crate::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_SET_VALUE)?;
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &windows_proxy_server(spec))?;
        key.set_value("ProxyOverride", &windows_bypass(&spec.bypass))?;
        refresh();
        Ok(())
    }

    pub fn clear() -> crate::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_SET_VALUE)?;
        key.set_value("ProxyEnable", &0u32)?;
        refresh();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    fn run(args: &[&str]) -> crate::Result<std::process::Output> {
        let out = std::process::Command::new("networksetup")
            .args(args)
            .output()
            .map_err(|e| crate::Error::Command(format!("networksetup: {e}")))?;
        if !out.status.success() {
            return Err(crate::Error::Command(format!(
                "networksetup {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(out)
    }

    fn services() -> crate::Result<Vec<String>> {
        let out = run(&["-listallnetworkservices"])?;
        Ok(parse_networksetup_services(&String::from_utf8_lossy(
            &out.stdout,
        )))
    }

    pub fn set(spec: &ProxySpec) -> crate::Result<()> {
        let services = services()?;
        if services.is_empty() {
            return Err(crate::Error::Command(
                "no enabled network services found".to_string(),
            ));
        }
        let http = spec.http_port.to_string();
        let socks = spec.socks_port.to_string();
        let bypass = macos_bypass_domains(&spec.bypass);
        for svc in &services {
            run(&["-setwebproxy", svc, &spec.host, &http])?;
            run(&["-setsecurewebproxy", svc, &spec.host, &http])?;
            run(&["-setsocksfirewallproxy", svc, &spec.host, &socks])?;
            let mut args: Vec<&str> = vec!["-setproxybypassdomains", svc];
            args.extend(bypass.iter().map(String::as_str));
            run(&args)?;
        }
        Ok(())
    }

    pub fn clear() -> crate::Result<()> {
        for svc in &services()? {
            run(&["-setwebproxystate", svc, "off"])?;
            run(&["-setsecurewebproxystate", svc, "off"])?;
            run(&["-setsocksfirewallproxystate", svc, "off"])?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    enum BackendError {
        Unavailable,
        Failed(String),
    }

    impl std::fmt::Display for BackendError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Unavailable => write!(f, "unavailable"),
                Self::Failed(e) => write!(f, "{e}"),
            }
        }
    }

    fn run(cmd: &str, args: &[&str]) -> std::result::Result<(), BackendError> {
        match std::process::Command::new(cmd).args(args).output() {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(BackendError::Failed(format!(
                "{cmd} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&o.stderr).trim()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(BackendError::Unavailable),
            Err(e) => Err(BackendError::Failed(format!("{cmd}: {e}"))),
        }
    }

    fn command_exists(cmd: &str) -> bool {
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
            .unwrap_or(false)
    }

    fn gnome_set(spec: &ProxySpec) -> std::result::Result<(), BackendError> {
        let host = spec.host.as_str();
        let http = spec.http_port.to_string();
        let socks = spec.socks_port.to_string();
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy", "mode", "manual"],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.http", "host", host],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.http", "port", &http],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.https", "host", host],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.https", "port", &http],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.socks", "host", host],
        )?;
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy.socks", "port", &socks],
        )?;
        let hosts = gnome_variant_array(&gnome_ignore_hosts(&spec.bypass));
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy", "ignore-hosts", &hosts],
        )?;
        Ok(())
    }

    fn gnome_clear() -> std::result::Result<(), BackendError> {
        run(
            "gsettings",
            &["set", "org.gnome.system.proxy", "mode", "none"],
        )
    }

    fn gnome_variant_array(items: &[String]) -> String {
        let quoted: Vec<String> = items
            .iter()
            .map(|s| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")))
            .collect();
        format!("[{}]", quoted.join(", "))
    }

    fn kde_apply(
        setter: impl Fn(&str) -> std::result::Result<(), BackendError>,
    ) -> std::result::Result<(), BackendError> {
        if command_exists("kwriteconfig6") {
            setter("kwriteconfig6")
        } else if command_exists("kwriteconfig5") {
            setter("kwriteconfig5")
        } else {
            Err(BackendError::Unavailable)
        }
    }

    fn kde_reparse() {
        if let Err(BackendError::Failed(msg)) = run(
            "dbus-send",
            &[
                "--type=signal",
                "/KIO/Scheduler",
                "org.kde.KIO.Scheduler.reparseSlaveConfiguration",
                "string:",
            ],
        ) {
            tracing::warn!("KDE proxy reparse failed: {msg}");
        }
    }

    fn kde_set(spec: &ProxySpec) -> std::result::Result<(), BackendError> {
        kde_apply(|tool| {
            let http = format!("http://{} {}", spec.host, spec.http_port);
            let socks = format!("socks://{} {}", spec.host, spec.socks_port);
            let no_proxy = kde_no_proxy(&spec.bypass).join(",");
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "ProxyType",
                    "1",
                ],
            )?;
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "httpProxy",
                    &http,
                ],
            )?;
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "httpsProxy",
                    &http,
                ],
            )?;
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "socksProxy",
                    &socks,
                ],
            )?;
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "NoProxyFor",
                    &no_proxy,
                ],
            )?;
            Ok(())
        })?;
        kde_reparse();
        Ok(())
    }

    fn kde_clear() -> std::result::Result<(), BackendError> {
        kde_apply(|tool| {
            run(
                tool,
                &[
                    "--file",
                    "kioslaverc",
                    "--group",
                    "Proxy Settings",
                    "--key",
                    "ProxyType",
                    "0",
                ],
            )
        })?;
        kde_reparse();
        Ok(())
    }

    fn finish(results: Vec<std::result::Result<(), BackendError>>) -> crate::Result<()> {
        let mut attempted = 0usize;
        let mut ok = 0usize;
        let mut last_error: Option<String> = None;
        for r in results {
            match r {
                Ok(()) => ok += 1,
                Err(BackendError::Unavailable) => {}
                Err(e) => {
                    attempted += 1;
                    last_error = Some(e.to_string());
                }
            }
        }
        if ok > 0 {
            Ok(())
        } else if attempted > 0 {
            Err(crate::Error::Command(last_error.unwrap_or_else(|| {
                "system proxy backend failed".to_string()
            })))
        } else {
            tracing::warn!("no GNOME/KDE proxy backend found; system proxy left unchanged");
            Ok(())
        }
    }

    pub fn set(spec: &ProxySpec) -> crate::Result<()> {
        finish(vec![gnome_set(spec), kde_set(spec)])
    }

    pub fn clear() -> crate::Result<()> {
        finish(vec![gnome_clear(), kde_clear()])
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn set(_spec: &super::ProxySpec) -> crate::Result<()> {
        Err(crate::Error::Unsupported)
    }

    pub fn clear() -> crate::Result<()> {
        Err(crate::Error::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ProxySpec {
        ProxySpec {
            host: "127.0.0.1".to_string(),
            http_port: 10809,
            socks_port: 10808,
            bypass: vec![
                "localhost".to_string(),
                "127.*".to_string(),
                "10.*".to_string(),
                "192.168.*".to_string(),
                "<local>".to_string(),
                "*.local".to_string(),
            ],
        }
    }

    #[test]
    fn windows_proxy_server_string() {
        assert_eq!(
            windows_proxy_server(&spec()),
            "http=127.0.0.1:10809;https=127.0.0.1:10809;socks=127.0.0.1:10808"
        );
    }

    #[test]
    fn windows_bypass_keeps_local_marker() {
        assert_eq!(
            windows_bypass(&spec().bypass),
            "localhost;127.*;10.*;192.168.*;<local>;*.local"
        );
    }

    #[test]
    fn macos_bypass_drops_local() {
        assert_eq!(
            macos_bypass_domains(&spec().bypass),
            vec!["localhost", "127.0.0.1", "10.*", "192.168.*", "*.local"]
        );
    }

    #[test]
    fn gnome_ignore_hosts_converts_ranges() {
        assert_eq!(
            gnome_ignore_hosts(&spec().bypass),
            vec![
                "localhost",
                "127.0.0.0/8",
                "10.0.0.0/8",
                "192.168.0.0/16",
                "*.local"
            ]
        );
    }

    #[test]
    fn kde_no_proxy_drops_local() {
        assert_eq!(
            kde_no_proxy(&spec().bypass),
            vec!["localhost", "127.0.0.1", "10.*", "192.168.*", "*.local"]
        );
    }

    #[test]
    fn networksetup_services_sample() {
        let sample = "An asterisk (*) denotes that a network service is disabled.\n\
                      Wi-Fi\n\
                      *Bluetooth PAN\n\
                      Thunderbolt Bridge\n\n";
        assert_eq!(
            parse_networksetup_services(sample),
            vec!["Wi-Fi", "Thunderbolt Bridge"]
        );
    }
}
