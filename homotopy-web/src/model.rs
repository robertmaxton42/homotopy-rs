use std::{collections::VecDeque, io::Write};

pub use history::Proof;
use history::{History, UndoState};
use homotopy_core::{common::Generator, signature::Signature as _, Diagram, SliceIndex};
use homotopy_graphics::{manim, stl, svg, tikz};
pub use homotopy_model::{history, job, migration, proof, serialize};
use homotopy_model::{
    job::run_to_end,
    proof::{
        ActionsJob, AttachOption, AttachOptionsJob, DraftEdit, Drafts, InferenceLimits,
        InferenceReport, NoAttachment,
    },
};
use serde::Serialize;
use thiserror::Error;
use wasm_bindgen::JsCast;

pub use self::task::{ActionsKind, ExportJob, ExportOutput, Task, TaskOutput};
use crate::app::{
    account,
    diagram_gl::{FrameCaptureControl, FRAME_CAPTURE},
    settings::AppSettings,
};

mod task;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Action {
    Proof(proof::Action),
    History(history::Action),
    ImportActions(proof::SerializedData),
    ExportProof,
    ExportActions,

    ExportImage(ImageFormat, ImageOption),

    Select(usize),
    ClearSelections,
    Merge(Generator),
    /// Click on a point of the workspace, to find what can be attached there: through weak units
    /// if the flag is set, and with the limits on inferring instances of schemas.
    SelectPoint(Vec<SliceIndex>, bool, InferenceLimits),
    /// Drop (or click, once armed) a generator on a point of the workspace, to find only the ways
    /// to attach that generator there: as for [`SelectPoint`](Self::SelectPoint) otherwise.
    AttachGenerator(Generator, Vec<SliceIndex>, bool, InferenceLimits),
    /// Pick a generator to attach, which the next click on the workspace attaches, or put it
    /// down (`None`). Not part of the project.
    Arm(Option<Generator>),
    HighlightAttachment(Option<AttachOption>),
    HighlightSlice(Option<SliceIndex>),

    /// Change an instantiation in progress (see [`Drafts`]). Not part of the project.
    Draft(DraftEdit),

    SetRemoteProjectMetadata(Option<account::RemoteProjectMetadata>),

    Help,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize)]
pub enum ImageFormat {
    // 3d formats
    Png,
    Stl(stl::StlOptions),

    // 2d formats
    Svg,
    Tikz(tikz::TikzOptions),
    Manim(manim::ManimOptions),
}

impl ImageFormat {
    const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Stl(_) => "stl",
            Self::Svg => "svg",
            Self::Tikz(_) => "tex",
            Self::Manim(_) => "py",
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize)]
pub enum ImageOption {
    // Export a single image
    Single,

    // Export a zip file containing multiple images
    Multiple,
}

impl Action {
    /// Determines if a given [Action] is valid given the current [Proof].
    #[must_use]
    pub fn is_valid(&self, proof: &Proof) -> bool {
        match self {
            Self::Proof(action) => action.is_valid(proof),
            Self::History(history::Action::Move(dir)) => proof.can_move(dir),
            _ => true,
        }
    }

    /// Determines if a given [Action] is experimental.
    #[must_use]
    pub const fn is_experimental(&self) -> bool {
        match self {
            Self::Proof(action) => action.is_experimental(),
            _ => false,
        }
    }
}

impl Action {
    /// What the action does, as a gerund phrase, if it can take noticeably long on a realistic
    /// project (see the policy in `CLAUDE.md`). Such actions are made into [`Task`]s that the app
    /// runs in the background behind a busy indicator, and that it can cancel.
    #[must_use]
    pub const fn heavy_label(&self) -> Option<&'static str> {
        match self {
            Self::Proof(action) => action.work_label(),
            Self::ImportActions(_) => Some(proof::REPLAYING),
            Self::SelectPoint(..) | Self::AttachGenerator(..) => Some("Searching for attachments"),
            Self::Draft(_) => Some("Inferring replacements"),
            Self::ExportImage(
                ImageFormat::Stl(_) | ImageFormat::Tikz(_) | ImageFormat::Manim(_),
                _,
            ) => Some("Exporting"),
            _ => None,
        }
    }

    /// Whether the action may be performed while a task is running. Everything that changes the
    /// project or reads it is refused, rather than put off until the task is done: the task is
    /// working on the project as it was, and the person who asked for something else did so
    /// without knowing that. What is allowed does not touch the project: highlighting what the
    /// pointer is over, and housekeeping.
    #[must_use]
    pub const fn allowed_while_busy(&self) -> bool {
        matches!(
            self,
            Self::HighlightAttachment(_)
                | Self::HighlightSlice(_)
                | Self::SetRemoteProjectMetadata(_)
                | Self::Help
        )
    }
}

