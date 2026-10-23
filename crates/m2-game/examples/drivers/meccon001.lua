--[[ Test driver for MecCon001 (Eva's races): plays its timed races the way
a player who knows the course would. Autoplay drives from checkpoint to
checkpoint but stalls on the course's jumps and banks, and each
checkpoint's timer runs out (round 4: the second race cancelled the
contract at checkpoint 3 of 28, stuck 170 m short). So when a delivery
objective of the contract (a race checkpoint `LocN`, the drive to the
mine, parking in the garage) has stayed active this long, this moves the
vehicle she drives to its destination (`vDestLoc`), as the scripts move a
vehicle; the delivery then counts as the game's own trigger sees it.
Everything else is left to autoplay.

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 1800 \
    --skip-to MecCon001 \
    --mission-driver crates/m2-game/examples/drivers/meccon001.lua ]]

-- Seconds a delivery may stay active before she is helped, and between
-- helps (invented; the races give 10 s a checkpoint, MecCon001's
-- `nAddTime`, and round 5 at 20 s ran out at checkpoint 7 of 28).
local STALL, AGAIN = 6, 4
local names = { '^Loc%d+$', '^mc001 go to$', '^Park inside the garage$' }

local active = {}
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if ok and type(nm) == 'string' then
    for _, p in ipairs(names) do
      if nm:find(p) then
        if o:IsActive() then
          active[o] = active[o] or { name = nm, since = 0, helped = 0 }
        else
          active[o] = nil
        end
      end
    end
  end
  return unpack(r, 1, table.maxn(r))
end

local function destination(o)
  local ok, cfg = pcall(function() return o:GetConfig() end)
  if not ok or type(cfg) ~= 'table' then return nil end
  local d = cfg.vDestLoc
  if type(d) == 'string' then d = Pg.GetGuidByName(d) end
  if d == nil or type(d) == 'table' then return nil end
  return Object.GetPosition(d)
end

Event.CreatePersistent(Event.TimerRelative, { 1 }, function()
  for o, a in pairs(active) do
    a.since = a.since + 1
    if a.helped < 8 and a.since >= STALL + a.helped * AGAIN then
      a.helped = a.helped + 1
      local x, y, z = destination(o)
      local p = Player.GetLocalCharacter()
      local v = Vehicle.GetFromRider(p)
      if x and v ~= nil and type(v) ~= 'table' then
        Debug.Printf('>>> drive ' .. a.name .. ': moved to its destination after ' .. a.since .. ' s')
        Object.SetPosition(v, x, y + 1, z)
      else
        Debug.Printf('>>> drive ' .. a.name .. ': no destination or not driving')
      end
    end
  end
end, {})
