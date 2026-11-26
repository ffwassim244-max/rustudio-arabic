#!/usr/bin/env bash
# The Mesa Vulkan drivers — Turnip for Adreno, PanVK for Mali — and
# libadrenotools, into the Android APK's lib/arm64-v8a
# (crates/rustudio-android/libs/arm64-v8a/, which manifest.yaml lists as
# runtime_libs). The app swaps the driver in at start on Snapdragon
# devices (Turnip) and on Mali devices with ARM's kbase kernel driver
# (PanVK) (src/android/turnip.rs; docs/ANDROID.md "Turnip driver"). Run
# before `x build` — the Android workflow does; a build without these
# files is a normal APK that keeps the device's driver.
#
# Sources (all prebuilt, none is built here; the pinned downloads):
#   * libadrenotools-native: TouseefX's fork of bylaws/libadrenotools
#     (BSD-2-Clause; libshadowhook / libbytehook inside are ByteDance's,
#     MIT) — libadrenotools.so and its hook libraries. The pinned build is
#     the fork's CI artifact of commit 8c7d9f5 (2026-11-20 05:08, workflow
#     run 44679007919), served by nightly.link:
#     https://nightly.link/TouseefX/libadrenotools-native/actions/runs/44679007919/libadrenotools-arm64-v8a.zip
#     Over the build before it (ca1ad33, run 44383713561) its
#     constructor's per-generation Turnip defaults give an Adreno 6xx
#     (the GPU model read after "Adreno" in kgsl's gpu_model, three
#     digits) TU_DEBUG=noconform,noubwc,nolrz when nothing set TU_DEBUG
#     before it ran (the UBWC whole-device restarts; the app sets
#     TU_DEBUG itself and switches UBWC off through the switches file,
#     src/android/turnip.rs default_switches) — nothing else changed in
#     its sources. It has the driver.cpp rewrite of 4d4d482 (2026-11-17: the Vulkan
#     loader looked for under the vendor paths too, which lets HyperOS
#     devices try Turnip; its own initialisation switchable off with
#     ADRENOTOOLS_DISABLE=1, which the app sets, and hooking only
#     vkGetInstanceProcAddr / vkGetDeviceProcAddr with the system's own
#     callers bypassed when it isn't; a working PanVK path for Mali; a
#     mailbox present-mode default for Mesa's own WSI) plus the fork's
#     fixes of 2026-11-18 for whole-device reboots with Turnip (95ddbd5,
#     3360339: the GSL free hook passed gpuaddr=0 to
#     IOCTL_KGSL_GPUMEM_GET_INFO and then IOCTL_KGSL_GPUOBJ_FREE a garbage
#     id — a GPU fault in the kernel, a KGSL panic, a reboot; the
#     adrenotools_set_turbo(false) its init did on every start, a
#     KGSL_PROP_PWRCTRL ioctl that some Adreno kernels crash on, is gone
#     — turbo is only ever on request now; mutexes around the GSL alloc /
#     free hooks and the pending GPU mapping; handle leaks; alignment
#     checks before MAP_FIXED). libadrenotools.so and libhook_impl.so of
#     this build carry a Rust layer (cxx; the pending-mapping bookkeeping,
#     the Mali mailbox, the dispatch state) and come with Rust std's debug
#     info, 8 MB each: the script strips that (below, STRIP=0 keeps it).
#     GitHub keeps workflow artifacts for 90 days: when it is gone, point
#     ADRENOTOOLS_ZIP at a release of the fork (V0.7 is the last one:
#     https://github.com/TouseefX/libadrenotools-native/releases/download/V0.7/libadrenotools-arm64-v8a.zip,
#     SHA-256 73eb6a8178d9280d0ed25ef1bee1852a8225dcfb4a8c82e11d0a5146424871b3;
#     it keeps HyperOS on the device's driver, and devices whose device
#     tree names a Mali, and its hooks of the system loader are run,
#     unused, since it has no ADRENOTOOLS_DISABLE).
#   * mesa-turnip-android-driver: nihui's Mesa builds for Android
#     (Mesa: MIT) — libvulkan_freedreno.so (Mesa 26.3.1-lto, 2026-11-19):
#     https://github.com/nihui/mesa-turnip-android-driver/releases/download/26.3.1/mesa-turnip-android-26.3.1.zip
#   * mali-panvk: JimVulkan's Mesa PanVK for Mali on the stock Arm kbase
#     kernel driver — no root, no custom kernel (Mesa: MIT; release r10,
#     Mesa 27.0.0-devel 58f2c55, Vulkan 1.4 on Bifrost G31-G76, Valhall
#     G57-G78 and the command-stream G610-G925 / G1; Android 12+). The
#     zip holds libvulkan_panfrost.so (18 MB; the name libadrenotools-
#     native's own PanVK path opens, src/driver.cpp init_panvk_driver),
#     a meta.json and the notices:
#     https://github.com/JimVulkan/mali-panvk/releases/download/v1.6.0/mali-panvk-27.0.0-devel-r10.zip
#     PANVK=0 leaves it out (an APK 18 MB smaller that keeps Mali
#     devices on their own driver).
#
# Overrides, for a newer build or one of your own:
#   ADRENOTOOLS_ZIP=<url or local .zip>   TURNIP_ZIP=<url or local .zip>
#   PANVK_ZIP=<url or local .zip, or a bare .so>  (PANVK_SO still works)
#   ADRENOTOOLS_SHA256= / TURNIP_SHA256= / PANVK_SHA256=
#                                          (checked when given; the
#                                          defaults are pinned below)
#   PANVK=0                                no PanVK driver
#   STRIP=0                                keep the libraries' debug info
#                                          (the libadrenotools and Turnip
#                                          files carry some; they are
#                                          stripped of it with llvm-strip
#                                          — from PATH or an NDK named by
#                                          ANDROID_NDK_HOME / _ROOT /
#                                          ANDROID_NDK_LATEST_HOME — when
#                                          one is found; kept as they are
#                                          otherwise)
#   FORCE=1                                replace files already there
#   X_CACHE_DIR=<dir>                      xbuild's cache, for the sysroot
#                                          markers (below; default
#                                          $XDG_CACHE_HOME/x, ~/.cache/x,
#                                          ~/Library/Caches/x on macOS)
#   XBUILD_SYSROOT=0                       no sysroot markers (packaging
#                                          with something other than
#                                          xbuild)
# Files already in libs/arm64-v8a/ are kept unless FORCE=1, so a build of
# your own can be dropped in by hand (git ignores the .so files; `git add
# -f` to commit them).
#
# xbuild's library check, last: before it packs the APK, xbuild reads
# every bundled library's NEEDED list (llvm-readobj) and wants each name
# either bundled too or in its NDK sysroot (~/.cache/x/Android.ndk/usr/lib/
# aarch64-linux-android and the minSdk's folder under it), else it stops:
# "Failed to collect all required libraries for libvulkan_panfrost.so …
# Shared library `libhardware.so` not found" (xbuild/src/command/build.rs
# at the workflow's XBUILD_REV). PanVK links libhardware.so (Mesa's
# Android gralloc path) and libc++.so: system libraries every device has
# (/system/lib64; VNDK-SP) that the NDK doesn't export. The app doesn't
# link them — libadrenotools loads the driver in a namespace cloned from
# the system Vulkan loader's own (hook_impl.cpp, where libc++.so is linked
# over by name), which reaches them. xbuild only reads the *names* in its
# sysroot folders, so an empty marker file there makes it treat the
# library as one the device provides, which it is; nothing is added to
# the APK (a stub in libs/ would be, and would shadow the real library in
# the driver's namespace). xbuild fetches its sysroot during the first
# `x build`, after cargo's half hour and before this check, so when the
# folder isn't there yet the same tarball goes in first (the xbuild
# release artifact the pinned commit names, download.rs
# WorkItem::VERSION; xbuild sees the folder and skips its download).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
dest="$root/crates/rustudio-android/libs/arm64-v8a"

