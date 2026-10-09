--[[ Test driver for the second contract, PmcCon001 (Solano's villa): plays
its objectives up to the villa hall the way a player would, once a second,
and logs every objective's state changes.

It moves the player into each "go to" objective's region and kills the
first objective's five entourage targets. It leaves the tank to the game:
the hall's tank breach (`LoadTank`, `GarageSmash`) and "Hijack the Tank"
are what this driver exists to watch (`m2-alpha --hijack` hijacks the
tank). It also kills the second entourage objective's soldiers (the wave
layers' names), so a run can reach the contract's end without a tank
fight.

Load it next to the first contract's play (`--autoplay`, or the
`vzacon001.lua` driver); it does nothing until PmcCon001's objectives
appear:

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 1500 \
    --after-contract 900 --mission-driver crates/m2-game/examples/drivers/pmccon001.lua

The contract is done when the log shows Notify("Pg.ContractCompleted") a
second time. ]]

-- Objective (part of its name), what to do, and where or whom.
local acts = {
  { 'PMC001: Go to the Villa', 'tp', 'reg_PMC001a' },
  { "Kill Solano's Entourage!", 'kill',
    { 'PMC001_HVT_01', 'PMC001_HVT_02', 'PMC001_HVT_03', 'PMC001_HVT_04', 'PMC001_HVT_05' } },
  { 'PMC001: Go to Villa Interior', 'tp', 'Region_PMC001_InvestigateVilla01' },
  { 'PMC001: Investigate Villa Interior', 'tp', 'Region_PMC001_VillaInterior' },
}
-- The second entourage objective's soldiers (its two wave layers).
local waves = {
  'top01', 'top02', 'top03', 'top04', 'bottom01', 'bottom02', 'garage02',
  'w2_top01', 'w2_top02', 'w2_top03', 'w2_top04',
  'w2_bottom01', 'w2_bottom02', 'w2_bottom03', 'w2_bottom04',
}
local watched = { 'PMC001', "Solano's Entourage", 'Hijack the Tank', 'PmcCon001' }

local state, tries = {}, {}
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if ok and type(nm) == 'string' then
    for _, w in ipairs(watched) do
      if nm:find(w, 1, true) then
        local st = (o:IsActive() and 'active') or (o:IsCompleted() and 'completed')
          or (o:IsCancelled() and 'cancelled') or 'latent'
        if state[nm] ~= st then
          Debug.Printf('>>> OBJECTIVE ' .. nm .. ' -> ' .. st)
          state[nm] = st
          -- The two entourage objectives share a name: count afresh.
          tries[nm] = 0
        end
        break
      end
    end
  end
  return unpack(r, 1, table.maxn(r))
end

local function pos(name)
  local g = Pg.GetGuidByName(name)
  if g then return Object.GetPosition(g) end
end

local function put(x, y, z)
  local p = Player.GetLocalCharacter()
  local v = Vehicle.GetFromRider(p)
  if v ~= nil and type(v) ~= 'table' then Vehicle.Exit(p) end
  Object.SetPosition(p, x, y, z)
end

local function kill(nm, names)
  for _, k in ipairs(names) do
    local g = Pg.GetGuidByName(k)
    if g and Object.IsAlive(g) then
      Debug.Printf('>>> drive ' .. nm .. ': kill ' .. k)
      Object.Kill(g)
    end
  end
end

local function act(nm, a)
  if a[2] == 'kill' then
    kill(nm, a[3])
    kill(nm, waves)
    return
  end
  local x, y, z = pos(a[3])
  if not x then
    Debug.Printf('>>> drive ' .. nm .. ': no position for ' .. a[3])
    return
  end
  -- Away first, so the region sees the player come in.
  Debug.Printf('>>> drive ' .. nm .. ': away, then into ' .. a[3])
  put(x + 60, y, z + 60)
  Event.Create(Event.TimerRelative, { 0.5 }, function() put(x, y, z) end, {})
end

Event.CreatePersistent(Event.TimerRelative, { 1 }, function()
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
