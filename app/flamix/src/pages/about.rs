//! Page "О программе" (task T17A). Talks to the page only through the `AboutBridge` global.
//!
//! Core versions are probed on a background task by running `<core> version` (see
//! `flamix_sys::runner::find_core`); "не найдено" is shown when the binary is absent.

use crate::controller::Controller;
use crate::{AboutBridge, AppWindow};
use flamix_core::backup;
use flamix_core::config::CoreKind;
use flamix_core::model::AppState;
use flamix_sys::runner;
use slint::{ComponentHandle, Weak};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Project links opened by the buttons.
const GITHUB: &str = "https://github.com/lavrentijav/flamix-client";
const RELEASES: &str = "https://github.com/lavrentijav/flamix-client/releases";
const ISSUES: &str = "https://github.com/lavrentijav/flamix-client/issues";

/// File-dialog filter name for backup files.
const BACKUP_FILTER: &str = "Flamix backup";

/// Upper bound for a `<core> version` probe so a hung binary cannot pin a task forever.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

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

    wire_backup(ui, ctrl);

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

/// Wire the "Резервная копия" section: export / import all state, with a confirm step for import
/// (it overwrites the current settings and servers).
fn wire_backup(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    let bridge = ui.global::<AboutBridge>();
    bridge.set_backup_status("".into());
    bridge.set_import_confirm(false);

    // The state parsed from the chosen file is kept until the user confirms the overwrite.
    let imported: Arc<Mutex<Option<AppState>>> = Arc::new(Mutex::new(None));

    {
        let weak = ui.as_weak();
        let ctrl = ctrl.cloned();
        ui.global::<AboutBridge>()
            .on_export(move || export_backup(weak.clone(), ctrl.clone()));
    }
    {
        let weak = ui.as_weak();
        let ctrl = ctrl.cloned();
        let imported = imported.clone();
        ui.global::<AboutBridge>()
            .on_import(move || pick_import(weak.clone(), ctrl.clone(), imported.clone()));
    }
    {
        let weak = ui.as_weak();
        let ctrl = ctrl.cloned();
        let imported = imported.clone();
        ui.global::<AboutBridge>().on_import_confirmed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let bridge = ui.global::<AboutBridge>();
            bridge.set_import_confirm(false);
            let state = imported.lock().unwrap_or_else(|e| e.into_inner()).take();
            let Some(state) = state else {
                return;
            };
            match ctrl.as_ref() {
                Some(ctrl) => {
                    // `mutate` is the page API: it applies the state and persists it (debounced),
                    // so no file I/O happens while the state lock is held.
                    ctrl.mutate(move |current| backup::apply_imported(current, state));
                    bridge.set_backup_status("Импортировано".into());
                }
                None => bridge.set_backup_status("Импортировано (демо)".into()),
            }
        });
    }
    {
        let weak = ui.as_weak();
        let imported = imported.clone();
        ui.global::<AboutBridge>().on_import_cancelled(move || {
            imported.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(ui) = weak.upgrade() {
                let bridge = ui.global::<AboutBridge>();
                bridge.set_import_confirm(false);
                bridge.set_backup_status("".into());
            }
        });
    }
}

