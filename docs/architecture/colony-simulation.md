# Colony simulation

The 64-worker native stress sample remains available:

    cargo run -p colony-simulation

It profiles its fixed-step update, queues food-delivery jobs, routes workers deterministically around blocked grid cells, protects the home and resource cells, supports pause and 1x/2x/3x time, and lets the user add blockers or build workers.

## Editor-authored colony project

Open `examples/colony_project/project.forgeproj` in Forge Editor or run the saved project through the standalone player:

    cargo run --release -p forge-player -- examples/colony_project/project.forgeproj

The saved project includes a negative-coordinate chunked tilemap, fixed blocked cells, a camera, eight animated worker objects, four resource caches, a hearth, and an authored HUD. Workers use a bounded deterministic breadth-first grid search with stable neighbor ordering to route between resources and home. F or right-click requests a job for each worker. The controller assigns resource nodes in deterministic round-robin order, tracks queued and active jobs, and updates the delivery HUD through `Forge.UI.set_text`. A bounded per-worker queue keeps outstanding assignments finite. Editor preview pause, single-step, runtime diagnostics, and profiling remain available. Tile edits are saved by the editor's existing tilemap authoring workflow.

A workspace integration test runs the editor-authored project for a full delivery, checks all eight animation instances, requires healthy scripts, and asserts that workers stay off blocked cells. The Windows packaging workflow validates and exports both projects and checks the exported colony tilemap, sprite clips, HUD, scripts, and standalone launcher.
