fn main() {
    // Phone layout (ui/android.slint); the Rust side reads it through `slint::include_modules!`.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/android.slint", config).expect("slint compile");
}
