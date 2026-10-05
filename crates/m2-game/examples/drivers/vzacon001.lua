--[[ Test driver for the first contract, VzaCon001, for the `mission`
example: plays its objectives the way a player would, once a second.

It watches objective state changes (MrxTask._SetState) and, for the active
objective, moves the player into its destination region, kills its
targets, or drives the dropped vehicle to its destination. Positions are
read at run time (Pg.GetGuidByName, Object.GetPosition); the dropped
vehicle is the objective's own target. Run it before play starts:

  cargo run -p m2-game --example mission -- <vz.wad> \
    --driver crates/m2-game/examples/drivers/vzacon001.lua --seconds 900

The contract is done when the log shows Notify("Pg.ContractCompleted").

A driver run before this one may set DRIVE_UNTIL to part of an objective's
name ('DestroyGate'): this one then stops acting once that objective is
active, leaving it and the rest to the player. ]]

-- Objective (part of its name), what to do, and where or whom.
local acts = {
  { 'Go to the Beach', 'tp', 'region_vza_beach' },
  { 'Wait for Weapon', 'tp', 'region_vza_weapondrop' },
  { 'EliminateBeachGuardsVZ', 'kill', { 'vza_beachguard_1', 'vza_beachguard_2', 'vza_beachguard_3', 'vza_beachguard_4' } },
  { 'VZA001: DestroyGate', 'kill', { 'vza001_gate' } },
  { 'GetToFirstWaypoint', 'tp', 'region_vza_waypoint1' },
  { 'EliminateVillageGuards', 'kill', { 'vza_villageguard_1', 'vza_villageguard_2', 'vza_villageguard_3' } },
  { 'Wait for Car', 'veh', 'loc_vza_cardrop' },
  { 'GetToSecondWaypoint', 'tp', 'region_vza_waypoint2' },
  { 'DestroySecondGate', 'kill', { 'vza001_gate2' } },
  { 'Wait for Tank', 'veh', 'region_vza_amxdrop' },
  { 'GetToThirdWaypoint', 'tp', 'region_vza_waypoint3' },
  { 'GetToFourthWaypoint', 'tp', 'region_brokenbridge' },
  { 'Rescue Carmona', 'tp', 'region_vza_youwin' },
}

local state, tasks, tries = {}, {}, {}
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if ok and type(nm) == 'string' and nm:find('VZA001', 1, true) then
    local st = (o:IsActive() and 'active') or (o:IsCompleted() and 'completed')
      or (o:IsCancelled() and 'cancelled') or 'latent'
    if state[nm] ~= st then
      Debug.Printf('>>> OBJECTIVE ' .. nm .. ' -> ' .. st)
      state[nm] = st
    end
    tasks[nm] = o
  end
  return unpack(r, 1, table.maxn(r))
end

local function pos(name)
  local g = Pg.GetGuidByName(name)
  if g then return Object.GetPosition(g) end
end

-- A seated player is where the vehicle is: move the vehicle too.
local function put(x, y, z)
  local p = Player.GetLocalCharacter()
  local v = Vehicle.GetFromRider(p)
  if v ~= nil and type(v) ~= 'table' then Object.SetPosition(v, x, y, z) end
  Object.SetPosition(p, x, y, z)
end

local function target(o)
  local ok, c = pcall(function() return o:GetConfig() end)
  local t = ok and type(c) == 'table' and c.vTgtInclude
  if type(t) == 'table' then return t[1] end
  return t
end

local function act(nm, a)
  if a[2] == 'kill' then
    for _, k in ipairs(a[3]) do
      local g = Pg.GetGuidByName(k)
      Debug.Printf('>>> drive ' .. nm .. ': kill ' .. k)
      if g then Object.Kill(g) end
    end
    return
  end
  local x, y, z = pos(a[3])
  if not x then
    Debug.Printf('>>> drive ' .. nm .. ': no position for ' .. a[3])
    return
  end
  if a[2] == 'tp' then
    -- Away first, so the region sees the player come in.
    Debug.Printf('>>> drive ' .. nm .. ': away, then into ' .. a[3])
    put(x + 400, y, z + 400)
    Event.Create(Event.TimerRelative, { 0.5 }, function() put(x, y, z) end, {})
  else
    local p, v = Player.GetLocalCharacter(), target(tasks[nm])
    Debug.Printf('>>> drive ' .. nm .. ': enter ' .. tostring(v) .. ', drive to ' .. a[3] .. ', exit')
    if not v then return end
    Vehicle.Enter(v, p, 'd', true)
    Event.Create(Event.TimerRelative, { 1 }, function()
      Object.SetPosition(v, x + 400, y, z + 400); put(x + 400, y, z + 400)
    end, {})
    Event.Create(Event.TimerRelative, { 2 }, function()
      Object.SetPosition(v, x, y, z); put(x, y, z)
    end, {})
    Event.Create(Event.TimerRelative, { 3 }, function() Vehicle.Exit(p) end, {})
  end
end

local stopped = false
-- Read raw: a global nobody set answers a stub table, not nil.
local untilName = rawget(_G, 'DRIVE_UNTIL')
if type(untilName) ~= 'string' then untilName = nil end

-- Every second, act on each active objective every fourth try, ten times.
Event.CreatePersistent(Event.TimerRelative, { 1 }, function()
  if not stopped and untilName then
    for nm, st in pairs(state) do
      if st == 'active' and nm:find(untilName, 1, true) then
        Debug.Printf('>>> drive: stopped at ' .. nm)
        stopped = true
      end
    end
  end
  if stopped then return end
  for nm, st in pairs(state) do
    if st == 'active' then
      for _, a in ipairs(acts) do
        if nm:find(a[1], 1, true) then
          local t = (tries[nm] or 0) + 1
          tries[nm] = t
          if t <= 40 and t % 4 == 1 then act(nm, a) end
        end
      end
    end
  end
end, {})
