# Native Forge player

Run a project with:

    cargo run -p forge-player -- path/to/project.forgeproj

The player opens the project's confined default scene, creates the fixed-step runtime, opens the project asset catalog, and uploads every PNG asset to the renderer by its stable GUID. It renders the active camera and sprite order from the runtime world. PNG additions, edits, and removals are picked up by the asset catalog scan while the window remains open.

This first runner is for authored scenes and runtime services that are already available in Rust. The project format does not yet contain a script component, and the player does not yet provide a native editor, game UI, audio service, or standalone export bundle.
