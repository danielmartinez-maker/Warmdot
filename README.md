# Forge 2D

Forge 2D is a 2D-first game engine project for action games, RPGs, and simulation-heavy worlds. Its design keeps editable project and scene files separate from runtime storage and keeps third-party engine libraries behind Forge-owned interfaces.

This repository is being initialized from the Forge 2D product proposal. The implementation is in progress; the proposal's P0 release includes a usable editor, runtime services, two sample games, and reproducible Windows builds. No performance claims are made until the documented benchmark has been run.

## Project goals

- Stable, reviewable project, scene, and asset formats
- A fixed-step runtime with explicit input actions
- A 2D renderer with measured workload baselines
- Physics, scripting, animation, audio, and game UI behind replaceable adapters
- A visual editor that supports a complete game creation workflow
- Sparse chunk storage for simulation-heavy worlds

## Status

The GitHub repository was empty when implementation began. Initial source and architecture are being added in reviewable commits. See the product specification and engine plan for the full P0 scope and remaining milestones.