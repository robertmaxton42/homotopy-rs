//! Laying diagrams out without making the page wait for it.
//!
//! The layout of a diagram (`homotopy_core::layout::Layout`, a linear program solved by HiGHS) is
//! what takes longest when a diagram is drawn: from a few milliseconds on the examples to
//! hundreds for a large project. So for diagrams of any size it is done by a Web Worker of its
//! own, the same module as the page's with HiGHS loaded (see [`crate::worker`] and
//! `static/worker.js?layout`), reached by bytes ([`homotopy_model::remote::layout`]).
//!
//! It is a different worker from the one that does jobs because cancelling a job ends its worker
//! (which is what makes cancelling immediate), and a layout, which is not something that anybody
//! asked for and would not know to ask for again, must not be lost with it; nor should a layout
//! of a diagram for the page's views wait behind a job that takes seconds.
//!
//! The views [`ask`] for the layout of the diagram they are to show. They get it at once if it
//! is cheap (a diagram of no more than [`IN_PAGE_WEIGHT`] parts, for which sending it back and
//! forth would cost more than laying it out; or one that was laid out recently), or if there is
//! no worker (it is started after the app, and is not there while it restarts, or at all if
//! workers do not work). Otherwise they get a [`Ticket`], and the layout through the callback
//! they gave when it is done, and are to go on showing what they showed meanwhile. They can
//! [`cancel`] a ticket, as a newer diagram replaces the one it was for: a layout that is not
//! wanted any more is dropped before it is sent, and its answer ignored if it is already on its
//! way. Only one is with the worker at a time, so that nothing is queued up in it that nobody
//! wants.
//!
//! Layout is not a job in the sense of the busy indicator: nobody asked for it, there is nothing
//! to cancel (the project is not being changed), and nothing that is refused (the project is not
//! working on anything): showing the diagram late is the worst that can happen, and a newer one
//! replaces it. The views say they are waiting by dimming what they show and a small "Laying
//! out…" note, which appear only if the wait is longer than the busy indicator's.

use std::{any::Any, cell::RefCell, collections::VecDeque, rc::Rc};

use gloo_timers::future::TimeoutFuture;
use homotopy_core::{
    layout::{self, Layout},
    Diagram,
};
use homotopy_graphics::geom::CubicalGeometry;
use homotopy_model::remote::layout as wire;
use instant::Instant;
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::{
    components::{
        layout_note,
        toast::{toast, Toast},
    },
    worker::{self, Client, Event},
};

/// How many parts a diagram has (see [`layout::weight`]) to be laid out where it is asked for,
/// without the worker. Sending a diagram to the worker and taking the layout back costs about as
/// much as laying out a diagram of this weight.
///
/// In the debug build (about ten times slower than the optimised one) a layout of a diagram up to
/// this weight takes 2ms in the middle and 10ms at most, on the examples, and the page's share of
/// the worker's (making the request, and taking in the answer) is about the same from there on:
/// 8ms for a weight just above it.
pub const IN_PAGE_WEIGHT: usize = 300;

/// The same for geometry (see [`ask_geometry`]), which takes much longer to make, whatever the
/// weight: 10 to 50ms in the debug build for diagrams that weigh up to 300.
pub const IN_PAGE_GEOMETRY_WEIGHT: usize = 100;

/// How many layouts of large diagrams are remembered, so that going back to a diagram does not
/// take as long again.
const CACHE_SIZE: usize = 8;

/// Identifies a layout that was asked for and is to come later.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

/// What asking for a layout (or geometry) gets.
pub enum Asked<T> {
    /// The layout, there and then.
    Ready(Rc<T>),
    /// The layout will be given to the callback, with this ticket.
    Pending(Ticket),
}

/// What there is to make a layout of when it is delivered.
enum Outcome<'a> {
    /// The answer of the worker.
    Answer(&'a [u8]),
    /// There is none (the worker failed or is gone): lay it out here.
    Here,
}

