//! `NarratorLinux`: flite's `cmu_us_kal16` voice, from the system's
//! `libflite` and `libflite_cmu_us_kal16`, opened at run time as JNA does (a
//! system without flite has no narrator, the game still runs).
use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_LAZY};

use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use super::linux_split::units;

/// `cst_voice`, `cst_utterance` and `cst_wave`, only ever handled by pointer.
type Voice = c_void;
type Utterance = c_void;
type Wave = c_void;

pub(super) struct Flite {
    /// The executor's queue: the pieces left to say, in order.
    pieces: Sender<Piece>,
    /// `executionBatch`: a piece is said only if no `clear` came after it was
    /// queued.
    batch: Arc<AtomicI32>,
}

/// One task of `say`: a piece of the message, the batch it belongs to and
/// the volume it is said at.
struct Piece {
    text: CString,
    batch: i32,
    volume: f32,
}

/// `FliteLibrary` and `FliteLibrary.CmuUsKal16`: flite's functions, kept with
/// the libraries that hold them.
struct FliteLibrary {
    _flite: Library,
    _cmu_us_kal16: Library,
    synth_text: unsafe extern "C" fn(*const c_char, *mut Voice) -> *mut Utterance,
    utt_wave: unsafe extern "C" fn(*mut Utterance) -> *mut Wave,
    play_wave: unsafe extern "C" fn(*mut Wave) -> c_int,
    wave_rescale: unsafe extern "C" fn(*mut Wave, c_int),
    delete_utterance: unsafe extern "C" fn(*mut Utterance),
}

/// The registered `cmu_us_kal16` voice, handed to the player thread.
struct VoicePtr(*mut Voice);
// SAFETY: the voice is registered on the narrator thread, then only read by
// `flite_synth_text` on the player thread, which owns it from then on.
unsafe impl Send for VoicePtr {}

impl Flite {
    /// The constructor: loads flite, then the voice; `flite_init` must return
    /// 0 and `register_cmu_us_kal16(null)` a voice. The executor's thread
    /// ("Narrator #0", a daemon: the game never waits for it) speaks.
    pub(super) fn open() -> Result<Flite, String> {
        // SAFETY: each symbol is looked up as the type flite declares it with
        // (`flite.h`, `cst_wave.h`, `cst_utterance.h`, `audio.h`), and the
        // pointers are kept no longer than the libraries they point into.
        let (library, voice) = unsafe {
            // `FliteLibrary.loadNative`: the library and all its functions.
            let flite = load("flite")?;
            let init = function::<unsafe extern "C" fn() -> c_int>(&flite, b"flite_init\0")
                .map_err(failed("flite"))?;
            let synth_text = function(&flite, b"flite_synth_text\0").map_err(failed("flite"))?;
            let utt_wave = function(&flite, b"utt_wave\0").map_err(failed("flite"))?;
            let play_wave = function(&flite, b"play_wave\0").map_err(failed("flite"))?;
            let wave_rescale = function(&flite, b"cst_wave_rescale\0").map_err(failed("flite"))?;
            let delete_utterance =
                function(&flite, b"delete_utterance\0").map_err(failed("flite"))?;
            // `CmuUsKal16.loadNative`.
            let cmu_us_kal16 = load("flite_cmu_us_kal16")?;
            let register = function::<unsafe extern "C" fn(*const c_char) -> *mut Voice>(
                &cmu_us_kal16,
                b"register_cmu_us_kal16\0",
            )
            .map_err(failed("flite_cmu_us_kal16"))?;
            let library = FliteLibrary {
                _flite: flite,
                _cmu_us_kal16: cmu_us_kal16,
                synth_text,
                utt_wave,
                play_wave,
                wave_rescale,
                delete_utterance,
            };
            let rc = init();
            if rc != 0 {
                return Err(format!("flite returned code {rc}"));
            }
            let voice = register(std::ptr::null());
            if voice.is_null() {
                return Err("flite_cmu_us_kal16 failed to register".into());
            }
            (library, VoicePtr(voice))
        };
        let batch = Arc::new(AtomicI32::new(0));
        let (pieces, queue) = mpsc::channel();
        let current = Arc::clone(&batch);
        std::thread::Builder::new()
            .name("Narrator #0".into())
            .spawn(move || play(&library, &voice, &current, &queue))
            .map_err(|e| format!("Failed to start the narrator's player: {e}"))?;
        Ok(Flite { pieces, batch })
    }
}

