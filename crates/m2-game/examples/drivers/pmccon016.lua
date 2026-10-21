--[[ Test driver for PmcCon016 ("Highway to Hell", the PMC's timed race
with pylons that add time): drives the course the way a player who knows
it would. Autoplay follows the gate now up, but its route finder goes
round the PMC compound's walls (round 3: 505 m to a gate 178 m off, the
race lost at gate 3 of 27), so once a gate has been up for 6 s this puts
her race car 12 m short of it, facing it, and she drives through. The
pylons near the gate are left to autoplay. Logs what the contract
spawns for her to race in (from the scripts: "Panhard (Assault)" at
`loc_PmcCon016_Racer`).

Load it next to pmc.lua:

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 600 \
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

-- The race's gate now up: the active checkpoint objective's place.
local function gate()
  local m = MrxPlayState and MrxPlayState.GetCurrentMission and MrxPlayState.GetCurrentMission()
  if type(m) ~= 'table' then return nil end
  local function walk(t, depth)
    if depth > 6 then return nil end
    local ok, kids = pcall(function() return t:GetChildren() end)
    if not ok or type(kids) ~= 'table' then return nil end
    for _, c in pairs(kids) do
      local okA, a = pcall(function() return c:IsActive() end)
      local cfg = type(c.GetConfig) == 'function' and c:GetConfig() or {}
      if okA and a and type(cfg) == 'table' and cfg.vDestLoc and c:GetName():find('^Loc') then
        local d = cfg.vDestLoc
        if type(d) == 'string' then d = Pg.GetGuidByName(d) end
        return d, c:GetName()
      end
      local g, n = walk(c, depth + 1)
      if g then return g, n end
    end
    return nil
  end
  return walk(m, 0)
end

local up, since = nil, 0
Event.CreatePersistent(Event.TimerRelative, { 1 }, function()
  local g, name = gate()
  if g == nil then up = nil return end
  if name ~= up then up, since = name, 0 return end
  since = since + 1
  if since ~= 6 then return end
  local p = Player.GetLocalCharacter()
  local v = Vehicle.GetFromRider(p)
  if v == nil or type(v) == 'table' then return end
  local gx, gy, gz = Object.GetPosition(g)
  local vx, vy, vz = Object.GetPosition(v)
  if not gx or not vx then return end
  local dx, dz = gx - vx, gz - vz
  local d = math.sqrt(dx * dx + dz * dz)
  if d < 20 then return end
  local x, z = gx - dx / d * 12, gz - dz / d * 12
  say(string.format('%s up 6 s, %.0f m off: car put 12 m short of it', name, d))
  Object.SetPosition(v, x, gy + 1, z)
  if Object.SetYaw then Object.SetYaw(v, math.atan2(dx, dz)) end
end, {})