ADRENOTOOLS_ZIP_DEFAULT="https://nightly.link/TouseefX/libadrenotools-native/actions/runs/44679007919/libadrenotools-arm64-v8a.zip"
TURNIP_ZIP_DEFAULT="https://github.com/nihui/mesa-turnip-android-driver/releases/download/26.3.1/mesa-turnip-android-26.3.1.zip"
PANVK_ZIP_DEFAULT="https://github.com/JimVulkan/mali-panvk/releases/download/v1.6.0/mali-panvk-27.0.0-devel-r10.zip"
ADRENOTOOLS_ZIP="${ADRENOTOOLS_ZIP:-$ADRENOTOOLS_ZIP_DEFAULT}"
TURNIP_ZIP="${TURNIP_ZIP:-$TURNIP_ZIP_DEFAULT}"
# (PANVK_SO: the override's earlier name, from when the driver came as a
# bare .so; either name takes a .zip or a .so)
PANVK_ZIP="${PANVK_ZIP:-${PANVK_SO:-$PANVK_ZIP_DEFAULT}}"
# The pinned checksums belong to the pinned downloads: an override without
# a checksum of its own isn't held to them (decided before the defaults go
# in, so that an unset *_SHA256 can still be told from a given one).
if [ -z "${ADRENOTOOLS_SHA256+x}" ]; then
  ADRENOTOOLS_SHA256=""
  [ "$ADRENOTOOLS_ZIP" != "$ADRENOTOOLS_ZIP_DEFAULT" ] || ADRENOTOOLS_SHA256=94aacd0a59fa3b23c361ad33a4809018374bcbb0d71564d2f5afc277b114c34c
