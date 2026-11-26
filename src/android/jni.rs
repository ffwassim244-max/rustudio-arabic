//! The Java side of the device services, through JNI (the `jni` crate on
//! the JavaVM android-activity registered): the Vibrator behind
//! HapticService, `Activity.setRequestedOrientation` behind
//! StarterGui.ScreenOrientation, the window insets behind GuiService /
//! ScreenGui.ScreenInsets and UserInputService.OnScreenKeyboardSize, and
//! ConnectivityManager for the content download. Every call is made on
//! whichever thread asks (the thread is attached if it wasn't) and a Java
//! exception comes back as an `Err`, never as a crash.

use std::sync::Mutex;

use jni::objects::{JObject, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};

pub type Result<T> = std::result::Result<T, String>;

fn vm() -> Result<JavaVM> {
    // jni's process-wide singleton is per jni-rs version: android-activity
    // registers the VM with its own copy of the crate, so the first call
    // here registers it with this one from the pointer AndroidApp carries.
    if let Ok(vm) = JavaVM::singleton() {
        return Ok(vm);
    }
    let app = super::app().ok_or("no AndroidApp yet")?;
    let ptr = app.vm_as_ptr();
    if ptr.is_null() {
        return Err("no JavaVM".into());
    }
    // SAFETY: the JavaVM NativeActivity was created in; it outlives the
    // process, and JavaVM::from_raw only stores the pointer.
    Ok(unsafe { JavaVM::from_raw(ptr.cast()) })
}

/// Runs `f` with this thread attached to the JVM (a local frame of its own;
/// a pending Java exception becomes the error). The exception is caught
/// and cleared here, and the error names it (`SecurityException: ...`):
/// left pending it would fail every JNI call made on this thread after
/// it — the editor's thread never returns to Java to throw it — and on
/// the Java main thread it would be thrown out of the Looper and end the
/// process.
pub fn with_env<T>(f: impl FnOnce(&mut Env) -> jni::errors::Result<T>) -> Result<T> {
    vm()?
        .attach_current_thread(|env| {
            let r = f(env);
            if env.exception_check() {
                if let Err(e) = env.exception_catch() {
                    return Err(e);
                }
            }
            r
        })
        .map_err(|e: jni::errors::Error| match e {
            jni::errors::Error::CaughtJavaException { name, msg, .. } => format!("{name}: {msg}"),
            e => e.to_string(),
        })
}

/// Runs `f` with the Activity (a borrowed reference to the one
/// android-activity keeps alive for the lifetime of `AndroidApp`).
pub fn with_activity<T>(f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>) -> Result<T> {
    let app = super::app().ok_or("no AndroidApp yet")?;
    let raw: jni::sys::jobject = app.activity_as_ptr().cast();
    if raw.is_null() {
        return Err("the Activity is gone".into());
    }
    // SAFETY: a valid global reference, held by AndroidApp for as long as
    // `app()` hands it out.
    unsafe { with_java_activity(raw, f) }
}

/// Runs `f` with the Java Activity `raw` refers to, on this thread.
///
/// # Safety
/// `raw` must be a live JNI reference (global, or local to this thread) to
/// an `android.app.Activity`; it is borrowed, never deleted.
pub unsafe fn with_java_activity<T>(raw: jni::sys::jobject, f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>) -> Result<T> {
    with_env(|env| {
        // SAFETY: the caller's live reference; Cast never deletes it.
        let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
        f(env, &activity)
    })
}

/// Runs `f` with the Activity of a raw `ANativeActivity`: the one the
/// framework passed to the library's entry point, before (or without)
/// android-activity's `AndroidApp` for it (src/android/intent.rs). Its own
/// JavaVM is used, so this works in a process where [`with_activity`]'s
/// handle isn't there yet.
///
/// # Safety
/// `activity` must be the live `ANativeActivity` (valid until its
/// `onDestroy` returns).
pub unsafe fn with_native_activity<T>(
    activity: *mut ndk_sys::ANativeActivity,
    f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>,
) -> Result<T> {
    if activity.is_null() {
        return Err("no ANativeActivity".into());
    }
    // SAFETY: the caller's live struct.
    let (vm, clazz) = unsafe { ((*activity).vm, (*activity).clazz) };
    if vm.is_null() || clazz.is_null() {
        return Err("ANativeActivity without a JavaVM or an Activity".into());
    }
    // SAFETY: the JavaVM NativeActivity was created in; JavaVM::from_raw
    // only stores the pointer.
    let vm = unsafe { JavaVM::from_raw(vm.cast()) };
    let clazz: jni::sys::jobject = clazz.cast();
    vm.attach_current_thread(|env| {
        // SAFETY: `clazz` is the framework's global reference to the
        // Activity, live as long as the ANativeActivity is.
        let activity = unsafe { env.as_cast_raw::<JObject>(&clazz)? };
        f(env, &activity)
    })
    .map_err(|e: jni::errors::Error| e.to_string())
}

/// `Build.VERSION.SDK_INT`.
pub fn sdk_int() -> i32 {
    super::AndroidApp::sdk_version()
}

