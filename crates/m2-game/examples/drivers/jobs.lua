--[[ Test driver for the jobs (the 16 the mission flow binds: OilJob004,
OilJob008, OilJob011, GurJob001, GurJob002, GurJob006, GurJob020, PirJob012,
PirJob020, AllJob002, AllJob003, AllJob020, ChiJob002, ChiJob003, ChiJob020,
PmcJob001). Logs, as JOB lines:
  - every job task's state as it changes (`JOB state <task> -> active`),
  - each target a job counts (`JOB part <job> <n>`) and each reward key
    granted, with her cash before and after (`JOB reward <key> <a> -> <b>`),
  - each job completed (`JOB done <job>`).

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 1500 \
    --skip-to AllJob002 \
    --mission-driver crates/m2-game/examples/drivers/jobs.lua ]]

-- The mission log keeps 60 prints a report: lines queue and go out a few
-- at a time.
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

local function cash()
  local ok, c = pcall(Player.GetCash)
  return ok and tostring(c) or '?'
end

local state = {}
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if ok and type(nm) == 'string' and nm:find('Job%d%d%d') then
    local st = (o:IsActive() and 'active') or (o:IsCompleted() and 'completed')
      or (o:IsCancelled() and 'cancelled') or 'latent'
    if state[nm] ~= st then
      say('JOB state ' .. nm .. ' -> ' .. st)
      if st == 'completed' and nm:match('^%u%l%lJob%d%d%dMission$') then
        say('JOB done ' .. nm:sub(1, 9))
      end
      state[nm] = st
    end
  end
  return unpack(r, 1, table.maxn(r))
end

local complete = MrxTaskJob._TargetComplete
MrxTaskJob._TargetComplete = function(o, ...)
  local r = { complete(o, ...) }
  local ok, id = pcall(function() return o:GetMissionId() end)
  say('JOB part ' .. tostring(ok and id) .. ' ' .. tostring(o._nTargetsComplete) .. ' cash ' .. cash())
  return unpack(r, 1, table.maxn(r))
end

local grant = MrxRewardData.GrantRewardKey
MrxRewardData.GrantRewardKey = function(key, ...)
  local before = cash()
  local r = { grant(key, ...) }
  if type(key) == 'string' and key:find('Job') then
    say('JOB reward ' .. key .. ' ' .. before .. ' -> ' .. cash())
  end
  return unpack(r, 1, table.maxn(r))
end
