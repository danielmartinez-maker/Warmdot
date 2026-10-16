use std::{collections::BTreeMap, env, io, path::PathBuf, process, time::Duration};

use forge_assets::{AssetCatalog, AssetKind};
use forge_core::Guid;
use forge_project::Project;
use forge_runtime::{GameRuntime, RuntimeConfig};
use forge_scene::{AuthoredScript, ScriptPropertyValue};

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
    let mut scene = project.default_scene().clone();
    let script = assets
        .entries()
        .find(|record| {
            record.kind == AssetKind::Script
                && record.relative_path == "Assets/PlayerController.luau"
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "sample Luau script is missing"))?;
    let player_guid = "529bb9ad-47b8-4992-874e-055c7fbf0003".parse::<Guid>()?;
    let player = scene
        .objects
        .iter_mut()
        .find(|object| object.object_guid == player_guid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "sample Player object is missing"))?;
    player.script = Some(AuthoredScript {
        asset: script.guid,
        properties: BTreeMap::from([
            ("speed".into(), ScriptPropertyValue::Number(1.5)),
            ("direction".into(), ScriptPropertyValue::Number(1.0)),
        ]),
    });
    if !scene.dependencies.contains(&script.guid) {
        scene.dependencies.push(script.guid);
    }
    let mut runtime = GameRuntime::from_scene_with_assets(
        &scene,
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
