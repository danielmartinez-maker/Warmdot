declare namespace Forge
    declare namespace Input
        function axis(actionId: number): number
        function is_pressed(actionId: number): boolean
    end
end

export type ForgeObject = {
    get: (self: ForgeObject, name: string) -> (boolean | number | string)?,
    set: (self: ForgeObject, name: string, value: boolean | number | string) -> (),
    get_position: (self: ForgeObject) -> (number, number),
    set_position: (self: ForgeObject, x: number, y: number) -> (),
    translate: (self: ForgeObject, dx: number, dy: number) -> (),
}

export type ForgeScript = {
    on_start: ((self: ForgeObject) -> ())?,
    on_fixed_update: ((self: ForgeObject, dt: number) -> ())?,
}
