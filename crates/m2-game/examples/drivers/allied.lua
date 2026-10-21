--[[ Test driver for the Allied contracts (AllCon050, AllCon002, AllCon001,
AllCon003, AllCon052, AllCon053): logs every task whose name holds one of
theirs as its state changes (OBJECTIVE lines), so a run shows how far each
contract got, and the Allied HQs' state (HQ and LOCK lines).

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 2700 \
    --skip-to AllCon002 \
    --mission-driver crates/m2-game/examples/drivers/allied.lua ]]

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

local watched = { 'AllCon', 'ALL0', 'All0', 'AllJob001' }
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

local function allied(name)
  return type(name) == 'string' and (name:find('^All') or name == 'ChiOutpost1')
end

-- Every 15 s: the Allied HQs' state, as `MrxHq.RefreshUiDisplay` decides
-- whether a portal is open (a starter with briefings, not locked, she not
-- inside). ChiOutpost1 for comparison.
local function hqs()
  local ok, t = pcall(function() return MrxHqManager._tHqs end)
  if not ok or type(t) ~= 'table' then
    say('>>> HQ table not found: ' .. tostring(t))
    t = {}
  end
  for name, hq in pairs(t) do
    if allied(name) then
      local st = hq._tStarter
      local okb, nb = pcall(function() return st:_GetBriefingCount() end)
      say('>>> HQ ' .. name .. ' locked=' .. tostring(hq._bLocked)
        .. ' global=' .. tostring(hq.bGloballyLocked) .. ' inside=' .. tostring(hq._bInside)
        .. ' starter=' .. tostring(st and st.GetName and st:GetName())
        .. ' briefings=' .. tostring(okb and nb))
    end
  end
  Event.Create(Event.TimerRelative, { 15 }, hqs, {})
end
Event.Create(Event.TimerRelative, { 15 }, hqs, {})

-- Who locks and unlocks the Allied HQs (LOCK lines, with the Lua stack
-- when the host has one).
local function traced(what, f)
  return function(a, b, ...)
    local nm = a
    if type(a) == 'table' then
      local ok, n = pcall(function() return a:GetName() end)
      nm = ok and n or '?'
    end
    if allied(nm) or what == 'LockAllHq' or what == 'UnlockAllHq' then
      local tb = ''
      if debug and debug.traceback then tb = ' ' .. (debug.traceback():gsub('\n', ' | ')) end
      say('>>> LOCK ' .. what .. ' ' .. tostring(nm) .. ' ' .. tostring(b) .. tb)
    end
    return f(a, b, ...)
  end
end
MrxHq.SetLock = traced('SetLock', MrxHq.SetLock)
MrxHqManager.UnlockHq = traced('UnlockHq', MrxHqManager.UnlockHq)
MrxHqManager.LockHq = traced('LockHq', MrxHqManager.LockHq)
MrxHqManager.LockAllHq = traced('LockAllHq', MrxHqManager.LockAllHq)
MrxHqManager.UnlockAllHq = traced('UnlockAllHq', MrxHqManager.UnlockAllHq)
