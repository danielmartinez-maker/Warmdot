//! Bounded, deterministic Luau scripting isolated behind Forge-owned APIs.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    error::Error,
    fmt,
    rc::Rc,
    time::Duration,
};

use forge_assets::LoadedScriptAsset;
use forge_core::{ActionId, ActionState, Guid};
use forge_scene::ScriptPropertyValue;
use forge_world::{RuntimeWorld, Transform2D};
use mlua::{Error as LuaError, Function, Lua, Table, Value as LuaValue, VmState};

const MAX_SCRIPT_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_VM_MEMORY_BYTES: usize = 4 * 1024 * 1024;
const MAX_SCRIPT_INSTANCES: usize = 64;
const MAX_INTERRUPT_CHECKPOINTS_PER_CALLBACK: usize = 20_000;
const MAX_SCRIPT_EVENT_NAME_BYTES: usize = 128;
const MAX_SCRIPT_EVENT_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_SCRIPT_EVENTS_PER_CALLBACK: usize = 64;
const MAX_QUEUED_SCRIPT_EVENTS: usize = 1024;

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
}

type SharedContext = Rc<RefCell<Option<ScriptCallContext>>>;
type SharedEvents = Rc<RefCell<Vec<ScriptEvent>>>;

struct ScriptInstance {
    _lua: Lua,
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
}

impl ScriptInstance {
    fn compile(
        definition: ScriptDefinition,
        context: SharedContext,
        pending_events: SharedEvents,
    ) -> Result<Self, ScriptError> {
        if definition.source.len() > MAX_SCRIPT_SOURCE_BYTES {
            return Err(ScriptError::SourceTooLarge(definition.source.len()));
        }

        let lua = Lua::new();
        lua.set_memory_limit(MAX_VM_MEMORY_BYTES)?;
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

        install_forge_api(&lua, context.clone(), pending_events.clone())?;
        lua.sandbox(true)?;

        let state: Table = lua
            .load(&definition.source)
            .set_name(&definition.source_path)
            .eval()?;
        install_object_api(&lua, &state, context.clone())?;
        let on_start: Option<Function> = state.get("on_start")?;
        let on_fixed_update: Option<Function> = state.get("on_fixed_update")?;
        let on_event: Option<Function> = state.get("on_event")?;

        Ok(Self {
            _lua: lua,
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
        })
    }

    fn invoke<F>(
        &mut self,
        transform: Transform2D,
        actions: &BTreeMap<ActionId, ActionState>,
        tick: u64,
        call: F,
    ) -> Result<ScriptCallContext, LuaError>
    where
        F: FnOnce(&Lua, &Table) -> Result<(), LuaError>,
    {
        *self.context.borrow_mut() = Some(ScriptCallContext {
            transform,
            properties: self.definition.properties.clone(),
            actions: actions.clone(),
            object_guid: self.definition.object_guid,
            tick,
            emitted_events: Vec::new(),
        });
        self.interrupt_checkpoints.set(0);
        let result = call(&self._lua, &self.state);
        let context = self
            .context
            .borrow_mut()
            .take()
            .unwrap_or_else(|| ScriptCallContext {
                transform,
                properties: self.definition.properties.clone(),
                actions: actions.clone(),
                object_guid: self.definition.object_guid,
                tick,
                emitted_events: Vec::new(),
            });
        result?;
        Ok(context)
    }
}

