# Runtime controls

GameRuntime starts in playing mode. While paused, advance_frame ignores wall-clock deltas and does not accrue catch-up work. step_once runs one complete fixed update and physics barrier and is only available while paused.

Replacing a scene compiles it before replacing the live world, then resets the fixed tick counter, fractional wall-clock accumulator, simulation time, queued and held input states, physics handles, and pending collision events. Registered fixed systems remain installed so the host can preserve runtime services across a scene transition.

No interpolation presentation snapshot is built yet; the frame exposes the clock alpha for a renderer or host to consume.