fi
if [ -z "${TURNIP_SHA256+x}" ]; then
  TURNIP_SHA256=""
  [ "$TURNIP_ZIP" != "$TURNIP_ZIP_DEFAULT" ] || TURNIP_SHA256=9f38529b5f5f6640c1d600a398cc5218cdf7706066e68e8d2fbc7cbde644a8c0
fi
if [ -z "${PANVK_SHA256+x}" ]; then
  PANVK_SHA256=""
  [ "$PANVK_ZIP" != "$PANVK_ZIP_DEFAULT" ] || PANVK_SHA256=7e97db4d79d81ec9461d1e0676b62a865561cb56e21c151693aa25ffbe749f1c
fi

mkdir -p "$dest"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# fetch <name> <url-or-path> <sha256-or-empty> <marker file in dest> <files...>
fetch() {
  local name="$1" src="$2" sum="$3" marker="$4"
  if [ -f "$dest/$marker" ] && [ "${FORCE:-0}" != "1" ]; then
    echo "$name: $dest/$marker present, keeping it (FORCE=1 replaces)"
    return
  fi
  local zip="$work/$name.zip"
  case "$src" in
    http://*|https://*)
      echo "$name: downloading $src"
      curl -fsSL --retry 3 -o "$zip" "$src"
      ;;
    *)
      echo "$name: using $src"
      cp "$src" "$zip"
      ;;
  esac
  if [ -n "$sum" ]; then
    echo "$sum  $zip" | sha256sum -c - >/dev/null || { echo "$name: checksum mismatch for $src" >&2; exit 1; }
  fi
  local out="$work/$name"
  mkdir -p "$out"
  unzip -q -o "$zip" -d "$out"
  local n=0
  local copied=()
  while IFS= read -r -d '' f; do
    cp "$f" "$dest/"
    chmod 644 "$dest/$(basename "$f")"
    copied+=("$dest/$(basename "$f")")
    n=$((n + 1))
  done < <(find "$out" -type f -name '*.so' -print0)
  [ "$n" -gt 0 ] || { echo "$name: no .so files in $src" >&2; exit 1; }
  [ -f "$dest/$marker" ] || { echo "$name: $marker not in $src" >&2; exit 1; }
  echo "$name: $n file(s) into $dest"
  strip_debug "$name" "${copied[@]}"
}

