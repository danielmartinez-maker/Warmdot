# Forge 2D

Forge 2D is a 2D-first game engine project for action games, RPGs, and simulation-heavy worlds. Its design keeps editable project and scene files separate from runtime storage and keeps third-party engine libraries behind Forge-owned interfaces.

This repository is being implemented from the Forge 2D product proposal. The target is the proposal's P0 alpha: a usable editor, runtime services, two sample games, and reproducible Windows builds. This is work in progress; no performance claims are made until the documented benchmark has been run.

## Runtime kernel currently on the implementation branch

- Stable authored GUIDs are kept separate from ephemeral runtime entities.
- Scene JSON is schema-versioned, validated, and compiled into a Forge-owned world API.
- Project manifests resolve a default scene and reject path traversal or symlink escape.
- The fixed-step clock caps frame time and simulation catch-up and reports dropped time; the runtime supports play, pause, one-tick stepping, and scene replacement with clock/input/physics reset.
- Timestamped input actions are delivered in stable queue order; keyboard and mouse bindings aggregate multiple held sources and release cleanly when focus is lost.
- The Rapier adapter supports fixed, dynamic, and position-based kinematic bodies; circle and box colliders; sensor/collision events; raycasts; and transform synchronization.
- The headless action-arena sample runs a dynamic player against a fixed floor.
- A windowed action-arena sample wires keyboard/mouse actions, fixed-step movement, spawning, collision gameplay, and the wgpu renderer together.
- The native wgpu renderer draws ordered, tinted sprite instances in one instanced batch and can extract draw data from the runtime world.
- The renderer benchmark defaults to 10,000 colored sprites, 120 warm-up frames, and 600 CPU frame-time samples; it emits raw samples plus p50/p95/p99 on the selected GPU.
- The asset service scans project Assets folders without following symlinks, assigns stable GUIDs, detects added/changed/removed files, decodes bounded PNG images, and validates sprite-sheet and animation data.
- The animation player advances clip frames using simulation delta and handles looping and completion without per-frame allocation.
- Tilemaps use sparse 32x32 chunks with correct negative-coordinate mapping, deterministic serialization, batched paint edits, and bounded undo/redo history.
- Scene authoring commands add, delete, and replace objects through a bounded undo/redo history with a saved-state marker.

## Not implemented yet

This branch is the runtime kernel, not the complete P0 engine. The renderer currently handles solid tinted quads; its texture sampling path is still outstanding. The native editor UI, tilemap painting tools, Luau scripts, audio, retained game UI, standalone export, and the colony simulation remain ahead. The stress benchmark executable is implemented but has not been run on a desktop GPU, so there are no measured performance results.

## Workspace

- `crates/forge_core`: stable IDs, fixed-step timing, and action input
- `crates/forge_world`: runtime components and stable render/physics extraction
- `crates/forge_scene`: versioned scene format and runtime compilation
- `crates/forge_project`: project lifecycle and confined scene paths
- `crates/forge_physics`: Rapier-backed rigid bodies, collision events, and raycasts
- `crates/forge_runtime`: headless simulation host and ordered systems
- `crates/forge_renderer`: native wgpu surface, camera projection, sprite batching, and runtime-world extraction
- `crates/forge_assets`: safe project asset catalog, PNG decode, sprite-sheet/clip validation, and animation playback
- `crates/forge_tilemap`: sparse chunk storage, versioned tilemap documents, paint edits, and bounded undo/redo
- `crates/forge_input`: winit keyboard/mouse bindings translated into timestamped game actions
- `crates/forge_editor_core`: validated scene edits with bounded undo/redo and dirty-state tracking
- `examples/action_arena`: small headless physics scene
- `examples/renderer_benchmark`: windowed renderer benchmark with raw CPU frame-time samples
- `examples/playable_action_arena`: playable keyboard/mouse action game using the runtime, input map, and renderer
- `.github/workflows/windows-check.yml`: Windows format and workspace type-check
- `docs/architecture/renderer-and-benchmark.md`: renderer scope and benchmark measurement contract
- `docs/architecture/assets-and-animation.md`: asset scan, stable identity, and animation data contract
- `docs/architecture/tilemaps-and-edit-history.md`: chunk layout, coordinate mapping, and editor history contract
- `docs/architecture/runtime-controls.md`: pause, stepping, and scene replacement behavior
- `docs/architecture/input-bindings.md`: key/mouse action mapping and focus-loss semantics
- `docs/architecture/editor-core.md`: scene command and save-state history behavior
- `docs/architecture/playable-action-arena.md`: controls and systems used by the windowed sample

## Build

Use stable Rust. On Windows, install the Visual C++ Build Tools workload and Windows SDK, then run:

    cargo fmt --all -- --check
    cargo check --workspace
    cargo run -p action-arena
    cargo run --release -p renderer-benchmark -- --sprites 10000 --warmup 120 --samples 600

The Windows GitHub Actions workflow runs formatting and a workspace type-check. It does not run a test suite.
