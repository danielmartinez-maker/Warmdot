use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionState {
    pub value: f32,
    pub pressed: bool,
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
        for event in &ready {
            self.states.insert(
                event.action,
                ActionState {
                    value: event.value,
                    pressed: event.pressed,
                },
            );
        }
        ready
    }

    pub fn clear(&mut self) {
        self.queued.clear();
        self.states.clear();
        self.next_sequence = 0;
    }

    pub fn state(&self, action: ActionId) -> Option<ActionState> {
        self.states.get(&action).copied()
    }
}
