use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionState {
    pub value: f32,
    pub pressed: bool,
    pub just_pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionEvent {
    pub at: Duration,
    pub sequence: u64,
    pub action: ActionId,
    pub value: f32,
    pub pressed: bool,
}

#[derive(Default)]
pub struct InputTimeline {
    queued: Vec<ActionEvent>,
    states: BTreeMap<ActionId, ActionState>,
    next_sequence: u64,
}

impl InputTimeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, at: Duration, action: ActionId, value: f32, pressed: bool) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.queued.push(ActionEvent {
            at,
            sequence,
            action,
            value: if value.is_finite() { value } else { 0.0 },
            pressed,
        });
    }

    pub fn drain_through(&mut self, at: Duration) -> Vec<ActionEvent> {
        self.queued.sort_by_key(|event| (event.at, event.sequence));
        let split = self.queued.partition_point(|event| event.at <= at);
        let ready = self.queued.drain(..split).collect::<Vec<_>>();
        for state in self.states.values_mut() {
            state.just_pressed = false;
        }
        for event in &ready {
            let state = self.states.entry(event.action).or_insert(ActionState {
                value: 0.0,
                pressed: false,
                just_pressed: false,
            });
            state.just_pressed |= event.pressed && !state.pressed;
            state.value = event.value;
            state.pressed = event.pressed;
        }
        ready
    }

    pub fn clear(&mut self) {
        self.queued.clear();
        self.states.clear();
        self.next_sequence = 0;
    }

    pub fn states(&self) -> &BTreeMap<ActionId, ActionState> {
        &self.states
    }

    pub fn state(&self, action: ActionId) -> Option<ActionState> {
        self.states.get(&action).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ActionId, ActionState, InputTimeline};

    #[test]
    fn quick_press_and_release_keeps_just_pressed_for_one_drain() {
        let action = ActionId(1);
        let mut input = InputTimeline::new();
        input.push(Duration::ZERO, action, 1.0, true);
        input.push(Duration::ZERO, action, 0.0, false);

        let events = input.drain_through(Duration::ZERO);
        assert_eq!(events.len(), 2);
        assert_eq!(
            input.state(action),
            Some(ActionState {
                value: 0.0,
                pressed: false,
                just_pressed: true,
            })
        );

        input.drain_through(Duration::from_millis(50));
        assert_eq!(
            input.state(action),
            Some(ActionState {
                value: 0.0,
                pressed: false,
                just_pressed: false,
            })
        );
    }
}
