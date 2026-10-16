# Forge 2D

Forge 2D is a 2D-first game engine project for action games, RPGs, and simulation-heavy worlds. Its design keeps editable project and scene files separate from runtime storage and keeps third-party engine libraries behind Forge-owned interfaces.

This repository is being implemented from the Forge 2D product proposal. The target is the proposal's P0 alpha: a usable editor, runtime services, two sample games, and reproducible Windows builds. This is work in progress; no performance claims are made until the documented benchmark has been run.

## Runtime kernel currently on the implementation branch

- Stable authored GUIDs are kept separate from ephemeral runtime entities.
- Scene JSON is schema-versioned, validated, and compiled into a Forge-owned world API.
- Project manifests resolve a default scene and reject path traversal or symlink escape.
- The fixed-step clock caps frame time and simulation catch-up and reports dropped time.
- Timestamped input actions are delivered in stable queue order.
- The Rapier adapter supports fixed, dynamic, and position-based kinematic bodies; circle and box colliders; sensor/collision events; raycasts; and transform synchronization.
- The headless action-arena sample runs a dynamic player against a fixed floor.

## Not implemented yet

This branch is the runtime kernel, not the complete P0 engine. The 2D renderer and native window host, imported textures and sprite sheets, animation, Luau scripts, the editor and undo/redo workflow, tilemap painting, audio, retained game UI, standalone export pipeline, and the colony simulation remain ahead. The stress benchmark has not been run, so there are no measured performance results.

## Workspace

- `crates/forge_core`: stable IDs, fixed-step timing, and action input
- `crates/forge_world`: runtime components and stable render/physics extraction
- `crates/forge_scene`: versioned scene format and runtime compilation
- `crates/forge_project`: project lifecycle and confined scene paths
- `crates/forge_physics`: Rapier-backed rigid bodies, collision events, and raycasts
- `crates/forge_runtime`: headless simulation host and ordered systems
- `examples/action_arena`: small headless physics scene
- `.github/workflows/windows-check.yml`: Windows format and workspace type-check

## Build

Use stable Rust. On Windows, install the Visual C++ Build Tools workload and Windows SDK, then run:

    cargo fmt --all -- --check
    cargo check --workspace
    cargo run -p action-arena

The Windows GitHub Actions workflow runs formatting and a workspace type-check. It does not run a test suite.