/// The display's refresh rate in Hz (`Display.getRefreshRate`: what the
/// panel runs at now, 60 or 120 on a phone that switches; 0 when unknown).
pub fn display_refresh_rate() -> Result<f32> {
    with_activity(|env, activity| {
        // Context.getDisplay from API 30; the WindowManager's default
        // display before (deprecated there, still answered).
        let display = if sdk_int() >= 30 {
            env.call_method(activity, jni_str!("getDisplay"), jni_sig!("()Landroid/view/Display;"), &[])?.l()?
        } else {
            let wm = env.call_method(activity, jni_str!("getWindowManager"), jni_sig!("()Landroid/view/WindowManager;"), &[])?.l()?;
            if wm.is_null() {
                return Ok(0.0);
            }
            env.call_method(&wm, jni_str!("getDefaultDisplay"), jni_sig!("()Landroid/view/Display;"), &[])?.l()?
        };
        if display.is_null() {
            return Ok(0.0);
        }
        env.call_method(&display, jni_str!("getRefreshRate"), jni_sig!("()F"), &[])?.f()
    })
}

/// `context.getSystemService(name)` (null when the device has none).
pub(super) fn system_service<'l>(env: &mut Env<'l>, context: &JObject, name: &str) -> jni::errors::Result<JObject<'l>> {
    let name = env.new_string(name)?;
    env.call_method(
        context,
        jni_str!("getSystemService"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
        &[JValue::Object(&name)],
    )?
    .l()
}

// ---------------------------------------------------------------- haptics

/// What the device's vibrator can do, asked once.
#[derive(Clone, Copy, Debug, Default)]
pub struct VibratorInfo {
    pub present: bool,
    /// `Vibrator.hasAmplitudeControl()` (API 26+): the motor strength
    /// HapticService:SetMotor sets is honoured, not just on / off.
    pub amplitude: bool,
}

static VIBRATOR: Mutex<Option<VibratorInfo>> = Mutex::new(None);

pub fn vibrator_info() -> VibratorInfo {
    let mut cached = VIBRATOR.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = *cached {
        return v;
    }
    let info = with_activity(|env, activity| {
        let vib = system_service(env, activity, "vibrator")?;
        if vib.is_null() {
            return Ok(VibratorInfo::default());
        }
        let present = env.call_method(&vib, jni_str!("hasVibrator"), jni_sig!("()Z"), &[])?.z()?;
        let amplitude = present
            && sdk_int() >= 26
            && env.call_method(&vib, jni_str!("hasAmplitudeControl"), jni_sig!("()Z"), &[])?.z()?;
        Ok(VibratorInfo { present, amplitude })
    })
    .unwrap_or_else(|e| {
        log::warn!("vibrator: {e}");
        VibratorInfo::default()
    });
    *cached = Some(info);
    info
}

/// One vibration: `millis` long at `strength` 0..1 (`VibrationEffect
/// .createOneShot`, API 26+, amplitude 1..255; a strength of 0 cancels).
/// HapticService:SetMotor(Enum.UserInputType.Touch, Enum.VibrationMotor
/// .Large, s) keeps the motor on by renewing this each frame.
pub fn vibrate(millis: u32, strength: f32) -> Result<()> {
    if strength <= 0.0 || millis == 0 {
        return cancel_vibration();
    }
    let amplitude = ((strength.clamp(0.0, 1.0) * 255.0).round() as i32).clamp(1, 255);
    with_activity(|env, activity| {
        let vib = system_service(env, activity, "vibrator")?;
        if vib.is_null() {
            return Ok(());
        }
        if sdk_int() >= 26 {
            let effect = env.call_static_method(
                jni_str!("android/os/VibrationEffect"),
                jni_str!("createOneShot"),
                jni_sig!("(JI)Landroid/os/VibrationEffect;"),
                &[JValue::Long(i64::from(millis)), JValue::Int(amplitude)],
            )?;
            let effect = effect.l()?;
            env.call_method(&vib, jni_str!("vibrate"), jni_sig!("(Landroid/os/VibrationEffect;)V"), &[JValue::Object(&effect)])?;
        } else {
            env.call_method(&vib, jni_str!("vibrate"), jni_sig!("(J)V"), &[JValue::Long(i64::from(millis))])?;
        }
        Ok(())
    })
}

/// A pattern: alternating off / on lengths in ms with a strength per step
/// (`VibrationEffect.createWaveform(long[], int[], -1)`), for HapticEffect
/// presets with more than one pulse.
pub fn vibrate_pattern(steps: &[(u32, f32)]) -> Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    with_activity(|env, activity| {
        let vib = system_service(env, activity, "vibrator")?;
        if vib.is_null() {
            return Ok(());
        }
        let timings: Vec<i64> = steps.iter().map(|(ms, _)| i64::from(*ms)).collect();
        let amplitudes: Vec<i32> = steps
            .iter()
            .map(|(_, s)| if *s <= 0.0 { 0 } else { ((s.clamp(0.0, 1.0) * 255.0).round() as i32).clamp(1, 255) })
            .collect();
        let jt = env.new_long_array(timings.len())?;
        jt.set_region(env, 0, &timings)?;
        if sdk_int() >= 26 {
            let ja = env.new_int_array(amplitudes.len())?;
            ja.set_region(env, 0, &amplitudes)?;
            let effect = env
                .call_static_method(
                    jni_str!("android/os/VibrationEffect"),
                    jni_str!("createWaveform"),
                    jni_sig!("([J[II)Landroid/os/VibrationEffect;"),
                    &[JValue::Object(&jt), JValue::Object(&ja), JValue::Int(-1)],
                )?
                .l()?;
            env.call_method(&vib, jni_str!("vibrate"), jni_sig!("(Landroid/os/VibrationEffect;)V"), &[JValue::Object(&effect)])?;
        } else {
            env.call_method(&vib, jni_str!("vibrate"), jni_sig!("([JI)V"), &[JValue::Object(&jt), JValue::Int(-1)])?;
        }
        Ok(())
    })
}

