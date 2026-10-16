//! Demo data + wiring for the main window, and the offscreen `--screenshot` renderer.
//!
//! Search filtering is done in Rust: the master data lives in `RefCell`s and the filtered
//! `[SubscriptionItem]` / `[ServerItem]` models are rebuilt on every change (see `rebuild`).

use crate::flags;
use crate::{AppWindow, ConnState, ServerItem, SubscriptionItem};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// Master data of one subscription (the UI model is derived from this).
struct SubData {
    id: String,
    name: String,
    pinned: bool,
    collapsed: bool,
    updating: bool,
    has_traffic: bool,
    traffic_fraction: f32,
    traffic_text: String,
    expire_text: String,
    announce: String,
    error: String,
    servers: Vec<ServerItem>,
}

type Subs = Rc<RefCell<Vec<SubData>>>;
type Locals = Rc<RefCell<Vec<ServerItem>>>;

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn server(id: &str, iso: &str, name: &str, summary: &str, ping: i32) -> ServerItem {
    ServerItem {
        id: id.into(),
        name: name.into(),
        country: iso.into(),
        flag: flags::flag(iso),
        summary: summary.into(),
        ping,
        has_quota: false,
        quota_fraction: 0.0,
        quota_text: "".into(),
        selected: false,
    }
}

fn build_subscriptions(all: &[SubData], query: &str) -> Vec<SubscriptionItem> {
    let q = query.to_lowercase();
    all.iter()
        .filter_map(|s| {
            let name_match = q.is_empty() || s.name.to_lowercase().contains(&q);
            let servers: Vec<ServerItem> = if name_match {
                s.servers.clone()
            } else {
                s.servers
                    .iter()
                    .filter(|sv| sv.name.to_lowercase().contains(&q))
                    .cloned()
                    .collect()
            };
            if !name_match && servers.is_empty() {
                return None;
            }
            Some(SubscriptionItem {
                id: s.id.clone().into(),
                name: s.name.clone().into(),
                pinned: s.pinned,
                collapsed: s.collapsed,
                updating: s.updating,
                has_traffic: s.has_traffic,
                traffic_fraction: s.traffic_fraction,
                traffic_text: s.traffic_text.clone().into(),
                expire_text: s.expire_text.clone().into(),
                announce: s.announce.clone().into(),
                error: s.error.clone().into(),
                servers: model(servers),
            })
        })
        .collect()
}

fn rebuild(ui: &AppWindow, subs: &Subs, locals: &Locals) {
    let query = ui.get_search_text().to_string();
    ui.set_subscriptions(model(build_subscriptions(&subs.borrow(), &query)));
    ui.set_local_servers(model(locals.borrow().clone()));
}

/// Name + ISO code of the server currently flagged `selected` (if any).
fn selected_server(subs: &Subs, locals: &Locals) -> Option<(String, String)> {
    for sub in subs.borrow().iter() {
        for sv in sub.servers.iter() {
            if sv.selected {
                return Some((sv.name.to_string(), sv.country.to_string()));
            }
        }
    }
    for sv in locals.borrow().iter() {
        if sv.selected {
            return Some((sv.name.to_string(), sv.country.to_string()));
        }
    }
    None
}

fn select(ui: &AppWindow, subs: &Subs, locals: &Locals, id: &str) {
    let mut found: Option<(String, String)> = None;
    for sub in subs.borrow_mut().iter_mut() {
        for sv in sub.servers.iter_mut() {
            sv.selected = sv.id == id;
            if sv.selected {
                found = Some((sv.name.to_string(), sv.country.to_string()));
            }
        }
    }
    for sv in locals.borrow_mut().iter_mut() {
        sv.selected = sv.id == id;
        if sv.selected {
            found = Some((sv.name.to_string(), sv.country.to_string()));
        }
    }
    if let Some((name, iso)) = found {
        ui.set_selected_name(name.into());
        ui.set_selected_flag(flags::flag(&iso));
        ui.set_ping_result("".into());
    }
    rebuild(ui, subs, locals);
}

