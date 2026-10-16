use std::{env, io, path::PathBuf, process, time::Duration};

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
    let mut runtime = GameRuntime::from_scene(project.default_scene(), RuntimeConfig::default())?;

    let mut frames = 0_u64;
    for _ in 0..120 {
        let frame = runtime.advance_frame(Duration::from_nanos(16_666_667));
        frames += u64::from(frame.fixed_steps);
        for event in frame.physics_events {
            println!("physics event: {event:?}");
        }
    }

    let player_guid = "529bb9ad-47b8-4992-874e-055c7fbf0003".parse()?;
    let player = runtime
        .entity_for_guid(player_guid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "sample Player object was not loaded"))?;
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
