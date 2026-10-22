//! In-app log file `<files dir>/flamix.log` shared with the Kotlin side (`FileLog.kt`).
//!
//! Both sides append lines `time level tag: message` and rotate at 200 KB (the old file becomes
//! `flamix.log.1`). Rust writes panics (hook) and tunnel start/stop reasons; the Settings tab
//! shows the tail.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_BYTES: u64 = 200 * 1024;
const FILE: &str = "flamix.log";

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn dir() -> Option<PathBuf> {
    DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Sets the log directory and installs the panic hook (once, from `android_main`).
pub fn init(dir: &Path) {
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let bt = std::backtrace::Backtrace::force_capture();
        log(
            "E",
            "panic",
            &format!("thread '{}': {info}\n{bt}", thread.name().unwrap_or("?")),
        );
    }));
}

/// Appends one entry. Never fails the caller.
pub fn log(level: &str, tag: &str, msg: &str) {
    if let Some(dir) = dir() {
        append(&dir, level, tag, msg);
    }
}

fn append(dir: &Path, level: &str, tag: &str, msg: &str) {
    let path = dir.join(FILE);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join(format!("{FILE}.1")));
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{secs} {level} {tag}: {msg}");
    }
}

/// Last `n` lines of the log (empty string when there is none).
pub fn tail(dir: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Deletes the log files.
pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
    let _ = std::fs::remove_file(dir.join(format!("{FILE}.1")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_clear_and_rotation() {
        let d = std::env::temp_dir().join(format!("flamix-applog-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..5 {
            append(&d, "I", "t", &format!("line {i}"));
        }
        let t = tail(&d, 2);
        assert_eq!(t.lines().count(), 2);
        assert!(t.contains("line 4") && !t.contains("line 2"));
        let big = "x".repeat(1000);
        for _ in 0..250 {
            append(&d, "I", "t", &big);
        }
        assert!(d.join("flamix.log.1").exists());
        assert!(std::fs::metadata(d.join("flamix.log")).unwrap().len() <= MAX_BYTES + 2000);
        clear(&d);
        assert_eq!(tail(&d, 5), "");
        let _ = std::fs::remove_dir_all(&d);
    }
}
