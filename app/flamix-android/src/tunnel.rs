//! VPN data path (milestone M3): reads the `VpnService` TUN fd and forwards it to the core.
//!
//! The stock sing-box binary has no `file_descriptor` option (it is only injected by the
//! gomobile `libbox` platform interface; `sing-box check` rejects it as an unknown field), so the
//! fd cannot be handed to the core at all. Instead the core stays in the M2 proxy-only shape
//! (mixed inbound on `127.0.0.1`) and this module runs `tun2proxy` on the fd inside the app
//! process: IP packets from the TUN are turned into TCP/UDP sessions and sent to the core's
//! SOCKS5 inbound. Virtual DNS maps DNS answers to fake IPs so the core still sees domain names.
//!
//! JNI surface: `online.flamix.NativeTunnel` (`start`, `stop`, `isRunning`, `lastError`).

use jni::objects::JClass;
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use std::sync::Mutex;
use std::thread::JoinHandle;
use tun2proxy::{ArgDns, Args, CancellationToken, ProxyParameters};

struct Running {
    token: CancellationToken,
    thread: JoinHandle<()>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn set_error(msg: String) {
    crate::applog::log("E", "tunnel", &msg);
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = msg;
}

/// Builds the tun2proxy arguments for an already established TUN fd (not owned: Kotlin closes it).
fn build_args(fd: i32, socks_port: u16) -> Result<Args, String> {
    let proxy = ProxyParameters::try_from(format!("socks5://127.0.0.1:{socks_port}").as_str())
        .map_err(|e| format!("proxy url: {e}"))?;
    let mut args = Args::default();
    args.proxy(proxy)
        .tun_fd(Some(fd))
        .close_fd_on_drop(false)
        .dns(ArgDns::Virtual)
        .ipv6_enabled(false)
        .setup(false);
    Ok(args)
}

/// Starts forwarding on a background thread. Errors if already running.
pub fn start(fd: i32, socks_port: u16, mtu: u16) -> Result<(), String> {
    let mut guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = guard.as_ref() {
        if !r.thread.is_finished() {
            return Err("tunnel already running".into());
        }
    }
    set_error(String::new());
    let args = build_args(fd, socks_port)?;
    let token = CancellationToken::new();
    let child_token = token.clone();
    let thread = std::thread::Builder::new()
        .name("tun2proxy".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => return set_error(format!("runtime: {e}")),
            };
            crate::applog::log("I", "tunnel", "tun2proxy started");
            let res = rt.block_on(tun2proxy::general_run_async(args, mtu, false, child_token));
            match res {
                Ok(_) => crate::applog::log("I", "tunnel", "tun2proxy exited normally"),
                Err(e) => set_error(format!("tun2proxy: {e}")),
            }
        })
        .map_err(|e| format!("thread: {e}"))?;
    *guard = Some(Running { token, thread });
    Ok(())
}

/// Cancels the forwarder and waits for the thread. No-op when not running.
pub fn stop() {
    let running = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(r) = running {
        crate::applog::log("I", "tunnel", "stop requested");
        r.token.cancel();
        let _ = r.thread.join();
    }
}

pub fn is_running() -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|r| !r.thread.is_finished())
}

/// `external fun start(fd: Int, socksPort: Int, mtu: Int): Boolean`
#[no_mangle]
pub extern "system" fn Java_online_flamix_NativeTunnel_start(
    _env: JNIEnv,
    _class: JClass,
    fd: jint,
    socks_port: jint,
    mtu: jint,
) -> jboolean {
    let (Ok(port), Ok(mtu)) = (u16::try_from(socks_port), u16::try_from(mtu)) else {
        set_error("port/mtu out of range".into());
        return JNI_FALSE;
    };
    match start(fd, port, mtu) {
        Ok(()) => JNI_TRUE,
        Err(e) => {
            set_error(e);
            JNI_FALSE
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_online_flamix_NativeTunnel_stop(_env: JNIEnv, _class: JClass) {
    stop();
}

#[no_mangle]
pub extern "system" fn Java_online_flamix_NativeTunnel_isRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if is_running() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_online_flamix_NativeTunnel_lastError(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let msg = LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone();
    env.new_string(msg)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_point_at_local_socks_with_borrowed_fd() {
        let a = build_args(42, 10808).unwrap();
        assert_eq!(a.tun_fd, Some(42));
        assert_eq!(a.close_fd_on_drop, Some(false));
        assert!(!a.setup);
        assert!(matches!(a.dns, ArgDns::Virtual));
        assert!(format!("{:?}", a.proxy).contains("10808"));
    }
}
