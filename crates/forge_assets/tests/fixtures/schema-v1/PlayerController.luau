return {
    on_fixed_update = function(self, dt)
        self:set_position(self:get_position())
    end
}
