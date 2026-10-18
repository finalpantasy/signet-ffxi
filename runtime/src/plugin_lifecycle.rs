//! Controlled fixture lifecycle only. Never initialize installed plugins with these mocks.
use crate::plugin::Module;
use std::{
    ffi::{c_char, c_void},
    path::Path,
    ptr,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

#[repr(C)]
struct Object {
    table: *const c_void,
}
#[repr(C)]
struct WTable {
    author: unsafe extern "system" fn(*mut Object) -> *const c_char,
    name: unsafe extern "system" fn(*mut Object) -> *const c_char,
    initialize: unsafe extern "system" fn(*mut Object, *mut WHost),
    destroy: unsafe extern "system" fn(*mut Object),
}
#[repr(C)]
struct ATable {
    name: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    author: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    description: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    link: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    version: unsafe extern "thiscall" fn(*mut Object) -> f64,
    interface: unsafe extern "thiscall" fn(*mut Object) -> f64,
    priority: unsafe extern "thiscall" fn(*mut Object) -> i32,
    flags: unsafe extern "thiscall" fn(*mut Object) -> u32,
    initialize: unsafe extern "thiscall" fn(*mut Object, *mut AHost, *mut c_void, u32) -> bool,
    release: unsafe extern "thiscall" fn(*mut Object),
    event: unsafe extern "thiscall" fn(*mut Object, *const c_char, *const c_void, u32),
    command: unsafe extern "thiscall" fn(*mut Object, i32, *const c_char, bool) -> bool,
}
#[derive(Default)]
struct Trace {
    commands: Vec<(i32, String)>,
}
unsafe fn text(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null mock chat text".into());
    }
    let mut bytes = Vec::new();
    for i in 0..1024 {
        let b = unsafe { *s.add(i) } as u8;
        if b == 0 {
            return String::from_utf8(bytes).map_err(|_| "Non-UTF8 mock chat".into());
        }
        bytes.push(b);
    }
    Err("Unterminated mock chat text".into())
}
#[repr(C)]
struct Capture {
    queue: unsafe extern "C" fn(*mut Trace, i32, *const c_char) -> i32,
    context: *mut Trace,
}
unsafe extern "C" fn capture(context: *mut Trace, mode: i32, command: *const c_char) -> i32 {
    if context.is_null() {
        return -1;
    }
    match unsafe { text(command) } {
        Ok(command) => {
            unsafe {
                (*context).commands.push((mode, command));
            }
            0
        }
        Err(_) => -1,
    }
}
#[repr(C)]
struct WHost {
    table: *const [usize; 4],
    capture: *mut Capture,
}
unsafe extern "system" fn wnull(_: *mut WHost) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "system" fn wget(self_: *mut WHost) -> *mut c_void {
    unsafe { (*self_).capture.cast() }
}
#[repr(C)]
struct ChatTable {
    parse: unsafe extern "thiscall" fn(*mut Chat, i32, *const c_char),
    queue: unsafe extern "thiscall" fn(*mut Chat, i32, *const c_char),
}
#[repr(C)]
struct Chat {
    table: *const ChatTable,
    trace: *mut Trace,
}
unsafe extern "thiscall" fn achat(self_: *mut Chat, mode: i32, s: *const c_char) {
    let _ = unsafe { capture((*self_).trace, mode, s) };
}
#[repr(C)]
struct CoreTable {
    handle: unsafe extern "thiscall" fn(*mut AHost) -> *mut c_void,
    path: unsafe extern "thiscall" fn(*mut AHost) -> *const c_char,
    device: unsafe extern "thiscall" fn(*mut AHost) -> *mut c_void,
    properties: unsafe extern "thiscall" fn(*mut AHost) -> *mut c_void,
    chat: unsafe extern "thiscall" fn(*mut AHost) -> *mut Chat,
}
#[repr(C)]
struct AHost {
    table: *const CoreTable,
    chat: *mut Chat,
}
unsafe extern "thiscall" fn anull(_: *mut AHost) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "thiscall" fn apath(_: *mut AHost) -> *const c_char {
    c"RustOffline".as_ptr()
}
unsafe extern "thiscall" fn aget(self_: *mut AHost) -> *mut Chat {
    unsafe { (*self_).chat }
}
type WCommand = unsafe extern "system" fn(*mut Object, i32, *const c_char, bool) -> bool;
type ADestroy = unsafe extern "system" fn(*mut Object);
enum Kind {
    Windower(WCommand),
    Ashita(ADestroy),
}
struct Session<'a> {
    object: *mut Object,
    active: bool,
    kind: Kind,
    _module: &'a Module,
}
impl Session<'_> {
    fn command(&self, mode: i32, text: &std::ffi::CStr, blocked: bool) -> Result<bool, String> {
        if !self.active || self.object.is_null() {
            return Err("Plugin is not initialized".into());
        }
        let next = unsafe {
            match self.kind {
                Kind::Windower(call) => call(self.object, mode, text.as_ptr(), blocked),
                Kind::Ashita(_) => {
                    let table = &*((*self.object).table.cast::<ATable>());
                    (table.command)(self.object, mode, text.as_ptr(), blocked)
                }
            }
        };
        Ok(blocked || next)
    }
    fn shutdown(&mut self) {
        if self.object.is_null() {
            return;
        }
        let object = std::mem::replace(&mut self.object, ptr::null_mut());
        unsafe {
            match self.kind {
                Kind::Windower(_) => {
                    let table = &*((*object).table.cast::<WTable>());
                    (table.destroy)(object);
                }
                Kind::Ashita(destroy) => {
                    if self.active {
                        let table = &*((*object).table.cast::<ATable>());
                        (table.release)(object);
                    }
                    destroy(object);
                }
            }
        }
        self.active = false;
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn check(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}
pub fn verify_fixture(path: &Path, sha256: &str) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    check(
        windower_rs::packages::digest(&bytes) == sha256,
        "Fixture native digest changed",
    )?;
    // A marker plus an explicit pinned path restrict this harness to the controlled fixture.
    let module = unsafe { Module::open(path) }?;
    let get = |name: &std::ffi::CStr| {
        unsafe { GetProcAddress(module.0, name.as_ptr().cast()) }.ok_or("Missing fixture export")
    };
    let marker: unsafe extern "system" fn() -> u32 =
        unsafe { std::mem::transmute(get(c"FixtureProtocol")?) };
    check(
        unsafe { marker() } == 0x52534631,
        "Not a controlled lifecycle fixture",
    )?;
    let count: unsafe extern "system" fn(u32) -> u32 =
        unsafe { std::mem::transmute(get(c"FixtureCount")?) };
    let wversion: unsafe extern "system" fn() -> u32 =
        unsafe { std::mem::transmute(get(c"GetInterfaceVersion")?) };
    let aversion: unsafe extern "system" fn() -> f64 =
        unsafe { std::mem::transmute(get(c"expGetInterfaceVersion")?) };
    check(
        unsafe { wversion() } == crate::plugin::INTERFACE_VERSION
            && unsafe { aversion() } == crate::ashita_plugin::INTERFACE_VERSION,
        "Fixture ABI version differs",
    )?;
    let wfactory: unsafe extern "system" fn() -> *mut Object =
        unsafe { std::mem::transmute(get(c"CreateInstance")?) };
    let wcallback: WCommand = unsafe { std::mem::transmute(get(c"FixtureWindowerCommand")?) };
    let afactory: unsafe extern "system" fn(*const c_char) -> *mut Object =
        unsafe { std::mem::transmute(get(c"expCreatePlugin")?) };
    let adestroy: ADestroy = unsafe { std::mem::transmute(get(c"expDestroyPlugin")?) };
    let mut wtrace = Trace::default();
    let mut atrace = Trace::default();
    let mut capture = Capture {
        queue: capture,
        context: &mut wtrace,
    };
    let wtable = [
        wnull as *const () as usize,
        wnull as *const () as usize,
        wnull as *const () as usize,
        wget as *const () as usize,
    ];
    let mut whost = WHost {
        table: &wtable,
        capture: &mut capture,
    };
    let ctable = ChatTable {
        parse: achat,
        queue: achat,
    };
    let mut chat = Chat {
        table: &ctable,
        trace: &mut atrace,
    };
    let coretable = CoreTable {
        handle: anull,
        path: apath,
        device: anull,
        properties: anull,
        chat: aget,
    };
    let mut ahost = AHost {
        table: &coretable,
        chat: &mut chat,
    };
    let object = unsafe { wfactory() };
    check(!object.is_null(), "Windower fixture factory failed")?;
    let mut w = Session {
        object,
        active: false,
        kind: Kind::Windower(wcallback),
        _module: &module,
    };
    let wt = unsafe { &*((*object).table.cast::<WTable>()) };
    check(
        unsafe { crate::plugin::read_metadata((wt.name)(object))? } == "WindowerLifecycleFixture",
        "Wrong Windower fixture metadata",
    )?;
    check(
        w.command(1, c"early", false).is_err(),
        "Command before initialization accepted",
    )?;
    unsafe {
        (wt.initialize)(object, &mut whost);
    }
    w.active = true;
    check(
        !w.command(5, c"/echo W fixture command", false)?
            && w.command(5, c"/fixture block", false)?
            && w.command(5, c"already blocked", true)?,
        "Windower fixture callback/block result",
    )?;
    w.shutdown();
    w.shutdown();
    check(
        w.command(1, c"late", false).is_err(),
        "Callback after shutdown accepted",
    )?;
    drop(w);
    // Failed Initialize is destroyed without calling Release on an uninitialized instance.
    let object = unsafe { afactory(c"".as_ptr()) };
    check(!object.is_null(), "Ashita factory failed")?;
    let failed = Session {
        object,
        active: false,
        kind: Kind::Ashita(adestroy),
        _module: &module,
    };
    let at = unsafe { &*((*object).table.cast::<ATable>()) };
    check(
        !unsafe { (at.initialize)(object, &mut ahost, ptr::null_mut(), 0) },
        "Expected fixture initialization rejection",
    )?;
    drop(failed);
    let object = unsafe { afactory(c"".as_ptr()) };
    check(!object.is_null(), "Ashita factory failed")?;
    let mut a = Session {
        object,
        active: false,
        kind: Kind::Ashita(adestroy),
        _module: &module,
    };
    let at = unsafe { &*((*object).table.cast::<ATable>()) };
    check(
        unsafe { (at.interface)(object) } == 4.30,
        "Ashita vtable interface version",
    )?;
    check(
        unsafe { (at.initialize)(object, &mut ahost, ptr::null_mut(), 7) },
        "Ashita fixture initialization failed",
    )?;
    a.active = true;
    let payload = [1u8, 2, 3];
    unsafe {
        (at.event)(object, c"fixture".as_ptr(), payload.as_ptr().cast(), 3);
    }
    check(
        !a.command(6, c"/echo A fixture command", false)?
            && a.command(6, c"/fixture block", false)?
            && a.command(6, c"already blocked", true)?,
        "Ashita public command/block result",
    )?;
    a.shutdown();
    a.shutdown();
    check(
        a.command(1, c"late", false).is_err(),
        "Ashita callback after shutdown accepted",
    )?;
    drop(a);
    let counts = (0..16).map(|i| unsafe { count(i) }).collect::<Vec<_>>();
    check(
        counts == vec![1, 1, 1, 1, 3, 2, 2, 1, 1, 2, 1, 3, 0, 0, 0, 0],
        "Native lifecycle/destruction counts differ",
    )?;
    check(
        wtrace.commands
            == vec![
                (7, "/echo W fixture init".into()),
                (5, "/echo W fixture command".into()),
            ]
            && atrace.commands
                == vec![
                    (3, "/echo A fixture init".into()),
                    (6, "/echo A fixture command".into()),
                ],
        "Mock host mode/command capture differs",
    )?;
    let filename = path
        .file_name()
        .ok_or("Fixture has no filename")?
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    drop(module);
    let unloaded = unsafe { GetModuleHandleW(filename.as_ptr()).is_null() };
    check(unloaded, "Fixture DLL did not unload")?;
    Ok(
        serde_json::json!({"checks_passed":true,"counts":counts,"windower_commands":wtrace.commands,
        "ashita_commands":atrace.commands,"dll_unloaded":unloaded,"game_commands_sent":0,
        "windower_legacy_callback_vtable":"NOT_RUN_unknown_signatures_private_fixture_export_used",
        "ashita_callbacks":"SDK4.30_HandleEvent_and_HandleCommand",
        "limits":["Controlled C++ fixtures and selected mock services only; installed plugin initialization remains unsupported"]}),
    )
}