fn build_demo_data() -> (Vec<SubData>, Vec<ServerItem>) {
    let mut premium = vec![
        server("de-1", "DE", "Frankfurt · Reality", "vless / tcp / reality", 42),
        server("de-2", "DE", "Берлин · WebSocket", "vless / ws / tls", 88),
        server("nl-1", "NL", "Amsterdam · gRPC", "vless / grpc / tls", 121),
        server("ru-1", "RU", "Москва · WS", "vmess / ws / tls", 15),
        server("fi-1", "FI", "Helsinki · xhttp", "vless / xhttp / tls", 214),
    ];
    // One server with its own quota.
    premium[3].has_quota = true;
    premium[3].quota_fraction = 0.38;
    premium[3].quota_text = "осталось 12.3 GB".into();
    premium[3].selected = true;

    let basic = vec![
        server("us-1", "US", "New York", "trojan / tcp / tls", 321),
        server("ru-2", "RU", "Санкт-Петербург", "vmess / ws / tls", -1),
        server("sg-1", "SG", "Singapore", "hysteria2", -2),
    ];

    let subs = vec![
        SubData {
            id: "sub-premium".into(),
            name: "Flamix Premium".into(),
            pinned: true,
            collapsed: false,
            updating: false,
            has_traffic: true,
            traffic_fraction: 0.15,
            traffic_text: "15 GB / 100 GB".into(),
            expire_text: "до 01.01.2027".into(),
            announce: "Скорость удвоена до 1 Гбит/с".into(),
            error: "".into(),
            servers: premium,
        },
        SubData {
            id: "sub-basic".into(),
            name: "Базовый тариф".into(),
            pinned: false,
            collapsed: true,
            updating: false,
            has_traffic: true,
            traffic_fraction: 0.0,
            traffic_text: "∞ / ∞".into(),
            expire_text: "".into(),
            announce: "".into(),
            error: "".into(),
            servers: basic,
        },
    ];

    let mut local = server("local-1", "RU", "Локальный сервер", "vless / tcp / tls", 7);
    local.selected = false;
    (subs, vec![local])
}

