use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt::Write as _,
    io::Write,
    sync::Mutex,
};

use homotopy_model::proof::{self, ProofState};
use wasm_bindgen::prelude::*;
use zip::write::{FileOptions, ZipWriter};

use crate::model::{generate_download, ModelError};

// This file contains all the disgusting
// panic handling logic which is responsible
// for producing crash dumps.
// Out of sight, out of mind, don't break it.
//
// What is done about a crash, in the order in which it is done by `panic_handler`:
//
// 1. The page is told (`display_panic_message`, in `index.html`) to show the crash message.
// 2. The project as it was after the last change that went through is made into a `.hom` file,
//    next to a short report (the panic message and place, the version, the last actions that
//    were started), and both are handed to `offer_recovery`, which makes them downloads in the
//    crash message. After a panic the wasm instance cannot be relied on, so the downloads are
//    made by plain JavaScript from bytes it has been given: nothing in them calls back in.
//
// `remember` keeps the last good project (a clone of `ProofState`: all of it is shared with
// the history, so it costs a few pointers and no serialisation until there is a crash).

thread_local! {
    /// The project after the last change that went through, for a crash to save.
    static LAST_GOOD: RefCell<Option<ProofState>> = const { RefCell::new(None) };
    /// The last few actions that were started (what they were, cut short), for the report.
    static RECENT: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    /// Whether the crash was dealt with already (a second panic while doing so is not).
    static HANDLED: Cell<bool> = const { Cell::new(false) };
}

/// How many of the last actions the report names, and how much of each.
const RECENT_ACTIONS: usize = 12;
const ACTION_CHARS: usize = 400;

/// Remembers the project as it is, as the one to save if the app crashes before the next change.
pub fn remember(proof: &ProofState) {
    // Not an error if it is in use (a crash is being dealt with): the older one stays.
    LAST_GOOD
        .try_with(|last| {
            if let Ok(mut last) = last.try_borrow_mut() {
                *last = Some(proof.clone());
            }
        })
        .ok();
}

/// Notes an action that is being started, for the report if the app crashes in it.
pub fn note_action(action: &dyn std::fmt::Debug) {
    /// Writes no more than the room it has.
    struct Capped(String);
    impl std::fmt::Write for Capped {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let room = ACTION_CHARS.saturating_sub(self.0.chars().count());
            self.0.extend(s.chars().take(room));
            if s.chars().count() > room {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }
    }
    let mut text = Capped(String::new());
    if write!(text, "{action:?}").is_err() {
        text.0.push_str("...");
    }
    RECENT
        .try_with(|recent| {
            if let Ok(mut recent) = recent.try_borrow_mut() {
                if recent.len() == RECENT_ACTIONS {
                    recent.pop_front();
                }
                recent.push_back(text.0);
            }
        })
        .ok();
}

pub fn panic_handler(info: &std::panic::PanicHookInfo<'_>) {
    display_panic_message();

    // This provides better error messages in debug mode.
    // It's disabled in release mode so it doesn't bloat up the file size.
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::hook(info);

    let location = info
        .location()
        .map_or_else(String::new, |location| format!(" at {location}"));
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned());
    recover(&format!("panic: {message}{location}"));
}

/// Hands the crash message the project to save and a report, if that has not been done.
fn recover(what: &str) {
    if HANDLED
        .try_with(|handled| handled.replace(true))
        .unwrap_or(true)
    {
        return;
    }
    let project = LAST_GOOD
        .try_with(|last| last.try_borrow().ok().and_then(|last| last.clone()))
        .ok()
        .flatten()
        .map(|proof| {
            crate::model::serialize::serialize(proof.signature, proof.workspace, proof.metadata)
        });

    let mut report = format!(
        "homotopy.io {}\n{what}\n\nThe last actions started, oldest first:\n",
        option_env!("GIT_DESCRIBE").unwrap_or(env!("CARGO_PKG_VERSION")),
    );
    RECENT
        .try_with(|recent| {
            if let Ok(recent) = recent.try_borrow() {
                for action in recent.iter() {
                    report.push_str(action);
                    report.push('\n');
                }
            }
        })
        .ok();

    offer_recovery(project.as_deref().unwrap_or_default(), &report);
}

