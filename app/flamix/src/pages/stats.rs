//! OWNER: the task implementing this page (docs/tasks/T17-pages.md).

use crate::controller::Controller;
use crate::AppWindow;
use std::sync::Arc;

pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    let _ = (ui, ctrl);
}
