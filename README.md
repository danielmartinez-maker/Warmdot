# Forge 2D

Forge 2D is a 2D-first game engine project for action games, RPGs, and simulation-heavy worlds. Its design keeps editable project and scene files separate from runtime storage and keeps third-party engine libraries behind Forge-owned interfaces.

This repository is being implemented from the Forge 2D product proposal. The target is the proposal's P0 alpha: a usable editor, runtime services, two sample games, and reproducible Windows builds. This is work in progress; no performance claims are made until the documented benchmark has been run.

## Runtime kernel currently on the implementation branch
- Stable authored GUIDs are kept separate from ephemeral runtime entities; world coordinates use +X right and +Y down.
- Scene JSON is schema-versioned, validated, and compiled into a Forge-owned world API.
- Project manifests resolve a default scene and reject path traversal or symlink escape.
- The fixed-step clock caps frame time and simulation catch-up and reports dropped time; the runtime supports play, pause, one-tick stepping, and scene replacement with clock/input/physics reset.
- Timestamped input actions are delivered in stable queue order; keyboard and mouse bindings aggregate multiple held sources and release cleanly when focus is lost.
- The Rapier adapter supports fixed, dynamic, and position-based kinematic bodies; circle and box colliders; sensor/collision events; raycasts; and transform synchronization.
- The headless action-arena sample runs a dynamic player against a fixed floor.
- A windowed action-arena sample wires keyboard/mouse actions, fixed-step movement, spawning, collision gameplay, and the wgpu renderer together.
- The native wgpu renderer samples registered RGBA images, applies sprite tint and alpha blending, and draws contiguous image groups with instanced quads; world extraction resolves each sprite's stable asset GUID.
- The renderer benchmark defaults to 10,000 colored sprites, 120 warm-up frames, and 600 CPU frame-time samples; it emits raw samples plus p50/p95/p99 on the selected GPU.
- The asset service scans project Assets folders without following symlinks, assigns stable GUIDs, detects added/changed/removed files, decodes bounded PNG images, and validates sprite-sheet and animation data.
- The animation player advances clip frames using fixed simulation delta and handles looping and completion without per-frame allocation; scene sprites can reference animation-clip assets, which the runtime validates, attaches, and applies as normalized sprite-sheet UV regions.
- Luau scripts load as scene components with exposed properties, per-component memory limits, fixed-tick input/transform APIs, per-script error isolation, and last-good hot reload in the standalone player and editor preview. The playable action-arena sample uses a Luau controller.
- Tilemaps use sparse 32x32 chunks with correct negative-coordinate mapping, deterministic serialization, batched paint edits, connected fill capped at 65,536 cells, an undoable collision-only brush, and bounded undo/redo history.
- Scene authoring commands add, delete, and replace objects through a bounded undo/redo history with a saved-state marker.

## Remaining P0 work

The native editor supports scene and tilemap authoring, undo/redo, scene saving, and an embedded runtime preview with Play, Pause/Resume, one-tick Step, and Stop. During play it keeps GUID-based selection, shows script diagnostics, edits exposed properties live, reloads PNG textures, and transactionally recompiles changed Luau while retaining the last good VM after a compile failure. Authoring work completed so far includes project name/default-scene settings, a stable-GUID asset browser with resource validation, viewport pan/zoom, bounded connected tile fill, and an explicit collision-only brush that preserves tile art and is undoable. Remaining P0 work includes external asset import, parent transforms and prefab instances/overrides, complete move/rotation/scale/anchor gizmos and layer workflow, script breakpoints/events/state migration, audio, retained game UI, scene transitions, profiler/logging, tutorial, and full action/colony vertical slices. The renderer benchmark executable is implemented but has not been run on a desktop GPU, so there are no measured performance results.

## Workspace

- `crates/forge_core`: stable IDs, fixed-step timing, and action input
- `crates/forge_world`: runtime components and stable render/physics extraction
- `crates/forge_scene`: versioned scene format and runtime compilation
- `crates/forge_project`: project lifecycle plus confined load/save for multiple scene resources
- `crates/forge_physics`: Rapier-backed rigid bodies, collision events, and raycasts
- `crates/forge_runtime`: headless simulation host and ordered systems
- `crates/forge_renderer`: native wgpu surface, camera projection, sprite batching, and runtime-world extraction
- `crates/forge_assets`: safe project asset catalog, PNG decode, sprite-sheet/clip validation, and animation playback
- `crates/forge_tilemap`: sparse chunk storage, versioned tilemap documents, paint edits, and bounded undo/redo
- `crates/forge_input`: winit keyboard/mouse bindings translated into timestamped game actions
- `crates/forge_editor_core`: validated scene edits with bounded undo/redo and dirty-state tracking
- `examples/action_arena`: small headless physics scene
- `examples/renderer_benchmark`: windowed renderer benchmark with raw CPU frame-time samples
- `examples/forge_player`: opens a `.forgeproj`, runs its default scene, authored animations, and saved tilemap with solid-cell colliders, refreshes project PNGs, and exports a portable player/project folder with a launcher
- `examples/forge_editor`: creates or opens projects, authors scenes and tilemaps, provides a runtime preview with live script/PNG/property updates, and saves with undo/redo
- `examples/playable_action_arena`: playable keyboard/mouse action game using the runtime, input map, and renderer
- `examples/colony_simulation`: worker colony sample with fixed-step resource trips and worker creation
- `.github/workflows/windows-check.yml`: Windows format and workspace type-check
- `docs/architecture/renderer-and-benchmark.md`: renderer scope and benchmark measurement contract
- `docs/architecture/assets-and-animation.md`: asset scan, stable identity, and animation data contract
- `docs/architecture/tilemaps-and-edit-history.md`: chunk layout, coordinate mapping, and editor history contract
- `docs/architecture/runtime-controls.md`: pause, stepping, and scene replacement behavior
- `docs/architecture/project-export.md`: standalone player and project-folder packaging contract
- `docs/architecture/input-bindings.md`: key/mouse action mapping and focus-loss semantics
- `docs/architecture/editor-core.md`: scene command and save-state history behavior
- `docs/architecture/playable-action-arena.md`: controls and systems used by the windowed sample

## Build

Use stable Rust. On Windows, install the Visual C++ Build Tools workload and Windows SDK, then run:

    cargo fmt --all -- --check
    cargo check --workspace
    cargo run -p action-arena
    cargo run -p forge-player -- path/to/project.forgeproj
    cargo run -p forge-player -- --export path/to/project.forgeproj path/to/export-folder
    cargo run -p forge-editor -- path/to/project.forgeproj
    cargo run -p colony-simulation
    cargo run --release -p renderer-benchmark -- --sprites 10000 --warmup 120 --samples 600

The Windows GitHub Actions workflow checks formatting, type-checks the workspace, and runs all workspace tests.
