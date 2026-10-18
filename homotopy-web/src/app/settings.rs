use web_sys::HtmlInputElement;
use yew::prelude::*;

use super::attach_inference;
use crate::{
    components::{
        delta::CallbackIdx,
        help_tip::{slug, HelpTip},
    },
    declare_settings,
};

declare_settings! {
    pub struct AppSettings {
        show_previews: bool = true,
        weak_units: bool = false,
        mouse_undo: bool = true,
        animated_3d: bool = false,
        contraction_step: u32 = 1,
        merge_exchanges: bool = false,
        // In the 2D view of a diagram of more dimensions: mark where a sheet ends or sheets meet.
        sheet_outlines: bool = true,
        // In the same view: shade the surfaces that have more sheets behind them.
        sheet_shading: bool = true,
        // In the same view: veil what is drawn on a sheet that is not foremost.
        sheet_veil: bool = true,
        autosave: bool = true,
        // A heuristic: a click on one cell offers the cell's definitions first.
        offer_definitions: bool = true,
        infer_instances: bool = true,
        // Seconds one generator may take when searching for attachments; 0 for no limit.
        generator_budget: u32 = 5,
        infer_max_dimension: u32 = 7,
        // A heuristic: only look for instances among the cells near the chosen sheet.
        infer_narrow_to_sheet: bool = false,
        // Set for the session after searches ran out of time, not part of the kept settings.
        infer_suspended: bool = false,

        cubical_subdivision: bool = true,
        dpr_scale: bool = true,
        smooth_time: bool = true,
        subdivision_depth: u32 = 2,
        geometry_samples: u32 = 10,

        orthographic_3d: bool = false,
        specularity: u32 = 25,
        shininess: u32 = 64,
        gamma: u32 = 22,
        animate_singularities: bool = true,
        singularity_duration: u32 = 5,
        geometry_scale: u32 = 10,

        wireframe_3d: bool = false,
        mesh_hidden: bool = false,
        debug_normals: bool = false,
        disable_lighting: bool = false,
        debug_axes: bool = false,
    }
}

#[derive(Properties, Clone, PartialEq, Eq)]
pub struct SettingsProps {}

pub struct SettingsView {
    callback_idxs: Vec<CallbackIdx>,
}

impl Component for SettingsView {
    type Message = AppSettingsMsg;
    type Properties = SettingsProps;

    fn create(ctx: &Context<Self>) -> Self {
        let callback_idxs = AppSettings::subscribe(AppSettings::ALL, ctx.link().callback(|x| x));

        Self { callback_idxs }
    }

    fn update(&mut self, _ctx: &Context<Self>, _msg: Self::Message) -> bool {
        true
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        AppSettings::unsubscribe(AppSettings::ALL, &self.callback_idxs);
    }

