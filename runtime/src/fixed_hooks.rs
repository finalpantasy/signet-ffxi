//! Two fixed, relocation-free x86 prologues only. Not a general detour engine.
//! Allocate/protect before suspending. While peers are stopped, only kernel
//! thread APIs, raw comparisons/copies and FlushInstructionCache are used.
//! Trampolines and active hooks intentionally remain until process exit.
use std::{ffi::c_void, ptr, sync::Mutex};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Diagnostics::Debug::{
            CONTEXT, CONTEXT_CONTROL_X86, FlushInstructionCache, GetThreadContext,
            ReadProcessMemory,
        },
        Memory::{
            MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_READWRITE,
            VirtualAlloc, VirtualProtect,
        },
        Threading::{
            GetCurrentProcess, GetCurrentThreadId, GetExitCodeThread, GetThreadId, ResumeThread,
            SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
        },
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtGetNextThread(
        process: HANDLE,
        thread: HANDLE,
        access: u32,
        attributes: u32,
        flags: u32,
        next: *mut HANDLE,
    ) -> i32;
}
const MAX_THREADS: usize = 512;
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
const NO_MORE_ENTRIES: u32 = 0x8000001a;
#[derive(Clone, Copy)]
struct Thread {
    handle: HANDLE,
    suspended: bool,
}
struct Frozen {
    threads: [Thread; MAX_THREADS],
    count: usize,
}
impl Drop for Frozen {
    fn drop(&mut self) {
        // Resume every owned suspension before closing anything or allocating.
        let _ = self.resume_all();
        let _ = self.resume_all(); // retry only failed resumptions before closing
        for record in &self.threads[..self.count] {
            unsafe {
                CloseHandle(record.handle);
            }
        }
    }
}
impl Frozen {
    fn resume_all(&mut self) -> Result<(), &'static str> {
        let mut resumed = true;
        for record in &mut self.threads[..self.count] {
            if !record.suspended {
                continue;
            }
            if unsafe { ResumeThread(record.handle) } != u32::MAX {
                record.suspended = false;
            } else {
                let mut exit_code = 259;
                if unsafe { GetExitCodeThread(record.handle, &mut exit_code) } != 0
                    && exit_code != 259
                {
                    record.suspended = false;
                } else {
                    resumed = false;
                }
            }
        }
        if resumed {
            Ok(())
        } else {
            Err("Could not resume every suspended peer; dispatch must remain disabled")
        }
    }
    fn acquire(ranges: &[(usize, usize)]) -> Result<Self, &'static str> {
        let mut result = Self {
            threads: [Thread {
                handle: ptr::null_mut(),
                suspended: false,
            }; MAX_THREADS],
            count: 0,
        };
        let mut previous = ptr::null_mut();
        loop {
            if result.count == MAX_THREADS {
                return Err("Thread limit prevents quiescent patching");
            }
            let mut next = ptr::null_mut();
            let status = unsafe {
                NtGetNextThread(
                    GetCurrentProcess(),
                    previous,
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                    0,
                    0,
                    &mut next,
                )
            };
            if status as u32 == NO_MORE_ENTRIES {
                break;
            }
            if status < 0 || next.is_null() {
                return Err("Could not enumerate every process thread");
            }
            let record = &mut result.threads[result.count];
            record.handle = next;
            result.count += 1;
            previous = next;
            let thread_id = unsafe { GetThreadId(next) };
            if thread_id == 0 {
                return Err("Could not identify a peer thread");
            }
            if thread_id == unsafe { GetCurrentThreadId() } {
                continue;
            }
            if unsafe { SuspendThread(next) } == u32::MAX {
                let mut exit_code = 259;
                if unsafe { GetExitCodeThread(next, &mut exit_code) } != 0 && exit_code != 259 {
                    continue;
                }
                return Err("Could not suspend a peer thread");
            }
            record.suspended = true;
        }
        // All peers are now stopped; reject PCs in instructions we would replace.
        for record in &result.threads[..result.count] {
            if !record.suspended {
                continue;
            }
            let mut context: CONTEXT = unsafe { std::mem::zeroed() };
            context.ContextFlags = CONTEXT_CONTROL_X86;
            if unsafe { GetThreadContext(record.handle, &mut context) } == 0 {
                return Err("Could not inspect a suspended thread");
            }
            if ranges
                .iter()
                .any(|&(start, length)| (start..start + length).contains(&(context.Eip as usize)))
            {
                return Err("A peer thread is inside a target prologue; no patch installed");
            }
        }
        Ok(result)
    }
}

