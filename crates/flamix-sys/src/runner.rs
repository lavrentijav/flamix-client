//! External core processes (xray / sing-box). OWNER: task T05.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flamix_core::config::{CoreKind, CorePlan};

/// Shared ring buffer of the combined stdout/stderr of every core process.
pub type LogBuffer = Arc<Mutex<VecDeque<String>>>;

/// Number of log lines kept (per task T05).
const LOG_CAP: usize = 500;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Locate a core binary: `FLAMIX_CORE_DIR`, then `<exe dir>/cores`, then `<exe dir>`,
/// then (macOS bundle) `<exe dir>/../Resources/cores`, then `/usr/lib/flamix/cores`, then `PATH`.
pub fn find_core(kind: CoreKind) -> crate::Result<PathBuf> {
    let file = format!("{}{}", kind.binary_name(), std::env::consts::EXE_SUFFIX);
    for dir in candidate_dirs() {
        let candidate = dir.join(&file);
        if is_executable_file(&candidate) {
            return Ok(candidate);
        }
    }
    if let Some(found) = find_in_path(&file) {
        return Ok(found);
    }
    Err(crate::Error::CoreNotFound(file))
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("FLAMIX_CORE_DIR") {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("cores"));
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("../Resources/cores"));
        }
    }
    dirs.push(PathBuf::from("/usr/lib/flamix/cores"));
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_in_path(file: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(file))
        .find(|candidate| is_executable_file(candidate))
}

/// A running set of core processes.
pub struct Running {
    procs: Vec<Proc>,
    log: LogBuffer,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

struct Proc {
    handle: Handle,
}

enum Handle {
    Child(tokio::process::Child),
    /// Elevated core started detached by macOS `osascript`; tracked through its pid.
    #[allow(dead_code)]
    Pid {
        pid: u32,
    },
}

impl Running {
    /// True while every process is alive.
    pub fn is_alive(&mut self) -> bool {
        self.procs.iter_mut().all(|p| handle_alive(&mut p.handle))
    }

    /// Last lines of combined stdout/stderr (for the log view), newest last.
    pub fn log_tail(&self, max_lines: usize) -> Vec<String> {
        let Ok(buf) = self.log.lock() else {
            return Vec::new();
        };
        let start = buf.len().saturating_sub(max_lines);
        buf.iter().skip(start).cloned().collect()
    }

    /// Kill all processes (elevated ones through the same elevation helper) and wait.
    pub async fn stop(self) -> crate::Result<()> {
        let Running { procs, tasks, .. } = self;
        for task in tasks {
            task.abort();
        }
        for mut proc in procs {
            stop_handle(&mut proc.handle).await?;
        }
        Ok(())
    }
}

/// Write each plan's config to `<work_dir>/<core>.json` and start the processes in order
/// (Xray before sing-box). Elevated plans go through [`crate::elevation`]. Fails if a
/// process exits within 1.5 s (error carries its last log lines).
pub async fn start(plans: &[CorePlan], work_dir: &Path) -> crate::Result<Running> {
    tokio::fs::create_dir_all(work_dir).await?;

    let log: LogBuffer = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_CAP)));
    let mut ordered: Vec<&CorePlan> = plans.iter().collect();
    ordered.sort_by_key(|p| match p.core {
        CoreKind::Xray => 0u8,
        CoreKind::SingBox => 1,
    });

    let mut procs: Vec<Proc> = Vec::new();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    for plan in ordered {
        match spawn_one(plan, work_dir, &log, &mut tasks).await {
            Ok(proc) => procs.push(proc),
            Err(e) => {
                for task in tasks {
                    task.abort();
                }
                for mut proc in procs {
                    let _ = stop_handle(&mut proc.handle).await;
                }
                return Err(e);
            }
        }
    }

    let mut running = Running { procs, log, tasks };
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let died = running
        .procs
        .iter_mut()
        .any(|p| !handle_alive(&mut p.handle));
    if died {
        // Let the reader tasks flush whatever the process wrote before it died.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let tail = running.log_tail(20).join("\n");
        let Running { procs, tasks, .. } = running;
        for task in tasks {
            task.abort();
        }
        for mut proc in procs {
            let _ = stop_handle(&mut proc.handle).await;
        }
        return Err(crate::Error::CoreExited(tail));
    }
    Ok(running)
}

