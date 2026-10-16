# Luau scripting

Forge loads .luau files beneath a project's Assets folder. The asset catalog assigns each file a stable GUID. A scene component stores that GUID and its exposed boolean, number, and string properties.

Each script file returns a table:

~~~lua
return {
    on_start = function(self)
        -- Called once when the scene instance starts.
    end,

    on_fixed_update = function(self, dt)
        local speed = self:get("speed")
        local horizontal = Forge.Input.axis(1)
        self:translate(horizontal * speed * dt, 0)
    end,
}
~~~

on_fixed_update(self, dt) runs once per simulation tick, after registered native fixed systems and before sprite animation and physics. The delta is the fixed simulation step in seconds. on_start(self) runs once before the first fixed update.

The object API currently provides:

- self:get(name) and self:set(name, value) for values declared on the scene component. Values must keep their authored type.
- self:get_position(), self:set_position(x, y), and self:translate(dx, dy) for the object's 2D position.
- Forge.Input.axis(action_id) and Forge.Input.is_pressed(action_id) for actions currently active on this tick. IDs are the numeric ActionId values configured by the project's input map.

Each component gets its own Luau VM. Forge uses the safe Luau standard-library subset and Luau sandbox mode. File, network, and wall-clock APIs are not exposed. Script sources are capped at 1 MiB; each VM is capped at 4 MiB; scenes can attach up to 64 scripts; and each callback has a bounded interrupt-checkpoint budget. A script error disables only that component and adds a diagnostic. The standalone player writes diagnostics to its console.

The player polls project assets while running. A syntactically invalid replacement script leaves the last working script installed and reports a diagnostic. A successful replacement creates a fresh script instance on the next fixed tick.

The current slice does not include breakpoints, named input action bindings, a debugger UI, or an editor play session. The existing editor inspector can select script assets and edit exposed properties; runtime diagnostics are available in the player console.

Forge.d.luau contains editor-facing API declarations. examples/playable_action_arena/Assets/PlayerController.luau demonstrates movement from numeric input actions, and examples/action_arena/Assets/PlayerController.luau is loaded through the project asset catalog.