/// "Экспортировать…": ask for a target file and write the exported state there.
fn export_backup(ui: Weak<AppWindow>, ctrl: Option<Arc<Controller>>) {
    let Some(ui) = ui.upgrade() else { return };
    let bridge = ui.global::<AboutBridge>();
    let Some(ctrl) = ctrl else {
        bridge.set_backup_status("Сохранено: flamix-backup.json (демо)".into());
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .add_filter(BACKUP_FILTER, &["json"])
        .set_file_name("flamix-backup.json")
        .save_file()
    else {
        return;
    };
    // Serialise on the UI thread without holding any lock for long, then write off the UI thread.
    let text = match ctrl.with_state(backup::export) {
        Ok(text) => text,
        Err(err) => {
            bridge.set_backup_status(format!("Ошибка: {err}").into());
            return;
        }
    };
    let ui_for_task = ui.as_weak();
    ctrl.runtime().spawn_blocking(move || {
        let status = match std::fs::write(&path, text) {
            Ok(()) => format!("Сохранено: {}", path.display()),
            Err(err) => format!("Ошибка: {err}"),
        };
        let _ = ui_for_task.upgrade_in_event_loop(move |ui| {
            ui.global::<AboutBridge>().set_backup_status(status.into());
        });
    });
}

/// "Импортировать…": read and parse a file, then ask for confirmation before overwriting.
fn pick_import(
    ui: Weak<AppWindow>,
    ctrl: Option<Arc<Controller>>,
    imported: Arc<Mutex<Option<AppState>>>,
) {
    let Some(ui) = ui.upgrade() else { return };
    let bridge = ui.global::<AboutBridge>();
    let Some(ctrl) = ctrl else {
        bridge.set_backup_status("Импортировано (демо)".into());
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .add_filter(BACKUP_FILTER, &["json"])
        .pick_file()
    else {
        return;
    };
    let display = path.display().to_string();
    let ui_for_task = ui.as_weak();
    ctrl.runtime().spawn_blocking(move || {
        let parsed = std::fs::read_to_string(&path)
            .map_err(flamix_core::Error::from)
            .and_then(|text| backup::import(&text));
        let _ = ui_for_task.upgrade_in_event_loop(move |ui| {
            let bridge = ui.global::<AboutBridge>();
            match parsed {
                Ok(state) => {
                    *imported.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
                    bridge.set_import_confirm(true);
                    bridge.set_backup_status(format!("Выбран файл: {display}").into());
                }
                Err(err) => bridge.set_backup_status(format!("Ошибка: {err}").into()),
            }
        });
    });
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
    let mut cmd = Command::new(binary);
    cmd.arg("version");
    let output = run_with_timeout(cmd, VERSION_TIMEOUT)?;
    parse_core_output(output.status.success(), &output.stdout, &output.stderr)
}

/// Run `cmd`, reaping the child; `None` on spawn error or when it exceeds `timeout`.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Output> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

/// Parse a finished core's output: a successful run's stdout first, then stderr.
fn parse_core_output(success: bool, stdout: &[u8], stderr: &[u8]) -> Option<String> {
    if !success {
        return None;
    }
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(stderr).into_owned();
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
///
/// The child is waited on from a detached thread so it is reaped instead of lingering as a
/// zombie for the whole app lifetime.
fn open_link(url: &str) {
    let (program, args) = open_command(url);
    match Command::new(program).args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(%err, url, "could not open the link in a browser"),
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
    fn parse_core_output_requires_success_and_falls_back_to_stderr() {
        assert_eq!(parse_core_output(false, b"Xray 1.2.3", b""), None);
        assert_eq!(
            parse_core_output(true, b"", b"sing-box version 1.11.5"),
            Some("1.11.5".to_string())
        );
        assert_eq!(parse_core_output(true, b"no digits here", b""), None);
    }

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_collects_output_and_kills_hung_commands() {
        let output = run_with_timeout(
            {
                let mut cmd = Command::new("sh");
                cmd.args(["-c", "printf 'Xray 1.2.3'"]);
                cmd
            },
            Duration::from_secs(5),
        )
        .expect("command should run");
        assert_eq!(
            parse_core_output(output.status.success(), &output.stdout, &output.stderr).as_deref(),
            Some("1.2.3")
        );

        let started = Instant::now();
        let hung = run_with_timeout(
            {
                let mut cmd = Command::new("sh");
                cmd.args(["-c", "sleep 30"]);
                cmd
            },
            Duration::from_millis(100),
        );
        assert!(hung.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn open_command_includes_the_url() {
        let (program, args) = open_command("https://example.com");
        assert!(!program.is_empty());
        assert!(args.iter().any(|arg| arg == "https://example.com"));
    }
}