impl From<proof::Action> for Action {
    fn from(action: proof::Action) -> Self {
        Self::Proof(action)
    }
}

impl From<history::Action> for Action {
    fn from(action: history::Action) -> Self {
        Self::History(action)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selectables {
    Merge(Generator, Vec<Generator>),
    Attach(Vec<AttachOption>),
}

impl Selectables {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Attach(_) => "Attach",
            Self::Merge(_, _) => "Merge",
        }
    }
}

/// What starting to update the state came to.
pub enum Started {
    /// It is done; whether the project changed.
    Done(bool),
    /// There is work to do first: see [`Task`].
    Running(Task),
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub remote_project_metadata: Option<account::RemoteProjectMetadata>,
    pub history: History,
    pub options: Option<Selectables>,
    /// The generator that the next click on the workspace is to attach, if one was picked
    /// (see [`Action::Arm`]).
    pub armed: Option<Generator>,
    pub attachment_highlight: Option<AttachOption>,
    pub slice_highlight: Option<SliceIndex>,
    /// The instantiations in progress, which are not part of the project: neither undo nor redo
    /// touches them, and what they infer is a job like the others.
    pub drafts: Drafts,
    /// What the last search for attachments has to say about inferring instances of schemas,
    /// for the app to pass on to whoever clicked (see [`take_inference_report`](Self::take_inference_report)).
    inference_report: Option<InferenceReport>,
}

impl State {
    /// What the search for attachments that was finished last said about how the inference of
    /// instances went, if it has not been asked for yet.
    pub const fn take_inference_report(&mut self) -> Option<InferenceReport> {
        self.inference_report.take()
    }

    #[inline]
    #[must_use]
    pub fn proof(&self) -> &Proof {
        self.history.proof()
    }

    /// Determines if a given [Action] should reset the panzoom state, given the current [State].
    #[must_use]
    pub fn resets_panzoom(&self, action: &Action) -> bool {
        match action {
            Action::Proof(action) => self.proof().resets_panzoom(action),
            _ => false,
        }
    }

    /// Update the state in response to an [Action], doing all the work there and then.
    ///
    /// The app does not use this for actions that take long, but starts them with
    /// [`start`](Self::start) and runs the tasks they need in the background.
    pub fn update(&mut self, action: Action) -> Result<bool, ModelError> {
        let mut started = self.start(action)?;
        loop {
            match started {
                Started::Done(changed) => return Ok(changed),
                Started::Running(task) => started = self.finish(run_to_end(task))?,
            }
        }
    }