/// Hands over the layout, making it here if there is no answer to make sense of. Returns whether
/// the answer was good.
type Delivery = Box<dyn FnOnce(Outcome<'_>) -> bool>;

/// A layout asked for but not yet sent.
struct Waiting {
    id: u64,
    weight: usize,
    what: &'static str,
    dimension: usize,
    /// Makes the request, given the number of milliseconds to delay by.
    request: Box<dyn FnOnce(u32) -> Vec<u8>>,
    deliver: Delivery,
}

/// A layout that the worker is making.
struct Flying {
    id: u64,
    weight: usize,
    what: &'static str,
    dimension: usize,
    /// Not there if it is not wanted any more.
    deliver: Option<Delivery>,
    sent: Instant,
    encode_ms: f64,
    bytes_out: usize,
}

/// How many layouts were made, and where, for the log and for tests.
#[derive(Default, Clone, Copy)]
struct Stats {
    by_worker: usize,
    small: usize,
    fallbacks: usize,
    cached: usize,
    cancelled: usize,
}

#[derive(Default)]
struct Service {
    client: Option<Client>,
    next_id: u64,
    queue: VecDeque<Waiting>,
    flying: Option<Flying>,
    /// Recent layouts of large diagrams, the most recent last.
    cache: VecDeque<(Diagram, usize, Rc<dyn Any>)>,
    stats: Stats,
    /// Whether the person was told that the worker is not there.
    told: bool,
}

thread_local! {
    static SERVICE: RefCell<Service> = RefCell::default();
}

// For the views.

/// Starts the worker, if it is not running. Until it is ready, layouts are done on the page.
pub fn start() {
    SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let service = &mut *service;
        let client = service.client.get_or_insert_with(new_client);
        client.ensure();
    });
}

fn new_client() -> Client {
    #[allow(unused_mut)] // It is changed by the debug hooks.
    let mut client = Client::new(
        Callback::from(|(generation, event)| heard(generation, event)),
        worker::LAYOUT_SCRIPT,
    );
    #[cfg(debug_assertions)]
    match crate::app::debug_hooks::layout_worker_mode().as_str() {
        "off" => client.disable(),
        "broken" => client.use_script("/no-such-worker.js"),
        _ => {}
    }
    client
}

/// Asks for the layout of the diagram. If it is not there at once, `on_layout` is told when it
/// is, with the ticket that this returns.
pub fn ask<const N: usize>(
    diagram: &Diagram,
    on_layout: Callback<(Ticket, Rc<Layout<N>>)>,
) -> Asked<Layout<N>> {
    ask_with(
        diagram,
        &Making {
            what: "layout",
            key: N,
            dimension: N,
            in_page_weight: IN_PAGE_WEIGHT,
        },
        here_layout::<N>,
        wire::request_layout::<N>,
        |bytes| wire::response_layout::<N>(bytes).map(|(layout, _)| layout),
        on_layout,
    )
}

/// Asks for the geometry that the view of the first `N` dimensions of the diagram is drawn from
/// (see [`CubicalGeometry::new`]), which includes its layout and takes longer than it.
pub fn ask_geometry<const N: usize>(
    diagram: &Diagram,
    animated: bool,
    on_geometry: Callback<(Ticket, Rc<CubicalGeometry>)>,
) -> Asked<CubicalGeometry> {
    ask_with(
        diagram,
        &Making {
            what: "geometry",
            key: 5 + N + 5 * usize::from(animated),
            dimension: N,
            in_page_weight: IN_PAGE_GEOMETRY_WEIGHT,
        },
        move |diagram| {
            let started = Instant::now();
            let geometry = CubicalGeometry::new::<N>(diagram, animated)
                .expect("the diagram has the dimensions it is viewed in");
            tracing::debug!(
                "Made geometry on the page in {:.1}ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
            geometry
        },
        move |diagram, delay| wire::request_geometry::<N>(diagram, animated, delay),
        |bytes| wire::response_geometry(bytes).map(|(geometry, _)| geometry),
        on_geometry,
    )
}

/// Like [`ask_geometry`], for a number of dimensions that is not known until the program runs.
///
/// # Panics
///
/// If `dimension` is more than 4.
pub fn ask_geometry_in(
    dimension: usize,
    diagram: &Diagram,
    animated: bool,
    on_geometry: Callback<(Ticket, Rc<CubicalGeometry>)>,
) -> Asked<CubicalGeometry> {
    match dimension {
        0 => ask_geometry::<0>(diagram, animated, on_geometry),
        1 => ask_geometry::<1>(diagram, animated, on_geometry),
        2 => ask_geometry::<2>(diagram, animated, on_geometry),
        3 => ask_geometry::<3>(diagram, animated, on_geometry),
        4 => ask_geometry::<4>(diagram, animated, on_geometry),
        _ => panic!("a view has no more than four dimensions"),
    }
}

/// What is being made, for the log and the cache.
struct Making {
    what: &'static str,
    /// Tells what is made of the same diagram apart (the dimensions, and whether geometry).
    key: usize,
    dimension: usize,
    in_page_weight: usize,
}

fn ask_with<T: 'static>(
    diagram: &Diagram,
    making: &Making,
    here: impl Fn(&Diagram) -> T + 'static,
    request: impl FnOnce(&Diagram, u32) -> Vec<u8> + 'static,
    decode: impl FnOnce(&[u8]) -> Result<T, String> + 'static,
    on_ready: Callback<(Ticket, Rc<T>)>,
) -> Asked<T> {
    if let Some(made) = cached::<T>(diagram, making.key) {
        return Asked::Ready(made);
    }
    #[cfg(debug_assertions)]
    let in_page_weight = crate::app::debug_hooks::layout_weight().unwrap_or(making.in_page_weight);
    #[cfg(not(debug_assertions))]
    let in_page_weight = making.in_page_weight;
    let weight = layout::weight(diagram, in_page_weight);
    if weight <= in_page_weight {
        SERVICE.with(|service| service.borrow_mut().stats.small += 1);
        return Asked::Ready(Rc::new(here(diagram)));
    }
    let id = SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let ready = service.client.as_ref().is_some_and(Client::is_ready);
        ready.then(|| {
            service.next_id += 1;
            service.next_id
        })
    });
    let Some(id) = id else {
        // There is no worker (yet, or just now).
        SERVICE.with(|service| service.borrow_mut().stats.fallbacks += 1);
        let made = Rc::new(here(diagram));
        remember(diagram, making.key, &made);
        return Asked::Ready(made);
    };

    let for_delivery = diagram.clone();
    let for_request = diagram.clone();
    let key = making.key;
    let waiting = Waiting {
        id,
        weight,
        what: making.what,
        dimension: making.dimension,
        request: Box::new(move |delay| request(&for_request, delay)),
        deliver: Box::new(move |outcome| {
            let answer = match outcome {
                Outcome::Answer(bytes) => decode(bytes)
                    .inspect_err(|error| tracing::error!("The answer was no good: {error}"))
                    .ok(),
                Outcome::Here => None,
            };
            let good = answer.is_some();
            let made = Rc::new(answer.unwrap_or_else(|| here(&for_delivery)));
            remember(&for_delivery, key, &made);
            on_ready.emit((Ticket(id), made));
            good
        }),
    };
    SERVICE.with(|service| service.borrow_mut().queue.push_back(waiting));
    pump();
    publish();
    Asked::Pending(Ticket(id))
}

