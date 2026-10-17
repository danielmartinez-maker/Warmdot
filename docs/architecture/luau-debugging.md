# Luau line debugging

P0.2 adds native source-line breakpoints to the existing sandboxed Luau host. The first slice supports setting and removing breakpoints, reporting the object/source/line/tick that stopped, and resuming the exact callback. Stepping, watch expressions, local-variable inspection, and a full debugger panel remain later work.

## VM boundary

Forge uses Luau's first-class `lua_breakpoint`, `debugbreak`, `lua_break`, and resumable `LUA_BREAK` status. It does not emulate line breakpoints with instruction hooks. A script callback runs through an mlua coroutine only while it has active breakpoints; ordinary callbacks retain the existing direct-call path.

mlua's high-level `Thread::resume` maps `LUA_BREAK` to an error and cannot resume that state. A dedicated `forge_luau_debug` crate wraps only the raw operations needed to place a breakpoint, capture its source line, suspend the coroutine, and resume it. The rest of the engine continues to inherit the workspace's unsafe-code deny policy. The raw adapter keeps unsafe operations in one annotated module and must not expose Lua stack pointers or lifetimes to callers.

The existing mlua interrupt budget stays installed on every VM and applies to coroutine execution. A normal script yield is treated as an unsupported callback error; the debugger uses Luau's separate breakpoint status.

## Tick and callback behavior

- Script callback work for a fixed tick is queued in the existing stable object-GUID and event order.
- A hit suspends that callback's coroutine and retains its transactional object context, staged events, and the rest of the current callback queue.
- The runtime pauses before animation and physics, retaining the in-progress fixed-tick continuation. It does not start another tick while a breakpoint is pending.
- Continue resumes the same callback with zero arguments. If it hits another breakpoint, the runtime stays paused. Once the queue completes, animation and physics finish for the retained tick, then normal play resumes.
- Syntax or migration failures keep the last-good VM. Hot reloads for an asset with a suspended callback are queued until the callback completes.
- Stop or scene replacement drops the suspended callback and its pending tick as part of the existing preview-reset behavior.
- Breakpoints are associated with stable asset GUIDs and source line numbers. Luau may move a requested line forward to the next line with executable code; the accepted line is reported to the caller.

## Public engine surface

`ScriptHost` exposes line-breakpoint set/remove, the current `ScriptBreakpointHit`, and continue. `GameRuntime` exposes the same controls and automatically holds simulation at a hit. The editor integrates source-line controls and a continue action in P0.3; P0.2 provides the runtime capability first.

## Limits and validation

Forge retains the loaded source chunk and places breakpoints on it; Luau propagates them to its nested functions, including callback bodies and helper functions. The chunk is executed once during compilation, so top-level initialization does not run again when a breakpoint is added. Breakpoint status includes authored asset path, object GUID, accepted source line, and fixed-tick number. The implementation must preserve callback event ordering, the 20,000-checkpoint budget, bounded emitted-event queues, state migration, and last-good reload behavior. Windows GitHub Actions supplies formatting, type-check, and existing workspace-suite evidence; player-facing debugger UI remains a separate acceptance check.
