//! Page "О программе" (task T17A). Talks to the page only through the `AboutBridge` global.
//!
//! Core versions are probed on a background task by running `<core> version` (see
//! `flamix_sys::runner::find_core`); "не найдено" is shown when the binary is absent.

use crate::controller::Controller;
use crate::{AboutBridge, AppWindow};
use flamix_core::config::CoreKind;
use flamix_sys::runner;
use slint::{ComponentHandle, Weak};
use std::sync::Arc;

/// Project links opened by the buttons.
const GITHUB: &str = "https://github.com/lavrentijav/flamix-client";
const RELEASES: &str = "https://github.com/lavrentijav/flamix-client/releases";
const ISSUES: &str = "https://github.com/lavrentijav/flamix-client/issues";

/// Core versions shown in `--demo` / screenshot mode.
const DEMO_XRAY: &str = "25.3.6";
const DEMO_SINGBOX: &str = "1.11.5";

pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    let bridge = ui.global::<AboutBridge>();
    bridge.set_version(env!("CARGO_PKG_VERSION").into());

    {
        ui.global::<AboutBridge>()
            .on_open_github(|| open_link(GITHUB));
    }
    {
        ui.global::<AboutBridge>()
            .on_open_releases(|| open_link(RELEASES));
    }
    {
        ui.global::<AboutBridge>()
            .on_open_issues(|| open_link(ISSUES));
    }
    {
        let weak = ui.as_weak();
        let ctrl = ctrl.cloned();
        ui.global::<AboutBridge>()
            .on_refresh_cores(move || detect_cores(weak.clone(), ctrl.clone()));
    }

    match ctrl {
        Some(ctrl) => {
            bridge.set_xray_version("проверка…".into());
            bridge.set_singbox_version("проверка…".into());
            detect_cores(ui.as_weak(), Some(Arc::clone(ctrl)));
        }
        None => {
            bridge.set_xray_version(DEMO_XRAY.into());
            bridge.set_singbox_version(DEMO_SINGBOX.into());
        }
    }
}

/// Probe both cores off the UI thread and publish the labels back.
fn detect_cores(ui: Weak<AppWindow>, ctrl: Option<Arc<Controller>>) {
    let Some(ctrl) = ctrl else {
        // Demo mode: the values are already filled by `init`; nothing to do.
        return;
    };
    let ui_for_task = ui.clone();
    ctrl.runtime().spawn_blocking(move || {
        let xray = core_label(CoreKind::Xray);
        let singbox = core_label(CoreKind::SingBox);
        let _ = ui_for_task.upgrade_in_event_loop(move |ui| {
            let bridge = ui.global::<AboutBridge>();
            bridge.set_xray_version(xray.into());
            bridge.set_singbox_version(singbox.into());
        });
    });
}

/// "X.Y.Z" or "не найдено" for one core.
fn core_label(kind: CoreKind) -> String {
    match core_version(kind) {
        Some(version) => version,
        None => "не найдено".to_string(),
    }
}

/// Run `<core> version` and parse the version token.
fn core_version(kind: CoreKind) -> Option<String> {
    let binary = runner::find_core(kind).ok()?;
    let output = std::process::Command::new(binary)
        .arg("version")
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    parse_version(&text)
}

/// First version-like token on the first non-empty line ("Xray 25.3.6 …" -> "25.3.6").
fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    line.split_whitespace()
        .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|token| {
            token
                .trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
                .to_string()
        })
        .filter(|version| !version.is_empty())
}

/// Open `url` in the default browser without blocking the UI.
fn open_link(url: &str) {
    let (program, args) = open_command(url);
    if let Err(err) = std::process::Command::new(program).args(args).spawn() {
        tracing::warn!(%err, url, "could not open the link in a browser");
    }
}

/// Program and arguments that open `url` in the default browser on this OS.
fn open_command(url: &str) -> (&'static str, Vec<String>) {
    #[cfg(target_os = "windows")]
    {
        (
            "cmd",
            vec![
                "/C".to_string(),
                "start".to_string(),
                String::new(),
                url.to_string(),
            ],
        )
    }
    #[cfg(target_os = "macos")]
    {
        ("open", vec![url.to_string()])
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        ("xdg-open", vec![url.to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_reads_xray_and_singbox_output() {
        assert_eq!(
            parse_version("Xray 25.3.6 (Xray, Penetration Test) [x86_64-linux]"),
            Some("25.3.6".to_string())
        );
        assert_eq!(
            parse_version("sing-box version 1.11.5\n\nEnvironment: go1.24.0 linux/amd64"),
            Some("1.11.5".to_string())
        );
        // A commit hash before the version must not confuse the parser.
        assert_eq!(
            parse_version("Xray 1.8.24 (Xray, Penetration Test)"),
            Some("1.8.24".to_string())
        );
    }

    #[test]
    fn parse_version_handles_missing_and_noise() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("   \n  \n"), None);
        assert_eq!(parse_version("usage: xray version"), None);
    }

    #[test]
    fn open_command_includes_the_url() {
        let (program, args) = open_command("https://example.com");
        assert!(!program.is_empty());
        assert!(args.iter().any(|arg| arg == "https://example.com"));
    }
}
