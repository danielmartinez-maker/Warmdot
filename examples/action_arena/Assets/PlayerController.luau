local PLAYER_GUID = "529bb9ad-47b8-4992-874e-055c7fbf0003"
local PROJECTILE_GUID = "529bb9ad-47b8-4992-874e-055c7fbf0015"
local ENEMY_1 = "529bb9ad-47b8-4992-874e-055c7fbf0011"
local ENEMY_2 = "529bb9ad-47b8-4992-874e-055c7fbf0012"
local ENEMY_3 = "529bb9ad-47b8-4992-874e-055c7fbf0013"
local ENEMY_4 = "529bb9ad-47b8-4992-874e-055c7fbf0014"

local function is_enemy(guid)
    return guid == ENEMY_1 or guid == ENEMY_2 or guid == ENEMY_3 or guid == ENEMY_4
end

return {
    on_start = function(self)
        if self:get("projectile") == true then
            self:set_position(0, 20)
        end
    end,

    on_fixed_update = function(self, dt)
        if self:get("projectile") == true then
            if self:get("active") ~= true then
                return
            end
            local x, y = self:get_position()
            y = y - (self:get("speed") or 24) * dt
            if y < (self:get("min_y") or -18) then
                self:set("active", false)
                self:set_position(0, 20)
            else
                self:set_position(x, y)
            end
            return
        end

        if self:get("enemy") == true then
            if self:get("defeated") == true then
                return
            end
            local x, y = self:get_position()
            local speed = self:get("speed") or 1.5
            local min_x = self:get("min_x") or -25
            local max_x = self:get("max_x") or 25
            local min_y = self:get("min_y") or -13
            local max_y = self:get("max_y") or 13
            local direction = self:get("direction") or 1
            if x >= max_x then
                direction = -1
            elseif x <= min_x then
                direction = 1
            end
            self:set("direction", direction)
            local phase = (self:get("phase") or 0) + dt
            self:set("phase", phase)
            local base_y = self:get("base_y") or y
            self:set_position(
                math.max(min_x, math.min(max_x, x + direction * speed * dt)),
                math.max(min_y, math.min(max_y, base_y + math.sin(phase * 2.4) * 0.18))
            )
            return
        end

        local hit_cooldown = math.max(0, (self:get("hit_cooldown") or 0) - dt)
        self:set("hit_cooldown", hit_cooldown)
        if self:get("game_over") == true then
            return
        end

        local x, y = self:get_position()
        local speed = self:get("speed") or 1.5
        local min_x = self:get("min_x") or -25
        local max_x = self:get("max_x") or 25
        local min_y = self:get("min_y") or -13
        local max_y = self:get("max_y") or 13
        local move_x = Forge.Input.axis(1)
        local move_y = Forge.Input.axis(2)
        local magnitude = math.max(math.sqrt(move_x * move_x + move_y * move_y), 1)
        x = x + move_x / magnitude * speed * dt
        y = y + move_y / magnitude * speed * dt
        self:set_position(
            math.max(min_x, math.min(max_x, x)),
            math.max(min_y, math.min(max_y, y))
        )

        local firing = Forge.Input.is_pressed(3)
        local was_firing = self:get("was_firing") == true
        if firing and not was_firing then
            Forge.Events.emit("arena.fire.x", x)
            Forge.Events.emit("arena.fire.y", y)
        end
        self:set("was_firing", firing)
    end,

    on_event = function(self, event_name, value, sender_guid, tick)
        if self:get("projectile") == true then
            if sender_guid == PLAYER_GUID and event_name == "arena.fire.x"
                and self:get("active") ~= true and type(value) == "number" then
                self:set("launch_x", value)
            elseif sender_guid == PLAYER_GUID and event_name == "arena.fire.y"
                and self:get("active") ~= true and type(value) == "number" then
                local launch_x = self:get("launch_x") or 0
                self:set_position(launch_x, value - 0.7)
                self:set("active", true)
            elseif event_name == "physics.collision.started"
                and sender_guid == self:get_guid()
                and self:get("active") == true
                and is_enemy(value) then
                self:set("active", false)
                self:set_position(0, 20)
            end
            return
        end

        if self:get("enemy") == true then
            if self:get("defeated") ~= true
                and event_name == "physics.collision.started"
                and sender_guid == self:get_guid()
                and value == PROJECTILE_GUID then
                self:set("defeated", true)
                local x, y = self:get_position()
                self:set_position(x, 100)
                Forge.Events.emit("arena.score", 1)
            end
            return
        end

        if event_name == "arena.game_over" then
            self:set("game_over", value == true)
        elseif event_name == "physics.collision.started"
            and sender_guid == self:get_guid()
            and is_enemy(value)
            and (self:get("hit_cooldown") or 0) <= 0
            and self:get("game_over") ~= true then
            self:set("hit_cooldown", 1)
            self:set_position(0, 1.5)
            Forge.Events.emit("arena.life_lost", 1)
        end
    end,
}
