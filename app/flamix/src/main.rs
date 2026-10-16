//! Flamix desktop application entry point.
//!
//! Arguments: `--demo` (fake data, no core), `--screenshot <out.png>` (offscreen render),
//! `--light`, `--minimized`, and an optional positional deep link / subscription URL.
//! A second launch forwards its arguments to the running instance and exits.

mod controller;
mod demo;
mod flags;
mod instance;
mod persist;
mod view;

use controller::Controller;
use slint::ComponentHandle;
use std::path::Path;
use std::sync::Arc;

slint::include_modules!();

#[derive(Default)]
struct Args {
    demo: bool,
    light: bool,
    minimized: bool,
    screenshot: Option<String>,
    import: Option<String>,
}

fn parse_args(raw: &[String]) -> Args {
    let mut args = Args::default();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => args.screenshot = it.next().cloned(),
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

/// Result of trying to hand our arguments to an already running instance.
enum Forward {
    /// A running instance accepted the arguments.
    Sent,
    /// The instance file was present but unusable (stale) — we take over.
    Stale,
    /// No instance file.
    None,
}

fn try_forward(dir: &Path, args: &[String]) -> Forward {
    let Ok(text) = std::fs::read_to_string(instance::instance_path(dir)) else {
        return Forward::None;
    };
    let Some(info) = instance::decode(&text) else {
        return Forward::Stale;
    };
    match instance::forward(&info, args) {
        Ok(()) => Forward::Sent,
        Err(_) => Forward::Stale,
    }
}

/// Bind the single-instance listener and publish it to `<data_dir>/instance`.
fn start_listener(
    rt: &tokio::runtime::Handle,
    dir: &Path,
) -> anyhow::Result<(tokio::net::TcpListener, String)> {
    let listener = rt.block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))?;
    let port = listener.local_addr()?.port();
    let token = controller::random_hex(16);
    let info = instance::InstanceInfo {
        port,
        token: token.clone(),
    };
    let path = instance::instance_path(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, instance::encode(&info))?;
    Ok((listener, token))
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&raw_args);
    let data_dir = flamix_core::store::data_dir();

    if let Some(path) = args.screenshot.as_deref() {
        let state = if args.demo {
            None
        } else {
            Some(persist::load(&data_dir)?)
        };
        return demo::screenshot(Path::new(path), args.light, state.as_ref());
    }

    // Hand our arguments to a running instance (unless this is the demo).
    if !args.demo {
        match try_forward(&data_dir, &raw_args) {
            Forward::Sent => return Ok(()),
            Forward::Stale => {
                let _ = std::fs::remove_file(instance::instance_path(&data_dir));
            }
            Forward::None => {}
        }
    }

    let state = persist::load(&data_dir)?;
    let minimize = args.minimized && state.settings.minimize_to_tray;

    let ui = AppWindow::new()?;
    let _controller = if args.demo {
        demo::setup(&ui)?;
        None
    } else {
        let rt = build_runtime()?;
        let (listener, token) = start_listener(&rt, &data_dir)?;
        let controller = Controller::new(&ui, data_dir, rt.clone(), state, args.import.clone());
        let handler = Arc::clone(&controller);
        rt.spawn(async move {
            instance::serve(listener, token, move |args| handler.handle_remote_args(args)).await;
        });
        Some(controller)
    };

    if minimize {
        ui.hide()?;
    }
    ui.run()?;
    Ok(())
}
