use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use forge_assets::{AssetCatalog, AssetChangeKind, AssetKind};
use forge_audio::{AudioBus, AudioEngine};
use forge_core::Guid;
use forge_project::{PROJECT_FILE_NAME, Project, SceneCatalog};
use forge_renderer::{GpuRenderer, RenderOutcome, SpriteInstance, TexturedSprite};
use forge_runtime::{GameRuntime, RuntimeConfig, ScriptAudioCommand};
use forge_scene::{
    AuthoredCollider, AuthoredColliderShape, AuthoredRigidBody, AuthoredRigidBodyKind,
    AuthoredSprite, AuthoredTransform, SceneDocument, SceneObject,
};
use forge_tilemap::{TileCell, TileMapDocument};
use forge_ui::{UiButtonEvent, UiDrawCommand, UiInputResult, UiKey, UiRuntime};
use serde::Serialize;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

struct RunState {
    window: Arc<Window>,
    renderer: GpuRenderer,
}

struct PlayerApp {
    project_name: String,
    project_root: PathBuf,
    scene: SceneDocument,
    runtime: GameRuntime,
    ui: UiRuntime,
    cursor_position: [f32; 2],
    assets: AssetCatalog,
    audio: Option<AudioEngine>,
    scene_catalog: Option<Result<SceneCatalog, String>>,
    state: Option<RunState>,
    last_frame: Instant,
    next_asset_refresh: Instant,
}

impl PlayerApp {
    fn new(
        project_name: String,
        project_root: PathBuf,
        scene: SceneDocument,
        runtime: GameRuntime,
        ui: UiRuntime,
        assets: AssetCatalog,
        audio: Option<AudioEngine>,
    ) -> Self {
        let now = Instant::now();
        Self {
            project_name,
            project_root,
            scene,
            runtime,
            ui,
            cursor_position: [0.0, 0.0],
            assets,
            audio,
            scene_catalog: None,
            state: None,
            last_frame: now,
            next_asset_refresh: now + Duration::from_millis(500),
        }
    }
}

