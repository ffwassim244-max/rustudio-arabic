# call-costs

Engine call costs from a LocalScript, in instructions per call, with a generous ceiling per call, plus a record of
query, signal and value-type results that must not change. Made for a port's report (2026-11-14) of slow
`Workspace:Raycast`, property writes, CFrame construction and idle engine steps.

```
python3 checks/call-costs/check.py [--bin target/release/rustudio] [--cpu] [--record]
```

- `make_place.py OUT` writes the place: a level of 3000 anchored Parts (70 % render pieces with SpecialMesh
  FileMeshes, CanQuery and CanCollide off; balls, cylinders, wedges, Glass, a collision group G2 that doesn't
  collide with Default among the rest), an `Actor` model (a rig part with 300 nested Bones, 600 Attachments, 50
  movable parts), `setup.server.luau` (grass terrain with a water pool, the collision group) and `probe.luau`.
- `probe.luau` prints `Q` lines (1280 rays over 8 RaycastParams variants including Include / Exclude with
  descendants, RespectCanCollide, IgnoreWater, CollisionGroup; Spherecast / Blockcast / Shapecast; GetPartBoundsInBox
  / InRadius / GetPartsInPart over 6 OverlapParams; the legacy FindPartOnRay family), `S` lines (Changed,
  GetPropertyChangedSignal, game.ItemChanged connect / disconnect / Wait, a moved part found by rays), `V` lines
  (CFrame tostring, arithmetic, methods, typeof, equality, table keys, error texts), then `MB name value` costs.
- `check.py` runs it with `--play-only --no-render` under `RUSTUDIO_INSTR_CLOCK=1` (os.clock = instructions
  retired, so the costs are instructions per call and don't move with machine load), compares the records with
  `expected-records.txt` exactly and each cost with `thresholds.json` (about 1.6x the 2026-11-14 build).
  `--cpu`: CPU-time clock (ns per call), no ceilings. `--record`: rewrite `expected-records.txt` (only after
  checking that a change in the records is meant).

## 2026-11-14 (CPU ns per call in this place, before -> after; Studio's figure from rustudio's round-2 notes)

| call | before | after | Studio |
|---|---|---|---|
| Raycast, Exclude 1 model, 36 down (terrain under it) | 14338 | 2684 | |
| Raycast, first of a frame after 1550 rig writes | 181361 | ~110000 | |
| Raycast in the port's level (no terrain), back to back | 5.3 us | 1.5 us | |
| GetPartBoundsInBox / InRadius | 5729 / 4008 | 2401 / 2106 | |
| set Bone.CFrame | 741 | 381 | |
| set Attachment.Position | 644 | 245 | |
| set Part.CFrame (anchored) | 771 | 381 | 1124 |
| set Part.Transparency | 351 | 164 | 253 |
| get Part.CFrame | 333 | 331 | 45 |
| CFrame.new(x, y, z) | 219 | 73 | 26 |
| CFrame.new(...) * CFrame.Angles(...) | 698 | 310 | |
| CFrame * Vector3 | 99 | 29 | |
| CFrame:Inverse | 208 | 70 | |

The port's level (--fixed-dt bench, the same 600 frames): 59.5 -> 46.1 M instructions and 6.55 -> 5.32 ms CPU per
60 Hz frame; engine domain steps 6.25 -> 0.52 M instructions.

Open: CFrame / Bone reads (`part.CFrame`) go through the domain route and still make the value with a registry
reference; a CFrame is still an mlua userdata (a Lua block plus a Rust heap cell): a 48-byte tagged userdata
measured 35-47 ns against 73 for the fast-path CFrame.new, so a native representation would give about 2x more on
construction (Studio: 26 ns) at the price of rewriting every CFrame use (87 sites and the method table).
