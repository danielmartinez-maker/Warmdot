//! Headless-capable runtime kernel with fixed-step input and physics barriers.

use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use forge_assets::{
    AnimationClipDocument, AnimationPlayer, AssetCatalog, AssetError, LoadedScriptAsset,
    SpriteSheetDocument,
};
use forge_core::{
    ActionEvent, ActionId, ActionState, FixedStepClock, FramePlan, Guid, InputTimeline,
};
use forge_physics::{CollisionEvent2D, PhysicsSimulation};
use forge_scene::{SceneDiagnostic, SceneDocument, ScriptPropertyValue};
pub use forge_scripting::{
    ScriptAudioCommand, ScriptBreakpointHit, ScriptDefinition, ScriptDiagnostic, ScriptError,
};
use forge_scripting::{ScriptEventValue, ScriptHost};
use forge_world::{RuntimeWorld, WorldError};

const MAX_QUEUED_SCENE_TRANSITION_REQUESTS: usize = 512;

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
    pub fixed_system_time: Duration,
    pub script_time: Duration,
    pub simulation_time: Duration,
    pub physics_events: Vec<CollisionEvent2D>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BreakpointContinueResult {
    Hit(ScriptBreakpointHit),
    FrameComplete(RuntimeFrame),
    NotPaused,
}

