//! A second activity in the running process: "Open with rustudio" from a
//! file manager (or a share sheet) while the editor is up.
//!
//! The editor is one `NativeActivity` (manifest.yaml, `launchMode`
//! `standard`). Android starts a new instance of it for every VIEW intent
//! — on top of the editor's task, or inside the file manager's — and
//! `NativeActivity.onCreate` calls the library's entry point again, on the
//! Java main thread, in the same process. A second editor can't run there:
//! winit's event loop exists once per process (eframe fails with
//! `RecreationAttempt`), and before this module the entry point handed
//! the second activity to android-activity's glue anyway, whose
//! `android_main` ran [`super::run`] into that failure and left the whole
//! process — `reason=1 (EXIT_SELF) status=0` in `dumpsys activity
//! exit-info`, in the foreground, with the editor and its place gone: the
//! "crash" seen when a place was opened from a Files app while rustudio
//! was already running (and, with the activity's VIEW intent delivered
//! again at the next launch, the same place again).
//!
//! [`second_activity`] is what the entry point does instead with any
//! activity after the first: the file its intent names is copied in
//! ([`super::storage::import_uri_from`]) and queued for the editor's next
//! frame ([`super::storage::take_opened`], src/app.rs), the editor's task
//! is brought to the front (`ActivityManager.moveTaskToFront`, in case
//! the new activity sits in the file manager's task), and the new
//! activity finishes before it is ever shown (`Activity.finish()` inside
//! its `onCreate`: no `onStart`, no window). The glue never sees
//! it, so no second `android_main` runs and the editor's `onDestroy` hook
//! (src/android/lifecycle.rs) stays with the activity that owns the loop.
//! `onNewIntent` would be the ordinary way, but `NativeActivity` doesn't
//! pass it on to native code, and a `singleTop` / `singleTask` activity
//! swallows the intent there; a new instance per intent is the one path
//! that reaches this library without a Java class in the APK.

use std::ffi::c_void;

/// Called by the library's entry point (crates/rustudio-android) for every
/// activity. True when `activity` is not the first of the process: it has
/// been dealt with here and the entry point must not hand it to the glue.
///
/// # Safety
/// `activity` must be the `ANativeActivity` the framework passed to the
/// entry point, on the Java main thread, inside its `onCreate`.
pub unsafe fn second_activity(activity: *mut c_void) -> bool {
    let activity = activity.cast::<ndk_sys::ANativeActivity>();
    if !super::lifecycle::loop_owned() {
        return false;
    }
    log::info!("a second activity in the running process: handing its intent to the editor");
    // SAFETY: the framework's live struct for the whole of this call.
    let uri = unsafe {
        super::jni::with_native_activity(activity, |env, a| super::jni::intent_uri(env, a)).unwrap_or_else(|e| {
            log::warn!("second activity's intent: {e}");
            None
        })
    };
    match uri {
        Some(uri) => {
            log::info!("second activity's intent: {uri}");
            // SAFETY: as above.
            match unsafe { super::storage::import_uri_from(activity, &uri) } {
                Some(path) => super::storage::push_opened(path),
                None => log::warn!("second activity: nothing to open from {uri}"),
            }
        }
        None => log::info!("second activity without a file (the launcher): the editor comes to the front"),
    }
    // The editor's task to the front (a no-op when the new activity is on
    // top of it already), and the editor's loop woken for the open.
    let editor_task = super::jni::with_activity(|env, a| super::jni::task_id(env, a));
    // SAFETY: as above.
    let moved = unsafe {
        super::jni::with_native_activity(activity, |env, a| {
            let own = super::jni::task_id(env, a)?;
            match editor_task {
                Ok(t) if t != own => {
                    super::jni::move_task_to_front(env, a, t)?;
                    Ok(Some(t))
                }
                _ => Ok(None),
            }
        })
    };
    match moved {
        Ok(Some(t)) => log::info!("second activity: the editor's task {t} moved to the front"),
        Ok(None) => {}
        Err(e) => log::warn!("second activity: moving the editor's task to the front: {e}"),
    }
    super::lifecycle::wake();
    // `Activity.finish()` from inside onCreate: the activity is finishing
    // before the framework would start and show it, so no window of it
    // appears. (`ANativeActivity_finish` queues the same call for after
    // onCreate, when the window would already be up: the fallback.)
    // SAFETY: the live activity.
    let finished = unsafe {
        super::jni::with_native_activity(activity, |env, a| {
            env.call_method(a, jni::jni_str!("finish"), jni::jni_sig!("()V"), &[])?;
            Ok(())
        })
    };
    if let Err(e) = finished {
        log::warn!("second activity: finish: {e}");
        // SAFETY: the live activity; the call is queued on the main looper.
        unsafe { ndk_sys::ANativeActivity_finish(activity) };
    }
    true
}