async fn spawn_one(
    plan: &CorePlan,
    work_dir: &Path,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let config_path = work_dir.join(format!("{}.json", plan.core.binary_name()));
    let json = serde_json::to_string_pretty(&plan.config).map_err(|e| {
        crate::Error::Command(format!("serialize {} config: {e}", plan.core.binary_name()))
    })?;
    tokio::fs::write(&config_path, json).await?;

    let binary = find_core(plan.core)?;
    if plan.elevated {
        spawn_elevated(plan, &binary, &config_path, work_dir, log, tasks).await
    } else {
        let args = core_args(plan.core, &config_path, work_dir);
        spawn_command(binary.as_os_str(), &args, log, tasks).await
    }
}

fn core_args(core: CoreKind, config_path: &Path, work_dir: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "run".into(),
        "-c".into(),
        config_path.as_os_str().to_os_string(),
    ];
    if core == CoreKind::SingBox {
        args.push("-D".into());
        args.push(work_dir.as_os_str().to_os_string());
    }
    args
}

async fn spawn_command(
    program: &OsStr,
    args: &[OsString],
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        crate::Error::Command(format!(
            "failed to start {}: {e}",
            program.to_string_lossy()
        ))
    })?;
    if let Some(stdout) = child.stdout.take() {
        tasks.push(spawn_reader(stdout, log.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(spawn_reader(stderr, log.clone()));
    }
    Ok(Proc {
        handle: Handle::Child(child),
    })
}

#[cfg(target_os = "linux")]
async fn spawn_elevated(
    plan: &CorePlan,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    if crate::elevation::is_elevated() || has_cap_net_admin(binary) {
        let args = core_args(plan.core, config_path, work_dir);
        return spawn_command(binary.as_os_str(), &args, log, tasks).await;
    }
    let mut args: Vec<OsString> = vec![binary.as_os_str().to_os_string()];
    args.extend(core_args(plan.core, config_path, work_dir));
    spawn_command(OsStr::new("pkexec"), &args, log, tasks).await
}

/// Best-effort check: does `getcap <binary>` report `cap_net_admin`?
#[cfg(target_os = "linux")]
fn has_cap_net_admin(binary: &Path) -> bool {
    let Ok(out) = std::process::Command::new("getcap").arg(binary).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("cap_net_admin")
}

#[cfg(target_os = "macos")]
async fn spawn_elevated(
    plan: &CorePlan,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let name = plan.core.binary_name();
    let pid_file = work_dir.join(format!("{name}.pid"));
    let log_file = work_dir.join(format!("{name}.log"));
    let _ = std::fs::remove_file(&pid_file);

    let mut command = shell_quote(&binary.to_string_lossy());
    for arg in core_args(plan.core, config_path, work_dir) {
        command.push(' ');
        command.push_str(&shell_quote(&arg.to_string_lossy()));
    }
    let shell = format!(
        "{command} > {log} 2>&1 & echo $! > {pid}",
        log = shell_quote(&log_file.to_string_lossy()),
        pid = shell_quote(&pid_file.to_string_lossy())
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(&shell)
    );
    let status = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
        .await?;
    if !status.success() {
        return Err(crate::Error::Command(
            "osascript elevation was cancelled or failed".to_string(),
        ));
    }
    let pid = read_pid(&pid_file).await?;
    tasks.push(spawn_tailer(log_file, log.clone()));
    Ok(Proc {
        handle: Handle::Pid { pid },
    })
}

#[cfg(target_os = "windows")]
async fn spawn_elevated(
    plan: &CorePlan,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    if crate::elevation::is_elevated() {
        let args = core_args(plan.core, config_path, work_dir);
        spawn_command(binary.as_os_str(), &args, log, tasks).await
    } else {
        Err(crate::Error::Command(
            "TUN mode needs Flamix to run as administrator".to_string(),
        ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
async fn spawn_elevated(
    _plan: &CorePlan,
    _binary: &Path,
    _config_path: &Path,
    _work_dir: &Path,
    _log: &LogBuffer,
    _tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    Err(crate::Error::Unsupported)
}

fn spawn_reader<R>(reader: R, log: LogBuffer) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            push_line(&log, line);
        }
    })
}

/// Tail `<path>` for lines appended by an elevated, detached core.
#[cfg(target_os = "macos")]
fn spawn_tailer(path: PathBuf, log: LogBuffer) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seen = 0usize;
        loop {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let mut lines: Vec<&str> = content.lines().collect();
                if !content.ends_with('\n') && !lines.is_empty() {
                    lines.pop();
                }
                for line in lines.iter().skip(seen) {
                    push_line(&log, (*line).to_string());
                }
                seen = seen.max(lines.len());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
}

fn push_line(log: &LogBuffer, line: String) {
    if let Ok(mut buf) = log.lock() {
        if buf.len() >= LOG_CAP {
            buf.pop_front();
        }
        buf.push_back(line);
    }
}

fn handle_alive(handle: &mut Handle) -> bool {
    match handle {
        Handle::Child(child) => matches!(child.try_wait(), Ok(None)),
        Handle::Pid { pid } => pid_alive(*pid),
    }
}

async fn stop_handle(handle: &mut Handle) -> crate::Result<()> {
    match handle {
        Handle::Child(child) => {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                let _ = tokio::process::Command::new("kill")
                    .arg(pid.to_string())
                    .status()
                    .await;
            }
            if !wait_exit(child, Duration::from_secs(3)).await {
                let _ = child.kill().await;
            }
            let _ = child.wait().await;
            Ok(())
        }
        Handle::Pid { pid } => {
            let _ = kill_pid_elevated(*pid, false).await;
            if !wait_pid_gone(*pid, Duration::from_secs(3)).await {
                let _ = kill_pid_elevated(*pid, true).await;
            }
            Ok(())
        }
    }
}

