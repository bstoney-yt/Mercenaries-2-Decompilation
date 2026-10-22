--[[ Test driver for the Universal Petroleum contracts and jobs (OilCon001,
002, 003, 005, 020, 021, outposts OilCon050/051/052, OilJob004/008/011):
logs every task whose name holds one of theirs as its state changes
(OBJECTIVE lines), so a run shows how far each contract got, and the UP
HQs' state every 15 s (HQ lines).

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 2700 \
    --skip-to OilCon002 \
    --mission-driver crates/m2-game/examples/drivers/oil.lua ]]

-- The mission log keeps 60 prints a report, and a contract's start prints
-- far more in one: these lines queue and go out a few at a time.
local queue, flushing = {}, false
local function flush()
  for _ = 1, 4 do
    local m = table.remove(queue, 1)
    if not m then
      flushing = false
      return
    end
    Debug.Printf(m)
  end
  Event.Create(Event.TimerRelative, { 0.25 }, flush, {})
end
local function say(m)
  table.insert(queue, m)
  if not flushing then
    flushing = true
    Event.Create(Event.TimerRelative, { 0.25 }, flush, {})
  end
end

local watched = { 'OilCon', 'OilJob', 'OIL0', 'Oil0' }
local state = {}
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
          say('>>> OBJECTIVE ' .. nm .. ' -> ' .. st)
          state[nm] = st
        end
        break
      end
    end
  end
  return unpack(r, 1, table.maxn(r))
end

-- What a delivery objective makes of its cargo by the place (DELIVER
-- lines): OilCon002's posts come in by helicopter.
for _, f in ipairs({ '_FilterTargetAtDestination', '_DeliveryCheck', '_FilterTargetLeftDestination' }) do
  local orig = MrxTaskObjectiveDeliver and MrxTaskObjectiveDeliver[f]
  if orig then
    MrxTaskObjectiveDeliver[f] = function(o, a, ...)
      local r = { orig(o, a, ...) }
      local okn, nm = pcall(function() return o:GetName() end)
      say('>>> DELIVER ' .. f .. ' ' .. tostring(okn and nm) .. ' ' .. tostring(a)
        .. ' winched=' .. tostring(Object.IsWinched(a)) .. ' -> ' .. tostring(r[1]))
      return unpack(r, 1, table.maxn(r))
    end
  else
    say('>>> DELIVER no ' .. f)
  end
end

-- Every 15 s: the UP HQs' state, as `MrxHq.RefreshUiDisplay` decides
-- whether a portal is open (a starter with briefings, not locked, she not
-- inside).
local function hqs()
  local ok, t = pcall(function() return MrxHqManager._tHqs end)
  if not ok or type(t) ~= 'table' then
    say('>>> HQ table not found: ' .. tostring(t))
    t = {}
  end
  for name, hq in pairs(t) do
    if type(name) == 'string' and name:find('^Oil') then
      local st = hq._tStarter
      local okb, nb = pcall(function() return st:_GetBriefingCount() end)
      say('>>> HQ ' .. name .. ' locked=' .. tostring(hq._bLocked)
        .. ' global=' .. tostring(hq.bGloballyLocked) .. ' inside=' .. tostring(hq._bInside)
        .. ' starter=' .. tostring(st and st.GetName and st:GetName())
        .. ' briefings=' .. tostring(okb and nb))
    end
  end
  -- OilCon002's posts: what carries the label its deliveries wait on
  -- ("Listening Post", `MrxTaskObjectiveDeliver`'s label filter) by each
  -- location.
  for _, loc in ipairs({ 'oilcon002_loc_postA', 'oilcon002_loc_postB', 'oilcon002_loc_postC' }) do
    local okp, n = pcall(function()
      local g = Pg.GetGuidByName(loc)
      local x, y, z = Object.GetPosition(g)
      return table.getn(Pg.GetObjectsInArea(x, y, z, 10, 'Listening Post'))
    end)
    if okp and n and n > 0 then
      say('>>> POSTS ' .. loc .. ' ' .. tostring(n))
    end
  end
  -- OilCon001's executive (layer guid 0x10001dac), as the scripts see him:
  -- his delivery wants him within 6 m of `oc001.obj3.loc.dropoff`.
  local okx, x, y, z = pcall(function() return Object.GetPosition(Sys.StringToGuid('268443052')) end)
  if okx and x then
    say('>>> EXEC at ' .. string.format('%.1f %.1f %.1f', x, y, z))
  end
  Event.Create(Event.TimerRelative, { 15 }, hqs, {})
end
Event.Create(Event.TimerRelative, { 15 }, hqs, {})
