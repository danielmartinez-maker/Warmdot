use std::{
    env,
    error::Error,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use forge_assets::{AssetCatalog, AssetChangeKind, AssetKind};
use forge_project::Project;
use forge_renderer::{GpuRenderer, RenderOutcome};
use forge_runtime::{GameRuntime, RuntimeConfig};
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
    let runtime = GameRuntime::from_scene(project.default_scene(), RuntimeConfig::default())?;
    let assets = AssetCatalog::open(project_root)?;

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut PlayerApp::new(project_name, runtime, assets))?;
    Ok(())
}