/// Fill the window with demo data and wire all callbacks (temporary until the controller exists).
pub fn setup(ui: &AppWindow) -> anyhow::Result<()> {
    let (subs_vec, locals_vec) = build_demo_data();
    let subs: Subs = Rc::new(RefCell::new(subs_vec));
    let locals: Locals = Rc::new(RefCell::new(locals_vec));

    ui.set_state(ConnState::Disconnected);
    ui.set_status_text("Не подключено".into());
    ui.set_speed_text("↑ 0 KB/s   ↓ 0 KB/s".into());
    rebuild(ui, &subs, &locals);
    // The connection panel must mirror the server flagged selected in the demo data.
    if let Some((name, iso)) = selected_server(&subs, &locals) {
        ui.set_selected_name(name.into());
        ui.set_selected_flag(flags::flag(&iso));
    }

    {
        let w = ui.as_weak();
        ui.on_toggle_connect(move || {
            let Some(ui) = w.upgrade() else { return };
            match ui.get_state() {
                ConnState::Connected | ConnState::Error | ConnState::Connecting => {
                    ui.set_state(ConnState::Disconnected);
                    ui.set_status_text("Не подключено".into());
                }
                ConnState::Disconnected => {
                    ui.set_state(ConnState::Connecting);
                    ui.set_status_text("Подключение…".into());
                    let w2 = ui.as_weak();
                    slint::Timer::single_shot(Duration::from_millis(1500), move || {
                        if let Some(ui) = w2.upgrade() {
                            ui.set_state(ConnState::Connected);
                            ui.set_status_text("Подключено".into());
                            ui.set_speed_text("↑ 1.2 MB/s   ↓ 3.4 MB/s".into());
                        }
                    });
                }
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_select_server(move |id| {
            if let Some(ui) = w.upgrade() {
                select(&ui, &s, &l, &id);
            }
        });
    }
    ui.on_open_server(|id| tracing::debug!(server = %id, "open server details"));
    {
        let w = ui.as_weak();
        ui.on_ping_selected(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_ping_result(" · 42 ms".into());
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_ping_all(move || {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    for sv in sub.servers.iter_mut() {
                        if sv.id == "ru-2" {
                            sv.ping = -1;
                        } else if sv.ping == -2 {
                            sv.ping = 60 + (sv.name.len() as i32 * 37) % 350;
                        }
                    }
                }
                for sv in l.borrow_mut().iter_mut() {
                    if sv.ping == -2 {
                        sv.ping = 12;
                    }
                }
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_update_subscription(move |id| {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    if sub.id.as_str() == id.as_str() {
                        sub.updating = true;
                    }
                }
                rebuild(&ui, &s, &l);
                let (w2, s2, l2) = (ui.as_weak(), s.clone(), l.clone());
                slint::Timer::single_shot(Duration::from_millis(1200), move || {
                    for sub in s2.borrow_mut().iter_mut() {
                        sub.updating = false;
                    }
                    if let Some(ui) = w2.upgrade() {
                        rebuild(&ui, &s2, &l2);
                    }
                });
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_update_all(move || {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    sub.updating = true;
                }
                rebuild(&ui, &s, &l);
                let (w2, s2, l2) = (ui.as_weak(), s.clone(), l.clone());
                slint::Timer::single_shot(Duration::from_millis(1200), move || {
                    for sub in s2.borrow_mut().iter_mut() {
                        sub.updating = false;
                    }
                    if let Some(ui) = w2.upgrade() {
                        rebuild(&ui, &s2, &l2);
                    }
                });
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_toggle_pin(move |id| {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    if sub.id.as_str() == id.as_str() {
                        sub.pinned = !sub.pinned;
                    }
                }
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_toggle_collapse(move |id| {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    if sub.id.as_str() == id.as_str() {
                        sub.collapsed = !sub.collapsed;
                    }
                }
                rebuild(&ui, &s, &l);
            }
        });
    }
    ui.on_subscription_settings(|id| tracing::debug!(subscription = %id, "subscription settings"));
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_delete_server(move |id| {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    sub.servers.retain(|sv| sv.id != id);
                }
                l.borrow_mut().retain(|sv| sv.id != id);
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_delete_subscription(move |id| {
            if let Some(ui) = w.upgrade() {
                s.borrow_mut().retain(|sub| sub.id.as_str() != id.as_str());
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_import_clipboard(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_status_text("Буфер обмена пуст".into());
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_sort_by_ping(move || {
            if let Some(ui) = w.upgrade() {
                for sub in s.borrow_mut().iter_mut() {
                    sub.servers.sort_by_key(|sv| match sv.ping {
                        p if p < 0 => i32::MAX,
                        p => p,
                    });
                }
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_mode_changed(move |tun| {
            if let Some(ui) = w.upgrade() {
                ui.set_tun_mode(tun);
            }
        });
    }
    {
        let (w, s, l) = (ui.as_weak(), subs.clone(), locals.clone());
        ui.on_search_changed(move |_q| {
            if let Some(ui) = w.upgrade() {
                rebuild(&ui, &s, &l);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_toggle_edit(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_edit_mode(!ui.get_edit_mode());
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_add_subscription(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_page(1);
            }
        });
    }

    Ok(())
}

/// Render the window offscreen (software renderer) and save it as a PNG.
pub fn screenshot(path: &Path, light: bool) -> anyhow::Result<()> {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};

    struct ShotPlatform {
        window: Rc<MinimalSoftwareWindow>,
    }
    impl slint::platform::Platform for ShotPlatform {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.window.clone())
        }
    }

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(ShotPlatform {
        window: window.clone(),
    }))
    .map_err(|e| anyhow::anyhow!("set_platform: {e}"))?;

    let ui = AppWindow::new()?;
    setup(&ui)?;

    // The build host has no system fonts; use the bundled DejaVu family for the shot.
    ui.set_font_family("DejaVu Sans".into());
    ui.set_dark(!light);
    ui.global::<crate::Theme>().set_dark(!light);
    ui.set_page(0);
    ui.set_state(ConnState::Connected);
    ui.set_status_text("Подключено".into());
    ui.set_speed_text("↑ 1.2 MB/s   ↓ 3.4 MB/s".into());
    ui.set_ping_result(" · 42 ms".into());

    let width = 960u32;
    let height = 600u32;
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(width, height));
    window.request_redraw();

    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    window.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), width as usize);
    });

    write_png(path, &buffer)
}

fn write_png(path: &Path, buffer: &slint::SharedPixelBuffer<slint::Rgb8Pixel>) -> anyhow::Result<()> {
    let (width, height) = (buffer.width(), buffer.height());
    let mut data = Vec::with_capacity((width * height * 3) as usize);
    for px in buffer.as_slice() {
        data.push(px.r);
        data.push(px.g);
        data.push(px.b);
    }

    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&data)?;
    Ok(())
}