/// `NativeLibrary.getInstance(name)`: `lib<name>.so`, then the versioned
/// name the runtime packages ship (`libflite1` has no unversioned link),
/// opened with JNA's flags (`RTLD_LAZY | RTLD_GLOBAL`).
fn load(name: &str) -> Result<Library, String> {
    let mut error = None;
    for file in [format!("lib{name}.so"), format!("lib{name}.so.1")] {
        // SAFETY: flite's libraries run no initialisers that need more than
        // the C runtime.
        match unsafe { Library::open(Some(&file), RTLD_LAZY | RTLD_GLOBAL) } {
            Ok(library) => return Ok(library),
            Err(e) => error = Some(e),
        }
    }
    Err(format!(
        "Failed to load library {name}: {}",
        error.map_or_else(String::new, |e| e.to_string())
    ))
}

/// JNA's `Native.register` fails as the library does when a function is
/// missing.
fn failed(name: &'static str) -> impl Fn(libloading::Error) -> String {
    move |e| format!("Failed to load library {name}: {e}")
}

/// A function of `library`, as a plain pointer.
///
/// # Safety
/// `T` must be the function's type, and the pointer must not outlive
/// `library`.
unsafe fn function<T: Copy>(library: &Library, symbol: &[u8]) -> Result<T, libloading::Error> {
    // SAFETY: the caller's.
    unsafe { library.get::<T>(symbol).map(|f| *f) }
}

/// The executor: each piece still in the current batch is synthesised,
/// rescaled to `(int)(volume * 65536.0F)` unless the volume is 1, and played
/// to the end (`play_wave` blocks), so a `clear` lets the piece being said
/// finish and skips the rest.
fn play(library: &FliteLibrary, voice: &VoicePtr, batch: &AtomicI32, queue: &Receiver<Piece>) {
    for piece in queue {
        if piece.batch < batch.load(Ordering::SeqCst) {
            continue;
        }
        // SAFETY: the functions are flite's, alive with `library`; the text
        // is a C string that outlives the call; the wave belongs to the
        // utterance, which is deleted last. A null utterance or wave (flite
        // failed to synthesise) is not handed on.
        unsafe {
            let utterance = (library.synth_text)(piece.text.as_ptr(), voice.0);
            if utterance.is_null() {
                continue;
            }
            let wave = (library.utt_wave)(utterance);
            if !wave.is_null() {
                if piece.volume != 1.0 {
                    (library.wave_rescale)(wave, (piece.volume * 65536.0) as c_int);
                }
                (library.play_wave)(wave);
            }
            (library.delete_utterance)(utterance);
        }
    }
}

impl super::Speech for Flite {
    /// Interrupting clears first; the message is split at punctuation and
    /// each piece queued in the batch current when `say` was called.
    fn say(&mut self, message: &str, interrupt: bool, volume: f32) {
        if interrupt {
            self.clear();
        }
        let batch = self.batch.load(Ordering::SeqCst);
        for unit in units(message) {
            // JNA hands flite the string up to its first NUL.
            let text = unit.split('\0').next().unwrap_or_default();
            let text = CString::new(text).expect("cut at the first NUL");
            let _ = self.pieces.send(Piece {
                text,
                batch,
                volume,
            });
        }
    }
    /// `executionBatch.incrementAndGet()`: the piece being said finishes, the
    /// queued ones are skipped.
    fn clear(&mut self) {
        self.batch.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for Flite {
    /// `destroy`: `executor.shutdownNow()` drops the queued pieces; the one
    /// being played finishes, then the player thread ends with the queue.
    fn drop(&mut self) {
        self.batch.fetch_add(1, Ordering::SeqCst);
    }
}
