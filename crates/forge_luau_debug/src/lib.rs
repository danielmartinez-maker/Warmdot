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
                let placed = unsafe {
                    ffi::lua_breakpoint(state, -1, line, if enabled { 1 } else { 0 })
                };
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
        let key = coroutine as usize;
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
                status = unsafe {
                    ffi::lua_resumex(coroutine, main_state, arg_count, &mut result_count)
                };
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
            if state.is_null()
                || debug.is_null()
                || unsafe { ffi::lua_atbreakpoint(state) } == 0
            {
                return;
            }
            let context = ACTIVE_CONTEXTS.with(|contexts| {
                contexts
                    .try_borrow()
                    .ok()
                    .and_then(|contexts| contexts.get(&(state as usize)).and_then(Weak::upgrade))
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
            context.borrow_mut().hit = Some(BreakpointHit {
                source,
                line: debug.currentline as u32,
            });
            unsafe {
                ffi::lua_break(state);
            }
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

}

pub use raw_ffi_boundary::{
    BreakpointContext, BreakpointHit, ResumeResult, install, resume, set_breakpoint,
};
