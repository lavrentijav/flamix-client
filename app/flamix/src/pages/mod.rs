//! Secondary pages. Each page module owns its `ui/pages/<name>-page.slint` file and talks to it
//! through that file's global `<Name>Bridge` (see docs/tasks/T17-pages.md).
//! `init` is called once after the window and (in real mode) the controller exist.

use crate::controller::Controller;
use crate::AppWindow;
use std::sync::Arc;

pub mod about;
pub mod add;
pub mod routing;
pub mod settings;
pub mod stats;

/// Wire every page. `ctrl` is `None` in `--demo` / screenshot mode (fill demo data then).
pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    add::init(ui, ctrl);
    routing::init(ui, ctrl);
    settings::init(ui, ctrl);
    stats::init(ui, ctrl);
    about::init(ui, ctrl);
}

/// Switch the app to dark or light: our own `Theme` and, in lockstep, the std-widgets `Palette`
/// (LineEdit/TextEdit/ComboBox/Button/… otherwise follow the OS colour scheme and look out of
/// place on our custom background).
pub fn apply_theme(ui: &AppWindow, dark: bool) {
    use slint::ComponentHandle;
    ui.global::<crate::Theme>().set_dark(dark);
    // `ColorScheme` is exported through Slint's re-export module; the generated `Palette` global
    // expects exactly this type.
    use slint::private_unstable_api::re_exports::ColorScheme;
    let scheme = if dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    ui.global::<crate::Palette>().set_color_scheme(scheme);
}