    fn view(&self, _ctx: &Context<Self>) -> Html {
        html! {
            <div class="settings">
                <h3>{"General"}</h3>
                <div class="settings__segment">
                    {
                        Self::view_checkbox(
                            "Show previews in signature",
                            AppSettings::get_show_previews(),
                            AppSettings::set_show_previews,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Allow attaching weak units",
                            AppSettings::get_weak_units(),
                            AppSettings::set_weak_units,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Mouse back and forward buttons undo and redo",
                            MOUSE_UNDO_HELP,
                            AppSettings::get_mouse_undo(),
                            AppSettings::set_mouse_undo,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Render 3D diagrams as movies",
                            AppSettings::get_animated_3d(),
                            AppSettings::set_animated_3d,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Merge exchanges automatically",
                            AppSettings::get_merge_exchanges(),
                            AppSettings::set_merge_exchanges,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Show where sheets meet (fans and echoes)",
                            SHEET_OUTLINES_HELP,
                            AppSettings::get_sheet_outlines(),
                            AppSettings::set_sheet_outlines,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Shade surfaces by the sheets behind them",
                            SHEET_SHADING_HELP,
                            AppSettings::get_sheet_shading(),
                            AppSettings::set_sheet_shading,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Veil what is on a back sheet",
                            SHEET_VEIL_HELP,
                            AppSettings::get_sheet_veil(),
                            AppSettings::set_sheet_veil,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Back up my work in this browser",
                            AUTOSAVE_HELP,
                            AppSettings::get_autosave(),
                            AppSettings::set_autosave,
                        )
                    }
                    {Self::view_note(Self::autosave_status())}
                    {
                        Self::view_checkbox_with_help(
                            "Offer definitions first when clicking a cell",
                            DEFINITIONS_HELP,
                            AppSettings::get_offer_definitions(),
                            AppSettings::set_offer_definitions,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Infer instances of schemas when attaching",
                            INFER_HELP,
                            AppSettings::get_infer_instances(),
                            attach_inference::set_enabled,
                        )
                    }
                    {Self::view_note(Self::inference_status())}
                    {
                        Self::view_number(
                            "Give up on a generator after",
                            GENERATOR_BUDGET_HELP,
                            AppSettings::get_generator_budget(),
                            attach_inference::MAX_GENERATOR_SECONDS,
                            "seconds (0 = no limit)",
                            AppSettings::set_generator_budget,
                        )
                    }
                    {
                        Self::view_slider_with(
                            "Infer instances up to dimension",
                            Some(INFER_DIMENSION_HELP),
                            AppSettings::get_infer_max_dimension(),
                            2,
                            attach_inference::NO_DIMENSION_LIMIT,
                            Self::dimension_limit,
                            true,
                            AppSettings::set_infer_max_dimension,
                        )
                    }
                    {
                        Self::view_checkbox_with_help(
                            "Heuristic: infer instances only from cells near the chosen sheet",
                            NARROW_HELP,
                            AppSettings::get_infer_narrow_to_sheet(),
                            AppSettings::set_infer_narrow_to_sheet,
                        )
                    }
                    {
                        Self::view_slider_with(
                            "Contraction step",
                            None,
                            AppSettings::get_contraction_step(),
                            1,
                            7,
                            |n| n.to_string(),
                            true,
                            AppSettings::set_contraction_step,
                        )
                    }
                </div>
                <h3>{"3D renderer"}</h3>
                <div class="settings__segment">
                    <h4>{"Quality"}</h4>
                    {
                        Self::view_checkbox(
                            "Cubical subdivision",
                            AppSettings::get_cubical_subdivision(),
                            AppSettings::set_cubical_subdivision,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Scale by device pixel ratio",
                            AppSettings::get_dpr_scale(),
                            AppSettings::set_dpr_scale,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Smooth in the time axis",
                            AppSettings::get_smooth_time(),
                            AppSettings::set_smooth_time,
                        )
                    }
                    {
                        Self::view_slider(
                            "Subdivision depth",
                            AppSettings::get_subdivision_depth(),
                            AppSettings::set_subdivision_depth,
                            0,
                            6,
                        )
                    }
                    {
                        Self::view_slider(
                            "Geometry samples",
                            AppSettings::get_geometry_samples(),
                            AppSettings::set_geometry_samples,
                            3,
                            15,
                        )
                    }
                </div>
                <div class="settings__segment">
                    <h4>{"Style"}</h4>
                    {
                        Self::view_checkbox(
                            "Orthographic projection",
                            AppSettings::get_orthographic_3d(),
                            AppSettings::set_orthographic_3d,
                        )
                    }
                    {
                        Self::view_slider(
                            "Specularity",
                            AppSettings::get_specularity(),
                            AppSettings::set_specularity,
                            0,
                            100,
                        )
                    }
                    {
                        Self::view_slider(
                            "Shininess",
                            AppSettings::get_shininess(),
                            AppSettings::set_shininess,
                            20,
                            80,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Animate singularities",
                            AppSettings::get_animate_singularities(),
                            AppSettings::set_animate_singularities,
                        )
                    }
                    {
                        Self::view_slider(
                            "Singularity duration",
                            AppSettings::get_singularity_duration(),
                            AppSettings::set_singularity_duration,
                            1,
                            9,
                        )
                    }
                    {
                        Self::view_slider(
                            "4D geometry scale",
                            AppSettings::get_geometry_scale(),
                            AppSettings::set_geometry_scale,
                            5,
                            20,
                        )
                    }
                </div>
                <div class="settings__segment">
                    <h4>{"Debugging"}</h4>
                    {
                        Self::view_checkbox(
                            "Debug wireframe",
                            AppSettings::get_wireframe_3d(),
                            AppSettings::set_wireframe_3d,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Hide mesh",
                            AppSettings::get_mesh_hidden(),
                            AppSettings::set_mesh_hidden,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Debug normals",
                            AppSettings::get_debug_normals(),
                            AppSettings::set_debug_normals,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Disable lighting",
                            AppSettings::get_disable_lighting(),
                            AppSettings::set_disable_lighting,
                        )
                    }
                    {
                        Self::view_checkbox(
                            "Debug axes",
                            AppSettings::get_debug_axes(),
                            AppSettings::set_debug_axes,
                        )
                    }
                </div>
            </div>
        }
    }
}

