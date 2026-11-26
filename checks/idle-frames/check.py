#!/usr/bin/env python3
"""Idle editor frames on a large place: no frame may do much more work than
the median frame, a selected or hovered part costs about what nothing
selected costs, and (with GPU timestamps) the see-through passes cost
nothing without a see-through surface and little for a small one. Catches
periodic stalls (a timer that walks the whole place, a rebuild every few
hundred ms), per-frame outline work and full-screen passes that run for
nothing, without needing a real game.

    check.py [--bin RUSTUDIO] [--place PLACE] [--instances N] [--seconds S]
             [--warmup S] [--ratio R] [--select-ratio R] [--see-share F]
             [--keep DIR] [--no-ui-only] [--no-gpu]

Opens the place (default: make_place.py's synthetic 300k-instance place) in
the editor with its full UI inside `gamescope --backend headless` (nothing on
screen), then with `--no-ui --headless`, idles with the camera still
(RUSTUDIO_BENCH_STILL=1) and counts the main thread's user-space
instructions per frame (RUSTUDIO_BENCH_INSTR=1, OUT.frames.json). Fails if
any measured frame's instructions exceed RATIO x the median (default 2.5),
and prints the spacing of the spikes (a steady spacing in ms is a timer, in
frames an every-Nth-frame job) and the bench sections of the worst frame.
Instruction counts don't move with the machine's load, unlike frame times.

The editor runs again with a part selected (a click) and with the pointer
resting over one (hover): their median frame may cost at most SELECT_RATIO
(default 1.3) x the unselected median (the selection's outline is kept, not
rebuilt every frame). With GPU timestamps (RUSTUDIO_GPU_TIMING; skipped with
--no-gpu or when the adapter has none), `--no-ui --headless` runs measure the
see-through passes ("gpu: …see-through…" / "…resolve (encoded)…"): none on
the place as made (nothing see-through), and at most SEE_SHARE (default 0.25)
of the GPU frame with one small half-transparent part in view.
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))


# Looking down across the grid: the view's middle is over the parts and
# the baseplate under them.
CAM = "0,60,-80,180,-35"
# A view position over that (the click / hover target); the pointer rests
# outside the view otherwise.
OVER = "300,200"
AWAY = "-40,-40"


def run(bin_, place, out, seconds, warmup, ui, extra=(), instr=True, env_extra=None):
    env = dict(os.environ, RUSTUDIO_NO_AUDIO="1", RUSTUDIO_BENCH_STILL="1", RUSTUDIO_BENCH_WARMUP=str(warmup))
    if instr:
        env["RUSTUDIO_BENCH_INSTR"] = "1"
    env.update(env_extra or {})
    args = [bin_, place, "--bench", out + ".json", "--seconds", str(seconds), "--log", out + ".log",
            "--camera", CAM, *extra]
    if ui:
        cmd = ["gamescope", "--backend", "headless", "-W", "1265", "-H", "900", "--"] + args
    else:
        cmd = args + ["--no-ui", "--headless", "--window", "1265x900"]
    with open(out + ".out", "w") as f:
        subprocess.run(cmd, env=env, stdout=f, stderr=subprocess.STDOUT, timeout=warmup + seconds + 600)
    note_unit(out)
    return json.load(open(out + ".frames.json"))


# What the per-frame work figures are: M instructions (Linux), M CPU cycles
# where the bench reports another work_unit (Windows: QueryThreadCycleTime).
UNIT = ["M instructions"]


def note_unit(out):
    if "cycles" in json.load(open(out + ".json")).get("work_unit", ""):
        UNIT[0] = "M cycles"


def median_instr(frames):
    return statistics.median(f[0] for f in frames) if frames else 0.0


def judge_select(name, frames, base, ratio):
    if ratio is None:
        ratio = 1.5 if UNIT[0] == "M cycles" else 1.3
    med = median_instr(frames)
    ok = base > 0 and med <= ratio * base
    print(f"{name}: median {med:.2f} {UNIT[0]}/frame against {base:.2f} unselected "
          f"({med / base if base else 0:.2f}x, at most {ratio}x) -> {'ok' if ok else 'FAIL'}")
    return ok


# Windows: two timestamps around a pass that does nothing still read a few
# microseconds apart (0.0007-0.0017 ms measured, master and branch alike),
# so "none" there is at most this; elsewhere none is 0.
EMPTY_SPAN_MS = 0.01 if sys.platform == "win32" else 0.0


def see_share(out):
    """(GPU ms per frame in see-through passes, GPU ms per frame) or None without timestamps."""
    r = json.load(open(out + ".json"))
    g = r.get("gpu_frame_ms")
    if not g:
        return None
    see = sum(s["ms_per_frame"] for s in r["sections"]
              if s["name"].startswith("gpu:") and ("see-through" in s["name"] or "resolve (encoded)" in s["name"]))
    return see, g["median"]


def judge(name, frames, ratio):
    if ratio is None:
        ratio = 4.0 if UNIT[0] == "M cycles" else 2.5
    mi = [f[0] for f in frames]
    if not mi or max(mi) <= 0:
        print(f"{name}: no instruction counts (perf_event_open unavailable?)")
        return False
    med = statistics.median(mi)
    s = sorted(mi)
    p99 = s[int(0.99 * (len(s) - 1))]
    worst = max(range(len(mi)), key=lambda i: mi[i])
    spikes = [i for i, v in enumerate(mi) if v > ratio * med]
    ok = not spikes
    print(f"{name}: {len(mi)} frames, {UNIT[0]}/frame median {med:.2f} p99 {p99:.2f} "
          f"max {s[-1]:.2f} ({s[-1] / med:.2f}x median) -> {'ok' if ok else 'FAIL'}")
    if spikes:
        t, at = 0.0, []
        for f in frames:
            at.append(t)
            t += f[1]
        gaps_ms = [round(at[b] - at[a]) for a, b in zip(spikes, spikes[1:])]
        gaps_n = [b - a for a, b in zip(spikes, spikes[1:])]
        print(f"  {len(spikes)} frames over {ratio}x median; spacing ms {gaps_ms[:12]} frames {gaps_n[:12]}")
        print(f"  worst frame sections: {frames[worst][2]}")
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(APP, "target", "release", "rustudio"))
    ap.add_argument("--place")
    ap.add_argument("--instances", type=int, default=300_000)
    ap.add_argument("--seconds", type=float, default=20)
    ap.add_argument("--warmup", type=float, default=15)
    ap.add_argument("--ratio", type=float, help="default 2.5 (instructions); 4 where the bench counts CPU cycles "
                    "(Windows), whose single frames swing more")
    ap.add_argument("--select-ratio", type=float, help="default 1.3 (instructions); 1.5 where the bench counts "
                    "CPU cycles (Windows: separate runs' medians differ more)")
    ap.add_argument("--see-share", type=float, default=0.25)
    ap.add_argument("--keep", help="write the runs here instead of a temporary folder")
    ap.add_argument("--no-ui-only", action="store_true", help="skip the full-UI runs (no gamescope); the selected / hovered "
                    "cases run with --no-ui --overlays instead")
    ap.add_argument("--no-gpu", action="store_true", help="skip the GPU timestamp runs")
    a = ap.parse_args()

    tmp = a.keep or tempfile.mkdtemp(prefix="idle-frames-")
    os.makedirs(tmp, exist_ok=True)
    try:
        place = a.place
        if not place:
            place = os.path.join(tmp, "large.rbxlx")
            subprocess.run([sys.executable, os.path.join(HERE, "make_place.py"), place,
                            "--instances", str(a.instances)], check=True)
        ok = True
        # (the click / hover lands before measuring starts)
        at = f"@{max(1.0, a.warmup - 3):g}"
        # (the pointer leaves a second after the click: a click and a move
        # in the same frame select nothing)
        later = f"@{max(1.0, a.warmup - 3) + 1:g}"
        if not a.no_ui_only:
            if shutil.which("gamescope") is None:
                print("editor (full UI): gamescope not found; use --no-ui-only")
                ok = False
            else:
                away = ["--move", AWAY + "@1"]
                base = run(a.bin, place, os.path.join(tmp, "ui"), a.seconds, a.warmup, True, away)
                ok &= judge("editor (full UI)", base, a.ratio)
                sel = run(a.bin, place, os.path.join(tmp, "ui-sel"), a.seconds, a.warmup, True,
                          away + ["--click", OVER + at, "--move", AWAY + later])
                ok &= judge("editor, a part selected", sel, a.ratio)
                ok &= judge_select("editor, a part selected", sel, median_instr(base), a.select_ratio)
                hov = run(a.bin, place, os.path.join(tmp, "ui-hover"), a.seconds, a.warmup, True,
                          ["--move", OVER + at])
                ok &= judge("editor, a part hovered", hov, a.ratio)
                ok &= judge_select("editor, a part hovered", hov, median_instr(base), a.select_ratio)
        ok &= judge("--no-ui --headless", run(a.bin, place, os.path.join(tmp, "noui"), a.seconds, a.warmup, False), a.ratio)
        if a.no_ui_only:
            # (no full UI: the selected / hovered cases in the 3D view alone,
            # its outlines kept by --overlays)
            ov = ["--overlays", "--move", AWAY + "@1"]
            base = run(a.bin, place, os.path.join(tmp, "noui-ov"), a.seconds, a.warmup, False, ov)
            ok &= judge("--no-ui --overlays", base, a.ratio)
            sel = run(a.bin, place, os.path.join(tmp, "noui-sel"), a.seconds, a.warmup, False,
                      ov + ["--click", OVER + at, "--move", AWAY + later])
            ok &= judge("--no-ui --overlays, a part selected", sel, a.ratio)
            ok &= judge_select("--no-ui --overlays, a part selected", sel, median_instr(base), a.select_ratio)
            hov = run(a.bin, place, os.path.join(tmp, "noui-hover"), a.seconds, a.warmup, False,
                      ["--overlays", "--move", OVER + at])
            ok &= judge("--no-ui --overlays, a part hovered", hov, a.ratio)
            ok &= judge_select("--no-ui --overlays, a part hovered", hov, median_instr(base), a.select_ratio)
        if not a.no_gpu:
            gpu = {"RUSTUDIO_GPU_TIMING": "1"}
            seconds = min(a.seconds, 10)
            out = os.path.join(tmp, "gpu")
            run(a.bin, place, out, seconds, a.warmup, False, instr=False, env_extra=gpu)
            r = see_share(out)
            if r is None:
                print("GPU: no timestamps on this adapter; GPU cases skipped")
            else:
                good = r[0] <= EMPTY_SPAN_MS
                none = f"at most {EMPTY_SPAN_MS} ms, an empty span's timestamps" if EMPTY_SPAN_MS else "0"
                print(f"GPU, nothing see-through: see-through passes {r[0]:.4f} ms of {r[1]:.3f} ms "
                      f"(must be none: {none}) -> {'ok' if good else 'FAIL'}")
                ok &= good
                glass = os.path.join(tmp, "see-through.rbxlx")
                subprocess.run([sys.executable, os.path.join(HERE, "make_place.py"), glass,
                                "--instances", "20000", "--see-through"], check=True, stdout=subprocess.DEVNULL)
                out = os.path.join(tmp, "gpu-see")
                run(a.bin, glass, out, seconds, a.warmup, False, instr=False, env_extra=gpu)
                r = see_share(out)
                good = r is not None and EMPTY_SPAN_MS < r[0] <= a.see_share * r[1]
                if r is not None:
                    print(f"GPU, one small see-through part: see-through passes {r[0]:.3f} ms of {r[1]:.3f} ms "
                          f"({r[0] / r[1] if r[1] else 0:.0%}, at most {a.see_share:.0%}, more than none) "
                          f"-> {'ok' if good else 'FAIL'}")
                ok &= good
        print("PASS" if ok else "FAIL")
        return 0 if ok else 1
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
