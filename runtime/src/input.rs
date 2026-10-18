//! Process-local window input bridge for isolated tests. No global keyboard
//! hooks, SendInput, or access to another process/window are used.
//! DirectInput and equivalence of all legacy keyboard flag bits remain pending.
use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::*,
};

#[derive(Clone, Copy)]
struct Binding {
    original: isize,
    references: u32,
    unicode: bool,
}
static WINDOWS: OnceLock<Mutex<HashMap<usize, Binding>>> = OnceLock::new();
static KEYS: AtomicU32 = AtomicU32::new(0);
static MOUSE: AtomicU32 = AtomicU32::new(0);
static BLOCKED: AtomicU32 = AtomicU32::new(0);
fn windows() -> &'static Mutex<HashMap<usize, Binding>> {
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub unsafe fn bind(window: HWND) {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return;
    }
    let mut owner = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut owner);
    }
    if owner != unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() } {
        return;
    }
    let mut bindings = windows().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(binding) = bindings.get_mut(&(window as usize)) {
        binding.references += 1;
        return;
    }
    let original = unsafe { GetWindowLongPtrW(window, GWLP_WNDPROC) } as isize;
    if original == 0 {
        return;
    }
    bindings.insert(
        window as usize,
        Binding {
            original,
            references: 1,
            unicode: unsafe { IsWindowUnicode(window) } != 0,
        },
    );
    unsafe {
        SetWindowLongPtrW(window, GWLP_WNDPROC, window_proc as *const () as i32);
    }
}
pub unsafe fn unbind(window: HWND) {
    if window.is_null() {
        return;
    }
    let mut bindings = windows().lock().unwrap_or_else(|error| error.into_inner());
    let Some(binding) = bindings.get_mut(&(window as usize)) else {
        return;
    };
    binding.references = binding.references.saturating_sub(1);
    if binding.references != 0 {
        return;
    }
    if unsafe { IsWindow(window) } == 0 {
        bindings.remove(&(window as usize));
        return;
    }
    if unsafe { GetWindowLongPtrW(window, GWLP_WNDPROC) } as usize
        == window_proc as *const () as usize
    {
        unsafe {
            SetWindowLongPtrW(window, GWLP_WNDPROC, binding.original as i32);
        }
        bindings.remove(&(window as usize));
    }
    // If another component installed a later procedure, preserve its chain and
    // retain our forwarding context until WM_NCDESTROY instead of clobbering it.
}
pub fn counter(which: u32) -> u32 {
    match which {
        0 => KEYS.load(Ordering::Relaxed),
        1 => MOUSE.load(Ordering::Relaxed),
        2 => BLOCKED.load(Ordering::Relaxed),
        3 => windows()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|binding| binding.references > 0)
            .count() as u32,
        _ => u32::MAX,
    }
}
unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let binding = windows()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(window as usize))
        .copied();
    let Some(binding) = binding else {
        return unsafe { DefWindowProcW(window, message, w, l) };
    };
    let handled = std::panic::catch_unwind(|| match message {
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            KEYS.fetch_add(1, Ordering::Relaxed);
            let flags = ((l as u32) >> 24) & 0xa1;
            let mut dik = ((l as u32) >> 16) & 0xff;
            if flags & 1 != 0 {
                dik |= 0x80;
            }
            crate::frame_host::keyboard(dik, matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN), flags)
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN
        | WM_RBUTTONUP | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK
        | WM_MOUSEWHEEL => {
            MOUSE.fetch_add(1, Ordering::Relaxed);
            let x = (l as u32 & 0xffff) as u16 as i16 as i32;
            let y = ((l as u32 >> 16) & 0xffff) as u16 as i16 as i32;
            let delta = if message == WM_MOUSEWHEEL {
                ((w >> 16) & 0xffff) as u16 as i16 as i32
            } else {
                0
            };
            crate::frame_host::mouse(message & 0xff, x, y, delta)
        }
        _ => false,
    })
    .unwrap_or(false);
    if handled {
        BLOCKED.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    if message == WM_NCDESTROY {
        windows()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(window as usize));
    }
    let original = unsafe {
        std::mem::transmute::<isize, unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>(
            binding.original,
        )
    };
    if binding.unicode {
        unsafe { CallWindowProcW(Some(original), window, message, w, l) }
    } else {
        unsafe { CallWindowProcA(Some(original), window, message, w, l) }
    }
}
