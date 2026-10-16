return {
    on_fixed_update = function(self, dt)
        local x, y = self:get_position()
        local move_x = Forge.Input.axis(1)
        local move_y = Forge.Input.axis(2)
        local magnitude = math.max(math.sqrt(move_x * move_x + move_y * move_y), 1)
        local speed = self:get("speed")

        self:set_position(
            math.max(self:get("min_x"), math.min(self:get("max_x"), x + move_x / magnitude * speed * dt)),
            math.max(self:get("min_y"), math.min(self:get("max_y"), y + move_y / magnitude * speed * dt))
        )
    end,
}
