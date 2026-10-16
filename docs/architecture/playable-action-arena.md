# Playable action arena

Run the integration sample on a Windows desktop with a graphics adapter:

    cargo run --release -p playable-action-arena

WASD or the arrow keys move the turquoise player. Space or the left mouse button fires. Enemies spawn from below, drift across the arena, pulse as they move, and disappear when hit by a shot. Contact costs a life and briefly respawns the player at the start. The window title reports score and lives; press R to restart after game over.

The arena has a tiled room, camera, bounded enemy population, deterministic fixed-step movement and overlap collisions. This sample exercises the runtime, input map, script-driven player movement, world-owned sprite extraction, and wgpu renderer. The editor-authored project in `examples/action_arena` demonstrates saved scene data, HUD/menu UI, audio, and scene transitions; this Rust integration sample does not replace that project workflow.
The editor-authored project in `examples/action_arena` is a saved top-down room using the default tilemap's solid border. Its player has no gravity and stays in place until WASD input; enemies patrol and bob with physics colliders. The HUD shows the actual controls (WASD to move, F or right mouse to fire), and its menu buttons transition between saved scenes. The firing action currently plays a sound cue in this project. The separate Rust integration sample above contains the projectile and scoring loop.
