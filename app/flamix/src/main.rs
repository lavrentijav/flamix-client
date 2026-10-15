//! Flamix desktop application entry point.
//!
//! Runs the demo UI until the controller task wires real data. Hidden flags:
//! `--screenshot <out.png>` renders the connected window offscreen (software renderer),
//! `--light` switches to the light theme (use together with `--screenshot`).

mod demo;
mod flags;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut screenshot: Option<String> = None;
    let mut light = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = args.next(),
            "--light" => light = true,
            "--demo" => {}
            other => tracing::warn!(argument = other, "ignoring unknown argument"),
        }
    }

    if let Some(path) = screenshot {
        return demo::screenshot(std::path::Path::new(&path), light);
    }

    let ui = AppWindow::new()?;
    demo::setup(&ui)?;
    ui.run()?;
    Ok(())
}
