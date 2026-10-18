//! LuaJIT VM provider only: no game hooks, DllMain worker or original cores.
//! Native source is caller-selected/trusted, matching the existing addon host.
#[path = "../../../runtime/src/lua.rs"]
pub mod lua;
use lua::{Addon, HostAction};
use mlua::{MultiValue, Value as LuaValue};
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock},
};
use windower_rs::vm_protocol::*;

#[derive(Default)]
struct State {
    next_id: u64,
    pending_creates: usize,
    addons: BTreeMap<u64, Arc<Mutex<Addon>>>,
    responses: BTreeMap<u64, Option<Vec<u8>>>,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}
fn next_id(state: &mut State) -> Result<u64, String> {
    state.next_id = state
        .next_id
        .checked_add(1)
        .ok_or("Provider handle space exhausted")?;
    Ok(state.next_id)
}
struct CreateReservation;
impl CreateReservation {
    fn acquire() -> Result<Self, String> {
        let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
        if state.addons.len() + state.pending_creates >= 256 {
            return Err("Provider addon capacity exhausted".into());
        }
        state.pending_creates += 1;
        Ok(Self)
    }
}
impl Drop for CreateReservation {
    fn drop(&mut self) {
        state()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending_creates -= 1;
    }
}
struct ResponseReservation(u64, bool);
impl Drop for ResponseReservation {
    fn drop(&mut self) {
        if !self.1 {
            state()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .responses
                .remove(&self.0);
        }
    }
}
fn actions(addon: &Addon) -> Vec<Action> {
    addon
        .take_actions()
        .into_iter()
        .map(HostAction::into_protocol)
        .collect()
}

fn callbacks(outcomes: Vec<(u32, mlua::Result<MultiValue>)>) -> Vec<Callback> {
    outcomes
        .into_iter()
        .map(|(id, outcome)| match outcome {
            Ok(values) => Callback {
                id,
                error: None,
                values: values
                    .into_iter()
                    .map(|value| match value {
                        LuaValue::Nil => Value::Nil,
                        LuaValue::Boolean(value) => Value::Boolean(value),
                        LuaValue::Integer(value) => Value::Number(value as f64),
                        LuaValue::Number(value) if value.is_finite() => Value::Number(value),
                        LuaValue::String(value) => Value::Bytes(value.as_bytes().to_vec()),
                        other => Value::Unsupported(other.type_name().into()),
                    })
                    .collect(),
            },
            Err(error) => Callback {
                id,
                values: Vec::new(),
                error: Some(error.to_string()),
            },
        })
        .collect()
}
fn arguments(addon: &Addon, values: Vec<Value>) -> Result<MultiValue, String> {
    values
        .into_iter()
        .map(|value| match value {
            Value::Nil => Ok(LuaValue::Nil),
            Value::Boolean(value) => Ok(LuaValue::Boolean(value)),
            Value::Number(value) if value.is_finite() => Ok(LuaValue::Number(value)),
            Value::Bytes(value) => addon
                .lua()
                .create_string(&value)
                .map(LuaValue::String)
                .map_err(|error| error.to_string()),
            _ => Err("Unsupported serialized Lua argument".into()),
        })
        .collect()
}
fn with_addon(
    handle: u64,
    work: impl FnOnce(&Addon) -> Result<Response, String>,
) -> Result<Response, String> {
    let addon = state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .addons
        .get(&handle)
        .cloned()
        .ok_or("Unknown addon handle")?;
    // Registry locks never span Lua execution. Each VM has its own lock.
    let addon = addon.lock().unwrap_or_else(|error| error.into_inner());
    let mut response = work(&addon)?;
    response.actions = actions(&addon);
    Ok(response)
}
fn execute(request: Request) -> Result<Response, String> {
    if request.version != MESSAGE_VERSION {
        return Err("Unsupported VM message version".into());
    }
    match request.operation {
        Operation::Create {
            name,
            runtime_path,
            addon_path,
            source,
            ecosystem,
        } => {
            let reservation = CreateReservation::acquire()?;
            let addon = Addon::from_source_with_ecosystem(
                &name,
                &runtime_path,
                &addon_path,
                &source,
                ecosystem,
            )
            .map_err(|error| error.to_string())?;
            let mut response = Response::success();
            response.actions = actions(&addon);
            let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
            let handle = next_id(&mut state)?;
            state.addons.insert(handle, Arc::new(Mutex::new(addon)));
            response.handle = Some(handle);
            drop(state);
            drop(reservation);
            Ok(response)
        }
        Operation::Destroy { handle } => {
            let removed = state()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .addons
                .remove(&handle)
                .ok_or("Unknown addon handle")?;
            drop(removed);
            Ok(Response::success())
        }
        Operation::Emit {
            handle,
            event,
            arguments: values,
        } => with_addon(handle, |addon| {
            let values = arguments(addon, values)?;
            Ok(Response {
                callbacks: callbacks(addon.emit(&event, values)),
                ..Response::success()
            })
        }),
        Operation::EmitBlocking {
            handle,
            event,
            arguments: values,
            blocked_at,
            blocked,
        } => with_addon(handle, |addon| {
            if blocked_at >= values.len() {
                return Err("Blocked argument index is outside event arguments".into());
            }
            let values = arguments(addon, values)?;
            let (blocked, outcomes) = addon.emit_blocking(&event, values, blocked_at, blocked);
            Ok(Response {
                callbacks: callbacks(outcomes),
                blocked,
                ..Response::success()
            })
        }),
        Operation::Frame { handle, index } => with_addon(handle, |addon| {
            Ok(Response {
                callbacks: callbacks(addon.emit_frame(index)),
                ..Response::success()
            })
        }),
        Operation::Poll { handle, seconds } => with_addon(handle, |addon| {
            let errors = addon.poll_at(seconds).map_err(|error| error.to_string())?;
            Ok(Response {
                errors,
                ..Response::success()
            })
        }),
        Operation::TakeActions { handle } => with_addon(handle, |_| Ok(Response::success())),
        Operation::Diagnostics { handle } => with_addon(handle, |addon| {
            Ok(Response { scheduled:addon.scheduled_count().map_err(|e|e.to_string())?, ..Response::success() })
        }),
    }
}

