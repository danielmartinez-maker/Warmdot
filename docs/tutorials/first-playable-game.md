# First playable game: move a sprite

This tutorial creates a small script-driven movement game in the Forge editor, runs it in the editor preview, validates it, and exports a standalone Windows player. It needs no Rust code edits, external services, or downloaded game assets.

## Prerequisites

- Windows x64 with the Visual C++ Build Tools workload and Windows SDK.
- Stable Rust as pinned by the repository.
- The repository dependencies available to Cargo. The first build may need network access to fetch crates; once cached, the editor, project, script, and exported game work locally.

Run these commands from the repository root.

## 1. Create a project

```powershell
cargo run -p forge-editor -- --new .\projects\first-game "First Game"
```

The Forge editor opens a new project with an empty `Main` scene. The project files are created under `projects\first-game`.

## 2. Add a player sprite

1. In the top toolbar, click **Add sprite**.
2. Select the new Sprite in the **Scene** hierarchy.
3. In the **Inspector**, set its name to `Player` if desired. The default blue square is a built-in white texture with a blue tint, so no image asset is needed.
4. Leave the sprite size at `1` and its position at `0, 0` for this first run.

## 3. Add the movement script

Create a text file named `PlayerController.luau` anywhere outside the project and paste this script:

```lua
return {
    on_fixed_update = function(self, dt)
        local x, y = self:get_position()
        local move_x = Forge.Input.axis(1)
        local move_y = Forge.Input.axis(2)
        local magnitude = math.max(math.sqrt(move_x * move_x + move_y * move_y), 1)
        local speed = self:get("property")
        self:set_position(
            x + move_x / magnitude * speed * dt,
            y + move_y / magnitude * speed * dt
        )
    end,
}
```

Drag `PlayerController.luau` from File Explorer into the editor window. Forge copies it into the project's `Assets` folder and refreshes the asset list. Select the Player in the hierarchy, click **Add Luau Script** in the Inspector, and keep `PlayerController.luau` selected.

In the **Luau Script** section, click **Add property**. Leave its name as `property` and set its number to `4`. The script reads this exposed property as movement speed in world units per second.

## 4. Run it in the editor

Click **Save scene**, then **Play**. Click **Move** in the top toolbar once to clear any Inspector text-field focus, then click the center viewport. Use **W/A/S/D** to move the blue sprite; diagonal movement is normalized so it is not faster. Click **Stop** to return to editing.

Script or asset errors appear in the bottom **Script Console**. Fix the Luau file and save it; the editor preview applies a compatible source change while running and keeps the last working script if a replacement fails.

## 5. Validate and export

```powershell
cargo run -p forge-player -- --validate .\projects\first-game\project.forgeproj
cargo run -p forge-player -- --export .\projects\first-game\project.forgeproj .\build\first-game
.\build\first-game\run-game.bat
```

The standalone player uses the same movement actions. It binds **W/A/S/D** to action IDs `1` and `2`, **F** and right mouse to action ID `3`, and releases held actions when the window loses focus. The project input map is currently provided by the player host rather than configured in Project Settings.

## If something does not move

- Confirm the Player has a **Luau Script** component and its script asset is `PlayerController.luau`.
- Confirm the exposed number property is named `property` and is greater than zero.
- Click the viewport before using WASD so a text field or editor control does not own keyboard focus.
- Open the bottom **Script Console** and check the source path, tick, and error message.
- Run the validation command again after changing asset or scene references. A valid project prints a JSON report with no diagnostics.
