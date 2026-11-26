//! Native Android `InputConnection` bridge from `RustudioActivity.java` directly
//! into RusTudio's real egui text fields (Script Editor, Properties, etc.) with
//! zero fake EditText overlay.

use std::sync::Mutex;

use egui::{Event, Key, Modifiers};
use jni::objects::JValue;
use jni::{jni_sig, jni_str};

enum ImeOp {
    CommitText(String),
    Delete { before: i32, after: i32 },
    Key { code: i32, down: bool },
    Action(i32),
}

static QUEUE: Mutex<Vec<ImeOp>> = Mutex::new(Vec::new());

fn push_op(op: ImeOp) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push(op);
    }
    super::lifecycle::wake();
}

/// Raw JNI bridge called from `crates/rustudio-android/src/lib.rs`.
///
/// # Safety
/// Must be called by the JVM with valid `JNIEnv*` and `jstring`.
pub unsafe fn jni_commit_text(env: *mut std::ffi::c_void, text: *mut std::ffi::c_void) {
    if env.is_null() || text.is_null() {
        return;
    }
    let env_ptr = env as *mut jni::sys::JNIEnv;
    let jstr = text as jni::sys::jstring;
    unsafe {
        let Some(vtable) = (*env_ptr).as_ref() else { return };
        let len = (vtable.v1_1.GetStringLength)(env_ptr, jstr);
        if len <= 0 {
            return;
        }
        let get_chars = vtable.v1_1.GetStringChars;
        let rel_chars = vtable.v1_1.ReleaseStringChars;
        let raw = get_chars(env_ptr, jstr, std::ptr::null_mut());
        if !raw.is_null() {
            let slice = std::slice::from_raw_parts(raw, len as usize);
            let s = String::from_utf16_lossy(slice);
            rel_chars(env_ptr, jstr, raw);
            on_commit_text(s);
        }
    }
}

/// Called from JNI `RustudioActivity.nativeImeCommitText`.
pub fn on_commit_text(text: String) {
    if !text.is_empty() {
        push_op(ImeOp::CommitText(text));
    }
}

/// Called from JNI `RustudioActivity.nativeImeDelete`.
pub fn on_delete(before: i32, after: i32) {
    if before > 0 || after > 0 {
        push_op(ImeOp::Delete { before, after });
    }
}

/// Called from JNI `RustudioActivity.nativeImeKey`.
pub fn on_key(code: i32, down: bool) {
    push_op(ImeOp::Key { code, down });
}

/// Called from JNI `RustudioActivity.nativeImeAction`.
pub fn on_action(action: i32) {
    push_op(ImeOp::Action(action));
}

/// Asks `RustudioActivity.setNativeImeOpen(open)` to attach or detach the soft
/// keyboard directly to `ImeProxyView`'s `InputConnection`. Returns `true` if
/// the call succeeded (false falls back to stock `AndroidApp::show_soft_input`).
pub fn set_ime_open(open: bool) -> bool {
    let res = super::jni::with_activity(|env, activity| {
        env.call_method(
            activity,
            jni_str!("setNativeImeOpen"),
            jni_sig!("(Z)V"),
            &[JValue::Bool(open)],
        )?;
        Ok(())
    });
    match res {
        Ok(()) => true,
        Err(e) => {
            log::warn!("ime_bridge: setNativeImeOpen({open}) fallback: {e}");
            false
        }
    }
}

/// Resets `ImeProxyView.composingLen` and restarts input when focus switches
/// between text fields while the soft keyboard stays open.
pub fn reset_composing() {
    let _ = super::jni::with_activity(|env, activity| {
        env.call_method(
            activity,
            jni_str!("resetImeComposing"),
            jni_sig!("()V"),
            &[],
        )?;
        Ok(())
    });
}

fn push_key_tap(events: &mut Vec<Event>, key: Key) {
    events.push(Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    events.push(Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}

fn map_android_keycode(code: i32) -> Option<Key> {
    match code {
        67 => Some(Key::Backspace),   // KEYCODE_DEL
        112 => Some(Key::Delete),     // KEYCODE_FORWARD_DEL
        66 | 160 => Some(Key::Enter), // KEYCODE_ENTER / KEYCODE_NUMPAD_ENTER
        61 => Some(Key::Tab),         // KEYCODE_TAB
        21 => Some(Key::ArrowLeft),   // KEYCODE_DPAD_LEFT
        22 => Some(Key::ArrowRight),  // KEYCODE_DPAD_RIGHT
        19 => Some(Key::ArrowUp),     // KEYCODE_DPAD_UP
        20 => Some(Key::ArrowDown),   // KEYCODE_DPAD_DOWN
        122 => Some(Key::Home),       // KEYCODE_MOVE_HOME
        123 => Some(Key::End),        // KEYCODE_MOVE_END
        _ => None,
    }
}

/// Drains queued native Android IME operations into `egui::RawInput::events`
/// inside `App::raw_input_hook`.
pub fn before_frame(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let ops = {
        let Ok(mut q) = QUEUE.lock() else { return };
        if q.is_empty() {
            return;
        }
        std::mem::take(&mut *q)
    };

    for op in ops {
        match op {
            ImeOp::CommitText(s) => {
                // Gboard's Enter/Search key sends commitText("\n") on multi-line
                // InputConnections: emit Key::Enter so single-line search boxes
                // (Toolbox, Explorer, Properties) submit AND Script Editor gets
                // normal Enter + Luau auto-indent!
                if s == "\n" || s == "\r\n" || s == "\r" {
                    push_key_tap(&mut raw.events, Key::Enter);
                } else if s.contains('\n') || s.contains('\r') || s.chars().count() > 8 {
                    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
                    raw.events.push(Event::Paste(normalized));
                } else {
                    raw.events.push(Event::Text(s));
                }
            }
            ImeOp::Delete { before, after } => {
                for _ in 0..before.clamp(0, 64) {
                    push_key_tap(&mut raw.events, Key::Backspace);
                }
                for _ in 0..after.clamp(0, 64) {
                    push_key_tap(&mut raw.events, Key::Delete);
                }
            }
            ImeOp::Key { code, down } => {
                if let Some(key) = map_android_keycode(code) {
                    raw.events.push(Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed: down,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
            ImeOp::Action(id) => match id {
                1 => {
                    // Select All (Ctrl+A)
                    raw.events.push(Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    });
                    raw.events.push(Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    });
                }
                2 => raw.events.push(Event::Cut),
                3 => raw.events.push(Event::Copy),
                4 => {
                    let text = super::clipboard::text_now()
                        .or_else(super::clipboard::text)
                        .unwrap_or_default();
                    if !text.is_empty() {
                        raw.events.push(Event::Paste(text));
                    }
                }
                _ => {}
            },
        }
    }
    ctx.request_repaint();
}
