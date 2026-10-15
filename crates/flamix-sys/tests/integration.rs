//! Integration tests for the core runner (Linux only). Uses fake cores (shell scripts) so no
//! real Xray/sing-box binary is needed.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use flamix_core::config::{CoreKind, CorePlan};
use tokio::sync::Mutex;

/// `FLAMIX_CORE_DIR` is process-global; serialize the tests that poke it.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flamix-sys-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fake_core(tag: &str, body: &str) -> PathBuf {
    let dir = temp_dir(tag);
    write_executable(&dir.join("xray"), body);
    dir
}

fn plan() -> CorePlan {
    CorePlan {
        core: CoreKind::Xray,
        config: serde_json::json!({ "log": { "loglevel": "warning" } }),
        elevated: false,
        api_port: 10809,
    }
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.is_file()
}

fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !pid_alive(pid)
}

#[tokio::test]
async fn logs_and_stops_a_running_core() {
    let _guard = ENV_LOCK.lock().await;
    let dir = fake_core(
        "run",
        "#!/bin/sh\necho hello core\necho boot line >&2\nsleep 30\n",
    );
    std::env::set_var("FLAMIX_CORE_DIR", &dir);

    let work = dir.join("work");
    let plans = vec![plan()];
    let mut running = flamix_sys::runner::start(&plans, &work).await.unwrap();

    assert!(running.is_alive());
    assert!(work.join("xray.json").is_file());

    // Give the reader tasks a moment to drain both pipes.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let tail = running.log_tail(10);
    assert!(
        tail.iter().any(|l| l.contains("hello core")),
        "stdout missing from {tail:?}"
    );
    assert!(
        tail.iter().any(|l| l.contains("boot line")),
        "stderr missing from {tail:?}"
    );

    running.stop().await.unwrap();

    std::env::remove_var("FLAMIX_CORE_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn early_exit_is_reported_as_core_exited() {
    let _guard = ENV_LOCK.lock().await;
    let dir = fake_core("exit", "#!/bin/sh\necho crash boom >&2\nexit 7\n");
    std::env::set_var("FLAMIX_CORE_DIR", &dir);

    let work = dir.join("work");
    let plans = vec![plan()];
    let err = match flamix_sys::runner::start(&plans, &work).await {
        Ok(_) => panic!("expected the fake core to fail"),
        Err(e) => e,
    };
    match err {
        flamix_sys::Error::CoreExited(tail) => {
            assert!(tail.contains("crash boom"), "tail was {tail:?}");
        }
        other => panic!("expected CoreExited, got {other:?}"),
    }

    std::env::remove_var("FLAMIX_CORE_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run the generated watchdog script directly (no pkexec) with `app_pid` as the app process.
fn launch_watchdog(dir: &Path, core: &Path, cfg: &Path, app_pid: u32) -> std::process::Child {
    let script = flamix_sys::runner::elevated_watchdog_script(CoreKind::SingBox, core, cfg, dir);
    Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("flamix")
        .arg(app_pid.to_string())
        .spawn()
        .unwrap()
}

/// A fake core that replaces itself with `sleep` so the recorded pid is directly killable.
fn fake_sleep_core(dir: &Path) -> PathBuf {
    let core = dir.join("sing-box");
    write_executable(&core, "#!/bin/sh\necho core-ready\nexec sleep 60\n");
    core
}

#[tokio::test]
async fn watchdog_stop_file_kills_core() {
    let dir = temp_dir("wd-stop");
    let core = fake_sleep_core(&dir);
    let cfg = dir.join("sing-box.json");
    std::fs::write(&cfg, "{}").unwrap();

    // The app stays alive (this test process), only the stop file ends the watchdog.
    let mut watchdog = launch_watchdog(&dir, &core, &cfg, std::process::id());

    let pid_path = dir.join("sing-box.pid");
    assert!(wait_for_file(&pid_path, Duration::from_secs(5)));
    let core_pid = read_pid(&pid_path);
    assert!(pid_alive(core_pid), "core should be running");

    std::fs::write(dir.join("stop"), b"").unwrap();
    assert!(
        wait_pid_gone(core_pid, Duration::from_secs(6)),
        "stop file should kill the core"
    );

    let _ = watchdog.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn watchdog_app_exit_kills_core() {
    let dir = temp_dir("wd-app");
    let core = fake_sleep_core(&dir);
    let cfg = dir.join("sing-box.json");
    std::fs::write(&cfg, "{}").unwrap();

    // A separate process plays the app; killing it must tear the core down.
    let mut app = Command::new("sleep").arg("60").spawn().unwrap();
    let app_pid = app.id();
    let mut watchdog = launch_watchdog(&dir, &core, &cfg, app_pid);

    let pid_path = dir.join("sing-box.pid");
    assert!(wait_for_file(&pid_path, Duration::from_secs(5)));
    let core_pid = read_pid(&pid_path);
    assert!(pid_alive(core_pid), "core should be running");

    Command::new("kill")
        .arg(app_pid.to_string())
        .status()
        .unwrap();
    let _ = app.wait();
    assert!(
        wait_pid_gone(core_pid, Duration::from_secs(6)),
        "app exit should kill the core"
    );

    let _ = watchdog.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
