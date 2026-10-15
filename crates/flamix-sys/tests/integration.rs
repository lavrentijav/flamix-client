//! Integration test for the core runner (Linux only). Uses fake cores (shell scripts) so no
//! real Xray/sing-box binary is needed.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::time::Duration;

use flamix_core::config::{CoreKind, CorePlan};
use tokio::sync::Mutex;

/// `FLAMIX_CORE_DIR` is process-global; serialize the tests that poke it.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

fn fake_core(tag: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flamix-sys-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("xray");
    std::fs::write(&binary, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
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
