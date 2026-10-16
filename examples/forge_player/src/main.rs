use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use forge_assets::{AssetCatalog, AssetChangeKind, AssetKind};
use forge_core::Guid;
use forge_project::Project;
use forge_renderer::{GpuRenderer, RenderOutcome};
use forge_runtime::{GameRuntime, RuntimeConfig};
use forge_scene::{
    AuthoredCollider, AuthoredColliderShape, AuthoredRigidBody, AuthoredRigidBodyKind,
    AuthoredSprite, AuthoredTransform, SceneDocument, SceneObject,
};
use forge_tilemap::{TileCell, TileMapDocument};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

struct RunState {
    window: Arc<Window>,
    renderer: GpuRenderer,
}

struct PlayerApp {
    project_name: String,
    runtime: GameRuntime,
    assets: AssetCatalog,
    state: Option<RunState>,
    last_frame: Instant,
    next_asset_refresh: Instant,
}

impl PlayerApp {
    fn new(project_name: String, runtime: GameRuntime, assets: AssetCatalog) -> Self {
        let now = Instant::now();
        Self {
            project_name,
            runtime,
            assets,
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
            WindowEvent::RedrawRequested => {
                if refresh_assets {
                    if let Err(error) =
                        refresh_project_textures(&mut self.assets, &mut state.renderer)
                    {
                        eprintln!("project asset refresh failed: {error}");
                        event_loop.exit();
                        return;
                    }
                }
                let delta = now.saturating_duration_since(self.last_frame);
                self.last_frame = now;
                self.runtime.advance_frame(delta);
                match state
                    .renderer
                    .render_world(self.runtime.world_mut(), [0.035, 0.045, 0.065, 1.0])
                {
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
            transform,
            sprite: Some(sprite),
            camera: None,
            rigid_body,
            collider,
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

fn refresh_project_textures(
    assets: &mut AssetCatalog,
    renderer: &mut GpuRenderer,
) -> Result<(), Box<dyn Error>> {
    for change in assets.refresh()? {
        match change.kind {
            AssetChangeKind::Removed => {
                renderer.remove_texture(change.guid);
            }
            AssetChangeKind::Added | AssetChangeKind::Modified => {
                if assets
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
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let Some(project_path) = args.next() else {
        return Err("usage: forge-player <project directory or .forgeproj path>".into());
    };
    if args.next().is_some() {
        return Err("forge-player accepts one project path".into());
    }

    let project = Project::open(PathBuf::from(project_path))?;
    let project_name = project.document().name.clone();
    let project_root = project.root().to_path_buf();
    let assets = AssetCatalog::open(project_root.clone())?;
    let mut scene = project.default_scene().clone();
    if let Some(tilemap) = load_project_tilemap(&project_root)? {
        append_tilemap_to_scene(&mut scene, &tilemap, &assets)?;
    }
    let runtime = GameRuntime::from_scene_with_assets(&scene, RuntimeConfig::default(), &assets)?;

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut PlayerApp::new(project_name, runtime, assets))?;
    Ok(())
}
