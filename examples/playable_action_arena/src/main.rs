use std::{
    collections::BTreeMap,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Instant,
};

use forge_core::{ActionId, Guid};
use forge_input::InputMap;
use forge_renderer::GpuRenderer;
use forge_runtime::{GameRuntime, RuntimeConfig, ScriptDefinition};
use forge_scene::{
    AuthoredCamera, AuthoredSprite, AuthoredTransform, SceneDocument, SceneObject,
    ScriptPropertyValue,
};
use forge_world::{Sprite, Transform2D};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const MOVE_X: ActionId = ActionId(1);
const MOVE_Y: ActionId = ActionId(2);
const FIRE: ActionId = ActionId(3);
const RESTART: ActionId = ActionId(4);
const PLAYER_GUID: Guid = Guid::from_uuid(uuid::uuid!("8d80f6af-a189-4f23-ae03-972341119bee"));

struct GameState {
    window: Arc<Window>,
    renderer: GpuRenderer,
    runtime: GameRuntime,
    input: InputMap,
    score: Arc<AtomicU32>,
    lives: Arc<AtomicU32>,
    game_over: Arc<AtomicBool>,
    shown_score: u32,
    shown_lives: u32,
    shown_game_over: bool,
    last_frame: Instant,
}

impl GameState {
    fn new(window: Arc<Window>, renderer: GpuRenderer) -> Result<Self, Box<dyn Error>> {
        let scene = make_scene();
        let mut runtime = GameRuntime::from_scene(&scene, RuntimeConfig::default())?;
        runtime.attach_script(ScriptDefinition {
            object_guid: PLAYER_GUID,
            asset_guid: Guid::new(),
            source_path: "Assets/PlayerController.luau".into(),
            source: include_str!("../Assets/PlayerController.luau").into(),
            properties: BTreeMap::from([
                ("speed".into(), ScriptPropertyValue::Number(16.0)),
                ("min_x".into(), ScriptPropertyValue::Number(-28.0)),
                ("max_x".into(), ScriptPropertyValue::Number(28.0)),
                ("min_y".into(), ScriptPropertyValue::Number(8.0)),
                ("max_y".into(), ScriptPropertyValue::Number(16.0)),
            ]),
        })?;
        let score = Arc::new(AtomicU32::new(0));
        let lives = Arc::new(AtomicU32::new(3));
        let game_over = Arc::new(AtomicBool::new(false));
        runtime.add_fixed_system(make_game_system(
            score.clone(),
            lives.clone(),
            game_over.clone(),
        ));

        let mut input = InputMap::new();
        input.bind_key(PhysicalKey::Code(KeyCode::KeyA), MOVE_X, -1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::ArrowLeft), MOVE_X, -1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyD), MOVE_X, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::ArrowRight), MOVE_X, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyW), MOVE_Y, -1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::ArrowUp), MOVE_Y, -1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyS), MOVE_Y, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::ArrowDown), MOVE_Y, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::Space), FIRE, 1.0)?;
        input.bind_mouse(MouseButton::Left, FIRE, 1.0)?;
        input.bind_key(PhysicalKey::Code(KeyCode::KeyR), RESTART, 1.0)?;

        Ok(Self {
            window,
            renderer,
            runtime,
            input,
            score,
            lives,
            game_over,
            shown_score: 0,
            shown_lives: 3,
            shown_game_over: false,
            last_frame: Instant::now(),
        })
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let frame_delta = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        self.runtime.advance_frame(frame_delta);
        let score = self.score.load(Ordering::Relaxed);
        let lives = self.lives.load(Ordering::Relaxed);
        let game_over = self.game_over.load(Ordering::Relaxed);
        if score != self.shown_score || lives != self.shown_lives || game_over != self.shown_game_over {
            self.shown_score = score;
            self.shown_lives = lives;
            self.shown_game_over = game_over;
            let state = if game_over { "GAME OVER | R restarts" } else { "WASD move | Space/click fire | R restart" };
            self.window
                .set_title(&format!("Forge 2D Action Arena | Score {score} | Lives {lives} | {state}"));
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

fn make_scene() -> SceneDocument {
    let mut objects = Vec::new();
    for row in 0..8 {
        for column in 0..14 {
            let tint = if (row + column) % 2 == 0 {
                [0.035, 0.055, 0.085, 1.0]
            } else {
                [0.045, 0.065, 0.095, 1.0]
            };
            objects.push(authored_sprite(
                format!("Arena tile {column},{row}"),
                [(column as f32 - 6.5) * 4.0, (row as f32 - 3.5) * 4.0],
                [3.92, 3.92],
                tint,
                -1,
                row * 14 + column,
            ));
        }
    }
    objects.push(SceneObject {
        object_guid: PLAYER_GUID,
        name: "Player".into(),
        parent_guid: None,
        prefab_instance: None,
        transform: AuthoredTransform {
            translation: [0.0, 13.0],
            rotation_radians: 0.0,
            scale: [1.0, 1.0],
        },
        sprite: Some(AuthoredSprite {
            asset: None,
            animation_clip: None,
            size: [1.4, 1.4],
            anchor: [0.5, 0.5],
            tint: [0.18, 0.88, 0.80, 1.0],
            layer: 1,
            order: 0,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
        }),
        camera: None,
        rigid_body: None,
        collider: None,
        script: None,
    });
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
    for (name, position, size) in [
        ("North wall", [0.0, 16.5], [60.0, 1.0]),
        ("South wall", [0.0, -16.5], [60.0, 1.0]),
        ("East wall", [29.5, 0.0], [1.0, 34.0]),
        ("West wall", [-29.5, 0.0], [1.0, 34.0]),
    ] {
        objects.push(authored_sprite(
            name,
            position,
            size,
            [0.12, 0.24, 0.34, 1.0],
            0,
            1000,
        ));
    }
    for index in 0..48 {
        let seed = index as u32 * 747_796_405 + 2_891_336_453;
        let x = (seed % 560) as f32 / 10.0 - 28.0;
        let y = ((seed / 19) % 320) as f32 / 10.0 - 16.0;
        objects.push(SceneObject {
            object_guid: Guid::new(),
            name: format!("Star {index}"),
            parent_guid: None,
            prefab_instance: None,
            transform: AuthoredTransform {
                translation: [x, y],
                rotation_radians: 0.0,
                scale: [1.0, 1.0],
            },
            sprite: Some(AuthoredSprite {
                asset: None,
                animation_clip: None,
                size: [0.16, 0.16],
                anchor: [0.5, 0.5],
                tint: [0.30, 0.48, 0.65, 1.0],
                layer: 0,
                order: index,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
            }),
            camera: None,
            rigid_body: None,
            collider: None,
            script: None,
        });
    }
    SceneDocument {
        schema_version: forge_scene::CURRENT_SCENE_SCHEMA,
        resource_guid: Guid::new(),
        ui_document: None,
        name: "Action Arena".into(),
        dependencies: Vec::new(),
        objects,
    }
}

fn make_game_system(
    score: Arc<AtomicU32>,
    lives: Arc<AtomicU32>,
    game_over: Arc<AtomicBool>,
) -> impl for<'a> FnMut(&mut forge_world::RuntimeWorld, &forge_runtime::FixedUpdateContext<'a>) + Send
{
    let mut bullets = Vec::<Guid>::new();
    let mut enemies = Vec::<Guid>::new();
    let mut invulnerable_until = 0;
    let mut player_flashing = false;
    move |world, context| {
        let restart = context
            .input_events
            .iter()
            .any(|event| event.action == RESTART && event.pressed);
        if restart {
            let old_bullets = std::mem::take(&mut bullets);
            let old_enemies = std::mem::take(&mut enemies);
            for guid in old_bullets.into_iter().chain(old_enemies) {
                if let Some(entity) = world.entity_for_guid(guid) {
                    let _ = world.despawn(entity);
                }
            }
            if let Some(player) = world.entity_for_guid(PLAYER_GUID) {
                let _ = world.set_transform(
                    player,
                    Transform2D {
                        translation: [0.0, 13.0],
                        ..Transform2D::default()
                    },
                );
                let _ = world.set_sprite(
                    player,
                    Sprite {
                        size: [1.4, 1.4],
                        tint: [0.18, 0.88, 0.80, 1.0],
                        layer: 1,
                        ..Sprite::default()
                    },
                );
            }
            score.store(0, Ordering::Relaxed);
            lives.store(3, Ordering::Relaxed);
            game_over.store(false, Ordering::Relaxed);
            invulnerable_until = 0;
            player_flashing = false;
            return;
        }
        if game_over.load(Ordering::Relaxed) {
            return;
        }

        let delta = context.delta.as_secs_f32();
        if context
            .input_events
            .iter()
            .any(|event| event.action == FIRE && event.pressed)
        {
            if let Some(player) = world.entity_for_guid(PLAYER_GUID) {
                if let Some(transform) = world.transform(player) {
                    let guid = Guid::new();
                    let bullet = Transform2D {
                        translation: [transform.translation[0], transform.translation[1] - 1.1],
                        ..Transform2D::default()
                    };
                    let sprite = Sprite {
                        size: [0.28, 0.72],
                        tint: [1.0, 0.78, 0.25, 1.0],
                        layer: 2,
                        ..Sprite::default()
                    };
                    if world.spawn(guid, bullet, Some(sprite)).is_ok() {
                        bullets.push(guid);
                    }
                }
            }
        }

        if context.tick % 90 == 0 && enemies.len() < 16 {
            let lane = ((context.tick.wrapping_mul(37) % 520) as f32 / 10.0) - 26.0;
            let guid = Guid::new();
            let transform = Transform2D {
                translation: [lane, -17.0],
                ..Transform2D::default()
            };
            let sprite = Sprite {
                size: [1.0, 1.0],
                tint: [0.96, 0.30, 0.42, 1.0],
                layer: 1,
                ..Sprite::default()
            };
            if world.spawn(guid, transform, Some(sprite)).is_ok() {
                enemies.push(guid);
            }
        }

        bullets.retain(|guid| {
            let Some(entity) = world.entity_for_guid(*guid) else {
                return false;
            };
            let Some(mut transform) = world.transform(entity) else {
                return false;
            };
            transform.translation[1] -= 24.0 * delta;
            if transform.translation[1] < -20.0 {
                let _ = world.despawn(entity);
                return false;
            }
            let _ = world.set_transform(entity, transform);
            true
        });
        enemies.retain(|guid| {
            let Some(entity) = world.entity_for_guid(*guid) else {
                return false;
            };
            let Some(mut transform) = world.transform(entity) else {
                return false;
            };
            transform.translation[0] += (context.tick as f32 * 0.035).sin() * 0.18 * delta;
            transform.translation[1] += 2.8 * delta;
            if transform.translation[1] > 19.0 {
                let _ = world.despawn(entity);
                return false;
            }
            let _ = world.set_transform(entity, transform);
            let pulse = 0.9 + ((context.tick as f32 * 0.16).sin() + 1.0) * 0.05;
            let _ = world.set_sprite(
                entity,
                Sprite {
                    size: [pulse, pulse],
                    tint: [0.96, 0.30, 0.42, 1.0],
                    layer: 1,
                    ..Sprite::default()
                },
            );
            true
        });

        let mut hit_bullets = Vec::new();
        let mut hit_enemies = Vec::new();
        for bullet_guid in &bullets {
            let Some(bullet_entity) = world.entity_for_guid(*bullet_guid) else {
                continue;
            };
            let Some(bullet) = world.transform(bullet_entity) else {
                continue;
            };
            for enemy_guid in &enemies {
                let Some(enemy_entity) = world.entity_for_guid(*enemy_guid) else {
                    continue;
                };
                let Some(enemy) = world.transform(enemy_entity) else {
                    continue;
                };
                let dx = bullet.translation[0] - enemy.translation[0];
                let dy = bullet.translation[1] - enemy.translation[1];
                if dx * dx + dy * dy < 0.8 {
                    hit_bullets.push(*bullet_guid);
                    hit_enemies.push(*enemy_guid);
                    break;
                }
            }
        }
        hit_bullets.sort();
        hit_bullets.dedup();
        hit_enemies.sort();
        hit_enemies.dedup();
        for guid in &hit_bullets {
            if let Some(entity) = world.entity_for_guid(*guid) {
                let _ = world.despawn(entity);
            }
        }
        for guid in &hit_enemies {
            if let Some(entity) = world.entity_for_guid(*guid) {
                let _ = world.despawn(entity);
            }
        }
        if !hit_enemies.is_empty() {
            score.fetch_add(hit_enemies.len() as u32, Ordering::Relaxed);
            bullets.retain(|guid| !hit_bullets.contains(guid));
            enemies.retain(|guid| !hit_enemies.contains(guid));
        }

        let mut player_hit = None;
        if context.tick >= invulnerable_until {
            if let (Some(player), Some(player_transform)) = (
                world.entity_for_guid(PLAYER_GUID),
                world
                    .entity_for_guid(PLAYER_GUID)
                    .and_then(|entity| world.transform(entity)),
            ) {
                for enemy_guid in &enemies {
                    let Some(enemy_entity) = world.entity_for_guid(*enemy_guid) else {
                        continue;
                    };
                    let Some(enemy_transform) = world.transform(enemy_entity) else {
                        continue;
                    };
                    let dx = player_transform.translation[0] - enemy_transform.translation[0];
                    let dy = player_transform.translation[1] - enemy_transform.translation[1];
                    if dx * dx + dy * dy < 1.6 {
                        player_hit = Some(*enemy_guid);
                        let _ = world.set_transform(
                            player,
                            Transform2D {
                                translation: [0.0, 13.0],
                                ..Transform2D::default()
                            },
                        );
                        break;
                    }
                }
            }
        }
        if let Some(enemy_guid) = player_hit {
            if let Some(entity) = world.entity_for_guid(enemy_guid) {
                let _ = world.despawn(entity);
            }
            enemies.retain(|guid| *guid != enemy_guid);
            let current_lives = lives.load(Ordering::Relaxed);
            if current_lives > 0 {
                let remaining = lives.fetch_sub(1, Ordering::Relaxed) - 1;
                if remaining == 0 {
                    game_over.store(true, Ordering::Relaxed);
                }
            }
            invulnerable_until = context.tick.saturating_add(90);
        }

        let is_invulnerable = context.tick < invulnerable_until;
        if is_invulnerable != player_flashing {
            player_flashing = is_invulnerable;
            if let Some(player) = world.entity_for_guid(PLAYER_GUID) {
                let tint = if player_flashing {
                    [1.0, 0.88, 0.42, 1.0]
                } else {
                    [0.18, 0.88, 0.80, 1.0]
                };
                let _ = world.set_sprite(
                    player,
                    Sprite {
                        size: [1.4, 1.4],
                        tint,
                        layer: 1,
                        ..Sprite::default()
                    },
                );
            }
        }
    }
}

fn authored_sprite(
    name: impl Into<String>,
    position: [f32; 2],
    size: [f32; 2],
    tint: [f32; 4],
    layer: i32,
    order: i32,
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
            layer,
            order,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
        }),
        camera: None,
        rigid_body: None,
        collider: None,
        script: None,
    }
}

#[derive(Default)]
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
                        .with_title("Forge 2D Action Arena | WASD move, Space/click fire")
                        .with_inner_size(PhysicalSize::new(1280, 720)),
                )
                .expect("could not create action-arena window"),
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
                eprintln!("action arena could not start: {error}");
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
                    eprintln!("action arena frame failed: {error}");
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
