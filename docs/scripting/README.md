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

- self:get_guid() returns this runtime object's stable authored GUID.
- self:get(name) and self:set(name, value) for values declared on the scene component. Values must keep their authored type.
- self:get_position(), self:set_position(x, y), and self:translate(dx, dy) for the object's 2D position.
- Forge.Input.axis(action_id) and Forge.Input.is_pressed(action_id) for actions currently active on this tick. IDs are the numeric ActionId values supplied by the host; the standalone player currently binds action 1/2 to WASD movement and action 3 to F/right-click.
- Forge.Events.emit(name, value?) to queue a scalar event for the next fixed tick. Payloads can be nil, boolean, integer/number, or UTF-8 string. Names must contain non-whitespace text and be at most 128 bytes; string payloads are capped at 16 KiB. Each callback can emit up to 64 events and the shared queue holds up to 1,024. The function returns false for an invalid payload/name or when a bound is reached.
- Forge.Audio.play(asset_guid, bus?) queues a WAV or OGG effect on master, sfx, or ui (default sfx). Forge.Audio.play_music(asset_guid, loop?) streams music (looping by default). Forge.Audio.set_bus_volume(bus, decibels) accepts master, music, sfx, or ui at -60 to +6 dB. Each callback stages at most 16 audio commands and the host queue holds at most 512. Commands commit only after a successful callback and are drained by the player outside simulation; unavailable devices, missing assets, and decode/playback errors are reported without stopping the game. The calls return false for invalid input or when a bound is reached.
- Forge.Scene.load(scene_guid) queues a stable scene resource GUID for the player. Each callback can stage up to 16 requests and each fixed tick queues at most 64, in callback order. Requests commit only after the callback succeeds and the world transform applies. The player validates each request and loads the first scene whose document and dependencies prepare successfully; invalid targets leave the active scene running and are reported.

Physics contacts are broadcast on the fixed tick after Rapier reports them as `physics.collision.started` or `physics.collision.ended`. Each contact sends one event for each endpoint; `sender_guid` is the endpoint and the string `value` is the other endpoint's stable GUID. Use `self:get_guid()` to filter for events involving the current script object. Contact events follow the same bounded event queue and stable ordering as other external events.

A script can receive broadcasts with on_event(self, name, value, sender_guid, emitted_tick). Events are delivered to active scripts in stable object-GUID order, then emission order. Events emitted during delivery are deferred to a later fixed tick. Events staged by a callback are committed only when that callback succeeds and its transform applies.

Each component gets its own Luau VM. Forge uses the safe Luau standard-library subset and Luau sandbox mode. File, network, and wall-clock APIs are not exposed. Script sources are capped at 1 MiB; each VM is capped at 4 MiB; scenes can attach up to 64 scripts; and each callback has a bounded interrupt-checkpoint budget. A script error disables only that component and adds a diagnostic. The standalone player writes diagnostics to its console.

The player polls project assets while running. A syntactically invalid replacement script leaves the last working script installed and reports a diagnostic. A successful replacement creates a fresh VM on the next fixed tick, copying same-name scalar state fields only when their types match and preserving whether on_start already ran. Removed or incompatible scalar fields reset with a diagnostic; a migration failure leaves the last-good VM installed.

Line breakpoints are available in the editor preview with set/remove, pause, and continue controls. Named action bindings, project-serialized input maps, and a full debugger UI remain open. The editor preview supports play, pause, single-step, and stop; the inspector can select script assets and edit exposed properties. Runtime diagnostics are available in the player console and editor preview.

Forge.d.luau contains editor-facing API declarations. examples/playable_action_arena/Assets/PlayerController.luau demonstrates movement from numeric input actions, and examples/action_arena/Assets/PlayerController.luau is loaded through the project asset catalog.
