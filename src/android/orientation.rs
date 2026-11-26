//! Screen orientation control for Android (`Activity.setRequestedOrientation`).
//!
//! Used by the Script Editor so the user can toggle between Portrait and
//! Landscape while writing Luau, while the rest of the Studio (3D viewport,
//! Playtest, etc.) automatically stays locked to Landscape.

#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicI8, Ordering};

/// `android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE` (API 9).
#[cfg(target_os = "android")]
pub const SCREEN_ORIENTATION_SENSOR_LANDSCAPE: i32 = 6;

/// `android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT` (API 9).
#[cfg(target_os = "android")]
pub const SCREEN_ORIENTATION_SENSOR_PORTRAIT: i32 = 7;

/// Last requested orientation (`-1` = unset, `0` = landscape, `1` = portrait),
/// used to avoid redundant JNI calls across frames.
#[cfg(target_os = "android")]
static LAST_PORTRAIT: AtomicI8 = AtomicI8::new(-1);

/// Requests Portrait (`true`) or Landscape (`false`) orientation for the Android
/// Activity via `Activity.setRequestedOrientation`.
///
/// Uses `SCREEN_ORIENTATION_SENSOR_PORTRAIT` (7) when `portrait` is `true` and
/// `SCREEN_ORIENTATION_SENSOR_LANDSCAPE` (6) when `portrait` is `false`.
/// Clean no-op on non-Android targets.
pub fn set_portrait(portrait: bool) {
    #[cfg(target_os = "android")]
    {
        let desired = if portrait { 1 } else { 0 };
        if LAST_PORTRAIT.load(Ordering::Relaxed) == desired {
            return;
        }
        let constant = if portrait {
            SCREEN_ORIENTATION_SENSOR_PORTRAIT
        } else {
            SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        };
        match super::jni::set_requested_orientation(constant) {
            Ok(()) => {
                LAST_PORTRAIT.store(desired, Ordering::Relaxed);
                super::system_bars::hide();
            }
            Err(e) => {
                log::warn!("orientation: setRequestedOrientation({constant}) failed: {e}");
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = portrait;
    }
}

/// Returns whether Portrait was most recently requested (always `false` on
/// non-Android targets or before any request has been made).
pub fn is_portrait_requested() -> bool {
    #[cfg(target_os = "android")]
    {
        LAST_PORTRAIT.load(Ordering::Relaxed) == 1
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}
