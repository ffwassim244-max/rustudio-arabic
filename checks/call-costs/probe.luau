-- call-costs probe (LocalScript; see README.md). Prints:
--   Q ...   world query results (rays, shapecasts, overlaps, legacy FindPartOnRay*) on a fixed set of inputs
--   S ...   Changed / GetPropertyChangedSignal / game.ItemChanged behaviour of property writes
--   V ...   value type behaviour (tostring, typeof, equality, table keys, arithmetic)
--   MB name value   per-call cost: ns under RUSTUDIO_CPU_CLOCK=1, instructions under RUSTUDIO_INSTR_CLOCK=1
-- and "CALLCOSTS DONE" at the end. Q / S / V lines must not change between builds; MB lines are the costs.
local RunService = game:GetService("RunService")
local t0 = os.clock()
while not workspace:GetAttribute("CallCostsReady") do task.wait(0.1) end
task.wait(1)
local Level = workspace:WaitForChild("Level")
local Actor = workspace:WaitForChild("Actor")
local Rig = Actor:WaitForChild("Rig")
local Body = Actor:WaitForChild("Body")
local function out(s) print(s) end
local function f3(v) return string.format("%.4f,%.4f,%.4f", v.X, v.Y, v.Z) end

-- ------------------------------------------------------------- query records
local rng = Random.new(7)
local queryable = {}
for _, p in Level:GetChildren() do
	if p:IsA("BasePart") and p.CanQuery then table.insert(queryable, p) end
end
table.sort(queryable, function(a, b) return a.Name < b.Name end)
local subset = {}
for i = 1, #queryable, math.max(1, #queryable // 50) do table.insert(subset, queryable[i]) end
local function rp(ft, list, extra)
	local p = RaycastParams.new()
	p.FilterType = ft
	p.FilterDescendantsInstances = list
	for k, v in extra or {} do p[k] = v end
	return p
end
local EX, IN = Enum.RaycastFilterType.Exclude, Enum.RaycastFilterType.Include
local RPS = {
	{ "nil", nil },
	{ "ex-actor", rp(EX, { Actor }) },
	{ "ex-empty", rp(EX, {}) },
	{ "in-subset", rp(IN, subset) },
	{ "in-level", rp(IN, { Level }) },
	{ "ex-actor-collide", rp(EX, { Actor }, { RespectCanCollide = true }) },
	{ "ignore-water", rp(EX, { Actor }, { IgnoreWater = true }) },
	{ "group-g2", rp(EX, {}, { CollisionGroup = "G2" }) },
}
local function hitline(h)
	if not h then return "nil" end
	return string.format("%s %s %s %s %.4f", h.Instance:GetFullName(), f3(h.Position), f3(h.Normal), h.Material.Name, h.Distance)
end
local rays = {}
for i = 1, 160 do
	local o = Vector3.new(rng:NextNumber(-420, 420), rng:NextNumber(0, 60), rng:NextNumber(-420, 420))
	local d
	if i % 3 == 0 then
		d = Vector3.new(rng:NextNumber(-1, 1), rng:NextNumber(-1, 0.3), rng:NextNumber(-1, 1)).Unit * 200
	elseif i % 3 == 1 then
		d = Vector3.new(0, -80, 0)
	else
		d = Vector3.new(0, -36, 0)
	end
	if i % 16 == 0 then o = Vector3.new(60 + rng:NextNumber(-15, 15), 10, 60 + rng:NextNumber(-15, 15)) d = Vector3.new(0, -40, 0) end -- over water
	rays[i] = { o, d }
end
for i, r in rays do
	for pi, p in RPS do
		out(string.format("Q ray %d %s %s", i, p[1], hitline(workspace:Raycast(r[1], r[2], p[2]))))
	end
end
local probePart = Instance.new("Part")
probePart.Name = "ProbeShape"
probePart.Anchored = true
probePart.CanCollide = false
probePart.CanQuery = false
probePart.Size = Vector3.new(2, 3, 2)
probePart.Parent = Actor
for i = 1, 60 do
	local r = rays[i]
	for pi, p in { RPS[1], RPS[2], RPS[4], RPS[8] } do
		local ok, h = pcall(workspace.Spherecast, workspace, r[1], 1.5, r[2], p[2])
		out(string.format("Q sphere %d %s %s", i, p[1], ok and hitline(h) or ("error " .. tostring(h))))
		ok, h = pcall(workspace.Blockcast, workspace, CFrame.new(r[1]) * CFrame.Angles(0, i, 0), Vector3.new(3, 2, 4), r[2], p[2])
		out(string.format("Q block %d %s %s", i, p[1], ok and hitline(h) or ("error " .. tostring(h))))
		probePart.CFrame = CFrame.new(r[1]) * CFrame.Angles(0.2, i, 0)
		ok, h = pcall(workspace.Shapecast, workspace, probePart, r[2], p[2])
		out(string.format("Q shape %d %s %s", i, p[1], ok and hitline(h) or ("error " .. tostring(h))))
	end
