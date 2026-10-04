inherit("Task")
function Activated(self) return Task.Activated(self) + 1 end
