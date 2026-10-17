//! Narrow bridge to Luau's first-class line-breakpoint and resumable-break APIs.

#[allow(unsafe_code)]
mod raw_ffi_boundary {
    use std::{
        cell::RefCell,
        collections::HashMap,
        convert::TryFrom,
        ffi::CStr,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::{Rc, Weak},
        slice,
    };

    use mlua::{Error, Function, Lua, MultiValue, Thread, ffi};

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct BreakpointHit {
        pub source: String,
        pub line: u32,
    }

    #[derive(Default)]
    struct BreakpointOutcome {
        hit: Option<BreakpointHit>,
        failure: Option<String>,
        resume_skip: Option<BreakpointHit>,
    }

    type SharedOutcome = Rc<RefCell<BreakpointOutcome>>;

    #[derive(Clone, Default)]
    pub struct BreakpointContext {
        outcome: SharedOutcome,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum ResumeResult {
        Complete,
        Paused(BreakpointHit),
        Failed(String),
    }

    thread_local! {
        static ACTIVE_CONTEXTS: RefCell<HashMap<usize, Weak<RefCell<BreakpointOutcome>>>> =
            RefCell::new(HashMap::new());
    }

    struct ContextRegistration(usize);

    impl Drop for ContextRegistration {
        fn drop(&mut self) {
            ACTIVE_CONTEXTS.with(|contexts| {
                if let Ok(mut contexts) = contexts.try_borrow_mut() {
                    contexts.remove(&self.0);
                }
            });
        }
    }

    pub fn install(lua: &Lua) -> mlua::Result<()> {
        let mut failure = None;
        // The VM is newly created and not executing code while its Luau callback table is changed.
        unsafe {
            lua.exec_raw::<()>((), |state| {
                let callbacks = unsafe { ffi::lua_callbacks(state) };
                if callbacks.is_null() {
                    failure = Some("Luau did not expose its callback table".to_owned());
                    return;
                }
                unsafe {
                    (*callbacks).debugbreak = Some(debugbreak_callback);
                }
            })?;
        }
        if let Some(message) = failure {
            return Err(Error::RuntimeError(message.into()));
        }
        Ok(())
    }

    pub fn set_breakpoint(
        lua: &Lua,
        function: &Function,
        line: u32,
        enabled: bool,
    ) -> mlua::Result<Option<u32>> {
        let line = i32::try_from(line)
            .map_err(|_| Error::RuntimeError("breakpoint line is outside Luau's range".into()))?;
        // exec_raw pushes the function argument; the adapter removes it and returns one integer.
        let result = unsafe {
            lua.exec_raw::<i32>(function.clone(), |state| {
                let top = unsafe { ffi::lua_gettop(state) };
                let placed =
                    unsafe { ffi::lua_breakpoint(state, -1, line, if enabled { 1 } else { 0 }) };
                unsafe {
                    ffi::lua_settop(state, top - 1);
                    ffi::lua_pushinteger(state, i64::from(placed));
                }
            })?
        };
        Ok((result >= 0).then_some(result as u32))
    }

    pub fn resume(
        lua: &Lua,
        thread: &Thread,
        args: MultiValue,
        context: &BreakpointContext,
    ) -> mlua::Result<ResumeResult> {
        let arg_count = i32::try_from(args.len())
            .map_err(|_| Error::RuntimeError("too many Luau callback arguments".into()))?;
        let coroutine = thread.state();
        let key = unsafe { ffi::lua_mainthread(coroutine) } as usize;
        {
            let mut outcome = context.outcome.borrow_mut();
            outcome.hit = None;
            outcome.failure = None;
        }
        ACTIVE_CONTEXTS.with(|contexts| {
            contexts
                .borrow_mut()
                .insert(key, Rc::downgrade(&context.outcome));
        });
        let _registration = ContextRegistration(key);

        let mut status = ffi::LUA_ERRRUN;
        let mut error = None;
        // The coroutine is owned by mlua's Thread; this closure moves only the supplied callback
        // arguments and resumes that same Luau state. LUA_BREAK is handled as a pause, not an error.
        unsafe {
            lua.exec_raw::<()>(args, |main_state| {
                if arg_count > 0 {
                    unsafe {
                        ffi::lua_xmove(main_state, coroutine, arg_count);
                    }
                }
                let mut result_count = 0;
                eprintln!("probe: before lua_resumex");
                status = unsafe {
                    ffi::lua_resumex(coroutine, main_state, arg_count, &mut result_count)
                };
                eprintln!("probe: lua_resumex returned {status}");
                if status == ffi::LUA_OK {
                    unsafe {
                        ffi::lua_settop(coroutine, 0);
                    }
                } else if status != ffi::LUA_BREAK {
                    error = Some(unsafe { lua_error_text(coroutine) });
                    unsafe {
                        ffi::lua_settop(coroutine, 0);
                    }
                }
            })?;
        }

        let mut outcome = context.outcome.borrow_mut();
        if let Some(message) = outcome.failure.take() {
            return Ok(ResumeResult::Failed(message));
        }
        match status {
            ffi::LUA_OK => Ok(ResumeResult::Complete),
            ffi::LUA_BREAK => match outcome.hit.take() {
                Some(hit) => Ok(ResumeResult::Paused(hit)),
                None => Ok(ResumeResult::Failed(
                    "Luau stopped without a breakpoint location".to_owned(),
                )),
            },
            ffi::LUA_YIELD => Ok(ResumeResult::Failed(
                "script callbacks cannot yield; use a Forge breakpoint to pause".to_owned(),
            )),
            _ => Ok(ResumeResult::Failed(
                error.unwrap_or_else(|| "Luau callback failed".to_owned()),
            )),
        }
    }

