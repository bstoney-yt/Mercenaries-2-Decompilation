--[[ Probe driver for the late PMC contracts: every 5 s for the first
minute it logs where she is and where the named places the contract sends
her are (3D distance and height difference), so a run shows why a
"move to" objective does or doesn't complete. It changes nothing in play.

  m2-alpha ... --mission-driver .../pmc.lua \
    --mission-driver crates/m2-game/examples/drivers/pmcprobe.lua ]]

local names = {
  'PmcCon032_Easy_Point1', 'PmcCon031_MGPoint', 'MGStatueLoc',
  'loc_PmcCon016_Racer', 'loc_pmccon016_001',
}

local function say(m) Debug.Printf('>>> PROBE ' .. m) end

-- Deliver objectives' arrival and delivery checks, as they run.
local D = rawget(_G, 'MrxTaskObjectiveDeliver')
if type(D) == 'table' then
  for _, f in ipairs({ '_TargetAtDestination', '_TargetLeftDestination', '_DeliveryCheck' }) do
    local orig = rawget(D, f)
    if type(orig) == 'function' then
      rawset(D, f, function(o, t, ...)
        local ok, nm = pcall(function() return o:GetName() end)
        say(f .. ' ' .. tostring(ok and nm) .. ' target ' .. tostring(t))
        return orig(o, t, ...)
      end)
    end
  end
else
  say('no MrxTaskObjectiveDeliver at load')
end

-- The contract's active objectives: their config's delivery fields and
-- the targets they track (`_tTargets`, with their events).
local function objectives()
  local m = MrxPlayState and MrxPlayState.GetCurrentMission and MrxPlayState.GetCurrentMission()
  if type(m) ~= 'table' then return end
  local ok, kids = pcall(function() return m:GetChildren() end)
  for _, c in pairs(ok and kids or {}) do
    local cfg = type(c.GetConfig) == 'function' and c:GetConfig() or {}
    local line = c:GetName() .. ': dest ' .. tostring(cfg.vDestLoc) .. ' dist ' .. tostring(cfg.fDist)
      .. ' stop ' .. tostring(cfg.bStop) .. ' xz ' .. tostring(cfg.bXZOnly) .. ' tgt ' .. tostring(cfg.vTgtInclude)
    local tt = rawget(c, '_tTargets')
    if type(tt) == 'table' then
      for k, v in pairs(tt) do
        local evs = {}
        if type(v) == 'table' and type(v.tEvents) == 'table' then
          for en, e in pairs(v.tEvents) do evs[#evs + 1] = tostring(en) .. '=' .. tostring(e) end
        end
        line = line .. ' | ' .. tostring(k) .. ' at ' .. tostring(type(v) == 'table' and v.bAtDestination)
          .. ' {' .. table.concat(evs, ' ') .. '}'
      end
    else
      line = line .. ' | no _tTargets'
    end
    say(line)
  end
end

local n = 0
Event.CreatePersistent(Event.TimerRelative, { 5 }, function()
  n = n + 1
  if n > 12 then return end
  pcall(objectives)
  local p = Player.GetLocalCharacter()
  local px, py, pz = Object.GetPosition(p)
  say(string.format('t%d her %s any %s at %s, %s, %s', n * 5, tostring(p),
    tostring(Player.GetAnyCharacter()), tostring(px), tostring(py), tostring(pz)))
  for _, nm in ipairs(names) do
    local g = Pg.GetGuidByName(nm)
    local x, y, z
    if g then x, y, z = Object.GetPosition(g) end
    if x and px then
      local dx, dy, dz = x - px, y - py, z - pz
      say(string.format('  %s %s at %.1f, %.1f, %.1f: %.1f m, %.1f m up',
        nm, tostring(g), x, y, z, math.sqrt(dx * dx + dy * dy + dz * dz), dy))
    elseif g then
      say('  ' .. nm .. ' ' .. tostring(g) .. ' has no position')
    end
  end
end, {})