impl ApplicationHandler for PlayerApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(format!("Forge 2D Player - {}", self.project_name))
                        .with_inner_size(PhysicalSize::new(1280, 720)),
                )
                .expect("could not create Forge player window"),
        );
        match GpuRenderer::new(event_loop.owned_display_handle(), window.clone()) {
            Ok(mut renderer) => {
                if let Err(error) = load_project_textures(&self.assets, &mut renderer) {
                    eprintln!("could not load project textures: {error}");
                    event_loop.exit();
                    return;
                }
                self.last_frame = Instant::now();
                self.state = Some(RunState { window, renderer });
            }
            Err(error) => {
                eprintln!("renderer initialization failed: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let now = Instant::now();
        let refresh_assets = now >= self.next_asset_refresh;
        if refresh_assets {
            self.next_asset_refresh = now + Duration::from_millis(500);
        }

        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.renderer.resize(size.width, size.height),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = [position.x as f32, position.y as f32];
                let size = state.window.inner_size();
                match self
                    .ui
                    .route_pointer_move(self.cursor_position, size.width, size.height)
                {
                    Ok(result) => submit_ui_event(&mut self.runtime, result),
                    Err(error) => eprintln!("game UI pointer move failed: {error}"),
                }
                state.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.ui.clear_pointer();
                state.window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                let size = state.window.inner_size();
                let result = if button_state == winit::event::ElementState::Pressed {
                    self.ui
                        .route_pointer_down(self.cursor_position, size.width, size.height)
                } else {
                    self.ui
                        .route_pointer_up(self.cursor_position, size.width, size.height)
                };
                match result {
                    Ok(result) => submit_ui_event(&mut self.runtime, result),
                    Err(error) => eprintln!("game UI pointer input failed: {error}"),
                }
                state.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed && !event.repeat =>
            {
                let key = match event.physical_key {
                    PhysicalKey::Code(KeyCode::Tab | KeyCode::ArrowDown | KeyCode::ArrowRight) => {
                        Some(UiKey::Next)
                    }
                    PhysicalKey::Code(KeyCode::ArrowUp | KeyCode::ArrowLeft) => {
                        Some(UiKey::Previous)
                    }
                    PhysicalKey::Code(KeyCode::Enter | KeyCode::Space) => Some(UiKey::Activate),
                    _ => None,
                };
                if let Some(key) = key {
                    let size = state.window.inner_size();
                    match self.ui.route_key(key, size.width, size.height) {
                        Ok(result) => submit_ui_event(&mut self.runtime, result),
                        Err(error) => eprintln!("game UI keyboard input failed: {error}"),
                    }
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if refresh_assets {
                    if let Err(error) = refresh_project_assets(
                        &mut self.assets,
                        &mut state.renderer,
                        &mut self.runtime,
                        &mut self.scene,
                        &mut self.ui,
                    ) {
                        eprintln!("project asset refresh failed: {error}");
                        event_loop.exit();
                        return;
                    }
                }
                let delta = now.saturating_duration_since(self.last_frame);
                self.last_frame = now;
                self.runtime.advance_frame(delta);
                process_audio_commands(
                    self.audio.as_mut(),
                    &self.assets,
                    self.runtime.take_audio_commands(),
                );
                let scene_requests = self.runtime.take_scene_transition_requests();
                process_scene_transition_requests(
                    &self.project_root,
                    &self.assets,
                    &mut self.runtime,
                    &mut self.scene,
                    &mut self.ui,
                    &mut self.scene_catalog,
                    scene_requests,
                );
                for diagnostic in self.runtime.take_script_diagnostics() {
                    eprintln!(
                        "Luau {} at tick {}: {}",
                        diagnostic.source_path, diagnostic.tick, diagnostic.message
                    );
                }
                let window_size = state.window.inner_size();
                let ui_sprites = match self.ui.draw_commands(window_size.width, window_size.height)
                {
                    Ok(commands) => ui_commands_to_sprites(commands),
                    Err(error) => {
                        eprintln!("game UI draw extraction failed: {error}");
                        Vec::new()
                    }
                };
                match state.renderer.render_world_with_ui(
                    self.runtime.world_mut(),
                    &ui_sprites,
                    [0.035, 0.045, 0.065, 1.0],
                ) {
                    Ok(RenderOutcome::Presented | RenderOutcome::Skipped) => {
                        state.window.request_redraw();
                    }
                    Err(error) => {
                        eprintln!("project rendering failed: {error}");
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

fn process_audio_commands(
    audio: Option<&mut AudioEngine>,
    assets: &AssetCatalog,
    commands: Vec<ScriptAudioCommand>,
) {
    let Some(audio) = audio else {
        return;
    };
    for command in commands {
        let result = match command {
            ScriptAudioCommand::PlayEffect { asset_guid, bus } => {
                AudioBus::parse(&bus).and_then(|bus| audio.play_effect(assets, asset_guid, bus))
            }
            ScriptAudioCommand::PlayMusic {
                asset_guid,
                looping,
            } => audio.play_music(assets, asset_guid, looping),
            ScriptAudioCommand::SetBusVolume { bus, decibels } => {
                AudioBus::parse(&bus).and_then(|bus| audio.set_bus_volume(bus, decibels as f32))
            }
        };
        if let Err(error) = result {
            eprintln!("audio command failed: {error}");
        }
    }
}

fn process_scene_transition_requests(
    project_root: &Path,
    assets: &AssetCatalog,
    runtime: &mut GameRuntime,
    current_scene: &mut SceneDocument,
    ui: &mut UiRuntime,
    scene_catalog: &mut Option<Result<SceneCatalog, String>>,
    requests: Vec<Guid>,
) {
    if requests.is_empty() {
        return;
    }
    if scene_catalog.is_none() {
        let catalog = SceneCatalog::open(project_root).map_err(|error| error.to_string());
        if let Err(error) = &catalog {
            eprintln!("scene catalog is unavailable: {error}");
        }
        *scene_catalog = Some(catalog);
    }
    let Some(Ok(catalog)) = scene_catalog.as_ref() else {
        return;
    };
    for scene_guid in requests {
        let scene = match catalog.load_scene(scene_guid) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("scene transition to {scene_guid} was rejected: {error}");
                continue;
            }
        };
        if let Err(error) = validate_scene_dependencies(&scene, assets) {
            eprintln!("scene transition to {scene_guid} was rejected: {error}");
            continue;
        }
        let replacement_ui = match load_scene_ui(&scene, assets) {
            Ok(ui) => ui,
            Err(error) => {
                eprintln!("scene transition to {scene_guid} has invalid UI: {error}");
                continue;
            }
        };
        match GameRuntime::from_scene_with_assets(&scene, RuntimeConfig::default(), assets) {
            Ok(replacement) => {
                eprintln!("entered scene {} ({scene_guid})", scene.name);
                *runtime = replacement;
                *current_scene = scene;
                *ui = replacement_ui;
                return;
            }
            Err(error) => {
                eprintln!("scene transition to {scene_guid} could not prepare: {error}");
            }
        }
    }
}

fn validate_scene_dependencies(
    scene: &SceneDocument,
    assets: &AssetCatalog,
) -> Result<(), Box<dyn Error>> {
    for guid in scene.dependencies.iter().copied() {
        let record = assets.get(guid).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("scene dependency {guid} is not in the project asset catalog"),
            )
        })?;
        match record.kind {
            AssetKind::Png => {
                assets.load_png(guid)?;
            }
            AssetKind::SpriteSheet => {
                assets.load_sprite_sheet(guid)?;
            }
            AssetKind::AnimationClip => {
                assets.load_animation_clip(guid)?;
            }
            AssetKind::Script => {
                assets.load_script(guid)?;
            }
            AssetKind::Prefab => {
                assets.load_prefab(guid)?;
            }
            AssetKind::Audio => {
                assets.load_audio(guid)?;
            }
            AssetKind::Ui => {
                assets.load_ui_source(guid)?;
            }
        }
    }
    Ok(())
}

fn load_scene_ui(
    scene: &SceneDocument,
    assets: &AssetCatalog,
) -> Result<UiRuntime, Box<dyn Error>> {
    let Some(guid) = scene.ui_document else {
        return Ok(UiRuntime::empty());
    };
    let source = assets.load_ui_source(guid)?;
    let document = forge_ui::UiDocument::from_json(&source.source)?;
    let ui = UiRuntime::new(document)?;
    ui.validate_assets(assets)?;
    Ok(ui)
}

fn ui_commands_to_sprites(commands: Vec<UiDrawCommand>) -> Vec<TexturedSprite> {
    commands
        .into_iter()
        .map(|command| TexturedSprite {
            instance: SpriteInstance {
                position: [
                    command.rect.x + command.rect.width * 0.5,
                    command.rect.y + command.rect.height * 0.5,
                ],
                size: [command.rect.width, command.rect.height],
                scale: [1.0, 1.0],
                rotation_radians: 0.0,
                anchor: [0.5, 0.5],
                tint: command.tint,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
            },
            texture_asset: command.texture_asset,
        })
        .collect()
}

fn submit_ui_event(runtime: &mut GameRuntime, result: UiInputResult) {
    if let Some(UiButtonEvent { source_id, name }) = result.event {
        if !runtime.emit_ui_event(name.clone(), source_id) {
            eprintln!("game UI event {name:?} was rejected by the script event queue");
        }
    }
}

fn load_project_tilemap(project_root: &Path) -> Result<Option<TileMapDocument>, Box<dyn Error>> {
    let project_root = project_root.canonicalize()?;
    let assets_root = project_root.join("Assets").canonicalize()?;
    if !assets_root.starts_with(&project_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "project Assets folder resolves outside the project",
        )
        .into());
    }
    let path = assets_root.join("Default.forgetilemap.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tilemap file must not be a symbolic link",
            )
            .into());
        }
        Ok(_) => {
            let canonical_path = path.canonicalize()?;
            if !canonical_path.starts_with(&assets_root) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "tilemap file resolves outside the Assets folder",
                )
                .into());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::metadata(&path)?;
    const MAX_TILEMAP_BYTES: u64 = 64 * 1024 * 1024;
    if metadata.len() > MAX_TILEMAP_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tilemap resource exceeds the player import limit",
        )
        .into());
    }
    let source = fs::read_to_string(path)?;
    Ok(Some(TileMapDocument::from_json(&source)?))
}

