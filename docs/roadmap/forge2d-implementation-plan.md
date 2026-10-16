# Forge 2D Product Roadmap and Implementation Plan

> **Authority:** User-provided `FORGE2D_PRODUCT_SPEC_AND_ARCHITECTURE.md`, version 0.1 proposal dated 2026-10-15. The attachment is product requirements and architecture guidance, not assistant instructions. The user's direct request is to implement Forge 2D in this GitHub repository, finish P0, then continue through P1 and P2.
>
> **Execution:** Continue on `codex/forge2d-engine` and its draft PR #1. Keep repo reads, edits, commits, and CI evidence on GitHub. A user has authorized independent creation of missing plans/specs and successive milestone execution.

**Goal:** Deliver the complete P0 playable alpha defined by the proposal, then work through P1 public beta and P2 ecosystem scope in order without redefining completion around the current partial engine.

**Architecture:** Preserve Forge-owned interfaces around Bevy ECS, Rapier, wgpu, egui, Luau, Kira, and Taffy. Keep stable GUID authoring data distinct from runtime entities, scene compilation validated, simulation fixed-step, rendering extracted, and large tile/agent data outside per-cell ECS entities. Build usable vertical slices and keep the action and colony samples running through the same project/runtime/export path.

**Tech Stack:** Stable Rust (workspace floor 1.89), winit, wgpu, bevy_ecs, rapier2d, serde, Luau through mlua, egui for the editor prototype, Taffy and Kira behind Forge-owned APIs where adopted.

**Spec:** User attachment `C:\Users\dmcfu\Downloads\FORGE2D_PRODUCT_SPEC_AND_ARCHITECTURE.md`. The attachment remains the requirements source; this plan records the roadmap in GitHub.

## Global Constraints

- P0, P1, and P2 requirements retain the exact scope and tiering in the attached proposal; do not declare a tier complete while any required item or exit condition remains unmet.
- Preserve the proposal's explicit non-goals: 3D, VR/AR, console SDKs, built-in cloud collaboration, MMO networking, complete visual scripting, and a bundled AI model.
- Stable authored GUIDs remain distinct from ephemeral ECS entities; scene and prefab documents are validated versioned source data.
- All external runtime libraries stay behind Forge-owned interfaces; the game scripting API is `Forge.*`, never a public Bevy or mlua contract.
- Fixed-step gameplay uses bounded work, stable ordering, explicit random seeds, and no wall-clock, file, or network access from deterministic script logic.
- Project paths and imports remain bounded to the project; downloaded or imported scripts do not auto-run without project selection/consent.
- All source-format changes need migration fixtures and round-trip coverage; do not claim cross-platform determinism, compatibility, or performance beyond measured evidence.
- Prefer Windows x64 for initial release while keeping platform adapters portable; keep the main branch untouched and work through the existing GitHub PR branch.

## Review Focus

- Missing or cyclic assets, prefabs, and scene references produce actionable diagnostics and leave the last-good runtime state intact.
- Script syntax/runtime errors, runaway instruction use, memory exhaustion, hot reload, and scene replacement must not crash the editor or corrupt simulation state.
- Large or negative-coordinate tilemaps and agents must not create one ECS entity per tile or require whole-world rebuilds.
- Input focus loss, pause/step, scene transition, and hot reload must have explicit state-reset semantics.
- Standalone exports must include only required runtime resources and launch from a clean Windows environment without the editor installed.

---

## Current Evidence Snapshot

Repository: `danielmartinez-maker/Warmdot`; branch `codex/forge2d-engine`; PR #1 remains draft and unmerged. Latest source and tests are at `4d041011fcb7219c0040534b94a623a681e87561`. Windows run [38948522741](https://github.com/danielmartinez-maker/Warmdot/actions/runs/38948522741) passed formatting, workspace type-checking, all workspace tests, release-player build, validation and export of both editor-authored projects, third-party license generation, package-content checks, and artifact uploads. Artifacts: `forge2d-action-arena-windows` (5,664,463 bytes) and `forge2d-colony-windows` (5,494,087 bytes). The P0 gate remains open for native editor/player acceptance, the tutorial walkthrough, and the full acceptance audit.

