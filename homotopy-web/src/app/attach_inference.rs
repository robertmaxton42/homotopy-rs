//! The limits on inferring instances of schemas when looking for attachments, and what to say
//! when they got in the way.
//!
//! Looking for what can be attached at a point includes, for each schema (a generator that
//! mentions placeholders), a search for the replacements of its placeholders that make it attach
//! there (see [`AttachOptionsJob`](homotopy_model::proof::AttachOptionsJob)). That is the one
//! part of the search that can take minutes: so it can be turned off, it is given a time to take
//! (and what it has found by then is offered) and it is not done at all for diagrams of a
//! dimension that is too high for it, unless that is asked for. These are settings (kept in this
//! browser, see `attach_inference.js`), which go with every search, also to the worker.
//!
//! A search that went over its time says so, and says which schemas it gave up on. Two in a row
//! turn the inference off *for the session* (not in the settings): it is the same schemas that
//! are too slow for the same diagram nearly always, and a search that takes the whole of its time
//! on every click is worse than one that offers fewer options.

use std::cell::Cell;

use homotopy_model::proof::{InferenceLimits, InferenceReport};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use super::settings::AppSettings;

#[wasm_bindgen(module = "/src/app/attach_inference.js")]
extern "C" {
    #[wasm_bindgen(js_name = "readInferenceSettings")]
    fn read_settings() -> Option<String>;

    #[wasm_bindgen(js_name = "writeInferenceSettings")]
    fn write_settings(json: &str);
}

/// How many searches in a row can run out of time before inference is turned off for the session.
const TIMEOUTS_BEFORE_SUSPENDING: u32 = 2;

/// The highest dimension limit that the slider offers, which means no limit.
pub const NO_DIMENSION_LIMIT: u32 = 12;

/// The settings as they are kept.
#[derive(Serialize, Deserialize)]
struct Kept {
    enabled: bool,
    budget_seconds: u32,
    max_dimension: u32,
}

thread_local! {
    /// How many searches in a row ran out of time.
    static TIMEOUTS: Cell<u32> = const { Cell::new(0) };
    /// Whether the person was told that diagrams of a high dimension are not inferred for.
    static TOLD_DIMENSION: Cell<bool> = const { Cell::new(false) };
}

/// The limits for a search that starts now.
#[must_use]
pub fn limits() -> InferenceLimits {
    let max_dimension = AppSettings::get_infer_max_dimension();
    InferenceLimits {
        enabled: AppSettings::get_infer_instances() && !AppSettings::get_infer_suspended(),
        budget_ms: AppSettings::get_infer_budget().saturating_mul(1000),
        max_dimension: if max_dimension >= NO_DIMENSION_LIMIT {
            u32::MAX
        } else {
            max_dimension
        },
    }
}

/// Takes the settings that were kept into the settings, at the start.
pub fn load() {
    let Some(kept) = read_settings().and_then(|json| serde_json::from_str::<Kept>(&json).ok())
    else {
        return;
    };
    AppSettings::set_infer_instances(kept.enabled);
    AppSettings::set_infer_budget(kept.budget_seconds);
    AppSettings::set_infer_max_dimension(kept.max_dimension.clamp(1, NO_DIMENSION_LIMIT));
}

/// Keeps the settings.
pub fn store() {
    let kept = Kept {
        enabled: AppSettings::get_infer_instances(),
        budget_seconds: AppSettings::get_infer_budget(),
        max_dimension: AppSettings::get_infer_max_dimension(),
    };
    if let Ok(json) = serde_json::to_string(&kept) {
        write_settings(&json);
    }
}

/// Turning the inference on or off in the settings: also ends the suspension.
pub fn set_enabled(on: bool) {
    TIMEOUTS.with(|timeouts| timeouts.set(0));
    AppSettings::set_infer_suspended(false);
    AppSettings::set_infer_instances(on);
}

/// What there is to tell the person about the inference in a search that has just finished, one
/// line for each thing, if anything.
pub fn notice(report: &InferenceReport) -> Vec<String> {
    let mut lines = vec![];
    if let Some(dimension) = report.too_high {
        // Once: every click on the diagram would say it otherwise.
        if !TOLD_DIMENSION.with(|told| told.replace(true)) {
            lines.push(format!(
                "Instances of schemas are not inferred for diagrams of dimension {dimension}, \
                 which is above the limit of {} that the search is stopped at (Settings, \
                 \"Infer instances up to dimension\"). Everything else is still offered. \
                 This is said once.",
                AppSettings::get_infer_max_dimension()
            ));
        }
    }
    if report.timed_out() {
        let seconds = report.spent_ms.div_ceil(1000);
        lines.push(format!(
            "Instance inference took too long and was skipped for this search ({seconds} s). \
             You can turn it off in Settings."
        ));
        let names = |names: &[String]| names.join(", ");
        if !report.cut_short.is_empty() {
            lines.push(format!("Gave up on: {}.", names(&report.cut_short)));
        }
        if !report.not_tried.is_empty() {
            lines.push(format!("Not tried: {}.", names(&report.not_tried)));
        }
        let timeouts = TIMEOUTS.with(|timeouts| {
            timeouts.set(timeouts.get() + 1);
            timeouts.get()
        });
        if timeouts >= TIMEOUTS_BEFORE_SUSPENDING {
            AppSettings::set_infer_suspended(true);
            lines.push(
                "Instances are no longer inferred for the rest of this session, as it ran out of \
                 time again. Turn \"Infer instances of schemas\" off and on in Settings to start \
                 again."
                    .to_owned(),
            );
        }
    } else if report.too_high.is_none() {
        TIMEOUTS.with(|timeouts| timeouts.set(0));
    }
    lines
}
