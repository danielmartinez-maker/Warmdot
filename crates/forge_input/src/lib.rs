//! Translate native keyboard and mouse events into timestamped Forge actions.
//!
//! Bindings are physical-key based so keyboard layout text does not change game controls.
//! Each key/button is an independent action source; opposing or repeated bindings are aggregated
//! before events enter the runtime input timeline.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    error::Error,
    fmt,
    time::Duration,
};

use forge_core::ActionId;
use forge_runtime::GameRuntime;
use winit::{
    event::{ElementState, WindowEvent},
    keyboard::PhysicalKey,
};

#[derive(Clone, Copy, Debug)]
struct Binding {
    action: ActionId,
    value: f32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum InputSource {
    Key(PhysicalKey),
    Mouse(winit::event::MouseButton),
}

#[derive(Debug, PartialEq)]
pub enum InputBindingError {
    NonFiniteOrZeroValue,
    DuplicateBinding,
}

impl fmt::Display for InputBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteOrZeroValue => f.write_str("binding values must be finite and non-zero"),
            Self::DuplicateBinding => {
                f.write_str("the same input source is already bound to this action")
            }
        }
    }
}

impl Error for InputBindingError {}

#[derive(Default)]
pub struct InputMap {
    bindings: HashMap<InputSource, Vec<Binding>>,
    down: HashSet<InputSource>,
    active: HashMap<(ActionId, InputSource), f32>,
}

impl InputMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind_key(
        &mut self,
        key: PhysicalKey,
        action: ActionId,
        value: f32,
    ) -> Result<(), InputBindingError> {
        self.bind(InputSource::Key(key), action, value)
    }

    pub fn bind_mouse(
        &mut self,
        button: winit::event::MouseButton,
        action: ActionId,
        value: f32,
    ) -> Result<(), InputBindingError> {
        self.bind(InputSource::Mouse(button), action, value)
    }

    pub fn handle_window_event(
        &mut self,
        event: &WindowEvent,
        at: Duration,
        runtime: &mut GameRuntime,
    ) -> bool {
        match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if *is_synthetic {
                    return false;
                }
                self.update_source(
                    InputSource::Key(event.physical_key.clone()),
                    event.state == ElementState::Pressed,
                    at,
                    runtime,
                );
                true
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.update_source(
                    InputSource::Mouse(*button),
                    *state == ElementState::Pressed,
                    at,
                    runtime,
                );
                true
            }
            WindowEvent::Focused(false) => {
                self.release_all(at, runtime);
                true
            }
            _ => false,
        }
    }

    pub fn release_all(&mut self, at: Duration, runtime: &mut GameRuntime) {
        let actions = self
            .active
            .keys()
            .map(|(action, _)| *action)
            .collect::<BTreeSet<_>>();
        self.active.clear();
        self.down.clear();
        for action in actions {
            runtime.push_action(at, action, 0.0, false);
        }
    }

    fn bind(
        &mut self,
        source: InputSource,
        action: ActionId,
        value: f32,
    ) -> Result<(), InputBindingError> {
        if !value.is_finite() || value == 0.0 {
            return Err(InputBindingError::NonFiniteOrZeroValue);
        }
        let bindings = self.bindings.entry(source).or_default();
        if bindings.iter().any(|binding| binding.action == action) {
            return Err(InputBindingError::DuplicateBinding);
        }
        bindings.push(Binding { action, value });
        Ok(())
    }

    fn update_source(
        &mut self,
        source: InputSource,
        pressed: bool,
        at: Duration,
        runtime: &mut GameRuntime,
    ) {
        if pressed {
            if !self.down.insert(source.clone()) {
                return;
            }
        } else if !self.down.remove(&source) {
            return;
        }
        let Some(bindings) = self.bindings.get(&source).cloned() else {
            return;
        };
        for binding in bindings {
            let key = (binding.action, source.clone());
            if pressed {
                self.active.insert(key, binding.value);
            } else {
                self.active.remove(&key);
            }
            let mut value = 0.0;
            let mut active_sources = 0;
            for ((action, _), source_value) in &self.active {
                if *action == binding.action {
                    value += *source_value;
                    active_sources += 1;
                }
            }
            runtime.push_action(
                at,
                binding.action,
                value.clamp(-1.0, 1.0),
                active_sources > 0,
            );
        }
    }
}
