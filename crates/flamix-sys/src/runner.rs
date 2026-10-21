//! External core processes (xray / sing-box / mihomo). OWNER: task T05.
//!
//! A normal core is a direct child process. An **elevated** core (TUN) is different: it runs as
//! root, so the app cannot signal it. It is therefore launched through a tiny root-owned POSIX
//! watchdog script which holds the core, watches the app pid and a stop file, and tears the core
//! down when either disappears. This keeps `stop()` working without a second password prompt and
//! cleans up even if the app crashes.

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

/// Stop-request file for elevated cores (inside the user-owned work dir).
const STOP_FILE: &str = "stop";

/// How long `stop` waits for the root watchdog to tear its core down.
#[cfg(unix)]
const ELEVATED_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the app waits for a freshly launched elevated core to report its pid.
/// Generous because the polkit password prompt is interactive.
#[cfg(unix)]
const ELEVATED_PID_TIMEOUT: Duration = Duration::from_secs(20);

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
    #[cfg(unix)]
    Elevated(Elevated),
}

/// An elevated core managed by a root watchdog script. Tracked through its pid file and torn down
/// by writing `<work_dir>/stop`.
#[cfg(unix)]
struct Elevated {
    pid: u32,
    name: String,
    work_dir: PathBuf,
}

#[cfg(unix)]
impl Elevated {
    fn pid_file(&self) -> PathBuf {
        self.work_dir.join(format!("{}.pid", self.name))
    }

