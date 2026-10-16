use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

use forge_core::{ActionId, Guid};
use forge_input::InputMap;
use forge_renderer::GpuRenderer;
use forge_runtime::{FixedUpdateContext, GameRuntime, RuntimeConfig};
use forge_scene::{AuthoredCamera, AuthoredSprite, AuthoredTransform, SceneDocument, SceneObject};
use forge_world::{RuntimeWorld, Sprite, Transform2D};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const BUILD_WORKER: ActionId = ActionId(1);
const MAX_WORKERS: usize = 256;
const WORKER_SPEED: f32 = 3.2;
const ARRIVAL_RADIUS: f32 = 0.8;
const HOME: [f32; 2] = [0.0, 0.0];
const RESOURCE_NODES: [[f32; 2]; 4] = [[-22.0, -11.0], [22.0, -11.0], [-22.0, 11.0], [22.0, 11.0]];

#[derive(Clone, Copy)]
struct WorkerState {
    guid: Guid,
    node: usize,
    carrying: bool,
}

struct GameState {
    window: Arc<Window>,
    renderer: GpuRenderer,
    runtime: GameRuntime,
    input: InputMap,
    delivered: Arc<AtomicU32>,
    population: Arc<AtomicU32>,
    shown_delivered: u32,
    shown_population: u32,
    last_frame: Instant,
}

impl GameState {
    fn new(window: Arc<Window>, renderer: GpuRenderer) -> Result<Self, Box<dyn Error>> {
        let (scene, workers) = make_scene();
        let mut runtime = GameRuntime::from_scene(&scene, RuntimeConfig::default())?;
        let delivered = Arc::new(AtomicU32::new(0));
        let population = Arc::new(AtomicU32::new(workers.len() as u32));
        runtime.add_fixed_system(make_colony_system(
            workers,
            delivered.clone(),
            population.clone(),
        ));

        let mut input = InputMap::new();
        input.bind_key(PhysicalKey::Code(KeyCode::Space), BUILD_WORKER, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyB), BUILD_WORKER, 1.0)?;

        Ok(Self {
            window,
            renderer,
            runtime,
            input,
            delivered,
            population,
            shown_delivered: u32::MAX,
            shown_population: u32::MAX,
            last_frame: Instant::now(),
        })
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let frame_delta = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        self.runtime.advance_frame(frame_delta);

        let delivered = self.delivered.load(Ordering::Relaxed);
        let population = self.population.load(Ordering::Relaxed);
        if delivered != self.shown_delivered || population != self.shown_population {
            self.shown_delivered = delivered;
            self.shown_population = population;
            self.window.set_title(&format!(
                "Forge Colony | Workers {population}/{MAX_WORKERS} | Delivered {delivered} | Space/B adds worker"
            ));
        }

        self.renderer
            .render_world(self.runtime.world_mut(), [0.018, 0.028, 0.050, 1.0])?;
        self.window.request_redraw();
        Ok(())
    }

    fn handle_input(&mut self, event: &WindowEvent) {
        let at = self.runtime.simulation_time();
        self.input.handle_window_event(event, at, &mut self.runtime);
    }
}

fn make_scene() -> (SceneDocument, Vec<WorkerState>) {
    let mut objects = Vec::new();
    objects.push(scene_sprite(
        "Colony",
        HOME,
        [2.5, 2.5],
        [0.18, 0.72, 0.95, 1.0],
        2,
    ));
    for (index, position) in RESOURCE_NODES.iter().copied().enumerate() {
        objects.push(scene_sprite(
            format!("Food Cache {}", index + 1),
            position,
            [2.0, 2.0],
            [0.96, 0.50, 0.18, 1.0],
            1,
        ));
    }
    objects.push(SceneObject {
        object_guid: Guid::new(),
        name: "Camera".into(),
        parent_guid: None,
        prefab_instance: None,
        transform: AuthoredTransform::default(),
        sprite: None,
        camera: Some(AuthoredCamera {
            zoom: [1.0, 1.0],
            pixels_per_world_unit: 20.0,
            priority: 1,
            active: true,
        }),
        rigid_body: None,
        collider: None,
        script: None,
    });

    let mut workers = Vec::new();
    for index in 0..64 {
        let guid = Guid::new();
        let angle = index as f32 * 2.399_963_1;
        let radius = 0.35 + (index % 5) as f32 * 0.12;
        let position = [angle.cos() * radius, angle.sin() * radius];
        objects.push(scene_sprite(
            format!("Worker {index}"),
            position,
            [0.72, 0.72],
            [0.30, 0.90, 0.66, 1.0],
            3,
        ));
        workers.push(WorkerState {
            guid,
            node: index % RESOURCE_NODES.len(),
            carrying: false,
        });
        objects
            .last_mut()
            .expect("worker sprite was just appended")
            .object_guid = guid;
    }

    (
        SceneDocument {
            schema_version: forge_scene::CURRENT_SCENE_SCHEMA,
            resource_guid: Guid::new(),
            ui_document: None,
            name: "Colony Simulation".into(),
            dependencies: Vec::new(),
            objects,
        },
        workers,
    )
}

