--[[ Test driver for the PMC story contracts after PmcCon001 (PmcCon002,
PmcCon003, PmcCon004 and on, with MecCon001 and JetCon001): logs every task
whose name holds one of theirs as its state changes (OBJECTIVE lines), and
every layer the scripts add or mark for removal (LAYER lines), so a run
shows how far each contract got and which act changes it made.

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 2700 \
    --skip-to PmcCon002 \
    --mission-driver crates/m2-game/examples/drivers/pmc.lua ]]

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

local watched = { 'PmcCon', 'PMC0', 'Pmc0', 'MecCon', 'MEC0', 'JetCon', 'JET0', 'PmcJob' }
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

local function traced(what, f)
  if type(f) ~= 'function' then
    return f
  end
  return function(...)
    local names = {}
    for i = 1, select('#', ...) do
      local v = select(i, ...)
      if type(v) == 'string' then table.insert(names, v) end
      if type(v) == 'table' then
        for _, n in ipairs(v) do
          if type(n) == 'string' then table.insert(names, n) end
        end
      end
    end
    say('>>> LAYER ' .. what .. ' ' .. table.concat(names, ', '))
    return f(...)
  end
end
MrxLayerManager.Add = traced('Add', MrxLayerManager.Add)
MrxLayerManager.MarkForRemoval = traced('MarkForRemoval', MrxLayerManager.MarkForRemoval)

-- The prompts the scripts put on objects (an action objective's "Enter
-- office", "Board the Lucky Lady") and take off them again (PROMPT lines).
local function prompts(what, f)
  return function(g, text, ...)
    say('>>> PROMPT ' .. what .. ' ' .. tostring(g) .. ' ' .. tostring(text))
    return f(g, text, ...)
  end
end
Pg.AddContextAction = prompts('Add', Pg.AddContextAction)
Pg.RemoveContextAction = prompts('Remove', Pg.RemoveContextAction)

-- Every second, as it changes: the vehicle the scripts see her in
-- (SEAT lines).
local lastSeat
local function seat()
  local ok, v = pcall(function() return Vehicle.GetFromRider(Player.GetAnyCharacter()) end)
  local s = tostring(ok and v)
  if s ~= lastSeat then
    say('>>> SEAT ' .. s)
    lastSeat = s
  end
  Event.Create(Event.TimerRelative, { 1 }, seat, {})
end
Event.Create(Event.TimerRelative, { 1 }, seat, {})
