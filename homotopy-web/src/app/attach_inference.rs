//! The limits on inferring instances of schemas when looking for attachments, and what to say
//! when they got in the way.
//!
//! Looking for what can be attached at a point includes, for each schema (a generator that
//! mentions placeholders), a search for the replacements of its placeholders that make it attach
//! there (see [`AttachOptionsJob`](homotopy_model::proof::AttachOptionsJob)). That is the one
//! part of the search that can take minutes: so it can be turned off, each generator (a schema
//! among them) is given a time to take (and what has been found for it by then is offered), and
//! it is not done at all for diagrams of a dimension that is too high for it, unless that is
//! asked for. These are settings (kept in this browser, see `attach_inference.js`), which go with
//! every search, also to the worker.
//!
//! The time is for *one generator* (matching it and its reflections, or inferring a schema's
//! instances), not for the search as a whole: more generators to look at take longer, as is to be
//! expected, but one that has no end to it does not make the rest wait. A search that gave up on
//! generators says which. Two searches in a row that gave up on *every* schema they looked at
//! turn the inference off *for the session* (not in the settings): then it is the same schemas
//! that are too slow for the same diagram, nothing came of the time spent, and it would be spent
//! again on every click. A search in which some schema finished is not counted, and neither is
//! one for a generator that was asked for by name.

use std::cell::Cell;

use homotopy_model::proof::{InferenceLimits, InferenceReport};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use super::settings::AppSettings;
use crate::components::inference_notice::NoticeLine;

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

/// The version of the settings as they are kept in which the time is for one generator.
const PER_GENERATOR: u32 = 2;

/// The settings as they are kept.
///
/// Before the time limit was for one generator there were two limits, on the whole inference
/// (`budget_seconds`, 20 by default) and on all the plain passes (`plain_budget_seconds`, 10),
/// which are read but no longer written; see [`Kept::generator_seconds`].
#[derive(Serialize, Deserialize)]
struct Kept {
    enabled: bool,
    max_dimension: u32,
    #[serde(default)]
    narrow_to_sheet: bool,
    #[serde(default = "yes")]
    offer_definitions: bool,
    #[serde(default)]
    version: u32,
    #[serde(default)]
    generator_seconds: Option<u32>,
    #[serde(default, skip_serializing)]
    budget_seconds: Option<u32>,
    #[serde(default, skip_serializing)]
    plain_budget_seconds: Option<u32>,
}

impl Kept {
    /// The time for one generator, in seconds (`0` for none).
    ///
    /// The limits that were kept before were totals for the inference and for the plain passes,
    /// which have no meaning for one generator, so what they said is not carried over, except
    /// that someone who had turned both off (`0`) is not given a limit they did not want. Anyone
    /// else gets the default, which is what the old defaults came to for a generator.
    fn generator_seconds(&self) -> u32 {
        if self.version >= PER_GENERATOR {
            self.generator_seconds.unwrap_or(DEFAULT_SECONDS)
        } else if self.budget_seconds == Some(0) && self.plain_budget_seconds == Some(0) {
            0
        } else {
            DEFAULT_SECONDS
        }
    }
}

const fn yes() -> bool {
    true
}

/// The default time for one generator, in seconds.
const DEFAULT_SECONDS: u32 = homotopy_model::proof::DEFAULT_GENERATOR_MS / 1000;

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
        generator_ms: AppSettings::get_generator_budget().saturating_mul(1000),
        max_dimension: if max_dimension >= NO_DIMENSION_LIMIT {
            u32::MAX
        } else {
            max_dimension
        },
        narrow_to_sheet: AppSettings::get_infer_narrow_to_sheet(),
    }
}

/// The limits for the search for one generator that was asked for by name (dropped on the diagram,
/// or picked and clicked): the settings, except that the time for a generator is not limited, as
/// it is not for a click.
///
/// Someone who names a generator wants it looked for, can see that the search is going on (the
/// busy pill, and what it has found listed as it goes along) and can end it at any time (`Esc`,
/// the pill, *Stop*; the project is as it was), so there is no point in giving up for them: the
/// limit in the settings is for the searches that look at everything and must not be held up by
/// one generator that has no end. The rest of the settings (whether to infer instances at all,
/// the dimension up to which to) are theirs still.
#[must_use]
pub fn limits_for_generator() -> InferenceLimits {
    InferenceLimits {
        generator_ms: 0,
        ..limits()
    }
}

