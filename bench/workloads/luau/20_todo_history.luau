-- Same deterministic todo command trace as todo_steady, with a bounded 64-entry undo/history ring.
local MOD = 2147483647
local CAPACITY = 256
local HISTORY = 64
local randomState = 1234567
local tasks = {}
local history = {}
local historyStep = 0
local revision = 0

local function nextRandom()
  randomState = (randomState * 48271) % MOD
  return randomState
end

local function makeTask(slot, value, previous)
  revision += 1
  local completed = value % 2 == 0
  if previous ~= nil and value % 3 == 0 then completed = previous[4] end
  return {slot, "task-" .. slot .. "-" .. revision .. "-" .. value, (value % 5) + 1, completed, value % 8, revision}
end

local function remember(operation, slot, value, oldTask)
  historyStep += 1
  history[((historyStep - 1) % HISTORY) + 1] = {operation, slot, value, oldTask}
end

local function f(n)
  local checksum = 0
  for _ = 1, n do
    local command = nextRandom()
    local selector = nextRandom()
    local value = nextRandom()
    local operation = command % 6
    local slot = (selector % CAPACITY) + 1
    local task = tasks[slot]

    if operation == 0 then
      remember(operation, slot, value, task)
      tasks[slot] = makeTask(slot, value, task)
    elseif operation == 1 then
      if task ~= nil then checksum += task[1] + #task[2] + task[3] + task[5] + task[6] end
    elseif operation == 2 then
      if task ~= nil then
        remember(operation, slot, value, task)
        tasks[slot] = makeTask(slot, value, task)
      end
    elseif operation == 3 then
      if task ~= nil then
        remember(operation, slot, value, task)
        revision += 1
        tasks[slot] = {task[1], task[2], task[3], not task[4], task[5], revision}
      end
    elseif operation == 4 then
      remember(operation, slot, value, task)
      tasks[slot] = nil
    else
      local wantedPriority = (value % 5) + 1
      local wantedTag = value % 8
      for _, candidate in pairs(tasks) do
        if candidate[3] == wantedPriority or candidate[5] == wantedTag then
          checksum += candidate[1] + candidate[6]
          if candidate[4] then checksum += 1 end
        end
      end
    end
    checksum = (checksum + operation + slot + value) % MOD
  end
  local live = 0
  for _ in pairs(tasks) do live += 1 end
  return (checksum + live * 257 + #history * 17 + revision) % MOD
end

return f