unsafe extern "system" fn request(input: *const u8, length: u32, output: *mut u64) -> i32 {
    if input.is_null() || output.is_null() || length == 0 || length as usize > MAX_REQUEST_BYTES {
        return BAD_ARGUMENT;
    }
    unsafe {
        output.write(0);
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let token = {
            let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
            if state.responses.len() >= 16 {
                return CAPACITY_EXCEEDED;
            }
            let token = match next_id(&mut state) {
                Ok(id) => id,
                Err(_) => return CAPACITY_EXCEEDED,
            };
            state.responses.insert(token, None);
            token
        };
        let mut reservation = ResponseReservation(token, false);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let bytes = unsafe { std::slice::from_raw_parts(input, length as usize) };
            match serde_json::from_slice::<Request>(bytes) {
                Ok(request) => execute(request).unwrap_or_else(Response::failure),
                Err(error) => Response::failure(error),
            }
        }))
        .unwrap_or_else(|_| Response::failure("VM provider caught a Rust panic"));
        let mut bytes = serde_json::to_vec(&result)
            .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"serialization failed\"}".to_vec());
        if bytes.len() > MAX_RESPONSE_BYTES {
            // A failed Create must not strand a VM whose handle the caller
            // never receives. Do not drop Lua objects under the registry lock.
            if let Some(handle) = result.handle {
                let removed = state()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .addons
                    .remove(&handle);
                drop(removed);
            }
            bytes = serde_json::to_vec(&Response::failure(
                "Provider response exceeds its size limit",
            ))
            .unwrap_or_default();
        }
        state()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .responses
            .insert(token, Some(bytes));
        unsafe {
            output.write(token);
        }
        reservation.1 = true;
        OK
    })) {
        Ok(code) => code,
        Err(_) => PANIC,
    }
}
unsafe extern "system" fn read_response(
    token: u64,
    output: *mut u8,
    capacity: u32,
    required: *mut u32,
) -> i32 {
    if required.is_null() {
        return BAD_ARGUMENT;
    }
    unsafe {
        required.write(0);
    }
    let state = state().lock().unwrap_or_else(|error| error.into_inner());
    let Some(Some(bytes)) = state.responses.get(&token) else {
        return UNKNOWN_RESPONSE;
    };
    unsafe {
        required.write(bytes.len() as u32);
    }
    if (capacity as usize) < bytes.len() {
        return BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return BAD_ARGUMENT;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
    }
    OK
}
unsafe extern "system" fn release_response(token: u64) -> i32 {
    if state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .responses
        .remove(&token)
        .is_some()
    {
        OK
    } else {
        UNKNOWN_RESPONSE
    }
}
unsafe extern "system" fn counts(addons: *mut u32, responses: *mut u32) -> i32 {
    if addons.is_null() || responses.is_null() {
        return BAD_ARGUMENT;
    }
    let state = state().lock().unwrap_or_else(|error| error.into_inner());
    unsafe {
        addons.write((state.addons.len() + state.pending_creates) as u32);
        responses.write(state.responses.len() as u32);
    }
    OK
}
static API: ProviderApiV1 = ProviderApiV1 {
    struct_bytes: std::mem::size_of::<ProviderApiV1>() as u32,
    abi_version: ABI_VERSION,
    pointer_bits: usize::BITS,
    engine: ENGINE_LUAJIT,
    request: Some(request),
    read_response: Some(read_response),
    release_response: Some(release_response),
    counts: Some(counts),
};

