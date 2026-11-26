//! The Android app's folders, for `paths.rs` and `crash.rs`: the app's own
//! stand in for `~/.local/share` and friends (`Context.getFilesDir()`,
//! `getExternalFilesDir()`, `getCacheDir()`). The activity's entry point
//! (the studio's `android::run`) tells this module the folders with
//! [`init`] before anything reads a path; a process that never does (a
//! test binary on a device shell) gets `/data/local/tmp/rustudio`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The app's folders, decided once at start.
struct Dirs {
    /// `Context.getFilesDir()`: `/data/user/0/<package>/files`. Private,
    /// backed up, counts against internal storage.
    files: PathBuf,
    /// `Context.getExternalFilesDir(null)`: `/storage/emulated/0/Android/
    /// data/<package>/files`. Still the app's own (no permission needed) but
    /// on the shared volume, visible to file managers, and the right place
    /// for the Studio content download and the person's places.
    external: Option<PathBuf>,
    /// `Context.getCacheDir()`: `/data/user/0/<package>/cache` (the system
    /// may clear it).
    cache: PathBuf,
}

static DIRS: OnceLock<Dirs> = OnceLock::new();

/// Sets the folders: the activity's internal data path (`getFilesDir()`)
/// and its external one, if any (`getExternalFilesDir(null)`; it is used
/// only where it can be written). Once; later calls change nothing.
pub fn init(files: PathBuf, external: Option<PathBuf>) {
    let _ = DIRS.set(make(files, external));
}

fn make(files: PathBuf, external: Option<PathBuf>) -> Dirs {
    let external = external.filter(|p| std::fs::create_dir_all(p).is_ok() && is_writable(p));
    // getCacheDir() is files/../cache for every app; android-activity
    // doesn't hand it over, so it's derived rather than asked for.
    let cache = files.parent().map(|p| p.join("cache")).unwrap_or_else(|| files.join("cache"));
    let _ = std::fs::create_dir_all(&files);
    let _ = std::fs::create_dir_all(&cache);
    Dirs { files, external, cache }
}

fn dirs() -> &'static Dirs {
    DIRS.get_or_init(|| make(PathBuf::from("/data/local/tmp/rustudio"), None))
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".rustudio-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// The folder standing in for the home folder (`getFilesDir()`).
pub fn files_dir() -> PathBuf {
    dirs().files.clone()
}

/// Where big, user-visible data goes (`paths::data()`): the external files
/// folder when there is one, else the internal one.
pub fn data_dir() -> PathBuf {
    let d = dirs();
    d.external.clone().unwrap_or_else(|| d.files.clone())
}

/// Caches (`paths::cache()`): `getCacheDir()`.
pub fn cache_dir() -> PathBuf {
    dirs().cache.clone()
}

/// Settings and state (`paths::config()` / `paths::state()`): under the
/// internal files folder, so they are backed up with the app.
pub fn config_dir() -> PathBuf {
    dirs().files.join("config")
}

pub fn state_dir() -> PathBuf {
    dirs().files.join("state")
}

/// Screenshots (`paths::pictures()`): the app's external Pictures folder
/// (writing to the shared Pictures collection needs MediaStore).
pub fn pictures_dir() -> PathBuf {
    data_dir().join("Pictures")
}

/// Files unpacked from the APK's `assets/` (api-dump.json), where
/// `paths::resource` looks first.
pub fn resource_dir() -> PathBuf {
    dirs().files.join("resources")
}