/// Takes the settings that were kept into the settings, at the start.
pub fn load() {
    let Some(kept) = read_settings().and_then(|json| serde_json::from_str::<Kept>(&json).ok())
    else {
        return;
    };
    AppSettings::set_infer_instances(kept.enabled);
    AppSettings::set_generator_budget(kept.generator_seconds());
    AppSettings::set_infer_max_dimension(kept.max_dimension.clamp(1, NO_DIMENSION_LIMIT));
    AppSettings::set_infer_narrow_to_sheet(kept.narrow_to_sheet);
    AppSettings::set_offer_definitions(kept.offer_definitions);
}

/// Keeps the settings.
pub fn store() {
    let kept = Kept {
        enabled: AppSettings::get_infer_instances(),
        max_dimension: AppSettings::get_infer_max_dimension(),
        narrow_to_sheet: AppSettings::get_infer_narrow_to_sheet(),
        offer_definitions: AppSettings::get_offer_definitions(),
        version: PER_GENERATOR,
        generator_seconds: Some(AppSettings::get_generator_budget()),
        budget_seconds: None,
        plain_budget_seconds: None,
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

/// What there is to tell the person about the search in one that has just finished, one line for
/// each thing, if anything.
pub fn notice(report: &InferenceReport) -> Vec<NoticeLine> {
    let mut lines = vec![];
    if let Some(dimension) = report.too_high {
        // Once: every click on the diagram would say it otherwise.
        if !TOLD_DIMENSION.with(|told| told.replace(true)) {
            lines.push(NoticeLine::Text(format!(
                "Instances of schemas are not inferred for diagrams of dimension {dimension}, \
                 which is above the limit of {} that the search is stopped at (Settings, \
                 \"Infer instances up to dimension\"). Everything else is still offered. \
                 This is said once.",
                AppSettings::get_infer_max_dimension()
            )));
        }
    }
    let seconds = format!("{} s", (u64::from(report.budget_ms) + 500) / 1000);
    // "after 5 s" for one, "after 5 s each" for several.
    let after = |count: usize| {
        if count == 1 {
            format!("after {seconds}")
        } else {
            format!("after {seconds} each")
        }
    };
    if report.plain.timed_out() {
        let plain = &report.plain;
        if !plain.cut_short.is_empty() {
            lines.push(NoticeLine::Names {
                text: format!(
                    "Gave up on {} {} (Settings, \"Give up on a generator after\")",
                    generators(plain.cut_short.len()),
                    after(plain.cut_short.len()),
                ),
                names: plain.cut_short.clone(),
            });
        }
        if !plain.not_tried.is_empty() {
            lines.push(NoticeLine::Names {
                text: format!("Not tried: {}", generators(plain.not_tried.len())),
                names: plain.not_tried.clone(),
            });
        }
        lines.push(NoticeLine::Text(
            "What was found is valid but may be missing some. To look for one generator, drag it \
             onto the diagram or pick it with the pointing hand; choosing a sheet narrows the search to it."
                .to_owned(),
        ));
        if report.forced && !report.timed_out() {
            lines.push(NoticeLine::Text(FORCED.to_owned()));
        }
    }
    if let Some(narrowed) = report.narrowed {
        lines.push(NoticeLine::Text(format!(
            "Heuristic: instances of schemas were only looked for among the {} of the diagram's {} \
             cells that are near the chosen sheet. Instances that use other cells are not offered. \
             Turn this off in Settings, or choose no sheet, to look at all of them.",
            narrowed.kept, narrowed.of
        )));
    }
    if report.timed_out() {
        if !report.cut_short.is_empty() {
            let count = report.cut_short.len();
            lines.push(NoticeLine::Names {
                text: format!(
                    "Instance inference gave up on {} {}",
                    if count == 1 {
                        "a schema".to_owned()
                    } else {
                        format!("{count} schemas")
                    },
                    after(count)
                ),
                names: report.cut_short.clone(),
            });
            lines.push(NoticeLine::Text(
                "What it had found for them is offered. You can give generators more time, or \
                 turn the inference off, in Settings."
                    .to_owned(),
            ));
        }
        if !report.not_tried.is_empty() {
            lines.push(NoticeLine::Names {
                text: format!("Not tried: {}", generators(report.not_tried.len())),
                names: report.not_tried.clone(),
            });
        }
        if report.forced {
            lines.push(NoticeLine::Text(FORCED.to_owned()));
        }
    }
    if gave_up_on_all(report) {
        let timeouts = TIMEOUTS.with(|timeouts| {
            timeouts.set(timeouts.get() + 1);
            timeouts.get()
        });
        if timeouts >= TIMEOUTS_BEFORE_SUSPENDING {
            AppSettings::set_infer_suspended(true);
            lines.push(NoticeLine::Text(
                "Instances are no longer inferred for the rest of this session, as the search \
                 gave up on every schema it looked at, twice in a row. Turn \"Infer instances \
                 of schemas\" off and on in Settings to start again."
                    .to_owned(),
            ));
        }
    } else if report.too_high.is_none() && !report.targeted {
        TIMEOUTS.with(|timeouts| timeouts.set(0));
    }
    lines
}

const FORCED: &str = "A generator took longer than its time and the search could not stop it, so \
     it was ended: what it had found before then is offered, the generator it was in the middle \
     of is given up on, and the ones after it were not tried.";

fn generators(count: usize) -> String {
    if count == 1 {
        "a generator".to_owned()
    } else {
        format!("{count} generators")
    }
}

/// Whether the inference looked at schemas and gave up on every one of them, in a search for
/// everything at the point (one for a generator asked for by name does not say anything about
/// the schemas in general).
const fn gave_up_on_all(report: &InferenceReport) -> bool {
    !report.targeted && report.inferred == 0 && !report.cut_short.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(json: &str) -> Kept {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn old_totals_are_not_carried_over_as_the_time_for_a_generator() {
        // The old defaults, and any other totals, give the default.
        let old = kept(
            r#"{"enabled":true,"budget_seconds":20,"max_dimension":7,"narrow_to_sheet":false,
                "plain_budget_seconds":10}"#,
        );
        assert_eq!(old.generator_seconds(), DEFAULT_SECONDS);
        let changed = kept(
            r#"{"enabled":true,"budget_seconds":60,"max_dimension":7,"plain_budget_seconds":3}"#,
        );
        assert_eq!(changed.generator_seconds(), DEFAULT_SECONDS);
        // From before the plain passes had a limit at all.
        let oldest = kept(r#"{"enabled":false,"budget_seconds":0,"max_dimension":4}"#);
        assert_eq!(oldest.generator_seconds(), DEFAULT_SECONDS);
        assert!(!oldest.enabled);
        assert_eq!(oldest.max_dimension, 4);
        // No limit on either stays no limit.
        let none = kept(
            r#"{"enabled":true,"budget_seconds":0,"max_dimension":7,"plain_budget_seconds":0}"#,
        );
        assert_eq!(none.generator_seconds(), 0);
    }

    #[test]
    fn the_time_for_a_generator_is_kept_as_it_is_and_the_old_keys_are_not_written() {
        let written = serde_json::to_string(&Kept {
            enabled: true,
            max_dimension: 7,
            narrow_to_sheet: true,
            offer_definitions: false,
            version: PER_GENERATOR,
            generator_seconds: Some(0),
            budget_seconds: Some(20),
            plain_budget_seconds: Some(10),
        })
        .unwrap();
        assert!(!written.contains("budget_seconds"), "{written}");
        let read = kept(&written);
        assert_eq!(read.generator_seconds(), 0);
        assert!(read.narrow_to_sheet);
        // Definitions first is on unless it was turned off.
        assert!(!read.offer_definitions);
        assert!(kept(r#"{"enabled":true,"max_dimension":7}"#).offer_definitions);
        let seven = kept(r#"{"enabled":true,"max_dimension":7,"version":2,"generator_seconds":7}"#);
        assert_eq!(seven.generator_seconds(), 7);
    }

    #[test]
    fn only_a_search_that_gave_up_on_every_schema_counts_towards_suspending() {
        let gave_up = |cut: &[&str], inferred, targeted| InferenceReport {
            cut_short: cut.iter().map(|name| (*name).to_owned()).collect(),
            inferred,
            targeted,
            ..InferenceReport::default()
        };
        assert!(gave_up_on_all(&gave_up(&["a", "b"], 0, false)));
        // Some schema finished: the time was not all for nothing.
        assert!(!gave_up_on_all(&gave_up(&["a"], 3, false)));
        // Nothing was given up on.
        assert!(!gave_up_on_all(&gave_up(&[], 0, false)));
        // Asked for by name.
        assert!(!gave_up_on_all(&gave_up(&["a"], 0, true)));
    }
}