end
local function op(ft, list, extra)
	local p = OverlapParams.new()
	p.FilterType = ft
	p.FilterDescendantsInstances = list
	for k, v in extra or {} do p[k] = v end
	return p
end
local OPS = {
	{ "nil", nil },
	{ "ex-actor", op(EX, { Actor }) },
	{ "in-subset", op(IN, subset) },
	{ "collide", op(EX, {}, { RespectCanCollide = true }) },
	{ "group-g2", op(EX, {}, { CollisionGroup = "G2" }) },
	{ "max5", op(EX, {}, { MaxParts = 5 }) },
}
local function names(list)
	local n = {}
	for _, p in list do table.insert(n, p:GetFullName()) end
	table.sort(n)
	local head = {}
	for k = 1, math.min(#n, 24) do head[k] = n[k] end
	return #list .. " " .. table.concat(head, ",")
end
for i = 1, 40 do
	local c = rays[i][1] + Vector3.new(0, -20, 0)
	for _, p in OPS do
		local cfb = CFrame.new(c) * CFrame.Angles(0, i * 0.3, 0)
		out(string.format("Q box %d %s %s", i, p[1], names(workspace:GetPartBoundsInBox(cfb, Vector3.new(30, 25, 20), p[2]))))
		out(string.format("Q radius %d %s %s", i, p[1], names(workspace:GetPartBoundsInRadius(c, 18, p[2]))))
		probePart.CFrame = cfb
		probePart.Size = Vector3.new(24, 20, 24)
		out(string.format("Q inpart %d %s %s", i, p[1], names(workspace:GetPartsInPart(probePart, p[2]))))
		probePart.Size = Vector3.new(2, 3, 2)
	end
end
for i = 1, 60 do
	local r = rays[i]
	local ray = Ray.new(r[1], r[2])
	local function leg(name, ...)
		local h, pos, n, m = workspace[name](workspace, ...)
		out(string.format("Q %s %d %s %s %s %s", name, i, h and h:GetFullName() or "nil", f3(pos), f3(n), tostring(m)))
	end
	leg("FindPartOnRay", ray, Actor)
	leg("FindPartOnRayWithIgnoreList", ray, { Actor })
	leg("FindPartOnRayWithWhitelist", ray, subset)
	leg("FindPartOnRay", ray, nil, false, true)
end

-- ------------------------------------------------------------- signal records
do
	local part = queryable[1]
	local bone = Rig:FindFirstChild("b0_0")
	local att = Rig:FindFirstChild("a0")
	local n = { changed = 0, cfsig = 0, bone = 0, att = 0, item = 0, itemCF = 0 }
	local c1 = part.Changed:Connect(function(p) if p == "CFrame" then n.changed += 1 end end)
	local c2 = part:GetPropertyChangedSignal("CFrame"):Connect(function() n.cfsig += 1 end)
	local c3 = bone:GetPropertyChangedSignal("CFrame"):Connect(function() n.bone += 1 end)
	local c4 = att.Changed:Connect(function(p) if p == "Position" then n.att += 1 end end)
	local base = part.CFrame
	for k = 1, 3 do part.CFrame = base * CFrame.new(0, k * 0.01, 0) end
	part.CFrame = part.CFrame -- same value: no fire
	for k = 1, 3 do bone.CFrame = CFrame.new(0, 0.3 + k * 0.01, 0) end
	for k = 1, 3 do att.Position = Vector3.new(k, 0, 0) end
	part.Position = part.Position + Vector3.new(0, 0.02, 0)
	task.wait()
	out(string.format("S writes changed=%d cfsig=%d bone=%d att=%d", n.changed, n.cfsig, n.bone, n.att))
	-- game.ItemChanged: fires for every change while connected, not after.
	local dc = game.DescendantAdded:Connect(function() end) -- the DataModel listened to, but not ItemChanged
	part.Transparency = 0.25
	local ic = game.ItemChanged:Connect(function(inst, prop)
		if inst == part then n.item += 1 if prop == "CFrame" then n.itemCF += 1 end end
	end)
	part.Transparency = 0.5
	part.CFrame = base
	task.wait()
	out(string.format("S itemchanged connected item=%d cf=%d", n.item, n.itemCF))
	ic:Disconnect()
	task.wait()
	part.Transparency = 0
	task.wait()
	out(string.format("S itemchanged after disconnect item=%d", n.item))
	local th = task.spawn(function()
		local inst, prop = game.ItemChanged:Wait()
		out(string.format("S itemchanged wait %s %s", inst == part and "part" or tostring(inst), tostring(prop)))
	end)
	part.Transparency = 0.1
	task.wait()
	part.Transparency = 0
	dc:Disconnect() c1:Disconnect() c2:Disconnect() c3:Disconnect() c4:Disconnect()
	-- a moved part is where rays find it
	local moved = queryable[2]
	local old = moved.CFrame
	moved.CFrame = CFrame.new(1000, 500, 1000)
	out("S moved ray " .. hitline(workspace:Raycast(Vector3.new(1000, 520, 1000), Vector3.new(0, -40, 0))))
	moved.CFrame = old
	out("S moved back ray " .. hitline(workspace:Raycast(Vector3.new(1000, 520, 1000), Vector3.new(0, -40, 0))))
	task.wait()
end

-- ------------------------------------------------------------- value type records
do
	local a = CFrame.new(1, 2, 3) * CFrame.Angles(0.1, 0.2, 0.3)
	local b = CFrame.fromEulerAnglesYXZ(0.5, -0.4, 1.2) + Vector3.new(4, -5, 6)
	local v = Vector3.new(1.5, -2.25, 3)
	out("V tostring " .. tostring(a) .. " | " .. tostring(b))
	out("V mul " .. tostring(a * b) .. " | " .. tostring(a:Inverse()) .. " | " .. tostring(a * v))
	out("V comps " .. table.concat({ a:GetComponents() }, ","))
	out("V lerp " .. tostring(a:Lerp(b, 0.3)) .. " | " .. tostring(a:ToObjectSpace(b)) .. " | " .. tostring(a:PointToObjectSpace(v)))
	out("V props " .. tostring(a.Position) .. " " .. tostring(a.LookVector) .. " " .. tostring(a.RightVector) .. " " .. tostring(a.UpVector) .. " " .. a.X .. " " .. a.Y .. " " .. a.Z)
	out("V typeof " .. typeof(a) .. " " .. typeof(v) .. " " .. typeof(a.Position) .. " " .. typeof(Color3.new()) .. " " .. typeof(UDim2.new()))
	out("V eq " .. tostring(CFrame.new(1, 2, 3) == CFrame.new(1, 2, 3)) .. " " .. tostring(a == b) .. " " .. tostring(rawequal(CFrame.new(), CFrame.new())) .. " " .. tostring(Vector3.new(1, 2, 3) == Vector3.new(1, 2, 3)))
	local t = {}
	t[CFrame.new(1, 2, 3)] = 1
	t[CFrame.new(1, 2, 3)] = 2
	t[Vector3.new(1, 2, 3)] = 3
	t[Vector3.new(1, 2, 3)] = 4
	local n = 0
	for _ in t do n += 1 end
	out("V keys " .. n .. " " .. tostring(t[Vector3.new(1, 2, 3)]) .. " " .. tostring(t[CFrame.new(1, 2, 3)]))
	out("V angles " .. tostring(CFrame.Angles(1, 2, 3)) .. " | " .. tostring(CFrame.fromAxisAngle(Vector3.new(0, 1, 1).Unit, 0.7)) .. " | " .. tostring(CFrame.lookAt(Vector3.new(1, 2, 3), Vector3.new(-4, 5, 9))))
	out("V ops " .. tostring(a + v) .. " | " .. tostring(a - v) .. " | " .. tostring(CFrame.new(v)) .. " | " .. tostring(CFrame.new(1, 2, 3, 0, 0, 0, 1)))
	out("V part " .. tostring(Body.CFrame) .. " " .. tostring(Body.Position) .. " " .. tostring(Body.CFrame == Body.CFrame) .. " " .. tostring(rawequal(Body.CFrame, Body.CFrame)))
	local ok, e = pcall(function() return CFrame.new(1, 2, 3) * "x" end)
	out("V err " .. tostring(ok) .. " " .. tostring(e):gsub("^.-:%d+: ", ""))
	ok, e = pcall(function() return CFrame.new("a") end)
	out("V err2 " .. tostring(ok) .. " " .. tostring(e):gsub("^.-:%d+: ", ""))
end
out("CALLCOSTS records done")

-- ------------------------------------------------------------- costs
local results = {}
local function bench(name, f, n)
	f(math.max(1, n // 20)) -- warm
	local c = os.clock()
	f(n)
	local dt = os.clock() - c
	table.insert(results, string.format("MB %-40s %10.1f", name, dt / n * 1e9))
end
local K = tonumber(workspace:GetAttribute("CallCostsScale")) or 1
local origin = Vector3.new(13, 30, -27)
local exActor = RPS[2][2]
local exEmpty = RPS[3][2]
-- R1: world queries
bench("Raycast ex-actor 36 down", function(n) for i = 1, n do workspace:Raycast(origin, Vector3.new(0, -36, 0), exActor) end end, 20000 * K)
bench("Raycast nil 36 down", function(n) for i = 1, n do workspace:Raycast(origin, Vector3.new(0, -36, 0)) end end, 20000 * K)
bench("Raycast ex-empty 1200 down", function(n) for i = 1, n do workspace:Raycast(origin, Vector3.new(0, -1200, 0), exEmpty) end end, 10000 * K)
bench("Raycast ex-actor diagonal 200", function(n) local d = Vector3.new(150, -60, 120) for i = 1, n do workspace:Raycast(origin, d, exActor) end end, 10000 * K)
bench("Spherecast ex-actor", function(n) for i = 1, n do workspace:Spherecast(origin, 1.5, Vector3.new(0, -36, 0), exActor) end end, 5000 * K)
bench("Blockcast ex-actor", function(n) local c = CFrame.new(origin) for i = 1, n do workspace:Blockcast(c, Vector3.new(3, 2, 4), Vector3.new(0, -36, 0), exActor) end end, 5000 * K)
probePart.CFrame = CFrame.new(origin)
bench("Shapecast ex-actor", function(n) for i = 1, n do workspace:Shapecast(probePart, Vector3.new(0, -36, 0), exActor) end end, 5000 * K)
local opEx = OPS[2][2]
bench("GetPartBoundsInBox ex-actor", function(n) local c = CFrame.new(origin) for i = 1, n do workspace:GetPartBoundsInBox(c, Vector3.new(30, 60, 30), opEx) end end, 5000 * K)
bench("GetPartBoundsInRadius ex-actor", function(n) for i = 1, n do workspace:GetPartBoundsInRadius(origin, 20, opEx) end end, 5000 * K)
probePart.Size = Vector3.new(24, 60, 24)
bench("GetPartsInPart ex-actor", function(n) for i = 1, n do workspace:GetPartsInPart(probePart, opEx) end end, 3000 * K)
probePart.Size = Vector3.new(2, 3, 2)
bench("FindPartOnRayWithIgnoreList", function(n) local r = Ray.new(origin, Vector3.new(0, -36, 0)) local l = { Actor } for i = 1, n do workspace:FindPartOnRayWithIgnoreList(r, l) end end, 10000 * K)
-- R3: property writes and reads
local bones, atts, movers = {}, {}, {}
for _, d in Rig:GetDescendants() do
	if d:IsA("Bone") then table.insert(bones, d) elseif d:IsA("Attachment") then table.insert(atts, d) end
end
for k = 0, 49 do table.insert(movers, Actor:FindFirstChild("m" .. k)) end
-- (the last ones: the records above connected to the first bone and attachment)
local bone, att, mover = bones[#bones], atts[#atts], movers[#movers]
bench("set Bone.CFrame", function(n) for i = 1, n do bone.CFrame = CFrame.new(0, 0.3 + (i % 7) * 0.01, 0) end end, 50000 * K)
bench("set Attachment.Position", function(n) for i = 1, n do att.Position = Vector3.new(i % 7, 0, 0) end end, 50000 * K)
bench("set Part.CFrame (anchored)", function(n) for i = 1, n do mover.CFrame = CFrame.new(i % 7, 50, 0) end end, 50000 * K)
bench("set Part.Position (anchored)", function(n) for i = 1, n do mover.Position = Vector3.new(i % 7, 50, 0) end end, 50000 * K)
bench("set Part.Transparency", function(n) for i = 1, n do mover.Transparency = (i % 2) * 0.5 end end, 50000 * K)
do
	local lb = bones[#bones - 1]
	local hits = 0
	local cn = lb.Changed:Connect(function() hits += 1 end)
	bench("set Bone.CFrame (Changed connected)", function(n) for i = 1, n do lb.CFrame = CFrame.new(0, 0.3 + (i % 7) * 0.01, 0) end end, 20000 * K)
	cn:Disconnect()
end
bench("get Part.CFrame", function(n) for i = 1, n do local x = mover.CFrame end end, 100000 * K)
bench("get Part.Position", function(n) for i = 1, n do local x = mover.Position end end, 100000 * K)
bench("get Bone.CFrame", function(n) for i = 1, n do local x = bone.CFrame end end, 100000 * K)
-- R1 + R3: a frame of writes, then the frame's first ray
do
	local writes, ray, frames = 0, 0, 0
	for f = 1, 90 * K do
		RunService.Heartbeat:Wait()
		local c = os.clock()
		for i, b in bones do b.CFrame = CFrame.new(0, 0.3 + (f % 5) * 0.01, 0) * CFrame.Angles(0, f * 0.01, 0) end
		for i, b in bones do b.CFrame = CFrame.new(0, 0.3 + (f % 3) * 0.01, 0) end
		for i, a in atts do a.Position = Vector3.new(f % 9, i % 3, 0) end
		for i = 1, 300 do atts[i].Position = Vector3.new(0, f % 4, 1) end
		for i, m in movers do m.CFrame = CFrame.new(i, 50 + f % 6, 0) end
		local c2 = os.clock()
		workspace:Raycast(origin, Vector3.new(0, -36, 0), exActor)
		ray += os.clock() - c2
		writes += c2 - c
		frames += 1
	end
	local nw = #bones * 2 + #atts + 300 + #movers
	table.insert(results, string.format("MB %-40s %10.1f", "frame of " .. nw .. " writes (per write)", writes / frames / nw * 1e9))
	table.insert(results, string.format("MB %-40s %10.1f", "Raycast after a frame of writes", ray / frames * 1e9))
end
-- R2: value types
bench("Vector3.new", function(n) for i = 1, n do local v = Vector3.new(i, 2, 3) end end, 200000 * K)
bench("CFrame.new(x,y,z)", function(n) for i = 1, n do local c = CFrame.new(i, 2, 3) end end, 200000 * K)
bench("CFrame.Angles", function(n) for i = 1, n do local c = CFrame.Angles(i, 0.2, 0.3) end end, 200000 * K)
bench("CFrame.new*CFrame.Angles", function(n) for i = 1, n do local c = CFrame.new(i, 2, 3) * CFrame.Angles(0.1, i, 0.3) end end, 200000 * K)
bench("CFrame * CFrame", function(n) local a, b = CFrame.new(1, 2, 3), CFrame.Angles(0, 1, 0) for i = 1, n do local c = a * b end end, 200000 * K)
bench("CFrame * Vector3", function(n) local a, v = CFrame.Angles(0, 1, 0), Vector3.new(1, 2, 3) for i = 1, n do local c = a * v end end, 200000 * K)
bench("CFrame:Inverse", function(n) local a = CFrame.Angles(0, 1, 0) for i = 1, n do local c = a:Inverse() end end, 200000 * K)
bench("CFrame:ToObjectSpace", function(n) local a, b = CFrame.Angles(0, 1, 0), CFrame.new(1, 2, 3) for i = 1, n do local c = a:ToObjectSpace(b) end end, 200000 * K)
bench("CFrame:PointToObjectSpace", function(n) local a, v = CFrame.Angles(0, 1, 0), Vector3.new(1, 2, 3) for i = 1, n do local c = a:PointToObjectSpace(v) end end, 200000 * K)
bench("CFrame.Position", function(n) local a = CFrame.new(1, 2, 3) for i = 1, n do local p = a.Position end end, 200000 * K)
bench("CFrame.LookVector", function(n) local a = CFrame.Angles(1, 2, 3) for i = 1, n do local p = a.LookVector end end, 200000 * K)
bench("CFrame.X", function(n) local a = CFrame.new(1, 2, 3) for i = 1, n do local p = a.X end end, 200000 * K)
bench("CFrame:Lerp", function(n) local a, b = CFrame.new(1, 2, 3), CFrame.Angles(0, 1, 0) for i = 1, n do local c = a:Lerp(b, 0.5) end end, 100000 * K)
bench("Color3.new", function(n) for i = 1, n do local c = Color3.new(0.1, 0.2, 0.3) end end, 200000 * K)
bench("UDim2.new", function(n) for i = 1, n do local u = UDim2.new(0, i, 0, 5) end end, 200000 * K)
probePart:Destroy()
for _, r in results do print(r) end
print("CALLCOSTS DONE")
