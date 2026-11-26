//! Opening a page in the browser from Studio's own windows (the account
//! popup, Help, the Explorer's class links, plugin prompts).

/// Opens `url` in the default browser: egui's own way on desktops
/// (webbrowser); on Android an ACTION_VIEW Intent from the Activity, which
/// also works from the worker threads the sign-in runs on.
pub fn open_url(ctx: &egui::Context, url: &str) {
    #[cfg(target_os = "android")]
    {
        match crate::android::jni::open_url(url) {
            Ok(()) => return,
            Err(e) => log::warn!("open {url}: {e}; trying egui's way"),
        }
    }
    ctx.open_url(egui::OpenUrl::new_tab(url));
}
