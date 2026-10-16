# Player validation, runtime contents, and licensing

## Validate a project

Run the standalone player in validation mode:

    cargo run -p forge-player -- --validate path/to/project.forgeproj

The command is read-only. It checks the project manifest and every scene under `Scenes/`, confirms the asset index matches the scan of `Assets/`, validates referenced asset kinds and resource data, checks the default tilemap and tileset indices, and compiles Luau source without executing it. It prints a schema-versioned JSON report with diagnostics sorted by severity, code, path, and message. A valid project exits with status 0; invalid data exits non-zero after printing the report.

A missing asset index is accepted only when the project has no indexed assets. The validation command never repairs the index or creates project folders.

## Windows x64 export contents

The Windows Actions workflow builds the release player from a clean checkout, validates the Action Arena project, exports it, checks required package files, and uploads the package as the `forge2d-action-arena-windows` artifact.

The exported folder contains:

| Path | Contents |
| --- | --- |
| `Forge2DPlayer.exe` | Forge player binary built for Windows x64 by the current CI toolchain |
| `run-game.bat` | Path-relative launcher for the packaged project |
| `LICENSE` | Forge 2D's MIT license text |
| `Game/project.forgeproj` | Project name and default-scene resource |
| `Game/Scenes/` | Authored scene resources |
| `Game/Assets/` | Project assets copied without following symlinks |
| `Game/.forge/asset-index.json` | Stable asset GUIDs and source fingerprints |

The package runs without the editor, Rust, or Cargo installed. It relies on the host's Windows graphics drivers; game audio remains optional when no compatible audio device is available. The CI artifact proves the build, validation, and package layout on a clean Windows runner. It does not prove the native window opens on a user's GPU or that audio is audible.

The root `LICENSE` in the repository covers Forge-owned code and is copied into the export. Rust dependencies are pinned by the repository's `Cargo.lock`; each dependency's license expression and source are recorded in its Cargo package metadata. This export currently does not bundle a consolidated copy of every third-party license text, so third-party notice packaging remains an open release item.

A clean build using the floating Rust `stable` channel is repeatable from the lockfile in CI, but the binary is not claimed to be bit-for-bit reproducible across toolchain releases or runner images.
