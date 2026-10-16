use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
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
    event::{ElementState, MouseButton, WindowEvent},
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
const GRID_WIDTH: i32 = 30;
const GRID_HEIGHT: i32 = 16;
const GRID_ORIGIN: [f32; 2] = [-30.0, -16.0];
const GRID_CELL_SIZE: f32 = 2.0;
const CAMERA_PIXELS_PER_UNIT: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GridCell {
    x: i32,
    y: i32,
}

struct NavigationGrid {
    blocked: BTreeMap<GridCell, Guid>,
    revision: u64,
}

impl NavigationGrid {
    fn new() -> Self {
        Self {
            blocked: BTreeMap::new(),
            revision: 0,
        }
    }

    fn cell_at(&self, position: [f32; 2]) -> Option<GridCell> {
        if position.iter().any(|coordinate| !coordinate.is_finite()) {
            return None;
        }
        let cell = GridCell {
            x: ((position[0] - GRID_ORIGIN[0]) / GRID_CELL_SIZE).floor() as i32,
            y: ((position[1] - GRID_ORIGIN[1]) / GRID_CELL_SIZE).floor() as i32,
        };
        self.in_bounds(cell).then_some(cell)
    }

    fn in_bounds(&self, cell: GridCell) -> bool {
        (0..GRID_WIDTH).contains(&cell.x) && (0..GRID_HEIGHT).contains(&cell.y)
    }

    fn is_walkable(&self, cell: GridCell) -> bool {
        self.in_bounds(cell) && !self.blocked.contains_key(&cell)
    }

    fn cell_center(&self, cell: GridCell) -> [f32; 2] {
        [
            GRID_ORIGIN[0] + (cell.x as f32 + 0.5) * GRID_CELL_SIZE,
            GRID_ORIGIN[1] + (cell.y as f32 + 0.5) * GRID_CELL_SIZE,
        ]
    }

    fn home_cell(&self) -> GridCell {
        self.cell_at(HOME).expect("home is inside the sample grid")
    }

    fn resource_cell(&self, index: usize) -> GridCell {
        self.cell_at(RESOURCE_NODES[index])
            .expect("resource node is inside the sample grid")
    }

    fn is_protected(&self, cell: GridCell) -> bool {
        cell == self.home_cell()
            || (0..RESOURCE_NODES.len()).any(|index| cell == self.resource_cell(index))
    }

    fn find_path(&self, start: GridCell, goal: GridCell) -> Option<VecDeque<GridCell>> {
        if !self.is_walkable(start) || !self.is_walkable(goal) {
            return None;
        }
        if start == goal {
            return Some(VecDeque::new());
        }

        let mut frontier = VecDeque::from([start]);
        let mut previous = BTreeMap::<GridCell, Option<GridCell>>::new();
        previous.insert(start, None);

        while let Some(current) = frontier.pop_front() {
            let neighbors = [
                GridCell {
                    x: current.x + 1,
                    y: current.y,
                },
                GridCell {
                    x: current.x,
                    y: current.y + 1,
                },
                GridCell {
                    x: current.x - 1,
                    y: current.y,
                },
                GridCell {
                    x: current.x,
                    y: current.y - 1,
                },
            ];
            for next in neighbors {
                if !self.is_walkable(next) || previous.contains_key(&next) {
                    continue;
                }
                previous.insert(next, Some(current));
                if next == goal {
                    let mut reverse_path = Vec::new();
                    let mut cursor = goal;
                    while cursor != start {
                        reverse_path.push(cursor);
                        cursor = previous.get(&cursor)?.as_ref().copied()?;
                    }
                    reverse_path.reverse();
                    return Some(reverse_path.into());
                }
                frontier.push_back(next);
            }
        }
        None
    }
}

struct WorkerState {
    guid: Guid,
    job_node: Option<usize>,
    carrying: bool,
    shown_carrying: bool,
    path: VecDeque<GridCell>,
    route_goal: Option<GridCell>,
    route_revision: u64,
    route_attempted: bool,
}

impl WorkerState {
    fn new(guid: Guid) -> Self {
        Self {
            guid,
            job_node: None,
            carrying: false,
            shown_carrying: false,
            path: VecDeque::new(),
            route_goal: None,
            route_revision: 0,
            route_attempted: false,
        }
    }
}

struct GameState {
    window: Arc<Window>,
    renderer: GpuRenderer,
    runtime: GameRuntime,
    input: InputMap,
    navigation_grid: Arc<Mutex<NavigationGrid>>,
    delivered: Arc<AtomicU32>,
    population: Arc<AtomicU32>,
    queued_jobs: Arc<AtomicU32>,
    cursor_position: Option<[f64; 2]>,
    paused: bool,
    time_scale: f32,
    frame_samples_ms: VecDeque<f64>,
    system_samples_ms: VecDeque<f64>,
    last_profile_title: Instant,
    last_frame: Instant,
}