    fn stop_file(&self) -> PathBuf {
        self.work_dir.join(STOP_FILE)
    }
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
///
/// `geo_dir` is the directory containing the Xray `geosite.dat`/`geoip.dat` databases. When
/// `Some`, Xray cores (normal and elevated) get `XRAY_LOCATION_ASSET` pointing there, so
/// `geoip:`/`geosite:` rules resolve from the downloaded files. When `None` the variable is
/// left unset and the core falls back to the directory next to its binary. sing-box is
/// unaffected: its config uses remote `.srs` rule-sets, not local geo files.
pub async fn start(
    plans: &[CorePlan],
    work_dir: &Path,
    geo_dir: Option<&Path>,
) -> crate::Result<Running> {
    tokio::fs::create_dir_all(work_dir).await?;
    // A stop file left over from a previous run must not kill the new core immediately.
    let _ = tokio::fs::remove_file(work_dir.join(STOP_FILE)).await;

    let log: LogBuffer = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_CAP)));
    let mut ordered: Vec<&CorePlan> = plans.iter().collect();
    ordered.sort_by_key(|p| match p.core {
        CoreKind::Xray => 0u8,
        CoreKind::SingBox => 1,
        CoreKind::Mihomo => 2,
    });

    let mut procs: Vec<Proc> = Vec::new();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    for plan in ordered {
        match spawn_one(plan, work_dir, geo_dir, &log, &mut tasks).await {
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
    geo_dir: Option<&Path>,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let extension = if plan.core == CoreKind::Mihomo {
        "yaml"
    } else {
        "json"
    };
    let config_path = work_dir.join(format!("{}.{extension}", plan.core.binary_name()));
    let text = if plan.core == CoreKind::Mihomo {
        flamix_core::config::mihomo::render_yaml(&plan.config)
            .map_err(|e| crate::Error::Command(format!("serialize mihomo config: {e}")))?
    } else {
        serde_json::to_string_pretty(&plan.config).map_err(|e| {
            crate::Error::Command(format!("serialize {} config: {e}", plan.core.binary_name()))
        })?
    };
    tokio::fs::write(&config_path, text).await?;
    // mihomo reads `geosite.dat` / `geoip.dat` from its home dir (`-d`): reuse the databases
    // the app already downloaded for Xray so the first start needs no download.
    if plan.core == CoreKind::Mihomo {
        if let Some(geo) = geo_dir {
            for name in ["geosite.dat", "geoip.dat"] {
                let (src, dst) = (geo.join(name), work_dir.join(name));
                if tokio::fs::metadata(&src).await.is_ok() {
                    if let Err(e) = tokio::fs::copy(&src, &dst).await {
                        tracing::warn!("copy {name} for mihomo: {e}");
                    }
                }
            }
        }
    }

    let binary = find_core(plan.core)?;
    if plan.elevated {
        spawn_elevated(plan, &binary, &config_path, work_dir, geo_dir, log, tasks).await
    } else {
        let args = core_args(plan.core, &config_path, work_dir);
        spawn_command(
            binary.as_os_str(),
            &args,
            xray_geo(plan.core, geo_dir),
            log,
            tasks,
        )
        .await
    }
}

/// The `XRAY_LOCATION_ASSET` value a plan should inherit: only Xray reads the local geo
/// databases, so sing-box always gets `None` even when a geo dir was supplied.
fn xray_geo(core: CoreKind, geo_dir: Option<&Path>) -> Option<&Path> {
    if core == CoreKind::Xray {
        geo_dir
    } else {
        None
    }
}

fn core_args(core: CoreKind, config_path: &Path, work_dir: &Path) -> Vec<OsString> {
    // mihomo: `mihomo -d <work_dir> -f <config>`.
    if core == CoreKind::Mihomo {
        return vec![
            "-d".into(),
            work_dir.as_os_str().to_os_string(),
            "-f".into(),
            config_path.as_os_str().to_os_string(),
        ];
    }
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
    geo_dir: Option<&Path>,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Point Xray at the downloaded geo databases. Unset when no geo dir was supplied, so the
    // core keeps its default (the directory next to its binary).
    if let Some(dir) = geo_dir {
        cmd.env("XRAY_LOCATION_ASSET", dir);
    }
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

// --- elevated launch helpers -------------------------------------------------

/// Quote a string for safe use as one `/bin/sh` word (POSIX single-quote form).
#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The body of the root watchdog script. `core_cmd`, `log`, `pid` and `stop` must already be
/// shell-quoted. `env_prefix` is an already-quoted `export …; ` prelude (empty when the core
/// needs no extra environment). The app pid is passed as `$1`.
#[cfg(unix)]
fn watchdog_script_body(
    core_cmd: &str,
    env_prefix: &str,
    log: &str,
    pid: &str,
    stop: &str,
) -> String {
    format!(
        "app_pid=$1\n\
         {env_prefix}{core_cmd} > {log} 2>&1 &\n\
         core_pid=$!\n\
         printf '%s\\n' \"$core_pid\" > {pid}\n\
         while kill -0 \"$app_pid\" 2>/dev/null && [ ! -e {stop} ] && kill -0 \"$core_pid\" 2>/dev/null; do\n\
         \tsleep 0.3\n\
         done\n\
         kill \"$core_pid\" 2>/dev/null\n\
         n=0\n\
         while kill -0 \"$core_pid\" 2>/dev/null; do\n\
         \tn=$((n+1))\n\
         \tif [ \"$n\" -ge 10 ]; then kill -9 \"$core_pid\" 2>/dev/null; break; fi\n\
         \tsleep 0.3\n\
         done\n\
         rm -f {stop}\n"
    )
}

/// Build the POSIX `/bin/sh` watchdog script used to launch an elevated core. The script starts
/// the core in the background, writes its pid to `<work_dir>/<core>.pid`, redirects its output to
/// `<work_dir>/<core>.log`, then loops until the app pid (`$1`) is gone, `<work_dir>/stop`
/// appears, or the core exits, and finally kills the core (TERM then KILL after 3 s).
///
/// When `geo_dir` is `Some` and `core` is Xray, the script exports
/// `XRAY_LOCATION_ASSET=<geo_dir>` before starting the (root-owned) core, so it resolves
/// `geosite:`/`geoip:` from the downloaded databases like the non-elevated path.
///
/// Exposed so tests can run it directly without `pkexec`/`osascript`.
#[cfg(unix)]
pub fn elevated_watchdog_script(
    core: CoreKind,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    geo_dir: Option<&Path>,
) -> String {
    let name = core.binary_name();
    let mut parts = vec![shell_quote(&binary.to_string_lossy())];
    for arg in core_args(core, config_path, work_dir) {
        parts.push(shell_quote(&arg.to_string_lossy()));
    }
    let core_cmd = parts.join(" ");
    let env_prefix = match xray_geo(core, geo_dir) {
        Some(dir) => format!(
            "export XRAY_LOCATION_ASSET={}; ",
            shell_quote(&dir.to_string_lossy())
        ),
        None => String::new(),
    };
    let log = shell_quote(&work_dir.join(format!("{name}.log")).to_string_lossy());
    let pid = shell_quote(&work_dir.join(format!("{name}.pid")).to_string_lossy());
    let stop = shell_quote(&work_dir.join(STOP_FILE).to_string_lossy());
    watchdog_script_body(&core_cmd, &env_prefix, &log, &pid, &stop)
}

/// Wait until the watchdog has written `<path>`, returning the core pid.
#[cfg(unix)]
async fn wait_pid_file(path: &Path, timeout: Duration) -> crate::Result<u32> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(text) = tokio::fs::read_to_string(path).await {
            if let Ok(pid) = text.trim().parse::<u32>() {
                if pid > 0 {
                    return Ok(pid);
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::Error::Command(
                "elevation was cancelled or the core did not start".to_string(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Is `pid` alive? Uses `kill -0`: success or EPERM ("Operation not permitted", i.e. a root
/// process we cannot signal) both mean "still running"; "No such process" means gone.
#[cfg(unix)]
fn pid_alive_privileged(pid: u32) -> bool {
    let Ok(out) = std::process::Command::new("kill")
        .env("LC_ALL", "C")
        .args(["-0", &pid.to_string()])
        .output()
    else {
        return false;
    };
    if out.status.success() {
        return true;
    }
    String::from_utf8_lossy(&out.stderr).contains("not permitted")
}

/// Ask the root watchdog to tear its core down by creating the stop file, then wait until the
/// core pid is gone (up to [`ELEVATED_STOP_TIMEOUT`]).
#[cfg(unix)]
async fn elevated_stop(e: &Elevated) -> crate::Result<()> {
    tokio::fs::write(e.stop_file(), b"").await?;
    let deadline = tokio::time::Instant::now() + ELEVATED_STOP_TIMEOUT;
    while pid_alive_privileged(e.pid) {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                "elevated core {} did not stop within {:?}",
                e.pid,
                ELEVATED_STOP_TIMEOUT
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = tokio::fs::remove_file(e.pid_file()).await;
    Ok(())
}

#[cfg(target_os = "linux")]
fn command_exists(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn spawn_elevated(
    plan: &CorePlan,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    geo_dir: Option<&Path>,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    // Already root or the binary carries the capability: run it directly.
    if crate::elevation::is_elevated() || has_cap_net_admin(binary) {
        let args = core_args(plan.core, config_path, work_dir);
        return spawn_command(
            binary.as_os_str(),
            &args,
            xray_geo(plan.core, geo_dir),
            log,
            tasks,
        )
        .await;
    }
    if !command_exists("pkexec") {
        return Err(crate::Error::Command(format!(
            "pkexec not found: install polkit, or grant the core the needed capabilities with \
             'sudo setcap cap_net_admin,cap_net_bind_service+ep {}'",
            binary.display()
        )));
    }

    let name = plan.core.binary_name().to_string();
    let pid_path = work_dir.join(format!("{name}.pid"));
    let log_path = work_dir.join(format!("{name}.log"));
    let _ = tokio::fs::remove_file(&pid_path).await;

    let script = elevated_watchdog_script(plan.core, binary, config_path, work_dir, geo_dir);
    let app_pid = std::process::id().to_string();
    let mut cmd = tokio::process::Command::new("pkexec");
    cmd.arg("/bin/sh")
        .arg("-c")
        .arg(&script)
        .arg("flamix")
        .arg(&app_pid)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Command(format!("failed to start pkexec: {e}")))?;
    if let Some(stdout) = child.stdout.take() {
        tasks.push(spawn_reader(stdout, log.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(spawn_reader(stderr, log.clone()));
    }
    // Keep the launcher reaped, but never signal it: the watchdog must survive the app.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    let pid = wait_pid_file(&pid_path, ELEVATED_PID_TIMEOUT).await?;
    tasks.push(spawn_tailer(log_path, log.clone()));
    Ok(Proc {
        handle: Handle::Elevated(Elevated {
            pid,
            name,
            work_dir: work_dir.to_path_buf(),
        }),
    })
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
    geo_dir: Option<&Path>,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    let name = plan.core.binary_name().to_string();
    let pid_path = work_dir.join(format!("{name}.pid"));
    let log_path = work_dir.join(format!("{name}.log"));
    let _ = tokio::fs::remove_file(&pid_path).await;

    let script = elevated_watchdog_script(plan.core, binary, config_path, work_dir, geo_dir);
    let app_pid = std::process::id().to_string();
    let shell = format!(
        "/bin/sh -c {} flamix {} &> /dev/null &",
        shell_quote(&script),
        app_pid
    );
    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(&shell)
    );
    let status = tokio::process::Command::new("osascript")
        .args(["-e", &osa])
        .status()
        .await?;
    if !status.success() {
        return Err(crate::Error::Command(
            "osascript elevation was cancelled or failed".to_string(),
        ));
    }

    let pid = wait_pid_file(&pid_path, ELEVATED_PID_TIMEOUT).await?;
    tasks.push(spawn_tailer(log_path, log.clone()));
    Ok(Proc {
        handle: Handle::Elevated(Elevated {
            pid,
            name,
            work_dir: work_dir.to_path_buf(),
        }),
    })
}

#[cfg(target_os = "windows")]
async fn spawn_elevated(
    plan: &CorePlan,
    binary: &Path,
    config_path: &Path,
    work_dir: &Path,
    geo_dir: Option<&Path>,
    log: &LogBuffer,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> crate::Result<Proc> {
    if crate::elevation::is_elevated() {
        let args = core_args(plan.core, config_path, work_dir);
        spawn_command(
            binary.as_os_str(),
            &args,
            xray_geo(plan.core, geo_dir),
            log,
            tasks,
        )
        .await
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
    _geo_dir: Option<&Path>,
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
#[cfg(unix)]
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
        #[cfg(unix)]
        Handle::Elevated(e) => pid_alive_privileged(e.pid),
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
        #[cfg(unix)]
        Handle::Elevated(e) => elevated_stop(e).await,
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

    #[cfg(unix)]
    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("plain/path"), "'plain/path'");
        assert_eq!(shell_quote("/tmp/a b/c"), "'/tmp/a b/c'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[cfg(unix)]
    #[test]
    fn watchdog_script_quotes_paths_and_loops() {
        let work = PathBuf::from("/tmp/fla mix/work'x");
        let bin = PathBuf::from("/opt/x ray/sing-box");
        let cfg = work.join("sing-box.json");
        let script = elevated_watchdog_script(CoreKind::SingBox, &bin, &cfg, &work, None);

        assert!(script.contains("app_pid=$1"));
        assert!(script.contains("'/opt/x ray/sing-box'"));
        assert!(script.contains(&shell_quote("/tmp/fla mix/work'x/sing-box.log")));
        assert!(script.contains("kill -9"));
        assert!(script.contains(&format!(
            "rm -f {}",
            shell_quote("/tmp/fla mix/work'x/stop")
        )));
        // No geo dir: the asset variable must not be exported.
        assert!(!script.contains("XRAY_LOCATION_ASSET"));
    }

    #[cfg(unix)]
    #[test]
    fn watchdog_script_exports_geo_only_for_xray() {
        let work = PathBuf::from("/tmp/work");
        let bin = PathBuf::from("/opt/xray");
        let cfg = work.join("xray.json");
        let geo = PathBuf::from("/home/u/.flamix/geo data");

        let xray = elevated_watchdog_script(CoreKind::Xray, &bin, &cfg, &work, Some(&geo));
        assert!(xray.contains(&format!(
            "export XRAY_LOCATION_ASSET={};",
            shell_quote("/home/u/.flamix/geo data")
        )));

        // sing-box ignores the geo dir (remote `.srs` rule-sets).
        let sing = elevated_watchdog_script(CoreKind::SingBox, &bin, &cfg, &work, Some(&geo));
        assert!(!sing.contains("XRAY_LOCATION_ASSET"));
    }
}
