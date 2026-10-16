return {
    on_fixed_update = function(self, dt)
        local x, y = self:get_position()
        local speed = self:get("speed") or 1.5
        local min_x = self:get("min_x") or -25
        local max_x = self:get("max_x") or 25
        local min_y = self:get("min_y") or -13
        local max_y = self:get("max_y") or 13

        if self:get("enemy") == true then
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

        local move_x = Forge.Input.axis(1)
        local move_y = Forge.Input.axis(2)
        if move_x ~= 0 or move_y ~= 0 then
            local magnitude = math.max(math.sqrt(move_x * move_x + move_y * move_y), 1)
            x = x + move_x / magnitude * speed * dt
            y = y + move_y / magnitude * speed * dt
        else
            local direction = self:get("direction") or 1
            if x >= 2 then
                direction = -1
            elseif x <= -2 then
                direction = 1
            end
            self:set("direction", direction)
            x = x + direction * speed * 0.25 * dt
        end
        self:set_position(
            math.max(min_x, math.min(max_x, x)),
            math.max(min_y, math.min(max_y, y))
        )

        local firing = Forge.Input.is_pressed(3)
        local was_firing = self:get("was_firing") == true
        if firing and not was_firing then
            Forge.Events.emit("arena.fire", true)
        end
        self:set("was_firing", firing)
    end,
}