# strip_debug <name> <files...>: drop the .debug_* sections (llvm-strip
# --strip-debug keeps the symbol tables, so crash logs still name
# functions). The libadrenotools build's two libraries with the Rust layer
# come with Rust std's debug info, 6 MB of each file's 8 MB; nihui's Turnip
# 26.3.1 carries 1.2 MB of its 17.6 MB (the PanVK release is stripped
# already, so nothing happens to it). STRIP=0 keeps the files as
# downloaded; without an llvm-strip they are kept too (GNU strip on a
# non-arm64 host can't read them).
strip_debug() {
  local name="$1"; shift
  [ "${STRIP:-1}" != "0" ] || { echo "$name: STRIP=0, debug info kept"; return; }
  local tool
  tool="$(llvm_tool llvm-strip)"
  if [ -z "$tool" ]; then
    echo "$name: no llvm-strip found (PATH, ANDROID_NDK_HOME / ANDROID_NDK_ROOT / ANDROID_NDK_LATEST_HOME); debug info kept"
    return
  fi
  local f before after
  for f in "$@"; do
    before=$(stat -c %s "$f")
    "$tool" --strip-debug "$f"
    after=$(stat -c %s "$f")
    [ "$after" -eq "$before" ] || echo "$name: $(basename "$f") $before -> $after bytes (debug info stripped)"
  done
}

# llvm_tool <name...>: the first of the named LLVM tools found on PATH or
# in an NDK named by ANDROID_NDK_HOME / ANDROID_NDK_ROOT /
# ANDROID_NDK_LATEST_HOME; prints its path, or nothing.
llvm_tool() {
  local name ndk cand
  for name in "$@"; do
    if command -v "$name" >/dev/null 2>&1; then
      command -v "$name"
      return
    fi
  done
  for ndk in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_ROOT:-}" "${ANDROID_NDK_LATEST_HOME:-}"; do
    [ -n "$ndk" ] || continue
    for name in "$@"; do
      for cand in "$ndk"/toolchains/llvm/prebuilt/*/bin/"$name"; do
        if [ -x "$cand" ]; then echo "$cand"; return; fi
      done
    done
  done
}

# needed_libs <reader> <file>: the file's NEEDED entries, one per line
# (llvm-readobj's --needed-libs listing, or readelf -d's).
needed_libs() {
  case "$1" in
    *llvm-readobj*) "$1" --needed-libs "$2" | sed -n '/^NeededLibraries \[/,/^\]/{/^  */s/^ *//p;}' ;;
    *) "$1" -d "$2" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p' ;;
  esac
}

