//! Flamix desktop application entry point.
//!
//! Arguments: `--demo` (fake data, no core), `--screenshot <out.png>` (offscreen render),
//! `--light`, `--minimized`, and an optional positional deep link / subscription URL.
//! A second launch forwards its arguments to the running instance and exits.

mod controller;
mod demo;
mod flags;
mod instance;
mod links;
mod pages;
mod persist;
mod tray;
mod view;

use controller::Controller;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

slint::include_modules!();

#[derive(Default)]
struct Args {
    demo: bool,
    light: bool,
    /// `--page N` for `--screenshot`: which page to render (0 servers .. 5 about).
    page: i32,
    /// `--collapsed` for the demo screenshot: every subscription collapsed.
    collapsed: bool,
    /// `--overlay menu|hwid|hwid-error` for `--screenshot`.
    overlay: Option<String>,
    /// `--size WxH` for `--screenshot` (default 960x600).
    size: Option<(u32, u32)>,
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
            "--page" => args.page = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--demo" => args.demo = true,
            "--collapsed" => args.collapsed = true,
            "--overlay" => args.overlay = it.next().cloned(),
            "--size" => {
                args.size = it.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
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

/// The tray, kept in a slot so the sync timer and the close handler can reach it.
type TraySlot = Rc<RefCell<Option<tray::Tray>>>;

/// Construct the tray (if the platform/environment provides one) and marshal its commands
/// back onto the Slint event loop and the controller.
fn build_tray(ui: &AppWindow, controller: &Arc<Controller>) -> Option<tray::Tray> {
    let (connected, server, tun) = controller.connection_state();
    let ui = ui.as_weak();
    let ctl = Arc::clone(controller);
    tray::Tray::new(tray::TrayHandle {
        on_command: Box::new(move |command| {
            let ctl = Arc::clone(&ctl);
            let ui = ui.clone();
            let _ = slint::invoke_from_event_loop(move || match command {
                tray::TrayCommand::ToggleConnect => ctl.toggle_connect_public(),
                tray::TrayCommand::SetMode(tun) => ctl.set_mode_public(tun),
                tray::TrayCommand::Show => {
                    if let Some(ui) = ui.upgrade() {
                        let _ = ui.show();
                    }
                }
                tray::TrayCommand::Quit => ctl.quit(),
            });
        }),
        initial: tray::TrayState {
            connected,
            server,
            tun,
        },
        rt: Some(controller.runtime()),
    })
}

/// Create the tray and start a timer that keeps it in sync with the controller. On macOS
/// `tray-icon` must be created while the event loop is already running, so creation is
/// deferred by one event-loop turn there. Returns the tray slot (kept alive by the caller)
/// and the sync timer (dropping it stops the updates).
fn install_tray(
    ui: &AppWindow,
    controller: Arc<Controller>,
    minimize: bool,
) -> (TraySlot, slint::Timer) {
    let slot: TraySlot = Rc::new(RefCell::new(None));

    #[cfg(not(target_os = "macos"))]
    {
        let _ = minimize;
        *slot.borrow_mut() = build_tray(ui, &controller);
    }

    #[cfg(target_os = "macos")]
    {
        let slot = Rc::clone(&slot);
        let ui_weak = ui.as_weak();
        let ctl = Arc::clone(&controller);
        slint::Timer::single_shot(Duration::from_millis(0), move || {
            if let Some(ui) = ui_weak.upgrade() {
                let tray = build_tray(&ui, &ctl);
                let has_tray = tray.is_some();
                *slot.borrow_mut() = tray;
                if minimize && has_tray {
                    let _ = ui.hide();
                }
            }
        });
    }

    // Close-to-tray: hide when a tray exists and the setting is on, otherwise quit through
    // the regular shutdown path. Without a tray this mirrors the old close behaviour.
    {
        let slot = Rc::clone(&slot);
        let ctl = Arc::clone(&controller);
        ui.window().on_close_requested(move || {
            if slot.borrow().is_some() && ctl.with_state(|s| s.settings.minimize_to_tray) {
                slint::CloseRequestResponse::HideWindow
            } else {
                ctl.quit();
                slint::CloseRequestResponse::KeepWindowShown
            }
        });
    }

    let timer = slint::Timer::default();
    {
        let slot = Rc::clone(&slot);
        let ctl = Arc::clone(&controller);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            move || {
                if let Some(tray) = slot.borrow().as_ref() {
                    let (connected, server, tun) = ctl.connection_state();
                    tray.set_connected(connected, &server);
                    tray.set_mode(tun);
                }
            },
        );
    }
    (slot, timer)
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
        let (width, height) = args.size.unwrap_or((960, 600));
        let opts = demo::ShotOptions {
            light: args.light,
            page: args.page,
            collapsed: args.collapsed,
            overlay: args.overlay.clone(),
            width,
            height,
        };
        return demo::screenshot(Path::new(path), &opts, state.as_ref());
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
    let controller = if args.demo {
        demo::setup(&ui)?;
        pages::init(&ui, None);
        None
    } else {
        let rt = build_runtime()?;
        let (listener, token) = start_listener(&rt, &data_dir)?;
        let controller = Controller::new(&ui, data_dir, rt.clone(), state, args.import.clone());
        let handler = Arc::clone(&controller);
        rt.spawn(async move {
            instance::serve(listener, token, move |args| {
                handler.handle_remote_args(args)
            })
            .await;
        });
        pages::init(&ui, Some(&controller));
        Some(controller)
    };

    // Tray + close-to-tray (real mode only). `_tray_timer` must stay alive for the whole run.
    let (tray_slot, _tray_timer) = match &controller {
        Some(controller) => install_tray(&ui, Arc::clone(controller), minimize),
        None => (Rc::new(RefCell::new(None)), slint::Timer::default()),
    };

    // Start hidden only when a tray actually exists to bring the window back.
    if minimize && tray_slot.borrow().is_some() {
        ui.hide()?;
    }
    ui.run()?;
    Ok(())
}