fn scene_sprite(
    name: impl Into<String>,
    position: [f32; 2],
    size: [f32; 2],
    tint: [f32; 4],
    layer: i32,
) -> SceneObject {
    SceneObject {
        object_guid: Guid::new(),
        name: name.into(),
        parent_guid: None,
        prefab_instance: None,
        transform: AuthoredTransform {
            translation: position,
            rotation_radians: 0.0,
            scale: [1.0, 1.0],
        },
        sprite: Some(AuthoredSprite {
            asset: None,
            animation_clip: None,
            size,
            anchor: [0.5, 0.5],
            tint,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            layer,
            order: 0,
        }),
        camera: None,
        rigid_body: None,
        collider: None,
        script: None,
    }
}

fn make_colony_system(
    mut workers: Vec<WorkerState>,
    delivered: Arc<AtomicU32>,
    population: Arc<AtomicU32>,
) -> impl for<'a> FnMut(&mut RuntimeWorld, &FixedUpdateContext<'a>) + Send {
    move |world, context| {
        let spawn_count = context
            .input_events
            .iter()
            .filter(|event| event.action == BUILD_WORKER && event.pressed)
            .count();

        for _ in 0..spawn_count {
            if workers.len() >= MAX_WORKERS {
                break;
            }
            let state = WorkerState {
                guid: Guid::new(),
                node: workers.len() % RESOURCE_NODES.len(),
                carrying: false,
            };
            let sprite = Sprite {
                size: [0.72, 0.72],
                tint: [0.30, 0.90, 0.66, 1.0],
                layer: 3,
                ..Sprite::default()
            };
            if world
                .spawn(state.guid, Transform2D::default(), Some(sprite))
                .is_ok()
            {
                workers.push(state);
                population.fetch_add(1, Ordering::Relaxed);
            }
        }

        let step = WORKER_SPEED * context.delta.as_secs_f32();
        let mut deliveries = 0;
        for worker in &mut workers {
            let Some(entity) = world.entity_for_guid(worker.guid) else {
                continue;
            };
            let Some(mut transform) = world.transform(entity) else {
                continue;
            };
            let destination = if worker.carrying {
                HOME
            } else {
                RESOURCE_NODES[worker.node]
            };
            let dx = destination[0] - transform.translation[0];
            let dy = destination[1] - transform.translation[1];
            let distance = (dx * dx + dy * dy).sqrt();
            if distance <= ARRIVAL_RADIUS {
                if worker.carrying {
                    worker.carrying = false;
                    worker.node = (worker.node + 1) % RESOURCE_NODES.len();
                    deliveries += 1;
                } else {
                    worker.carrying = true;
                }
            } else if distance > 0.0 {
                let movement = step.min(distance);
                transform.translation[0] += dx / distance * movement;
                transform.translation[1] += dy / distance * movement;
                let _ = world.set_transform(entity, transform);
            }

            let tint = if worker.carrying {
                [1.0, 0.80, 0.24, 1.0]
            } else {
                [0.30, 0.90, 0.66, 1.0]
            };
            let _ = world.set_sprite(
                entity,
                Sprite {
                    size: [0.72, 0.72],
                    tint,
                    layer: 3,
                    ..Sprite::default()
                },
            );
        }
        if deliveries > 0 {
            delivered.fetch_add(deliveries, Ordering::Relaxed);
        }
    }
}

struct App {
    state: Option<GameState>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Forge Colony | Space/B adds worker")
                        .with_inner_size(PhysicalSize::new(1280, 720)),
                )
                .expect("could not create colony-simulation window"),
        );
        match GpuRenderer::new(event_loop.owned_display_handle(), window.clone()).and_then(
            |renderer| {
                GameState::new(window.clone(), renderer)
                    .map_err(|error| forge_renderer::RendererError::Surface(error.to_string()))
            },
        ) {
            Ok(state) => {
                window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                eprintln!("colony simulation could not start: {error}");
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
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.renderer.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                if let Err(error) = state.draw() {
                    eprintln!("colony simulation frame failed: {error}");
                    event_loop.exit();
                }
            }
            _ => state.handle_input(&event),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::default())?;
    Ok(())
}

impl Default for App {
    fn default() -> Self {
        Self { state: None }
    }
}
