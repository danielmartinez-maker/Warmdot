# Native scene editor

Create a starter project:

    cargo run -p forge-editor -- --new path/to/MyGame "My Game"

Open an existing project:

    cargo run -p forge-editor -- path/to/MyGame/project.forgeproj

The editor provides a scene hierarchy, click-to-select 2D preview, object creation/deletion, transform and sprite tint/size/UV inspectors, bounded undo/redo, dirty-state tracking, and safe default-scene saving. The preview uses tinted rectangles; the native player renders registered project textures.

Tilemap authoring, asset browsing, script/audio inspectors, and multi-scene workflows are not included yet.
