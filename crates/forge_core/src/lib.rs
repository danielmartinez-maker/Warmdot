//! Small, platform-independent primitives shared by Forge runtime services.

mod fixed_clock;
mod guid;
mod input;
mod diagnostics;

pub use fixed_clock::{FixedStepClock, FramePlan};
pub use guid::Guid;
pub use input::{ActionEvent, ActionId, ActionState, InputTimeline};
pub use diagnostics::{DiagnosticBuffer, DiagnosticLevel, DiagnosticRecord};