#[unsafe(no_mangle)]
pub extern "system" fn windower_vm_api_v1() -> *const ProviderApiV1 {
    &API
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn submit(bytes: &[u8]) -> u64 {
        let mut token = 0;
        assert_eq!(
            unsafe { request(bytes.as_ptr(), bytes.len() as u32, &mut token) },
            OK
        );
        assert_ne!(token, 0);
        token
    }
    fn read(token: u64) -> Response {
        let mut needed = 0;
        assert_eq!(
            unsafe { read_response(token, std::ptr::null_mut(), 0, &mut needed) },
            BUFFER_TOO_SMALL
        );
        let mut bytes = vec![0; needed as usize];
        assert_eq!(
            unsafe { read_response(token, bytes.as_mut_ptr(), bytes.len() as u32, &mut needed) },
            OK
        );
        serde_json::from_slice(&bytes).unwrap()
    }
    fn sizes() -> (usize, usize) {
        let state = state().lock().unwrap();
        (
            state.addons.len() + state.pending_creates,
            state.responses.len(),
        )
    }
    #[test]
    fn malformed_requests_response_limits_and_double_release_are_checked() {
        let _guard = TEST_LOCK.lock().unwrap();
        let tokens = (0..16).map(|_| submit(b"{} invalid")).collect::<Vec<_>>();
        let mut output = 99;
        assert_eq!(
            unsafe { request(b"{}".as_ptr(), 2, &mut output) },
            CAPACITY_EXCEEDED
        );
        assert_eq!(output, 0);
        assert_eq!(sizes(), (0, 16));
        for token in tokens {
            assert!(!read(token).ok);
            assert!(!read(token).ok); // sizing/reads do not re-execute or consume.
            assert_eq!(unsafe { release_response(token) }, OK);
            assert_eq!(unsafe { release_response(token) }, UNKNOWN_RESPONSE);
        }
        assert_eq!(sizes(), (0, 0));
        let request = Request::new(Operation::TakeActions { handle: 1 });
        let mut malformed = serde_json::to_value(request).unwrap();
        malformed["operation"]["unexpected"] = true.into();
        let token = submit(&serde_json::to_vec(&malformed).unwrap());
        assert!(!read(token).ok);
        assert_eq!(unsafe { release_response(token) }, OK);
    }
    #[test]
    fn failed_or_oversized_creation_and_panics_release_reservations() {
        let _guard = TEST_LOCK.lock().unwrap();
        for source in [
            "error('fixture failure')",
            "ffxi.commands.queue(string.rep('a', 400000))",
        ] {
            let bytes = serde_json::to_vec(&Request::new(Operation::Create {
                name: "failure".into(),
                runtime_path: "C:/Test/".into(),
                addon_path: "C:/Test/failure/".into(),
                ecosystem: windower_rs::platform::Ecosystem::Native,
                source: source.into(),
            }))
            .unwrap();
            let token = submit(&bytes);
            assert!(!read(token).ok);
            assert_eq!(sizes(), (0, 1));
            assert_eq!(unsafe { release_response(token) }, OK);
        }
        let reservations = (0..256)
            .map(|_| CreateReservation::acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(CreateReservation::acquire().is_err());
        drop(reservations);
        assert!(
            catch_unwind(|| {
                let _reservation = CreateReservation::acquire().unwrap();
                panic!("fixture panic");
            })
            .is_err()
        );
        assert_eq!(sizes(), (0, 0));
    }
}
