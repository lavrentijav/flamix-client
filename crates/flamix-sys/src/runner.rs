//! External core processes (xray / sing-box). OWNER: task T05.

use flamix_core::config::{CoreKind, CorePlan};
use std::path::{Path, PathBuf};

/// Locate a core binary: `FLAMIX_CORE_DIR`, then `<exe dir>/cores`, then `<exe dir>`,
/// then (macOS bundle) `<exe dir>/../Resources/cores`, then `/usr/lib/flamix/cores`, then `PATH`.
pub fn find_core(kind: CoreKind) -> crate::Result<PathBuf> {
    let _ = kind;
    todo!("T05")
}

/// A running set of core processes.
pub struct Running {
    _private: (),
}

impl Running {
    /// True while every process is alive.
    pub fn is_alive(&mut self) -> bool {
        todo!("T05")
    }

    /// Last lines of combined stdout/stderr (for the log view), newest last.
    pub fn log_tail(&self, max_lines: usize) -> Vec<String> {
        let _ = max_lines;
        todo!("T05")
    }

    /// Kill all processes (elevated ones through the same elevation helper) and wait.
    pub async fn stop(self) -> crate::Result<()> {
        todo!("T05")
    }
}

/// Write each plan's config to `<work_dir>/<core>.json` and start the processes in order
/// (Xray before sing-box). Elevated plans go through [`crate::elevation`]. Fails if a
/// process exits within 1.5 s (error carries its last log lines).
pub async fn start(plans: &[CorePlan], work_dir: &Path) -> crate::Result<Running> {
    let _ = (plans, work_dir);
    todo!("T05")
}
