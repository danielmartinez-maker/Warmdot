# Standalone project export

Run the player binary with:

    forge-player --export path/to/project.forgeproj path/to/export-folder

The exporter creates a `Forge2DPlayer.exe` and `run-game.bat` on Windows, or a `Forge2DPlayer` binary and executable `run-game.sh` on Unix. It copies the project manifest, its default scene, the Assets tree, and the Forge asset index into a `Game` subfolder. The launcher opens the packaged project and forwards additional command-line arguments to the player.

The export folder must not be inside the source project, and the source project must not be inside the export folder. The exporter rejects a symlinked manifest, scene, or asset-index file and skips symlink entries while copying Assets. It verifies that the default scene and the Assets and metadata folders resolve inside the project before copying them.

This packages the currently running player binary, so exports target the operating system and architecture of that build. The editor can author multiple scene resources under `Scenes/`, and the exporter copies that folder with the default scene. The player still starts from the project's configured default scene; there is no in-game scene-selection UI yet.
