//! Scheduling hints for the game's threads and its allocator: the few OS calls
//! that the other crates, which forbid unsafe code, can't make themselves.

/// The game window's boot screen, before wgpu's first frame paints over it.
#[cfg(target_os = "windows")]
pub mod boot_paint;

use std::sync::atomic::{AtomicBool, Ordering};

/// Ask the OS to run the calling thread ahead of the process's other
/// threads, as vanilla's `MinecraftServer.spin` does for its server thread
/// (`Thread.setPriority(8)` when there are more than four processors), so
/// world generation and chunk encoding on every other core don't hold a tick
/// back (the client asks the same for its render thread, so they don't hold
/// a frame back either): user-interactive quality of service on macOS,
/// above-normal priority on Windows. Nothing elsewhere: Linux lets an
/// unprivileged thread lower its priority but not raise it (and the JVM
/// leaves it there too). Whether the OS took it.
pub fn favour_current() -> bool {
    if std::thread::available_parallelism().map_or(1, |n| n.get()) <= 4 {
        return false;
    }
    favour()
}

#[cfg(target_vendor = "apple")]
fn favour() -> bool {
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0) == 0
    }
}

#[cfg(windows)]
fn favour() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    // SAFETY: GetCurrentThread's pseudo handle needs no closing.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) != 0 }
}

#[cfg(not(any(target_vendor = "apple", windows)))]
fn favour() -> bool {
    false
}

/// The CPU time the calling thread has used, where only an OS call gives it
/// precisely. Windows counts it exactly only in cycles of the time-stamp
/// counter (`QueryThreadCycleTime`; `GetThreadTimes` moves in scheduler ticks
/// of ~15.6 ms), so the counter's rate is measured once against the wall
/// clock and cycles convert to time. `None` elsewhere: unix callers read
/// `CLOCK_THREAD_CPUTIME_ID` themselves.
#[cfg(all(windows, target_arch = "x86_64"))]
pub fn thread_cpu() -> Option<std::time::Duration> {
    use std::{
        sync::OnceLock,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::{
        Threading::GetCurrentThread, WindowsProgramming::QueryThreadCycleTime,
    };
    // Time-stamp counter cycles per nanosecond.
    static RATE: OnceLock<f64> = OnceLock::new();
    let rate = *RATE.get_or_init(|| {
        // SAFETY: RDTSC has no preconditions on x86_64.
        let tsc = || unsafe { core::arch::x86_64::_rdtsc() };
        let (wall, start) = (Instant::now(), tsc());
        std::thread::sleep(Duration::from_millis(20));
        (tsc() - start) as f64 / wall.elapsed().as_nanos() as f64
    });
    let mut cycles = 0;
    // SAFETY: GetCurrentThread's pseudo handle needs no closing; a valid out pointer.
    if unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) } == 0 {
        return None;
    }
    Some(Duration::from_nanos((cycles as f64 / rate) as u64))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn thread_cpu() -> Option<std::time::Duration> {
    None
}

/// Set once as the game starts, before its helper threads do: they then go
/// behind the server thread and the render thread (`behind_server`).
static HELPERS_BEHIND: AtomicBool = AtomicBool::new(false);

/// How many nice levels a helper thread goes below the process on Linux and Android.
#[cfg(any(target_os = "linux", target_os = "android"))]
const BEHIND: i32 = 10;

/// From now on, the helper threads that call `behind_server` as they start
/// go behind the server thread (a dedicated server's or single player's)
/// and the render thread. Call it before they start.
pub fn put_helpers_behind() {
    HELPERS_BEHIND.store(true, Ordering::Relaxed);
}

