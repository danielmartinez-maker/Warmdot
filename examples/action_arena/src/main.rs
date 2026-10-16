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
    use forge_runtime::{ScriptAudioCommand, ScriptUiCommand};

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

        assert_eq!(
            runtime.sprite_animation_count(),
            5,
            "the authored player and four enemies should load looping clips"
        );
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
        let ui_commands = runtime.take_ui_commands();
        assert!(
            ui_commands.contains(&ScriptUiCommand::SetText {
                element_guid: "a1d999a0-d8d1-4a1e-a700-4c49ed030013"
                    .parse()
                    .expect("score UI GUID"),
                text: "SCORE 0".into(),
            })
        );
        assert!(
            ui_commands.contains(&ScriptUiCommand::SetText {
                element_guid: "a1d999a0-d8d1-4a1e-a700-4c49ed030014"
                    .parse()
                    .expect("lives UI GUID"),
                text: "LIVES 3".into(),
            })
        );
        let audio_commands = runtime.take_audio_commands();
        assert!(audio_commands.iter().any(|command| matches!(
            command,
            ScriptAudioCommand::PlayEffect { bus, .. } if bus == "sfx"
        )));
    }
    #[test]
    fn authored_projectile_collisions_score_and_update_the_hud() {
        let project = Project::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .expect("open Action Arena sample");
        let assets = AssetCatalog::open(project.root()).expect("open sample assets");
        let mut runtime = GameRuntime::from_scene_with_assets(
            project.default_scene(),
            RuntimeConfig::default(),
            &assets,
        )
        .expect("load authored scene and controllers");
        runtime.push_action(Duration::ZERO, ActionId(3), 1.0, true);
        runtime.advance_frame(Duration::from_millis(50));

        let projectile_guid = "529bb9ad-47b8-4992-874e-055c7fbf0015"
            .parse::<Guid>()
            .expect("parse projectile GUID");
        let enemy_guid = "529bb9ad-47b8-4992-874e-055c7fbf0011"
            .parse::<Guid>()
            .expect("parse enemy GUID");
        let projectile = runtime
            .entity_for_guid(projectile_guid)
            .expect("authored projectile exists");
        let enemy = runtime.entity_for_guid(enemy_guid).expect("enemy exists");
        let mut projectile_transform = runtime
            .world()
            .transform(projectile)
            .expect("projectile transform");
        let enemy_transform = runtime.world().transform(enemy).expect("enemy transform");
        projectile_transform.translation = [
            enemy_transform.translation[0],
            enemy_transform.translation[1] + 0.5,
        ];
        runtime
            .world_mut()
            .set_transform(projectile, projectile_transform)
            .expect("place launched projectile near the enemy");

        let frame = runtime.advance_frame(Duration::from_millis(50));
        assert!(
            frame.physics_events.iter().any(|event| {
                event.started
                    && ((event.object_a == projectile_guid && event.object_b == enemy_guid)
                        || (event.object_a == enemy_guid && event.object_b == projectile_guid))
            }),
            "Rapier contact should reach the authored projectile and enemy"
        );
        assert!(runtime.take_script_diagnostics().is_empty());
        assert!(
            runtime
                .take_ui_commands()
                .contains(&ScriptUiCommand::SetText {
                    element_guid: "a1d999a0-d8d1-4a1e-a700-4c49ed030013"
                        .parse()
                        .expect("score UI GUID"),
                    text: "SCORE 1".into(),
                }),
            "the collision should award one point and update the authored HUD"
        );
    }

    #[test]
    fn enemy_contact_consumes_a_life_and_updates_the_hud() {
        let project = Project::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .expect("open Action Arena sample");
        let assets = AssetCatalog::open(project.root()).expect("open sample assets");
        let mut runtime = GameRuntime::from_scene_with_assets(
            project.default_scene(),
            RuntimeConfig::default(),
            &assets,
        )
        .expect("load authored scene and controllers");
        let player_guid = "529bb9ad-47b8-4992-874e-055c7fbf0003"
            .parse::<Guid>()
            .expect("parse player GUID");
        let enemy_guid = "529bb9ad-47b8-4992-874e-055c7fbf0011"
            .parse::<Guid>()
            .expect("parse enemy GUID");
        let player = runtime.entity_for_guid(player_guid).expect("player exists");
        let enemy = runtime.entity_for_guid(enemy_guid).expect("enemy exists");
        let enemy_transform = runtime.world().transform(enemy).expect("enemy transform");
        runtime
            .world_mut()
            .set_transform(player, enemy_transform)
            .expect("place player at the enemy");

        let frame = runtime.advance_frame(Duration::from_millis(50));
        assert!(frame.physics_events.iter().any(|event| {
            event.started
                && ((event.object_a == player_guid && event.object_b == enemy_guid)
                    || (event.object_a == enemy_guid && event.object_b == player_guid))
        }));
        assert!(runtime.take_script_diagnostics().is_empty());
        assert!(
            runtime
                .take_ui_commands()
                .contains(&ScriptUiCommand::SetText {
                    element_guid: "a1d999a0-d8d1-4a1e-a700-4c49ed030014"
                        .parse()
                        .expect("lives UI GUID"),
                    text: "LIVES 2".into(),
                }),
            "player contact should consume one life and update the HUD"
        );
    }
}
