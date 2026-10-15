//! The line at the top of the list of attachments while the search for them is still going on:
//! what it is doing and how far along it is, and a way to stop it (`Esc` does too).
//!
//! The list itself only changes when options arrive; how far the search is changes many times a
//! second, so this listens to the same reports as the busy indicator (see [`busy::watch`]) and
//! is the only thing that renders for them.

use yew::prelude::*;

use super::busy::{self, Event, Info};

#[derive(Properties, PartialEq)]
pub struct Props {
    pub on_stop: Callback<()>,
}

pub struct SearchStatus {
    listener: u64,
    info: Option<Info>,
}

impl Component for SearchStatus {
    type Message = Event;
    type Properties = Props;

    fn create(ctx: &Context<Self>) -> Self {
        let (listener, info) = busy::watch(ctx.link().callback(|event| event));
        Self { listener, info }
    }

    fn update(&mut self, _ctx: &Context<Self>, event: Event) -> bool {
        match event {
            Event::Show(info) => {
                let changed = self.info.as_ref() != Some(&info);
                self.info = Some(info);
                changed
            }
            Event::Hide => self.info.take().is_some(),
            Event::Refused => false,
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let doing = self.info.as_ref().map(|info| {
            let progress = info
                .progress
                .filter(|p| p.total > 1)
                .map_or_else(String::new, |p| {
                    format!(", {} of {}", p.done.min(p.total), p.total)
                });
            format!("{}{progress}", info.label)
        });
        let stop = ctx.props().on_stop.reform(|_: MouseEvent| ());
        html! {
            <li class="attach__searching" role="status" aria-live="off">
                <span class="busy__spinner" aria-hidden="true"></span>
                <span class="attach__searching-text">
                    <span>{"Still searching…"}</span>
                    if let Some(doing) = doing {
                        <span class="attach__searching-doing">{doing}</span>
                    }
                </span>
                <button type="button" class="attach__searching-stop" onclick={stop}>
                    {"Stop"}
                </button>
                <span class="attach__searching-hint">
                    {"Pick an option to attach it and stop, or Esc to stop and keep these."}
                </span>
            </li>
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        busy::unwatch(self.listener);
    }
}
