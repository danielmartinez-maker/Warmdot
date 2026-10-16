//! Headless-capable runtime kernel with fixed-step input and physics barriers.

use std::{collections::BTreeMap, error::Error, fmt, time::Duration};

use forge_assets::{
    AnimationClipDocument, AnimationPlayer, AssetCatalog, AssetError, LoadedScriptAsset,
    SpriteSheetDocument,
};
use forge_core::{
    ActionEvent, ActionId, ActionState, FixedStepClock, FramePlan, Guid, InputTimeline,
};
use forge_physics::{CollisionEvent2D, PhysicsSimulation};
use forge_scene::{SceneDiagnostic, SceneDocument};
pub use forge_scripting::{ScriptDefinition, ScriptDiagnostic, ScriptError};
use forge_scripting::ScriptHost;
use forge_world::{RuntimeWorld, WorldError};

#[derive(Debug)]
pub enum SpriteAnimationError {
    Asset(AssetError),
    MissingObject(Guid),
    World(WorldError),
}

impl fmt::Display for SpriteAnimationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => {
                write!(f, "sprite animation asset could not be loaded: {error}")
            }
            Self::MissingObject(guid) => write!(
                f,
                "sprite animation object {guid} does not exist in this scene"
            ),
            Self::World(error) => {
                write!(f, "sprite animation could not update the world: {error}")
            }
        }
    }
}

impl Error for SpriteAnimationError {}

impl From<AssetError> for SpriteAnimationError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

impl From<WorldError> for SpriteAnimationError {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

struct RuntimeSpriteAnimation {
    clip: AnimationClipDocument,
    sheet: SpriteSheetDocument,
    texture_width: u32,
    texture_height: u32,
    player: AnimationPlayer,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub fixed_step: Duration,
    pub max_frame_delta: Duration,
    pub max_steps_per_frame: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            fixed_step: Duration::from_nanos(16_666_667),
            max_frame_delta: Duration::from_millis(250),
            max_steps_per_frame: 8,
        }
    }
}

pub struct FixedUpdateContext<'a> {
    pub tick: u64,
    pub delta: Duration,
    pub simulation_time: Duration,
    pub input_events: &'a [ActionEvent],
    pub physics_events: &'a [CollisionEvent2D],
    input: &'a InputTimeline,
}

impl FixedUpdateContext<'_> {
    pub fn action(&self, action: ActionId) -> Option<ActionState> {
        self.input.state(action)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeFrame {
    pub first_tick: u64,
    pub fixed_steps: u32,
    pub interpolation_alpha: f64,
    pub accepted_frame_time: Duration,
    pub dropped_time: Duration,
    pub simulation_time: Duration,
    pub physics_events: Vec<CollisionEvent2D>,
}

#[derive(Debug)]
pub enum RuntimeError {
    InvalidClockConfiguration(&'static str),
    Scene(Vec<SceneDiagnostic>),
    Animation(SpriteAnimationError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidClockConfiguration(message) => f.write_str(message),
            Self::Scene(diagnostics) => write!(f, "scene has {} diagnostic(s)", diagnostics.len()),
            Self::Animation(error) => write!(f, "{error}"),
        }
    }
}

impl Error for RuntimeError {}

type FixedSystem = Box<dyn for<'a> FnMut(&mut RuntimeWorld, &FixedUpdateContext<'a>) + Send>;

pub struct GameRuntime {
    world: RuntimeWorld,
    clock: FixedStepClock,
    input: InputTimeline,
    fixed_systems: Vec<FixedSystem>,
    simulation_time: Duration,
    physics: PhysicsSimulation,
    pending_physics_events: Vec<CollisionEvent2D>,
    paused: bool,
    sprite_animations: BTreeMap<Guid, RuntimeSpriteAnimation>,
    script_host: ScriptHost,
}

impl GameRuntime {
    pub fn new(world: RuntimeWorld, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let clock = FixedStepClock::new(
            config.fixed_step,
            config.max_frame_delta,
            config.max_steps_per_frame,
        )
        .map_err(RuntimeError::InvalidClockConfiguration)?;
        Ok(Self {
            world,
            clock,
            input: InputTimeline::new(),
            fixed_systems: Vec::new(),
            simulation_time: Duration::ZERO,
            physics: PhysicsSimulation::new(),
            pending_physics_events: Vec::new(),
            paused: false,
            sprite_animations: BTreeMap::new(),
            script_host: ScriptHost::new(),
        })
    }

    pub fn from_scene(scene: &SceneDocument, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let world = scene.instantiate().map_err(RuntimeError::Scene)?;
        Self::new(world, config)
    }

    pub fn from_scene_json(source: &str, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let scene = SceneDocument::from_json(source).map_err(RuntimeError::Scene)?;
        Self::from_scene(&scene, config)
    }