pub fn cancel_vibration() -> Result<()> {
    with_activity(|env, activity| {
        let vib = system_service(env, activity, "vibrator")?;
        if !vib.is_null() {
            env.call_method(&vib, jni_str!("cancel"), jni_sig!("()V"), &[])?;
        }
        Ok(())
    })
}

// ------------------------------------------------------------ orientation

/// `ActivityInfo.SCREEN_ORIENTATION_*` for a Roblox `Enum.ScreenOrientation`
/// name (StarterGui.ScreenOrientation / PlayerGui.ScreenOrientation).
pub fn orientation_constant(name: &str) -> Option<i32> {
    Some(match name {
        "LandscapeLeft" => 0,         // SCREEN_ORIENTATION_LANDSCAPE
        "LandscapeRight" => 8,        // SCREEN_ORIENTATION_REVERSE_LANDSCAPE
        "LandscapeSensor" => 6,       // SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        "Portrait" => 1,              // SCREEN_ORIENTATION_PORTRAIT
        "Sensor" => 10,               // SCREEN_ORIENTATION_FULL_SENSOR
        _ => return None,
    })
}

/// `Activity.setRequestedOrientation(constant)`.
pub fn set_requested_orientation(constant: i32) -> Result<()> {
    with_activity(|env, activity| {
        env.call_method(activity, jni_str!("setRequestedOrientation"), jni_sig!("(I)V"), &[JValue::Int(constant)])?;
        Ok(())
    })
}

/// `Display.getRotation()` as the Roblox orientation the screen is in now:
/// "LandscapeLeft", "LandscapeRight", "Portrait" (PlayerGui
/// .CurrentScreenOrientation; a tablet held upside down counts as
/// Portrait too, as in Roblox).
pub fn current_orientation() -> Result<&'static str> {
    with_activity(|env, activity| {
        let display = if sdk_int() >= 30 {
            env.call_method(activity, jni_str!("getDisplay"), jni_sig!("()Landroid/view/Display;"), &[])?.l()?
        } else {
            let wm = system_service(env, activity, "window")?;
            env.call_method(&wm, jni_str!("getDefaultDisplay"), jni_sig!("()Landroid/view/Display;"), &[])?.l()?
        };
        if display.is_null() {
            return Ok("LandscapeLeft");
        }
        let rotation = env.call_method(&display, jni_str!("getRotation"), jni_sig!("()I"), &[])?.i()?;
        // Surface.ROTATION_0 = 0, _90 = 1, _180 = 2, _270 = 3. A phone's
        // natural orientation is portrait, so ROTATION_90 is landscape with
        // the top of the device to the left (Roblox's LandscapeLeft).
        let natural_portrait = {
            let rect = super::app().map(|a| a.content_rect());
            let (w, h) = rect.map(|r| (r.right - r.left, r.bottom - r.top)).unwrap_or((1, 1));
            matches!((rotation, w > h), (0, false) | (2, false) | (1, true) | (3, true))
        };
        Ok(match (rotation, natural_portrait) {
            (1, true) | (0, false) => "LandscapeLeft",
            (3, true) | (2, false) => "LandscapeRight",
            _ => "Portrait",
        })
    })
}

// ----------------------------------------------------------------- insets

/// The window's insets in physical pixels: what the status bar, navigation
/// bar and display cutout cover (`system`), and the soft keyboard's height
/// (`ime`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub ime_bottom: i32,
}

/// `getWindow().getDecorView().getRootWindowInsets()`, read through the
/// API 30 `getInsets(type)` when there is one, else the API 21 system
/// window insets plus the API 28 cutout.
pub fn window_insets() -> Result<Insets> {
    with_activity(|env, activity| {
        let window = env.call_method(activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?.l()?;
        let decor = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?.l()?;
        let insets = env
            .call_method(&decor, jni_str!("getRootWindowInsets"), jni_sig!("()Landroid/view/WindowInsets;"), &[])?
            .l()?;
        if insets.is_null() {
            return Ok(Insets::default());
        }
        let sdk = sdk_int();
        if sdk >= 30 {
            let bars = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("systemBars"), jni_sig!("()I"), &[])?.i()?;
            let cutout = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("displayCutout"), jni_sig!("()I"), &[])?.i()?;
            let ime = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("ime"), jni_sig!("()I"), &[])?.i()?;
            let system = read_insets(env, &insets, bars | cutout)?;
            let keyboard = read_insets(env, &insets, ime)?;
            Ok(Insets {
                left: system.0,
                top: system.1,
                right: system.2,
                bottom: system.3,
                ime_bottom: keyboard.3,
            })
        } else {
            let get = |env: &mut Env, name: &'static jni::strings::JNIStr| -> jni::errors::Result<i32> {
                env.call_method(&insets, name, jni_sig!("()I"), &[])?.i()
            };
            let mut out = Insets {
                left: get(env, jni_str!("getSystemWindowInsetLeft"))?,
                top: get(env, jni_str!("getSystemWindowInsetTop"))?,
                right: get(env, jni_str!("getSystemWindowInsetRight"))?,
                bottom: get(env, jni_str!("getSystemWindowInsetBottom"))?,
                ime_bottom: 0,
            };
            if sdk >= 28 {
                let cutout = env
                    .call_method(&insets, jni_str!("getDisplayCutout"), jni_sig!("()Landroid/view/DisplayCutout;"), &[])?
                    .l()?;
                if !cutout.is_null() {
                    let get = |env: &mut Env, name: &'static jni::strings::JNIStr| -> jni::errors::Result<i32> {
                        env.call_method(&cutout, name, jni_sig!("()I"), &[])?.i()
                    };
                    out.left = out.left.max(get(env, jni_str!("getSafeInsetLeft"))?);
                    out.top = out.top.max(get(env, jni_str!("getSafeInsetTop"))?);
                    out.right = out.right.max(get(env, jni_str!("getSafeInsetRight"))?);
                    out.bottom = out.bottom.max(get(env, jni_str!("getSafeInsetBottom"))?);
                }
            }
            // Before API 30 the keyboard's height isn't an inset of its own;
            // with adjustResize it shrinks the content rect instead, which
            // `super::storage` / the input code read from AndroidApp.
            Ok(out)
        }
    })
}