    /// Starts to update the state in response to an [Action]. This is either all that there is
    /// to it, or a [`Task`] that does the rest of the work without touching the state, whose
    /// output [`finish`](Self::finish) then commits.
    pub fn start(&mut self, action: Action) -> Result<Started, ModelError> {
        match action {
            Action::Proof(action) => {
                // Merging is its own history step (rather than being folded into the homotopy's)
                // so that the recorded actions replay to the same proof when exported. It cannot
                // loop, as `MergeExchanges` is not a `Homotopy`.
                let merge = matches!(action, proof::Action::Homotopy(_))
                    && AppSettings::get_merge_exchanges();
                let steps =
                    std::iter::once(action).chain(merge.then_some(proof::Action::MergeExchanges));
                return Ok(self.start_proof_actions(steps.collect()));
            }

            Action::History(history::Action::Move(dir)) => {
                use homotopy_core::Direction::{Backward, Forward};
                match dir {
                    history::Direction::Linear(Forward) => {
                        self.history.redo()?;
                        if let Some(action) = self.history.last_action() {
                            crate::panic::push_action(&action);
                        }
                    }
                    history::Direction::Linear(Backward) => {
                        self.history.undo()?;
                        // The action we just undid is an ImportProof
                        // So we need to reimport the context before
                        // that into the panic handler.
                        if !crate::panic::pop_action() {
                            for a in self.history.get_last_import_segment() {
                                crate::panic::push_action(&a);
                            }
                        }
                    }
                }
                self.clear_selections();
            }

            Action::ExportImage(ImageFormat::Svg, option) => {
                assert_eq!(
                    option,
                    ImageOption::Single,
                    "Multiple SVG export is not supported.",
                );

                let signature = &self.proof().signature;

                // First we locate the element containing the SVG rendered the SVG rendering
                // pipeline. We *could* do this by using a lookup by class name, which would not
                // require any chances to components/panzoom.rs, but get_elements_by_class_name
                // has a return type that requires a feature of web-sys that is not
                // currently activated. Thus this solution avoids increases in build times.
                let svg_element = web_sys::window()
                    .expect("no window")
                    .document()
                    .expect("no document")
                    .get_element_by_id("panzoom__inner__0")
                    .expect("no SVG in document");
                let svg = svg_element.inner_html();

                // We must now pull all the relevant stylesheets that are needed in the SVG.
                // Failure to do so gives a fully-black SVG. We can generate the stylesheet in the
                // same way the `SignatureStylesheet` struct does.
                // We also strip the styles of whitespace since it is unneeded.
                let stylesheet = {
                    let mut inner_stylesheet = svg::stylesheet(signature);
                    inner_stylesheet.retain(|c| !c.is_whitespace());
                    format!("<style>{inner_stylesheet}</style>")
                };

                // So we now have the SVG and its stylesheet in separate strings.
                // It is not enough to just concatenate them, the stylesheets need to be inside the
                // root element. Since we know that this will have form
                // <svg.*><.*
                // We just look for the first >< and use some very light indexing maths to insert
                // the stylesheet in the appropriate place.
                // Again, the function would be half as long and twice as clean if the SVG and the SVG's
                // stylesheet were in the same place in the DOM.
                let content_start = svg.find("><").unwrap() + 1;
                let mut data = String::new();
                data.push_str(&svg[..content_start]);
                data.push_str(&stylesheet);
                data.push_str(&svg[content_start..]);

                generate_download("homotopy_io_export", "svg", data.as_bytes())
                    .map_err(ModelError::Export)?;
            }

            Action::ExportImage(ImageFormat::Png, option) => {
                let Some(ws) = self.proof().workspace.as_ref() else {
                    return Ok(Started::Done(false));
                };
                let Diagram::DiagramN(diagram) = ws.visible_diagram() else {
                    return Ok(Started::Done(false));
                };
                let control = match option {
                    ImageOption::Single => FrameCaptureControl::One,
                    ImageOption::Multiple => FrameCaptureControl::All(diagram.size() as u16 * 60),
                };
                tracing::info!("Exporting PNG with control: {:?}", control);
                FRAME_CAPTURE.with(|fc| {
                    fc.borrow().emit(control);
                });
            }

            Action::ExportImage(format, ImageOption::Single) => {
                let signature = self.proof().signature.clone();
                let Some(ws) = self.proof().workspace.as_ref() else {
                    return Ok(Started::Done(false));
                };

                return Ok(Started::Running(Task::Export(ExportJob::single(
                    signature,
                    format,
                    ws.visible_diagram(),
                    ws.view.dimension(),
                ))));
            }

            Action::ExportImage(format, ImageOption::Multiple) => {
                let signature = self.proof().signature.clone();
                let Some(ws) = self.proof().workspace.as_ref() else {
                    return Ok(Started::Done(false));
                };

                let Diagram::DiagramN(diagram) = ws.visible_diagram() else {
                    return Ok(Started::Done(false));
                };
                let view_dimension = ws.view.dimension().min(diagram.dimension() as u8 - 1);

                return Ok(Started::Running(Task::Export(ExportJob::slices(
                    signature,
                    format,
                    &diagram,
                    view_dimension,
                ))));
            }

            Action::ExportActions => {
                crate::panic::export_dump(true)?;
            }

            Action::ExportProof => {
                let data = serialize::serialize(
                    self.proof().signature.clone(),
                    self.proof().workspace.clone(),
                    self.proof().metadata.clone(),
                );
                generate_download("homotopy_io_export", "hom", data.as_slice())
                    .map_err(ModelError::Export)?;
            }

            Action::ImportActions(data) => {
                let (safe, actions): (bool, Vec<proof::Action>) =
                    serde_json::from_slice(&data.0)
                        .or(Err(ModelError::Proof(proof::ProofError::Import)))?;
                let len = if safe {
                    actions.len()
                } else {
                    actions.len().saturating_sub(1)
                };

                // Replay actions in top of workspace
                return Ok(Started::Running(Task::Actions {
                    job: Box::new(ActionsJob::replay(
                        self.proof().clone(),
                        actions.into_iter().take(len),
                        proof::REPLAYING,
                    )),
                    kind: ActionsKind::Import,
                    redone: false,
                }));
            }

            Action::Select(index) => {
                let action = match self.options.as_ref() {
                    // Select a generator.
                    None => proof::Action::SelectGenerator(
                        self.proof()
                            .signature
                            .iter()
                            .nth(index)
                            .ok_or(ModelError::IndexOutOfBounds)?
                            .generator,
                    ),
                    // Select an attachment option.
                    Some(Selectables::Attach(att)) => proof::Action::Attach(
                        att.get(index).ok_or(ModelError::IndexOutOfBounds)?.clone(),
                    ),
                    Some(Selectables::Merge(from, tos)) => proof::Action::Merge(
                        *from,
                        *tos.get(index).ok_or(ModelError::IndexOutOfBounds)?,
                    ),
                };
                return self.start(Action::Proof(action));
            }
            Action::SelectPoint(point, weak_units, limits) => {
                return self.start_attach_search(&point, weak_units, limits, None);
            }
            Action::AttachGenerator(generator, point, weak_units, limits) => {
                // Whatever came of it, the generator is not waiting any more.
                self.armed = None;
                return self.start_attach_search(&point, weak_units, limits, Some(generator));
            }
            Action::Arm(generator) => {
                self.clear_selections();
                self.armed = generator;
                // Picking is not changing the project.
                return Ok(Started::Done(false));
            }
            Action::Draft(edit) => {
                let signature = &self.history.proof().signature;
                if let Some(job) = self.drafts.start(signature, edit) {
                    return Ok(Started::Running(Task::Infer(Box::new(job))));
                }
                // Choosing is not changing the project.
                return Ok(Started::Done(false));
            }
            Action::HighlightAttachment(option) => self.highlight_attachment(option),
            Action::HighlightSlice(slice) => self.highlight_slice(slice),
            Action::ClearSelections => self.clear_selections(),
            Action::Merge(generator) => self.merge_options(generator),
            Action::SetRemoteProjectMetadata(metadata) => {
                self.set_remote_project_metadata(metadata);
            }
            Action::Help => help()?,
        }

        Ok(Started::Done(true))
    }

