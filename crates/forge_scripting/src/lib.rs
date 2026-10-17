//! Bounded, deterministic Luau scripting isolated behind Forge-owned APIs.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    rc::Rc,
    str::FromStr,
    time::Duration,
};

use forge_assets::LoadedScriptAsset;
use forge_core::{ActionId, ActionState, Guid};
use forge_luau_debug::{self as luau_debug, BreakpointContext, BreakpointHit, ResumeResult};
use forge_scene::ScriptPropertyValue;
use forge_world::{RuntimeWorld, Transform2D};
use mlua::{
    Error as LuaError, Function, IntoLuaMulti, Lua, MultiValue, Table, Thread, Value as LuaValue,
    VmState,
};

const MAX_SCRIPT_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_VM_MEMORY_BYTES: usize = 4 * 1024 * 1024;
const MAX_SCRIPT_INSTANCES: usize = 64;
const MAX_INTERRUPT_CHECKPOINTS_PER_CALLBACK: usize = 20_000;
const MAX_SCRIPT_EVENT_NAME_BYTES: usize = 128;
const MAX_SCRIPT_EVENT_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_SCRIPT_EVENTS_PER_CALLBACK: usize = 64;
const MAX_QUEUED_SCRIPT_EVENTS: usize = 1024;
const MAX_AUDIO_COMMANDS_PER_CALLBACK: usize = 16;
const MAX_QUEUED_AUDIO_COMMANDS: usize = 512;
const MAX_AUDIO_GUID_BYTES: usize = 64;
const MAX_SCENE_GUID_BYTES: usize = 64;
const MAX_SCENE_REQUESTS_PER_CALLBACK: usize = 16;
const MAX_SCENE_REQUESTS_PER_TICK: usize = 64;
const MAX_UI_COMMANDS_PER_CALLBACK: usize = 32;
const MAX_QUEUED_UI_COMMANDS: usize = 512;
const MAX_UI_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub struct ScriptDefinition {
    pub object_guid: Guid,
    pub asset_guid: Guid,
    pub source_path: String,
    pub source: String,
    pub properties: BTreeMap<String, ScriptPropertyValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptDiagnostic {
    pub object_guid: Guid,
    pub asset_guid: Guid,
    pub source_path: String,
    pub tick: u64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptEventValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEvent {
    pub name: String,
    pub value: ScriptEventValue,
    pub sender_guid: Guid,
    pub tick: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptAudioCommand {
    PlayEffect { asset_guid: Guid, bus: String },
    PlayMusic { asset_guid: Guid, looping: bool },
    SetBusVolume { bus: String, decibels: f64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptUiCommand {
    SetText { element_guid: Guid, text: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptBreakpointHit {
    pub object_guid: Guid,
    pub asset_guid: Guid,
    pub source_path: String,
    pub line: u32,
    pub tick: u64,
}

#[derive(Debug)]
pub enum ScriptError {
    Lua(LuaError),
    MissingObject(Guid),
    TooManyInstances,
    SourceTooLarge(usize),
    MissingInstance(Guid),
    InvalidProperty(String),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lua(error) => write!(f, "Luau script failed: {error}"),
            Self::MissingObject(guid) => {
                write!(f, "script object {guid} is not in the runtime world")
            }
            Self::TooManyInstances => write!(
                f,
                "the scene exceeds the {MAX_SCRIPT_INSTANCES}-script limit"
            ),
            Self::SourceTooLarge(size) => write!(
                f,
                "script source is {size} bytes; maximum is {MAX_SCRIPT_SOURCE_BYTES}"
            ),
            Self::MissingInstance(guid) => {
                write!(f, "script instance for object {guid} was not found")
            }
            Self::InvalidProperty(name) => {
                write!(f, "script property {name:?} is invalid or changes type")
            }
        }
    }
}

impl Error for ScriptError {}

impl From<LuaError> for ScriptError {
    fn from(error: LuaError) -> Self {
        Self::Lua(error)
    }
}

#[derive(Clone)]
struct ScriptCallContext {
    transform: Transform2D,
    properties: BTreeMap<String, ScriptPropertyValue>,
    actions: BTreeMap<ActionId, ActionState>,
    object_guid: Guid,
    tick: u64,
    emitted_events: Vec<ScriptEvent>,
    audio_commands: Vec<ScriptAudioCommand>,
    scene_requests: Vec<Guid>,
    ui_commands: Vec<ScriptUiCommand>,
}

type SharedContext = Rc<RefCell<Option<ScriptCallContext>>>;
type SharedEvents = Rc<RefCell<Vec<ScriptEvent>>>;
type SharedAudioCommands = Rc<RefCell<Vec<ScriptAudioCommand>>>;
type SharedSceneRequests = Rc<RefCell<Vec<Guid>>>;
type SharedUiCommands = Rc<RefCell<Vec<ScriptUiCommand>>>;

struct ScriptInstance {
    _lua: Lua,
    source_function: Function,
    state: Table,
    on_start: Option<Function>,
    on_fixed_update: Option<Function>,
    on_event: Option<Function>,
    context: SharedContext,
    pending_events: SharedEvents,
    interrupt_checkpoints: Rc<Cell<usize>>,
    definition: ScriptDefinition,
    start_pending: bool,
    disabled: bool,
    breakpoint_lines: BTreeSet<u32>,
    breakpoint_context: BreakpointContext,
    suspended: Option<SuspendedCallback>,
}

struct SuspendedCallback {
    thread: Thread,
    context: ScriptCallContext,
}

enum ScriptInvocation {
    Completed(ScriptCallContext),
    Paused(BreakpointHit),
    Failed(LuaError),
}

impl ScriptInstance {
    fn compile(
        definition: ScriptDefinition,
        context: SharedContext,
        pending_events: SharedEvents,
        pending_audio_commands: SharedAudioCommands,
        pending_scene_requests: SharedSceneRequests,
        pending_ui_commands: SharedUiCommands,
    ) -> Result<Self, ScriptError> {
        if definition.source.len() > MAX_SCRIPT_SOURCE_BYTES {
            return Err(ScriptError::SourceTooLarge(definition.source.len()));
        }

        let lua = Lua::new();
        lua.set_memory_limit(MAX_VM_MEMORY_BYTES)?;
        luau_debug::install(&lua)?;
        let interrupt_checkpoints = Rc::new(Cell::new(0_usize));
        let checkpoints = interrupt_checkpoints.clone();
        lua.set_interrupt(move |_| {
            let count = checkpoints.get().saturating_add(1);
            checkpoints.set(count);
            if count > MAX_INTERRUPT_CHECKPOINTS_PER_CALLBACK {
                return Err(LuaError::RuntimeError(
                    "script execution budget exceeded".into(),
                ));
            }
            Ok(VmState::Continue)
        });

        install_forge_api(
            &lua,
            context.clone(),
            pending_events.clone(),
            pending_audio_commands,
            pending_scene_requests,
            pending_ui_commands,
        )?;
        lua.sandbox(true)?;

        let source_function: Function = lua
            .load(&definition.source)
            .set_name(&definition.source_path)
            .into_function()?;
        let state: Table = source_function.call(())?;
        install_object_api(&lua, &state, context.clone())?;
        let on_start: Option<Function> = state.get("on_start")?;
        let on_fixed_update: Option<Function> = state.get("on_fixed_update")?;
        let on_event: Option<Function> = state.get("on_event")?;

        Ok(Self {
            _lua: lua,
            source_function,
            state,
            on_start,
            on_fixed_update,
            on_event,
            context,
            pending_events,
            interrupt_checkpoints,
            definition,
            start_pending: true,
            disabled: false,
            breakpoint_lines: BTreeSet::new(),
            breakpoint_context: BreakpointContext::default(),
            suspended: None,
        })
    }

    fn invoke_callback(
        &mut self,
        callback: &Function,
        transform: Transform2D,
        actions: &BTreeMap<ActionId, ActionState>,
        tick: u64,
        args: MultiValue,
    ) -> ScriptInvocation {
        let fallback = ScriptCallContext {
            transform,
            properties: self.definition.properties.clone(),
            actions: actions.clone(),
            object_guid: self.definition.object_guid,
            tick,
            emitted_events: Vec::new(),
            audio_commands: Vec::new(),
            scene_requests: Vec::new(),
            ui_commands: Vec::new(),
        };
        *self.context.borrow_mut() = Some(fallback.clone());
        self.interrupt_checkpoints.set(0);

        if self.breakpoint_lines.is_empty() {
            let result = callback.call::<()>(args);
            let context = self.context.borrow_mut().take().unwrap_or(fallback);
            return match result {
                Ok(()) => ScriptInvocation::Completed(context),
                Err(error) => ScriptInvocation::Failed(error),
            };
        }

        let thread = match self._lua.create_thread(callback.clone()) {
            Ok(thread) => thread,
            Err(error) => {
                self.context.borrow_mut().take();
                return ScriptInvocation::Failed(error);
            }
        };
        match luau_debug::resume(&self._lua, &thread, args, &self.breakpoint_context) {
            Ok(ResumeResult::Complete) => {
                let context = self.context.borrow_mut().take().unwrap_or(fallback);
                ScriptInvocation::Completed(context)
            }
            Ok(ResumeResult::Paused(hit)) => {
                let context = self.context.borrow_mut().take().unwrap_or(fallback);
                self.suspended = Some(SuspendedCallback { thread, context });
                ScriptInvocation::Paused(hit)
            }
            Ok(ResumeResult::Failed(message)) => {
                self.context.borrow_mut().take();
                ScriptInvocation::Failed(LuaError::RuntimeError(message.into()))
            }
            Err(error) => {
                self.context.borrow_mut().take();
                ScriptInvocation::Failed(error)
            }
        }
    }

    fn resume_suspended(&mut self) -> ScriptInvocation {
        let Some(suspended) = self.suspended.take() else {
            return ScriptInvocation::Failed(LuaError::RuntimeError(
                "no suspended Luau callback is available".into(),
            ));
        };
        let fallback = suspended.context.clone();
        *self.context.borrow_mut() = Some(fallback.clone());
        match luau_debug::resume(
            &self._lua,
            &suspended.thread,
            MultiValue::default(),
            &self.breakpoint_context,
        ) {
            Ok(ResumeResult::Complete) => {
                let context = self.context.borrow_mut().take().unwrap_or(fallback);
                ScriptInvocation::Completed(context)
            }
            Ok(ResumeResult::Paused(hit)) => {
                let context = self.context.borrow_mut().take().unwrap_or(fallback);
                self.suspended = Some(SuspendedCallback {
                    thread: suspended.thread,
                    context,
                });
                ScriptInvocation::Paused(hit)
            }
            Ok(ResumeResult::Failed(message)) => {
                self.context.borrow_mut().take();
                ScriptInvocation::Failed(LuaError::RuntimeError(message.into()))
            }
            Err(error) => {
                self.context.borrow_mut().take();
                ScriptInvocation::Failed(error)
            }
        }
    }

    fn set_breakpoint(&mut self, line: u32, enabled: bool) -> Result<Option<u32>, LuaError> {
        let accepted_line =
            luau_debug::set_breakpoint(&self._lua, &self.source_function, line, enabled)?;
        if enabled && accepted_line.is_some() {
            self.breakpoint_lines.insert(line);
        } else if !enabled {
            self.breakpoint_lines.remove(&line);
        }
        Ok(accepted_line)
    }

    fn restore_breakpoints(&mut self, lines: &BTreeSet<u32>) -> Result<(), LuaError> {
        for line in lines {
            self.set_breakpoint(*line, true)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
enum ScriptCallbackKind {
    Start,
    Event(ScriptEvent),
    FixedUpdate,
}

struct QueuedScriptCallback {
    object_guid: Guid,
    kind: ScriptCallbackKind,
}

struct PendingScriptUpdate {
    actions: BTreeMap<ActionId, ActionState>,
    delta: Duration,
    tick: u64,
}

struct DeferredScriptReload {
    asset_guid: Guid,
    source_path: String,
    source: String,
}

/// Compiles Luau source without executing its top-level code.
pub fn validate_source(source: &str) -> Result<(), String> {
    if source.len() > MAX_SCRIPT_SOURCE_BYTES {
        return Err(format!(
            "script source is {} bytes; maximum supported size is {} bytes",
            source.len(),
            MAX_SCRIPT_SOURCE_BYTES
        ));
    }
    let lua = Lua::new();
    lua.set_memory_limit(MAX_VM_MEMORY_BYTES)
        .map_err(|error| error.to_string())?;
    lua.load(source)
        .into_function()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub struct ScriptHost {
    instances: BTreeMap<Guid, ScriptInstance>,
    diagnostics: Vec<ScriptDiagnostic>,
    pending_events: SharedEvents,
    pending_audio_commands: SharedAudioCommands,
    pending_scene_requests: SharedSceneRequests,
    pending_ui_commands: SharedUiCommands,
    pending_callbacks: VecDeque<QueuedScriptCallback>,
    pending_update: Option<PendingScriptUpdate>,
    current_breakpoint: Option<ScriptBreakpointHit>,
    deferred_reloads: BTreeMap<Guid, DeferredScriptReload>,
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptHost {
    pub fn new() -> Self {
        Self {
            instances: BTreeMap::new(),
            diagnostics: Vec::new(),
            pending_events: Rc::new(RefCell::new(Vec::new())),
            pending_audio_commands: Rc::new(RefCell::new(Vec::new())),
            pending_scene_requests: Rc::new(RefCell::new(Vec::new())),
            pending_ui_commands: Rc::new(RefCell::new(Vec::new())),
            pending_callbacks: VecDeque::new(),
            pending_update: None,
            current_breakpoint: None,
            deferred_reloads: BTreeMap::new(),
        }
    }

    pub fn attach(
        &mut self,
        definition: ScriptDefinition,
        world: &mut RuntimeWorld,
    ) -> Result<(), ScriptError> {
        if !self.instances.contains_key(&definition.object_guid)
            && self.instances.len() >= MAX_SCRIPT_INSTANCES
        {
            return Err(ScriptError::TooManyInstances);
        }
        if world.entity_for_guid(definition.object_guid).is_none() {
            return Err(ScriptError::MissingObject(definition.object_guid));
        }
        let context = Rc::new(RefCell::new(None));
        let instance = ScriptInstance::compile(
            definition.clone(),
            context,
            self.pending_events.clone(),
            self.pending_audio_commands.clone(),
            self.pending_scene_requests.clone(),
            self.pending_ui_commands.clone(),
        )?;
        self.instances.insert(definition.object_guid, instance);
        Ok(())
    }

    pub fn set_breakpoint(
        &mut self,
        object_guid: Guid,
        line: u32,
        enabled: bool,
    ) -> Result<Option<u32>, ScriptError> {
        let instance = self
            .instances
            .get_mut(&object_guid)
            .ok_or(ScriptError::MissingInstance(object_guid))?;
        instance
            .set_breakpoint(line, enabled)
            .map_err(ScriptError::Lua)
    }

    pub fn current_breakpoint(&self) -> Option<ScriptBreakpointHit> {
        self.current_breakpoint.clone()
    }

    /// Applies compatible authored values to a live script without resetting its Luau VM.
    pub fn update_properties(
        &mut self,
        object_guid: Guid,
        properties: BTreeMap<String, ScriptPropertyValue>,
    ) -> Result<(), ScriptError> {
        let instance = self
            .instances
            .get_mut(&object_guid)
            .ok_or(ScriptError::MissingInstance(object_guid))?;
        for (name, value) in &properties {
            if name.trim().is_empty()
                || name.len() > 128
                || matches!(value, ScriptPropertyValue::Number(number) if !number.is_finite())
                || matches!(value, ScriptPropertyValue::String(text) if text.len() > 16 * 1024)
                || instance
                    .definition
                    .properties
                    .get(name)
                    .is_some_and(|current| !same_property_type(current, value))
            {
                return Err(ScriptError::InvalidProperty(name.clone()));
            }
        }
        let old_properties = instance.definition.properties.clone();
        if let Some(suspended) = instance.suspended.as_mut() {
            suspended
                .context
                .properties
                .retain(|name, _| properties.contains_key(name));
            for (name, value) in &properties {
                if old_properties.get(name) != Some(value) {
                    suspended
                        .context
                        .properties
                        .insert(name.clone(), value.clone());
                }
            }
        }
        instance.definition.properties = properties;
        Ok(())
    }

    pub fn update(
        &mut self,
        world: &mut RuntimeWorld,
        actions: &BTreeMap<ActionId, ActionState>,
        delta: Duration,
        tick: u64,
    ) {
        let _ = self.update_with_breakpoints(world, actions, delta, tick);
    }

    pub fn update_with_breakpoints(
        &mut self,
        world: &mut RuntimeWorld,
        actions: &BTreeMap<ActionId, ActionState>,
        delta: Duration,
        tick: u64,
    ) -> Option<ScriptBreakpointHit> {
        if self.pending_update.is_some() {
            return self.current_breakpoint.clone();
        }
        self.pending_scene_requests.borrow_mut().clear();
        let pending_events = std::mem::take(&mut *self.pending_events.borrow_mut());
        let mut orphaned = Vec::new();
        for (object_guid, instance) in &mut self.instances {
            if world.entity_for_guid(*object_guid).is_none() {
                orphaned.push(*object_guid);
                continue;
            }
            if instance.disabled {
                continue;
            }
            if instance.start_pending {
                instance.start_pending = false;
                if instance.on_start.is_some() {
                    self.pending_callbacks.push_back(QueuedScriptCallback {
                        object_guid: *object_guid,
                        kind: ScriptCallbackKind::Start,
                    });
                }
            }
            if instance.on_event.is_some() {
                for event in &pending_events {
                    self.pending_callbacks.push_back(QueuedScriptCallback {
                        object_guid: *object_guid,
                        kind: ScriptCallbackKind::Event(event.clone()),
                    });
                }
            }
            if instance.on_fixed_update.is_some() {
                self.pending_callbacks.push_back(QueuedScriptCallback {
                    object_guid: *object_guid,
                    kind: ScriptCallbackKind::FixedUpdate,
                });
            }
        }
        for object_guid in orphaned {
            self.instances.remove(&object_guid);
        }
        self.pending_update = Some(PendingScriptUpdate {
            actions: actions.clone(),
            delta,
            tick,
        });
        self.process_pending_callbacks(world)
    }

    /// Continues the current callback and then finishes the rest of its fixed-tick script queue.
    pub fn continue_breakpoint(&mut self, world: &mut RuntimeWorld) -> Option<ScriptBreakpointHit> {
        let hit = self.current_breakpoint.take()?;
        let invocation = self
            .instances
            .get_mut(&hit.object_guid)
            .map(ScriptInstance::resume_suspended);
        if let Some(invocation) = invocation {
            if let Some(next_hit) =
                self.finish_invocation(hit.object_guid, invocation, world, hit.tick)
            {
                return Some(next_hit);
            }
        }
        self.process_pending_callbacks(world)
    }

    fn process_pending_callbacks(
        &mut self,
        world: &mut RuntimeWorld,
    ) -> Option<ScriptBreakpointHit> {
        while let Some(queued) = self.pending_callbacks.pop_front() {
            let Some(update) = self.pending_update.as_ref() else {
                break;
            };
            let actions = update.actions.clone();
            let delta = update.delta;
            let tick = update.tick;
            let invocation = {
                let Some(instance) = self.instances.get_mut(&queued.object_guid) else {
                    continue;
                };
                if instance.disabled {
                    continue;
                }
                let callback = match &queued.kind {
                    ScriptCallbackKind::Start => instance.on_start.clone(),
                    ScriptCallbackKind::Event(_) => instance.on_event.clone(),
                    ScriptCallbackKind::FixedUpdate => instance.on_fixed_update.clone(),
                };
                let Some(callback) = callback else {
                    continue;
                };
                match callback_arguments(instance, &queued.kind, delta) {
                    Ok(args) => {
                        let Some(entity) = world.entity_for_guid(queued.object_guid) else {
                            continue;
                        };
                        let Some(transform) = world.transform(entity) else {
                            continue;
                        };
                        instance.invoke_callback(&callback, transform, &actions, tick, args)
                    }
                    Err(error) => ScriptInvocation::Failed(error),
                }
            };
            if let Some(next_hit) =
                self.finish_invocation(queued.object_guid, invocation, world, tick)
            {
                return Some(next_hit);
            }
        }
        self.pending_update = None;
        self.apply_deferred_reloads();
        None
    }

    fn finish_invocation(
        &mut self,
        object_guid: Guid,
        invocation: ScriptInvocation,
        world: &mut RuntimeWorld,
        tick: u64,
    ) -> Option<ScriptBreakpointHit> {
        match invocation {
            ScriptInvocation::Completed(context) => {
                let audio_commands = context.audio_commands.clone();
                let scene_requests = context.scene_requests.clone();
                let ui_commands = context.ui_commands.clone();
                let (
                    instances,
                    diagnostics,
                    pending_audio_commands,
                    pending_scene_requests,
                    pending_ui_commands,
                ) = (
                    &mut self.instances,
                    &mut self.diagnostics,
                    &self.pending_audio_commands,
                    &self.pending_scene_requests,
                    &self.pending_ui_commands,
                );
                if let Some(instance) = instances.get_mut(&object_guid) {
                    if apply_callback_result(instance, world, Ok(context), tick, diagnostics) {
                        pending_audio_commands.borrow_mut().extend(audio_commands);
                        pending_scene_requests.borrow_mut().extend(scene_requests);
                        pending_ui_commands.borrow_mut().extend(ui_commands);
                    }
                }
                None
            }
            ScriptInvocation::Failed(error) => {
                let (instances, diagnostics) = (&mut self.instances, &mut self.diagnostics);
                if let Some(instance) = instances.get_mut(&object_guid) {
                    disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
                }
                None
            }
            ScriptInvocation::Paused(native_hit) => {
                let instance = self.instances.get(&object_guid)?;
                let hit = ScriptBreakpointHit {
                    object_guid,
                    asset_guid: instance.definition.asset_guid,
                    source_path: instance.definition.source_path.clone(),
                    line: native_hit.line,
                    tick,
                };
                self.current_breakpoint = Some(hit.clone());
                Some(hit)
            }
        }
    }

    /// Recompiles every active instance that references this source. Invalid updates leave the
    /// last working instance installed and produce a diagnostic.
    pub fn reload_asset(&mut self, asset: &LoadedScriptAsset) {
        let update = DeferredScriptReload {
            asset_guid: asset.guid,
            source_path: asset.relative_path.clone(),
            source: asset.source.clone(),
        };
        if self.current_breakpoint.is_some() {
            self.deferred_reloads.insert(update.asset_guid, update);
        } else {
            self.apply_script_reload(update);
        }
    }

    fn apply_deferred_reloads(&mut self) {
        let updates = std::mem::take(&mut self.deferred_reloads);
        for (_, update) in updates {
            self.apply_script_reload(update);
        }
    }

    fn apply_script_reload(&mut self, update: DeferredScriptReload) {
        let targets = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.definition.asset_guid == update.asset_guid)
            .map(|(object_guid, instance)| (*object_guid, instance.definition.clone()))
            .collect::<Vec<_>>();

        for (object_guid, mut definition) in targets {
            definition.source_path = update.source_path.clone();
            definition.source = update.source.clone();
            let context = Rc::new(RefCell::new(None));
            match ScriptInstance::compile(
                definition.clone(),
                context,
                self.pending_events.clone(),
                self.pending_audio_commands.clone(),
                self.pending_scene_requests.clone(),
                self.pending_ui_commands.clone(),
            ) {
                Ok(mut instance) => {
                    let migration = self.instances.get(&object_guid).map(|previous| {
                        migrate_compatible_state(&previous.state, &instance.state, &instance._lua)
                            .map(|warnings| {
                                (
                                    previous.start_pending,
                                    previous.breakpoint_lines.clone(),
                                    warnings,
                                )
                            })
                    });
                    match migration {
                        Some(Ok((start_pending, breakpoints, warnings))) => {
                            instance.start_pending = start_pending;
                            if let Err(error) = instance.restore_breakpoints(&breakpoints) {
                                self.record_diagnostic(
                                    object_guid,
                                    update.asset_guid,
                                    update.source_path.clone(),
                                    0,
                                    format!("breakpoints could not be restored after reload; the last-good script remains active: {error}"),
                                );
                                continue;
                            }
                            self.instances.insert(object_guid, instance);
                            for warning in warnings {
                                self.record_diagnostic(
                                    object_guid,
                                    update.asset_guid,
                                    update.source_path.clone(),
                                    0,
                                    warning,
                                );
                            }
                        }
                        Some(Err(error)) => self.record_diagnostic(
                            object_guid,
                            update.asset_guid,
                            update.source_path.clone(),
                            0,
                            format!("script state migration failed; the last-good script remains active: {error}"),
                        ),
                        None => {
                            self.instances.insert(object_guid, instance);
                        }
                    }
                }
                Err(error) => self.record_diagnostic(
                    object_guid,
                    update.asset_guid,
                    update.source_path.clone(),
                    0,
                    error.to_string(),
                ),
            }
        }
    }

    pub fn report_asset_issue(
        &mut self,
        asset_guid: Guid,
        source_path: impl Into<String>,
        message: impl Into<String>,
    ) {
        let source_path = source_path.into();
        let message = message.into();
        for instance in self
            .instances
            .values()
            .filter(|instance| instance.definition.asset_guid == asset_guid)
        {
            self.diagnostics.push(ScriptDiagnostic {
                object_guid: instance.definition.object_guid,
                asset_guid,
                source_path: source_path.clone(),
                tick: 0,
                message: message.clone(),
            });
        }
    }

    pub fn take_diagnostics(&mut self) -> Vec<ScriptDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    pub fn take_audio_commands(&mut self) -> Vec<ScriptAudioCommand> {
        std::mem::take(&mut *self.pending_audio_commands.borrow_mut())
    }

    /// Queues a bounded event from an external Forge-owned system for the next script tick.
    pub fn queue_external_event(
        &mut self,
        name: impl Into<String>,
        value: ScriptEventValue,
        sender_guid: Guid,
        tick: u64,
    ) -> bool {
        let name = name.into();
        let valid_value = match &value {
            ScriptEventValue::Number(number) => number.is_finite(),
            ScriptEventValue::String(text) => text.len() <= MAX_SCRIPT_EVENT_PAYLOAD_BYTES,
            _ => true,
        };
        if name.trim().is_empty() || name.len() > MAX_SCRIPT_EVENT_NAME_BYTES || !valid_value {
            return false;
        }
        let mut pending = self.pending_events.borrow_mut();
        if pending.len() >= MAX_QUEUED_SCRIPT_EVENTS {
            return false;
        }
        pending.push(ScriptEvent {
            name,
            value,
            sender_guid,
            tick,
        });
        true
    }

    pub fn take_scene_transition_requests(&mut self) -> Vec<Guid> {
        std::mem::take(&mut *self.pending_scene_requests.borrow_mut())
    }

    pub fn take_ui_commands(&mut self) -> Vec<ScriptUiCommand> {
        std::mem::take(&mut *self.pending_ui_commands.borrow_mut())
    }

    pub fn record_diagnostic(
        &mut self,
        object_guid: Guid,
        asset_guid: Guid,
        source_path: impl Into<String>,
        tick: u64,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(ScriptDiagnostic {
            object_guid,
            asset_guid,
            source_path: source_path.into(),
            tick,
            message: message.into(),
        });
    }
}

fn callback_arguments(
    instance: &ScriptInstance,
    kind: &ScriptCallbackKind,
    delta: Duration,
) -> Result<MultiValue, LuaError> {
    match kind {
        ScriptCallbackKind::Start => {
            (instance.state.clone(), 0.0_f64).into_lua_multi(&instance._lua)
        }
        ScriptCallbackKind::Event(event) => (
            instance.state.clone(),
            event.name.clone(),
            event_value_to_lua(&instance._lua, &event.value)?,
            event.sender_guid.to_string(),
            event.tick,
        )
            .into_lua_multi(&instance._lua),
        ScriptCallbackKind::FixedUpdate => {
            (instance.state.clone(), delta.as_secs_f64()).into_lua_multi(&instance._lua)
        }
    }
}

fn apply_callback_result(
    instance: &mut ScriptInstance,
    world: &mut RuntimeWorld,
    result: Result<ScriptCallContext, LuaError>,
    tick: u64,
    diagnostics: &mut Vec<ScriptDiagnostic>,
) -> bool {
    match result {
        Ok(context) => {
            if !transform_is_finite(context.transform) {
                disable_with_diagnostic(
                    instance,
                    tick,
                    "script produced a non-finite transform".into(),
                    diagnostics,
                );
                return false;
            }
            let Some(entity) = world.entity_for_guid(instance.definition.object_guid) else {
                return false;
            };
            if let Err(error) = world.set_transform(entity, context.transform) {
                disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
                return false;
            }
            instance.definition.properties = context.properties;
            instance
                .pending_events
                .borrow_mut()
                .extend(context.emitted_events);
            true
        }
        Err(error) => {
            disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
            false
        }
    }
}

fn disable_with_diagnostic(
    instance: &mut ScriptInstance,
    tick: u64,
    message: String,
    diagnostics: &mut Vec<ScriptDiagnostic>,
) {
    instance.disabled = true;
    diagnostics.push(ScriptDiagnostic {
        object_guid: instance.definition.object_guid,
        asset_guid: instance.definition.asset_guid,
        source_path: instance.definition.source_path.clone(),
        tick,
        message,
    });
}

fn migrate_compatible_state(
    previous: &Table,
    replacement: &Table,
    replacement_lua: &Lua,
) -> Result<Vec<String>, LuaError> {
    let mut diagnostics = Vec::new();
    for pair in previous.clone().pairs::<LuaValue, LuaValue>() {
        let (key, previous_value) = pair?;
        let LuaValue::String(key) = key else {
            continue;
        };
        let name = key.to_string_lossy();
        let replacement_value: LuaValue = replacement.get(name.as_str())?;
        let migrated_value = match (&previous_value, &replacement_value) {
            (LuaValue::Boolean(value), LuaValue::Boolean(_)) => Some(LuaValue::Boolean(*value)),
            (LuaValue::Integer(value), LuaValue::Integer(_)) => Some(LuaValue::Integer(*value)),
            (LuaValue::Number(value), LuaValue::Number(_)) => Some(LuaValue::Number(*value)),
            (LuaValue::String(value), LuaValue::String(_)) => Some(LuaValue::String(
                replacement_lua.create_string(value.to_string_lossy())?,
            )),
            (previous, _) if is_compatible_state_scalar(previous) => {
                diagnostics.push(format!(
                    "script state field {name:?} was reset because the replacement declares a different type or no compatible default"
                ));
                None
            }
            _ => None,
        };
        if let Some(value) = migrated_value {
            replacement.set(name.as_str(), value)?;
        }
    }
    Ok(diagnostics)
}

fn is_compatible_state_scalar(value: &LuaValue) -> bool {
    matches!(
        value,
        LuaValue::Boolean(_) | LuaValue::Integer(_) | LuaValue::Number(_) | LuaValue::String(_)
    )
}

fn install_forge_api(
    lua: &Lua,
    context: SharedContext,
    pending_events: SharedEvents,
    pending_audio_commands: SharedAudioCommands,
    pending_scene_requests: SharedSceneRequests,
    pending_ui_commands: SharedUiCommands,
) -> Result<(), LuaError> {
    let input = lua.create_table()?;

    let axis_context = context.clone();
    input.set(
        "axis",
        lua.create_function(move |_, action: u32| {
            Ok(active_context(&axis_context)?
                .actions
                .get(&ActionId(action))
                .map(|state| f64::from(state.value))
                .unwrap_or(0.0))
        })?,
    )?;

    let pressed_context = context.clone();
    input.set(
        "is_pressed",
        lua.create_function(move |_, action: u32| {
            Ok(active_context(&pressed_context)?
                .actions
                .get(&ActionId(action))
                .is_some_and(|state| state.pressed))
        })?,
    )?;

    let events = lua.create_table()?;
    let event_context = context.clone();
    let queued_events = pending_events;
    events.set(
        "emit",
        lua.create_function(move |_, (name, value): (String, Option<LuaValue>)| {
            if name.trim().is_empty() || name.len() > MAX_SCRIPT_EVENT_NAME_BYTES {
                return Ok(false);
            }
            let Some(value) = event_value_from_lua(value.unwrap_or(LuaValue::Nil)) else {
                return Ok(false);
            };
            let mut slot = event_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.emitted_events.len() >= MAX_SCRIPT_EVENTS_PER_CALLBACK
                || queued_events
                    .borrow()
                    .len()
                    .saturating_add(context.emitted_events.len())
                    >= MAX_QUEUED_SCRIPT_EVENTS
            {
                return Ok(false);
            }
            context.emitted_events.push(ScriptEvent {
                name,
                value,
                sender_guid: context.object_guid.clone(),
                tick: context.tick,
            });
            Ok(true)
        })?,
    )?;

    let audio = lua.create_table()?;

    let play_context = context.clone();
    let play_pending = pending_audio_commands.clone();
    audio.set(
        "play",
        lua.create_function(move |_, (guid, bus): (String, Option<String>)| {
            let bus = bus.unwrap_or_else(|| "sfx".to_owned());
            if guid.len() > MAX_AUDIO_GUID_BYTES || !matches!(bus.as_str(), "master" | "sfx" | "ui")
            {
                return Ok(false);
            }
            let Ok(asset_guid) = Guid::from_str(&guid) else {
                return Ok(false);
            };
            let mut slot = play_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.audio_commands.len() >= MAX_AUDIO_COMMANDS_PER_CALLBACK
                || play_pending
                    .borrow()
                    .len()
                    .saturating_add(context.audio_commands.len())
                    >= MAX_QUEUED_AUDIO_COMMANDS
            {
                return Ok(false);
            }
            context
                .audio_commands
                .push(ScriptAudioCommand::PlayEffect { asset_guid, bus });
            Ok(true)
        })?,
    )?;

    let music_context = context.clone();
    let music_pending = pending_audio_commands.clone();
    audio.set(
        "play_music",
        lua.create_function(move |_, (guid, looping): (String, Option<bool>)| {
            if guid.len() > MAX_AUDIO_GUID_BYTES {
                return Ok(false);
            }
            let Ok(asset_guid) = Guid::from_str(&guid) else {
                return Ok(false);
            };
            let mut slot = music_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.audio_commands.len() >= MAX_AUDIO_COMMANDS_PER_CALLBACK
                || music_pending
                    .borrow()
                    .len()
                    .saturating_add(context.audio_commands.len())
                    >= MAX_QUEUED_AUDIO_COMMANDS
            {
                return Ok(false);
            }
            context.audio_commands.push(ScriptAudioCommand::PlayMusic {
                asset_guid,
                looping: looping.unwrap_or(true),
            });
            Ok(true)
        })?,
    )?;

    let volume_context = context.clone();
    let volume_pending = pending_audio_commands;
    audio.set(
        "set_bus_volume",
        lua.create_function(move |_, (bus, decibels): (String, f64)| {
            if bus.len() > 16
                || !matches!(bus.as_str(), "master" | "music" | "sfx" | "ui")
                || !decibels.is_finite()
                || !(-60.0..=6.0).contains(&decibels)
            {
                return Ok(false);
            }
            let mut slot = volume_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.audio_commands.len() >= MAX_AUDIO_COMMANDS_PER_CALLBACK
                || volume_pending
                    .borrow()
                    .len()
                    .saturating_add(context.audio_commands.len())
                    >= MAX_QUEUED_AUDIO_COMMANDS
            {
                return Ok(false);
            }
            context
                .audio_commands
                .push(ScriptAudioCommand::SetBusVolume { bus, decibels });
            Ok(true)
        })?,
    )?;

    let scene = lua.create_table()?;
    let scene_context = context.clone();
    let scene_queue = pending_scene_requests;
    scene.set(
        "load",
        lua.create_function(move |_, guid: String| {
            if guid.len() > MAX_SCENE_GUID_BYTES {
                return Ok(false);
            }
            let Ok(scene_guid) = Guid::from_str(&guid) else {
                return Ok(false);
            };
            let mut slot = scene_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.scene_requests.len() >= MAX_SCENE_REQUESTS_PER_CALLBACK
                || scene_queue
                    .borrow()
                    .len()
                    .saturating_add(context.scene_requests.len())
                    >= MAX_SCENE_REQUESTS_PER_TICK
            {
                return Ok(false);
            }
            context.scene_requests.push(scene_guid);
            Ok(true)
        })?,
    )?;

    let ui = lua.create_table()?;
    let ui_context = context.clone();
    let ui_queue = pending_ui_commands;
    ui.set(
        "set_text",
        lua.create_function(move |_, (guid, text): (String, String)| {
            if guid.len() > MAX_SCENE_GUID_BYTES
                || text.len() > MAX_UI_TEXT_BYTES
                || text.contains('\0')
            {
                return Ok(false);
            }
            let Ok(element_guid) = Guid::from_str(&guid) else {
                return Ok(false);
            };
            let mut slot = ui_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            if context.ui_commands.len() >= MAX_UI_COMMANDS_PER_CALLBACK
                || ui_queue
                    .borrow()
                    .len()
                    .saturating_add(context.ui_commands.len())
                    >= MAX_QUEUED_UI_COMMANDS
            {
                return Ok(false);
            }
            context
                .ui_commands
                .push(ScriptUiCommand::SetText { element_guid, text });
            Ok(true)
        })?,
    )?;

    let forge = lua.create_table()?;
    forge.set("Input", input)?;
    forge.set("Events", events)?;
    forge.set("Audio", audio)?;
    forge.set("Scene", scene)?;
    forge.set("UI", ui)?;
    lua.globals().set("Forge", forge)?;
    Ok(())
}

fn install_object_api(lua: &Lua, state: &Table, context: SharedContext) -> Result<(), LuaError> {
    let guid_context = context.clone();
    state.set(
        "get_guid",
        lua.create_function(move |_, _self: Table| {
            Ok(active_context(&guid_context)?.object_guid.to_string())
        })?,
    )?;

    let getter_context = context.clone();
    state.set(
        "get",
        lua.create_function(move |lua, (_self, name): (Table, String)| {
            let context = active_context(&getter_context)?;
            match context.properties.get(&name) {
                Some(value) => property_to_lua(lua, value),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;

    let setter_context = context.clone();
    state.set(
        "set",
        lua.create_function(move |_, (_self, name, value): (Table, String, LuaValue)| {
            let updated = property_from_lua(value)?;
            let mut slot = setter_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            let current = context.properties.get(&name).ok_or_else(|| {
                LuaError::RuntimeError(format!("script property {name:?} is not exposed"))
            })?;
            if !same_property_type(current, &updated) {
                return Err(LuaError::RuntimeError(format!(
                    "script property {name:?} cannot change type"
                )));
            }
            context.properties.insert(name, updated);
            Ok(())
        })?,
    )?;

    let position_context = context.clone();
    state.set(
        "get_position",
        lua.create_function(move |_, _self: Table| {
            let context = active_context(&position_context)?;
            Ok((
                f64::from(context.transform.translation[0]),
                f64::from(context.transform.translation[1]),
            ))
        })?,
    )?;

    let set_position_context = context.clone();
    state.set(
        "set_position",
        lua.create_function(move |_, (_self, x, y): (Table, f64, f64)| {
            if !x.is_finite() || !y.is_finite() {
                return Err(LuaError::RuntimeError(
                    "script position must be finite".into(),
                ));
            }
            let mut slot = set_position_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            context.transform.translation = [x as f32, y as f32];
            Ok(())
        })?,
    )?;

    let translate_context = context;
    state.set(
        "translate",
        lua.create_function(move |_, (_self, x, y): (Table, f64, f64)| {
            if !x.is_finite() || !y.is_finite() {
                return Err(LuaError::RuntimeError(
                    "script translation must be finite".into(),
                ));
            }
            let mut slot = translate_context.borrow_mut();
            let context = slot.as_mut().ok_or_else(inactive_context_error)?;
            context.transform.translation[0] += x as f32;
            context.transform.translation[1] += y as f32;
            Ok(())
        })?,
    )?;

    Ok(())
}

fn event_value_from_lua(value: LuaValue) -> Option<ScriptEventValue> {
    match value {
        LuaValue::Nil => Some(ScriptEventValue::Nil),
        LuaValue::Boolean(value) => Some(ScriptEventValue::Boolean(value)),
        LuaValue::Integer(value) => Some(ScriptEventValue::Integer(value)),
        LuaValue::Number(value) if value.is_finite() => Some(ScriptEventValue::Number(value)),
        LuaValue::String(value) => {
            let value = value.to_str().ok()?.to_string();
            (value.len() <= MAX_SCRIPT_EVENT_PAYLOAD_BYTES)
                .then_some(ScriptEventValue::String(value))
        }
        _ => None,
    }
}

fn event_value_to_lua(lua: &Lua, value: &ScriptEventValue) -> Result<LuaValue, LuaError> {
    match value {
        ScriptEventValue::Nil => Ok(LuaValue::Nil),
        ScriptEventValue::Boolean(value) => Ok(LuaValue::Boolean(*value)),
        ScriptEventValue::Integer(value) => Ok(LuaValue::Integer(*value)),
        ScriptEventValue::Number(value) => Ok(LuaValue::Number(*value)),
        ScriptEventValue::String(value) => Ok(LuaValue::String(lua.create_string(value)?)),
    }
}

fn active_context(
    context: &SharedContext,
) -> Result<std::cell::Ref<'_, ScriptCallContext>, LuaError> {
    std::cell::Ref::filter_map(context.borrow(), Option::as_ref)
        .map_err(|_| inactive_context_error())
}

fn inactive_context_error() -> LuaError {
    LuaError::RuntimeError("Forge object API used outside a script callback".into())
}

fn property_to_lua(lua: &Lua, value: &ScriptPropertyValue) -> Result<LuaValue, LuaError> {
    match value {
        ScriptPropertyValue::Bool(value) => Ok(LuaValue::Boolean(*value)),
        ScriptPropertyValue::Number(value) => Ok(LuaValue::Number(*value)),
        ScriptPropertyValue::String(value) => Ok(LuaValue::String(lua.create_string(value)?)),
    }
}

fn property_from_lua(value: LuaValue) -> Result<ScriptPropertyValue, LuaError> {
    match value {
        LuaValue::Boolean(value) => Ok(ScriptPropertyValue::Bool(value)),
        LuaValue::Number(value) if value.is_finite() => Ok(ScriptPropertyValue::Number(value)),
        LuaValue::Integer(value) => Ok(ScriptPropertyValue::Number(value as f64)),
        LuaValue::String(value) => Ok(ScriptPropertyValue::String(value.to_str()?.to_string())),
        _ => Err(LuaError::RuntimeError(
            "script properties support only booleans, finite numbers, and strings".into(),
        )),
    }
}

fn same_property_type(left: &ScriptPropertyValue, right: &ScriptPropertyValue) -> bool {
    matches!(
        (left, right),
        (ScriptPropertyValue::Bool(_), ScriptPropertyValue::Bool(_))
            | (
                ScriptPropertyValue::Number(_),
                ScriptPropertyValue::Number(_)
            )
            | (
                ScriptPropertyValue::String(_),
                ScriptPropertyValue::String(_)
            )
    )
}

fn transform_is_finite(transform: Transform2D) -> bool {
    transform.translation.iter().all(|value| value.is_finite())
        && transform.scale.iter().all(|value| value.is_finite())
        && transform.rotation_radians.is_finite()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use forge_assets::LoadedScriptAsset;
    use forge_core::{ActionId, ActionState, Guid};
    use forge_scene::ScriptPropertyValue;
    use forge_world::{RuntimeWorld, Transform2D};

    use super::{ScriptAudioCommand, ScriptDefinition, ScriptHost, ScriptUiCommand};

    #[test]
    fn fixed_update_changes_only_its_authored_object_and_reads_properties() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Move.luau".into(),
            source: r#"
                return {
                    on_fixed_update = function(self, dt)
                        self:translate(self:get("speed") * dt, -dt)
                    end,
                }
            "#
            .into(),
            properties: BTreeMap::from([("speed".into(), ScriptPropertyValue::Number(4.0))]),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(50), 1);

        let transform = world.transform(entity).expect("object transform");
        assert!((transform.translation[0] - 0.2).abs() < 0.0001);
        assert!((transform.translation[1] + 0.05).abs() < 0.0001);
    }

    #[test]
    fn updating_script_properties_changes_the_next_tick_without_recompiling() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Move.luau".into(),
            source: r#"
                return {
                    on_fixed_update = function(self, dt)
                        self:translate(self:get("speed") * dt, 0)
                    end,
                }
            "#
            .into(),
            properties: BTreeMap::from([("speed".into(), ScriptPropertyValue::Number(1.0))]),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");
        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(100), 1);

        host.update_properties(
            object_guid,
            BTreeMap::from([("speed".into(), ScriptPropertyValue::Number(2.0))]),
        )
        .expect("apply compatible property value");
        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(100), 2);

        let position = world
            .transform(entity)
            .expect("scripted object transform")
            .translation[0];
        assert!((position - 0.3).abs() < 0.0001);
    }

    #[test]
    fn live_property_update_rejects_type_changes_without_replacing_values() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Move.luau".into(),
            source: "return {}".into(),
            properties: BTreeMap::from([("speed".into(), ScriptPropertyValue::Number(1.0))]),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");

        let result = host.update_properties(
            object_guid,
            BTreeMap::from([("speed".into(), ScriptPropertyValue::Bool(true))]),
        );

        assert!(matches!(
            result,
            Err(super::ScriptError::InvalidProperty(_))
        ));
    }

    #[test]
    fn runaway_script_is_disabled_and_reported_without_stopping_the_world() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Runaway.luau".into(),
            source: "return { on_fixed_update = function() while true do end end }".into(),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("compile runaway script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 7);

        let diagnostics = host.take_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].object_guid, object_guid);
        assert_eq!(diagnostics[0].tick, 7);
        assert!(diagnostics[0].message.contains("budget"));
        assert_eq!(
            world
                .transform(entity)
                .expect("object transform")
                .translation,
            [0.0, 0.0]
        );
    }

    #[test]
    fn hot_reload_keeps_last_good_code_when_the_replacement_is_invalid() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let asset_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let mut host = ScriptHost::new();
        host.attach(
            ScriptDefinition {
                object_guid,
                asset_guid,
                source_path: "Assets/Move.luau".into(),
                source: "return { on_fixed_update = function(self, dt) self:translate(dt, 0) end }"
                    .into(),
                properties: BTreeMap::new(),
            },
            &mut world,
        )
        .expect("attach original script");

        host.reload_asset(&LoadedScriptAsset {
            guid: asset_guid,
            relative_path: "Assets/Move.luau".into(),
            source: "return { on_fixed_update = function(self, dt) self:translate(dt * 2, 0) end }"
                .into(),
        });
        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(50), 1);
        let first_position = world
            .transform(entity)
            .expect("object transform")
            .translation[0];
        assert!((first_position - 0.1).abs() < 0.0001);

        host.reload_asset(&LoadedScriptAsset {
            guid: asset_guid,
            relative_path: "Assets/Move.luau".into(),
            source: "return {".into(),
        });
        let diagnostics = host.take_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Luau script failed"));

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(50), 2);
        let second_position = world
            .transform(entity)
            .expect("object transform")
            .translation[0];
        assert!((second_position - 0.2).abs() < 0.0001);
    }

    #[test]
    fn input_api_is_tick_scoped_and_uses_numeric_action_ids() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Input.luau".into(),
            source: r#"
                return {
                    on_fixed_update = function(self, dt)
                        self:set_position(Forge.Input.axis(3) * dt, 0)
                    end,
                }
            "#
            .into(),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");
        let actions = BTreeMap::from([(
            ActionId(3),
            ActionState {
                value: 2.0,
                pressed: true,
            },
        )]);

        host.update(&mut world, &actions, Duration::from_millis(100), 1);

        let entity = world.entity_for_guid(object_guid).expect("runtime object");
        let transform = world.transform(entity).expect("object transform");
        assert!((transform.translation[0] - 0.2).abs() < 0.0001);
    }
    #[test]
    fn audio_commands_are_staged_in_callback_order() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let asset_guid = Guid::new();
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Audio.luau".into(),
            source: format!(
                r#"return {{ on_fixed_update = function()
                    Forge.Audio.play("{asset_guid}", "sfx")
                    Forge.Audio.play_music("{asset_guid}", true)
                    Forge.Audio.set_bus_volume("music", -6)
                end }}"#
            ),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach audio script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1);

        assert_eq!(
            host.take_audio_commands(),
            vec![
                ScriptAudioCommand::PlayEffect {
                    asset_guid,
                    bus: "sfx".into(),
                },
                ScriptAudioCommand::PlayMusic {
                    asset_guid,
                    looping: true,
                },
                ScriptAudioCommand::SetBusVolume {
                    bus: "music".into(),
                    decibels: -6.0,
                },
            ]
        );
    }

    #[test]
    fn audio_commands_are_discarded_when_the_script_callback_fails() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let audio_guid = Guid::new();
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Audio.luau".into(),
            source: format!(
                r#"return {{ on_fixed_update = function()
                    Forge.Audio.play("{audio_guid}", "sfx")
                    error("rollback audio command")
                end }}"#
            ),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach audio script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1);

        assert!(host.take_audio_commands().is_empty());
        assert_eq!(host.take_diagnostics().len(), 1);
    }

    #[test]
    fn ui_text_commands_commit_only_after_successful_callbacks() {
        let mut world = RuntimeWorld::new();
        let first_object = Guid::new();
        let failed_object = Guid::new();
        world
            .spawn(first_object, Transform2D::default(), None)
            .expect("spawn first scripted object");
        world
            .spawn(failed_object, Transform2D::default(), None)
            .expect("spawn failing scripted object");
        let element_guid = Guid::new();
        let mut host = ScriptHost::new();
        host.attach(
            ScriptDefinition {
                object_guid: first_object,
                asset_guid: Guid::new(),
                source_path: "Assets/UiCommand.luau".into(),
                source: format!(
                    r#"return {{ on_fixed_update = function()
                        assert(Forge.UI.set_text("{element_guid}", "SCORE 7"))
                    end }}"#
                ),
                properties: BTreeMap::new(),
            },
            &mut world,
        )
        .expect("attach command script");
        host.attach(
            ScriptDefinition {
                object_guid: failed_object,
                asset_guid: Guid::new(),
                source_path: "Assets/FailingUiCommand.luau".into(),
                source: format!(
                    r#"return {{ on_fixed_update = function()
                        Forge.UI.set_text("{element_guid}", "SCORE 99")
                        error("rollback UI command")
                    end }}"#
                ),
                properties: BTreeMap::new(),
            },
            &mut world,
        )
        .expect("attach failing command script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1);

        assert_eq!(
            host.take_ui_commands(),
            vec![ScriptUiCommand::SetText {
                element_guid,
                text: "SCORE 7".into(),
            }]
        );
        assert_eq!(host.take_diagnostics().len(), 1);
    }

    #[test]
    fn scene_requests_preserve_callback_order() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let first_scene = Guid::new();
        let second_scene = Guid::new();
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/SceneRequest.luau".into(),
            source: format!(
                r#"return {{ on_fixed_update = function()
                    assert(Forge.Scene.load("{first_scene}"))
                    assert(Forge.Scene.load("{second_scene}"))
                end }}"#
            ),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach scene request script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1);

        assert_eq!(
            host.take_scene_transition_requests(),
            vec![first_scene, second_scene]
        );
    }

    #[test]
    fn scene_requests_are_discarded_when_the_script_callback_fails() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let scene_guid = Guid::new();
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/SceneRequest.luau".into(),
            source: format!(
                r#"return {{ on_fixed_update = function()
                    Forge.Scene.load("{scene_guid}")
                    error("rollback scene request")
                end }}"#
            ),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach scene request script");

        host.update(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1);

        assert!(host.take_scene_transition_requests().is_empty());
        assert_eq!(host.take_diagnostics().len(), 1);
    }

    #[test]
    fn breakpoint_pauses_inside_nested_fixed_update_and_resumes_that_tick() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Breakpoint.luau".into(),
            source: r#"return {
    on_fixed_update = function(self, dt)
        self:translate(1.0, 0.0)
    end,
}
"#
            .into(),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");
        assert_eq!(
            host.set_breakpoint(object_guid, 3, true)
                .expect("set callback breakpoint"),
            Some(3)
        );

        let hit = host
            .update_with_breakpoints(&mut world, &BTreeMap::new(), Duration::from_millis(16), 1)
            .expect("nested fixed update should pause at line 3");
        assert_eq!(hit.object_guid, object_guid);
        assert_eq!(hit.source_path, "Assets/Breakpoint.luau");
        assert_eq!(hit.line, 3);
        assert_eq!(hit.tick, 1);
        assert_eq!(
            world
                .transform(entity)
                .expect("object transform")
                .translation[0],
            0.0
        );

        assert!(host.continue_breakpoint(&mut world).is_none());
        assert_eq!(
            world
                .transform(entity)
                .expect("object transform")
                .translation[0],
            1.0
        );
    }
}

#[cfg(test)]
mod external_event_tests {
    use super::*;

    #[test]
    fn external_events_are_validated_and_queue_capacity_is_bounded() {
        let mut host = ScriptHost::new();
        let sender = Guid::new();
        assert!(host.queue_external_event(
            "menu.start",
            ScriptEventValue::String("clicked".into()),
            sender,
            7
        ));
        assert!(!host.queue_external_event(" ", ScriptEventValue::Nil, sender, 7));
        assert!(!host.queue_external_event(
            "menu.invalid",
            ScriptEventValue::Number(f64::NAN),
            sender,
            7
        ));
        for index in 1..MAX_QUEUED_SCRIPT_EVENTS {
            assert!(host.queue_external_event(
                format!("menu.event.{index}"),
                ScriptEventValue::Nil,
                sender,
                7
            ));
        }
        assert!(!host.queue_external_event("menu.overflow", ScriptEventValue::Nil, sender, 7));
    }
}
