use std::time::Duration;

/// The bounded amount of simulation work due for one display frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlan {
    pub first_tick: u64,
    pub fixed_steps: u32,
    pub accepted_frame_time: Duration,
    pub dropped_time: Duration,
    pub interpolation_alpha: f64,
}

/// Accumulates wall time into fixed simulation ticks with bounded catch-up.
#[derive(Clone, Debug)]
pub struct FixedStepClock {
    step: Duration,
    max_frame_delta: Duration,
    max_steps_per_frame: u32,
    accumulator: Duration,
    next_tick: u64,
    total_dropped_time: Duration,
}

impl FixedStepClock {
    pub fn new(
        step: Duration,
        max_frame_delta: Duration,
        max_steps_per_frame: u32,
    ) -> Result<Self, &'static str> {
        if step.is_zero() {
            return Err("fixed step must be greater than zero");
        }
        if max_frame_delta.is_zero() {
            return Err("maximum frame delta must be greater than zero");
        }
        if max_steps_per_frame == 0 {
            return Err("maximum fixed steps per frame must be greater than zero");
        }
        Ok(Self {
            step,
            max_frame_delta,
            max_steps_per_frame,
            accumulator: Duration::ZERO,
            next_tick: 0,
            total_dropped_time: Duration::ZERO,
        })
    }

    pub fn advance(&mut self, frame_delta: Duration) -> FramePlan {
        let accepted_frame_time = frame_delta.min(self.max_frame_delta);
        let mut dropped_time = frame_delta.saturating_sub(accepted_frame_time);
        self.accumulator = self.accumulator.saturating_add(accepted_frame_time);

        let available = self.accumulator.as_nanos() / self.step.as_nanos();
        let fixed_steps = available.min(u128::from(self.max_steps_per_frame)) as u32;
        self.accumulator = self
            .accumulator
            .saturating_sub(self.step.saturating_mul(fixed_steps));

        let step_nanos = self.step.as_nanos();
        let remaining_nanos = self.accumulator.as_nanos();
        let retained_nanos = remaining_nanos % step_nanos;
        if retained_nanos != remaining_nanos {
            let retained = duration_from_nanos(retained_nanos);
            dropped_time = dropped_time.saturating_add(self.accumulator.saturating_sub(retained));
            self.accumulator = retained;
        }

        let plan = FramePlan {
            first_tick: self.next_tick,
            fixed_steps,
            accepted_frame_time,
            dropped_time,
            interpolation_alpha: self.accumulator.as_secs_f64() / self.step.as_secs_f64(),
        };
        self.next_tick = self.next_tick.saturating_add(u64::from(fixed_steps));
        self.total_dropped_time = self.total_dropped_time.saturating_add(dropped_time);
        plan
    }

    /// Produces one deterministic tick without consuming wall-clock time.
    pub fn advance_one_step(&mut self) -> FramePlan {
        let first_tick = self.next_tick;
        self.next_tick = self.next_tick.saturating_add(1);
        self.accumulator = Duration::ZERO;
        FramePlan {
            first_tick,
            fixed_steps: 1,
            accepted_frame_time: Duration::ZERO,
            dropped_time: Duration::ZERO,
            interpolation_alpha: 0.0,
        }
    }

    /// Clears fractional frame time while preserving tick and dropped-time counters.
    pub fn clear_accumulator(&mut self) {
        self.accumulator = Duration::ZERO;
    }

    /// Resets simulation tick, accumulated time, and dropped-time statistics.
    pub fn reset(&mut self) {
        self.accumulator = Duration::ZERO;
        self.next_tick = 0;
        self.total_dropped_time = Duration::ZERO;
    }

    pub fn step(&self) -> Duration {
        self.step
    }

    pub fn total_dropped_time(&self) -> Duration {
        self.total_dropped_time
    }
}

fn duration_from_nanos(nanos: u128) -> Duration {
    const NANOS_PER_SECOND: u128 = 1_000_000_000;
    Duration::new(
        (nanos / NANOS_PER_SECOND) as u64,
        (nanos % NANOS_PER_SECOND) as u32,
    )
}
