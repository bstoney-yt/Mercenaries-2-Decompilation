--[[ Test driver for PmcCon003 (Solano's bunker, then back to the PMC):
plays its timed race the way a player with a fast ride would. Autoplay
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
