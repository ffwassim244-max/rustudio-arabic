#!/usr/bin/env python3
"""call-costs check: engine call costs in instructions per call, and query / signal / value type results.

    python3 checks/call-costs/check.py [--bin PATH] [--out DIR] [--cpu] [--record]

Builds the place (make_place.py) in OUT (default /tmp/call-costs-<pid>), runs it headless with no rendering
(`--play-only --no-render`) under RUSTUDIO_INSTR_CLOCK=1 (os.clock = the main thread's instructions retired, so
"ns" per call read as instructions per call; stable on a loaded machine), then:
  - compares the Q / S / V record lines with expected-records.txt (exact),
  - compares each MB cost with thresholds.json (instructions per call; a generous ceiling each),
and prints a table. --cpu: CPU-time clock instead (ns per call; no thresholds). --record: write the records
this binary gives to expected-records.txt (after checking a change is meant). Exit 1 on any failure.
"""
import argparse, json, os, re, shutil, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bin', default=shutil.which('rustudio') or 'rustudio')
    ap.add_argument('--out', type=Path, default=Path(f'/tmp/call-costs-{os.getpid()}'))
    ap.add_argument('--cpu', action='store_true')
    ap.add_argument('--record', action='store_true')
    ap.add_argument('--seconds', type=int, default=150)
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    subprocess.run([sys.executable, str(HERE / 'make_place.py'), str(a.out)], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, RUSTUDIO_NO_AUDIO='1')
    env['RUSTUDIO_CPU_CLOCK' if a.cpu else 'RUSTUDIO_INSTR_CLOCK'] = '1'
    log = a.out / 'run.log'
    subprocess.run([a.bin, str(a.out / 'callcosts.rbxlx'), '--play-only', '--no-render', '--seconds', str(a.seconds),
                    '--log', str(log)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=a.seconds + 120)
    lines = []
    for l in log.read_text(errors='replace').splitlines():
        m = re.match(r'^Info: \[Client\] (.*?)(  —  .*)?$', l)
        if m:
            lines.append(m.group(1))
    if 'CALLCOSTS DONE' not in lines:
        print(f'FAIL: the probe did not finish (see {log})')
        return 1
    rec = [l for l in lines if re.match(r'^[QSV] ', l)]
    costs = {}
    for l in lines:
        m = re.match(r'^MB (.*?)\s+([\d.]+)$', l)
        if m:
            costs[m.group(1).strip()] = float(m.group(2))
    ok = True
    exp_path = HERE / 'expected-records.txt'
    if a.record:
        exp_path.write_text('\n'.join(rec) + '\n')
        print(f'wrote {len(rec)} records to {exp_path}')
    else:
        exp = exp_path.read_text().splitlines()
        diff = [(i, x, y) for i, (x, y) in enumerate(zip(exp, rec)) if x != y]
        if len(exp) != len(rec) or diff:
            ok = False
            print(f'FAIL records: {len(rec)} lines (expected {len(exp)}), {len(diff)} differ')
            for i, x, y in diff[:10]:
                print(f'  line {i + 1}\n    expected {x}\n    got      {y}')
        else:
            print(f'records: {len(rec)} lines, identical')
    th = json.loads((HERE / 'thresholds.json').read_text())['instructions_per_call']
    unit = 'ns' if a.cpu else 'instr'
    print(f'{"call":44s} {unit:>10s} {"ceiling":>10s}')
    for k, v in costs.items():
        lim = th.get(k)
        bad = not a.cpu and lim is not None and v > lim
        ok &= not bad
        print(f'{k:44s} {v:10.0f} {lim if lim is not None and not a.cpu else "":>10} {"OVER" if bad else ""}')
    missing = [k for k in th if k not in costs]
    if missing:
        ok = False
        print('FAIL: no cost for', ', '.join(missing))
    print('PASS' if ok else 'FAIL')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