/// Where [`keyboard_height_later`]'s answer arrives (`None` until it has).
pub type KeyboardAnswer = std::sync::Arc<std::sync::Mutex<Option<Result<i32>>>>;

/// The soft keyboard's height in window pixels (0 while it is down),
/// read on the Java main thread (a View's insets belong to it): API 30+
/// `getInsets(Type.ime()).bottom`; before that the system window inset,
/// which counts the keyboard, less the stable inset (the navigation bar).
/// The editor's thread doesn't wait for it (the main thread may itself be
/// waiting on the editor's, as it does around a pause or a configuration
/// change): the answer lands in the returned slot when the main thread
/// gets to the question.
pub fn keyboard_height_later() -> KeyboardAnswer {
    let slot: KeyboardAnswer = std::sync::Arc::new(std::sync::Mutex::new(None));
    let answer = slot.clone();
    match super::app() {
        None => *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err("no AndroidApp yet".to_string())),
        Some(app) => app.run_on_java_main_thread(Box::new(move || {
            let height = read_keyboard_height();
            *answer.lock().unwrap_or_else(|e| e.into_inner()) = Some(height);
        })),
    }
    slot
}

fn read_keyboard_height() -> Result<i32> {
    with_activity(|env, activity| {
        let window = env.call_method(activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?.l()?;
        let decor = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?.l()?;
        let insets = env
            .call_method(&decor, jni_str!("getRootWindowInsets"), jni_sig!("()Landroid/view/WindowInsets;"), &[])?
            .l()?;
        if insets.is_null() {
            return Ok(0);
        }
        if sdk_int() >= 30 {
            let ime = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("ime"), jni_sig!("()I"), &[])?.i()?;
            Ok(read_insets(env, &insets, ime)?.3.max(0))
        } else {
            let system = env.call_method(&insets, jni_str!("getSystemWindowInsetBottom"), jni_sig!("()I"), &[])?.i()?;
            let stable = env.call_method(&insets, jni_str!("getStableInsetBottom"), jni_sig!("()I"), &[])?.i()?;
            Ok((system - stable).max(0))
        }
    })
}

/// `WindowInsets.getInsets(type)` → (left, top, right, bottom).
fn read_insets(env: &mut Env, insets: &JObject, kind: i32) -> jni::errors::Result<(i32, i32, i32, i32)> {
    let r = env
        .call_method(insets, jni_str!("getInsets"), jni_sig!("(I)Landroid/graphics/Insets;"), &[JValue::Int(kind)])?
        .l()?;
    let field = |env: &mut Env, name: &'static jni::strings::JNIStr| -> jni::errors::Result<i32> {
        env.get_field(&r, name, jni_sig!("I"))?.i()
    };
    Ok((field(env, jni_str!("left"))?, field(env, jni_str!("top"))?, field(env, jni_str!("right"))?, field(env, jni_str!("bottom"))?))
}

// ---------------------------------------------------------------- network

/// Whether the active network can reach the internet
/// (`NetworkCapabilities.NET_CAPABILITY_INTERNET` and `_VALIDATED`), for
/// deciding whether to start the Studio content download. None when the
/// question can't be asked (no ConnectivityManager, a Java error).
pub fn network_available() -> Option<bool> {
    with_activity(|env, activity| {
        let cm = system_service(env, activity, "connectivity")?;
        if cm.is_null() {
            return Ok(None);
        }
        let network = env.call_method(&cm, jni_str!("getActiveNetwork"), jni_sig!("()Landroid/net/Network;"), &[])?.l()?;
        if network.is_null() {
            return Ok(Some(false));
        }
        let caps = env
            .call_method(
                &cm,
                jni_str!("getNetworkCapabilities"),
                jni_sig!("(Landroid/net/Network;)Landroid/net/NetworkCapabilities;"),
                &[JValue::Object(&network)],
            )?
            .l()?;
        if caps.is_null() {
            return Ok(Some(false));
        }
        // NET_CAPABILITY_INTERNET = 12. (Not VALIDATED, 16: the system's
        // connectivity check fails on some networks — a blocked or slow
        // check server, a VPN, some vendors' builds — while the internet
        // works; a download given up on that would never start there.)
        let internet = env.call_method(&caps, jni_str!("hasCapability"), jni_sig!("(I)Z"), &[JValue::Int(12)])?.z()?;
        Ok(Some(internet))
    })
    .unwrap_or_else(|e| {
        log::warn!("connectivity: {e}");
        None
    })
}

