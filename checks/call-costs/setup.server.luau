-- call-costs: terrain (grass with a water pool) and a collision group that doesn't collide with Default.
local PhysicsService = game:GetService("PhysicsService")
pcall(function() PhysicsService:RegisterCollisionGroup("G2") end)
PhysicsService:CollisionGroupSetCollidable("G2", "Default", false)
local t = workspace.Terrain
t:FillBlock(CFrame.new(0, -6, 0), Vector3.new(512, 8, 512), Enum.Material.Grass)
t:FillBlock(CFrame.new(60, -3, 60), Vector3.new(40, 6, 40), Enum.Material.Water)
workspace:SetAttribute("CallCostsReady", true)