#[derive(Clone)]
struct PendingRuntimeFrame {
    first_tick: u64,
    next_tick: u64,
    remaining_steps: u32,
    completed_steps: u32,
    interpolation_alpha: f64,
    accepted_frame_time: Duration,
    dropped_time: Duration,
    fixed_system_time: Duration,
    script_time: Duration,
    physics_events: Vec<CollisionEvent2D>,
    current_tick_started: bool,
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
    debugger_paused: bool,
    pending_debug_frame: Option<PendingRuntimeFrame>,
    sprite_animations: BTreeMap<Guid, RuntimeSpriteAnimation>,
    script_host: ScriptHost,
    pending_scene_transitions: VecDeque<Guid>,
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
            debugger_paused: false,
            pending_debug_frame: None,
            sprite_animations: BTreeMap::new(),
            script_host: ScriptHost::new(),
            pending_scene_transitions: VecDeque::new(),
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
        self.paused || self.debugger_paused
    }

    /// Advances exactly one fixed tick while manually paused. Returns None while playing or
    /// stopped at a script breakpoint.
    pub fn step_once(&mut self) -> Option<RuntimeFrame> {
        if !self.paused || self.debugger_paused {
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
        self.pending_scene_transitions.clear();
        self.debugger_paused = false;
        self.pending_debug_frame = None;
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
        if self.debugger_paused {
            let plan = self.clock.advance(Duration::ZERO);
            return RuntimeFrame {
                first_tick: plan.first_tick,
                fixed_steps: 0,
                interpolation_alpha: plan.interpolation_alpha,
                accepted_frame_time: Duration::ZERO,
                dropped_time: Duration::ZERO,
                fixed_system_time: Duration::ZERO,
                script_time: Duration::ZERO,
                simulation_time: self.simulation_time,
                physics_events: Vec::new(),
            };
        }
        let plan = if self.paused {
            self.clock.advance(Duration::ZERO)
        } else {
            self.clock.advance(frame_delta)
        };
        self.execute_plan(plan)
    }

    fn execute_plan(&mut self, plan: FramePlan) -> RuntimeFrame {
        let mut pending = PendingRuntimeFrame {
            first_tick: plan.first_tick,
            next_tick: plan.first_tick,
            remaining_steps: plan.fixed_steps,
            completed_steps: 0,
            interpolation_alpha: plan.interpolation_alpha,
            accepted_frame_time: plan.accepted_frame_time,
            dropped_time: plan.dropped_time,
            fixed_system_time: Duration::ZERO,
            script_time: Duration::ZERO,
            physics_events: Vec::new(),
            current_tick_started: false,
        };
        if self.process_pending_frame(&mut pending) {
            self.debugger_paused = true;
            self.pending_debug_frame = Some(pending.clone());
        }
        self.frame_from_pending(&pending)
    }

    fn process_pending_frame(&mut self, pending: &mut PendingRuntimeFrame) -> bool {
        while pending.remaining_steps > 0 {
            if pending.current_tick_started {
                self.finish_script_tick(pending);
                pending.current_tick_started = false;
                pending.completed_steps = pending.completed_steps.saturating_add(1);
                pending.remaining_steps = pending.remaining_steps.saturating_sub(1);
                pending.next_tick = pending.next_tick.saturating_add(1);
                continue;
            }

            self.simulation_time = self.simulation_time.saturating_add(self.clock.step());
            let input_events = self.input.drain_through(self.simulation_time);
            let context = FixedUpdateContext {
                tick: pending.next_tick,
                delta: self.clock.step(),
                simulation_time: self.simulation_time,
                input_events: &input_events,
                physics_events: &self.pending_physics_events,
                input: &self.input,
            };
            let systems_started = Instant::now();
            for system in &mut self.fixed_systems {
                system(&mut self.world, &context);
            }
            pending.fixed_system_time = pending
                .fixed_system_time
                .saturating_add(systems_started.elapsed());
            let script_started = Instant::now();
            let hit = self.script_host.update_with_breakpoints(
                &mut self.world,
                self.input.states(),
                self.clock.step(),
                context.tick,
            );
            pending.script_time = pending.script_time.saturating_add(script_started.elapsed());
            if hit.is_some() {
                pending.current_tick_started = true;
                return true;
            }

            self.collect_scene_transition_requests();
            self.finish_script_tick(pending);
            pending.completed_steps = pending.completed_steps.saturating_add(1);
            pending.remaining_steps = pending.remaining_steps.saturating_sub(1);
            pending.next_tick = pending.next_tick.saturating_add(1);
        }
        false
    }

    fn collect_scene_transition_requests(&mut self) {
        let available = MAX_QUEUED_SCENE_TRANSITION_REQUESTS
            .saturating_sub(self.pending_scene_transitions.len());
        self.pending_scene_transitions.extend(
            self.script_host
                .take_scene_transition_requests()
                .into_iter()
                .take(available),
        );
    }

    fn finish_script_tick(&mut self, pending: &mut PendingRuntimeFrame) {
        let systems_started = Instant::now();
        self.advance_sprite_animations(self.clock.step());
        self.pending_physics_events = self
            .physics
            .step(&mut self.world, self.clock.step().as_secs_f32());
        pending
            .physics_events
            .extend(self.pending_physics_events.iter().copied());
        pending.fixed_system_time = pending
            .fixed_system_time
            .saturating_add(systems_started.elapsed());
    }

    fn frame_from_pending(&self, pending: &PendingRuntimeFrame) -> RuntimeFrame {
        RuntimeFrame {
            first_tick: pending.first_tick,
            fixed_steps: pending.completed_steps,
            interpolation_alpha: pending.interpolation_alpha,
            accepted_frame_time: pending.accepted_frame_time,
            dropped_time: pending.dropped_time,
            fixed_system_time: pending.fixed_system_time,
            script_time: pending.script_time,
            simulation_time: self.simulation_time,
            physics_events: pending.physics_events.clone(),
        }
    }

    pub fn set_script_breakpoint(
        &mut self,
        object_guid: Guid,
        line: u32,
        enabled: bool,
    ) -> Result<Option<u32>, ScriptError> {
        self.script_host.set_breakpoint(object_guid, line, enabled)
    }

    pub fn current_script_breakpoint(&self) -> Option<ScriptBreakpointHit> {
        self.script_host.current_breakpoint()
    }

    /// Resumes a suspended callback and completes its original fixed tick before advancing the
    /// remaining steps from the frame that reached the breakpoint.
    pub fn continue_script_breakpoint(&mut self) -> BreakpointContinueResult {
        if !self.debugger_paused {
            return BreakpointContinueResult::NotPaused;
        }
        let Some(mut pending) = self.pending_debug_frame.take() else {
            self.debugger_paused = false;
            return BreakpointContinueResult::NotPaused;
        };
        if let Some(hit) = self.script_host.continue_breakpoint(&mut self.world) {
            self.pending_debug_frame = Some(pending);
            return BreakpointContinueResult::Hit(hit);
        }
        self.collect_scene_transition_requests();
        if self.process_pending_frame(&mut pending) {
            let hit = self
                .script_host
                .current_breakpoint()
                .expect("script host paused while processing a pending fixed tick");
            self.pending_debug_frame = Some(pending);
            return BreakpointContinueResult::Hit(hit);
        }
        self.debugger_paused = false;
        BreakpointContinueResult::FrameComplete(self.frame_from_pending(&pending))
    }

    pub fn attach_script(&mut self, definition: ScriptDefinition) -> Result<(), ScriptError> {
        self.script_host.attach(definition, &mut self.world)
    }

    pub fn reload_script_asset(&mut self, asset: &LoadedScriptAsset) {
        self.script_host.reload_asset(asset);
    }

    pub fn update_script_properties(
        &mut self,
        object_guid: Guid,
        properties: BTreeMap<String, ScriptPropertyValue>,
    ) -> Result<(), ScriptError> {
        self.script_host.update_properties(object_guid, properties)
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

    pub fn take_audio_commands(&mut self) -> Vec<ScriptAudioCommand> {
        self.script_host.take_audio_commands()
    }

    /// Queues a UI event for delivery to script callbacks on the next fixed tick.
    pub fn emit_ui_event(&mut self, name: impl Into<String>, source_id: Guid) -> bool {
        let step_nanos = self.clock.step().as_nanos().max(1);
        let tick = u64::try_from(self.simulation_time.as_nanos() / step_nanos).unwrap_or(u64::MAX);
        self.script_host.queue_external_event(
            name,
            ScriptEventValue::String("clicked".into()),
            source_id,
            tick,
        )
    }

    pub fn take_scene_transition_requests(&mut self) -> Vec<Guid> {
        self.pending_scene_transitions.drain(..).collect()
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