    pub fn from_scene_with_assets(
        scene: &SceneDocument,
        config: RuntimeConfig,
        assets: &AssetCatalog,
    ) -> Result<Self, RuntimeError> {
        let mut runtime = Self::from_scene(scene, config)?;
        runtime.attach_scene_animations(scene, assets)?;
        runtime.attach_scene_scripts(scene, assets);
        Ok(runtime)
    }

    /// Registers one fixed update system. Insertion order is the execution order.
    pub fn add_fixed_system(
        &mut self,
        system: impl for<'a> FnMut(&mut RuntimeWorld, &FixedUpdateContext<'a>) + Send + 'static,
    ) {
        self.fixed_systems.push(Box::new(system));
    }

    /// Queues an action transition with a timestamp on the simulation timeline.
    pub fn push_action(&mut self, at: Duration, action: ActionId, value: f32, pressed: bool) {
        self.input.push(at, action, value, pressed);
    }

    /// Pausing stops wall-clock accumulation while preserving the current scene and systems.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.clock.clear_accumulator();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Advances exactly one fixed tick while paused. Returns None while the runtime is playing.
    pub fn step_once(&mut self) -> Option<RuntimeFrame> {
        if !self.paused {
            return None;
        }
        let plan = self.clock.advance_one_step();
        Some(self.execute_plan(plan))
    }

    /// Replaces the scene and resets physics, clock, input, and pending collision events.
    pub fn replace_scene(&mut self, scene: &SceneDocument) -> Result<(), RuntimeError> {
        self.world = scene.instantiate().map_err(RuntimeError::Scene)?;
        self.sprite_animations.clear();
        self.script_host = ScriptHost::new();
        self.clock.reset();
        self.input.clear();
        self.simulation_time = Duration::ZERO;
        self.physics = PhysicsSimulation::new();
        self.pending_physics_events.clear();
        Ok(())
    }

    pub fn replace_scene_with_assets(
        &mut self,
        scene: &SceneDocument,
        assets: &AssetCatalog,
    ) -> Result<(), RuntimeError> {
        self.replace_scene(scene)?;
        self.attach_scene_animations(scene, assets)?;
        self.attach_scene_scripts(scene, assets);
        Ok(())
    }

    fn attach_scene_animations(
        &mut self,
        scene: &SceneDocument,
        assets: &AssetCatalog,
    ) -> Result<(), RuntimeError> {
        for object in &scene.objects {
            if let Some(clip_guid) = object.sprite.and_then(|sprite| sprite.animation_clip) {
                self.attach_sprite_animation(object.object_guid, clip_guid, assets)
                    .map_err(RuntimeError::Animation)?;
            }
        }
        Ok(())
    }

    fn attach_scene_scripts(&mut self, scene: &SceneDocument, assets: &AssetCatalog) {
        for object in &scene.objects {
            let Some(authored) = object.script.as_ref() else {
                continue;
            };
            match assets.load_script(authored.asset) {
                Ok(asset) => {
                    let definition = ScriptDefinition {
                        object_guid: object.object_guid,
                        asset_guid: authored.asset,
                        source_path: asset.relative_path,
                        source: asset.source,
                        properties: authored.properties.clone(),
                    };
                    if let Err(error) = self.script_host.attach(definition, &mut self.world) {
                        self.script_host.record_diagnostic(
                            object.object_guid,
                            authored.asset,
                            "script",
                            0,
                            error.to_string(),
                        );
                    }
                }
                Err(error) => self.script_host.record_diagnostic(
                    object.object_guid,
                    authored.asset,
                    "script",
                    0,
                    error.to_string(),
                ),
            }
        }
    }

    pub fn attach_sprite_animation(
        &mut self,
        object_guid: Guid,
        clip_guid: Guid,
        assets: &AssetCatalog,
    ) -> Result<(), SpriteAnimationError> {
        let clip = assets.load_animation_clip(clip_guid)?;
        let sheet_guid = clip.sprite_sheet;
        let sheet = assets.load_sprite_sheet(sheet_guid)?;
        let texture = assets.load_png(sheet.texture)?;
        clip.validate(sheet_guid, &sheet)?;
        let entity = self
            .world
            .entity_for_guid(object_guid)
            .ok_or(SpriteAnimationError::MissingObject(object_guid))?;
        let mut player = AnimationPlayer::default();
        let uv_rect = player.current_frame_uv_rect(&clip, &sheet, texture.width, texture.height)?;
        self.world.set_sprite_asset(entity, Some(sheet.texture))?;
        self.world.set_sprite_uv_rect(entity, uv_rect)?;
        self.sprite_animations.insert(
            object_guid,
            RuntimeSpriteAnimation {
                clip,
                sheet,
                texture_width: texture.width,
                texture_height: texture.height,
                player,
            },
        );
        Ok(())
    }

    pub fn detach_sprite_animation(&mut self, object_guid: Guid) -> bool {
        self.sprite_animations.remove(&object_guid).is_some()
    }

    pub fn set_sprite_animation_playing(&mut self, object_guid: Guid, playing: bool) -> bool {
        let Some(animation) = self.sprite_animations.get_mut(&object_guid) else {
            return false;
        };
        animation.player.set_playing(playing);
        true
    }

    pub fn sprite_animation_count(&self) -> usize {
        self.sprite_animations.len()
    }

    fn advance_sprite_animations(&mut self, delta: Duration) {
        let mut remove = Vec::new();
        for (object_guid, animation) in &mut self.sprite_animations {
            let Some(entity) = self.world.entity_for_guid(*object_guid) else {
                remove.push(*object_guid);
                continue;
            };
            if animation.player.advance(&animation.clip, delta).is_err() {
                remove.push(*object_guid);
                continue;
            }
            let Ok(uv_rect) = animation.player.current_frame_uv_rect(
                &animation.clip,
                &animation.sheet,
                animation.texture_width,
                animation.texture_height,
            ) else {
                remove.push(*object_guid);
                continue;
            };
            if self.world.set_sprite_uv_rect(entity, uv_rect).is_err() {
                remove.push(*object_guid);
            }
        }
        for object_guid in remove {
            self.sprite_animations.remove(&object_guid);
        }
    }

    /// Advances bounded fixed steps. Systems run before physics; collision events produced by
    /// physics are delivered to systems at the next fixed-tick barrier. Paused time is ignored.
    pub fn advance_frame(&mut self, frame_delta: Duration) -> RuntimeFrame {
        let plan = if self.paused {
            self.clock.advance(Duration::ZERO)
        } else {
            self.clock.advance(frame_delta)
        };
        self.execute_plan(plan)
    }

    fn execute_plan(&mut self, plan: FramePlan) -> RuntimeFrame {
        let mut frame_physics_events = Vec::new();
        for offset in 0..plan.fixed_steps {
            self.simulation_time = self.simulation_time.saturating_add(self.clock.step());
            let input_events = self.input.drain_through(self.simulation_time);
            let context = FixedUpdateContext {
                tick: plan.first_tick.saturating_add(offset as u64),
                delta: self.clock.step(),
                simulation_time: self.simulation_time,
                input_events: &input_events,
                physics_events: &self.pending_physics_events,
                input: &self.input,
            };
            for system in &mut self.fixed_systems {
                system(&mut self.world, &context);
            }
            self.script_host.update(
                &mut self.world,
                self.input.states(),
                self.clock.step(),
                context.tick,
            );
            self.advance_sprite_animations(self.clock.step());
            self.pending_physics_events = self
                .physics
                .step(&mut self.world, self.clock.step().as_secs_f32());
            frame_physics_events.extend(self.pending_physics_events.iter().copied());
        }

        RuntimeFrame {
            first_tick: plan.first_tick,
            fixed_steps: plan.fixed_steps,
            interpolation_alpha: plan.interpolation_alpha,
            accepted_frame_time: plan.accepted_frame_time,
            dropped_time: plan.dropped_time,
            simulation_time: self.simulation_time,
            physics_events: frame_physics_events,
        }
    }

    pub fn attach_script(&mut self, definition: ScriptDefinition) -> Result<(), ScriptError> {
        self.script_host.attach(definition, &mut self.world)
    }

    pub fn reload_script_asset(&mut self, asset: &LoadedScriptAsset) {
        self.script_host.reload_asset(asset);
    }

    pub fn report_script_asset_issue(
        &mut self,
        asset_guid: Guid,
        source_path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.script_host
            .report_asset_issue(asset_guid, source_path, message);
    }

    pub fn take_script_diagnostics(&mut self) -> Vec<ScriptDiagnostic> {
        self.script_host.take_diagnostics()
    }

    pub fn world(&self) -> &RuntimeWorld {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut RuntimeWorld {
        &mut self.world
    }

    pub fn physics(&self) -> &PhysicsSimulation {
        &self.physics
    }

    pub fn physics_mut(&mut self) -> &mut PhysicsSimulation {
        &mut self.physics
    }

    pub fn input_state(&self, action: ActionId) -> Option<ActionState> {
        self.input.state(action)
    }

    pub fn simulation_time(&self) -> Duration {
        self.simulation_time
    }

    pub fn total_dropped_time(&self) -> Duration {
        self.clock.total_dropped_time()
    }

    pub fn entity_for_guid(&self, guid: Guid) -> Option<forge_world::RuntimeEntity> {
        self.world.entity_for_guid(guid)
    }
}