async fn wait_exit(child: &mut tokio::process::Child, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

async fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !pid_alive(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "macos")]
async fn kill_pid_elevated(pid: u32, force: bool) -> crate::Result<()> {
    let command = if force {
        format!("kill -9 {pid}")
    } else {
        format!("kill {pid}")
    };
    let script = format!("do shell script \"{command}\" with administrator privileges");
    tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
        .await?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn kill_pid_elevated(pid: u32, force: bool) -> crate::Result<()> {
    let _ = (pid, force);
    Err(crate::Error::Unsupported)
}

#[cfg(target_os = "macos")]
async fn read_pid(path: &Path) -> crate::Result<u32> {
    for _ in 0..20 {
        if let Ok(text) = tokio::fs::read_to_string(path).await {
            if let Ok(pid) = text.trim().parse() {
                return Ok(pid);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(crate::Error::Command(
        "elevated core did not report a pid".to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `FLAMIX_CORE_DIR` is process-global; serialize the tests that set it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("flamix-sys-runner-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_executable(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn find_core_uses_env_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir("env");
        let binary = dir.join(format!("xray{}", std::env::consts::EXE_SUFFIX));
        make_executable(&binary);

        std::env::set_var("FLAMIX_CORE_DIR", &dir);
        let found = find_core(CoreKind::Xray).unwrap();
        std::env::remove_var("FLAMIX_CORE_DIR");

        assert_eq!(found, binary);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_core_missing_is_error() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir("missing");
        std::env::set_var("FLAMIX_CORE_DIR", &dir);
        let err = find_core(CoreKind::SingBox).unwrap_err();
        std::env::remove_var("FLAMIX_CORE_DIR");
        assert!(matches!(err, crate::Error::CoreNotFound(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
