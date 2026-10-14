#![recursion_limit = "1024"]

use tracing::metadata::Metadata;
use tracing_subscriber::{
    filter::{FilterFn, LevelFilter},
    fmt::format::Pretty,
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
};
use tracing_web::{performance_layer, MakeConsoleWriter};
use wasm_bindgen::prelude::*;

mod app;
mod components;
mod layout;
mod panic;
mod worker;
// Model has to be public for tests to work
pub mod model;

/// What the crash log has recorded (see `panic`), for tests that check that what was not
/// committed was not logged either: the number of actions, and the size of the import they start
/// from.
#[cfg(debug_assertions)]
#[must_use]
pub fn crash_log_summary() -> (usize, usize) {
    panic::debug_summary()
}

fn tracing_filter(meta: &Metadata<'_>) -> bool {
    meta.target().contains("homotopy")
}

// This is like the `main` function, except for JavaScript.
#[wasm_bindgen(start)]
#[allow(clippy::unnecessary_wraps)]
pub fn main_js() -> Result<(), JsValue> {
    // The same module is run by the web worker that does jobs (see `worker`): there is no page
    // there, and the worker just waits for them.
    if worker::in_worker() {
        tracing_subscriber::registry()
            .with(LevelFilter::DEBUG)
            .with(FilterFn::new(tracing_filter))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .without_time()
                    .with_writer(MakeConsoleWriter),
            )
            .init();
        worker::init();
        return Ok(());
    }

    // Set panic hook.
    // We must use the yew provided method,
    // otherwise yew will override whatever we
    // set with its own handler at start_app!
    yew::set_custom_panic_hook(Box::new(panic::panic_handler));

    // check if we are the main/UI thread
    // setup tracing
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_writer(MakeConsoleWriter);
    let perf_layer = performance_layer().with_details_from_fields(Pretty::default());

    tracing_subscriber::registry()
        .with(LevelFilter::DEBUG)
        .with(FilterFn::new(tracing_filter))
        .with(fmt_layer)
        .with(perf_layer)
        .init();

    yew::Renderer::<app::App>::new().render();
    Ok(())
}