pub struct ScriptHost {
    instances: BTreeMap<Guid, ScriptInstance>,
    diagnostics: Vec<ScriptDiagnostic>,
    pending_events: SharedEvents,
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
        let instance =
            ScriptInstance::compile(definition.clone(), context, self.pending_events.clone())?;
        self.instances.insert(definition.object_guid, instance);
        Ok(())
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
                if let Some(callback) = instance.on_start.clone() {
                    run_callback(
                        instance,
                        &callback,
                        world,
                        actions,
                        Duration::ZERO,
                        tick,
                        &mut self.diagnostics,
                    );
                }
            }
            if !instance.disabled {
                if let Some(callback) = instance.on_event.clone() {
                    for event in &pending_events {
                        run_event_callback(
                            instance,
                            &callback,
                            event,
                            world,
                            actions,
                            tick,
                            &mut self.diagnostics,
                        );
                        if instance.disabled {
                            break;
                        }
                    }
                }
            }
            if !instance.disabled {
                if let Some(callback) = instance.on_fixed_update.clone() {
                    run_callback(
                        instance,
                        &callback,
                        world,
                        actions,
                        delta,
                        tick,
                        &mut self.diagnostics,
                    );
                }
            }
        }
        for object_guid in orphaned {
            self.instances.remove(&object_guid);
        }
    }

    /// Recompiles every active instance that references this source. Invalid updates leave the
    /// last working instance installed and produce a diagnostic.
    pub fn reload_asset(&mut self, asset: &LoadedScriptAsset) {
        let targets = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.definition.asset_guid == asset.guid)
            .map(|(object_guid, instance)| (*object_guid, instance.definition.clone()))
            .collect::<Vec<_>>();

        for (object_guid, mut definition) in targets {
            definition.source_path = asset.relative_path.clone();
            definition.source = asset.source.clone();
            let context = Rc::new(RefCell::new(None));
            match ScriptInstance::compile(definition.clone(), context, self.pending_events.clone())
            {
                Ok(mut instance) => {
                    let migration = self.instances.get(&object_guid).map(|previous| {
                        migrate_compatible_state(&previous.state, &instance.state, &instance._lua)
                            .map(|warnings| (previous.start_pending, warnings))
                    });
                    match migration {
                        Some(Ok((start_pending, warnings))) => {
                            instance.start_pending = start_pending;
                            self.instances.insert(object_guid, instance);
                            for warning in warnings {
                                self.record_diagnostic(
                                    object_guid,
                                    asset.guid,
                                    asset.relative_path.clone(),
                                    0,
                                    warning,
                                );
                            }
                        }
                        Some(Err(error)) => self.record_diagnostic(
                            object_guid,
                            asset.guid,
                            asset.relative_path.clone(),
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
                    asset.guid,
                    asset.relative_path.clone(),
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

fn run_callback(
    instance: &mut ScriptInstance,
    callback: &Function,
    world: &mut RuntimeWorld,
    actions: &BTreeMap<ActionId, ActionState>,
    delta: Duration,
    tick: u64,
    diagnostics: &mut Vec<ScriptDiagnostic>,
) {
    let Some(entity) = world.entity_for_guid(instance.definition.object_guid) else {
        return;
    };
    let Some(transform) = world.transform(entity) else {
        return;
    };

    let result = instance.invoke(transform, actions, tick, |_, state| {
        callback.call::<()>((state.clone(), delta.as_secs_f64()))
    });
    apply_callback_result(instance, world, result, tick, diagnostics);
}

fn run_event_callback(
    instance: &mut ScriptInstance,
    callback: &Function,
    event: &ScriptEvent,
    world: &mut RuntimeWorld,
    actions: &BTreeMap<ActionId, ActionState>,
    tick: u64,
    diagnostics: &mut Vec<ScriptDiagnostic>,
) {
    let Some(entity) = world.entity_for_guid(instance.definition.object_guid) else {
        return;
    };
    let Some(transform) = world.transform(entity) else {
        return;
    };

    let result = instance.invoke(transform, actions, tick, |lua, state| {
        callback.call::<()>((
            state.clone(),
            event.name.clone(),
            event_value_to_lua(lua, &event.value)?,
            event.sender_guid.to_string(),
            event.tick,
        ))
    });
    apply_callback_result(instance, world, result, tick, diagnostics);
}

fn apply_callback_result(
    instance: &mut ScriptInstance,
    world: &mut RuntimeWorld,
    result: Result<ScriptCallContext, LuaError>,
    tick: u64,
    diagnostics: &mut Vec<ScriptDiagnostic>,
) {
    match result {
        Ok(context) => {
            if !transform_is_finite(context.transform) {
                disable_with_diagnostic(
                    instance,
                    tick,
                    "script produced a non-finite transform".into(),
                    diagnostics,
                );
                return;
            }
            let Some(entity) = world.entity_for_guid(instance.definition.object_guid) else {
                return;
            };
            if let Err(error) = world.set_transform(entity, context.transform) {
                disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
                return;
            }
            instance.definition.properties = context.properties;
            instance
                .pending_events
                .borrow_mut()
                .extend(context.emitted_events);
        }
        Err(error) => {
            disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
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
    let event_context = context;
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

    let forge = lua.create_table()?;
    forge.set("Input", input)?;
    forge.set("Events", events)?;
    lua.globals().set("Forge", forge)?;
    Ok(())
}

fn install_object_api(lua: &Lua, state: &Table, context: SharedContext) -> Result<(), LuaError> {
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

    use super::{ScriptDefinition, ScriptHost};

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
}
