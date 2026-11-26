//! Android's performance hints (ADPF: `PerformanceHintManager`, API 31+)
//! for the thread that draws the frames.
//!
//! A phone's CPU governor sets the clocks from how busy a core looks, and
//! a frame loop that works for 8 ms then waits 25 ms for the display looks
//! idle: the big core it runs on stays at its lowest step, and the 8 ms
//! become 15 (then the frame misses its vsync, waits for the next, and
//! looks idler still). A hint session tells the governor the thread's
//! frame budget and, every frame, how long the work took; it raises the
//! clocks when the work runs close to the budget and lowers them when it
//! has slack — what Android's games do through the Frame Pacing library
//! and Unity / Unreal through their ADPF plug-ins (the vendors' figures:
//! 10–20 % off the CPU side of a frame, and a steadier one).
//!
//! The session is for the calling thread (eframe's: the editor, egui and
//! the renderer's encoding all run on it; the Vulkan driver's own threads
//! are its business). The same thread is also given Android's display
//! priority (`Process.setThreadPriority(THREAD_PRIORITY_DISPLAY)`, nice
//! −4, what the system gives an app's RenderThread): `android_main` runs
//! on a thread of android-activity's at the default priority, level with
//! the editor's own workers (texture decoding, downloads, the content
//! install), and a frame should not wait its turn behind them.
//! `RUSTUDIO_NO_PERF_HINT=1` leaves both off. Nothing here fails loudly:
//! a device without the service (API 30 and below, or a vendor that
//! returns no session) is logged once and that's that.

use std::sync::Mutex;
use std::time::Duration;

use jni::objects::{Global, JIntArray, JObject, JValue};
use jni::{jni_sig, jni_str};

use super::jni::{sdk_int, system_service, with_activity, with_env};

struct Session {
    session: Global<JObject<'static>>,
    target: Duration,
    /// Reports that failed in a row (the session is dropped after a few).
    failures: u32,
}

enum State {
    Untried,
    Paused,
    Off,
    On(Session),
}

static STATE: Mutex<State> = Mutex::new(State::Untried);

/// Whether hints are wanted (`RUSTUDIO_NO_PERF_HINT` turns them off).
pub fn wanted() -> bool {
    std::env::var_os("RUSTUDIO_NO_PERF_HINT").is_none()
}

/// This thread's kernel id, what the hint session is keyed by.
fn tid() -> i32 {
    // SAFETY: gettid takes nothing and can't fail.
    unsafe { libc::gettid() }
}

/// `Process.setThreadPriority(THREAD_PRIORITY_DISPLAY)` for this thread.
fn raise_priority() -> Result<(), String> {
    const THREAD_PRIORITY_DISPLAY: i32 = -4;
    with_env(|env| {
        env.call_static_method(jni_str!("android/os/Process"), jni_str!("setThreadPriority"), jni_sig!("(I)V"), &[JValue::Int(THREAD_PRIORITY_DISPLAY)])?;
        Ok(())
    })
}

fn open(target: Duration) -> Result<Session, String> {
    if sdk_int() < 31 {
        return Err(format!("API {} has no PerformanceHintManager (31+)", sdk_int()));
    }
    let tid = tid();
    let target_ns = target.as_nanos().clamp(1, i64::MAX as u128) as i64;
    let session = with_activity(|env, activity| {
        let manager = system_service(env, activity, "performance_hint")?;
        if manager.is_null() {
            return Ok(None);
        }
        let tids = JIntArray::new(env, 1)?;
        tids.set_region(env, 0, &[tid])?;
        let session = env
            .call_method(
                &manager,
                jni_str!("createHintSession"),
                jni_sig!("([IJ)Landroid/os/PerformanceHintManager$Session;"),
                &[JValue::Object(&tids), JValue::Long(target_ns)],
            )?
            .l()?;
        if session.is_null() {
            return Ok(None);
        }
        Ok(Some(env.new_global_ref(&session)?))
    })?;
    match session {
        Some(session) => Ok(Session { session, target, failures: 0 }),
        None => Err("no hint session from the device".into()),
    }
}

/// Once a frame, from the frame thread: how long the last frame's work
/// took (`actual`; the time on the CPU, not what it waited for the
/// display) against the budget it has (`target`: the display's period).
/// Opens the session on the first call.
pub fn frame(actual: Duration, target: Duration) {
    if !wanted() || target.is_zero() {
        return;
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(*state, State::Untried | State::Paused) {
        if matches!(*state, State::Untried) {
            match raise_priority() {
                Ok(()) => log::info!("frame thread {}: display priority", tid()),
                Err(e) => log::info!("frame thread {}: priority unchanged ({e})", tid()),
            }
        }
        *state = match open(target) {
            Ok(s) => {
                log::info!("perf hints: session for thread {} at {:.1} ms a frame", tid(), target.as_secs_f32() * 1000.0);
                State::On(s)
            }
            Err(e) => {
                log::info!("perf hints: off ({e})");
                State::Off
            }
        };
    }
    let State::On(s) = &mut *state else { return };
    let actual_ns = actual.as_nanos().clamp(1, i64::MAX as u128) as i64;
    let retarget = s.target.abs_diff(target) > Duration::from_micros(500);
    let target_ns = target.as_nanos().clamp(1, i64::MAX as u128) as i64;
    let result = with_env(|env| {
        if retarget {
            env.call_method(s.session.as_obj(), jni_str!("updateTargetWorkDuration"), jni_sig!("(J)V"), &[JValue::Long(target_ns)])?;
        }
        env.call_method(s.session.as_obj(), jni_str!("reportActualWorkDuration"), jni_sig!("(J)V"), &[JValue::Long(actual_ns)])?;
        Ok(())
    });
    let give_up = match result {
        Ok(()) => {
            s.failures = 0;
            if retarget {
                s.target = target;
            }
            false
        }
        Err(e) => {
            s.failures += 1;
            if s.failures >= 5 {
                log::warn!("perf hints: reports fail ({e}); off");
            }
            s.failures >= 5
        }
    };
    if give_up {
        *state = State::Off;
    }
}

/// Closes the session (the activity going away).
pub fn stop() {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let State::On(s) = &*state {
        let _ = with_env(|env| env.call_method(s.session.as_obj(), jni_str!("close"), jni_sig!("()V"), &[]).map(|_| ()));
        *state = State::Paused;
    }
}
