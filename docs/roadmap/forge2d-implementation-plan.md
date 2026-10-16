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

Repository: `danielmartinez-maker/Warmdot`; branch `codex/forge2d-engine`; PR #1, draft and unmerged. Verified code snapshot `d34a4835157ba07beb0f54377b5036e8058a6485`; Windows run `38875425714` passed formatting, workspace type-check, and workspace tests. The latest documentation-only commits follow that verified code snapshot.

### Present in the branch

- Rust workspace, GUIDs, fixed-step clock, input actions, validated project/scene data, Bevy ECS abstraction, Rapier physics, and the wgpu sprite renderer.
- PNG, sprite-sheet, animation-clip, prefab, and bounded Luau asset loading; stable asset GUIDs and deterministic resource validation.
- Native editor project-name/default-scene settings, external file import by drag/drop, asset browser/inspector, multi-scene lifecycle, hierarchy, parent transforms, and history-backed edits with dirty-state save markers.
- Versioned prefab documents and instances with stable object/component/property override identifiers; refreshing a prefab preserves only locally overridden fields.
- Viewport selection, pan/zoom, move/rotate/scale/anchor gizmos, snapping, sprite anchors, and selectable render-layer visibility.
- Sparse 32x32 tilemaps with paint/erase, bounded undo/redo, connected fill capped at 65,536 cells, and a collision-only brush that preserves tile art.
- Player play/pause/step APIs, standalone project export and launchers.
- Luau VM isolation with source and per-VM memory caps, a fixed-tick Forge transform/property/input/event API, bounded next-tick typed events, compatible scalar state migration on successful hot reload, runaway callback interruption, per-script diagnostics, and last-good script retention on invalid replacements.
- Editor runtime preview with Play/Pause/Resume/Step/Stop, GUID-based rendering and selection, script diagnostics, live compatible property updates, PNG texture refresh, transactional Luau hot reload, and input focus/paused-queue gating.
- Playable keyboard/mouse action arena, colony simulation sample, and renderer benchmark. Windows GitHub Actions checks formatting, workspace type-checking, and workspace tests.

### Confirmed gaps or partial coverage

- The native Luau integration still lacks script-line breakpoints; typed events and compatible scalar state migration are implemented.
- The editor preview still needs player-facing verification for focus loss, pause/step, live changes, and last-good behavior.
- No game audio runtime, retained game UI, scene-transition flow, profiler/logging overlay, or newcomer tutorial.
- Current tilemaps are chunked in storage, but rendering/collision are expanded per tile; large-map incremental rendering/collision and navigation are not implemented.
- The action and colony samples are early gameplay slices, not the complete P0 vertical slices. The renderer benchmark has not been run on a desktop GPU, so no performance results are claimed.

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
- [ ] Add script-line breakpoints with resumable callback execution and a breakpoint hit result.
- [x] Broadcast bounded scalar events on the next fixed tick with deterministic delivery ordering.
- [x] Migrate same-name boolean, integer, number, and string state fields on successful reload; preserve on_start completion and diagnose fields that reset.

The existing suite covers asset GUID stability/reload, old-scene compatibility and script round-tripping, property/input updates, runaway isolation, last-good source reload, live property updates without VM recompilation, state migration, and typed-event ordering/bounds. Windows formatting, workspace type-checking, and workspace tests passed on `d34a4835157ba07beb0f54377b5036e8058a6485` (run `38875425714`).

**Exit:** The action sample's player behavior is authored in Luau, edits are reflected safely in play mode when compatible, and invalid/runaway scripts report diagnostics without terminating the editor or last-good game state.

### P0.3 — Play-in-editor and live updates

- [x] Embed a runtime preview in the editor with play, pause, one-step, and stop/reset controls.
- [x] Keep editor selection available while playing and resolve rendered runtime sprites by stable authored GUID.
- [x] Apply compatible script-property, PNG texture, and Luau source updates transactionally; incompatible property/source changes retain their prior working value/VM and report diagnostics.
- [x] Define pause accumulator reset, stop/discard behavior, and prevent scene switching during a preview.
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
- 2026-10-16: Completed the P0.1 authoring-model slice on `aa6173577d4116defac860a406ea1e2ab8a04ea4`: bounded drag/drop asset import; parent hierarchy and world transforms; prefab instances with granular sprite property overrides; move/rotate/scale/anchor gizmos with snapping; selectable render layers; and validated history/save handling. Windows run `38873215811` passed formatting, workspace type-check, and workspace tests.
- 2026-10-16: Added compatible scalar state migration and typed next-tick events to P0.2, then fixed event docs/types and bounded editor preview input while paused with window-focus/text-input gating. Windows formatting, type-check, and workspace tests passed on `d34a4835157ba07beb0f54377b5036e8058a6485` (run `38875425714`). Line breakpoints remain the P0.2 code gap; P0.3 still needs player-facing verification.
- Next: implement resumable line breakpoints, then finish the player-facing P0.3 verification, P0.4 audio/UI/transitions, P0.5 diagnostics/shipping/tutorial, and P0.6 sample exits before opening P1.
