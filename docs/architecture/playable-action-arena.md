# Playable Action Arena

The editor-authored Action Arena project demonstrates a complete arcade loop through saved scene data, Luau scripts, and the standalone Forge player.

## Editor-authored project

Run the saved project in the standalone player:

    cargo run --release -p forge-player -- examples/action_arena/project.forgeproj

WASD moves the player. Press F or right-click to fire in the standalone player; the editor preview accepts WASD and F. The player and four patrolling enemies use separate four-frame animation clips from the authored sprite atlas. The room uses the saved tilemap and the Main scene camera. The authored Pulse Shot travels from the player and uses Rapier contacts to defeat enemies, add score, and show an arena-clear message after all four targets are defeated. Touching an enemy consumes one of three lives and respawns the player after a short invulnerability period. At zero lives, the player stops and the HUD shows a game-over prompt. Use MENU, then PLAY, to start a fresh run from the saved scenes.

The HUD, menu, scene transitions, music, effects, score, and lives are authored project assets. The score and lives labels update through the bounded `Forge.UI.set_text` scripting command. The player, enemies, and projectile use the shared `PlayerController.luau` asset with per-object properties; collision events update gameplay on the next fixed tick. CI validates the scene, loads its sprite clips through the runtime tests, exports the project, and checks that the exported package contains the animation atlas, clips, HUD, tilemap, scripts, and audio.

## Rust integration sample

Run the separate windowed Rust sample:

    cargo run --release -p playable-action-arena

WASD or the arrow keys move; Space or the left mouse button fires. Enemies spawn from below, drift across the arena, and disappear when hit. Contact costs a life and briefly respawns the player. The window title reports score and lives; press R to restart after game over.

This sample remains useful for runtime integration checks. Native desktop walkthroughs of the editor preview, exported standalone player, and first-game tutorial remain part of the P0 acceptance gate because no Desktop Commander device is connected to this task.