pub struct Patch {
    target: usize,
    trampoline: usize,
    length: usize,
    original: [u8; 16],
    replacement: [u8; 16],
}
impl Patch {
    /// # Safety
    /// Target and callback must have matching ABIs and remain valid for the
    /// process lifetime. Expected bytes must be complete instructions with no
    /// IP-relative operands. Only the two audited game prologues are admitted.
    pub unsafe fn prepare(
        target: usize,
        callback: usize,
        expected: &[u8],
    ) -> Result<Self, &'static str> {
        if expected != [0x81, 0xec, 4, 4, 0, 0]
            && expected != [0x56, 0x8b, 0xf1, 0x8b, 0x86, 0x50, 0x0d, 0, 0]
        {
            return Err("Unaudited x86 prologue");
        }
        let length = expected.len();
        let mut original = [0u8; 16];
        let mut copied = 0;
        if unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                target as *const c_void,
                original.as_mut_ptr().cast(),
                length,
                &mut copied,
            )
        } == 0
            || copied != length
            || &original[..length] != expected
        {
            return Err("Target prologue differs from the audited bytes");
        }
        let allocation =
            unsafe { VirtualAlloc(ptr::null(), 32, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) }
                as usize;
        if allocation == 0 {
            return Err("Could not allocate a trampoline");
        }
        let mut trampoline = [0x90u8; 32];
        trampoline[..length].copy_from_slice(expected);
        trampoline[length..length + 5].copy_from_slice(&jump(allocation + length, target + length));
        unsafe {
            ptr::copy_nonoverlapping(trampoline.as_ptr(), allocation as *mut u8, 32);
        }
        let mut old = 0;
        if unsafe { VirtualProtect(allocation as *const c_void, 32, PAGE_EXECUTE_READ, &mut old) }
            == 0
            || unsafe {
                FlushInstructionCache(GetCurrentProcess(), allocation as *const c_void, 32)
            } == 0
        {
            return Err("Could not finalize a trampoline");
        }
        let mut replacement = [0x90; 16];
        replacement[..5].copy_from_slice(&jump(target, callback));
        Ok(Self {
            target,
            trampoline: allocation,
            length,
            original,
            replacement,
        })
    }
    pub fn trampoline(&self) -> usize {
        self.trampoline
    }
}
fn jump(source: usize, destination: usize) -> [u8; 5] {
    let mut bytes = [0xe9, 0, 0, 0, 0];
    bytes[1..].copy_from_slice(
        &(destination as u32)
            .wrapping_sub((source as u32).wrapping_add(5))
            .to_le_bytes(),
    );
    bytes
}
pub fn install(patches: &[Patch; 2]) -> Result<usize, &'static str> {
    let _serial = INSTALL_LOCK
        .lock()
        .map_err(|_| "Patch transaction lock poisoned")?;
    let mut old = [0u32; 2];
    let mut protected = 0;
    let outcome = (|| {
        for patch in patches {
            if unsafe {
                VirtualProtect(
                    patch.target as *const c_void,
                    patch.length,
                    PAGE_EXECUTE_READWRITE,
                    &mut old[protected],
                )
            } == 0
            {
                return Err("Could not protect target code");
            }
            protected += 1;
        }
        let ranges = [
            (patches[0].target, patches[0].length),
            (patches[1].target, patches[1].length),
        ];
        let mut frozen = Frozen::acquire(&ranges)?;
        for patch in patches {
            if unsafe { std::slice::from_raw_parts(patch.target as *const u8, patch.length) }
                != &patch.original[..patch.length]
            {
                return Err("Target code changed before the quiescent patch");
            }
        }
        for patch in patches {
            unsafe {
                ptr::copy_nonoverlapping(
                    patch.replacement.as_ptr(),
                    patch.target as *mut u8,
                    patch.length,
                );
            }
        }
        if unsafe { FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0) } == 0 {
            for patch in patches {
                unsafe {
                    ptr::copy_nonoverlapping(
                        patch.original.as_ptr(),
                        patch.target as *mut u8,
                        patch.length,
                    );
                }
            }
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0);
            }
            return Err("Instruction-cache flush failed; original bytes restored");
        }
        let stopped = frozen.threads[..frozen.count]
            .iter()
            .filter(|t| t.suspended)
            .count();
        frozen.resume_all()?;
        drop(frozen);
        Ok(stopped)
    })();
    // Reverse order also restores correctly if both targets share one page.
    let mut restored = true;
    for index in (0..protected).rev() {
        let mut discarded = 0;
        restored &= unsafe {
            VirtualProtect(
                patches[index].target as *const c_void,
                patches[index].length,
                old[index],
                &mut discarded,
            )
        } != 0;
    }
    if !restored {
        return Err("Target page protection restoration failed; dispatch must remain disabled");
    }
    outcome
}

#[cfg(test)]
pub(crate) fn isolated_fixture(name: &str) -> bool {
    if std::env::var("WINDOWER_OWNED_HOOK_FIXTURE").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads=1", "--nocapture"])
        .env("WINDOWER_OWNED_HOOK_FIXTURE", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Owned hook fixture failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };
    #[repr(C)]
    struct Flags {
        stop: AtomicU32,
        inside: AtomicU32,
    }
    #[test]
    fn quiescence_rejects_a_peer_in_the_patch_range_and_resumes_it() {
        if isolated_fixture(
            "fixed_hooks::tests::quiescence_rejects_a_peer_in_the_patch_range_and_resumes_it",
        ) {
            return;
        }
        let _serial = INSTALL_LOCK.lock().unwrap();
        // mov eax,[esp+4]; mov dword ptr[eax+4],1;
        // cmp dword ptr[eax],0; je -5; ret -- bounded by our stop flag.
        let code = [
            0x8b, 0x44, 0x24, 0x04, 0xc7, 0x40, 0x04, 1, 0, 0, 0, 0x83, 0x38, 0, 0x74, 0xfb, 0xc3,
        ];
        let allocation = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as usize;
        assert_ne!(allocation, 0);
        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), allocation as *mut u8, code.len());
            FlushInstructionCache(GetCurrentProcess(), allocation as _, code.len());
        }
        let flags = Arc::new(Flags {
            stop: AtomicU32::new(0),
            inside: AtomicU32::new(0),
        });
        let owner = flags.clone();
        let worker = std::thread::spawn(move || {
            let f: unsafe extern "C" fn(*const Flags) = unsafe { std::mem::transmute(allocation) };
            unsafe { f(Arc::as_ptr(&owner)) };
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while flags.inside.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if flags.inside.load(Ordering::Acquire) == 0 {
            flags.stop.store(1, Ordering::Release);
            worker.join().unwrap();
            panic!("Fixture thread did not enter its owned code");
        }
        let rejected = Frozen::acquire(&[(allocation, code.len())]).err();
        flags.stop.store(1, Ordering::Release);
        worker.join().unwrap();
        assert_eq!(
            rejected,
            Some("A peer thread is inside a target prologue; no patch installed")
        );
    }
}
