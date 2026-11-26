# idle-frames: no periodic stalls in an idle editor on a large place

An idle editor should do the same work every frame. A job that runs on a
timer and costs in proportion to the place (walking every instance, a full
rebuild) shows up as a hitch a few times a second, even when the average
frame is cheap. This check opens a large place, leaves it idle with the
camera still, and fails if any frame's main-thread instruction count is
more than 2.5x the median.

    python3 checks/idle-frames/check.py [--bin target/release/rustudio]
        [--place PLACE] [--instances 300000] [--seconds 20] [--warmup 15]
        [--ratio 2.5] [--keep DIR] [--no-ui-only]

- `make_place.py OUT.rbxlx [--instances N]`: the synthetic place (default
  300 000 instances: 4 000 anchored parts in 40 Models with lights and 4
  Highlights, a ScreenGui, and nested Folders of StringValues for bulk).
- `check.py`: two runs, the editor with its full UI inside
  `gamescope --backend headless` (nothing on screen) and `--no-ui
  --headless`, each a `--bench` with `RUSTUDIO_BENCH_INSTR=1` (sections and
  frames in instructions, steady on a loaded machine) and
  `RUSTUDIO_BENCH_STILL=1` (the bench doesn't turn the camera). The bench
  writes `OUT.frames.json`: per measured frame, the main thread's
  instructions (millions), its wall time (ms) and its largest sections. On a
  failure the check prints the spikes' spacing (steady in ms: a timer;
  steady in frames: an every-Nth-frame job) and the worst frame's sections.
  Exit 1 on a failure.

## Results (2026-11-14, release builds)

Synthetic place (299 954 instances), 20 s idle after 15 s:

| build | editor, full UI: median / max M instr | `--no-ui`: median / max |
|---|---|---|
| c23f8a6 | 9.42 / 9.62 (1.02x), PASS | 4.52 / 4.62 (1.02x), PASS |
| c23f8a6 with 184e2f3's viewport.rs | 9.42 / 58.2 (6.2x), FAIL: 74 spikes every 270 ms | 4.53 / 53.4 (11.8x), FAIL |

The failure was a periodic stall the owner saw on a large place (about
330 000 instances): the Highlight list (viewport.rs, added in 0a5c596) was
found by walking the whole DataModel every 0.25 s, ~50 M instructions each
time on that place, against ~15 M for a whole idle frame. The Highlights
are now kept by a `doc::ClassIndex` fed from the change journal.

That place, 20 s idle, M instructions per frame (median / p99 / max):

| | before (184e2f3's viewport.rs) | after (c23f8a6) |
|---|---|---|
| editor, full UI | 14.65 / 67.89 / 68.40, 72 spikes 275 ms apart | 14.68 / 14.91 / 15.32 (60 s) |
| `--no-ui --headless` | 9.92 / 63.30 / 63.67, 73 spikes | 9.89 / 10.12 / 10.26 |

Other places, editor, full UI: a 120k-instance place 25.42 / 44.88 / 46.55
before, 25.45 / 27.36 / 29.81 after; a one-part place 6.41 / 6.52 / 6.62
both. Right after a large place loads, parts whose meshes arrive late
rejoin the static batches in one rebuild every 3 s (`render_static_rebuild`,
~94 M instructions on the 120k place) until loading settles: a warm-up
that long (`--warmup`) keeps it out of the measurement.
