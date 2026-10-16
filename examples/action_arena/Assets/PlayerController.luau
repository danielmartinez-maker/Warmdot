return {
    on_fixed_update = function(self, dt)
        local x = self:get_position()
        local direction = self:get("direction")
        if x >= 2 then
            direction = -1
        elseif x <= -2 then
            direction = 1
        end
        self:set("direction", direction)
        self:translate(direction * self:get("speed") * dt, 0)
    end,
}