fn append_tilemap_to_scene(
    scene: &mut SceneDocument,
    document: &TileMapDocument,
    assets: &AssetCatalog,
) -> Result<(), Box<dyn Error>> {
    let map = document.instantiate()?;
    let texture = assets.load_png(document.tileset_guid)?;
    let texture_tile_size = document.texture_tile_size;
    let columns = texture.width / texture_tile_size[0];
    let rows = texture.height / texture_tile_size[1];
    let tile_count = columns
        .checked_mul(rows)
        .filter(|count| *count > 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "tilemap tileset has no complete tiles at its configured pixel size",
            )
        })?;
    if !scene.dependencies.contains(&document.tileset_guid) {
        scene.dependencies.push(document.tileset_guid);
    }

    for (coordinate, cell) in map.iter() {
        let uv_rect = tile_uv_rect(
            cell,
            columns,
            tile_count,
            texture.width,
            texture.height,
            texture_tile_size,
        )?;
        let object_guid = Guid::new();
        let rotated_size = if cell.rotation_quarters % 2 == 0 {
            document.tile_size
        } else {
            [document.tile_size[1], document.tile_size[0]]
        };
        let transform = AuthoredTransform {
            translation: [
                (coordinate.x as f32 + 0.5) * document.tile_size[0],
                (coordinate.y as f32 + 0.5) * document.tile_size[1],
            ],
            rotation_radians: f32::from(cell.rotation_quarters) * std::f32::consts::FRAC_PI_2,
            scale: [
                if cell.flip_x { -1.0 } else { 1.0 },
                if cell.flip_y { -1.0 } else { 1.0 },
            ],
        };
        let sprite = AuthoredSprite {
            asset: Some(document.tileset_guid),
            animation_clip: None,
            size: rotated_size,
            anchor: [0.5, 0.5],
            tint: [1.0; 4],
            uv_rect,
            layer: i32::MIN,
            order: 0,
        };
        let (rigid_body, collider) = if cell.solid {
            (
                Some(AuthoredRigidBody {
                    kind: AuthoredRigidBodyKind::Fixed,
                    ..AuthoredRigidBody::default()
                }),
                Some(AuthoredCollider {
                    shape: AuthoredColliderShape::Box {
                        half_extents: [rotated_size[0] * 0.5, rotated_size[1] * 0.5],
                    },
                    offset: [0.0, 0.0],
                    rotation_radians: 0.0,
                    density: 1.0,
                    friction: 0.5,
                    restitution: 0.0,
                    sensor: false,
                }),
            )
        } else {
            (None, None)
        };
        scene.objects.push(SceneObject {
            object_guid,
            name: format!("Tile ({}, {})", coordinate.x, coordinate.y),
            parent_guid: None,
            prefab_instance: None,
            transform,
            sprite: Some(sprite),
            camera: None,
            rigid_body,
            collider,
            script: None,
        });
    }
    Ok(())
}

