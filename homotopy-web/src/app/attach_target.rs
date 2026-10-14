//! Attaching one generator that was picked from the signature, by dragging it to the diagram.
//!
//! A drag from the signature carries the generator in two ways: as the plain text that moving
//! generators around the signature uses (so that dropping it in the signature still moves it),
//! and under [`MIME`], which only the diagram looks at. The browser does not let a page read
//! what is being dragged until the drop, so the diagram, which has to say whether it takes the
//! drop while the pointer is still over it, is told here which generator it is and whether it
//! can be attached to the diagram at all: that is worked out once, when the drag starts, from
//! the diagram and the cells (see [`can_attach`](homotopy_model::proof::can_attach)), and never
//! by searching.

use std::cell::RefCell;

use homotopy_core::{Diagram, Generator};
use homotopy_model::proof::{can_attach, CannotAttach, Signature};
use web_sys::DataTransfer;

use super::{attach_inference, settings::AppSettings};

/// The type of the data of a drag that carries a generator to attach.
pub const MIME: &str = "application/x-homotopy-generator";

/// The generator being dragged and whether the diagram can take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dragged {
    pub generator: Generator,
    pub verdict: Result<(), CannotAttach>,
}

thread_local! {
    static DRAGGED: RefCell<Option<Dragged>> = const { RefCell::new(None) };
}

/// What is being dragged to the diagram, if a generator is.
#[must_use]
pub fn dragged() -> Option<Dragged> {
    DRAGGED.with(|dragged| dragged.borrow().clone())
}

/// A drag of the generator from the signature has started.
pub fn start(
    signature: &Signature,
    workspace: Option<&Diagram>,
    generator: Generator,
    transfer: &DataTransfer,
) {
    // "copyMove": the signature moves it, the diagram attaches a copy of it.
    transfer.set_effect_allowed("copyMove");
    transfer
        .set_data(MIME, &format!("{},{}", generator.id, generator.dimension))
        .ok();
    let verdict = can_attach(
        signature,
        workspace,
        generator,
        AppSettings::get_weak_units(),
        attach_inference::limits(),
    );
    DRAGGED.with(|dragged| *dragged.borrow_mut() = Some(Dragged { generator, verdict }));
}

/// The drag is over, whether or not it was dropped.
pub fn end() {
    DRAGGED.with(|dragged| *dragged.borrow_mut() = None);
}

/// The generator that a drop carries, if it is one that was dragged from the signature.
#[must_use]
pub fn dropped(transfer: &DataTransfer) -> Option<Generator> {
    let data = transfer.get_data(MIME).ok()?;
    let (id, dimension) = data.split_once(',')?;
    Some(Generator::new(id.parse().ok()?, dimension.parse().ok()?))
}

/// Whether a drag over the diagram carries a generator (all that can be told before the drop).
#[must_use]
pub fn carries_generator(transfer: &DataTransfer) -> bool {
    transfer
        .types()
        .iter()
        .any(|kind| kind.as_string().as_deref() == Some(MIME))
}