/// The explanations of the settings, which are shown in a popover after the title (see
/// [`HelpTip`]): one or two short sentences each.
const MOUSE_UNDO_HELP: &str = "Mouse back and forward buttons, and Alt+Left and Alt+Right, undo and redo instead of leaving the page. The browser's own back button undoes too while this is on; turn it off to leave with it.";
const SHEET_OUTLINES_HELP: &str = "In 2D views of higher-dimensional diagrams, draws a fan where sheets meet (a page for each sheet, the one in front on top) with faint echoes of wires that go into sheets behind. Choosing a sheet over the diagram takes away the pages in front of it. Computed from the diagram, not guessed.";
const SHEET_SHADING_HELP: &str = "A surface is drawn darker (up to three steps) the more sheets pass through it, compared with the thinnest surface in view. Changing it redraws the diagram.";
const SHEET_VEIL_HELP: &str = "In 2D views of higher-dimensional diagrams, covers with a pale frosted veil what is drawn on a sheet that something else is in front of elsewhere (a back sheet), so that things that look level but are not can be told apart. Where nothing is veiled, everything is on a front sheet. Changing it redraws the diagram.";
const AUTOSAVE_HELP: &str = "A backup is made a few seconds after you stop changing the project, and offered back after a crash or a closed tab. Turning it off deletes the backups.";
const DEFINITIONS_HELP: &str = "A heuristic. Clicking a cell first offers its definitions (generators one dimension up with that cell as source or target), with a button to search for everything else. It can miss, but never offers what does not attach.";
const INFER_HELP: &str = "Searching for attachments also looks for replacements of placeholders that make a schema attach, which can take long in high dimensions. Off: only cells that need no replacements are offered.";
const GENERATOR_BUDGET_HELP: &str = "The time one generator may take in a search for attachments: matching it, its inverse and reflections and, for a schema, inferring instances. Then the search keeps what it found and moves on (Esc ends the whole search). A search for one generator dragged onto the diagram has no limit.";
const INFER_DIMENSION_HELP: &str = "Diagrams of a higher dimension than this are not searched for instances. The right end of the slider means no limit.";
const NARROW_HELP: &str = "A heuristic. With a sheet chosen on the diagram (Shift+wheel, [ or ]), placeholders are replaced only by cells near that sheet. Faster, but can miss instances; it never offers an invalid one.";

impl SettingsView {
    fn dimension_limit(dimension: u32) -> String {
        if dimension >= attach_inference::NO_DIMENSION_LIMIT {
            "any".to_owned()
        } else {
            dimension.to_string()
        }
    }