fn tile_uv_rect(
    cell: TileCell,
    columns: u32,
    tile_count: u32,
    texture_width: u32,
    texture_height: u32,
    texture_tile_size: [u32; 2],
) -> Result<[f32; 4], Box<dyn Error>> {
    if cell.tile_index >= tile_count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "tile index {} is outside the configured tileset grid",
                cell.tile_index
            ),
        )
        .into());
    }
    let column = cell.tile_index % columns;
    let row = cell.tile_index / columns;
    Ok([
        (column * texture_tile_size[0]) as f32 / texture_width as f32,
        (row * texture_tile_size[1]) as f32 / texture_height as f32,
        texture_tile_size[0] as f32 / texture_width as f32,
        texture_tile_size[1] as f32 / texture_height as f32,
    ])
}

fn copy_tree_without_symlinks(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            copy_tree_without_symlinks(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "export source files must be regular files, not symbolic links",
        )
        .into());
    }
    let canonical = source.canonicalize()?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(canonical, destination)?;
    Ok(())
}

fn export_project(project_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let manifest_source = if project_path.is_dir() {
        project_path.join(PROJECT_FILE_NAME)
    } else {
        project_path.clone()
    };
    let manifest_source = manifest_source.canonicalize()?;
    let project = Project::open(manifest_source.clone())?;
    let project_root = project.root().to_path_buf();
    let output_path = if output_path.is_absolute() {
        output_path
    } else {
        env::current_dir()?.join(output_path)
    };
    fs::create_dir_all(&output_path)?;
    let output_root = output_path.canonicalize()?;
    if output_root.starts_with(&project_root) || project_root.starts_with(&output_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export folder and source project must not contain one another",
        )
        .into());
    }

    fs::write(output_root.join("LICENSE"), include_str!("../../../LICENSE"))?;

    let game_root = output_root.join("Game");
    fs::create_dir_all(&game_root)?;
    copy_regular_file(&manifest_source, &game_root.join(PROJECT_FILE_NAME))?;

    let scene_relative = Path::new(&project.document().default_scene);
    let scene_source = project_root.join(scene_relative);
    let scene_source = scene_source.canonicalize()?;
    if !scene_source.starts_with(&project_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "default scene resolves outside the source project",
        )
        .into());
    }
    let scene_target = game_root.join(scene_relative);
    copy_regular_file(&scene_source, &scene_target)?;

    let scenes_source = project_root.join("Scenes");
    if scenes_source.exists() {
        let canonical_scenes = scenes_source.canonicalize()?;
        if !canonical_scenes.starts_with(&project_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Scenes folder resolves outside the source project",
            )
            .into());
        }
        copy_tree_without_symlinks(&canonical_scenes, &game_root.join("Scenes"))?;
    }

    let assets_source = project_root.join("Assets");
    if assets_source.exists() {
        let canonical_assets = assets_source.canonicalize()?;
        if !canonical_assets.starts_with(&project_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Assets folder resolves outside the source project",
            )
            .into());
        }
        copy_tree_without_symlinks(&canonical_assets, &game_root.join("Assets"))?;
    }
    let metadata_root = project_root.join(".forge");
    if metadata_root.exists() {
        let canonical_metadata_root = metadata_root.canonicalize()?;
        if !canonical_metadata_root.starts_with(&project_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "project metadata folder resolves outside the source project",
            )
            .into());
        }
        let asset_index = canonical_metadata_root.join("asset-index.json");
        if asset_index.exists() {
            copy_regular_file(
                &asset_index,
                &game_root.join(".forge").join("asset-index.json"),
            )?;
        }
    }

    let executable_name = if cfg!(windows) {
        "Forge2DPlayer.exe"
    } else {
        "Forge2DPlayer"
    };
    fs::copy(env::current_exe()?, output_root.join(executable_name))?;
    let launcher = if cfg!(windows) {
        "@echo off\r\n\"%~dp0Forge2DPlayer.exe\" \"%~dp0Game\\project.forgeproj\" %*\r\n"
    } else {
        "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\n\"$DIR/Forge2DPlayer\" \"$DIR/Game/project.forgeproj\" \"$@\"\n"
    };
    let launcher_path = output_root.join(if cfg!(windows) {
        "run-game.bat"
    } else {
        "run-game.sh"
    });
    fs::write(&launcher_path, launcher)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&launcher_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&launcher_path, permissions)?;
    }
    Ok(())
}

