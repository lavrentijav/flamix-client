//! Android entry point for Flamix (milestone M1: an empty Slint window).
//!
//! Kept in its own crate so the Android build does not pull in the desktop-only
//! dependencies of `app/flamix` (tray, native file dialogs, clipboard, ...).

#[cfg(unix)]
pub mod tunnel;

slint::slint! {
    export component MainWindow inherits Window {
        title: "Flamix";
        background: #101418;
        Text {
            text: "Flamix";
            color: white;
            font-size: 28px;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
    }
}

/// Create and run the minimal window. Shared by the Android entry point and host checks.
pub fn run() -> Result<(), slint::PlatformError> {
    MainWindow::new()?.run()
}

/// Called by `android-activity` from the `NativeActivity` glue.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    if slint::android::init(app).is_ok() {
        let _ = run();
    }
}
