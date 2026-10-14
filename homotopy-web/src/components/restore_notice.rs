//! The notice that offers the work of an earlier visit that was never exported (see
//! [`crate::autosave`]). It is not a dialog: nothing waits for it, and nothing is restored until
//! *Restore* is pressed.

use yew::prelude::*;

use crate::autosave::Snapshot;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub backup: Snapshot,
    pub on_restore: Callback<()>,
    pub on_discard: Callback<()>,
}

/// The title cut to a length that fits the notice.
fn short(title: &str) -> String {
    const LONGEST: usize = 40;
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > LONGEST {
        let mut cut: String = title.chars().take(LONGEST - 1).collect();
        cut.push('…');
        cut
    } else {
        title
    }
}

#[function_component]
pub fn RestoreNotice(props: &Props) -> Html {
    let backup = &props.backup;
    let title = backup
        .title
        .as_deref()
        .map(short)
        .filter(|title| !title.is_empty());
    let restore = props.on_restore.reform(|_| ());
    let discard = props.on_discard.reform(|_| ());
    html! {
        <div class="restore-notice" role="status">
            <span class="restore-notice__text">
                {format!("Restore unsaved work from {}", backup.when())}
                if let Some(title) = title {
                    {" ("}<span class="restore-notice__title">{title}</span>{")"}
                }
                {"?"}
            </span>
            <button class="restore-notice__button restore-notice__button--primary" onclick={restore}>
                {"Restore"}
            </button>
            <button class="restore-notice__button" onclick={discard}>{"Discard"}</button>
        </div>
    }
}
