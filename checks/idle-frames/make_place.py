#!/usr/bin/env python3
"""A large synthetic place for idle-frame checks: N instances (default
300 000), most of them cheap (Folders, values, GUI frames) so it loads fast,
plus a few thousand anchored parts in Models, some Highlights and lights.

    make_place.py OUT.rbxlx [--instances N] [--see-through]

--see-through: one small half-transparent part in the middle of the grid
(the GPU case of check.py: see-through work bounded by what it covers).
"""
import argparse

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("--instances", type=int, default=300_000)
ap.add_argument("--see-through", action="store_true")
a = ap.parse_args()

ref = 0


def r():
    global ref
    ref += 1
    return f"R{ref}"


def cf(x, y, z):
    return (f"<CoordinateFrame name=\"CFrame\"><X>{x}</X><Y>{y}</Y><Z>{z}</Z>"
            "<R00>1</R00><R01>0</R01><R02>0</R02><R10>0</R10><R11>1</R11><R12>0</R12>"
            "<R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>")


def item(cls, name, props="", kids=""):
    return (f"<Item class=\"{cls}\" referent=\"{r()}\"><Properties><string name=\"Name\">{name}</string>"
            f"{props}</Properties>{kids}</Item>")


def part(name, x, y, z, s=4):
    return item("Part", name, "<bool name=\"Anchored\">true</bool>" + cf(x, y, z)
                + f"<Vector3 name=\"size\"><X>{s}</X><Y>{s}</Y><Z>{s}</Z></Vector3>")


out = []
count = 0
# Workspace: 40 Models x 100 parts on a grid, a few Highlights and lights.
models = []
for m in range(40):
    parts = []
    for p in range(100):
        i = m * 100 + p
        x, z = (i % 64) * 6 - 192, (i // 64) * 6 - 192
        kid = ""
        if p == 0:
            kid = item("PointLight", "Light", "<float name=\"Range\">12</float>")
            count += 1
        parts.append(part(f"P{p}", x, 2, z)[:-len("</Item>")] + kid + "</Item>")
        count += 1
    hl = item("Highlight", "Highlight") if m % 10 == 0 else ""
    count += 1 + (1 if hl else 0)
    models.append(item("Model", f"Model{m}", "", "".join(parts) + hl))
if a.see_through:
    glass = part("SeeThrough", 3, 8, 3, 2)[:-len("</Item>")]
    glass = glass.replace("</Properties>", "<float name=\"Transparency\">0.5</float></Properties>") + "</Item>"
    models.append(glass)
    count += 1
base = part("Baseplate", 0, -0.5, 0)
base = base.replace("<X>4</X><Y>4</Y><Z>4</Z></Vector3>", "<X>512</X><Y>1</Y><Z>512</Z></Vector3>")
count += 2
ws = item("Workspace", "Workspace", "", base + "".join(models))

# ReplicatedStorage: the bulk, nested Folders of values.
left = max(0, a.instances - count - 300)
folders = []
per = 200
while left > 0:
    n = min(per, left - 1)
    vals = "".join(item("StringValue", f"V{j}", f"<string name=\"Value\">v{j}</string>") for j in range(n))
    folders.append(item("Folder", f"F{len(folders)}", "", vals))
    left -= n + 1
    count += n + 1
rs = item("ReplicatedStorage", "ReplicatedStorage", "", "".join(folders))
count += 1

# StarterGui: a ScreenGui with 250 frames.
frames = "".join(item("Frame", f"Frame{j}") for j in range(250))
sg = item("StarterGui", "StarterGui", "", item("ScreenGui", "Gui", "", frames))
count += 252

light = item("Lighting", "Lighting")
count += 1
with open(a.out, "w") as f:
    f.write("<roblox version=\"4\">" + ws + rs + sg + light + "</roblox>")
print(f"{a.out}: {count} instances")