### Present in the branch

- Rust workspace, GUIDs, fixed-step clock, input actions, validated project/scene data, Bevy ECS abstraction, Rapier physics, and the wgpu sprite renderer.
- PNG, sprite-sheet, animation-clip, prefab, and bounded Luau asset loading; stable asset GUIDs and deterministic resource validation.
- Native editor project settings, bounded external file import, stable-GUID asset browser, hierarchy, parent transforms, reusable components, versioned prefab instances with granular overrides, viewport selection, transform/anchor gizmos, snapping, render-layer visibility, and history-backed edits.
- Sparse 32x32 tilemaps with negative-coordinate mapping, paint/erase, connected fill capped at 65,536 cells, bounded undo/redo, and a collision-only brush.
- Player play/pause/step APIs, standalone project export and launchers, plus editor runtime preview with GUID-based rendering and selection.
- Luau VM isolation with source and per-VM memory caps, fixed-tick Forge APIs, bounded typed events, compatible scalar state migration, resumable line breakpoints, error isolation, and last-good hot reload.
- Kira-backed audio commands, scene-transition requests, a retained game UI runtime, UI rendering/input, and the Action Arena HUD/menu/audio/scene sample. Editor preview handles UI pointer/keyboard input, scripted transitions, and audio playback.
- The player `--validate` command checks project scenes/assets without modifying the project or executing Luau top-level code; the updated editor-authored Action Arena passed validation and export in run `38934893620`. The player maps WASD/F/right-click to actions 1/2/3, and run `38935389248` passed its input-to-movement and fire-to-SFX integration test. The Windows package workflow asserts that the exported project includes the tilemap, tileset, both UI documents, both scripts, and both audio files.
- The editor-authored Action Arena now contains the full playable loop in one project: a camera and tile room, animated player and enemies, movement, projectile collisions, score/lives HUD, menu, audio, scene transitions, and restart flow. Windows runtime tests cover movement, fire audio, projectile scoring, enemy contact, and HUD updates.
- The editor-authored colony project includes a chunked negative-coordinate tilemap, eight animated workers, four resource nodes, bounded queues, deterministic BFS routes, and an authored HUD. Its Windows runtime test verifies worker delivery, script health, animation loading, and obstacle avoidance.
- Run [38948522741](https://github.com/danielmartinez-maker/Warmdot/actions/runs/38948522741) validates and exports both projects, checks required files and license notices, and uploads separate Windows artifacts. Generic standalone projects receive WASD actions 1/2 and F/right-click action 3.

### Confirmed gaps or partial coverage

- Editor preview and standalone player interactions still need a player-facing desktop session for focus loss, pause/step, live changes, last-good behavior, breakpoints, HUD/audio, and scene transitions. No Remote Desktop device is connected to this task, and native CUA controls are unavailable.
- The tutorial exists and is offline, but it has not been completed through the editor and exported player.
- The Action Arena's authored project files persist, while runtime score and lives are session data that reset when the Main scene is loaded again.
- The colony Rust sample still provides the larger 64-worker stress run with runtime obstacle editing, time controls, and profiling; the separate saved colony project is the editor/export vertical slice.
- Tilemaps are chunked in storage, but rendering/collision still expand per tile; incremental large-map rendering/collision and generalized navigation remain open.
- The renderer benchmark has not been run on a desktop GPU, so no performance results are claimed.


---

## P0 — Minimal Playable Editor (active)

Work each group as a reviewable vertical slice; update this plan with exact commit and CI evidence after each group.

### P0.1 — Authoring model and editor workflow

- [x] Finish project settings and the asset import workflow. The editor edits the project name/default scene, imports external files through bounded drag/drop, lists stable-GUID assets, and validates imported resources.
- [x] Add object hierarchy/parent transforms, reusable components, versioned prefab assets/instances, and stable component/property override identifiers. Parent cycles and missing references produce diagnostics; prefab refresh preserves granular local overrides.
- [x] Complete viewport selection, transform/anchor gizmos, snapping, and selectable/rendered layers. Move, rotate, scale, and anchor edits commit through scene history; editor-only layer visibility is selectable.
- [x] Add bounded connected-region tile fill and an explicit collision-only brush. Each edit is recorded as an undoable stroke.
- [x] Keep editor changes on the validated command/history model with bounded undo/redo, saved-state markers, and actionable save/open errors.

Windows formatting, workspace type-check, and workspace tests passed on commit `aa6173577d4116defac860a406ea1e2ab8a04ea4` (run `38873215811`). This verifies the authoring implementation; a manual desktop-editor session is still required for visual acceptance.

**Exit:** The authoring workflow and data model are implemented and pass the Windows workspace checks. A user can create/open/save a project, import a sprite, compose/reuse a prefab, author a scene and tilemap, undo edits, and reopen saved data.

### P0.2 — Luau game scripting

- [x] Register bounded .luau assets and attach scripts to authored objects through scene dependencies.
- [x] Add a sandboxed Luau host with on_start/on_fixed_update, authored boolean/number/string properties, numeric input actions, and position/translation APIs.
- [x] Bound source size (1 MiB), VM memory (4 MiB), scene instances (64), and callback interrupt checkpoints (20,000); isolate errors with source path, object GUID, and tick.
- [x] Support transactional source compilation reloads and retain the last working VM when replacement compilation fails.
- [x] Add an editor runtime script diagnostics console and fixed-tick pause/step controls.
- [x] Add script-line breakpoints with resumable callback execution and a breakpoint hit result. `ScriptHost` retains the loaded source chunk so breakpoints apply to nested callbacks; the runtime resumes the paused call at the same fixed tick and reports object, asset, source, line, and tick.
- [x] Broadcast bounded scalar events on the next fixed tick with deterministic delivery ordering.
- [x] Migrate same-name boolean, integer, number, and string state fields on successful reload; preserve on_start completion and diagnose fields that reset.

The existing suite covers asset GUID stability/reload, old-scene compatibility and script round-tripping, property/input updates, runaway isolation, last-good source reload, live property updates without VM recompilation, state migration, typed-event ordering/bounds, and the integrated breakpoint/editor API. Windows formatting, workspace type-checking, and workspace tests passed on `faeaa9679bfd2d497f6acddbf943fd37b73cd895` (run `38879901911`).

**Exit:** The action sample's player behavior is authored in Luau, edits are reflected safely in play mode when compatible, and invalid/runaway scripts report diagnostics without terminating the editor or last-good game state.

### P0.3 — Play-in-editor and live updates

- [x] Embed a runtime preview in the editor with play, pause, one-step, and stop/reset controls.
- [x] Keep editor selection available while playing and resolve rendered runtime sprites by stable authored GUID.
- [x] Apply compatible script-property, PNG texture, and Luau source updates transactionally; incompatible property/source changes retain their prior working value/VM and report diagnostics.
- [x] Define pause accumulator reset, stop/discard behavior, and prevent scene switching during a preview.
- [x] Add source-line breakpoint Set/Remove controls, accepted-line feedback, hit metadata, and Continue in the editor preview.
**Exit:** A project can run inside the editor; compatible property/asset/script changes visibly update the live game, while rejected updates retain the last-good state. Code and CI are verified; player-facing desktop acceptance remains open.

### P0.4 — Audio, transitions, and game UI

- [x] Add a Forge-owned playback API and Kira adapter for sound effects, music/streaming, and sound buses.
- [x] Add authored scene-transition requests and player/editor scene lifecycle integration.
- [x] Add a separate retained-mode game UI tree with basic layout, image/text/button, focus, and input; keep egui editor UI separate.
- [x] Add runtime UI rendering and sample HUD/menu interaction.

Implementation and Windows format/type-check/test evidence: commit `99d3bf7f024bb425ee17c48c072f2112fbd92836`, runs `38917440379` and `38917444348`. The P0.4 exit remains open until the standalone action sample is exercised in a native desktop session; P0.3 editor-preview desktop acceptance also remains open.

**Exit:** Pending player-facing desktop acceptance of responsive HUD/menu input, sound effects/music, and transitions between authored scenes in the standalone player.

### P0.5 — Runtime diagnostics and dependable shipping

- [x] Add structured runtime/editor logging, a visible error console, and a profiler overlay for frame percentiles, system/script cost, draw calls, and entity/chunk counts. Windows run `38931980946` passed the workspace tests on the diagnostics implementation.
- [x] Make the Windows player/export path reproducible from clean CI and document its runtime/license contents. Run `38931980946` built, validated, exported, checked, generated third-party license notices, and uploaded the Action Arena package. Run `38935389248` also built, validated, and exported the updated editor project, passed the expanded package-content checks, and uploaded the artifact. Run `38938549080` passed all source checks and the full Windows release/export/package workflow at the latest source head.
- [x] Add import/scene validation commands or editor equivalents with deterministic diagnostic output. `forge-player --validate` passed against Action Arena in runs `38931980946` and `38934893620`.
- [x] Add schema migration fixtures and serialization round trips for projects, scenes, prefabs, scripts, and tilemaps. Workspace tests passed in run `38931980946`.
- [x] Add an offline first-game tutorial in `docs/tutorials/first-playable-game.md`; its project assets and instructions are local and it does not require online services.

**Exit:** Still open. Structured logs, profiler data, migration fixtures, and license notices are implemented and have Windows CI evidence. Run `38948522741` passed both project validations/exports, required package-content checks, license generation, and both artifact uploads at the latest source head. Native desktop acceptance and a player-facing tutorial completion pass remain pending.

### P0.6 — Two real sample vertical slices and release gate

- [x] Complete one editor-authored Action Arena with enemy behavior, animations, collisions, camera, tile room, HUD, audio, transitions, saveable project data, projectiles, scoring, lives, and restart. The project, scenes, tilemap, scripts, UI, and assets are authored files; Windows runtime tests cover movement, projectile scoring, enemy contact/life updates, and the HUD. Run `38948522741` validates and exports the complete project.
- [x] Complete the colony simulation behaviors: walkable/unwalkable tile editing, resource jobs, queues/navigation, time controls, and system profiling. The native 64-worker sample retains runtime obstacle editing and profiling. The editor-authored project adds deterministic worker routing, resource deliveries, animations, an authored HUD, and a standalone export. Run `38948522741` passed the runtime integration test and both project exports.
- [ ] Verify a newcomer can build and change the complete action game through the editor without Rust edits; complete the tutorial walkthrough.
- [ ] Run the P0 acceptance audit against every P0 bullet and both sample exits; publish no performance claim without measurements.

**P0 completion gate:** Every P0 item above must have current evidence, Windows CI must be green, both vertical slices must run through editor and standalone export, and remaining gaps must be explicitly zero. This gate is not met: native editor/player sessions, the tutorial walkthrough, and the full audit remain open.

## P1 — Differentiated Public Beta (start only after P0 gate)

- [ ] Multi-layer chunked maps, tile rules/autotiling, streamable large worlds, and incremental render/collision/navigation updates.
- [ ] Navigation grids, async pathfinding, deterministic result commit ordering, job system/scheduler, and agent utilities.
- [ ] Event/timeline animation, hitboxes, particles, and 2D lights/shadows.
- [ ] Responsive game UI constraints, data binding, inventory and list widgets.
- [ ] Asset dependency graph/invalidation, prefab overrides, hot-reload compatibility rules, and migration tooling.
- [ ] Deterministic simulation playback with seeded inputs, snapshots, replay, state inspection, and temporal diagnostics under the proposal's same-build contract.
- [ ] Windows, macOS, and Linux export; document a Web build proof-of-concept only if the dependency graph supports it.
- [ ] Versioned plugin API baseline, command-line import/export/test, full developer documentation, and example projects.

**P1 completion gate:** All public-beta features work in both sample projects, CI/export matrix is reproducible, migration/replay contracts are verified, and benchmark reports include hardware and workload.

---

## P2 — Later Ecosystem (start only after P1 gate)

- [ ] Visual scripting and visual logic graph.
- [ ] Localized text/workflow and higher-end particles/shaders.
- [ ] Navigation across floors/portals and specialized automation/economy authoring tools.
- [ ] Full debugger and opt-in AI-assisted editor actions with explicit project-scoped capabilities.
- [ ] Mobile export, optional multiplayer primitives, and a community asset repository.

**P2 completion gate:** Each item has a documented compatibility, capability, migration, and packaging contract; opt-in AI and community assets remain offline/safe by default.

---

- 2026-10-16: Completed P0.4 implementation on `99d3bf7f024bb425ee17c48c072f2112fbd92836`: Kira audio, authored scene transitions, retained game UI, Action Arena HUD/menu/audio resources, and editor-preview UI/audio/transition handling. Windows runs `38917440379` and `38917444348` passed format, type-check, and workspace tests. Native desktop interaction acceptance remains open.
- 2026-10-16: Added `forge-player --validate` with read-only asset-index consistency checking, scene/resource validation, JSON diagnostics, and syntax-only Luau compilation. Updated SceneCatalog to reject malformed scene files instead of silently omitting them. Formatting correction passed on head `e2ae31bf430e7d923c2a1ed1f973ad82caa62736`; type-check and tests were still running when the Windows export workflow was added.
- 2026-10-16: Added the Windows release build/validation/export/artifact workflow, copied Forge's MIT license into exports, and documented package contents. Run `38920804391` on `18da157e53720d0610c765d7f7918bab23294be0` passed all check and package steps; artifact `forge2d-action-arena-windows` uploaded. The first validation attempt exposed Windows CRLF conversion in fingerprinted text assets; `.gitattributes` now pins text to LF and CI passes.
- 2026-10-16: Wired the standalone project player's WASD/F/right-click input map into the runtime using the existing bounded `InputMap` and fixed action IDs 1/2/3. Added an offline first-game tutorial and corrected the scripting/input docs; commit `0a0a3d85b90cdd40dbfa309c2f56103e49d4a44` is under Windows CI run `38922572623`.

## Execution ledger

- 2026-10-15: Created from the user-provided product proposal after the user authorized independent planning and milestone work. Verified the live branch head and PR #1 through GitHub; current checked head is `2a9ae3e`.
- 2026-10-15: Read the proposal and compared its full P0/P1/P2 scope against the GitHub README and repository tree. P0 remains incomplete; the README's shorter remaining-work list is not the completion authority.
- 2026-10-16: Added the native editor runtime preview, play/pause/step/stop controls, GUID-based runtime rendering and selection, a 256-entry Luau diagnostics console, live script-property edits that preserve the running VM, and hot reload for changed Luau/PNG assets. Added tests for compatible and rejected live property updates. Fixed the Action Arena player GUID lookup and nested collider shape JSON after Windows CI caught them. Formatting, type-checking, and workspace tests passed on commit f87a949 (run 38861032595).
- 2026-10-16: Added a stable-GUID asset browser/resource validator, project name/default-scene settings, and explicit undoable collision-only tile painting; bounded connected fill and viewport pan/zoom are also complete. Windows formatting, workspace type-checking, and workspace tests passed on `6865d674` (run `38864521933`). The asset browser commit `89bd190` passed the same Windows checks on run `38863863484`.
- 2026-10-16: Completed the P0.1 authoring-model slice on `aa6173577d4116defac860a406ea1e2ab8a04ea4`: bounded drag/drop asset import; parent hierarchy and world transforms; prefab instances with granular sprite property overrides; move/rotate/scale/anchor gizmos with snapping; selectable render layers; and validated history/save handling. Windows run `38873215811` passed formatting, workspace type-check, and workspace tests.
- 2026-10-16: Added compatible scalar state migration and typed next-tick events to P0.2, then fixed event docs/types and bounded editor preview input while paused with window-focus/text-input gating. Windows formatting, type-check, and workspace tests passed on `d34a4835157ba07beb0f54377b5036e8058a6485` (run `38875425714`). Line breakpoints remain the P0.2 code gap; P0.3 still needs player-facing verification.
- 2026-10-16: Added resumable Luau line breakpoints, deterministic callback queue suspension, fixed-tick Continue semantics, and editor Set/Remove/Continue controls with accepted-line and hit feedback. Windows formatting, workspace type-check, and workspace tests passed on `faeaa9679bfd2d497f6acddbf943fd37b73cd895` (run `38879901911`). P0.3 still needs a player-facing desktop acceptance session.
- 2026-10-16: Completed P0.5 structured logs/profiling, migration/round-trip fixtures, and third-party license packaging. Windows run `38931980946` passed format/type-check/tests, Action Arena validation/export, license generation, package checks, and artifact upload. Run `38934893620` passed the updated editor-project validation, full asset-content checks, license generation, and artifact upload; the duplicate package job for `38935389248` is still running.
- 2026-10-16: Expanded the colony simulation with blocked-tile editing, queued food jobs, deterministic BFS rerouting, pause/time-scale controls, worker cap, and profiling. Windows run `38931980946` passed. It remains a Rust example and is not yet available as an editor-authored/exported project.
- 2026-10-16: Expanded the Action Arena into a saved top-down project with tilemap borders, zero-gravity controllable player, moving collider enemies, UI menu/HUD, audio, and scene transitions. The Rust integration sample has projectiles/scoring. Fixed an unexposed fire-state property, removed player launch velocity, and added input-to-movement/fire-to-SFX coverage; run `38935389248` passed formatting, type-check, and workspace tests.
- Next: complete the native editor/player acceptance sessions, walk through the offline first-game tutorial, and run the full P0 acceptance audit. Do not start P1 until the documented gate is satisfied.

- 2026-10-16: Integrated the action gameplay loop into the editor-authored Action Arena project and added a separate editor-authored colony project with animated workers, bounded jobs, deterministic routes, resource deliveries, and an authored HUD. Added a colony runtime integration test and Windows validation/export/package checks for both samples. Run `38947562709` first exposed a life-HUD assertion that checked before collision event delivery; the test now advances through the next fixed tick. Run `38948522741` passed formatting, type-check, all workspace tests, release-player build, both project validations/exports, license and package-content checks, and both artifact uploads at commit `4d041011fcb7219c0040534b94a623a681e87561`. Action Arena artifact: 5,664,463 bytes; colony artifact: 5,494,087 bytes. P0 remains open because native desktop acceptance, tutorial walkthrough, and full P0 audit are outstanding; P1 has not started.

- 2026-10-16: Current-head run `38974381930` at `61700790fcd1eec6c6d72d7172875309e90497fd` passed formatting, workspace type-check, all workspace tests (including the Colony editor-preview tilemap regression), release player/editor/benchmark builds, both project validations/exports, license and package-content checks, and post-upload validation. SHA-256 hashes of all three downloaded ZIPs matched GitHub's artifact digests. Since `3ce085ec8b1a` to this head only `examples/forge_editor/src/main.rs` changed, the approved run-535 player is current for standalone runtime checks: the tutorial project validated and D/A/W/S visibly moved the sprite; the Action Arena menu-to-game roundtrip was also visually verified. Windows Application Control blocked launching the newly built #539 editor/player binaries, and no Desktop Commander device is connected. The latest editor preview map fix still lacks native visual acceptance; acoustic audio acceptance, a fresh benchmark, the tutorial's editor walkthrough, and the full P0 audit remain open. P1 has not started.