    /// What there is to say about inferring instances, if it is not on as usual.
    fn inference_status() -> Option<&'static str> {
        AppSettings::get_infer_suspended().then_some("Turned off for this session because searches ran out of time: turn it off and on again to start again.")
    }

    /// What there is to say about the backups, if anything: when the last was made, or what is
    /// wrong with them.
    fn autosave_status() -> Option<String> {
        let status = crate::autosave::status();
        if !status.enabled {
            return None;
        }
        if let Some(error) = status.error {
            return Some(format!("Backups are not working: {error}"));
        }
        let (time, bytes) = status.saved?;
        let when = crate::autosave::Snapshot {
            session: String::new(),
            time,
            title: None,
            size: bytes,
        }
        .when();
        Some(format!(
            "Last backup at {when} ({} KB).",
            bytes.div_ceil(1024)
        ))
    }

    /// A line of what is the case at the moment, under a setting. (Explanations are in popovers.)
    fn view_note<S: AsRef<str>>(note: Option<S>) -> Html {
        note.map_or_else(
            || html! {},
            |note| html! { <div class="settings__note">{note.as_ref().to_owned()}</div> },
        )
    }

    fn view_help(name: &str, help: Option<&'static str>) -> Html {
        help.map_or_else(
            || html! {},
            |text| {
                html! {
                    <HelpTip id={format!("help-{}", slug(name))} label={name.to_owned()} {text} />
                }
            },
        )
    }

    fn view_checkbox<S>(name: &str, current: bool, setter: S) -> Html
    where
        S: Fn(bool) + 'static,
    {
        Self::view_checkbox_help(name, None, current, setter)
    }

    fn view_checkbox_with_help<S>(name: &str, help: &'static str, current: bool, setter: S) -> Html
    where
        S: Fn(bool) + 'static,
    {
        Self::view_checkbox_help(name, Some(help), current, setter)
    }

    fn view_checkbox_help<S>(
        name: &str,
        help: Option<&'static str>,
        current: bool,
        setter: S,
    ) -> Html
    where
        S: Fn(bool) + 'static,
    {
        html! {
            <div class="settings__toggle-setting">
                <label class="settings__label">
                    <input
                        type="checkbox"
                        checked={current}
                        onclick={Callback::from(move |_| setter(!current))}
                    />
                    {name}
                </label>
                {Self::view_help(name, help)}
            </div>
        }
    }

    /// A slider with no explanation, whose number is shown while it is dragged and which takes
    /// effect when it is let go (for settings that make something expensive happen).
    fn view_slider<S>(name: &str, current: u32, setter: S, min: u32, max: u32) -> Html
    where
        S: Fn(u32) + 'static,
    {
        Self::view_slider_with(
            name,
            None,
            current,
            min,
            max,
            |n| n.to_string(),
            false,
            setter,
        )
    }

    /// A slider that shows its number as it is dragged (`oninput`). With `live` the setting also
    /// takes effect as it is dragged, which is for settings that cost nothing to change; without,
    /// it takes effect when the slider is let go (`onchange`).
    #[allow(clippy::too_many_arguments)]
    fn view_slider_with<S>(
        name: &str,
        help: Option<&'static str>,
        current: u32,
        min: u32,
        max: u32,
        format: fn(u32) -> String,
        live: bool,
        setter: S,
    ) -> Html
    where
        S: Fn(u32) + 'static,
    {
        let value = NodeRef::default();
        let setter = std::rc::Rc::new(setter);
        let read = move |input: HtmlInputElement| {
            input.value().parse::<u32>().unwrap_or(min).clamp(min, max)
        };
        let oninput = {
            let (value, setter) = (value.clone(), setter.clone());
            Callback::from(move |e: InputEvent| {
                let updated = read(e.target_unchecked_into());
                show_value(&value, &format(updated));
                if live {
                    setter(updated);
                }
            })
        };
        let onchange = Callback::from(move |e: Event| setter(read(e.target_unchecked_into())));

        html! {
            <div class="settings__slider-setting">
                <div class="settings__setting-title">
                    <label class="settings__label" for={format!("setting-{}", slug(name))}>{name}</label>
                    {Self::view_help(name, help)}
                    <output class="settings__value" for={format!("setting-{}", slug(name))} ref={value}>
                        {format(current)}
                    </output>
                </div>
                <input
                    id={format!("setting-{}", slug(name))}
                    type="range"
                    min={min.to_string()}
                    max={max.to_string()}
                    value={current.to_string()}
                    {oninput}
                    {onchange}
                />
            </div>
        }
    }

    /// A number in a box, in `0..=max`, for a setting that needs more than a slider can give.
    /// What is typed takes effect as soon as it is a number (clamped to the range, negative
    /// numbers being 0 and fractions rounded down), and the box is put right when it is left.
    fn view_number<S>(
        name: &str,
        help: &'static str,
        current: u32,
        max: u32,
        unit: &'static str,
        setter: S,
    ) -> Html
    where
        S: Fn(u32) + 'static,
    {
        let setter = std::rc::Rc::new(setter);
        let oninput = {
            let setter = setter.clone();
            Callback::from(move |e: InputEvent| {
                let input: HtmlInputElement = e.target_unchecked_into();
                if let Some(seconds) = parse_count(&input.value(), max) {
                    setter(seconds);
                }
            })
        };
        let onchange = Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            // What is not a number goes back to what the setting is.
            let seconds = parse_count(&input.value(), max).unwrap_or(current);
            setter(seconds);
            input.set_value(&seconds.to_string());
        });

        html! {
            <div class="settings__number-setting">
                <div class="settings__setting-title">
                    <label class="settings__label" for={format!("setting-{}", slug(name))}>{name}</label>
                    {Self::view_help(name, Some(help))}
                </div>
                <div class="settings__number-box">
                    <input
                        id={format!("setting-{}", slug(name))}
                        type="number"
                        inputmode="numeric"
                        min="0"
                        max={max.to_string()}
                        step="1"
                        value={current.to_string()}
                        {oninput}
                        {onchange}
                    />
                    <span class="settings__unit">{unit}</span>
                </div>
            </div>
        }
    }
}