    /// Starts the search for the ways to attach at a point of the workspace, all of them or those
    /// of one generator.
    fn start_attach_search(
        &self,
        point: &[SliceIndex],
        weak_units: bool,
        limits: InferenceLimits,
        only: Option<Generator>,
    ) -> Result<Started, ModelError> {
        match AttachOptionsJob::new(self.proof(), point, weak_units, limits, only) {
            Ok(job) => Ok(Started::Running(Task::AttachOptions(Box::new(job)))),
            Err(NoAttachment::NoWorkspace) => Ok(Started::Done(true)),
            Err(NoAttachment::NoBoundary | NoAttachment::UnknownGenerator) => {
                Err(ModelError::NoAttachment)
            }
        }
    }

    /// Starts to perform actions on the proof, each following the one before. Those that were
    /// performed before from here are just redone, which takes no time; the rest is a task.
    fn start_proof_actions(&mut self, actions: Vec<proof::Action>) -> Started {
        let mut actions = VecDeque::from(actions);
        let mut redone = false;
        while let Some(action) = actions.front() {
            if self.history.try_redo(action).is_err() {
                break;
            }
            crate::panic::push_action(action);
            self.clear_selections();
            redone = true;
            actions.pop_front();
        }
        if actions.is_empty() {
            return Started::Done(true);
        }
        Started::Running(Task::Actions {
            job: Box::new(ActionsJob::chain(self.proof().clone(), actions)),
            kind: ActionsKind::Proof,
            redone,
        })
    }

    /// Commits what a [`Task`] came to, once it is done.
    ///
    /// The task worked on the proof as it was when it started, so this is only right if nothing
    /// has changed the history since: the app refuses actions while a task runs.
    pub fn finish(&mut self, output: TaskOutput) -> Result<Started, ModelError> {
        match output {
            TaskOutput::Actions {
                outcome,
                kind,
                redone,
            } => {
                let changed = redone || !outcome.applied.is_empty();
                for (action, proof) in outcome.applied {
                    if kind == ActionsKind::Proof {
                        // Only exfiltrate proof actions, otherwise
                        // we risk funny business with circular action imports.
                        crate::panic::push_action(&action);
                    }
                    self.history.add(action, proof);
                }
                if changed && kind == ActionsKind::Proof {
                    self.clear_selections();
                }
                if let Some(error) = outcome.error {
                    return Err(error.into());
                }
                Ok(Started::Done(changed || kind == ActionsKind::Import))
            }
            TaskOutput::AttachOptions(outcome) => {
                let matches = outcome.options;
                self.inference_report = Some(outcome.inference);
                match matches.len() {
                    0 => {
                        self.clear_selections();
                        return Err(ModelError::NoAttachment);
                    }
                    1 => {
                        self.clear_selections();
                        return self.start(Action::Proof(proof::Action::Attach(
                            matches.into_iter().next().unwrap(),
                        )));
                    }
                    _ => {
                        self.options = Some(Selectables::Attach(matches));
                        self.attachment_highlight = None;
                        self.slice_highlight = None;
                    }
                }
                Ok(Started::Done(true))
            }
            TaskOutput::Infer(schema, inference) => {
                self.drafts.finish(schema, inference);
                Ok(Started::Done(false))
            }
            TaskOutput::Export(ExportOutput {
                files,
                extension,
                multiple,
            }) => {
                if multiple {
                    let data = zip_files(files.into_iter());
                    generate_download("homotopy_io_export", "zip", &data)
                        .map_err(ModelError::Export)?;
                } else if let Some((_, data)) = files.first() {
                    generate_download("homotopy_io_export", extension, data.as_bytes())
                        .map_err(ModelError::Export)?;
                }
                Ok(Started::Done(true))
            }
        }
    }

