//! Small, platform-independent primitives shared by Forge runtime services.

mod diagnostics;
mod fixed_clock;
mod guid;
mod input;

pub use diagnostics::{DiagnosticBuffer, DiagnosticLevel, DiagnosticRecord};
pub use fixed_clock::{FixedStepClock, FramePlan};
pub use guid::Guid;
pub use input::{ActionEvent, ActionId, ActionState, InputTimeline};