/// Says that the layout for the ticket is not wanted after all.
pub fn cancel(ticket: Ticket) {
    SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let before = service.queue.len();
        service.queue.retain(|waiting| waiting.id != ticket.0);
        let mut dropped = before - service.queue.len();
        if let Some(flying) = &mut service.flying {
            if flying.id == ticket.0 && flying.deliver.take().is_some() {
                dropped += 1;
            }
        }
        service.stats.cancelled += dropped;
    });
    publish();
}

/// How many layouts are on their way or waiting for the worker, and others, for tests.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub fn summary() -> String {
    SERVICE.with(|service| {
        let service = service.borrow();
        serde_json::json!({
            "pending": pending(&service),
            "worker": service.stats.by_worker,
            "small": service.stats.small,
            "fallbacks": service.stats.fallbacks,
            "cached": service.stats.cached,
            "cancelled": service.stats.cancelled,
            "ready": service.client.as_ref().is_some_and(Client::is_ready),
        })
        .to_string()
    })
}

// Inside.

fn here_layout<const N: usize>(diagram: &Diagram) -> Layout<N> {
    let started = Instant::now();
    let layout = Layout::new(diagram).expect("the diagram has the dimensions it is laid out in");
    tracing::debug!(
        "Laid out a diagram on the page in {:.1}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    layout
}

fn cached<T: 'static>(diagram: &Diagram, key: usize) -> Option<Rc<T>> {
    SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let position = service
            .cache
            .iter()
            .position(|(cached, kind, _)| *kind == key && cached == diagram)?;
        let entry = service.cache.remove(position)?;
        let layout = Rc::clone(&entry.2).downcast::<T>().ok()?;
        service.cache.push_back(entry);
        service.stats.cached += 1;
        Some(layout)
    })
}

/// Keeps the layout of a large diagram for a while.
fn remember<T: 'static>(diagram: &Diagram, key: usize, made: &Rc<T>) {
    SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let cache = &mut service.cache;
        cache.retain(|(cached, kind, _)| !(*kind == key && cached == diagram));
        cache.push_back((diagram.clone(), key, Rc::clone(made) as Rc<dyn Any>));
        while cache.len() > CACHE_SIZE {
            cache.pop_front();
        }
    });
}

fn pending(service: &Service) -> usize {
    service.queue.len() + usize::from(service.flying.as_ref().is_some_and(|f| f.deliver.is_some()))
}

/// Tells the note how many layouts are being waited for.
fn publish() {
    let count = SERVICE.with(|service| pending(&service.borrow()));
    layout_note::show(count);
}

