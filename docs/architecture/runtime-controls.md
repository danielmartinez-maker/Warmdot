# Runtime controls

GameRuntime starts in playing mode. While paused, advance_frame ignores wall-clock deltas and does not accrue catch-up work. step_once runs one complete fixed update and physics barrier and is only available while paused.

Replacing a scene compiles it before replacing the live world, then resets the fixed tick counter, fractional wall-clock accumulator, simulation time, queued and held input states, physics handles, and pending collision events. Registered fixed systems remain installed so the host can preserve runtime services across a scene transition.

No interpolation presentation snapshot is built yet; the frame exposes the clock alpha for a renderer or host to consume.

## Editor preview

The native editor builds a fresh `GameRuntime` from the current in-memory scene when Play is pressed. Pause clears the clock accumulator; Step advances one complete fixed tick only while paused. Stop discards the preview runtime and leaves scene changes in the editor history, where they can be saved. Scene switching and authoring commands stay disabled during play, while selection remains tied to stable authored GUIDs.

During preview, numeric input action 1 maps to horizontal A/D or left/right arrows, and action 2 maps to vertical W/S or up/down arrows. The editor refreshes the project asset catalog while running. PNG texture handles are reloaded by stable asset GUID; changed Luau compiles into replacement VMs only on success, and invalid source keeps the last working VM while adding a console diagnostic. Script property edits apply to the existing VM without resetting its Luau state; invalid values or type changes are rejected.