// ------------------------------------------------------------------ files

/// `ContentResolver.openInputStream(Uri.parse(uri))` read to the end: a
/// file the system picker or a share sheet handed over as a `content:` URI.
pub fn read_content_uri(uri: &str) -> Result<Vec<u8>> {
    with_activity(|env, activity| content_bytes(env, activity, uri))
}

/// [`read_content_uri`] with the Activity at hand.
pub fn content_bytes(env: &mut Env, activity: &JObject, uri: &str) -> jni::errors::Result<Vec<u8>> {
    {
        let juri = env.new_string(uri)?;
        let uri = env
            .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&juri)])?
            .l()?;
        let resolver = env
            .call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[])?
            .l()?;
        let stream = env
            .call_method(
                &resolver,
                jni_str!("openInputStream"),
                jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"),
                &[JValue::Object(&uri)],
            )?
            .l()?;
        if stream.is_null() {
            return Err(jni::errors::Error::NullPtr("openInputStream"));
        }
        let mut out = Vec::new();
        let chunk = env.new_byte_array(1 << 16)?;
        let mut buf = vec![0i8; 1 << 16];
        loop {
            let n = env.call_method(&stream, jni_str!("read"), jni_sig!("([B)I"), &[JValue::Object(&chunk)])?.i()?;
            if n <= 0 {
                break;
            }
            let n = n as usize;
            chunk.get_region(env, 0, &mut buf[..n])?;
            out.extend(buf[..n].iter().map(|b| *b as u8));
        }
        env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[])?;
        Ok(out)
    }
}

/// The display name of a `content:` URI (`OpenableColumns.DISPLAY_NAME`
/// through `ContentResolver.query`), for the file's extension.
pub fn content_display_name(uri: &str) -> Result<Option<String>> {
    with_activity(|env, activity| display_name(env, activity, uri))
}

/// [`content_display_name`] with the Activity at hand.
pub fn display_name(env: &mut Env, activity: &JObject, uri: &str) -> jni::errors::Result<Option<String>> {
    {
        let juri = env.new_string(uri)?;
        let uri = env
            .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&juri)])?
            .l()?;
        let resolver = env
            .call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[])?
            .l()?;
        let column = env.new_string("_display_name")?;
        let projection = env.new_object_array(1, jni_str!("java/lang/String"), &column)?;
        let null = JObject::null();
        let cursor = env
            .call_method(
                &resolver,
                jni_str!("query"),
                jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
                &[JValue::Object(&uri), JValue::Object(&projection), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
            )?
            .l()?;
        if cursor.is_null() {
            return Ok(None);
        }
        let mut name = None;
        if env.call_method(&cursor, jni_str!("moveToFirst"), jni_sig!("()Z"), &[])?.z()? {
            let s = env.call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::Int(0)])?.l()?;
            if !s.is_null() {
                let s = env.as_cast::<jni::objects::JString>(&s)?;
                name = Some((*s).to_string());
            }
        }
        env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[])?;
        Ok(name)
    }
}

/// `Intent.getData().toString()` of the Intent this activity was started
/// with (a VIEW of a place file), if any.
pub fn launch_intent_uri() -> Result<Option<String>> {
    with_activity(intent_uri)
}

/// [`launch_intent_uri`] with the Activity at hand.
pub fn intent_uri(env: &mut Env, activity: &JObject) -> jni::errors::Result<Option<String>> {
    {
        let intent = env.call_method(activity, jni_str!("getIntent"), jni_sig!("()Landroid/content/Intent;"), &[])?.l()?;
        if intent.is_null() {
            return Ok(None);
        }
        let data = env.call_method(&intent, jni_str!("getData"), jni_sig!("()Landroid/net/Uri;"), &[])?.l()?;
        if data.is_null() {
            return Ok(None);
        }
        let s = env.call_method(&data, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        let s = env.as_cast::<jni::objects::JString>(&s)?;
        Ok(Some((*s).to_string()))
    }
}

/// `Activity.getTaskId()`.
pub fn task_id(env: &mut Env, activity: &JObject) -> jni::errors::Result<i32> {
    env.call_method(activity, jni_str!("getTaskId"), jni_sig!("()I"), &[])?.i()
}

/// `ActivityManager.moveTaskToFront(task, 0)` (REORDER_TASKS, a normal
/// permission, in manifest.yaml): the task holding the editor comes to the
/// front of the screen.
pub fn move_task_to_front(env: &mut Env, activity: &JObject, task: i32) -> jni::errors::Result<()> {
    let am = system_service(env, activity, "activity")?;
    if am.is_null() {
        return Err(jni::errors::Error::NullPtr("ActivityManager"));
    }
    env.call_method(&am, jni_str!("moveTaskToFront"), jni_sig!("(II)V"), &[JValue::Int(task), JValue::Int(0)])?;
    Ok(())
}

// ---------------------------------------------------------- main thread

/// Runs `f` on the Java main thread and waits up to `wait` for its answer.
/// Some framework objects (ClipboardManager, for one) want to be made on a
/// thread with a Looper, which the editor's thread isn't.
fn on_main_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static, wait: std::time::Duration) -> Result<T> {
    let app = super::app().ok_or("no AndroidApp yet")?;
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_java_main_thread(Box::new(move || {
        let _ = tx.send(f());
    }));
    rx.recv_timeout(wait).map_err(|_| "the main thread didn't answer in time".to_string())
}

