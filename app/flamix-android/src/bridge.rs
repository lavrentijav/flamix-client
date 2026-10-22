//! Rust -> Kotlin bridge (M4): calls the static methods of `online.flamix.FlamixBridge`.
//!
//! The Kotlin side is the source of truth for VPN consent: `connect` hands the config to
//! `MainActivity.startVpn`, which asks `VpnService.prepare` and then starts `FlamixVpn`.
//!
//! Why the class is looked up through the activity's class loader: `android_main` runs on a
//! native thread, where `FindClass` only sees the system class loader and cannot find app classes.
//!
//! The JavaVM and activity pointers come from `android_main` (`AndroidApp::vm_as_ptr` /
//! `activity_as_ptr`) and are passed to [`init`]; this module itself has no Android-only code, so
//! the host build checks it.

use jni::objects::{JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::Mutex;

const BRIDGE_CLASS: &str = "online.flamix.FlamixBridge";

#[derive(Clone, Copy)]
struct Handles {
    vm: usize,
    activity: usize,
}

static HANDLES: Mutex<Option<Handles>> = Mutex::new(None);

/// Stores the JavaVM and the NativeActivity jobject. Called once from `android_main`.
pub fn init(vm: *mut c_void, activity: *mut c_void) {
    *HANDLES.lock().unwrap_or_else(|e| e.into_inner()) = Some(Handles {
        vm: vm as usize,
        activity: activity as usize,
    });
}

/// Asks Kotlin to start the VPN with `config_json` (consent dialog if needed).
pub fn connect(config_json: &str) -> Result<(), String> {
    let ok = call_bool("connect", "(Ljava/lang/String;)Z", Some(config_json))?;
    if ok {
        Ok(())
    } else {
        Err("Нет активного окна приложения, повторите попытку".into())
    }
}

/// Asks Kotlin to stop the VPN service.
pub fn disconnect() -> Result<(), String> {
    call_bool("disconnect", "()Z", None).map(|_| ())
}

/// Whether the native tunnel is currently forwarding (Kotlin `NativeTunnel.isRunning`).
pub fn is_running() -> bool {
    call_bool("isRunning", "()Z", None).unwrap_or(false)
}

/// Calls a static boolean method of the bridge class with an optional String argument.
fn call_bool(name: &str, sig: &str, arg: Option<&str>) -> Result<bool, String> {
    let handles = (*HANDLES.lock().unwrap_or_else(|e| e.into_inner()))
        .ok_or_else(|| "JNI не инициализирован".to_string())?;

    // SAFETY: the pointers were handed over by android_main from a live NativeActivity; the VM
    // lives for the whole process and the activity jobject for the activity's lifetime.
    let vm = unsafe { JavaVM::from_raw(handles.vm as *mut jni::sys::JavaVM) }
        .map_err(|e| e.to_string())?;
    let mut guard = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let env: &mut JNIEnv = &mut guard;

    // The activity reference is owned by NativeActivity; never delete it from here.
    let activity =
        ManuallyDrop::new(unsafe { JObject::from_raw(handles.activity as jni::sys::jobject) });
    let class = find_bridge_class(env, &activity).map_err(|e| e.to_string())?;

    let arg_obj: Option<JObject> = match arg {
        Some(text) => Some(env.new_string(text).map_err(|e| e.to_string())?.into()),
        None => None,
    };
    let args: Vec<JValue> = match &arg_obj {
        Some(obj) => vec![JValue::Object(obj)],
        None => Vec::new(),
    };
    let result = env
        .call_static_method(&class, name, sig, &args)
        .and_then(|v| v.z());
    match result {
        Ok(value) => Ok(value),
        Err(e) => {
            // A Kotlin exception must not stay pending on this thread.
            let _ = env.exception_clear();
            Err(format!("{name}: {e}"))
        }
    }
}

fn find_bridge_class<'l>(
    env: &mut JNIEnv<'l>,
    activity: &JObject,
) -> jni::errors::Result<JClass<'l>> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name: JString = env.new_string(BRIDGE_CLASS)?;
    let name_obj: JObject = name.into();
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name_obj)],
        )?
        .l()?;
    Ok(JClass::from(class))
}
