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

Repository: `danielmartinez-maker/Warmdot`; branch `codex/forge2d-engine`; PR #1, draft and unmerged. Verified snapshot commit `6865d674ed5474246ae710493ce4fa76e146f4fe` (Windows run `38864521933` passed format, workspace type-check, and tests).

### Present in the branch

- Rust workspace, GUIDs, fixed-step clock, input actions, validated project/scene data, Bevy ECS abstraction, Rapier physics, wgpu sprite renderer.
- PNG, sprite-sheet, animation-clip, and bounded Luau script asset loading; fixed-tick sprite animation and scene-script lifecycle.
- Native scene editor with hierarchy, transforms/components, undo/redo, dirty-state saving, project and multi-scene lifecycle; project settings can edit the project name and choose the current saved scene as startup scene.
- Sparse 32x32 tilemaps with paint/erase, bounded undo/redo, connected fill capped at 65,536 cells, and a collision-only brush that preserves tile art and records strokes as one undo entry.
- Player play/pause/step APIs, standalone project export and launchers.
- Luau VM isolation with source and per-VM memory caps, a fixed-tick Forge transform/property/input API, runaway callback interruption, per-script diagnostics, transactional compile reloads, and last-good script retention on invalid replacements.
- Editor inspector script attachment and boolean/number/string property editing; action-arena movement is authored in Luau in both the project asset and playable keyboard sample.
- Embedded editor runtime preview with Play/Pause/Resume/Step/Stop, GUID-based runtime rendering and selection, a capped script diagnostics console, live compatible script-property updates, PNG texture refresh, and transactional Luau hot reload.
- Windows workflow checks formatting, type-checking, and workspace tests.
- Playable keyboard/mouse action arena, colony simulation sample, renderer benchmark.
- GitHub Actions checks Windows formatting, workspace type-checking, and workspace tests.

### Confirmed gaps or partial coverage

- The editor now embeds play/pause/one-tick-step/stop and a script diagnostics console, with live compatible script-property, PNG texture, and Luau source updates. Script-line breakpoints and a full debugger remain absent.
- The editor preview still needs a player-facing verification pass for input focus loss, pause/step, live changes, and last-good behavior.
- Project settings currently cover the project name and startup scene; a full settings/import flow is still missing. The basic asset browser lists stable GUIDs and validates resources, but does not copy/import external files.
- Viewport pan/zoom and selection are present; transform/anchor gizmos, snapping, and selectable/rendered layer controls are still incomplete at this verified snapshot.
- No game audio runtime, retained game UI, in-game scene transition flow, profiler/logging overlay, or tutorial.
- No prefab/instance/override authoring model or parent-transform workflow; no full external asset-import UI; no anchor/rotation/scale gizmos or complete layer workflow.
- Current tilemaps are chunked in storage, but rendering/collision are expanded per tile; large-map incremental chunk rendering/collision and navigation are not implemented.
- Samples are early gameplay slices, not the proposal's complete P0 vertical slices.

---

## P0 — Minimal Playable Editor (active)

Work each group as a reviewable vertical slice; update this plan with exact commit and CI evidence after each group.

### P0.1 — Authoring model and editor workflow

- [ ] Finish project settings and the asset import workflow. The project name/default-scene controls and basic stable-GUID browser/resource validation are implemented; importing files from outside the project and richer import diagnostics remain.
- [ ] Add object hierarchy/parent transforms, reusable components, prefab assets, prefab instances, and stable component/property override identifiers.
- [ ] Complete viewport selection, transform/anchor gizmos, snapping, and selectable/rendered layers. Pan/zoom and selection are present; the remaining authoring tools are still open.
- [x] Add bounded connected-region tile fill and an explicit collision-only brush. Each edit is recorded as an undoable stroke; Windows run `38864521933` passed formatting, workspace type-check, and tests on `6865d674`.
- [ ] Keep all editor changes on the validated command/history model with save markers and error recovery.

**Exit:** A user can create/open/save a project, import a sprite, compose and reuse a prefab, author a scene and tilemap through the editor, undo every operation, and reopen the same data without losing or mutating it.

### P0.2 — Luau game scripting

- [x] Register bounded .luau assets and attach scripts to authored objects through scene dependencies.
- [x] Add a sandboxed Luau host with on_start/on_fixed_update, authored boolean/number/string properties, numeric input actions, and position/translation APIs.
- [x] Bound source size (1 MiB), VM memory (4 MiB), scene instances (64), and callback interrupt checkpoints (20,000); isolate errors with source path, object GUID, and tick.
- [x] Support transactional source compilation reloads and retain the last working VM when replacement compilation fails.
- [x] Add an editor runtime script diagnostics console and fixed-tick pause/step controls.
- [ ] Add script-line breakpoints, typed events, and compatible state migration behavior.