/// Sends the next layout to the worker, if it is free and there is one to send.
fn pump() {
    let failed = SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let service = &mut *service;
        if service.flying.is_some() {
            return None;
        }
        let client = service.client.as_ref().filter(|client| client.is_ready())?;
        let waiting = service.queue.pop_front()?;
        let encoding = Instant::now();
        #[cfg(debug_assertions)]
        let (delay, fault) = (
            crate::app::debug_hooks::layout_delay_ms(),
            crate::app::debug_hooks::layout_worker_mode(),
        );
        #[cfg(not(debug_assertions))]
        let (delay, fault) = (0, String::new());
        let bytes = (waiting.request)(delay);
        let encode_ms = encoding.elapsed().as_secs_f64() * 1000.0;
        match client.lay_out(waiting.id, &bytes, &fault) {
            Ok(()) => {
                service.flying = Some(Flying {
                    id: waiting.id,
                    weight: waiting.weight,
                    what: waiting.what,
                    dimension: waiting.dimension,
                    deliver: Some(waiting.deliver),
                    sent: Instant::now(),
                    encode_ms,
                    bytes_out: bytes.len(),
                });
                None
            }
            Err(error) => {
                tracing::error!("Could not give the layout to the worker: {:?}", error);
                Some(waiting.deliver)
            }
        }
    });
    if let Some(deliver) = failed {
        deliver_here(deliver);
    }
}

/// Makes the layout on the page, once the page has had a chance to respond to input.
fn deliver_here(deliver: Delivery) {
    spawn_local(async move {
        TimeoutFuture::new(0).await;
        deliver(Outcome::Here);
        publish();
    });
}

/// Does what is to be done when the worker is of no use any more: everything waiting is laid
/// out on the page, and the person is told once.
fn drain_if_gone() {
    let (waiting, tell) = SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let gone = service.client.as_ref().is_some_and(Client::is_disabled);
        if !gone {
            return (vec![], false);
        }
        let tell = !std::mem::replace(&mut service.told, true);
        (service.queue.drain(..).collect(), tell)
    });
    if tell {
        toast(Toast::warn(
            "The background worker for laying out diagrams is not available; the app will lay \
             them out itself and may pause while it does",
        ));
    }
    for waiting in waiting {
        deliver_here(waiting.deliver);
    }
}

/// Takes in what the worker said.
fn heard(generation: u32, event: Event) {
    let current = SERVICE.with(|service| {
        service
            .borrow_mut()
            .client
            .as_mut()
            .is_some_and(|client| client.heard(generation, &event))
    });
    if !current {
        return;
    }
    match event {
        Event::Ready => {
            tracing::info!("The layout worker is ready");
            pump();
        }
        Event::Progress { .. } | Event::Partial { .. } => {}
        Event::Done { job, bytes } => done(job, &bytes),
        Event::Crashed { job, message } => crashed(job, &message),
    }
    publish();
}

/// The worker has answered for a layout.
fn done(job: u64, bytes: &[u8]) {
    let flying = SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        if service.flying.as_ref().is_some_and(|f| f.id == job) {
            service.stats.by_worker += 1;
            service.flying.take()
        } else {
            None
        }
    });
    if let Some(Flying {
        deliver: Some(deliver),
        weight,
        what,
        dimension,
        sent,
        encode_ms,
        bytes_out,
        ..
    }) = flying
    {
        let wall_ms = sent.elapsed().as_secs_f64() * 1000.0;
        let received = Instant::now();
        let good = deliver(Outcome::Answer(bytes));
        tracing::info!(
            "Making the {what} of a {dimension}-dimensional view of a diagram (weight {weight}) \
             took {:.1}ms in the worker: {wall_ms:.1}ms there and back, {encode_ms:.1}ms to \
             prepare and {:.1}ms to take in the answer ({}KB there, {}KB back)",
            wall_ms + encode_ms,
            received.elapsed().as_secs_f64() * 1000.0,
            bytes_out / 1024,
            bytes.len() / 1024
        );
        if !good {
            // The worker failed without crashing: it is replaced, and the layout was made here.
            SERVICE.with(|service| {
                if let Some(client) = service.borrow_mut().client.as_mut() {
                    client.fail();
                    client.stop();
                    client.ensure();
                }
            });
            drain_if_gone();
        }
    }
    pump();
}

/// The worker crashed, or never started. If it was making a layout, that is made here instead.
fn crashed(job: Option<u64>, message: &str) {
    tracing::error!("The layout worker crashed: {}", message);
    let flying = SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        let lost = service
            .flying
            .as_ref()
            .is_some_and(|f| job.is_none_or(|job| job == f.id));
        let flying = if lost { service.flying.take() } else { None };
        if let Some(client) = service.client.as_mut() {
            client.stop();
            client.ensure();
        }
        flying
    });
    if let Some(deliver) = flying.and_then(|flying| flying.deliver) {
        deliver_here(deliver);
    }
    drain_if_gone();
}