    /// Handler for [Action::HighlightAttachment].
    fn highlight_attachment(&mut self, option: Option<AttachOption>) {
        self.attachment_highlight = option;
    }

    /// Handler for [Action::HighlightSlice].
    const fn highlight_slice(&mut self, option: Option<SliceIndex>) {
        self.slice_highlight = option;
    }

    /// Handler for [Action::ClearSelections].
    fn clear_selections(&mut self) {
        self.options = None;
        self.armed = None;
        self.attachment_highlight = None;
        self.slice_highlight = None;
    }

    /// Handler for [Action::MergeOptions].
    fn merge_options(&mut self, generator: Generator) {
        let result = self.proof().signature.globular_pairs(generator);
        if !result.is_empty() {
            self.options = Some(Selectables::Merge(generator, result));
        }
    }

    /// Handler for [Action::SetRemoteProjectId].
    fn set_remote_project_metadata(&mut self, metadata: Option<account::RemoteProjectMetadata>) {
        if let Some(md) = &metadata {
            let path = if md.visibility == account::ProjectVisibility::Published {
                format!("/p/{}", md.id)
            } else {
                format!("/u/{}/{}", md.uid, md.id)
            };
            update_window_url_path(&path);
        } else {
            update_window_url_path("/");
        }
        self.remote_project_metadata = metadata;
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("export failed")]
    Export(wasm_bindgen::JsValue),
    #[error(transparent)]
    Proof(#[from] proof::ProofError),
    #[error(transparent)]
    History(#[from] history::HistoryError),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error("internal error")]
    Internal,
    #[error("no attachment found")]
    NoAttachment,
    #[error("index out of bounds")]
    IndexOutOfBounds,
}

fn help() -> Result<(), ModelError> {
    let window = web_sys::window().ok_or(ModelError::Internal)?;
    let document = window.document().ok_or(ModelError::Internal)?;
    let location = document.location().ok_or(ModelError::Internal)?;
    location.set_href("#help").or(Err(ModelError::Internal))
}

pub fn zip_files(files: impl Iterator<Item = (String, impl AsRef<[u8]>)>) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let file_options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::STORE);
        for (name, data) in files {
            zip.start_file(name, file_options).unwrap();
            zip.write_all(data.as_ref()).unwrap();
        }
    }
    buf
}

pub fn generate_download(name: &str, ext: &str, data: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    let val: js_sys::Uint8Array = data.into();
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/msgpack");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(
        &js_sys::Array::of1(&val.into()).into(),
        &options,
    )?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;
    let e = document.create_element("a")?;
    let a = e
        .dyn_ref::<web_sys::HtmlElement>()
        .ok_or("failed to create anchor")?;
    a.set_attribute("href", &url)?;
    a.set_attribute("download", &format!("{name}.{ext}"))?;
    body.append_child(a)?;
    a.click();
    a.remove();
    web_sys::Url::revoke_object_url(&url)
}

pub fn update_window_url_path(new_path: &str) {
    let window = web_sys::window().unwrap();
    let origin = window.location().origin().unwrap();
    window
        .history()
        .unwrap()
        .replace_state_with_url(
            &None::<u8>.into(),
            "title",
            Some(&format!("{origin}{new_path}")),
        )
        .unwrap();
}

#[must_use]
pub fn subscript(n: usize) -> String {
    n.to_string()
        .chars()
        .map(|c| char::from_u32(0x2080 + c.to_digit(10).unwrap()).unwrap())
        .collect()
}
