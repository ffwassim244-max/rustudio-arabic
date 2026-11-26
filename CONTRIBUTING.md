# Contributing to rustudio

Thank you for contributing to **rustudio**! Please read this guide carefully before opening an issue or pull request so your changes fit cleanly into the codebase and pass all checks.

## Table of Contents

1. [Development Setup](#development-setup)
2. [Workspace Layout](#workspace-layout)
3. [Android & Mobile Engineering Rules](#android--mobile-engineering-rules)
4. [Code Style & Quality Standards](#code-style--quality-standards)
5. [Testing & Verification Checklist](#testing--verification-checklist)
6. [Pull Request Workflow](#pull-request-workflow)

---

## Development Setup

### Desktop (Linux / macOS / Windows)

- **Rust toolchain**: Install the latest stable Rust toolchain via [rustup](https://rustup.rs/) with `rustfmt` and `clippy`.
- **Build & run**:
  ```bash
  cargo build
  cargo run
  ```
- **Standalone player**:
  ```bash
  cargo build -p rustudio-player
  ```

### Android (`arm64-v8a`)

- **Package ID**: `com.logosforgestudios.rustudio` (`crates/rustudio-android/manifest.yaml`)
- **Targets & tools**:
  ```bash
  rustup target add aarch64-linux-android
  ```
  Build the release APK with `xbuild` (`x`):
  ```bash
  CARGO_PROFILE_RELEASE_LTO=thin x build -p rustudio-android --platform android --arch arm64 --format apk --release
  ```
- See [docs/ANDROID.md](docs/ANDROID.md) for Android NDK/SDK setup, touch input architecture, and device debugging.

---

## Workspace Layout

| Path | Responsibility |
| --- | --- |
| `src/` | Studio editor UI, panels, mobile floating windows, virtual joystick, Play session orchestration |
| `crates/rustudio-core/` | DataModel (`Document`), settings, serialization, profiling, mechanical constants |
| `crates/rustudio-render/` | `wgpu` renderer, WGSL shaders, shadow cascades, SSAO, PostFx, quality profiles |
| `crates/rustudio-engine/` | Luau runtime, engine domains, GUI layout/rendering, touch input, environment probes |
| `crates/rustudio-physics/` | Physics solver and collision/joint simulation |
| `crates/rustudio-player/` | Shared `Game` view, offscreen device setup, Play loop |
| `crates/rustudio-net/` | Team Create matchmaking, QUIC transport, replication |
| `crates/rustudio-account/` | Roblox authentication, cloud APIs, session management |
| `crates/rustudio-android/` | Android `NativeActivity` entry point and `manifest.yaml` |

---

## Android & Mobile Engineering Rules

Contributions touching Android, rendering, or settings **must** follow these rules:

1. **Native Android graphics drivers only**: Do **not** bundle, load, or activate third-party driver shims such as Turnip, PanVK, or `libadrenotools`. Target standard Android Vulkan 1.1+ and OpenGL ES 3.0+.
2. **Never silently downgrade user quality settings**: Do **not** hardcode forced "weak-GPU" overrides on Android that strip shadows, lock quality levels, or lower resolution behind the user's back. Optimize the renderer itself (tile-memory efficiency, pass reduction, shader specialization, shadow caching) and keep quality controls in the user's hands.
3. **Verify WGSL shaders on real pipeline builds**: Whenever modifying `shader.wgsl`, `postfx.wgsl`, or `sun_shadow.rs`, run the GPU pipeline build test (`renderer_pipelines_build_on_this_gpu`) before building an APK so shader syntax/binding regressions never reach a device.
4. **Respect touch & multi-touch isolation**: Editor UI overlays (floating windows, virtual joystick) must claim their pointer/touch IDs cleanly so they never leak into 3D camera rotation or pinch-to-zoom gestures.
5. **No fabricated benchmarks**: Only report real, measured device numbers or clearly label static/host-side checks.

---

## Code Style & Quality Standards

- Keep changes focused and minimal—avoid unrelated refactors in feature or bugfix PRs.
- Follow standard Rust formatting (`cargo fmt --check`) and avoid introducing new compiler or `clippy` warnings.
- Keep platform-specific optimizations gated cleanly with `#[cfg(target_os = "android")]` or `cfg!(target_os = "android")` so desktop parity is preserved.
- Add or update unit tests for every bug fix, setting, or protocol change.
- Update relevant docs (`README.md`, `docs/ANDROID.md`, `docs/DEVELOPING.md`, etc.) when behavior or configuration changes.

---

## Testing & Verification Checklist

Before submitting a pull request, run the relevant test suites:

```bash
# Core, renderer, and networking unit tests
cargo test -p rustudio-core --lib
cargo test -p rustudio-render --lib
cargo test -p rustudio-net --lib

# Mandatory after any WGSL / pipeline change
cargo test -p rustudio-render --lib -- render::gpu_tests::renderer_pipelines_build_on_this_gpu
```

If your PR touches Android:
- Confirm `crates/rustudio-android/manifest.yaml` keeps `package: com.logosforgestudios.rustudio`.
- Confirm the built APK contains only `lib/arm64-v8a/librustudio_android.so` (no Turnip/PanVK/adrenotools `.so` files).

---

## Pull Request Workflow

1. Fork the repository and create a descriptive branch (`fix/playtest-shader-overdraw`, `feat/mobile-window-snap`, etc.).
2. Make small, atomic commits with clear commit messages explaining **why** the change is needed.
3. Open a pull request describing:
   - What problem it solves or feature it adds.
   - Which platforms/devices were tested (e.g., Linux Vulkan, Vivo Y27 / Mali-G52 MC2).
   - Screenshots or FPS measurements if the change affects UI or rendering.
4. Respond to review feedback and keep your branch up to date with `main`.

Need help or want to discuss a bigger change first? Join the [rustudio Discord](https://discord.gg/8EX6cpgsRx) or open an issue!
