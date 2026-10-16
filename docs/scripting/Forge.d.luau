declare namespace Forge
    declare namespace Input
        function axis(actionId: number): number
        function is_pressed(actionId: number): boolean
    end

    declare namespace Events
        function emit(name: string, value: (boolean | number | string)?): boolean
    end

    declare namespace Audio
        function play(assetGuid: string, bus: ("master" | "sfx" | "ui")?): boolean
        function play_music(assetGuid: string, loop: boolean?): boolean
        function set_bus_volume(bus: ("master" | "music" | "sfx" | "ui"), decibels: number): boolean
    end

    declare namespace Scene
        function load(sceneGuid: string): boolean
    end

    declare namespace UI
        function set_text(elementGuid: string, text: string): boolean
    end
end

export type ForgeObject = {
    get_guid: (self: ForgeObject) -> string,
    get: (self: ForgeObject, name: string) -> (boolean | number | string)?,
    set: (self: ForgeObject, name: string, value: boolean | number | string) -> (),
    get_position: (self: ForgeObject) -> (number, number),
    set_position: (self: ForgeObject, x: number, y: number) -> (),
    translate: (self: ForgeObject, dx: number, dy: number) -> (),
}

export type ForgeScript = {
    on_start: ((self: ForgeObject) -> ())?,
    on_fixed_update: ((self: ForgeObject, dt: number) -> ())?,
    on_event: ((self: ForgeObject, name: string, value: (boolean | number | string)?, sender_guid: string, emitted_tick: number) -> ())?,
}