impl GameState {
    fn new(window: Arc<Window>, renderer: GpuRenderer) -> Result<Self, Box<dyn Error>> {
        let (scene, workers, grid) = make_scene();
        let worker_count = workers.len();
        let navigation_grid = Arc::new(Mutex::new(grid));
        let jobs = Arc::new(Mutex::new(
            (0..worker_count + 64)
                .map(|index| index % RESOURCE_NODES.len())
                .collect::<VecDeque<_>>(),
        ));
        let delivered = Arc::new(AtomicU32::new(0));
        let population = Arc::new(AtomicU32::new(worker_count as u32));
        let queued_jobs = Arc::new(AtomicU32::new((worker_count + 64) as u32));
        let mut runtime = GameRuntime::from_scene(&scene, RuntimeConfig::default())?;
        runtime.add_fixed_system(make_colony_system(
            workers,
            delivered.clone(),
            population.clone(),
            queued_jobs.clone(),
            jobs,
            navigation_grid.clone(),
        ));

        let mut input = InputMap::new();
        input.bind_key(PhysicalKey::Code(KeyCode::Space), BUILD_WORKER, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyB), BUILD_WORKER, 1.0)?;

        Ok(Self {
            window,
            renderer,
            runtime,
            input,
            navigation_grid,
            delivered,
            population,
            queued_jobs,
            cursor_position: None,
            paused: false,
            time_scale: 1.0,
            frame_samples_ms: VecDeque::with_capacity(120),
            system_samples_ms: VecDeque::with_capacity(120),
            last_profile_title: Instant::now(),
            last_frame: Instant::now(),
        })
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let frame_delta = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        let simulation_delta = if self.paused {
            Duration::ZERO
        } else {
            frame_delta.mul_f32(self.time_scale)
        };
        let update_start = Instant::now();
        self.runtime.advance_frame(simulation_delta);
        self.system_samples_ms
            .push_back(update_start.elapsed().as_secs_f64() * 1000.0);
        if self.system_samples_ms.len() > 120 {
            self.system_samples_ms.pop_front();
        }
        self.frame_samples_ms
            .push_back(frame_delta.as_secs_f64() * 1000.0);
        if self.frame_samples_ms.len() > 120 {
            self.frame_samples_ms.pop_front();
        }

        if now.saturating_duration_since(self.last_profile_title) >= Duration::from_millis(500) {
            self.last_profile_title = now;
            let delivered = self.delivered.load(Ordering::Relaxed);
            let population = self.population.load(Ordering::Relaxed);
            let queued_jobs = self.queued_jobs.load(Ordering::Relaxed);
            let object_count = self.runtime.world().object_count();
            let average_frame = average(&self.frame_samples_ms);
            let fps = if average_frame > 0.0 {
                1000.0 / average_frame
            } else {
                0.0
            };
            let profile_ms = average(&self.system_samples_ms);
            let paused = if self.paused { "PAUSED" } else { "RUNNING" };
            self.window.set_title(&format!(
                "Forge Colony | {paused} | {population}/{MAX_WORKERS} workers | {queued_jobs} queued jobs | food {delivered} | {fps:.0} FPS | update {profile_ms:.2} ms | {object_count} objects | P pause, 1-3 speed, B/Space build, click tiles"
            ));
        }