/// For something that went wrong without a panic (JavaScript that was called from here threw
/// and nothing caught it, which leaves the app in a state that nothing can be done with): the
/// page calls this to be given the project all the same.
#[wasm_bindgen]
pub fn recover_after_error(what: &str) {
    recover(what);
}

#[derive(Default)]
struct CrashDump {
    actions: Vec<String>,
    import: Option<Vec<u8>>,
}

static CRASH_INFO: Mutex<CrashDump> = Mutex::new(CrashDump::new());

impl CrashDump {
    const fn new() -> Self {
        Self {
            actions: Vec::new(),
            import: None,
        }
    }

    fn push_action(&mut self, action: &proof::Action) {
        if let proof::Action::ImportProof(buf) = action {
            self.actions.clear();
            self.import = Some(buf.0.clone());
        } else {
            // Temporarily disabled due to memory overflows on large proofs
            /*
                let data = serde_json::to_string(&action).expect("Failed to serialize action.");
                web_sys::console::log(&js_sys::Array::from(&JsValue::from_str(&data)));
                self.actions.push(data);
            */
        }
    }

    fn pop_action(&mut self) -> bool {
        match (self.actions.pop().is_some(), self.import.is_some()) {
            (true, _) => true,
            (false, true) => {
                self.import = None;
                false
            }
            (false, false) => false,
        }
    }

    const fn needs_zip(&self) -> bool {
        self.import.is_some()
    }

    fn get_dump(&self, safe: bool) -> Option<Vec<u8>> {
        let mut actions: Vec<u8> = Vec::new();
        if safe {
            actions.extend(b"[true,[");
        } else {
            actions.extend(b"[false,[");
        }
        actions.extend(self.actions.join(",").as_bytes());
        actions.extend(b"]]");

        if let Some(ibuf) = &self.import {
            let mut buf: Vec<u8> = vec![0; ibuf.len() + actions.len()];
            let size = {
                let mut zip = ZipWriter::new(std::io::Cursor::new(&mut *buf));
                let options =
                    FileOptions::default().compression_method(zip::CompressionMethod::DEFLATE);

                zip.start_file("crash_last_import.hom", options).ok()?;
                let mut j = 0;
                while j < ibuf.len() {
                    j += zip.write(&ibuf[j..]).ok()?;
                }

                zip.start_file("crash_action_dump.txt", options).ok()?;

                let mut j = 0;
                while j < actions.len() {
                    j += zip.write(&actions[j..]).ok()?;
                }
                zip.flush().ok()?;
                zip.finish().ok()?.position()
            };
            buf.resize(size as usize, 0);
            Some(buf)
        } else {
            Some(actions)
        }
    }
}

pub fn push_action(action: &proof::Action) {
    CRASH_INFO.lock().unwrap().push_action(action);
}

pub fn pop_action() -> bool {
    CRASH_INFO.lock().unwrap().pop_action()
}

pub fn needs_zip() -> bool {
    CRASH_INFO.lock().unwrap().needs_zip()
}

pub fn get_dump(safe: bool) -> Option<Vec<u8>> {
    CRASH_INFO.lock().unwrap().get_dump(safe)
}

pub fn export_dump(safe: bool) -> Result<(), ModelError> {
    let data = get_dump(safe).ok_or(ModelError::Internal)?;
    if needs_zip() {
        generate_download("homotopy_io_state", "zip", &data).map_err(ModelError::Export)
    } else {
        generate_download("homotopy_io_actions", "txt", &data).map_err(ModelError::Export)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen]
    pub fn display_panic_message();

    /// Makes the project (empty if there was none) and the report downloads in the crash
    /// message. Both are copied at once.
    #[wasm_bindgen]
    pub fn offer_recovery(project: &[u8], report: &str);
}

/// How many actions the crash dump has logged, and how big the import it starts from is, for
/// checking that a cancelled job left the log alone.
#[cfg(debug_assertions)]
pub fn debug_summary() -> (usize, usize) {
    let info = CRASH_INFO.lock().unwrap();
    (info.actions.len(), info.import.as_ref().map_or(0, Vec::len))
}