Tests cover asset GUID stability/reload, old-scene compatibility and script round-tripping, property/input updates, runaway isolation, last-good source reload, and live property updates without VM recompilation. Windows formatting, workspace type-checking, and workspace tests passed on commit f87a949 (run 38861032595).

**Exit:** The action sample's player behavior is authored in Luau, edits are reflected safely in play mode when compatible, and invalid/runaway scripts report diagnostics without terminating the editor or last-good game state.

### P0.3 — Play-in-editor and live updates

- [x] Embed a runtime preview in the editor with play, pause, one-step, and stop/reset controls.
- [x] Keep editor selection available while playing and resolve rendered runtime sprites by stable authored GUID.
- [x] Apply compatible script-property, PNG texture, and Luau source updates transactionally; incompatible property/source changes retain their prior working value/VM and report diagnostics.
- [x] Define pause accumulator reset, stop/discard behavior, and prevent scene switching during a preview.
- [ ] Verify input focus loss, pause/step, live updates, and last-good behavior in a player-facing editor session.

**Exit:** A project can run inside the editor; compatible property/asset/script changes visibly update the live game, while rejected updates retain the last-good state.

### P0.4 — Audio, transitions, and game UI

- [ ] Add a Forge-owned playback API and Kira adapter for sound effects, music/streaming, and sound buses.
- [ ] Add authored scene-transition requests and player/editor scene lifecycle integration.
- [ ] Add a separate retained-mode game UI tree with basic layout, image/text/button, focus, and input; keep egui editor UI separate.
- [ ] Add runtime UI rendering and sample HUD/menu interaction.

**Exit:** The action sample has responsive HUD/menu, sound effects/music, and a real transition between authored scenes using the standalone player.

### P0.5 — Runtime diagnostics and dependable shipping

- [ ] Add structured runtime/editor logging, a visible error console, and a profiler overlay for frame percentiles, system/script cost, draw calls, and entity/chunk counts.
- [ ] Make the Windows player/export path reproducible from clean CI and document its runtime/license contents.
- [ ] Add import/scene validation commands or editor equivalents with deterministic diagnostic output.
- [ ] Add schema migration fixtures and serialization round trips for projects, scenes, prefabs, scripts, and tilemaps.
- [ ] Add a newcomer tutorial that builds a small playable game using the editor.

**Exit:** Clean Windows CI opens/validates/builds/exports a template project; logs and performance costs are inspectable; the tutorial is completable offline.

### P0.6 — Two real sample vertical slices and release gate

- [ ] Complete the action arena with enemy behavior, animations, collisions, camera, tile room, HUD, audio, transitions, saveable authored data, and editor-built workflow.
- [ ] Complete a small colony simulation with walkable/unwalkable tile edits, resource jobs, queues/navigation, time controls, and system profiling.
- [ ] Verify a newcomer can build and change the action game through the editor without Rust edits.
- [ ] Run the P0 acceptance audit against every P0 bullet and both sample exits; publish no performance claim without measurements.

**P0 completion gate:** Every P0 item above is checked with current repository evidence, Windows CI is green, both vertical slices run through editor and standalone export, and remaining gaps are explicitly zero.

---

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

## Execution ledger

- 2026-10-15: Created from the user-provided product proposal after the user authorized independent planning and milestone work. Verified the live branch head and PR #1 through GitHub; current checked head is `2a9ae3e`.
- 2026-10-15: Read the proposal and compared its full P0/P1/P2 scope against the GitHub README and repository tree. P0 remains incomplete; the README's shorter remaining-work list is not the completion authority.
- 2026-10-16: Added the native editor runtime preview, play/pause/step/stop controls, GUID-based runtime rendering and selection, a 256-entry Luau diagnostics console, live script-property edits that preserve the running VM, and hot reload for changed Luau/PNG assets. Added tests for compatible and rejected live property updates. Fixed the Action Arena player GUID lookup and nested collider shape JSON after Windows CI caught them. Formatting, type-checking, and workspace tests passed on commit f87a949 (run 38861032595).
- 2026-10-16: Added a stable-GUID asset browser/resource validator, project name/default-scene settings, and explicit undoable collision-only tile painting; bounded connected fill and viewport pan/zoom are also complete. Windows formatting, workspace type-checking, and workspace tests passed on `6865d674` (run `38864521933`). The asset browser commit `89bd190` passed the same Windows checks on run `38863863484`.
- Next: finish the remaining P0.1 hierarchy/prefab/import/gizmo workflow, then P0.2 breakpoints/events/state migration, P0.4 audio/UI/transitions, P0.5 shipping/diagnostics/tutorial, and both P0.6 sample exits. Keep the P0 gate closed until the preview and both samples receive player-facing acceptance.
