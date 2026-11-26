//! The system bars out of the way while the editor runs (Android's
//! immersive mode): the navigation bar — Back / Home / Recents, or the
//! gesture pill, along the right edge in landscape — and the status bar.
//! The manifest's `Theme…Fullscreen` only takes the status bar; the
//! navigation bar stayed, a strip of the screen the editor never got, with
//! the Recents button a thumb's width from the 3D view's edge.
//!
//! The bars are hidden the way a game hides them: a swipe in from the edge
//! shows them for a moment, translucent over the editor (no relayout), and
//! they go again by themselves. API 30+: `WindowInsetsController.hide
//! (Type.systemBars())` with `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`;
//! before that the view flags `SYSTEM_UI_FLAG_HIDE_NAVIGATION |
//! FULLSCREEN | IMMERSIVE_STICKY` with their `LAYOUT_*` twins.
//!
//! Both are the decor view's business, so the calls run on the Java main
//! thread (`AndroidApp::run_on_java_main_thread`; the editor's thread
//! never waits for them). Asked for at start — before the window is
//! attached, which API 30+'s pending controller and the view's own flag
//! field both carry over to the attach — and again whenever the window
//! regains the focus or the soft keyboard goes down (the two moments
//! Android is known to leave the bars up: the focus one is Google's own
//! advice, the keyboard one API 26–29's sticky flags losing the
//! navigation bar after the IME).
//!
//! `RUSTUDIO_SYSTEM_BARS=1` (`debug.rustudio.env`) keeps the bars, as
//! before. The display cutout is left as it was: the window stays out of
//! the camera hole's side in landscape (`layoutInDisplayCutoutMode`
//! default), since the editor's chrome doesn't pad itself by the cutout
//! inset yet.

use std::sync::atomic::{AtomicBool, Ordering};

use jni::objects::JValue;
use jni::{jni_sig, jni_str};

/// `WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE` (API 30).
const BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE: i32 = 2;
/// `View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | FULLSCREEN | LAYOUT_STABLE |
/// LAYOUT_HIDE_NAVIGATION | LAYOUT_FULLSCREEN | IMMERSIVE_STICKY`
/// (0x2 | 0x4 | 0x100 | 0x200 | 0x400 | 0x1000), for API 26–29.
const LEGACY_IMMERSIVE_FLAGS: i32 = 0x1706;

/// Decided once: the switch that keeps the bars.
static KEEP: AtomicBool = AtomicBool::new(false);
static DECIDED: AtomicBool = AtomicBool::new(false);

fn wanted() -> bool {
    if !DECIDED.swap(true, Ordering::SeqCst) {
        let keep = std::env::var("RUSTUDIO_SYSTEM_BARS").is_ok_and(|v| v == "1");
        KEEP.store(keep, Ordering::SeqCst);
        if keep {
            log::info!("system bars kept (RUSTUDIO_SYSTEM_BARS=1)");
        }
    }
    !KEEP.load(Ordering::SeqCst)
}

/// Asks the main thread to hide the bars (nothing waits for it).
pub fn hide() {
    if !wanted() {
        return;
    }
    let Some(app) = super::app() else {
        log::warn!("system bars: no activity to ask");
        return;
    };
    app.run_on_java_main_thread(Box::new(|| {
        if let Err(e) = apply() {
            log::warn!("system bars: {e}");
        }
    }));
}

/// The frame's raw input: the window regaining the focus asks again.
pub fn before_frame(raw: &egui::RawInput) {
    if raw.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true))) {
        hide();
    }
}

/// The soft keyboard went down: asks again (the bars came up with it).
pub fn keyboard_closed() {
    hide();
}

/// On the Java main thread: the decor view's request.
fn apply() -> super::jni::Result<()> {
    super::jni::with_activity(|env, activity| {
        let window = env.call_method(activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?.l()?;
        if window.is_null() {
            return Ok(());
        }
        let decor = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?.l()?;
        if decor.is_null() {
            return Ok(());
        }
        if super::jni::sdk_int() >= 30 {
            let _ = env.call_method(
                &window,
                jni_str!("setDecorFitsSystemWindows"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(false)],
            );
            // (a pending controller before the view is attached: it replays
            // the behaviour and the hide at the attach)
            let controller = env
                .call_method(&decor, jni_str!("getWindowInsetsController"), jni_sig!("()Landroid/view/WindowInsetsController;"), &[])?
                .l()?;
            if controller.is_null() {
                return Ok(());
            }
            // Clear APPEARANCE_LIGHT_STATUS_BARS (0x08) | APPEARANCE_LIGHT_NAVIGATION_BARS (0x10)
            let _ = env.call_method(
                &controller,
                jni_str!("setSystemBarsAppearance"),
                jni_sig!("(II)V"),
                &[JValue::Int(0), JValue::Int(0x08 | 0x10)],
            );
            env.call_method(
                &controller,
                jni_str!("setSystemBarsBehavior"),
                jni_sig!("(I)V"),
                &[JValue::Int(BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE)],
            )?;
            let bars = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("systemBars"), jni_sig!("()I"), &[])?.i()?;
            env.call_method(&controller, jni_str!("hide"), jni_sig!("(I)V"), &[JValue::Int(bars)])?;
        } else {
            env.call_method(&decor, jni_str!("setSystemUiVisibility"), jni_sig!("(I)V"), &[JValue::Int(LEGACY_IMMERSIVE_FLAGS)])?;
        }
        Ok(())
    })
}