# xbuild_sysroot_markers: the marker files xbuild's library check needs
# for the bundled libraries (the comment at the top). The bundled set and
# xbuild's two sysroot folders count as provided; every other NEEDED name
# gets an empty file in the sysroot folder, and is named here.
xbuild_sysroot_markers() {
  if [ "${XBUILD_SYSROOT:-1}" = "0" ]; then
    echo "xbuild: XBUILD_SYSROOT=0, no sysroot markers"
    return
  fi
  local reader
  reader="$(llvm_tool llvm-readobj llvm-readelf readelf)"
  if [ -z "$reader" ]; then
    echo "xbuild: no llvm-readobj / readelf found (PATH, ANDROID_NDK_HOME / ANDROID_NDK_ROOT / ANDROID_NDK_LATEST_HOME): the libraries' NEEDED lists weren't checked against xbuild's sysroot. If \`x build\` stops with \"Shared library \`<name>\` not found\", that is a system library the NDK doesn't export: \`touch ~/.cache/x/Android.ndk/usr/lib/aarch64-linux-android/<name>\` (the script's comment on the check)" >&2
    return
  fi
  local xcache="${X_CACHE_DIR:-}"
  if [ -z "$xcache" ]; then
    case "$(uname -s)" in
      Darwin) xcache="$HOME/Library/Caches/x" ;;
      MINGW*|MSYS*|CYGWIN*) xcache="${LOCALAPPDATA:-$HOME/AppData/Local}/x" ;;
      *) xcache="${XDG_CACHE_HOME:-$HOME/.cache}/x" ;;
    esac
  fi
  local sysroot="$xcache/Android.ndk/usr/lib/aarch64-linux-android"
  if [ ! -d "$sysroot" ]; then
    # xbuild's own sysroot tarball (download.rs: WorkItem::xbuild_release
    # of VERSION v0.1.0+3 at the pinned commit), unpacked where xbuild
    # unpacks it, so its first `x build` finds the folder and goes on.
    local url="https://github.com/rust-mobile/xbuild/releases/download/v0.1.0%2B3/Android.ndk.tar.zst"
    if ! command -v unzstd >/dev/null 2>&1; then
      echo "xbuild: its NDK sysroot isn't in $xcache yet and there is no unzstd to unpack it with: no sysroot markers. Install zstd and run this script again, or run \`x build\` once (it fetches the sysroot; the build then stops at the library check) and this script after it" >&2
      return
    fi
    echo "xbuild: its NDK sysroot isn't in $xcache yet; fetching $url (70 MB) — xbuild would fetch the same at the next build"
    mkdir -p "$xcache/download"
    curl -fsSL --retry 3 -o "$xcache/download/Android.ndk.tar.zst" "$url"
    tar --use-compress-program=unzstd -xf "$xcache/download/Android.ndk.tar.zst" -C "$xcache"
    [ -d "$sysroot" ] || { echo "xbuild: $sysroot still missing after unpacking $url" >&2; exit 1; }
  fi
  local min_sdk
  min_sdk="$(sed -n 's/^[[:space:]]*min_sdk_version:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "$root/crates/rustudio-android/manifest.yaml" 2>/dev/null | head -n 1 || true)"
  [ -n "$min_sdk" ] || min_sdk=26
  local f need
  local -a added=()
  for f in "$dest"/*.so; do
    [ -f "$f" ] || continue
    while IFS= read -r need; do
      [ -n "$need" ] || continue
      # libc++_shared.so xbuild copies out of the NDK itself.
      [ "$need" != "libc++_shared.so" ] || continue
      if [ -e "$dest/$need" ] || [ -e "$sysroot/$need" ] || [ -e "$sysroot/$min_sdk/$need" ]; then
        continue
      fi
      : > "$sysroot/$need"
      added+=("$need (for $(basename "$f"))")
    done < <(needed_libs "$reader" "$f")
  done
  if [ "${#added[@]}" -eq 0 ]; then
    echo "xbuild: every NEEDED library of the bundle is bundled or in its sysroot ($sysroot); nothing to mark"
  else
    echo "xbuild: system libraries the bundle needs that the NDK doesn't export, marked as provided in $sysroot (empty files; not in the APK): ${added[*]}"
  fi
}

# fetch_file <name> <url-or-path> <sha256-or-empty> <file name in dest>
fetch_file() {
  local name="$1" src="$2" sum="$3" file="$4"
  if [ -f "$dest/$file" ] && [ "${FORCE:-0}" != "1" ]; then
    echo "$name: $dest/$file present, keeping it (FORCE=1 replaces)"
    return
  fi
  local out="$work/$file"
  case "$src" in
    http://*|https://*)
      echo "$name: downloading $src"
      curl -fsSL --retry 3 -o "$out" "$src"
      ;;
    *)
      echo "$name: using $src"
      cp "$src" "$out"
      ;;
  esac
  if [ -n "$sum" ]; then
    echo "$sum  $out" | sha256sum -c - >/dev/null || { echo "$name: checksum mismatch for $src" >&2; exit 1; }
  fi
  cp "$out" "$dest/$file"
  chmod 644 "$dest/$file"
  echo "$name: $file into $dest"
}

fetch adrenotools "$ADRENOTOOLS_ZIP" "$ADRENOTOOLS_SHA256" libadrenotools.so
fetch turnip "$TURNIP_ZIP" "$TURNIP_SHA256" libvulkan_freedreno.so
if [ "${PANVK:-1}" != "0" ]; then
  # A zip (the release: the .so, its meta.json and notices; only the .so
  # goes in) or a bare .so (a build of your own).
  case "$PANVK_ZIP" in
    *.so) fetch_file panvk "$PANVK_ZIP" "$PANVK_SHA256" libvulkan_panfrost.so ;;
    *) fetch panvk "$PANVK_ZIP" "$PANVK_SHA256" libvulkan_panfrost.so ;;
  esac
else
  echo "panvk: PANVK=0, not fetched (Mali devices keep their driver)"
fi

# What the app looks for (src/android/turnip.rs): the loader shim, the
# hook libraries it loads from the APK's native library folder, and the
# drivers under the names the shim opens (PanVK's optional).
for f in libadrenotools.so libhook_impl.so libmain_hook.so libvulkan_freedreno.so; do
  [ -f "$dest/$f" ] || { echo "missing $dest/$f" >&2; exit 1; }
done
ls -l "$dest"
xbuild_sysroot_markers
