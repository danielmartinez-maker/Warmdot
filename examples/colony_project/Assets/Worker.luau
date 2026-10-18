local WIDTH = 30
local HEIGHT = 16
local CELL_SIZE = 2
local ORIGIN_X = -30
local ORIGIN_Y = -16
local HOME = { 15, 8 }
local RESOURCES = {
    { 4, 2 },
    { 26, 2 },
    { 4, 13 },
    { 26, 13 },
}
local NEIGHBORS = {
    { 1, 0 },
    { 0, 1 },
    { -1, 0 },
    { 0, -1 },
}
local MAX_PENDING_JOBS = 64
local BLOCKED = {}
local function key(x, y)
    return y * WIDTH + x
end
for y = 5, 10 do
    BLOCKED[key(8, y)] = true
    BLOCKED[key(21, y)] = true
end
for x = 12, 17 do
    BLOCKED[key(x, 12)] = true
end

local route = {}
local route_index = 1
local assigned_jobs = {}
local job_read = 1
local job_write = 1
local job_count = 0

local function in_bounds(x, y)
    return x >= 0 and x < WIDTH and y >= 0 and y < HEIGHT
end

local function cell_at(x, y)
    local cell_x = math.floor((x - ORIGIN_X) / CELL_SIZE)
    local cell_y = math.floor((y - ORIGIN_Y) / CELL_SIZE)
    if not in_bounds(cell_x, cell_y) then
        return nil
    end
    return { cell_x, cell_y }
end

local function cell_center(cell)
    return {
        ORIGIN_X + (cell[1] + 0.5) * CELL_SIZE,
        ORIGIN_Y + (cell[2] + 0.5) * CELL_SIZE,
    }
end

local function find_path(start_cell, goal)
    if not start_cell or not in_bounds(goal[1], goal[2])
        or BLOCKED[key(goal[1], goal[2])] then
        return nil
    end
    local queue = { { start_cell[1], start_cell[2] } }
    local previous = {}
    previous[key(start_cell[1], start_cell[2])] = false
    local cursor = 1
    while cursor <= #queue and cursor <= WIDTH * HEIGHT do
        local current = queue[cursor]
        cursor = cursor + 1
        if current[1] == goal[1] and current[2] == goal[2] then
            local reversed = {}
            local x, y = goal[1], goal[2]
            while x ~= start_cell[1] or y ~= start_cell[2] do
                reversed[#reversed + 1] = { x, y }
                local parent = previous[key(x, y)]
                if type(parent) ~= "table" then
                    return nil
                end
                x, y = parent[1], parent[2]
            end
            local result = {}
            for index = #reversed, 1, -1 do
                result[#result + 1] = reversed[index]
            end
            return result
        end
        for _, offset in ipairs(NEIGHBORS) do
            local next_x = current[1] + offset[1]
            local next_y = current[2] + offset[2]
            local next_key = key(next_x, next_y)
            if in_bounds(next_x, next_y) and not BLOCKED[next_key]
                and previous[next_key] == nil then
                previous[next_key] = current
                queue[#queue + 1] = { next_x, next_y }
            end
        end
    end
    return nil
end

local function set_route(self, goal)
    local x, y = self:get_position()
    route = find_path(cell_at(x, y), goal) or {}
    route_index = 1
end

local function move_along_route(self, dt, speed)
    local next_cell = route[route_index]
    if not next_cell then
        return true
    end
    local target = cell_center(next_cell)
    local x, y = self:get_position()
    local dx, dy = target[1] - x, target[2] - y
    local distance = math.sqrt(dx * dx + dy * dy)
    if distance <= 0.05 then
        route_index = route_index + 1
    elseif distance > 0 then
        local step = math.min(distance, speed * dt)
        self:set_position(x + dx / distance * step, y + dy / distance * step)
        if step >= distance then
            route_index = route_index + 1
        end
    end
    return route_index > #route
end

local function start_job(self, resource_index)
    set_route(self, RESOURCES[resource_index + 1])
    self:set("phase", 1)
    Forge.Events.emit("colony.job_started", 1)
end

return {
    on_start = function(self)
        route = {}
        route_index = 1
        assigned_jobs = {}
        job_read = 1
        job_write = 1
        job_count = 0
        Forge.Events.emit("colony.worker_request", 1)
    end,

    on_fixed_update = function(self, dt)
        if Forge.Input.is_just_pressed(3) then
            Forge.Events.emit("colony.worker_request", 1)
        end

        local phase = math.floor(self:get("phase") or 0)
        if phase == 0 then
            if job_count <= 0 then
                return
            end
            local resource_index = assigned_jobs[job_read]
            assigned_jobs[job_read] = nil
            job_read = job_read % MAX_PENDING_JOBS + 1
            job_count = job_count - 1
            start_job(self, resource_index)
            phase = 1
        end

        local speed = self:get("speed") or 3.2
        if phase == 1 and move_along_route(self, dt, speed) then
            self:set("phase", 2)
            Forge.Events.emit("colony.food_picked", 1)
            set_route(self, HOME)
        elseif phase == 2 and move_along_route(self, dt, speed) then
            self:set("phase", 0)
            Forge.Events.emit("colony.food_delivered", 1)
            route = {}
            route_index = 1
        end
    end,

    on_event = function(self, event_name, value, sender_guid, tick)
        if event_name == "colony.assign." .. self:get_guid() and type(value) == "number" then
            if job_count >= MAX_PENDING_JOBS then
                Forge.Events.emit("colony.job_rejected", 1)
                return
            end
            assigned_jobs[job_write] = math.floor(value) % #RESOURCES
            job_write = job_write % MAX_PENDING_JOBS + 1
            job_count = job_count + 1
        end
    end,
}