// ---------------------------------------------------------- browser

/// `startActivity(new Intent(ACTION_VIEW, Uri.parse(url)))`: the default
/// browser (the Quick Sign-in confirm page, the documentation links).
pub fn open_url(url: &str) -> Result<()> {
    // As a Custom Tab first: the page opens inside the editor's own task,
    // in the default browser with that browser's cookies (a roblox.com
    // sign-in among them), and Back or the tab's close button lead
    // straight back to the editor, which is what a browser sign-in needs.
    // A browser without Custom Tabs ignores the extra and shows the page
    // as it would any link. If no activity takes it, the plain intent in
    // a task of its own is tried (and reports the failure).
    match view_url(url, true) {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("open url: as a Custom Tab: {e}; trying a plain view");
            view_url(url, false)
        }
    }
}

/// An `ACTION_VIEW` of `url`: as a Custom Tab in the editor's task
/// (`custom_tab`; the `EXTRA_SESSION` extra with no binder is what marks
/// one, no support library needed), or plainly with
/// `FLAG_ACTIVITY_NEW_TASK`, in a task of its own.
fn view_url(url: &str, custom_tab: bool) -> Result<()> {
    with_activity(|env, activity| {
        let jurl = env.new_string(url)?;
        let uri = env
            .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&jurl)])?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        if custom_tab {
            let extras = env.new_object(jni_str!("android/os/Bundle"), jni_sig!("()V"), &[])?;
            let key = env.new_string("android.support.customtabs.extra.SESSION")?;
            let null = JObject::null();
            env.call_method(
                &extras,
                jni_str!("putBinder"),
                jni_sig!("(Ljava/lang/String;Landroid/os/IBinder;)V"),
                &[JValue::Object(&key), JValue::Object(&null)],
            )?;
            env.call_method(&intent, jni_str!("putExtras"), jni_sig!("(Landroid/os/Bundle;)Landroid/content/Intent;"), &[JValue::Object(&extras)])?;
        } else {
            env.call_method(&intent, jni_str!("addFlags"), jni_sig!("(I)Landroid/content/Intent;"), &[JValue::Int(0x1000_0000)])?;
        }
        env.call_method(activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
        Ok(())
    })
}

// ---------------------------------------------------------- relaunch

/// `startActivity(getPackageManager().getLaunchIntentForPackage(getPackageName()))`
/// with `FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK`: a fresh copy
/// of the editor's activity, which the system hosts in a new process once
/// this one has exited (the caller exits right after; a process whose GPU
/// device is lost can draw nothing more). If the system doesn't, the
/// user's next tap on the icon starts it, and the report written before
/// says what happened and where the place was saved.
pub fn relaunch() -> Result<()> {
    with_activity(|env, activity| {
        let pm = env
            .call_method(activity, jni_str!("getPackageManager"), jni_sig!("()Landroid/content/pm/PackageManager;"), &[])?
            .l()?;
        let pkg = env.call_method(activity, jni_str!("getPackageName"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        let intent = env
            .call_method(
                &pm,
                jni_str!("getLaunchIntentForPackage"),
                jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&pkg)],
            )?
            .l()?;
        if intent.is_null() {
            return Err(jni::errors::Error::NullPtr("no launch intent for this package"));
        }
        // NEW_TASK | CLEAR_TASK | CLEAR_TOP
        let flags = 0x1000_0000 | 0x0000_8000 | 0x0400_0000;
        env.call_method(&intent, jni_str!("addFlags"), jni_sig!("(I)Landroid/content/Intent;"), &[JValue::Int(flags)])?;
        env.call_method(activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
        Ok(())
    })
}

// ---------------------------------------------------------- clipboard

/// A clipboard read's answer, once the main thread has made it (`None`: no
/// text on the clipboard, or none this window may read).
pub type ClipboardAnswer = std::sync::Arc<std::sync::Mutex<Option<Result<Option<String>>>>>;

/// Puts `text` on the system clipboard (`ClipboardManager.setPrimaryClip`),
/// so a copy in the editor pastes in other apps. Done here, on the calling
/// thread: since API 24 the ClipboardManager a Context hands out uses the
/// main thread's Handler itself, so any thread may ask it (the "Looper"
/// condition is Android 4's), and Android 10+'s focus rule is about the
/// app's window, not the thread. Should it fail all the same (a vendor
/// framework), the Java main thread is asked, without the editor's thread
/// waiting for it (the main thread may itself be waiting on the editor's
/// around a pause or a configuration change). The outcome goes to the log.
pub fn clipboard_set(text: &str) -> Result<()> {
    match write_clipboard(text) {
        Ok(()) => {
            log::info!("clipboard: {} chars put on the system clipboard", text.chars().count());
            return Ok(());
        }
        Err(e) => log::warn!("clipboard: put failed here ({e}); asking the main thread"),
    }
    let app = super::app().ok_or("no AndroidApp yet")?;
    let text = text.to_string();
    app.run_on_java_main_thread(Box::new(move || match write_clipboard(&text) {
        Ok(()) => log::info!("clipboard: {} chars put on the system clipboard (main thread)", text.chars().count()),
        Err(e) => log::warn!("clipboard: put failed: {e}"),
    }));
    Ok(())
}

/// The system clipboard's text, read now on the calling thread (see
/// [`clipboard_set`] for why any thread may): what is there at the moment
/// of the paste, not what was there when the field was last asked about.
/// `Ok(None)`: no text, or (Android 10+) this window hasn't the focus.
pub fn clipboard_get_now() -> Result<Option<String>> {
    let text = read_clipboard();
    match &text {
        Ok(Some(s)) => log::info!("clipboard: {} chars read from the system clipboard", s.chars().count()),
        Ok(None) => log::info!("clipboard: the system clipboard has no text (or this window lacks the focus)"),
        Err(e) => log::warn!("clipboard: read here failed: {e}"),
    }
    text
}

fn write_clipboard(text: &str) -> Result<()> {
    with_activity(|env, activity| {
        let cm = system_service(env, activity, "clipboard")?;
        if cm.is_null() {
            return Err(jni::errors::Error::NullPtr("ClipboardManager"));
        }
        let label = env.new_string("rustudio")?;
        let jtext = env.new_string(text)?;
        let clip = env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
                &[JValue::Object(&label), JValue::Object(&jtext)],
            )?
            .l()?;
        env.call_method(&cm, jni_str!("setPrimaryClip"), jni_sig!("(Landroid/content/ClipData;)V"), &[JValue::Object(&clip)])?;
        Ok(())
    })
}

