//! The Android library's link flags. Only a real link of the cdylib runs
//! them (`x build`, `cargo build --target aarch64-linux-android`); a `cargo
//! check` of the Android target compiles everything and links nothing, so
//! what is checked here is exactly what a check can't see.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    // A shared library links fine with a symbol nothing defines; the phone's
    // dlopen then refuses the whole library at NativeActivity.onCreate
    // ("cannot locate symbol ..."), which is how Luau's code generator
    // calling __clear_cache from the compiler's runtime library (absent from
    // a Rust link) first showed up: in a logcat, after a CI build and an
    // install. The linker can say it instead: every undefined symbol must
    // come from a library in NEEDED (libc, libm, libdl, liblog, libandroid
    // of the NDK sysroot the link runs against). Weak references (Rust's
    // std probes newer bionic entry points that way) stay allowed.
    println!("cargo:rustc-link-arg-cdylib=-Wl,--no-undefined");
}
