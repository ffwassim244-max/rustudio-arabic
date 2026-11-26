//! The Android app around the rustudio library.
//!
//! `NativeActivity` (declared in manifest.yaml, `android.app.lib_name` =
//! `rustudio_android`) dlopens this library and calls the entry point its
//! `android.app.func_name` meta-data names: [`rustudio_on_create`] here. It
//! hands the activity to android-activity's own `ANativeActivity_onCreate`
//! (the glue, which runs [`android_main`] on its own thread with the
//! `AndroidApp` handle) and then wraps the activity's `onDestroy`
//! (`rustudio::android::lifecycle`), which needs the `ANativeActivity`
//! itself: `AndroidApp` only gives out the Java `Activity` reference.
//! Everything after that is `rustudio::android::run`. An activity created
//! while that one runs (a VIEW intent from a file manager: a new instance
//! per intent) never reaches the glue: `rustudio::android::intent` hands
//! its file to the running editor and finishes it.
//!
//! On every other target this crate is empty (see Cargo.toml).

#[cfg(target_os = "android")]
use std::ffi::c_void;

#[cfg(target_os = "android")]
unsafe extern "C" {
    /// android-activity's glue entry (`native_activity::glue`), exported
    /// from this same library.
    fn ANativeActivity_onCreate(activity: *mut c_void, saved_state: *const c_void, saved_state_size: usize);
}

/// The activity's native entry point (`android.app.func_name` in
/// manifest.yaml): the same signature as `ANativeActivity_onCreate`.
///
/// # Safety
/// Called by the framework (NativeActivity.loadNativeCode) on the Java
/// main thread with the live `ANativeActivity` and its saved state.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustudio_on_create(activity: *mut c_void, saved_state: *const c_void, saved_state_size: usize) {
    // SAFETY: the framework's arguments, passed on unchanged; the glue has
    // filled `activity->callbacks` when it returns, and no callback can
    // run before this returns (they share the main thread).
    unsafe {
        if rustudio::android::intent::second_activity(activity) {
            return;
        }
        ANativeActivity_onCreate(activity, saved_state, saved_state_size);
        rustudio::android::lifecycle::hook(activity);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: rustudio::android::AndroidApp) {
    rustudio::android::run(app);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeImeCommitText(
    env: *mut c_void,
    _class: *mut c_void,
    text: *mut c_void,
) {
    unsafe { rustudio::android::ime_bridge::jni_commit_text(env, text) };
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeImeDelete(
    _env: *mut c_void,
    _class: *mut c_void,
    before: i32,
    after: i32,
) {
    rustudio::android::ime_bridge::on_delete(before, after);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeImeKey(
    _env: *mut c_void,
    _class: *mut c_void,
    key_code: i32,
    down: u8,
) {
    rustudio::android::ime_bridge::on_key(key_code, down != 0);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeImeAction(
    _env: *mut c_void,
    _class: *mut c_void,
    action: i32,
) {
    rustudio::android::ime_bridge::on_action(action);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeOnLifecycle(
    _env: *mut c_void,
    _class: *mut c_void,
    state: i32,
) {
    rustudio::android::lifecycle::on_java_lifecycle(state);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeOnTrimMemory(
    _env: *mut c_void,
    _class: *mut c_void,
    level: i32,
) {
    rustudio::android::lifecycle::on_trim_memory(level);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeOnSafDocumentCreated(
    env: *mut c_void,
    _class: *mut c_void,
    uri: *mut c_void,
) {
    unsafe { rustudio::android::crash_export::jni_on_saf_document_created(env, uri) };
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_com_logosforgestudios_rustudio_RustudioActivity_nativeOnSafDocumentOpened(
    env: *mut c_void,
    _class: *mut c_void,
    uri: *mut c_void,
) {
    unsafe { rustudio::android::storage::jni_on_saf_document_opened(env, uri) };
}