        self.renderer
            .render_world(self.runtime.world_mut(), [0.018, 0.028, 0.050, 1.0])?;
        self.window.request_redraw();
        Ok(())
    }

    fn handle_input(&mut self, event: &WindowEvent) {
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor_position = Some([position.x, position.y]);
        }
        if let WindowEvent::KeyboardInput { event, .. } = event {
            if event.state == ElementState::Pressed && !event.repeat {
                let key = match &event.physical_key {
                    PhysicalKey::Code(key) => Some(*key),
                    _ => None,
                };
                match key {
                    Some(KeyCode::KeyP) => self.paused = !self.paused,
                    Some(KeyCode::Digit1) => self.time_scale = 1.0,
                    Some(KeyCode::Digit2) => self.time_scale = 2.0,
                    Some(KeyCode::Digit3) => self.time_scale = 3.0,
                    _ => {}
                }
            }
        }
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
            ..
        } = event
        {
            self.toggle_tile_at_cursor();
        }

        let at = self.runtime.simulation_time();
        self.input.handle_window_event(event, at, &mut self.runtime);
    }

    fn toggle_tile_at_cursor(&mut self) {
        let Some(cursor) = self.cursor_position else {
            return;
        };
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let world_position = [
            (cursor[0] as f32 - size.width as f32 * 0.5) / CAMERA_PIXELS_PER_UNIT,
            (size.height as f32 * 0.5 - cursor[1] as f32) / CAMERA_PIXELS_PER_UNIT,
        ];
        let Ok(mut grid) = self.navigation_grid.lock() else {
            return;
        };
        let Some(cell) = grid.cell_at(world_position) else {
            return;
        };
        if grid.is_protected(cell) {
            return;
        }
        if let Some(guid) = grid.blocked.remove(&cell) {
            let entity = self.runtime.world().entity_for_guid(guid);
            if let Some(entity) = entity {
                let _ = self.runtime.world_mut().despawn(entity);
            }
            grid.revision = grid.revision.wrapping_add(1);
        } else {
            let guid = Guid::new();
            let sprite = Sprite {
                size: [GRID_CELL_SIZE * 0.94, GRID_CELL_SIZE * 0.94],
                tint: [0.28, 0.35, 0.43, 1.0],
                layer: 1,
                ..Sprite::default()
            };
            if self
                .runtime
                .world_mut()
                .spawn(
                    guid,
                    Transform2D {
                        translation: grid.cell_center(cell),
                        ..Transform2D::default()
                    },
                    Some(sprite),
                )
                .is_ok()
            {
                grid.blocked.insert(cell, guid);
                grid.revision = grid.revision.wrapping_add(1);
            }
        }
    }
}

fn average(samples: &VecDeque<f64>) -> f64 {
    if samples.is_empty() {
        0.0
    } else {
        samples.iter().sum::<f64>() / samples.len() as f64
    }
}

