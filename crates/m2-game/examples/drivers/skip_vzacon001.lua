--[[ Test driver: completes the first contract's mission task
(`VzaCon001Mission`) a few seconds after it starts, so the flow goes on to
the contract's end (fanfare, closing movie, teleport to PmcCon001_Start1)
and the second contract, PmcCon001, without the first being played.
For testing the second contract (with `pmccon001.lua`):

  m2-alpha --install <dir> --mission --autoplay --mission-seconds 1500 \
    --after-contract 900 \
    --mission-driver crates/m2-game/examples/drivers/skip_vzacon001.lua \
    --mission-driver crates/m2-game/examples/drivers/pmccon001.lua ]]

local done = false
local setState = MrxTask._SetState
MrxTask._SetState = function(o, ...)
  local r = { setState(o, ...) }
  local ok, nm = pcall(function() return o:GetName() end)
  if not done and ok and nm == 'VzaCon001Mission' and o:IsActive() then
    done = true
    Debug.Printf('>>> skip: VzaCon001Mission is active; completing it in 5 s')
    Event.Create(Event.TimerRelative, { 5 }, function()
      Debug.Printf('>>> skip: completing VzaCon001Mission')
      o:Complete()
    end, {})
  end
  return unpack(r, 1, table.maxn(r))
end
