--[[ Test driver for PmcCon016 ("Highway to Hell", the PMC's timed race
with pylons that add time): logs what the contract spawns for her to race
in and where its course starts, so a run shows whether the race vehicle
(`tVehicle`, "Panhard (Assault)" at `loc_PmcCon016_Racer`, from the
scripts) is there. It changes nothing in play.

Load it next to pmc.lua:

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 1500 \
    --skip-to PmcCon016 --mission-driver .../pmc.lua \
    --mission-driver crates/m2-game/examples/drivers/pmccon016.lua ]]

local function say(m) Debug.Printf('>>> PMC016 ' .. m) end

if type(MrxUtil) == 'table' and type(MrxUtil.SpawnObject) == 'function' then
  local spawn = MrxUtil.SpawnObject
  MrxUtil.SpawnObject = function(what, at, ...)
    local r = { spawn(what, at, ...) }
    say('SpawnObject ' .. tostring(what) .. ' at ' .. tostring(at) .. ' -> ' .. tostring(r[1]))
    return unpack(r, 1, table.maxn(r))
  end
end

local n = 0
Event.CreatePersistent(Event.TimerRelative, { 5 }, function()
  n = n + 1
  if n > 6 then return end
  local g = Pg.GetGuidByName('loc_PmcCon016_Racer')
  local x, y, z
  if g then x, y, z = Object.GetPosition(g) end
  say('racer spot ' .. tostring(g) .. ' at ' .. tostring(x) .. ', ' .. tostring(y) .. ', ' .. tostring(z)
    .. '; tVehicle ' .. tostring(rawget(_G, 'tVehicle')))
end, {})