/// Writes the number next to a slider while it is dragged, into the text that is already there
/// (the page's own text node, which it goes on updating).
fn show_value(output: &NodeRef, text: &str) {
    if let Some(node) = output.get().and_then(|output| output.first_child()) {
        node.set_text_content(Some(text));
    }
}

/// A whole number in `0..=max` from what was typed: negative numbers are 0, fractions are rounded
/// down and larger numbers are `max`. `None` if it is not a number (or is empty).
fn parse_count(text: &str, max: u32) -> Option<u32> {
    let number: f64 = text.trim().parse().ok()?;
    if !number.is_finite() {
        return None;
    }
    // (Clamped, so the conversion cannot overflow; `as` rounds towards zero.)
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(number.clamp(0.0, f64::from(max)) as u32)
}

#[cfg(test)]
mod tests {
    use super::parse_count;

    #[test]
    fn numbers_typed_into_a_box_are_made_valid() {
        assert_eq!(parse_count("5", 3600), Some(5));
        assert_eq!(parse_count(" 12 ", 3600), Some(12));
        assert_eq!(parse_count("0", 3600), Some(0));
        assert_eq!(parse_count("007", 3600), Some(7));
        // Negative: none. Fractions: rounded down. Large: the most.
        assert_eq!(parse_count("-4", 3600), Some(0));
        assert_eq!(parse_count("2.9", 3600), Some(2));
        assert_eq!(parse_count("99999", 3600), Some(3600));
        assert_eq!(parse_count("1e9", 3600), Some(3600));
        // Not numbers.
        assert_eq!(parse_count("", 3600), None);
        assert_eq!(parse_count("-", 3600), None);
        assert_eq!(parse_count("abc", 3600), None);
        assert_eq!(parse_count("inf", 3600), None);
        assert_eq!(parse_count("NaN", 3600), None);
    }
}
