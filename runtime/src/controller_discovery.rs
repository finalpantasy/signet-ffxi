//! Process-local DirectInput 8 A/W enumeration cache. Only controller discovery
//! is deferred; device creation, polling, mappings,
//! rumble, keyboard-only and mouse-only enumeration retain their native path.
//!
//! Installed before the suspended application's entry point. Native objects are
//! created, used and released on one worker. Neither game COM pointers nor game
//! callbacks cross threads. Hooks, their trampolines and this module stay mapped
//! until process exit. This is deliberately not an unloadable addon.
use retour::RawDetour;
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{
    LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
        GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    },
    Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL},
};

type Object = *mut c_void;
type EnumCallback = unsafe extern "system" fn(*const Descriptor, *mut c_void) -> i32;
type Enumerate =
    unsafe extern "system" fn(Object, u32, Option<EnumCallback>, *mut c_void, u32) -> i32;
type Create = unsafe extern "system" fn(Object, u32, *const Guid, *mut Object, Object) -> i32;
const OK: i32 = 0;
const FAIL: i32 = 0x80004005_u32 as i32;
const OUT_OF_MEMORY: i32 = 0x8007000e_u32 as i32;
const FLAG_BITS: [u32; 2] = [1, 0x100];
const MAX_DEVICES: usize = 128;
const REFRESH: Duration = Duration::from_secs(1);
const VALID_FLAGS: u32 = 1 | 0x100;
const INSTANCE_BYTES: [usize; 2] = [580, 1100];

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IIDS: [Guid; 2] = [
    Guid(
        0xbf798030,
        0x483a,
        0x4da2,
        [0xaa, 0x99, 0x5d, 0x64, 0xed, 0x36, 0x97, 0x00],
    ),
    Guid(
        0xbf798031,
        0x483a,
        0x4da2,
        [0xaa, 0x99, 0x5d, 0x64, 0xed, 0x36, 0x97, 0x00],
    ),
];

