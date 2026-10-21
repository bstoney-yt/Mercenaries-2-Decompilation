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

local n = 0
Event.CreatePersistent(Event.TimerRelative, { 5 }, function()
  n = n + 1
  if n > 12 then return end
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
