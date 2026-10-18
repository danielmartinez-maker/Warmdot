local CLICK = "a1d999a0-d8d1-4a1e-a700-4c49ed030003"
local MUSIC = "a1d999a0-d8d1-4a1e-a700-4c49ed030004"
local SCORE_LABEL = "a1d999a0-d8d1-4a1e-a700-4c49ed030013"
local LIVES_LABEL = "a1d999a0-d8d1-4a1e-a700-4c49ed030014"
local HELP_LABEL = "a1d999a0-d8d1-4a1e-a700-4c49ed030011"
local MAIN_SCENE = "529bb9ad-47b8-4992-874e-055c7fbf0002"
local MENU_SCENE = "529bb9ad-47b8-4992-874e-055c7fbf0005"

local score = 0
local lives = 3

return {
    on_start = function(self)
        Forge.Audio.play_music(MUSIC, true)
        if self:get("hud") == true then
            Forge.UI.set_text(SCORE_LABEL, "SCORE 0")
            Forge.UI.set_text(LIVES_LABEL, "LIVES 3")
        end
    end,

    on_event = function(self, event_name, value, sender_guid, tick)
        if event_name == "arena.menu" then
            Forge.Audio.play(CLICK, "ui")
            Forge.Scene.load(MENU_SCENE)
        elseif event_name == "arena.play" then
            Forge.Audio.play(CLICK, "ui")
            Forge.Scene.load(MAIN_SCENE)
        elseif event_name == "arena.fire.y" then
            Forge.Audio.play(CLICK, "sfx")
        elseif self:get("hud") == true and event_name == "arena.score" then
            local points = type(value) == "number" and math.max(1, math.floor(value)) or 1
            score = score + points
            Forge.UI.set_text(SCORE_LABEL, "SCORE " .. tostring(score))
            Forge.Audio.play(CLICK, "sfx")
            if score >= 4 then
                Forge.UI.set_text(HELP_LABEL, "ARENA CLEAR - MENU THEN PLAY TO RESTART")
                Forge.Events.emit("arena.game_over", true)
            end
        elseif self:get("hud") == true and event_name == "arena.life_lost" then
            lives = math.max(0, lives - 1)
            Forge.UI.set_text(LIVES_LABEL, "LIVES " .. tostring(lives))
            Forge.Audio.play(CLICK, "sfx")
            if lives == 0 then
                Forge.UI.set_text(HELP_LABEL, "GAME OVER - MENU THEN PLAY TO RESTART")
                Forge.Events.emit("arena.game_over", true)
            end
        end
    end,
}
