#!/usr/bin/env python3
"""Writes the call-costs place: OUT_DIR/callcosts.rbxlx (+ meshes/, the local-content marker).

    python3 make_place.py OUT_DIR [--parts 3000]

A level of anchored Parts with SpecialMesh FileMeshes (most not queryable, as a converted level's render pieces
are), queryable boxes / balls / cylinders / wedges / mesh parts among them, a collision group that doesn't collide
with Default, an "Actor" model (parts, a rig part with nested Bones, Attachments), a server Script that fills
terrain (grass and water), and the LocalScript probe.luau (beside this file) that records query results and value
type behaviour and times the calls (see README.md).
"""
import argparse, math, os, random
from pathlib import Path

HERE = Path(__file__).resolve().parent


def obj_rock(path, seed):
    """A closed, lumpy 6x6 latitude/longitude sphere of radius 0.5 (unit mesh)."""
    r = random.Random(seed)
    lat, lon = 6, 8
    vs, fs = [], []
    vs.append((0, 0.5, 0))
    for i in range(1, lat):
        th = math.pi * i / lat
        for j in range(lon):
            ph = 2 * math.pi * j / lon
            k = 0.5 * (0.8 + 0.2 * r.random())
            vs.append((k * math.sin(th) * math.cos(ph), k * math.cos(th), k * math.sin(th) * math.sin(ph)))
    vs.append((0, -0.5, 0))
    top, bot = 1, len(vs)
    for j in range(lon):
        fs.append((top, 2 + (j + 1) % lon, 2 + j))
    for i in range(lat - 2):
        for j in range(lon):
            a = 2 + i * lon + j
            b = 2 + i * lon + (j + 1) % lon
            c = a + lon
            d = b + lon
            fs.append((a, b, d))
            fs.append((a, d, c))
    base = 2 + (lat - 2) * lon
    for j in range(lon):
        fs.append((base + j, base + (j + 1) % lon, bot))
    with open(path, 'w') as f:
        for v in vs:
            f.write('v %.5f %.5f %.5f\n' % v)
        for t in fs:
            f.write('f %d %d %d\n' % t)


def esc(s):
    return s.replace(']]>', ']]]]><![CDATA[>')


class W:
    def __init__(self):
        self.out, self.n = [], 0

    def ref(self):
        self.n += 1
        return 'R%d' % self.n

    def open(self, cls, props):
        self.out.append('<Item class="%s" referent="%s"><Properties>%s</Properties>' % (cls, self.ref(), ''.join(props)))

    def close(self):
        self.out.append('</Item>')

    def leaf(self, cls, props):
        self.open(cls, props)
        self.close()


def s(n, v): return '<string name="%s">%s</string>' % (n, v)
def b(n, v): return '<bool name="%s">%s</bool>' % (n, 'true' if v else 'false')
def tok(n, v): return '<token name="%s">%d</token>' % (n, v)
def v3(n, x, y, z): return '<Vector3 name="%s"><X>%r</X><Y>%r</Y><Z>%r</Z></Vector3>' % (n, x, y, z)
def url(n, v): return '<Content name="%s"><url>%s</url></Content>' % (n, v)
def src(v): return '<ProtectedString name="Source"><![CDATA[%s]]></ProtectedString>' % esc(v)


