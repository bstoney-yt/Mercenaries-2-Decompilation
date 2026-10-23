--[[ Test driver for PmcCon004 (Solano's mountain bunker): takes her up
the mountain the way a player with a helicopter would. Autoplay drives
from the PMC towards the bunker on Angel Falls' peak, 160 m up, but its
route finder never finds the way up (round 3: still 1757 m off after
3000 s). So once "destroy the bunker" is up and she is still over 400 m
from it, this puts her at whichever of the scripts' own places round the
base (`loc_BunkerDoor`, `loc_solattack1`..4, `loc_spawnmans`) lies nearest
the bunker, a little above the ground there. Everything after is left to
autoplay. It logs where those places are.

Load it next to pmc.lua:

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 2400 \
    --skip-to PmcCon004 --mission-driver .../pmc.lua \
    --mission-driver crates/m2-game/examples/drivers/pmccon004.lua ]]

local function say(m) Debug.Printf('>>> PMC004 ' .. m) end

local places = { 'loc_BunkerDoor', 'loc_solattack1', 'loc_solattack2', 'loc_solattack3',
  'loc_solattack4', 'loc_spawnmans' }

local function at(name)
  local g = Pg.GetGuidByName(name)
  if not g then return nil end
  local x, y, z = Object.GetPosition(g)
  if not x then return nil end
  return x, y, z
end

local function bunker_up()
  local m = MrxPlayState and MrxPlayState.GetCurrentMission and MrxPlayState.GetCurrentMission()
  if type(m) ~= 'table' then return false end
  local ok, kids = pcall(function() return m:GetChildren() end)
  for _, c in pairs(ok and kids or {}) do
    local okA, a = pcall(function() return c:IsActive() end)
    if okA and a and c:GetName() == 'DestroyBunker' then return true end
  end
  return false
end

local tries, logged = 0, false
Event.CreatePersistent(Event.TimerRelative, { 2 }, function()
  if not bunker_up() then return end
  local bx, by, bz = at('Solano_Bunker_PMC004')
  if not bx then return end
  if not logged then
    logged = true
    for _, n in ipairs(places) do
      local x, y, z = at(n)
      say(n .. ' at ' .. tostring(x) .. ', ' .. tostring(y) .. ', ' .. tostring(z))
    end
  end
  local p = Player.GetLocalCharacter()
  local px, py, pz = Object.GetPosition(p)
  if not px then return end
  local d = math.sqrt((bx - px) ^ 2 + (bz - pz) ^ 2)
  if d < 400 or tries >= 3 then return end
  local best, bd
  for _, n in ipairs(places) do
    local x, y, z = at(n)
    if x then
      local e = math.sqrt((bx - x) ^ 2 + (bz - z) ^ 2)
      if not bd or e < bd then best, bd = { x, y, z, n }, e end
    end
  end
  if not best then return end
  tries = tries + 1
  local v = Vehicle.GetFromRider(p)
  if v ~= nil and type(v) ~= 'table' then Vehicle.Exit(p) end
  say(string.format('%.0f m from the bunker: put at %s (%.0f m from it)', d, best[4], bd))
  Object.SetPosition(p, best[1], best[2] + 1.5, best[3])
end, {})
