# Keyboard and mouse action bindings

InputMap binds physical keyboard codes and mouse buttons to ActionId values. Each binding includes a signed value, allowing two keys to drive one axis action. Active sources are summed and clamped to [-1, 1]; releasing one key leaves an action held while another bound source remains down.

Repeated key-down events do not enqueue duplicate presses. Synthetic keyboard events are ignored, and focus loss releases all active actions so controls cannot remain stuck after the window loses focus. Input events are timestamped by the host and queued into GameRuntime; the host must pass timestamps on the same monotonic simulation timeline used by the runtime.

The standalone player binds A/D to action ID 1 (-1/+1), W/S to action ID 2 (-1/+1), F to action ID 3 (fire), and the right mouse button to action ID 3. The editor preview uses the same axes and signs. This starter map is host-provided and currently cannot be changed in Project Settings.

During a fixed update, `Forge.Input.is_just_pressed(action)` is true only on the first tick where the logical action changes from released to pressed. The edge lasts for that tick even when a press and release are both drained before the tick, so quick taps can trigger one-shot actions while `is_pressed(action)` still reports the final released state. The edge is cleared before the next fixed tick; holding a key does not retrigger it.

Gamepad input, configurable project action-map serialization, and editor rebinding UI are not included yet.
