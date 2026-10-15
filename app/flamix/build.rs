use std::fmt::Write as _;
use std::path::Path;

fn main() {
    #[cfg(windows)]
    windows_resources();

    // `ui/app.slint` imports `assets/fonts/DejaVuSans*.ttf`, embedded via the
    // default `EmbedFiles` and registered at runtime, so the software renderer
    // has a font on hosts without system fonts.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile");

    generate_flags();
}

/// Generate `$OUT_DIR/flags.rs` mapping every `assets/flags/<ISO>.svg` to its bytes.
fn generate_flags() {
    let flags_dir = Path::new("assets/flags");
    println!("cargo:rerun-if-changed=assets/flags");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");

    let mut codes: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(flags_dir).expect("read assets/flags");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("svg") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                codes.push(stem.to_ascii_uppercase());
            }
        }
    }
    codes.sort();
    codes.dedup();

    let mut code = String::new();
    code.push_str("pub fn svg_for(iso: &str) -> &'static [u8] {\n    match iso {\n");
    for iso in &codes {
        let _ = writeln!(
            code,
            "        {iso:?} => include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/flags/{iso}.svg\")),"
        );
    }
    code.push_str(
        "        _ => include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/flags/UN.svg\")),\n    }\n}\n",
    );

    std::fs::write(Path::new(&out_dir).join("flags.rs"), code).expect("write flags.rs");
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