fn load_project_textures(
    assets: &AssetCatalog,
    renderer: &mut GpuRenderer,
) -> Result<(), Box<dyn Error>> {
    let texture_guids = assets
        .entries()
        .filter(|record| record.kind == AssetKind::Png)
        .map(|record| record.guid)
        .collect::<Vec<_>>();
    for guid in texture_guids {
        let image = assets.load_png(guid)?;
        renderer.register_texture(image.guid, image.width, image.height, &image.rgba8)?;
    }
    Ok(())
}

fn refresh_project_assets(
    assets: &mut AssetCatalog,
    renderer: &mut GpuRenderer,
    runtime: &mut GameRuntime,
    scene: &mut SceneDocument,
    ui: &mut UiRuntime,
) -> Result<(), Box<dyn Error>> {
    for change in assets.refresh()? {
        match change.kind {
            AssetChangeKind::Removed => {
                renderer.remove_texture(change.guid);
                runtime.report_script_asset_issue(
                    change.guid,
                    change.relative_path,
                    "script asset was removed; the active last-good version remains loaded",
                );
            }
            AssetChangeKind::Added | AssetChangeKind::Modified => {
                if scene.ui_document == Some(change.guid)
                    && assets
                        .get(change.guid)
                        .is_some_and(|record| record.kind == AssetKind::Ui)
                {
                    match load_scene_ui(scene, assets) {
                        Ok(replacement) => *ui = replacement,
                        Err(error) => eprintln!("game UI reload was rejected: {error}"),
                    }
                } else if assets
                    .get(change.guid)
                    .is_some_and(|record| record.kind == AssetKind::Png)
                {
                    let image = assets.load_png(change.guid)?;
                    renderer.register_texture(
                        image.guid,
                        image.width,
                        image.height,
                        &image.rgba8,
                    )?;
                } else if assets
                    .get(change.guid)
                    .is_some_and(|record| record.kind == AssetKind::Script)
                {
                    match assets.load_script(change.guid) {
                        Ok(script) => runtime.reload_script_asset(&script),
                        Err(error) => runtime.report_script_asset_issue(
                            change.guid,
                            change.relative_path,
                            error.to_string(),
                        ),
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum ValidationSeverity {
    Error,
}

#[derive(Debug, Serialize, Eq, Ord, PartialEq, PartialOrd)]
struct ValidationDiagnostic {
    severity: ValidationSeverity,
    code: String,
    path: String,
    message: String,
}

#[derive(Serialize)]
struct ValidationReport {
    schema_version: u32,
    project_name: Option<String>,
    valid: bool,
    scene_count: usize,
    asset_count: usize,
    diagnostics: Vec<ValidationDiagnostic>,
}

fn add_validation_diagnostic(
    diagnostics: &mut Vec<ValidationDiagnostic>,
    code: &str,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        code: code.into(),
        path: path.into(),
        message: message.into(),
    });
}

fn validate_project(path: &Path) -> ValidationReport {
    let mut report = ValidationReport {
        schema_version: 1,
        project_name: None,
        valid: false,
        scene_count: 0,
        asset_count: 0,
        diagnostics: Vec::new(),
    };
    let project = match Project::open(path) {
        Ok(project) => project,
        Err(error) => {
            add_validation_diagnostic(
                &mut report.diagnostics,
                "project.invalid",
                ".forgeproj",
                error.to_string(),
            );
            return finish_validation_report(report);
        }
    };
    report.project_name = Some(project.document().name.clone());
    let root = project.root().to_path_buf();
    let assets = match AssetCatalog::open_read_only(&root) {
        Ok(assets) => {
            report.asset_count = assets.entries().count();
            Some(assets)
        }
        Err(error) => {
            add_validation_diagnostic(
                &mut report.diagnostics,
                "assets.catalog_invalid",
                ".forge/asset-index.json",
                error.to_string(),
            );
            None
        }
    };

    let mut scenes = BTreeMap::new();
    let default_scene = project.default_scene().clone();
    scenes.insert(default_scene.resource_guid, default_scene);
    match SceneCatalog::open(&root) {
        Ok(catalog) => {
            let scene_guids = catalog.scene_guids().collect::<Vec<_>>();
            report.scene_count = scene_guids.len();
            if !scene_guids.contains(&project.document().default_scene_guid) {
                add_validation_diagnostic(
                    &mut report.diagnostics,
                    "project.default_scene_unindexed",
                    "Scenes",
                    "default scene is not indexed below the Scenes folder",
                );
            }
            for guid in scene_guids {
                match catalog.load_scene(guid) {
                    Ok(scene) => {
                        scenes.insert(guid, scene);
                    }
                    Err(error) => add_validation_diagnostic(
                        &mut report.diagnostics,
                        "scene.invalid",
                        format!("Scenes/{guid}"),
                        error.to_string(),
                    ),
                }
            }
        }
        Err(error) => add_validation_diagnostic(
            &mut report.diagnostics,
            "scenes.catalog_invalid",
            "Scenes",
            error.to_string(),
        ),
    }

    if let Some(assets) = assets.as_ref() {
        for record in assets.entries() {
            let result = match record.kind {
                AssetKind::Png => assets.load_png(record.guid).map(|_| ()),
                AssetKind::SpriteSheet => assets.load_sprite_sheet(record.guid).map(|_| ()),
                AssetKind::AnimationClip => assets.load_animation_clip(record.guid).map(|_| ()),
                AssetKind::Script => assets.load_script(record.guid).and_then(|script| {
                    forge_scripting::validate_source(&script.source)
                        .map_err(forge_assets::AssetError::Invalid)
                }),
                AssetKind::Prefab => assets.load_prefab(record.guid).map(|_| ()),
                AssetKind::Audio => assets.load_audio(record.guid).map(|_| ()),
                AssetKind::Ui => assets.load_ui_source(record.guid).and_then(|source| {
                    let document = forge_ui::UiDocument::from_json(&source.source)
                        .map_err(|error| forge_assets::AssetError::Invalid(error.to_string()))?;
                    let ui = UiRuntime::new(document)
                        .map_err(|error| forge_assets::AssetError::Invalid(error.to_string()))?;
                    ui.validate_assets(assets)
                        .map_err(|error| forge_assets::AssetError::Invalid(error.to_string()))?;
                    Ok(())
                }),
            };
            if let Err(error) = result {
                add_validation_diagnostic(
                    &mut report.diagnostics,
                    "asset.invalid",
                    record.relative_path.clone(),
                    error.to_string(),
                );
            }
        }
        for scene in scenes.values() {
            validate_scene_asset_references(scene, assets, &mut report.diagnostics);
        }
        validate_project_tilemap(&root, assets, &mut report.diagnostics);
    }

    finish_validation_report(report)
}

fn require_asset_kind(
    assets: &AssetCatalog,
    guid: Guid,
    expected: AssetKind,
    path: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match assets.get(guid) {
        None => add_validation_diagnostic(
            diagnostics,
            "asset.missing_reference",
            path,
            format!("asset {guid} is not in the project catalog"),
        ),
        Some(record) if record.kind != expected => add_validation_diagnostic(
            diagnostics,
            "asset.wrong_kind",
            path,
            format!("asset {guid} is {:?}; expected {:?}", record.kind, expected),
        ),
        Some(_) => {}
    }
}

fn validate_scene_asset_references(
    scene: &SceneDocument,
    assets: &AssetCatalog,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let path = format!("Scenes/{}", scene.resource_guid);
    for dependency in &scene.dependencies {
        if assets.get(*dependency).is_none() {
            add_validation_diagnostic(
                diagnostics,
                "scene.missing_dependency",
                &path,
                format!("scene dependency {dependency} is not in the project asset catalog"),
            );
        }
    }
    if let Some(guid) = scene.ui_document {
        require_asset_kind(assets, guid, AssetKind::Ui, &path, diagnostics);
    }
    for object in &scene.objects {
        let object_path = format!("{path}#{}", object.object_guid);
        if let Some(sprite) = object.sprite {
            if let Some(guid) = sprite.asset {
                require_asset_kind(assets, guid, AssetKind::Png, &object_path, diagnostics);
            }
            if let Some(guid) = sprite.animation_clip {
                require_asset_kind(
                    assets,
                    guid,
                    AssetKind::AnimationClip,
                    &object_path,
                    diagnostics,
                );
            }
        }
        if let Some(instance) = &object.prefab_instance {
            require_asset_kind(
                assets,
                instance.prefab_asset,
                AssetKind::Prefab,
                &object_path,
                diagnostics,
            );
        }
        if let Some(script) = &object.script {
            require_asset_kind(
                assets,
                script.asset,
                AssetKind::Script,
                &object_path,
                diagnostics,
            );
        }
    }
}

fn validate_project_tilemap(
    root: &Path,
    assets: &AssetCatalog,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let relative_path = "Assets/Default.forgetilemap.json";
    let path = root.join(relative_path);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            add_validation_diagnostic(
                diagnostics,
                "tilemap.read_failed",
                relative_path,
                error.to_string(),
            );
            return;
        }
        Ok(_) => {}
    }
    let tilemap = match load_project_tilemap(root) {
        Ok(Some(tilemap)) => tilemap,
        Ok(None) => return,
        Err(error) => {
            add_validation_diagnostic(
                diagnostics,
                "tilemap.invalid",
                relative_path,
                error.to_string(),
            );
            return;
        }
    };
    let texture = match assets.load_png(tilemap.tileset_guid) {
        Ok(texture) => texture,
        Err(error) => {
            add_validation_diagnostic(
                diagnostics,
                "tilemap.invalid_tileset",
                relative_path,
                error.to_string(),
            );
            return;
        }
    };
    let columns = texture.width / tilemap.texture_tile_size[0];
    let rows = texture.height / tilemap.texture_tile_size[1];
    let tile_count = columns.checked_mul(rows).filter(|count| *count > 0);
    let Some(tile_count) = tile_count else {
        add_validation_diagnostic(
            diagnostics,
            "tilemap.invalid_tileset_layout",
            relative_path,
            "tileset has no complete tiles at the configured pixel size",
        );
        return;
    };
    let invalid = tilemap
        .chunks
        .iter()
        .flat_map(|chunk| {
            chunk.entries.iter().map(move |entry| {
                (
                    chunk.coordinate.x,
                    chunk.coordinate.y,
                    entry.local_index,
                    entry.cell.tile_index,
                )
            })
        })
        .filter(|entry| entry.3 >= tile_count)
        .min();
    if let Some((chunk_x, chunk_y, local_index, tile_index)) = invalid {
        add_validation_diagnostic(
            diagnostics,
            "tilemap.tile_index_out_of_range",
            relative_path,
            format!(
                "tile index {tile_index} at chunk ({chunk_x}, {chunk_y}) cell {local_index} exceeds tileset tile count {tile_count}"
            ),
        );
    }
}

fn finish_validation_report(mut report: ValidationReport) -> ValidationReport {
    report.diagnostics.sort();
    report.diagnostics.dedup();
    report.valid = report.diagnostics.is_empty();
    report
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|argument| argument == "--validate")
    {
        if args.len() != 2 {
            return Err(
                "usage: forge-player --validate <project directory or .forgeproj path>".into(),
            );
        }
        let report = validate_project(Path::new(&args[1]));
        let valid = report.valid;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !valid {
            return Err("project validation failed".into());
        }
        return Ok(());
    }
    if args.first().is_some_and(|argument| argument == "--export") {
        if args.len() != 3 {
            return Err(
                "usage: forge-player --export <project directory or .forgeproj path> <output folder>"
                    .into(),
            );
        }
        export_project(PathBuf::from(&args[1]), PathBuf::from(&args[2]))?;
        println!("Standalone project exported successfully.");
        return Ok(());
    }
    if args.len() != 1 {
        return Err("usage: forge-player <project directory or .forgeproj path>".into());
    }

    let project = Project::open(PathBuf::from(&args[0]))?;
    let project_name = project.document().name.clone();
    let project_root = project.root().to_path_buf();
    let assets = AssetCatalog::open(project_root.clone())?;
    let mut scene = project.default_scene().clone();
    if let Some(tilemap) = load_project_tilemap(&project_root)? {
        append_tilemap_to_scene(&mut scene, &tilemap, &assets)?;
    }
    let ui = load_scene_ui(&scene, &assets)?;
    let runtime = GameRuntime::from_scene_with_assets(&scene, RuntimeConfig::default(), &assets)?;
    let audio = match AudioEngine::new() {
        Ok(audio) => Some(audio),
        Err(error) => {
            eprintln!("audio is unavailable; continuing without game audio: {error}");
            None
        }
    };

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut PlayerApp::new(
        project_name,
        project_root,
        scene,
        runtime,
        ui,
        assets,
        audio,
    ))?;
    Ok(())
}