/// Called by a helper thread as it starts (world generation, lighting and
/// meshing, a connection's packet writer): work no tick or frame waits for.
/// Once `put_helpers_behind` was called, on Linux and Android, where the server and
/// render threads can't be raised (`favour_current`), the helper lowers
/// itself instead, which an unprivileged thread may: 10 nice levels below
/// the process, or, where nice runs out (a game started under nice 10 or
/// more), the idle class, which runs the thread only when no ordinary thread
/// wants its core. A tick or a frame then gets a core when it starts instead
/// of waiting behind them. Nothing elsewhere. Whether the OS took it.
pub fn behind_server() -> bool {
    HELPERS_BEHIND.load(Ordering::Relaxed) && behind()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn behind() -> bool {
    // SAFETY: these read and set the calling thread's own nice value or
    // policy (who 0, pid 0 is the calling thread on Linux and Android); `param` lives
    // across the call.
    unsafe {
        let nice = libc::getpriority(libc::PRIO_PROCESS as _, 0);
        if nice + BEHIND <= 19 {
            libc::setpriority(libc::PRIO_PROCESS as _, 0, nice + BEHIND) == 0
        } else {
            let param = libc::sched_param { sched_priority: 0 };
            libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) == 0
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn behind() -> bool {
    false
}

/// Run the calling thread only on a core nothing else wants: work that may
/// take seconds and that nothing waits for yet (the stronghold rings while a
/// world opens). The idle class on Linux, background quality of service on
/// macOS, idle priority on Windows. Whether the OS took it.
pub fn idle_current() -> bool {
    idle()
}

#[cfg(target_os = "linux")]
fn idle() -> bool {
    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: sets the calling thread's own policy (pid 0); `param` lives
    // across the call.
    unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) == 0 }
}

#[cfg(target_vendor = "apple")]
fn idle() -> bool {
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) == 0 }
}

#[cfg(windows)]
fn idle() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_IDLE,
    };
    // SAFETY: GetCurrentThread's pseudo handle needs no closing.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE) != 0 }
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn idle() -> bool {
    false
}

/// How many stop requests (Ctrl-C, SIGTERM, SIGHUP) arrived since `catch_stop_signals`.
pub fn stop_requests() -> u32 {
    STOP_REQUESTS.load(std::sync::atomic::Ordering::SeqCst)
}

static STOP_REQUESTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Count stop requests from outside instead of dying on them: vanilla's "Client Shutdown Thread"
/// (`Main.java`) halts the integrated server when the JVM shuts down, and the JVM does that on
/// these signals, so a killed game still saves. The caller's event loop turns the first request
/// into a normal quit; a second one (the save is stuck) ends the process at once.
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
pub fn catch_stop_signals() {
    extern "C" fn handle(_: libc::c_int) {
        if STOP_REQUESTS.fetch_add(1, Ordering::SeqCst) >= 1 {
            // SAFETY: _exit is async-signal-safe; 130 is the shell's code for a Ctrl-C death.
            unsafe { libc::_exit(130) }
        }
    }
    let handler = handle as extern "C" fn(libc::c_int);
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only touches an atomic and calls _exit, both async-signal-safe.
        unsafe {
            libc::signal(signal, handler as libc::sighandler_t);
        }
    }
}

/// Windows and the other systems keep their default: a closed window is a `CloseRequested`.
#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
pub fn catch_stop_signals() {}

/// The game's global allocator (`#[global_allocator]` in its `main.rs`).
/// World generation, meshing and the tick allocate and free small blocks on
/// every worker at once; the system heaps serialise them (Windows' heap was
/// 16% of a chunk's generation time), mimalloc keeps a heap per thread.
pub use mimalloc::MiMalloc as Allocator;

/// `mi_option_purge_delay` in mimalloc.h (index 15 in v2 and v3); the bindings
/// don't name it.
const PURGE_DELAY: libmimalloc_sys::mi_option_t = 15;

/// Set the allocator up, first thing in `main`: free pages go back to the OS
/// at once instead of a second later. With mimalloc's delay the game held a
/// quarter more memory than with the system heap; without it a few percent,
/// and generation keeps most of its speed-up.
pub fn configure_allocator() {
    // SAFETY: sets one integer option, which mimalloc reads at each purge.
    unsafe {
        libmimalloc_sys::mi_option_set(PURGE_DELAY, 0);
    }
}