/// Asks the Java main thread for the system clipboard's text (the first
/// item of the primary clip, coerced to text). The answer lands in the
/// returned slot when the main thread gets to it (a frame or two); the
/// editor's thread doesn't wait (see [`clipboard_set`]). Android 10+
/// answers only the app whose window has the input focus — the editor's,
/// once it is in front — and Android 12+ may show "rustudio pasted from
/// your clipboard" for a read.
pub fn clipboard_get_later() -> ClipboardAnswer {
    let slot: ClipboardAnswer = std::sync::Arc::new(std::sync::Mutex::new(None));
    let answer = slot.clone();
    match super::app() {
        None => *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err("no AndroidApp yet".to_string())),
        Some(app) => app.run_on_java_main_thread(Box::new(move || {
            let text = read_clipboard();
            match &text {
                Ok(Some(s)) => log::info!("clipboard: {} chars read from the system clipboard", s.chars().count()),
                Ok(None) => log::info!("clipboard: the system clipboard has no text (or this window lacks the focus)"),
                Err(e) => log::warn!("clipboard: read failed: {e}"),
            }
            *answer.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        })),
    }
    slot
}

fn read_clipboard() -> Result<Option<String>> {
    with_activity(|env, activity| {
        let cm = system_service(env, activity, "clipboard")?;
        if cm.is_null() {
            return Ok(None);
        }
        let clip = env.call_method(&cm, jni_str!("getPrimaryClip"), jni_sig!("()Landroid/content/ClipData;"), &[])?.l()?;
        if clip.is_null() {
            // Empty, or (Android 10+) this window hasn't the input focus,
            // which only the focused app's reads are allowed.
            return Ok(None);
        }
        let n = env.call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?.i()?;
        if n <= 0 {
            return Ok(None);
        }
        let item = env
            .call_method(&clip, jni_str!("getItemAt"), jni_sig!("(I)Landroid/content/ClipData$Item;"), &[JValue::Int(0)])?
            .l()?;
        let cs = env
            .call_method(&item, jni_str!("coerceToText"), jni_sig!("(Landroid/content/Context;)Ljava/lang/CharSequence;"), &[JValue::Object(activity)])?
            .l()?;
        if cs.is_null() {
            return Ok(None);
        }
        let s = env.call_method(&cs, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        let s = env.as_cast::<jni::objects::JString>(&s)?;
        let s = (*s).to_string();
        Ok((!s.is_empty()).then_some(s))
    })
}

// ---------------------------------------------------------- storage access

/// Whether the app may read shared storage by path: "All files access"
/// (`Environment.isExternalStorageManager`) on API 30+, the storage
/// permission below that.
pub fn has_storage_access() -> bool {
    if sdk_int() >= 30 {
        with_env(|env| {
            env.call_static_method(jni_str!("android/os/Environment"), jni_str!("isExternalStorageManager"), jni_sig!("()Z"), &[])?.z()
        })
        .unwrap_or(false)
    } else {
        has_permission("android.permission.READ_EXTERNAL_STORAGE")
    }
}

/// `checkSelfPermission(name) == PERMISSION_GRANTED`.
pub fn has_permission(name: &str) -> bool {
    with_activity(|env, activity| {
        let jname = env.new_string(name)?;
        let r = env
            .call_method(activity, jni_str!("checkSelfPermission"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&jname)])?
            .i()?;
        Ok(r == 0)
    })
    .unwrap_or(false)
}

/// Asks for shared-storage access: API 30+ opens the app's "All files
/// access" settings page (ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION);
/// below that, the runtime permission dialog. NativeActivity doesn't
/// forward the result; the caller asks [`has_storage_access`] again later
/// (storage.rs does while its picker is open).
pub fn request_storage_access() -> Result<()> {
    if sdk_int() >= 30 {
        return with_activity(|env, activity| {
            let pkg = env.call_method(activity, jni_str!("getPackageName"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
            let pkg = env.as_cast::<jni::objects::JString>(&pkg)?;
            let juri = env.new_string(format!("package:{}", &*pkg))?;
            let uri = env
                .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&juri)])?
                .l()?;
            let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
            let intent = env.new_object(
                jni_str!("android/content/Intent"),
                jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
            Ok(())
        });
    }
    on_main_thread(
        || {
            with_activity(|env, activity| {
                let read = env.new_string("android.permission.READ_EXTERNAL_STORAGE")?;
                let write = env.new_string("android.permission.WRITE_EXTERNAL_STORAGE")?;
                let names = env.new_object_array(2, jni_str!("java/lang/String"), &read)?;
                names.set_element(env, 1, &write)?;
                env.call_method(activity, jni_str!("requestPermissions"), jni_sig!("([Ljava/lang/String;I)V"), &[JValue::Object(&names), JValue::Int(1)])?;
                Ok(())
            })
        },
        std::time::Duration::from_millis(400),
    )?
}

