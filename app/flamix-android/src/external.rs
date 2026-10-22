//! Opens provider links (support, site, banner button) through the system app (Android only).
//!
//! Calls `online.flamix.FlamixLinks.open(Context, String)` (Kotlin). It has its own JNI handles
//! so the VPN bridge (`bridge.rs`) stays untouched. Same class-loader lookup as the bridge.

use jni::objects::{JClass, JObject, JString, JValue};
use jni::JavaVM;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::Mutex;

const LINKS_CLASS: &str = "online.flamix.FlamixLinks";

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

/// Opens `url` in the app that handles it. The URL must already be checked by the caller.
pub fn open(url: &str) -> Result<(), String> {
    let handles = (*HANDLES.lock().unwrap_or_else(|e| e.into_inner()))
        .ok_or_else(|| "JNI не инициализирован".to_string())?;
    // SAFETY: the pointers come from `android_main` for a live NativeActivity (see `init`).
    let vm = unsafe { JavaVM::from_raw(handles.vm as *mut jni::sys::JavaVM) }
        .map_err(|e| e.to_string())?;
    let mut guard = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let env: &mut jni::JNIEnv = &mut guard;
    // The activity reference is owned by NativeActivity; never delete it from here.
    let activity =
        ManuallyDrop::new(unsafe { JObject::from_raw(handles.activity as jni::sys::jobject) });
    let class = find_class(env, &activity).map_err(|e| e.to_string())?;
    let text: JString = env.new_string(url).map_err(|e| e.to_string())?;
    let text_obj: JObject = text.into();
    let result = env
        .call_static_method(
            &class,
            "open",
            "(Landroid/content/Context;Ljava/lang/String;)Z",
            &[JValue::Object(&activity), JValue::Object(&text_obj)],
        )
        .and_then(|v| v.z());
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err("Нет приложения для этой ссылки".into()),
        Err(e) => {
            let _ = env.exception_clear();
            Err(format!("open: {e}"))
        }
    }
}

/// Text of the system clipboard (empty when there is none). Blocking JNI call; run it off the UI
/// thread when that matters.
pub fn read_clipboard() -> Result<String, String> {
    let handles = (*HANDLES.lock().unwrap_or_else(|e| e.into_inner()))
        .ok_or_else(|| "JNI не инициализирован".to_string())?;
    // SAFETY: as in `open`.
    let vm = unsafe { JavaVM::from_raw(handles.vm as *mut jni::sys::JavaVM) }
        .map_err(|e| e.to_string())?;
    let mut guard = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let env: &mut jni::JNIEnv = &mut guard;
    let activity =
        ManuallyDrop::new(unsafe { JObject::from_raw(handles.activity as jni::sys::jobject) });
    let class = find_class(env, &activity).map_err(|e| e.to_string())?;
    let result = env
        .call_static_method(
            &class,
            "readClipboard",
            "(Landroid/content/Context;)Ljava/lang/String;",
            &[JValue::Object(&activity)],
        )
        .and_then(|v| v.l());
    match result {
        Ok(obj) => {
            let text = JString::from(obj);
            let value: String = env.get_string(&text).map_err(|e| e.to_string())?.into();
            Ok(value)
        }
        Err(e) => {
            let _ = env.exception_clear();
            Err(format!("readClipboard: {e}"))
        }
    }
}

fn find_class<'l>(
    env: &mut jni::JNIEnv<'l>,
    activity: &JObject,
) -> jni::errors::Result<JClass<'l>> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name: JString = env.new_string(LINKS_CLASS)?;
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
