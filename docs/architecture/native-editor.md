# Native scene editor

Create a starter project:

    cargo run -p forge-editor -- --new path/to/MyGame "My Game"

Open an existing project:

    cargo run -p forge-editor -- path/to/MyGame/project.forgeproj

The editor provides a scene hierarchy, click-to-select 2D preview, object creation/deletion, transform, sprite texture/animation/tint/size/UV, camera, rigid-body, and collider inspectors, bounded undo/redo, dirty-state tracking, and safe default-scene saving. PNG images and animation clips in the project Assets folder appear in the sprite picker. The preview renders the first frame for an assigned clip; the native player advances it on fixed ticks. The editor refreshes changed images while open; sprites without a loaded texture use a tinted rectangle.

Tilemap authoring, asset browsing, script/audio inspectors, and multi-scene workflows are not included yet.