    unsafe extern "C-unwind" fn debugbreak_callback(
        state: *mut ffi::lua_State,
        debug: *mut ffi::lua_Debug,
    ) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if state.is_null() || debug.is_null() {
                return;
            }
            // Luau only invokes this callback for a LOP_BREAK. During the hook, lua_atbreakpoint
            // can inspect an already-advanced saved instruction pointer and reject the real hit.
            let context_key = unsafe { ffi::lua_mainthread(state) } as usize;
            let context = ACTIVE_CONTEXTS.with(|contexts| {
                contexts
                    .try_borrow()
                    .ok()
                    .and_then(|contexts| contexts.get(&context_key).and_then(Weak::upgrade))
            });
            let Some(context) = context else {
                return;
            };
            if unsafe { ffi::lua_isyieldable(state) } == 0 {
                context.borrow_mut().failure =
                    Some("Luau breakpoint was reached outside a resumable callback".to_owned());
                return;
            }

            let debug = unsafe { &*debug };
            let source = if debug.source.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(debug.source) }
                    .to_string_lossy()
                    .into_owned()
            };
            if debug.currentline <= 0 {
                context.borrow_mut().failure =
                    Some("Luau breakpoint has no executable source line".to_owned());
                return;
            }
            let hit = BreakpointHit {
                source,
                line: debug.currentline as u32,
            };
            // Resume re-enters the same break instruction once before executing its line.
            // Consume only that re-entry; later loop visits should still stop here.
            let skip_resumed_breakpoint = {
                let mut outcome = context.borrow_mut();
                if outcome.resume_skip.as_ref() == Some(&hit) {
                    outcome.resume_skip = None;
                    true
                } else {
                    outcome.hit = Some(hit.clone());
                    false
                }
            };
            eprintln!(
                "probe: debugbreak at line {} skip={skip_resumed_breakpoint}",
                hit.line
            );
            if skip_resumed_breakpoint {
                return;
            }

            unsafe {
                ffi::lua_break(state);
            }
            context.borrow_mut().resume_skip = Some(hit);
        }));
    }

    unsafe fn lua_error_text(state: *mut ffi::lua_State) -> String {
        let mut length = 0;
        let message = unsafe { ffi::lua_tolstring(state, -1, &mut length) };
        if message.is_null() {
            return "Luau callback failed".to_owned();
        }
        let bytes = unsafe { slice::from_raw_parts(message.cast::<u8>(), length) };
        String::from_utf8_lossy(bytes).into_owned()
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{cell::Cell, rc::Rc};

        #[test]
        fn entrypoint_breakpoint_pauses_nested_callback() {
            let lua = Lua::new();
            install(&lua).expect("install Luau debugger");
            let calls = Rc::new(Cell::new(0));
            let callback_calls = Rc::clone(&calls);
            let translate = lua
                .create_function(move |_, (_self: mlua::Table, x: f64, y: f64)| {
                    assert_eq!((x, y), (1.0, 0.0));
                    callback_calls.set(callback_calls.get() + 1);
                    Ok(())
                })
                .expect("create native Forge-style callback");
            let object = lua.create_table().expect("create script object");
            object
                .set("translate", translate)
                .expect("install native object method");
            let entrypoint: Function = lua
                .load("return {\n    callback = function(self, dt)\n        self:translate(dt, 0)\n    end,\n}\n")
                .into_function()
                .expect("compile script entrypoint");
            let state: mlua::Table = entrypoint.call(()).expect("run script entrypoint");
            let callback: Function = state.get("callback").expect("load nested callback");
            assert_eq!(
                set_breakpoint(&lua, &entrypoint, 3, true).expect("set source breakpoint"),
                Some(3)
            );

            let thread = lua.create_thread(callback).expect("create callback thread");
            let context = BreakpointContext::default();
            let args = MultiValue::from_vec(vec![
                mlua::Value::Table(object),
                mlua::Value::Number(1.0),
            ]);
            let result = resume(&lua, &thread, args, &context).expect("resume callback");

            assert!(
                matches!(&result, ResumeResult::Paused(hit) if hit.line == 3),
                "result={result:?}; native calls={}",
                calls.get()
            );
            assert_eq!(calls.get(), 0, "native callback ran before breakpoint");

            let continued = resume(&lua, &thread, MultiValue::default(), &context)
                .expect("continue paused callback");
            assert_eq!(continued, ResumeResult::Complete);
            assert_eq!(calls.get(), 1, "resumed native callback must run once");
        }
    }
}

pub use raw_ffi_boundary::{
    BreakpointContext, BreakpointHit, ResumeResult, install, resume, set_breakpoint,
};