// ---------------------------------------------------------- back

/// Sends the task to the background (`Activity.moveTaskToBack(true)`): the
/// Back button on the editor itself, with nothing open over it. The
/// activity stays alive and comes back where it was.
pub fn move_task_to_back() -> Result<()> {
    on_main_thread(
        || {
            with_activity(|env, activity| {
                env.call_method(activity, jni_str!("moveTaskToBack"), jni_sig!("(Z)Z"), &[JValue::Bool(true)])?;
                Ok(())
            })
        },
        std::time::Duration::from_millis(400),
    )?
}

// ---------------------------------------------------------- KBM & pointer capture

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareInput {
    pub keyboard: bool,
    pub mouse: bool,
}

/// Probes Android's InputDevice list for a non-virtual physical alphabetic
/// keyboard (SOURCE_KEYBOARD = 0x101, KEYBOARD_TYPE_ALPHABETIC = 2) and/or
/// mouse / touchpad (SOURCE_MOUSE = 0x2002, SOURCE_MOUSE_RELATIVE = 0x20004,
/// SOURCE_TOUCHPAD = 0x100008).
pub fn hardware_input_devices() -> Result<HardwareInput> {
    with_activity(|env, _| {
        let ids_obj = env
            .call_static_method(
                jni_str!("android/view/InputDevice"),
                jni_str!("getDeviceIds"),
                jni_sig!("()[I"),
                &[],
            )?
            .l()?;
        if ids_obj.is_null() {
            return Ok(HardwareInput::default());
        }
        let arr = unsafe { jni::objects::JIntArray::from_raw(env, ids_obj.into_raw()) };
        let len = arr.len(env)?;
        let mut raw = vec![0i32; len];
        arr.get_region(env, 0, &mut raw)?;
        let mut hw = HardwareInput::default();
        for id in raw {
            let dev = env
                .call_static_method(
                    jni_str!("android/view/InputDevice"),
                    jni_str!("getDevice"),
                    jni_sig!("(I)Landroid/view/InputDevice;"),
                    &[JValue::Int(id)],
                )?
                .l()?;
            if dev.is_null() {
                continue;
            }
            let is_virtual = env
                .call_method(&dev, jni_str!("isVirtual"), jni_sig!("()Z"), &[])?
                .z()?;
            if is_virtual {
                continue;
            }
            let src = env
                .call_method(&dev, jni_str!("getSources"), jni_sig!("()I"), &[])?
                .i()?;
            let ktype = env
                .call_method(&dev, jni_str!("getKeyboardType"), jni_sig!("()I"), &[])?
                .i()?;
            if (src & 0x101) == 0x101 && ktype == 2 {
                hw.keyboard = true;
            }
            if (src & 0x2002) == 0x2002
                || (src & 0x20004) == 0x20004
                || (src & 0x100008) == 0x100008
            {
                hw.mouse = true;
            }
        }
        Ok(hw)
    })
}

/// Requests or releases Android API 26+ pointer capture on the activity's decor view.
pub fn set_pointer_capture(capture: bool) {
    if sdk_int() < 26 {
        return;
    }
    let _ = on_main_thread(
        move || {
            with_activity(|env, activity| {
                let win = env
                    .call_method(activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?
                    .l()?;
                if win.is_null() {
                    return Ok(());
                }
                let decor = env
                    .call_method(&win, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?
                    .l()?;
                if decor.is_null() {
                    return Ok(());
                }
                if capture {
                    env.call_method(&decor, jni_str!("requestPointerCapture"), jni_sig!("()V"), &[])?;
                } else {
                    env.call_method(&decor, jni_str!("releasePointerCapture"), jni_sig!("()V"), &[])?;
                }
                Ok(())
            })
        },
        std::time::Duration::from_millis(200),
    );
}

/// Requests Android's "Install unknown apps" permission if needed and launches
/// the native PackageInstaller session for `apk_path`.
/// Returns `"installing"` or `"permission_requested"` on success.
pub fn install_apk(apk_path: &std::path::Path) -> Result<String> {
    let path_str = apk_path.to_str().ok_or("APK path is not valid UTF-8")?;
    with_activity(|env, activity| {
        let jpath = env.new_string(path_str)?;
        let res = env
            .call_method(
                activity,
                jni_str!("installApk"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&jpath)],
            )?
            .l()?;
        if res.is_null() {
            return Ok("error:null response".to_string());
        }
        let jstr = env.as_cast::<jni::objects::JString>(&res)?;
        let s = jstr.to_string();
        Ok(s)
    })
}

/// Relaunches the app in a fresh task and terminates the current process
/// via `RustudioActivity.restartApp()`.
pub fn restart_app() {
    let _ = with_activity(|env, activity| {
        env.call_method(activity, jni_str!("restartApp"), jni_sig!("()V"), &[])?;
        Ok(())
    });
}