/// Give back to the OS the memory the allocator holds free, after something
/// large was dropped: mimalloc's free pages, then the system heap's, which
/// native libraries still use: glibc keeps freed pages in the arena of the
/// thread that allocated them, and macOS in its zones until memory runs short.
pub fn release_free_memory() {
    // SAFETY: frees only pages no allocation uses (this thread's heap, shared arenas).
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
    release();
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release() {
    // SAFETY: returns free heap pages of every arena to the OS; no live
    // allocation moves.
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(target_vendor = "apple")]
fn release() {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // SAFETY: a null zone means every zone and a goal of 0 as much as it can;
    // it frees only pages no allocation uses.
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

#[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_vendor = "apple")))]
fn release() {}

/// The REPARSE_DATA_BUFFER of a mount point naming `name` (UTF-16, `\??\` prefixed). The length
/// fields are 16 bits: a name too long for them is an error, so the caller copies instead.
#[cfg(windows)]
fn mount_point_buffer(name: &[u16]) -> std::io::Result<Vec<u8>> {
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    let too_long = || std::io::Error::from(std::io::ErrorKind::InvalidInput);
    let name_bytes = u16::try_from(name.len() * 2).map_err(|_| too_long())?;
    // Substitute name, its NUL, then an empty print name and its NUL.
    let print_offset = name_bytes.checked_add(2).ok_or_else(too_long)?;
    let data_len = name_bytes.checked_add(12).ok_or_else(too_long)?;
    let mut buffer = Vec::with_capacity(16 + name.len() * 2 + 4);
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(data_len.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(name_bytes.to_le_bytes());
    buffer.extend(print_offset.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(name.iter().flat_map(|c| c.to_le_bytes()));
    buffer.extend([0; 4]);
    Ok(buffer)
}

/// A directory junction, which Windows creates without privileges (a directory symlink needs
/// developer mode): the resource pack overlay links its untouched directories with them.
#[cfg(windows)]
pub fn junction(target: &std::path::Path, at: &std::path::Path) -> std::io::Result<()> {
    // What `mklink /J` does, without its ~100 ms `cmd` process per directory: an empty
    // directory given a mount point reparse point (REPARSE_DATA_BUFFER) naming the target.
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    let target = std::path::absolute(target)?;
    let target = target.as_os_str().to_string_lossy();
    let target = target.strip_prefix(r"\\?\").unwrap_or(&target);
    let name: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{target}"))
        .encode_wide()
        .collect();
    let buffer = mount_point_buffer(&name)?;
    std::fs::create_dir(at)?;
    let set = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(at)
        .and_then(|dir| {
            let mut returned = 0;
            // SAFETY: a live directory handle, and `buffer` outlives the call.
            let ok = unsafe {
                windows_sys::Win32::System::IO::DeviceIoControl(
                    dir.as_raw_handle(),
                    FSCTL_SET_REPARSE_POINT,
                    buffer.as_ptr().cast(),
                    buffer.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    if set.is_err() {
        let _ = std::fs::remove_dir(at);
    }
    set
}

/// Keep this process's standard handles out of the processes it starts. Windows hands a child
/// every inheritable handle of its parent, not only the three standard handles it is given
/// (CreateProcessW with bInheritHandles, as `std::process::Command` starts it). horizon-ctl's own
/// standard handles are inheritable when its caller gave it pipes (a script that reads its output,
/// an MCP client), so the game it launches would hold the caller's pipe open for its whole life and
/// the caller, waiting for the end of horizon-ctl's output, would wait until the game quits. A
/// child given `Stdio::inherit()` still gets them: `Command` duplicates the handles it passes.
/// Unix needs none of this: a child's 0-2 are replaced and the rest are close-on-exec.
#[cfg(windows)]
pub fn keep_stdio_from_children() {
    use windows_sys::Win32::Foundation::{
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle has no preconditions; SetHandleInformation only gets a handle this
        // process holds, and when it fails (no console, a pseudo-handle) the handle stays as it was.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// After it, this process's standard output is no longer handed to the processes it starts.
    #[cfg(windows)]
    #[test]
    fn standard_handles_stay_out_of_children() {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
        super::keep_stdio_from_children();
        // SAFETY: reads the flags of this process's own standard output handle.
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut flags = 0;
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && GetHandleInformation(handle, &mut flags) != 0
            {
                assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
            }
        }
    }

    /// The hint holds on the platforms that take it.
    #[test]
    fn favouring_a_thread_takes_where_the_os_allows_it() {
        let took = std::thread::spawn(super::favour).join().unwrap();
        assert_eq!(took, cfg!(any(target_vendor = "apple", windows)));
        #[cfg(target_vendor = "apple")]
        std::thread::spawn(|| {
            assert!(super::favour());
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: reads the calling thread's own QoS class into locals.
            let read = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative)
            };
            assert_eq!(read, 0);
            assert!(matches!(
                class,
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE
            ));
        })
        .join()
        .unwrap();
    }

    /// A helper goes behind only once a dedicated server asks for it, and
    /// then, on Linux and Android, 10 nice levels below the process or in the idle class.
    #[test]
    fn helpers_go_behind_only_when_asked() {
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
        super::put_helpers_behind();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        std::thread::spawn(|| {
            // SAFETY: reads the calling thread's own nice value.
            let before = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) };
            assert!(super::behind_server());
            // SAFETY: reads the calling thread's own nice value and policy.
            let (after, policy) = unsafe {
                (
                    libc::getpriority(libc::PRIO_PROCESS as _, 0),
                    libc::sched_getscheduler(0),
                )
            };
            if before + super::BEHIND <= 19 {
                assert_eq!(after, before + super::BEHIND);
            } else {
                assert_eq!(policy, libc::SCHED_IDLE);
            }
        })
        .join()
        .unwrap();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
    }

    /// The idle hint holds on the three platforms.
    #[test]
    fn idling_a_thread_takes() {
        let took = std::thread::spawn(super::idle_current).join().unwrap();
        assert_eq!(
            took,
            cfg!(any(target_os = "linux", target_vendor = "apple", windows))
        );
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            assert!(super::idle_current());
            // SAFETY: reads the calling thread's own policy.
            assert_eq!(unsafe { libc::sched_getscheduler(0) }, libc::SCHED_IDLE);
        })
        .join()
        .unwrap();
    }

    /// The option set is the purge delay: mimalloc v3's default for it is
    /// 1000 ms, and it is 0 once configured.
    #[test]
    fn configuring_the_allocator_purges_at_once() {
        // SAFETY: reads and sets one integer option.
        let get = || unsafe { libmimalloc_sys::mi_option_get(super::PURGE_DELAY) };
        if std::env::var_os("MIMALLOC_PURGE_DELAY").is_none() {
            assert_eq!(get(), 1000);
        }
        super::configure_allocator();
        assert_eq!(get(), 0);
    }
}

#[cfg(all(test, windows))]
mod junction_tests {
    #[test]
    fn mount_point_buffer_layout() {
        let name: Vec<u16> = r"\??\C:\a".encode_utf16().collect();
        let bytes = u16::try_from(name.len() * 2).unwrap();
        let buffer = super::mount_point_buffer(&name).unwrap();
        assert_eq!(buffer[..4], 0xA000_0003u32.to_le_bytes());
        assert_eq!(buffer[4..6], (bytes + 12).to_le_bytes());
        assert_eq!(buffer[8..10], 0u16.to_le_bytes());
        assert_eq!(buffer[10..12], bytes.to_le_bytes());
        assert_eq!(buffer[12..14], (bytes + 2).to_le_bytes());
        assert_eq!(buffer.len(), 16 + usize::from(bytes) + 4);
        // Past the 8-byte header, the length field counts the rest of the buffer.
        assert_eq!(usize::from(bytes) + 12, buffer.len() - 8);
    }

    #[test]
    fn names_too_long_for_the_length_fields_are_errors() {
        // 32764 units is 65528 bytes: the name fits, its length field sums do not.
        let name = vec![b'a' as u16; 32_764];
        assert!(super::mount_point_buffer(&name).is_err());
        assert!(super::mount_point_buffer(&vec![0; 32_768]).is_err());
        assert!(super::mount_point_buffer(&[0; 100]).is_ok());
    }
}
