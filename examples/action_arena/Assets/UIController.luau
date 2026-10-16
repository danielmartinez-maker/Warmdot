local CLICK = "a1d999a0-d8d1-4a1e-a700-4c49ed030003"
local MUSIC = "a1d999a0-d8d1-4a1e-a700-4c49ed030004"
local MAIN_SCENE = "529bb9ad-47b8-4992-874e-055c7fbf0002"
local MENU_SCENE = "529bb9ad-47b8-4992-874e-055c7fbf0005"

return {
    on_start = function(self)
        Forge.Audio.play_music(MUSIC, true)
    end,

    on_event = function(self, event_name, value, sender_guid, tick)
        if event_name == "arena.menu" then
            Forge.Audio.play(CLICK, "ui")
            Forge.Scene.load(MENU_SCENE)
        elseif event_name == "arena.play" then
            Forge.Audio.play(CLICK, "ui")
            Forge.Scene.load(MAIN_SCENE)
        elseif event_name == "arena.fire" then
            Forge.Audio.play(CLICK, "sfx")
        end
    end,
}
