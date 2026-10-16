# Keyboard and mouse action bindings

InputMap binds physical keyboard codes and mouse buttons to ActionId values. Each binding includes a signed value, allowing two keys to drive one axis action. Active sources are summed and clamped to [-1, 1]; releasing one key leaves an action held while another bound source remains down.

Repeated key-down events do not enqueue duplicate presses. Synthetic keyboard events are ignored, and focus loss releases all active actions so controls cannot remain stuck after the window loses focus. Input events are timestamped by the host and queued into GameRuntime; the host must pass timestamps on the same monotonic simulation timeline used by the runtime.

The standalone player binds WASD to action IDs 1 (horizontal) and 2 (vertical), F to action ID 3 (fire), and the right mouse button to action ID 3. The sample and editor preview use the same action IDs. This starter map is host-provided and currently cannot be changed in Project Settings.

Gamepad input, configurable project action-map serialization, and editor rebinding UI are not included yet.
