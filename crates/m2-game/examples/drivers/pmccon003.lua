--[[ Test driver for PmcCon003 (Solano's bunker, then back to the PMC):
stands in where autoplay can't play it the way a player would: the timed
race, the tank waves still standing after 600 s (killed), and the walk to
the bunker if she is stuck on the hillside 600 s (put 150 m short of it).
For the race: Autoplay
drives the 5.7 km back from the bunker through the jungle too slowly for
the scripts' 600 s timer (round 7: the contract was cancelled at "Time
UP!"), so once "Race back to the base in time!" is active this puts her
into its region, Region_PMC003_MansionApproach (away first, so the region
sees her come in). Everything else is left to autoplay.

Load it next to pmc.lua:

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 2700 \
    --skip-to PmcCon003 --mission-driver .../pmc.lua \
    --mission-driver crates/m2-game/examples/drivers/pmccon003.lua ]]

local acts = {
  { 'Race back to the base in time!', 'Region_PMC003_MansionApproach' },
}

local active, tries = {}, {}
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if ok and type(nm) == 'string' then
    for _, a in ipairs(acts) do
      if nm:find(a[1], 1, true) then
        active[a] = o:IsActive() or nil
      end
    end
  end
  return unpack(r, 1, table.maxn(r))
end

local function put(x, y, z)
  local p = Player.GetLocalCharacter()
  local v = Vehicle.GetFromRider(p)
  if v ~= nil and type(v) ~= 'table' then Vehicle.Exit(p) end
  Object.SetPosition(p, x, y, z)
end

Event.CreatePersistent(Event.TimerRelative, { 1 }, function()
  for a in pairs(active) do
    local t = (tries[a] or 0) + 1
    tries[a] = t
    if t <= 40 and t % 4 == 1 then
      local g = Pg.GetGuidByName(a[2])
      local x, y, z
      if g then x, y, z = Object.GetPosition(g) end
      if not x then
        Debug.Printf('>>> drive ' .. a[1] .. ': no position for ' .. a[2])
      else
        Debug.Printf('>>> drive ' .. a[1] .. ': away, then into ' .. a[2])
        put(x + 60, y, z + 60)
        Event.Create(Event.TimerRelative, { 0.5 }, function() put(x, y, z) end, {})
      end
    end
  end
end, {})

-- The PMC's two tank waves ("Clear out the VZ forces attacking the PMC!"):
-- autoplay goes for them first; any tank still alive 600 s into its
-- wave (polled every 10 s) is killed, so a run gets on to "Get inside the
-- PMC!" (autoplay took 1900 s on one moving tank in round 14 and never
-- finished the second wave in round 19).
local tanks = {
  'PMC003_AMX_01', 'PMC003_AMX_02', 'PMC003_AMX_03', 'PMC003_AMX_04',
  'OBJ_Wave02_Tank01', 'OBJ_Wave02_Tank02', 'OBJ_Wave02_Tank03', 'OBJ_Wave02_Tank04',
}
-- Whether a task whose name holds `name` is active under the current contract
-- (polled: a task module re-imported for the second wave no longer goes
-- through a wrapped MrxTask._SetState, round 29).
local function active(name)
  local ok, m = pcall(function() return MrxPlayState.GetCurrentMission() end)
  local found = false
  local function walk(t, depth)
    if found or depth > 6 or type(t) ~= 'table' then return end
    local okk, kids = pcall(function() return t:GetChildren() end)
    if not okk or type(kids) ~= 'table' then return end
    for _, c in pairs(kids) do
      local okn, nm = pcall(function() return c:GetName() end)
      local oka, act = pcall(function() return c:IsActive() end)
      if okn and oka and act and type(nm) == 'string' and nm:find(name, 1, true) then
        found = true
        return
      end
      walk(c, depth + 1)
    end
  end
  if ok then walk(m, 0) end
  return found
end

local waited = 0
Event.CreatePersistent(Event.TimerRelative, { 10 }, function()
  local alive = {}
  for _, k in ipairs(tanks) do
    local g = Pg.GetGuidByName(k)
    if g and Object.IsAlive(g) then table.insert(alive, { k, g }) end
  end
  local on = active('Clear out the VZ forces')
  if waited % 60 == 0 and (on or #alive > 0) then
    Debug.Printf('>>> drive clear: ' .. #alive .. ' alive, wave ' .. tostring(on) .. ', ' .. waited .. ' s')
  end
  if #alive == 0 or not on then
    waited = 0
    return
  end
  waited = waited + 10
  if waited < 600 then return end
  for _, a in ipairs(alive) do
    Debug.Printf('>>> drive clear: kill ' .. a[1] .. ' after ' .. waited .. ' s')
    Object.Kill(a[2])
  end
  waited = 0
end, {})

-- "Deploy Bunker Buster on Solano's Bunker.": autoplay walks from the
-- Lucky Lady's drop-off up to the bunker to call its strike, and some runs
-- stick on the hillside on the way (rounds 25 and 32: over 120 hops). Still
-- active 600 s on, she is put 150 m short of the bunker, on her side of it.
local busting = 0
Event.CreatePersistent(Event.TimerRelative, { 10 }, function()
  if not active('Deploy Bunker Buster') then
    busting = 0
    return
  end
  busting = busting + 10
  if busting ~= 600 then return end
  local b = Pg.GetGuidByName('Solano_Bunker')
  local p = Player.GetLocalCharacter()
  if not b or not p then return end
  local bx, by, bz = Object.GetPosition(b)
  local px, _, pz = Object.GetPosition(p)
  if not bx or not px then return end
  local dx, dz = px - bx, pz - bz
  local d = math.sqrt(dx * dx + dz * dz)
  if d < 160 then return end
  local x, z = bx + dx / d * 150, bz + dz / d * 150
  Debug.Printf('>>> drive bunker: put 150 m short of Solano_Bunker after ' .. busting .. ' s')
  put(x, by + 5, z)
end, {})
