local HELP = "e055c011-0a71-4e5e-bc01-000000000001"
local FOOD = "e055c011-0a71-4e5e-bc01-000000000002"
local QUEUE = "e055c011-0a71-4e5e-bc01-000000000003"
local WORKERS = "e055c011-0a71-4e5e-bc01-000000000004"

local deliveries = 0
local queued_jobs = 0
local active_workers = 0
local next_resource = 0

local function refresh()
    Forge.UI.set_text(FOOD, "FOOD " .. tostring(deliveries))
    Forge.UI.set_text(QUEUE, "JOBS " .. tostring(queued_jobs))
    Forge.UI.set_text(WORKERS, "ACTIVE " .. tostring(active_workers))
end

return {
    on_start = function(self)
        deliveries = 0
        queued_jobs = 0
        active_workers = 0
        next_resource = 0
        Forge.UI.set_text(HELP, "F / RMB QUEUES WORK | EDITOR: PAUSE / STEP")
        refresh()
    end,

    on_event = function(self, event_name, value, sender_guid, tick)
        local amount = type(value) == "number" and math.max(1, math.floor(value)) or 1
        if event_name == "colony.worker_request" then
            local assigned = Forge.Events.emit("colony.assign." .. sender_guid, next_resource)
            if assigned then
                queued_jobs = math.min(256, queued_jobs + amount)
                next_resource = (next_resource + 1) % 4
            end
        elseif event_name == "colony.job_started" then
            queued_jobs = math.max(0, queued_jobs - amount)
            active_workers = math.min(256, active_workers + amount)
        elseif event_name == "colony.job_rejected" then
            queued_jobs = math.max(0, queued_jobs - amount)
        elseif event_name == "colony.food_delivered" then
            deliveries = deliveries + amount
            active_workers = math.max(0, active_workers - amount)
        end
        refresh()
    end,
}
