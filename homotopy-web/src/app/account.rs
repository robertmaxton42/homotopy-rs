use std::{
    fmt,
    ops::Deref,
    time::{Duration, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use futures::future::join_all;
use homotopy_model::proof::ProofState;
use js_sys::Uint8Array;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use yew::prelude::*;

use crate::{
    components::{
        icon::{Icon, IconSize},
        toast::{toast, Toast, ToastKind},
    },
    model,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStatus {
    Failed,
    Updating,
    Succeeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProjectMetadata {
    pub id: String,
    pub uid: String,
    pub title: String,
    pub author: String,
    pub abstr: String,
    pub visibility: ProjectVisibility,
    pub updated: u64,
    pub version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProjectVisibility {
    #[default]
    Private,
    Public,
    Published,
}

impl fmt::Display for ProjectVisibility {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::Private => "Private",
            Self::Public => "Public",
            Self::Published => "Published",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    LogOut,
    SetProjectsView(ProjectsView),
    FetchProjects,
    ProjectsFetched(ProjectCollection),
    SaveProject,
    SaveNewProject,
    ProjectSaved(Result<RemoteProjectMetadata, CloudError>),
    PublishProject(String),
    ProjectPublished(Result<PublishResult, CloudError>),
    PublishProjectVersion(String),
    SetPublic(String, bool),
    ProjectPublicChanged(String, bool),
    DeleteProject(String),
    ProjectDeleted(String),
    OpenPersonalProject(String, String),
    OpenPublishedProject(String, u64),
    ProjectDownloaded((RemoteProjectMetadata, Vec<u8>)),
    Resolved(Toast),
    /// Tries to load Firebase again (see `static/firebase.js`).
    Retry,
    /// What came of that: why Firebase is still not available, or `None` if it is now.
    Retried(Option<CloudError>),
    /// The sign-in buttons could not be started.
    SignInFailed(CloudError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectsView {
    Personal,
    Published,
}

#[derive(Debug, Properties, Clone, PartialEq)]
pub struct Props {
    pub proof: model::history::Proof,
    pub remote_project_metadata: Option<RemoteProjectMetadata>,
    pub dispatch: Callback<model::Action>,
}

pub struct AccountView {
    projects: ProjectCollection,
    projects_view: ProjectsView,
    status: OperationStatus,
    /// Why Firebase is not available, if it is not. It is not there for people who block
    /// gstatic.com or are offline, and then the drawer says so and nothing else is attempted.
    unavailable: Option<CloudError>,
    retrying: bool,
    sign_in_failed: Option<CloudError>,
}

/// An error from Firebase, or from there being none, as something to tell the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudError(String);

impl CloudError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<JsValue> for CloudError {
    /// Errors from Firebase are objects with a `message`; others are whatever was thrown.
    fn from(value: JsValue) -> Self {
        let message = js_sys::Reflect::get(&value, &JsValue::from_str("message"))
            .ok()
            .and_then(|message| message.as_string())
            .or_else(|| value.as_string())
            .unwrap_or_else(|| format!("{value:?}"));
        Self(message)
    }
}

impl From<serde_wasm_bindgen::Error> for CloudError {
    fn from(error: serde_wasm_bindgen::Error) -> Self {
        Self(error.to_string())
    }
}

/// Why Firebase is not available, or `None` if it is: whether its scripts could be loaded and it
/// started. Nothing here can throw.
pub fn cloud_unavailable() -> Option<CloudError> {
    cloud_error().map(CloudError::new)
}

type CloudResult<T> = Result<T, CloudError>;

impl Component for AccountView {
    type Message = Msg;
    type Properties = Props;

    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().callback(|()| Msg::FetchProjects).emit(());
        tracing::debug!("creating account view");

        Self {
            projects: ProjectCollection::default(),
            projects_view: ProjectsView::Personal,
            status: OperationStatus::Updating,
            unavailable: cloud_unavailable(),
            retrying: false,
            sign_in_failed: None,
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        if let Some(reason) = &self.unavailable {
            return self.unavailable_view(ctx, reason);
        }
        if let Some(userdata) = current_user().ok().flatten() {
            let user_icon = if let Some(url) = &userdata.photo_url() {
                html! {
                    <img
                        src={url.clone()}
                        referrerpolicy="no-referrer"
                        class="account__user-photo"
                        width="36px"
                        height="36px" />
                }
            } else {
                html! {
                    <Icon name="account_circle" size={IconSize::Icon36} />
                }
            };

            let status_indicator = match self.status {
                OperationStatus::Failed => html! {
                    <div class="account__save-status account__save-status-failed">
                        {"An error occurred"}
                    </div>
                },
                OperationStatus::Updating => html! {
                    <div class="account__save-status">
                        {"Updating..."}
                    </div>
                },
                OperationStatus::Succeeded => html! {
                    <div class="account__save-status">
                        {"Up to date"}
                    </div>
                },
            };

            let toggle_personal_cb = ctx
                .link()
                .callback(move |_| Msg::SetProjectsView(ProjectsView::Personal));
            let toggle_published_cb = ctx
                .link()
                .callback(move |_| Msg::SetProjectsView(ProjectsView::Published));
            let (toggle_personal_class, toggle_published_class) = {
                let (a, b) = match self.projects_view {
                    ProjectsView::Personal => ("active", "inactive"),
                    ProjectsView::Published => ("inactive", "active"),
                };
                (
                    format!("account__projects-view-toggle account__projects-view-toggle-{a}"),
                    format!("account__projects-view-toggle account__projects-view-toggle-{b}"),
                )
            };

            let maybe_save = if ctx.props().remote_project_metadata.is_some() {
                html! {
                    <button onclick={ctx.link().callback(|_| Msg::SaveProject)}>{"Save"}</button>
                }
            } else {
                html! {}
            };

            let maybe_publish = if let Some(metadata) = &ctx.props().remote_project_metadata {
                let id = metadata.id.clone();
                match metadata.visibility {
                    ProjectVisibility::Public => html! {
                        <button onclick={ctx.link().callback(move |_| Msg::PublishProject(id.clone()))}>{"Publish"}</button>
                    },
                    ProjectVisibility::Published
                        if ctx
                            .props()
                            .remote_project_metadata
                            .as_ref()
                            .map(|meta| meta.uid.clone())
                            == user_id().ok() =>
                    {
                        html! {
                            <button onclick={ctx.link().callback(move |_| Msg::PublishProjectVersion(id.clone()))}>{"Publish new version"}</button>
                        }
                    }
                    _ => html! {},
                }
            } else {
                html! {}
            };

            html! {
                <>
                    <div style="color:red">
                        <p>
                            {"Warning: This server functionality is a work in progress. Please do not use this for anything important, and export any workspaces which you wish to save."}
                        </p>
                        <p>
                            {"In this testing period, no guarantees will be made about data retention and the server may be wiped at any point."}
                        </p>
                    </div>
                    <div class="account__user-details">
                        { user_icon }
                        <div class="account__user-name">{userdata.display_name().clone()}</div>
                    </div>
                    <button onclick={ctx.link().callback(|_| Msg::LogOut)}>{"Log out"}</button>
                    <button onclick={ctx.link().callback(|_| Msg::FetchProjects)}>{"Refresh Projects"}</button>
                    {maybe_save}
                    <button onclick={ctx.link().callback(|_| Msg::SaveNewProject)}>{"Save new"}</button>
                    {maybe_publish}
                    <p>{status_indicator}</p>
                    <div class="account__project-list-wrapper">
                        <div class="account__projects-view-toggles-wrapper">
                            <div
                                class={toggle_personal_class}
                                onclick={toggle_personal_cb}>
                                    {"Personal"}
                            </div>
                            <div
                                class={toggle_published_class}
                                onclick={toggle_published_cb}>
                                    {"Published"}
                            </div>
                        </div>
                        { self.user_projects_list(ctx) }
                    </div>
                </>
            }
        } else {
            let sign_in_failed = self.sign_in_failed.as_ref().map(|error| {
                html! {
                    <p class="account__unavailable-detail">
                        {format!("The sign-in options could not be shown: {error}")}
                    </p>
                }
            });
            html! {
                <div>
                    <span>{"You are not logged in. Please try the following methods."}</span>
                    <div id="firebaseui-auth-container"></div>
                    {sign_in_failed}
                </div>
            }
        }
    }

    fn rendered(&mut self, ctx: &Context<Self>, _first_render: bool) {
        if self.unavailable.is_some() || self.sign_in_failed.is_some() {
            return;
        }
        if current_user().ok().flatten().is_none() {
            let logged_in_cb = ctx
                .link()
                .callback(|()| Msg::Resolved(Toast::success("Logged in")));
            let logged_in_cb_js = Closure::once_into_js(move || {
                logged_in_cb.emit(());
            });
            if let Err(error) = init_ui(logged_in_cb_js) {
                ctx.link().send_message(Msg::SignInFailed(error.into()));
            }
        }
    }

    #[allow(clippy::cognitive_complexity)]
    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        let set_remote_project_metadata = ctx
            .props()
            .dispatch
            .reform(model::Action::SetRemoteProjectMetadata);
        match msg {
            Msg::LogOut => {
                ctx.link().send_future(async {
                    match sign_out().await {
                        Ok(()) => Msg::Resolved(Toast::success("Logged out")),
                        Err(error) => {
                            Msg::Resolved(Toast::error(format!("Failed to log out: {error}")))
                        }
                    }
                });
                false
            }
            Msg::Retry => {
                self.retrying = true;
                ctx.link()
                    .send_future(async { Msg::Retried(try_again().await) });
                true
            }
            Msg::Retried(reason) => {
                self.retrying = false;
                self.sign_in_failed = None;
                self.unavailable = reason;
                if self.unavailable.is_none() {
                    toast(Toast::success("Cloud storage is available"));
                    ctx.link().send_message(Msg::FetchProjects);
                }
                true
            }
            Msg::SignInFailed(error) => {
                tracing::warn!("Failed to start the sign-in options: {error}");
                self.sign_in_failed = Some(error);
                true
            }
            Msg::SetProjectsView(view) => {
                self.projects_view = view;
                true
            }
            Msg::FetchProjects => {
                // Without Firebase, or logged out, there is nothing to fetch, and nothing to
                // complain about.
                if self.unavailable.is_some() || user_id().is_err() {
                    self.status = OperationStatus::Succeeded;
                    return false;
                }
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async {
                    match fetch_user_projects().await {
                        Ok(collection) => Msg::ProjectsFetched(collection),
                        Err(error) => Msg::Resolved(Toast::error(format!(
                            "Failed to fetch projects: {error}"
                        ))),
                    }
                });
                false
            }
            Msg::ProjectsFetched(projects) => {
                self.status = OperationStatus::Succeeded;
                self.projects = projects;
                true
            }
            Msg::SaveProject => {
                if let Some(id) = project_id(ctx) {
                    self.status = OperationStatus::Updating;
                    let proof = ctx.props().proof.inner().deref().clone();
                    ctx.link().send_future_batch(async move {
                        vec![
                            Msg::ProjectSaved(save_project(&id, proof).await),
                            Msg::FetchProjects, // TODO: fetch only the new project instead of everything
                        ]
                    });
                }
                false
            }
            Msg::SaveNewProject => {
                self.status = OperationStatus::Updating;
                let proof = ctx.props().proof.inner().deref().clone();
                ctx.link().send_future_batch(async {
                    let saved = match fresh_id().await {
                        Ok(id) => save_project(&id, proof).await,
                        Err(error) => Err(error),
                    };
                    vec![
                        Msg::ProjectSaved(saved),
                        Msg::FetchProjects, // TODO: fetch only the new project instead of everything
                    ]
                });
                false
            }
            Msg::ProjectSaved(new_remote_metadata) => {
                // Show successful save
                match new_remote_metadata {
                    Ok(metadata) => {
                        tracing::debug!("project saved successfully");
                        self.status = OperationStatus::Succeeded;
                        toast(Toast::success("Project saved successfully"));
                        set_remote_project_metadata.emit(Some(metadata));
                    }
                    Err(error) => {
                        self.status = OperationStatus::Failed;
                        toast(Toast::error(format!("Project save failed: {error}")));
                    }
                }
                true
            }
            Msg::PublishProject(id) => {
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async move {
                    let res = async {
                        let raw = publish_project(&id).await?;
                        tracing::debug!("publish result: {:?}", &raw);
                        let wrapped: Wrapped<PublishResult> = serde_wasm_bindgen::from_value(raw)?;
                        CloudResult::Ok(wrapped.data)
                    }
                    .await;
                    Msg::ProjectPublished(res)
                });
                false
            }
            Msg::ProjectPublished(res) => {
                if let Ok(res) = res {
                    self.status = OperationStatus::Succeeded;
                    toast(Toast::success(format!(
                        "Project published: {}v{}",
                        res.tag, res.version
                    )));
                    let meta = ctx.props().remote_project_metadata.clone().map(|mut m| {
                        m.id = res.tag;
                        m.visibility = ProjectVisibility::Published;
                        m.version = Some(1);
                        m
                    });
                    set_remote_project_metadata.emit(meta);
                } else if let Err(error) = res {
                    self.status = OperationStatus::Failed;
                    toast(Toast::error(format!("Project publish failed: {error}")));
                }
                true
            }
            Msg::PublishProjectVersion(tag) => {
                self.status = OperationStatus::Updating;
                let proof = ctx.props().proof.inner().deref().clone();
                ctx.link().send_future(async move {
                    match publish_project_version(&tag, proof).await {
                        Ok(()) => {
                            Msg::Resolved(Toast::success(format!("New version published: {tag}")))
                        }
                        Err(error) => Msg::Resolved(Toast::error(format!(
                            "Failed to publish a new version: {error}"
                        ))),
                    }
                });
                false
            }
            Msg::SetPublic(id, public) => {
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async move {
                    match set_public(&id, public).await {
                        Ok(()) => Msg::ProjectPublicChanged(id, public),
                        Err(error) => Msg::Resolved(Toast::error(format!(
                            "Failed to change whether {id} is public: {error}"
                        ))),
                    }
                });
                false
            }
            Msg::ProjectPublicChanged(id, public) => {
                self.status = OperationStatus::Succeeded;
                let projects = &mut self.projects.personal;
                if let Some(project) = projects.iter_mut().find(|p| p.id == id) {
                    project.visibility = if public {
                        ProjectVisibility::Public
                    } else {
                        ProjectVisibility::Private
                    };
                }
                if ctx.props().remote_project_metadata.as_ref().map(|m| &m.id) == Some(&id) {
                    let metadata = ctx.props().remote_project_metadata.clone().map(|mut m| {
                        m.visibility = if public {
                            ProjectVisibility::Public
                        } else {
                            ProjectVisibility::Private
                        };
                        m
                    });
                    set_remote_project_metadata.emit(metadata);
                }
                toast(Toast::success(format!(
                    "Made project {id} {}",
                    if public { "public" } else { "private" }
                )));
                true
            }
            Msg::DeleteProject(id) => {
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async move {
                    match delete_project(&id).await {
                        Ok(()) => Msg::ProjectDeleted(id.clone()),
                        Err(error) => {
                            Msg::Resolved(Toast::error(format!("Failed to delete {id}: {error}")))
                        }
                    }
                });
                false
            }
            Msg::ProjectDeleted(id) => {
                self.status = OperationStatus::Succeeded;
                self.projects.personal.retain(|p| p.id != id);
                if ctx.props().remote_project_metadata.as_ref().map(|m| &m.id) == Some(&id) {
                    set_remote_project_metadata.emit(None);
                }
                toast(Toast::success(format!("Project {id} deleted")));
                true
            }
            Msg::OpenPersonalProject(uid, id) => {
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async move {
                    match download_personal_project(&uid, &id).await {
                        Ok((project, blob)) => Msg::ProjectDownloaded((project, blob)),
                        Err(error) => Msg::Resolved(Toast::error(format!(
                            "Failed to download project: {error}"
                        ))),
                    }
                });
                false
            }
            Msg::OpenPublishedProject(id, version) => {
                self.status = OperationStatus::Updating;
                ctx.link().send_future(async move {
                    match download_published_project(&id, version).await {
                        Ok((project, blob)) => Msg::ProjectDownloaded((project, blob)),
                        Err(error) => Msg::Resolved(Toast::error(format!(
                            "Failed to download project: {error}"
                        ))),
                    }
                });
                false
            }
            Msg::ProjectDownloaded((remote_project_metadata, blob)) => {
                // TODO: handle failure
                self.status = OperationStatus::Succeeded;
                let import_proof = ctx
                    .props()
                    .dispatch
                    .reform(move |bytes| model::proof::Action::ImportProof(bytes).into());
                import_proof.emit(blob.into());
                let set_remote_project_metadata = ctx
                    .props()
                    .dispatch
                    .reform(model::Action::SetRemoteProjectMetadata);
                set_remote_project_metadata.emit(Some(remote_project_metadata));
                toast(Toast::success("Project downloaded"));
                true
            }
            Msg::Resolved(t) => {
                self.status = if t.kind == ToastKind::Success {
                    OperationStatus::Succeeded
                } else {
                    OperationStatus::Failed
                };
                toast(t);
                true
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PublishResult {
    tag: String,
    version: u64,
}

#[derive(Debug, Deserialize)]
struct Wrapped<T> {
    data: T,
}

impl AccountView {
    /// What the drawer says when Firebase could not be loaded or started: that it is not the
    /// person's work that is at fault, why, and how to try again without losing it.
    fn unavailable_view(&self, ctx: &Context<Self>, reason: &CloudError) -> Html {
        html! {
            <div class="account__unavailable">
                <p><strong>{"Cloud storage is unavailable"}</strong></p>
                <p>
                    {"Firebase couldn't be loaded (perhaps it is blocked by an extension, or you \
                      are offline). Your work here is unaffected."}
                </p>
                <p>
                    {"If you allow gstatic.com for this page, press Retry: the page does not have \
                      to be reloaded. (Reloading would lose what has not been exported.)"}
                </p>
                <button
                    onclick={ctx.link().callback(|_| Msg::Retry)}
                    disabled={self.retrying}>
                    { if self.retrying { "Retrying..." } else { "Retry" } }
                </button>
                <p class="account__unavailable-detail">{reason.to_string()}</p>
            </div>
        }
    }

    fn user_projects_list(&self, ctx: &Context<Self>) -> Html {
        let current_project_id = ctx
            .props()
            .remote_project_metadata
            .as_ref()
            .map(|metadata| metadata.id.clone());

        let projects_list = if self.projects_view == ProjectsView::Personal {
            &self.projects.personal
        } else {
            &self.projects.published
        };

        let list_items = projects_list
            .iter()
            .map(|project| {
                let project_id = project.id.clone();
                let title = if project.title.is_empty() {
                    "(Untitled)".to_owned()
                } else {
                    project.title.clone()
                };
                let author = if project.author.is_empty() {
                    "(No Author)".to_owned()
                } else {
                    project.author.clone()
                };
                let lm = format!("Updated: {}", format_timestamp(project.updated));
                let update_visibility_cb = {
                    let id = project_id.clone();
                    ctx.link().callback(move |e: MouseEvent| {
                        e.stop_propagation();
                        let input: web_sys::HtmlInputElement = e.target_unchecked_into();
                        Msg::SetPublic(id.clone(), input.checked())
                    })
                };
                let delete = {
                    let delete_project_id = project_id.clone();
                    let delete_cb = ctx.link().callback(move |e: MouseEvent| {
                        e.stop_propagation();
                        Msg::DeleteProject(delete_project_id.clone())
                    });
                    html! {
                        <button onclick={delete_cb}>{"Delete"}</button>
                    }
                };
                let open_cb = {
                    let project = project.clone();
                    if project.visibility == ProjectVisibility::Published {
                        ctx.link()
                            .callback(move |_| Msg::OpenPublishedProject(project.id.clone(), project.version.unwrap_or(1)))
                    } else {
                        ctx.link()
                            .callback(move |_| Msg::OpenPersonalProject(project.uid.clone(), project.id.clone()))
                    }
                };
                let li_class = if matches!(current_project_id, Some(ref id) if &project_id == id) {
                    "account__project-list-item account__project-list-item-current"
                } else {
                    "account__project-list-item"
                };
                let delete_or_versions = if project.visibility == ProjectVisibility::Published {
                    // TODO: handle other versions
                    html! {
                        <div class="account__project-list-item-versions">
                            {
                                #[allow(clippy::iter_on_single_items)]
                                [project.version.unwrap_or_default()].iter().map(|v| html! { <span>{format!("v{v}")}</span> }).collect::<Html>()
                            }
                        </div>
                    }
                } else {
                    html! {
                        <div class="account__project-list-item-delete">{delete}</div>
                    }
                };
                let visibility_checkbox = if project.visibility == ProjectVisibility::Published {
                    html! {}
                } else {
                    html! {
                        <div class="account__project-list-item-visibility">
                            <div>{"Public"}</div>
                            <input
                                type="checkbox"
                                checked={project.visibility == ProjectVisibility::Public}
                                onclick={update_visibility_cb}
                            />
                        </div>
                    }
                };
                html! {
                    <li class={li_class} onclick={open_cb}>
                        <div class="account__project-list-item-title">{title}</div>
                        <div class="account__project-list-item-author">{author}</div>
                        {visibility_checkbox}
                        <div class="account__project-list-item-id">{project.id.clone()}</div>
                        <div class="account__project-list-item-lm">{lm}</div>
                        {delete_or_versions}
                    </li>
                }
            })
            .collect::<Vec<Html>>();

        if list_items.is_empty() && self.status != OperationStatus::Updating {
            html! {
                <div class="account__no-projects-msg">{"No projects yet"}</div>
            }
        } else {
            html! {
                <ul class="account__project-list">
                    {list_items}
                </ul>
            }
        }
    }
}

fn format_timestamp(seconds: u64) -> String {
    let d = UNIX_EPOCH + Duration::from_secs(seconds);
    let dt = DateTime::<Utc>::from(d);
    dt.format("%F %H:%M").to_string()
}

fn firestore() -> CloudResult<Firestore> {
    Ok(get_firebase()?.firestore()?)
}

fn storage() -> CloudResult<Storage> {
    Ok(get_firebase()?.storage()?)
}

fn current_user() -> CloudResult<Option<User>> {
    Ok(get_firebase()?.auth()?.current_user()?)
}

fn user_id() -> CloudResult<String> {
    current_user()?
        .map(|user| user.uid())
        .ok_or_else(|| CloudError::new("you are not logged in"))
}

async fn sign_out() -> CloudResult<()> {
    get_firebase()?.auth()?.sign_out().await?;
    Ok(())
}

/// Loads Firebase again, if it could not be loaded: `None` if it is now available, and why not
/// if it is not.
async fn try_again() -> Option<CloudError> {
    match retry_cloud().await {
        Ok(reason) => reason.as_string().map(CloudError::new),
        Err(error) => Some(error.into()),
    }
}

fn project_id(ctx: &Context<AccountView>) -> Option<String> {
    ctx.props()
        .remote_project_metadata
        .as_ref()
        .map(|metadata| metadata.id.clone())
}

async fn fresh_id() -> CloudResult<String> {
    // get an id from the server
    let uid = user_id()?;
    tracing::debug!("getting fresh project id");
    let docref = firestore()?
        .collection(format!("personal-rs/{uid}/projects").as_str())?
        .add(js_sys::Object::new().into())
        .await?;
    let x: &DocumentReference = docref.as_ref().unchecked_ref();
    let id = x.id();
    tracing::debug!("got fresh id: {}", id);
    Ok(id)
}

async fn fetch_personal_project(uid: &str, id: &str) -> CloudResult<RemoteProjectMetadata> {
    let ds = firestore()?
        .doc(&format!("personal-rs/{uid}/projects/{id}"))?
        .get()
        .await?
        .unchecked_into::<DocumentSnapshot>();
    if !ds.exists() {
        return Err(CloudError::new(format!("there is no project {id}")));
    }
    let decoded = serde_wasm_bindgen::from_value::<PersonalRecord>(ds.data())?;
    Ok(RemoteProjectMetadata {
        id: id.to_owned(),
        uid: uid.to_owned(),
        title: decoded.title,
        author: decoded.author,
        abstr: decoded.r#abstract,
        visibility: if decoded.public {
            ProjectVisibility::Public
        } else {
            ProjectVisibility::Private
        },
        updated: decoded.updated.seconds,
        version: None,
    })
}

async fn fetch_published_project(id: &str, version: u64) -> CloudResult<RemoteProjectMetadata> {
    let uid = firestore()?
        .doc(&format!("published-rs/{id}"))?
        .get()
        .await?
        .unchecked_into::<DocumentSnapshot>()
        .get("uid")
        .as_string()
        .ok_or_else(|| CloudError::new(format!("there is no published project {id}")))?;
    let ds = firestore()?
        .doc(&format!("published-rs/{id}/versions/v{version}"))?
        .get()
        .await?
        .unchecked_into::<DocumentSnapshot>();
    if !ds.exists() {
        return Err(CloudError::new(format!(
            "there is no version {version} of {id}"
        )));
    }
    let decoded = serde_wasm_bindgen::from_value::<PublishedRecord>(ds.data())?;
    Ok(RemoteProjectMetadata {
        id: id.to_owned(),
        uid,
        title: decoded.title,
        author: decoded.author,
        abstr: decoded.r#abstract,
        visibility: ProjectVisibility::Published,
        updated: decoded.created.seconds,
        version: Some(version),
    })
}

#[allow(clippy::cognitive_complexity)]
async fn fetch_user_projects() -> CloudResult<ProjectCollection> {
    let uid = user_id()?;
    let personal = firestore()?
        .collection(&format!("personal-rs/{uid}/projects"))?
        .order_by("updated", "desc")?
        .get()
        .await?
        .unchecked_into::<QuerySnapshot>()
        .docs()
        .into_iter()
        .filter_map(|qds| {
            let data = qds.data();
            let decoded = serde_wasm_bindgen::from_value::<PersonalRecord>(data).ok()?;
            Some(RemoteProjectMetadata {
                id: qds.id(),
                uid: uid.clone(),
                title: decoded.title,
                author: decoded.author,
                abstr: decoded.r#abstract,
                visibility: if decoded.public {
                    ProjectVisibility::Public
                } else {
                    ProjectVisibility::Private
                },
                updated: decoded.updated.seconds,
                version: None,
            })
        })
        .collect();
    tracing::debug!("personal collection: {:?}", &personal);

    let published_ids: Vec<String> = serde_wasm_bindgen::from_value(
        firestore()?
            .doc(&format!("personal-rs/{uid}"))?
            .get()
            .await?
            .unchecked_into::<DocumentSnapshot>()
            .get("published"),
    )
    .unwrap_or_default();
    tracing::debug!("published ids: {:?}", &published_ids);
    let published = join_all(published_ids.into_iter().map(|id| {
        let uid = uid.clone();
        async move {
            let versions = firestore()?
                .collection(&format!("published-rs/{id}/versions"))?
                .order_by("created", "desc")?
                .get()
                .await?
                .unchecked_into::<QuerySnapshot>()
                .docs();
            CloudResult::Ok(
                versions
                    .iter()
                    .map(|v| {
                        let version = Some(v.id()[1..].parse().ok()?);
                        let data = v.data();
                        tracing::debug!("data: {:?}", &data);
                        let decoded =
                            serde_wasm_bindgen::from_value::<PublishedRecord>(data).ok()?;
                        tracing::debug!("decoded: {:?}", &decoded);
                        Some(RemoteProjectMetadata {
                            id: id.clone(),
                            uid: uid.clone(),
                            title: decoded.title,
                            author: decoded.author,
                            abstr: decoded.r#abstract,
                            visibility: ProjectVisibility::Published,
                            updated: decoded.created.seconds,
                            version,
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        }
    }))
    .await
    .into_iter()
    .collect::<CloudResult<Vec<_>>>()?
    .into_iter()
    .flatten()
    .flatten()
    .collect();
    tracing::debug!("published collection: {:?}", &published);

    Ok(ProjectCollection {
        personal,
        published,
    })
}

#[derive(Debug, Serialize)]
struct UploadMetadata {
    #[serde(rename = "contentType")]
    content_type: &'static str,
    #[serde(rename = "customMetadata")]
    custom_metadata: CustomMetadata,
}
#[derive(Debug, Serialize)]
struct CustomMetadata {
    title: String,
    author: String,
    r#abstract: String,
}

/// Uploads a project to `path` in storage, with its metadata. Returns the metadata, with the
/// author filled in with the user's name if there was none.
async fn upload_project(path: &str, proof: ProofState) -> CloudResult<model::proof::Metadata> {
    let storageref = storage()?.storage_ref(path)?;
    tracing::debug!("got path: {:?}", storageref);

    let mut metadata = proof.metadata;

    // Author is filled in with user's name if applicable
    metadata.author = metadata.author.or_else(|| {
        current_user()
            .ok()
            .flatten()
            .as_ref()
            .and_then(User::display_name)
    });

    let blob = model::serialize::serialize(proof.signature, proof.workspace, metadata.clone());

    let upload_metadata = UploadMetadata {
        content_type: "application/msgpack",
        custom_metadata: CustomMetadata {
            title: metadata.title.clone().unwrap_or_default(),
            author: metadata.author.clone().unwrap_or_default(),
            r#abstract: metadata.abstr.clone().unwrap_or_default(),
        },
    };

    tracing::debug!("upload metadata: {:?}", upload_metadata);
    let task = storageref.put_with_metadata(
        Uint8Array::from(blob.as_slice()).into(),
        serde_wasm_bindgen::to_value(&upload_metadata)?,
    )?;
    task.then(JsValue::null(), JsValue::null()).await?;
    Ok(metadata)
}

async fn save_project(id: &str, proof: ProofState) -> CloudResult<RemoteProjectMetadata> {
    // get destination path
    tracing::debug!("Saving project {id}");
    let uid = user_id()?;
    let path = format!("personal-rs/{uid}/projects/{id}.hom");
    let metadata = upload_project(&path, proof).await?;

    Ok(RemoteProjectMetadata {
        id: id.to_owned(),
        uid: uid.clone(),
        title: metadata.title.unwrap_or_default(),
        author: metadata.author.unwrap_or_default(),
        abstr: metadata.abstr.unwrap_or_default(),
        visibility: ProjectVisibility::Private,
        updated: 0,
        version: None,
    })
}

async fn publish_project_version(tag: &str, proof: ProofState) -> CloudResult<()> {
    // get destination path
    let path = format!("published-rs/{tag}/versions/new.hom");
    upload_project(&path, proof).await?;
    Ok(())
}

async fn delete_project(id: &str) -> CloudResult<()> {
    tracing::debug!("Deleting {id}");
    let uid = user_id()?;
    let path = format!("personal-rs/{uid}/projects/{id}.hom");
    let storageref = storage()?.storage_ref(&path)?;
    tracing::debug!("got path: {:?}", storageref);

    storageref.delete().await?;
    Ok(())
}

/// The metadata and file of a personal project. Also used to open one from its URL when the page
/// loads, so this is where Firebase being unavailable is first noticed.
pub(super) async fn download_personal_project(
    uid: &str,
    id: &str,
) -> CloudResult<(RemoteProjectMetadata, Vec<u8>)> {
    tracing::debug!("Downloading {id}");
    let path = format!("personal-rs/{uid}/projects/{id}.hom");
    let url = download_url(&path).await?;
    let blob = download(&url).await?;
    let project = fetch_personal_project(uid, id).await?;
    Ok((project, blob))
}

pub(super) async fn download_published_project(
    tag: &str,
    version: u64,
) -> CloudResult<(RemoteProjectMetadata, Vec<u8>)> {
    tracing::debug!("Downloading {tag}v{version}");
    let path = format!("published-rs/{tag}/versions/v{version}.hom");
    let url = download_url(&path).await?;
    let blob = download(&url).await?;
    let project = fetch_published_project(tag, version).await?;
    Ok((project, blob))
}

async fn download_url(path: &str) -> CloudResult<String> {
    let url = storage()?
        .storage_ref(path)?
        .get_download_url()
        .await?
        .as_string()
        .ok_or_else(|| CloudError::new("storage gave no download address"))?;
    tracing::debug!("got download URL: {url}");
    Ok(url)
}

async fn download(url: &str) -> CloudResult<Vec<u8>> {
    let opts = web_sys::RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(web_sys::RequestMode::Cors);
    let req = web_sys::Request::new_with_str_and_init(url, &opts)?;
    req.headers().set("Accept", "application/msgpack")?;
    let window = web_sys::window().ok_or_else(|| CloudError::new("there is no window"))?;
    let res: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
        .await?
        .unchecked_into();
    if !res.ok() {
        return Err(CloudError::new(format!(
            "the download failed ({} {})",
            res.status(),
            res.status_text()
        )));
    }

    let array_buffer = JsFuture::from(res.array_buffer()?).await?;
    Ok(Uint8Array::new(&array_buffer).to_vec())
}

async fn set_public(id: &str, public: bool) -> CloudResult<()> {
    tracing::debug!("Making {id} {}", if public { "public" } else { "private" });
    let uid = user_id()?;
    let path = format!("personal-rs/{uid}/projects/{id}");
    let docref = firestore()?.doc(&path)?;
    docref.update("public", public.into()).await?;
    tracing::debug!("done setting public");
    Ok(())
}

#[wasm_bindgen]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProjectCollection {
    personal: Vec<RemoteProjectMetadata>,
    published: Vec<RemoteProjectMetadata>,
}

#[derive(Debug, Deserialize)]
struct Timestamp {
    seconds: u64,
    #[allow(dead_code)] // needs to be present for JSON deserialization
    nanoseconds: u64,
}

// metadata that exists in firestore
#[derive(Debug, Deserialize)]
struct PersonalRecord {
    title: String,
    author: String,
    r#abstract: String,
    public: bool,
    #[allow(dead_code)] // needs to be present for JSON deserialization
    created: Timestamp,
    updated: Timestamp,
}

#[derive(Debug, Deserialize)]
struct PublishedRecord {
    title: String,
    author: String,
    r#abstract: String,
    created: Timestamp,
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.uid() == other.uid()
    }
}

impl Eq for User {}

#[wasm_bindgen(module = "/src/app/account/account_script.js")]
extern "C" {
    /// Why Firebase is not available (its scripts were blocked or not loaded, or it could not
    /// start), or `null` if it is. Everything below throws when it is not, so all of it is
    /// imported with `catch`: a failure here is a message and not a crash.
    #[wasm_bindgen(js_name = "cloudError")]
    pub fn cloud_error() -> Option<String>;

    /// Loads the Firebase scripts again, and resolves to what `cloud_error` is after that.
    #[wasm_bindgen(catch, js_name = "retryCloud")]
    pub async fn retry_cloud() -> Result<JsValue, JsValue>;

    #[derive(Debug, Clone)]
    pub type Firebase;

    #[wasm_bindgen(catch, js_name = "getFirebase")]
    pub fn get_firebase() -> Result<Firebase, JsValue>;

    #[wasm_bindgen(method, catch)]
    pub fn auth(this: &Firebase) -> Result<Auth, JsValue>;

    #[wasm_bindgen(method, catch)]
    pub fn firestore(this: &Firebase) -> Result<Firestore, JsValue>;

    #[wasm_bindgen(method, catch)]
    pub fn storage(this: &Firebase) -> Result<Storage, JsValue>;

    // firebase.auth
    #[derive(Debug, Clone)]
    pub type Auth;

    #[wasm_bindgen(method, getter, catch, js_name = "currentUser")]
    pub fn current_user(this: &Auth) -> Result<Option<User>, JsValue>;

    #[wasm_bindgen(method, catch, js_name = "signOut")]
    pub async fn sign_out(this: &Auth) -> Result<JsValue, JsValue>;

    #[derive(Debug, Clone)]
    pub type User;

    #[wasm_bindgen(method, getter, js_name = "displayName")]
    pub fn display_name(this: &User) -> Option<String>;

    #[wasm_bindgen(method, getter)]
    pub fn email(this: &User) -> Option<String>;

    #[wasm_bindgen(method, getter, js_name = "photoURL")]
    pub fn photo_url(this: &User) -> Option<String>;

    #[wasm_bindgen(method, getter)]
    pub fn uid(this: &User) -> String;

    // firebase.firestore
    #[derive(Debug, Clone)]
    pub type Firestore;

    #[wasm_bindgen(method, catch)]
    pub fn collection(
        this: &Firestore,
        collectionPath: &str,
    ) -> Result<CollectionReference, JsValue>;

    #[wasm_bindgen(method, catch)]
    pub fn doc(this: &Firestore, documentPath: &str) -> Result<DocumentReference, JsValue>;

    #[derive(Debug, Clone)]
    pub type CollectionReference;

    #[wasm_bindgen(method, catch)]
    pub async fn add(this: &CollectionReference, data: JsValue) -> Result<JsValue, JsValue> /* DocumentReference */;

    #[wasm_bindgen(method, catch)]
    pub async fn get(this: &CollectionReference) -> Result<JsValue, JsValue> /* QuerySnapshot */;

    #[wasm_bindgen(method, catch, js_name = "orderBy")]
    pub fn order_by(
        this: &CollectionReference,
        fieldPath: &str,
        directionStr: &str,
    ) -> Result<Query, JsValue>;

    #[derive(Debug, Clone)]
    pub type Query;

    #[wasm_bindgen(method, catch)]
    pub async fn get(this: &Query) -> Result<JsValue, JsValue> /* QuerySnapshot */;

    #[wasm_bindgen(method, catch, js_name = "orderBy")]
    pub fn order_by(this: &Query, fieldPath: &str, directionStr: &str) -> Result<Query, JsValue>;

    #[derive(Debug, Clone)]
    pub type DocumentReference;

    #[wasm_bindgen(method, getter)]
    pub fn id(this: &DocumentReference) -> String;

    #[wasm_bindgen(method, catch)]
    pub async fn get(this: &DocumentReference) -> Result<JsValue, JsValue> /* DocumentSnapshot */;

    #[wasm_bindgen(method, catch)]
    pub async fn update(
        this: &DocumentReference,
        field: &str,
        value: JsValue,
    ) -> Result<JsValue, JsValue>;

    #[derive(Debug, Clone)]
    pub type DocumentSnapshot;

    #[wasm_bindgen(method, getter)]
    pub fn exists(this: &DocumentSnapshot) -> bool;

    #[wasm_bindgen(method, getter)]
    pub fn id(this: &DocumentSnapshot) -> String;

    #[wasm_bindgen(method)]
    pub fn data(this: &DocumentSnapshot) -> JsValue;

    #[wasm_bindgen(method)]
    pub fn get(this: &DocumentSnapshot, fieldPath: &str) -> JsValue;

    #[derive(Debug, Clone)]
    pub type QuerySnapshot;

    #[wasm_bindgen(method, getter)]
    pub fn docs(this: &QuerySnapshot) -> Vec<QueryDocumentSnapshot>;

    #[derive(Debug, Clone)]
    pub type QueryDocumentSnapshot;

    #[wasm_bindgen(method, getter)]
    pub fn id(this: &QueryDocumentSnapshot) -> String;

    #[wasm_bindgen(method)]
    pub fn data(this: &QueryDocumentSnapshot) -> JsValue;

    // firebase.storage
    #[derive(Debug, Clone)]
    pub type Storage;

    #[wasm_bindgen(method, catch, js_name = "ref")]
    pub fn storage_ref(this: &Storage, path: &str) -> Result<Reference, JsValue>;

    #[derive(Debug, Clone)]
    pub type Reference;

    #[wasm_bindgen(method, catch)]
    pub async fn delete(this: &Reference) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method, catch, js_name = "getDownloadURL")]
    pub async fn get_download_url(this: &Reference) -> Result<JsValue, JsValue> /* String */;

    #[wasm_bindgen(method, catch, js_name = "put")]
    pub fn put_with_metadata(
        this: &Reference,
        data: JsValue,
        metadata: JsValue,
    ) -> Result<UploadTask, JsValue>;

    #[derive(Debug, Clone)]
    pub type UploadTask;

    #[wasm_bindgen(method, catch)]
    pub async fn then(
        this: &UploadTask,
        onFulfilled: JsValue,
        onRejected: JsValue,
    ) -> Result<JsValue, JsValue>;

    // firebaseui
    #[wasm_bindgen(catch, js_name = "initUI")]
    pub fn init_ui(onSignInSuccessWithAuthResult: JsValue) -> Result<(), JsValue>;

    #[wasm_bindgen(catch, js_name = "publishPersonal")]
    pub async fn publish_project(id: &str) -> Result<JsValue, JsValue>;
}
