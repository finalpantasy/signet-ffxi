//! Typed, thread-confined game-call boundary. No signature match or address
//! configuration automatically creates this backend. Actual client bindings
//! must be validated separately before using its unsafe constructor.
use std::{
    ffi::{CString, c_char},
    marker::PhantomData,
    rc::Rc,
    thread::{self, ThreadId},
};
use windower_rs::commands::{Backend, Input};

// Fenestra e5bfb644, hooks/ffximain.cpp + hooklib/hook.hpp: plain C function,
// two arguments, caller cleanup. This is a reference ABI, not live validation.
// Clean-client epilogues return an i32 in EAX; queued dispatch ignores it.
pub type InputFunction = unsafe extern "C" fn(*const c_char, i32) -> i32;
pub struct NativeInput {
    function: InputFunction,
    thread: ThreadId,
    _thread_confined: PhantomData<Rc<()>>,
}
impl NativeInput {
    /// # Safety
    /// The function must be a validated, live cdecl entry/trampoline with this
    /// exact ABI. Its module and any required game state must remain valid for
    /// every call. Construct on the verified game update thread only. Merely
    /// finding a matching byte signature does not satisfy these conditions.
    pub unsafe fn from_validated_function(function: InputFunction) -> Self {
        Self {
            function,
            thread: thread::current().id(),
            _thread_confined: PhantomData,
        }
    }
}
impl Backend for NativeInput {
    fn input(&mut self, input: &Input) -> Result<(), String> {
        input.validate()?;
        if thread::current().id() != self.thread {
            return Err("Game input called from the wrong thread".into());
        }
        let bytes = CString::new(input.bytes.clone()).map_err(|_| "NUL in game input")?;
        unsafe {
            (self.function)(bytes.as_ptr(), input.mode);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::CStr, sync::Mutex};
    static CALLS: Mutex<Vec<(i32, Vec<u8>)>> = Mutex::new(Vec::new());
    unsafe extern "C" fn fixture(text: *const c_char, mode: i32) -> i32 {
        CALLS
            .lock()
            .unwrap()
            .push((mode, unsafe { CStr::from_ptr(text) }.to_bytes().to_vec()));
        73
    }
    #[test]
    fn reference_cdecl_boundary_preserves_bytes_and_rejects_invalid_input_before_call() {
        let mut backend = unsafe { NativeInput::from_validated_function(fixture) };
        let mut input = Input {
            origin: windower_rs::platform::Ecosystem::Ashita4,
            mode: 2,
            bytes: vec![b'/', 0x83, 0x5c, 0xff],
        };
        backend.input(&input).unwrap();
        input.bytes.push(0);
        assert!(backend.input(&input).is_err());
        input.bytes = b"/echo valid".to_vec();
        input.mode = -1;
        assert!(backend.input(&input).is_err());
        assert_eq!(
            *CALLS.lock().unwrap(),
            vec![(2, vec![b'/', 0x83, 0x5c, 0xff])]
        );
    }
}