def cf(n, x, y, z, ry=0.0, rx=0.0):
    cy, sy, cx, sx = math.cos(ry), math.sin(ry), math.cos(rx), math.sin(rx)
    # R = Ry * Rx
    m = [[cy, sy * sx, sy * cx], [0, cx, -sx], [-sy, cy * sx, cy * cx]]
    return ('<CoordinateFrame name="%s"><X>%r</X><Y>%r</Y><Z>%r</Z>' % (n, x, y, z) +
            ''.join('<R%d%d>%r</R%d%d>' % (i, j, m[i][j], i, j) for i in range(3) for j in range(3)) + '</CoordinateFrame>')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--parts', type=int, default=3000)
    a = ap.parse_args()
    out = a.out
    (out / 'meshes').mkdir(parents=True, exist_ok=True)
    (out / '.rustudio-local-content').write_text('Files in this folder are place content for the .rbxlx here.\n')
    for k in range(8):
        obj_rock(out / 'meshes' / ('rock%d.obj' % k), k)
    r = random.Random(1234)
    w = W()
    w.out.append('<roblox version="4">')
    w.open('Workspace', [s('Name', 'Workspace')])
    # The level: a 40 x 40 grid of 20-stud cells, pieces at several heights.
    w.open('Model', [s('Name', 'Level')])
    side = int(math.ceil(math.sqrt(a.parts)))
    for i in range(a.parts):
        gx, gz = i % side, i // side
        x = (gx - side / 2) * 20 + r.uniform(-6, 6)
        z = (gz - side / 2) * 20 + r.uniform(-6, 6)
        y = r.choice([2, 2, 2, 6, 12, 30])
        sx, sy, sz = r.uniform(2, 18), r.uniform(1, 10), r.uniform(2, 18)
        kind = i % 10
        query = kind in (0, 3, 6)  # queryable 30 %; the rest are render pieces (no collide, no query)
        cls = 'Part'
        props = [s('Name', 'p%d' % i), b('Anchored', True), b('CanCollide', query), b('CanQuery', query),
                 b('CanTouch', False), v3('size', sx, sy, sz), cf('CFrame', x, y, z, r.uniform(0, 6.28) if kind % 2 else 0.0)]
        if query and i % 30 == 0:
            props.append(tok('shape', 0))  # Ball
        elif query and i % 30 == 3:
            props.append(tok('shape', 2))  # Cylinder
        if query and i % 30 == 6:
            cls = 'WedgePart'
        if query and i % 60 == 3:
            props.append(s('CollisionGroup', 'G2'))
        if query and i % 50 == 6:
            props.append(tok('Material', 1088))  # Glass
        w.open(cls, props)
        if not query or i % 4 == 0:
            w.leaf('SpecialMesh', [s('Name', 'Mesh'), tok('MeshType', 5), url('MeshId', 'meshes/rock%d.obj' % (i % 8)),
                                   v3('Scale', 1.0, 1.0, 1.0)])
        w.close()
    w.close()
    # The actor: parts, a rig part with nested bones, attachments.
    w.open('Model', [s('Name', 'Actor')])
    w.leaf('Part', [s('Name', 'Body'), b('Anchored', True), v3('size', 2.0, 5.0, 2.0), cf('CFrame', 0.0, 40.0, 0.0)])
    w.open('Part', [s('Name', 'Rig'), b('Anchored', True), b('CanCollide', False), v3('size', 1.0, 1.0, 1.0),
                    cf('CFrame', 0.0, 45.0, 0.0)])
    for c in range(30):  # 30 chains of 10 bones
        for d in range(10):
            w.open('Bone', [s('Name', 'b%d_%d' % (c, d)), cf('CFrame', 0.0, 0.3, 0.0)])
        for d in range(10):
            w.close()
    for k in range(600):
        w.leaf('Attachment', [s('Name', 'a%d' % k), v3('Position', 0.0, 0.0, 0.0)])
    w.close()
    for k in range(50):
        w.leaf('Part', [s('Name', 'm%d' % k), b('Anchored', True), b('CanCollide', False), b('CanQuery', False),
                        v3('size', 1.0, 1.0, 1.0), cf('CFrame', 0.0, 50.0 + k, 0.0)])
    w.close()
    w.close()  # Workspace
    w.open('ServerScriptService', [s('Name', 'ServerScriptService')])
    w.leaf('Script', [s('Name', 'Setup'), src(open(HERE / 'setup.server.luau').read())])
    w.close()
    w.open('StarterPlayer', [s('Name', 'StarterPlayer')])
    w.open('StarterPlayerScripts', [s('Name', 'StarterPlayerScripts')])
    w.leaf('LocalScript', [s('Name', 'Probe'), src(open(HERE / 'probe.luau').read())])
    w.close()
    w.close()
    w.out.append('</roblox>')
    (out / 'callcosts.rbxlx').write_text('\n'.join(w.out))
    print(out / 'callcosts.rbxlx')


if __name__ == '__main__':
    main()
