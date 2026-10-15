fn main() {
    #[cfg(windows)]
    windows_resources();

    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile");
}

/// Embed the application icon and manifest into the Windows executable.
#[cfg(windows)]
fn windows_resources() {
    use std::path::Path;

    let mut res = winresource::WindowsResource::new();

    let icon = Path::new("assets/app-icon/flamix.ico");
    if icon.exists() {
        res.set_icon(icon.to_str().expect("utf-8 icon path"));
    }

    let manifest = Path::new("../../packaging/windows/flamix.manifest");
    if manifest.exists() {
        res.set_manifest_file(manifest.to_str().expect("utf-8 manifest path"));
    }

    res.compile().expect("embed windows resources");
}
