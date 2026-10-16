# Playable action arena

Run with cargo run --release -p playable-action-arena on a Windows desktop with a graphics adapter.

WASD or the arrow keys move the turquoise player. Space or the left mouse button fires. Enemies spawn at the top edge, move downward, and disappear when hit; the window title reports score. The game loop uses GameRuntime fixed updates, InputMap source aggregation, world-owned sprite extraction, and the wgpu renderer.

This is a small functional integration sample. It has no menus, audio, asset textures, save system, or game-over state.
