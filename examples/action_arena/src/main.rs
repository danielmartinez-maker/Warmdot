use std::{env, io, path::PathBuf, process, time::Duration};

use forge_assets::AssetCatalog;
use forge_core::Guid;
use forge_project::Project;
use forge_runtime::{GameRuntime, RuntimeConfig};

fn main() {
    if let Err(error) = run() {
        eprintln!("action-arena: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let project_path = env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let project = Project::open(project_path)?;
    let assets = AssetCatalog::open(project.root())?;
    let mut runtime = GameRuntime::from_scene_with_assets(
        project.default_scene(),
        RuntimeConfig::default(),
        &assets,
    )?;

    let mut frames = 0_u64;
    for _ in 0..120 {
        let frame = runtime.advance_frame(Duration::from_nanos(16_666_667));
        frames += u64::from(frame.fixed_steps);
        for event in frame.physics_events {
            println!("physics event: {event:?}");
        }
    }

    let player_guid = "529bb9ad-47b8-4992-874e-055c7fbf0003"
        .parse::<Guid>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid sample Player GUID"))?;
    let player = runtime.entity_for_guid(player_guid).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "sample Player object was not loaded",
        )
    })?;
    let position = runtime
        .world()
        .transform(player)
        .ok_or_else(|| io::Error::other("Player transform missing"))?;
    println!(
        "ran {frames} fixed steps; Player settled at ({:.2}, {:.2})",
        position.translation[0], position.translation[1]
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use forge_core::ActionId;
    use forge_runtime::ScriptAudioCommand;

    use super::*;

    #[test]
    fn action_arena_runs_its_authored_luau_controller() {
        let project = Project::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .expect("open Action Arena sample");
        let assets = AssetCatalog::open(project.root()).expect("open sample assets");
        let player_guid = "529bb9ad-47b8-4992-874e-055c7fbf0003"
            .parse::<Guid>()
            .expect("parse sample player GUID");
        let mut runtime = GameRuntime::from_scene_with_assets(
            project.default_scene(),
            RuntimeConfig::default(),
            &assets,
        )
        .expect("load authored scene and controller");
        let player = runtime
            .entity_for_guid(player_guid)
            .expect("sample player exists");
        let before = runtime
            .world()
            .transform(player)
            .expect("player transform")
            .translation[0];

        let _ = runtime.take_audio_commands();
        runtime.push_action(Duration::ZERO, ActionId(1), 1.0, true);
        runtime.push_action(Duration::ZERO, ActionId(3), 1.0, true);
        runtime.advance_frame(Duration::from_millis(50));

        let after = runtime
            .world()
            .transform(player)
            .expect("player transform")
            .translation[0];
        assert!(after > before, "Luau controller should move the player");
        assert!(runtime.take_script_diagnostics().is_empty());
        let audio_commands = runtime.take_audio_commands();
        assert!(audio_commands.iter().any(|command| matches!(
            command,
            ScriptAudioCommand::PlayEffect { bus, .. } if bus == "sfx"
        )));
    }
}
