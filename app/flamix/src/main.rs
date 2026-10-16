//! Flamix desktop application entry point.
//!
//! Arguments: `--demo` (fake data, no core), `--screenshot <out.png>` (offscreen render),
//! `--light`, `--minimized`, and an optional positional deep link / subscription URL.
//! Everything else lives in [`controller`]; `--demo` keeps the standalone demo wiring.

mod controller;
mod demo;
mod flags;
mod persist;
mod view;

use controller::Controller;
use slint::ComponentHandle;
use std::path::Path;

slint::include_modules!();

#[derive(Default)]
struct Args {
    demo: bool,
    light: bool,
    minimized: bool,
    screenshot: Option<String>,
    import: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => args.screenshot = it.next(),
            "--light" => args.light = true,
            "--demo" => args.demo = true,
            "--minimized" => args.minimized = true,
            other if other.starts_with('-') => {
                tracing::warn!(argument = other, "ignoring unknown argument")
            }
            other => {
                if args.import.is_none() {
                    args.import = Some(other.to_string());
                } else {
                    tracing::warn!(argument = other, "ignoring extra positional argument");
                }
            }
        }
    }
    args
}

/// Build the app's multi-thread tokio runtime on a background thread and return its handle.
fn build_runtime() -> anyhow::Result<tokio::runtime::Handle> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("flamix-rt")
        .build()?;
    let handle = runtime.handle().clone();
    std::thread::Builder::new()
        .name("flamix-runtime".into())
        .spawn(move || runtime.block_on(std::future::pending::<()>()))?;
    Ok(handle)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args();
    let data_dir = flamix_core::store::data_dir();

    if let Some(path) = args.screenshot.as_deref() {
        let state = if args.demo {
            None
        } else {
            Some(persist::load(&data_dir)?)
        };
        return demo::screenshot(Path::new(path), args.light, state.as_ref());
    }

    let ui = AppWindow::new()?;
    let _controller = if args.demo {
        demo::setup(&ui)?;
        None
    } else {
        let rt = build_runtime()?;
        let state = persist::load(&data_dir)?;
        Some(Controller::new(&ui, data_dir, rt, state, args.import.clone()))
    };

    if args.minimized {
        ui.hide()?;
    }
    ui.run()?;
    Ok(())
}