fn make_scene() -> (SceneDocument, Vec<WorkerState>, NavigationGrid) {
    let mut objects = Vec::new();
    let mut grid = NavigationGrid::new();

    for y in 0..GRID_HEIGHT {
        for x in 0..GRID_WIDTH {
            let cell = GridCell { x, y };
            let tint = if (x + y) % 2 == 0 {
                [0.055, 0.085, 0.11, 1.0]
            } else {
                [0.065, 0.095, 0.12, 1.0]
            };
            objects.push(scene_sprite(
                format!("Ground tile {x},{y}"),
                grid.cell_center(cell),
                [GRID_CELL_SIZE * 0.96, GRID_CELL_SIZE * 0.96],
                tint,
                0,
            ));
        }
    }

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
            2,
        ));
    }

    let obstacle_cells = [
        GridCell { x: 8, y: 5 },
        GridCell { x: 8, y: 6 },
        GridCell { x: 8, y: 7 },
        GridCell { x: 8, y: 8 },
        GridCell { x: 8, y: 9 },
        GridCell { x: 8, y: 10 },
        GridCell { x: 21, y: 5 },
        GridCell { x: 21, y: 6 },
        GridCell { x: 21, y: 7 },
        GridCell { x: 21, y: 8 },
        GridCell { x: 21, y: 9 },
        GridCell { x: 21, y: 10 },
        GridCell { x: 12, y: 12 },
        GridCell { x: 13, y: 12 },
        GridCell { x: 14, y: 12 },
        GridCell { x: 15, y: 12 },
        GridCell { x: 16, y: 12 },
        GridCell { x: 17, y: 12 },
    ];
    for cell in obstacle_cells {
        let guid = Guid::new();
        grid.blocked.insert(cell, guid);
        let mut object = scene_sprite(
            format!("Blocked tile {},{}", cell.x, cell.y),
            grid.cell_center(cell),
            [GRID_CELL_SIZE * 0.94, GRID_CELL_SIZE * 0.94],
            [0.28, 0.35, 0.43, 1.0],
            1,
        );
        object.object_guid = guid;
        objects.push(object);
    }

    let mut camera = scene_sprite("Camera", [0.0, 0.0], [0.0, 0.0], [1.0; 4], 0);
    camera.sprite = None;
    camera.camera = Some(AuthoredCamera {
        zoom: [1.0, 1.0],
        pixels_per_world_unit: CAMERA_PIXELS_PER_UNIT,
        priority: 1,
        active: true,
    });
    objects.push(camera);

    let mut workers = Vec::new();
    for index in 0..64 {
        let guid = Guid::new();
        let angle = index as f32 * 2.399_963_1;
        let radius = 0.35 + (index % 5) as f32 * 0.12;
        let position = [angle.cos() * radius, angle.sin() * radius];
        let mut object = scene_sprite(
            format!("Worker {index}"),
            position,
            [0.72, 0.72],
            [0.30, 0.90, 0.66, 1.0],
            3,
        );
        object.object_guid = guid;
        objects.push(object);
        workers.push(WorkerState::new(guid));
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
        grid,
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
    queued_jobs: Arc<AtomicU32>,
    jobs: Arc<Mutex<VecDeque<usize>>>,
    navigation_grid: Arc<Mutex<NavigationGrid>>,
) -> impl for<'a> FnMut(&mut RuntimeWorld, &FixedUpdateContext<'a>) + Send {
    move |world, context| {
        let Ok(grid) = navigation_grid.lock() else {
            return;
        };
        let Ok(mut jobs) = jobs.lock() else {
            return;
        };

        let spawn_count = context
            .input_events
            .iter()
            .filter(|event| event.action == BUILD_WORKER && event.pressed)
            .count();

        for _ in 0..spawn_count {
            if workers.len() >= MAX_WORKERS {
                break;
            }
            let guid = Guid::new();
            let angle = workers.len() as f32 * 2.399_963_1;
            let sprite = Sprite {
                size: [0.72, 0.72],
                tint: [0.30, 0.90, 0.66, 1.0],
                layer: 3,
                ..Sprite::default()
            };
            let position = [HOME[0] + angle.cos() * 0.55, HOME[1] + angle.sin() * 0.55];
            if world
                .spawn(
                    guid,
                    Transform2D {
                        translation: position,
                        ..Transform2D::default()
                    },
                    Some(sprite),
                )
                .is_ok()
            {
                workers.push(WorkerState::new(guid));
                population.fetch_add(1, Ordering::Relaxed);
            }
        }

        let step = WORKER_SPEED * context.delta.as_secs_f32();
        let mut deliveries = 0;
        for worker in &mut workers {
            if worker.job_node.is_none() {
                worker.job_node = jobs.pop_front();
            }
            let Some(node_index) = worker.job_node else {
                continue;
            };
            let Some(entity) = world.entity_for_guid(worker.guid) else {
                continue;
            };
            let Some(mut transform) = world.transform(entity) else {
                continue;
            };
            let Some(current_cell) = grid.cell_at(transform.translation) else {
                continue;
            };
            let goal = if worker.carrying {
                grid.home_cell()
            } else {
                grid.resource_cell(node_index)
            };

            if current_cell == goal {
                if worker.carrying {
                    worker.carrying = false;
                    worker.job_node = None;
                    jobs.push_back(node_index);
                    deliveries += 1;
                } else {
                    worker.carrying = true;
                }
                worker.path.clear();
                worker.route_goal = None;
                worker.route_attempted = false;
                if worker.carrying != worker.shown_carrying {
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
                    worker.shown_carrying = worker.carrying;
                }
                continue;
            }

            if worker.route_goal != Some(goal)
                || worker.route_revision != grid.revision
                || !worker.route_attempted
            {
                worker.path = grid.find_path(current_cell, goal).unwrap_or_default();
                worker.route_goal = Some(goal);
                worker.route_revision = grid.revision;
                worker.route_attempted = true;
            }
            let Some(next_cell) = worker.path.front().copied() else {
                continue;
            };
            let destination = grid.cell_center(next_cell);
            let dx = destination[0] - transform.translation[0];
            let dy = destination[1] - transform.translation[1];
            let distance = (dx * dx + dy * dy).sqrt();
            if distance <= ARRIVAL_RADIUS {
                worker.path.pop_front();
            } else if distance > 0.0 {
                let movement = step.min(distance);
                transform.translation[0] += dx / distance * movement;
                transform.translation[1] += dy / distance * movement;
                let _ = world.set_transform(entity, transform);
            }
        }
        queued_jobs.store(jobs.len() as u32, Ordering::Relaxed);
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
                        .with_title("Forge Colony | Space/B builds, P pauses, 1-3 controls time")
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

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn cell_mapping_rejects_non_finite_and_out_of_bounds_positions() {
        let grid = NavigationGrid::new();
        let cell = GridCell { x: 0, y: 0 };
        assert_eq!(grid.cell_at(grid.cell_center(cell)), Some(cell));
        assert_eq!(grid.cell_at([f32::NAN, -15.0]), None);
        assert_eq!(grid.cell_at([GRID_ORIGIN[0] - 0.01, GRID_ORIGIN[1]]), None);
        assert_eq!(
            grid.cell_at([
                GRID_ORIGIN[0] + GRID_WIDTH as f32 * GRID_CELL_SIZE,
                GRID_ORIGIN[1]
            ]),
            None
        );
    }

    #[test]
    fn pathfinding_avoids_blockers_and_is_deterministic() {
        let mut grid = NavigationGrid::new();
        let start = GridCell { x: 5, y: 5 };
        let goal = GridCell { x: 8, y: 5 };
        grid.blocked.insert(GridCell { x: 6, y: 5 }, Guid::new());
        grid.blocked.insert(GridCell { x: 7, y: 5 }, Guid::new());

        let path = grid.find_path(start, goal).expect("route around blockers");
        assert_eq!(path.len(), 5);
        assert_eq!(path, grid.find_path(start, goal).expect("same route"));
        let mut previous = start;
        for cell in path {
            assert!(grid.is_walkable(cell));
            assert_eq!(
                (previous.x - cell.x).abs() + (previous.y - cell.y).abs(),
                1,
                "each route step must move to one neighboring tile"
            );
            previous = cell;
        }
        assert_eq!(previous, goal);
    }

    #[test]
    fn pathfinding_rejects_blocked_and_out_of_bounds_endpoints() {
        let mut grid = NavigationGrid::new();
        let open = GridCell { x: 10, y: 8 };
        let blocked = GridCell { x: 11, y: 8 };
        let outside = GridCell {
            x: GRID_WIDTH,
            y: 8,
        };
        grid.blocked.insert(blocked, Guid::new());

        assert_eq!(
            grid.find_path(open, open),
            Some(VecDeque::new()),
            "an open cell can route to itself"
        );
        assert!(grid.find_path(blocked, open).is_none());
        assert!(grid.find_path(open, blocked).is_none());
        assert!(grid.find_path(blocked, blocked).is_none());
        assert!(grid.find_path(outside, outside).is_none());
    }

    #[test]
    fn home_and_resource_cells_are_protected() {
        let grid = NavigationGrid::new();
        assert!(grid.is_protected(grid.home_cell()));
        for index in 0..RESOURCE_NODES.len() {
            assert!(grid.is_protected(grid.resource_cell(index)));
        }
        assert!(!grid.is_protected(GridCell { x: 1, y: 1 }));
    }

    #[test]
    fn editor_authored_colony_routes_workers_and_updates_the_hud() {
        use forge_assets::AssetCatalog;
        use forge_project::Project;
        use forge_runtime::ScriptUiCommand;

        let project_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../colony_project");
        let project = Project::open(project_path).expect("open editor-authored colony project");
        let assets = AssetCatalog::open(project.root()).expect("open colony assets");
        let mut runtime = GameRuntime::from_scene_with_assets(
            project.default_scene(),
            RuntimeConfig::default(),
            &assets,
        )
        .expect("load editor-authored colony scene");

        assert_eq!(
            runtime.sprite_animation_count(),
            8,
            "each colony worker should load the authored walk clip"
        );
        runtime.advance_frame(Duration::from_millis(50));
        let simulation_time = runtime.simulation_time();
        runtime.push_action(simulation_time, ActionId(3), 1.0, true);
        for _ in 0..3000 {
            runtime.advance_frame(Duration::from_nanos(16_666_667));
        }

        let diagnostics = runtime.take_script_diagnostics();
        assert!(
            diagnostics.is_empty(),
            "colony scripts should remain healthy: {diagnostics:?}"
        );
        let ui_commands = runtime.take_ui_commands();
        let food_guid = "e055c011-0a71-4e5e-bc01-000000000002"
            .parse::<Guid>()
            .expect("parse food label GUID");
        let deliveries = ui_commands
            .iter()
            .filter_map(|command| match command {
                ScriptUiCommand::SetText { element_guid, text } if *element_guid == food_guid => {
                    text.strip_prefix("FOOD ")
                        .and_then(|value| value.parse::<u32>().ok())
                }
                _ => None,
            })
            .max()
            .unwrap_or_default();
        assert!(deliveries > 0, "a routed worker should deliver food");

        let (_, _, grid) = make_scene();
        for index in 1..=8 {
            let worker_guid = format!("d055c011-0a71-4e5e-bc01-{index:012}")
                .parse::<Guid>()
                .expect("parse worker GUID");
            let entity = runtime
                .entity_for_guid(worker_guid)
                .expect("authored worker should remain in the scene");
            let transform = runtime.world().transform(entity).expect("worker transform");
            let cell = grid
                .cell_at(transform.translation)
                .expect("worker stays within the navigable grid");
            assert!(
                !grid.blocked.contains_key(&cell),
                "worker {index} crossed a blocked tile"
            );
        }
    }
}