// Both native DIDEVICEINSTANCE layouts have DWORD alignment. Copy only dwSize
// bytes from native memory, never read a W-sized record from an A callback.
#[repr(C, align(4))]
#[derive(Clone)]
struct Descriptor([u8; 1100]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    wide: bool,
    device_type: u32,
    flags: u32,
}
impl Key {
    fn index(self) -> usize {
        usize::from(self.wide) * 4
            + FLAG_BITS.iter().enumerate().fold(0, |index, (bit, mask)| {
                index | (usize::from(self.flags & mask != 0) << bit)
            })
    }
    fn cached(self) -> bool {
        // Leave mixed ALL queries on the native path as well, so startup
        // discovery of keyboards/mice and uncommon alias/phantom/hidden flags
        // retain native behavior. Only standard attached/force-feedback queries
        // are intercepted; every intercepted slot is continuously refreshed.
        // Do not mask invalid values: native error checking remains authoritative.
        self.device_type == 4 && self.flags & !VALID_FLAGS == 0
    }
}
#[derive(Clone)]
struct Snapshot {
    result: i32,
    devices: Arc<[Descriptor]>,
}
impl Snapshot {
    fn empty() -> Self {
        Self {
            result: OK,
            devices: Arc::from([]),
        }
    }
    unsafe fn replay(&self, callback: EnumCallback, context: *mut c_void) -> i32 {
        if self.result >= 0 {
            for device in self.devices.iter() {
                if unsafe { callback(device, context) } == 0 {
                    break;
                }
            }
        }
        self.result
    }
}
struct Entry {
    key: Key,
    snapshot: arc_swap::ArcSwap<Snapshot>,
    requested: AtomicBool,
    generation: AtomicU32,
}
struct Cache {
    entries: Vec<Entry>,
    control: Mutex<()>,
    wake: Condvar,
    active: AtomicBool,
    stopped: AtomicBool,
    queries: AtomicU32,
    scans: AtomicU32,
    errors: AtomicU32,
    query_max_us: AtomicU32,
    scan_max_us: AtomicU32,
    startup_ms: AtomicU32,
}
impl Cache {
    fn new() -> Self {
        let entries = (0..8)
            .map(|index| {
                let flags = FLAG_BITS.iter().enumerate().fold(0, |flags, (bit, mask)| {
                    if index & (1 << bit) != 0 {
                        flags | mask
                    } else {
                        flags
                    }
                });
                Entry {
                    key: Key {
                        wide: index >= 4,
                        device_type: 4,
                        flags,
                    },
                    snapshot: arc_swap::ArcSwap::from_pointee(Snapshot::empty()),
                    requested: AtomicBool::new(false),
                    generation: AtomicU32::new(0),
                }
            })
            .collect();
        Self {
            entries,
            control: Mutex::new(()),
            wake: Condvar::new(),
            active: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            queries: AtomicU32::new(0),
            scans: AtomicU32::new(0),
            errors: AtomicU32::new(0),
            query_max_us: AtomicU32::new(0),
            scan_max_us: AtomicU32::new(0),
            startup_ms: AtomicU32::new(0),
        }
    }
    fn lookup(&self, key: Key) -> Arc<Snapshot> {
        let started = Instant::now();
        self.queries.fetch_add(1, Ordering::Relaxed);
        let entry = &self.entries[key.index()];
        entry.requested.store(true, Ordering::Relaxed);
        // Immutable publication: no caller mutex, no cold-cache fabricated
        // disconnect, and no OS discovery. Old readers retain their snapshot.
        let snapshot = entry.snapshot.load_full();
        maximum(&self.query_max_us, started.elapsed());
        snapshot
    }
    fn publish(&self, entry: &Entry, snapshot: Snapshot, elapsed: Duration) {
        maximum(&self.scan_max_us, elapsed);
        if snapshot.result < 0 {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        entry.snapshot.store(Arc::new(snapshot));
        entry.generation.fetch_add(1, Ordering::Release);
        self.scans.fetch_add(1, Ordering::Relaxed);
        let _control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        self.wake.notify_all();
    }
    fn prepare(&self, mut scan: impl FnMut(Key) -> Snapshot) -> Result<(), String> {
        let startup = Instant::now();
        // Before game startup, prepare all eight supported ANSI/Unicode flag
        // combination. Do not assume the game repeats an initially empty call.
        for entry in &self.entries {
            if self.stopped.load(Ordering::Acquire) {
                return Err("Controller discovery startup cancelled".into());
            }
            let started = Instant::now();
            let snapshot = scan(entry.key);
            self.publish(entry, snapshot, started.elapsed());
        }
        self.startup_ms.store(
            startup.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            Ordering::Relaxed,
        );
        Ok(())
    }
    fn run(&self, mut scan: impl FnMut(Key) -> Snapshot) {
        loop {
            let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            let (control, _) = self
                .wake
                .wait_timeout(control, REFRESH)
                .unwrap_or_else(|e| e.into_inner());
            drop(control);
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            if !self.active.load(Ordering::Acquire) {
                continue;
            }
            for entry in &self.entries {
                if self.stopped.load(Ordering::Acquire) {
                    return;
                }
                let started = Instant::now();
                let snapshot = scan(entry.key);
                self.publish(entry, snapshot, started.elapsed());
            }
        }
    }
    fn activate(&self) {
        let _control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        self.active.store(true, Ordering::Release);
        self.wake.notify_all();
    }
    fn stop(&self) {
        let _control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}
fn maximum(counter: &AtomicU32, duration: Duration) {
    counter.fetch_max(
        duration.as_micros().min(u128::from(u32::MAX)) as u32,
        Ordering::Relaxed,
    );
}
struct Hooks {
    detours: [RawDetour; 2],
    cache: Arc<Cache>,
    installed: AtomicBool,
}
static HOOKS: OnceLock<Hooks> = OnceLock::new();
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
#[path = "controller_discovery_tests.rs"]
mod tests;

/// # Safety
/// Call on the bootstrap thread before resuming the new application's main
/// thread. Refuses a process that already initialized FFXI or an original host.
pub unsafe fn install() -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::AcqRel) {
        return Err("Controller discovery was already initialized".into());
    }
    for name in ["FFXiMain.dll", "polcore.dll", "Hook.dll", "Ashita.dll"] {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        if !unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null() {
            return Err(format!(
                "Controller discovery requires an early independent runtime; {name} is loaded"
            ));
        }
    }
    // The worker and system API callbacks cannot outlive the runtime's code.
    let mut pinned = ptr::null_mut();
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            (install as *const ()).cast(),
            &mut pinned,
        )
    } == 0
    {
        return Err("Could not pin controller discovery runtime".into());
    }
    let cache = Arc::new(Cache::new());
    let worker_cache = Arc::clone(&cache);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("signet-controller-discovery".into())
        .spawn(move || {
            unsafe {
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
            }
            match unsafe { Native::new() } {
                Ok(native) => {
                    if let Err(error) = worker_cache.prepare(|key| unsafe {
                        native.scan_with(key, native.targets()[usize::from(key.wide)] as *const ())
                    }) {
                        let _ = sender.send(Err(error));
                        return;
                    }
                    if sender.send(Ok(native.targets())).is_err() {
                        return;
                    }
                    worker_cache.run(|key| unsafe { native.scan(key) });
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let prepared = (|| {
        let targets = receiver
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| {
                "Controller discovery preparation exceeded its 15-second startup budget".to_string()
            })??;
        // A and W descriptors have different layouts. Never guess the encoding
        // on a provider that uses a single shared method implementation.
        if targets[0] == targets[1] {
            return Err(
                "DirectInput A/W enumeration targets alias; refusing ambiguous hooks".into(),
            );
        }
        let a = unsafe { RawDetour::new(targets[0] as *const (), enumerate_a as *const ()) }
            .map_err(|e| e.to_string())?;
        let w = unsafe { RawDetour::new(targets[1] as *const (), enumerate_w as *const ()) }
            .map_err(|e| e.to_string())?;
        HOOKS
            .set(Hooks {
                detours: [a, w],
                cache: Arc::clone(&cache),
                installed: AtomicBool::new(false),
            })
            .map_err(|_| "Controller discovery installation raced".to_string())?;
        let hooks = HOOKS.get().ok_or("Missing controller discovery state")?;
        for (i, detour) in hooks.detours.iter().enumerate() {
            if let Err(error) = unsafe { detour.enable() } {
                for enabled in hooks.detours[..i].iter().rev() {
                    let _ = unsafe { enabled.disable() };
                }
                return Err(format!("Controller discovery detour refused: {error}"));
            }
        }
        hooks.installed.store(true, Ordering::Release);
        cache.activate();
        Ok(())
    })();
    if prepared.is_err() {
        cache.stop();
    }
    prepared
}

unsafe extern "system" fn enumerate_a(
    this: Object,
    kind: u32,
    callback: Option<EnumCallback>,
    context: *mut c_void,
    flags: u32,
) -> i32 {
    unsafe { enumerate(false, this, kind, callback, context, flags) }
}
unsafe extern "system" fn enumerate_w(
    this: Object,
    kind: u32,
    callback: Option<EnumCallback>,
    context: *mut c_void,
    flags: u32,
) -> i32 {
    unsafe { enumerate(true, this, kind, callback, context, flags) }
}
unsafe fn enumerate(
    wide: bool,
    this: Object,
    device_type: u32,
    callback: Option<EnumCallback>,
    context: *mut c_void,
    flags: u32,
) -> i32 {
    let Some(hooks) = HOOKS.get() else {
        return FAIL;
    };
    let key = Key {
        wide,
        device_type,
        flags,
    };
    if this.is_null()
        || callback.is_none()
        || !key.cached()
        || !hooks.installed.load(Ordering::Acquire)
    {
        let original: Enumerate =
            unsafe { std::mem::transmute(hooks.detours[usize::from(wide)].trampoline()) };
        return unsafe { original(this, device_type, callback, context, flags) };
    }
    // Release the cache lock before invoking any caller code. Callbacks can
    // reenter enumeration and always execute on their original caller thread.
    let cache: &Cache = hooks.cache.as_ref();
    match std::panic::catch_unwind(|| cache.lookup(key)) {
        Ok(snapshot) => unsafe { snapshot.replay(callback.unwrap(), context) },
        Err(_) => FAIL,
    }
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoUninitialize();
}
struct Native {
    objects: [Object; 2],
}
impl Native {
    unsafe fn new() -> Result<Self, String> {
        if unsafe { CoInitializeEx(ptr::null_mut(), 0) } < 0 {
            return Err("Controller worker COM initialization failed".into());
        }
        let mut native = Self {
            objects: [ptr::null_mut(); 2],
        };
        let name: Vec<u16> = "dinput8.dll".encode_utf16().chain(Some(0)).collect();
        let module =
            unsafe { LoadLibraryExW(name.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return Err("Could not load system DirectInput 8".into());
        }
        let symbol = unsafe { GetProcAddress(module, c"DirectInput8Create".as_ptr().cast()) }
            .ok_or("Missing DirectInput8Create")?;
        let create: Create = unsafe { std::mem::transmute(symbol) };
        for (i, iid) in IIDS.iter().enumerate() {
            let result = unsafe {
                create(
                    GetModuleHandleW(ptr::null()),
                    0x0800,
                    iid,
                    &mut native.objects[i],
                    ptr::null_mut(),
                )
            };
            if result < 0 || native.objects[i].is_null() {
                return Err(format!(
                    "DirectInput {} initialization failed: {result:#x}",
                    if i == 0 { "A" } else { "W" }
                ));
            }
        }
        Ok(native)
    }
    fn targets(&self) -> [usize; 2] {
        self.objects
            .map(|object| unsafe { *(*(object as *const *const usize)).add(4) })
    }
    unsafe fn scan(&self, key: Key) -> Snapshot {
        let Some(hooks) = HOOKS.get() else {
            return Snapshot {
                result: FAIL,
                devices: Arc::from([]),
            };
        };
        unsafe { self.scan_with(key, hooks.detours[usize::from(key.wide)].trampoline()) }
    }
    unsafe fn scan_with(&self, key: Key, original: *const ()) -> Snapshot {
        let index = usize::from(key.wide);
        let original: Enumerate = unsafe { std::mem::transmute(original) };
        let mut collector = Collector {
            bytes: INSTANCE_BYTES[index],
            devices: Vec::with_capacity(MAX_DEVICES),
            error: OK,
        };
        let result = unsafe {
            original(
                self.objects[index],
                key.device_type,
                Some(collect),
                (&mut collector as *mut Collector).cast(),
                key.flags,
            )
        };
        let result = if collector.error < 0 {
            collector.error
        } else {
            result
        };
        Snapshot {
            result,
            devices: if result >= 0 {
                collector.devices.into()
            } else {
                Arc::from([])
            },
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        for object in self.objects {
            if !object.is_null() {
                let release: unsafe extern "system" fn(Object) -> u32 =
                    unsafe { std::mem::transmute(*(*(object as *const *const usize)).add(2)) };
                unsafe {
                    release(object);
                }
            }
        }
        unsafe {
            CoUninitialize();
        }
    }
}
struct Collector {
    bytes: usize,
    devices: Vec<Descriptor>,
    error: i32,
}
unsafe extern "system" fn collect(instance: *const Descriptor, context: *mut c_void) -> i32 {
    let collector = unsafe { &mut *context.cast::<Collector>() };
    if instance.is_null()
        || unsafe { instance.cast::<u32>().read_unaligned() } as usize != collector.bytes
    {
        collector.error = FAIL;
        return 0;
    }
    if collector.devices.len() == MAX_DEVICES {
        collector.error = OUT_OF_MEMORY;
        return 0;
    }
    let mut copy = Descriptor([0; 1100]);
    unsafe {
        ptr::copy_nonoverlapping(instance.cast::<u8>(), copy.0.as_mut_ptr(), collector.bytes);
    }
    collector.devices.push(copy);
    1
}

#[cfg(any(feature = "client-diagnostics", test))]
pub fn observe() -> serde_json::Value {
    let Some(hooks) = HOOKS.get() else {
        return serde_json::json!({"installed":false});
    };
    let cache = &hooks.cache;
    serde_json::json!({"installed":hooks.installed.load(Ordering::Acquire),
        "queries":cache.queries.load(Ordering::Relaxed),
        "scans":cache.scans.load(Ordering::Relaxed),"scan_errors":cache.errors.load(Ordering::Relaxed),
        "query_max_us":cache.query_max_us.load(Ordering::Relaxed),"scan_max_us":cache.scan_max_us.load(Ordering::Relaxed),
        "startup_ms":cache.startup_ms.load(Ordering::Relaxed),
        "refresh_ms":REFRESH.as_millis(),"lifetime":"process","native_hitch_fix_verified":false})
}
pub fn counter(which: u32) -> u32 {
    let Some(hooks) = HOOKS.get() else {
        return 0;
    };
    match which {
        0 => u32::from(hooks.installed.load(Ordering::Acquire)),
        1 => hooks.cache.queries.load(Ordering::Relaxed),
        2 => hooks.cache.scans.load(Ordering::Relaxed),
        3 => hooks.cache.errors.load(Ordering::Relaxed),
        4 => hooks.cache.query_max_us.load(Ordering::Relaxed),
        5 => hooks.cache.scan_max_us.load(Ordering::Relaxed),
        6 => hooks
            .cache
            .entries
            .iter()
            .filter(|entry| entry.requested.load(Ordering::Relaxed))
            .count() as u32,
        7 => hooks.cache.startup_ms.load(Ordering::Relaxed),
        _ => u32::MAX,
    }
}
