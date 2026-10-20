-- Probe: PirCon004's organ boxes on its truck's bed as the scripts count
-- them (radius 1 round HP_Truckbed), every 15 s.
local function count()
  local truck = Pg.GetGuidByName('PirCon004_OrganTruck')
  if not truck then return end
  local x, y, z = Object.GetPosition(truck)
  local hx, hy, hz = Object.GetHardpointPosition(truck, 'HP_Truckbed')
  local one = table.getn(Pg.GetObjectsInArea(hx, hy, hz, 1, 'Organ Container'))
  local four = table.getn(Pg.GetObjectsInArea(hx, hy, hz, 4, 'Organ Container'))
  local b = Pg.GetGuidByName('OrganBox_01')
  local bx, by, bz = 0, 0, 0
  if b then bx, by, bz = Object.GetPosition(b) end
  Debug.Printf(string.format('>>> probe: truck %.2f %.2f %.2f bed %.2f %.2f %.2f; within 1 m %d, within 4 m %d; box01 %.2f %.2f %.2f',
    x, y, z, hx, hy, hz, one, four, bx, by, bz))
end
Event.CreatePersistent(Event.TimerRelative, { 15 }, count, {})
