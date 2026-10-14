// This file is included in `attach_inference.rs`.
//
// The settings for inferring instances of schemas while looking for attachments are kept in this
// browser from one visit to the next (the other settings are not), because they are about what
// the person can bear to wait for, which does not change between visits. Everything is in `try`:
// a private window or blocked site data means that they are not remembered, and nothing else.

const KEY = "homotopy-inference-settings";

// The settings as JSON, or `null` if none were kept.
export function readInferenceSettings() {
  try {
    return window.localStorage.getItem(KEY);
  } catch (error) {
    return null;
  }
}

export function writeInferenceSettings(json) {
  try {
    window.localStorage.setItem(KEY, json);
  } catch (error) {
    // Not remembered.
  }
}
