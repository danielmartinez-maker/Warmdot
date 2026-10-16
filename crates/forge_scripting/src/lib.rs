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

#[derive(Debug)]
pub enum ScriptError {
    Lua(LuaError),
    MissingObject(Guid),
    TooManyInstances,
    SourceTooLarge(usize),
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
}

type SharedContext = Rc<RefCell<Option<ScriptCallContext>>>;

struct ScriptInstance {
    _lua: Lua,
    state: Table,
    on_start: Option<Function>,
    on_fixed_update: Option<Function>,
    context: SharedContext,
    interrupt_checkpoints: Rc<Cell<usize>>,
    definition: ScriptDefinition,
    start_pending: bool,
    disabled: bool,
}

impl ScriptInstance {
    fn compile(definition: ScriptDefinition, context: SharedContext) -> Result<Self, ScriptError> {
        if definition.source.len() > MAX_SCRIPT_SOURCE_BYTES {
            return Err(ScriptError::SourceTooLarge(definition.source.len()));
        }

        let lua = Lua::new();
        lua.set_memory_limit(MAX_VM_MEMORY_BYTES)?;
        let interrupt_checkpoints = Rc::new(Cell::new(0));
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

        install_forge_api(&lua, context.clone())?;
        lua.sandbox(true)?;

        let state: Table = lua
            .load(&definition.source)
            .set_name(&definition.source_path)
            .eval()?;
        install_object_api(&lua, &state, context.clone())?;
        let on_start: Option<Function> = state.get("on_start")?;
        let on_fixed_update: Option<Function> = state.get("on_fixed_update")?;

        Ok(Self {
            _lua: lua,
            state,
            on_start,
            on_fixed_update,
            context,
            interrupt_checkpoints,
            definition,
            start_pending: true,
            disabled: false,
        })
    }

    fn invoke(
        &mut self,
        callback: &Function,
        transform: Transform2D,
        actions: &BTreeMap<ActionId, ActionState>,
        delta: Duration,
    ) -> Result<ScriptCallContext, LuaError> {
        *self.context.borrow_mut() = Some(ScriptCallContext {
            transform,
            properties: self.definition.properties.clone(),
            actions: actions.clone(),
        });
        self.interrupt_checkpoints.set(0);
        let result = callback.call::<()>((self.state.clone(), delta.as_secs_f64()));
        let context = self
            .context
            .borrow_mut()
            .take()
            .unwrap_or_else(|| ScriptCallContext {
                transform,
                properties: self.definition.properties.clone(),
                actions: actions.clone(),
            });
        result?;
        Ok(context)
    }
}

pub struct ScriptHost {
    instances: BTreeMap<Guid, ScriptInstance>,
    diagnostics: Vec<ScriptDiagnostic>,
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
        let instance = ScriptInstance::compile(definition.clone(), context)?;
        self.instances.insert(definition.object_guid, instance);
        Ok(())
    }

    pub fn update(
        &mut self,
        world: &mut RuntimeWorld,
        actions: &BTreeMap<ActionId, ActionState>,
        delta: Duration,
        tick: u64,
    ) {
        for instance in self.instances.values_mut() {
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
            match ScriptInstance::compile(definition.clone(), context) {
                Ok(instance) => {
                    self.instances.insert(object_guid, instance);
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

    match instance.invoke(callback, transform, actions, delta) {
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
            if let Err(error) = world.set_transform(entity, context.transform) {
                disable_with_diagnostic(instance, tick, error.to_string(), diagnostics);
                return;
            }
            instance.definition.properties = context.properties;
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

fn install_forge_api(lua: &Lua, context: SharedContext) -> Result<(), LuaError> {
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

    let pressed_context = context;
    input.set(
        "is_pressed",
        lua.create_function(move |_, action: u32| {
            Ok(active_context(&pressed_context)?
                .actions
                .get(&ActionId(action))
                .is_some_and(|state| state.pressed))
        })?,
    )?;

    let forge = lua.create_table()?;
    forge.set("Input", input)?;
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
