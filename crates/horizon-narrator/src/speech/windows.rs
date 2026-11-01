//! `NarratorWindows`: SAPI's `SpVoice`, the voice and rate chosen in Windows'
//! speech settings.
use windows::Win32::Media::Speech::{
    ISpVoice, SPF_ASYNC, SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_HANDLER, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED,
    CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{HSTRING, w};

use std::mem::ManuallyDrop;

pub(super) struct Sapi {
    /// Released before `CoUninitialize` (`drop`).
    voice: ManuallyDrop<ISpVoice>,
}

impl Sapi {
    /// `initSAPI`: `CoCreateInstance(SP_VOICE)` with the in-process and local
    /// server contexts (7). The speech thread is its own multithreaded
    /// apartment: no window messages reach it, and asynchronous speech needs
    /// none.
    pub(super) fn open() -> Result<Sapi, String> {
        // SAFETY: COM is initialised on this thread before any COM call, and
        // released in `drop` on the same thread.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx returned {e}"))?;
            match CoCreateInstance(
                &SpVoice,
                None,
                CLSCTX_INPROC_SERVER | CLSCTX_INPROC_HANDLER | CLSCTX_LOCAL_SERVER,
            ) {
                Ok(voice) => Ok(Sapi {
                    voice: ManuallyDrop::new(voice),
                }),
                Err(e) => {
                    CoUninitialize();
                    Err(format!("SP_VOICE returned code {e}"))
                }
            }
        }
    }
}

impl super::Speech for Sapi {
    /// `Speak` asynchronously as plain text, purging what is queued when
    /// interrupting; the volume is set first, as `(short)(volume * 100.0F)`.
    fn say(&mut self, message: &str, interrupt: bool, volume: f32) {
        let mut flags = (SPF_ASYNC.0 | SPF_IS_NOT_XML.0) as u32;
        if interrupt {
            flags |= SPF_PURGEBEFORESPEAK.0 as u32;
        }
        // SAFETY: `voice` is a live SpVoice of this thread's apartment.
        unsafe {
            let _ = self.voice.SetVolume((volume * 100.0) as i16 as u16);
            if let Err(e) = self.voice.Speak(&HSTRING::from(message), flags, None) {
                log::warn!("Narrator: Speak failed: {e}");
            }
        }
    }
    /// `Skip("Sentence", Integer.MAX_VALUE)`: skips what is left of the
    /// queue.
    fn clear(&mut self) {
        let mut skipped = 0;
        // SAFETY: as in `say`; `skipped` outlives the call.
        unsafe {
            let _ = self.voice.Skip(w!("Sentence"), i32::MAX, &mut skipped);
        }
    }
}

impl Drop for Sapi {
    fn drop(&mut self) {
        // SAFETY: the voice is released before COM is, on its own thread.
        unsafe {
            ManuallyDrop::drop(&mut self.voice);
            CoUninitialize();
        }
    }
}
