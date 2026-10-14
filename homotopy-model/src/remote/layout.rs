//! Laying out a diagram in another instance of this code (the layout worker, or a thread of its
//! own).
//!
//! Unlike the jobs of the parent module this needs no proof: a request is a diagram (through a
//! [`Store`], like every diagram that crosses) and how many of its dimensions are to be laid out,
//! and the answer is the [`Layout`], which is plain data (the positions of points, with no
//! diagrams in it). The positions are sent flat and compactly: the points as numbers, the
//! positions as floats.

use homotopy_core::{
    common::{Boundary, Height, SliceIndex},
    layout::Layout,
    serialize::{Key, Store},
    Diagram,
};
use instant::Instant;
use serde::{Deserialize, Serialize};

/// The most dimensions that are laid out (those of a four-dimensional view).
const MAX_DIMENSION: usize = 4;

#[derive(Serialize, Deserialize)]
struct Request {
    store: Store,
    diagram: Key<Diagram>,
    dimension: u8,
    /// For debugging: how long to take at least, in milliseconds.
    delay_ms: u32,
}

#[derive(Serialize, Deserialize)]
enum Response {
    Done(WireLayout),
    Failed(String),
}

#[derive(Serialize, Deserialize)]
struct WireLayout {
    dimension: u8,
    /// The coordinates of the points, `dimension` for each (see [`encode`]).
    points: Vec<u32>,
    /// Where the points are, `dimension` floats for each, in the same order.
    positions: Vec<f32>,
    /// How long it took to lay out, in milliseconds.
    worked_ms: f64,
}

const fn encode_index(index: SliceIndex) -> u32 {
    match index {
        SliceIndex::Boundary(Boundary::Source) => 0,
        SliceIndex::Boundary(Boundary::Target) => 1,
        SliceIndex::Interior(Height::Regular(h)) => 2 + 2 * h as u32,
        SliceIndex::Interior(Height::Singular(h)) => 3 + 2 * h as u32,
    }
}

const fn decode_index(code: u32) -> SliceIndex {
    match code {
        0 => SliceIndex::Boundary(Boundary::Source),
        1 => SliceIndex::Boundary(Boundary::Target),
        c if c % 2 == 0 => SliceIndex::Interior(Height::Regular(((c - 2) / 2) as usize)),
        c => SliceIndex::Interior(Height::Singular(((c - 3) / 2) as usize)),
    }
}

fn pack<const N: usize>(layout: &Layout<N>, worked_ms: f64) -> WireLayout {
    let mut points = Vec::with_capacity(layout.len() * N);
    let mut positions = Vec::with_capacity(layout.len() * N);
    for (point, position) in layout.points() {
        points.extend(point.iter().map(|index| encode_index(*index)));
        positions.extend_from_slice(position);
    }
    WireLayout {
        dimension: N as u8,
        points,
        positions,
        worked_ms,
    }
}

fn unpack<const N: usize>(wire: &WireLayout) -> Option<Layout<N>> {
    if usize::from(wire.dimension) != N || wire.points.len() != wire.positions.len() {
        return None;
    }
    if N == 0 {
        // A single point with no coordinates, which the flat lists cannot count.
        return Some(Layout::from_points([(
            [SliceIndex::Boundary(Boundary::Source); N],
            [0.0; N],
        )]));
    }
    let (points, loose) = wire.points.as_chunks::<N>();
    let (positions, also_loose) = wire.positions.as_chunks::<N>();
    if !loose.is_empty() || !also_loose.is_empty() {
        return None;
    }
    Some(Layout::from_points(points.iter().zip(positions).map(
        |(point, position)| (point.map(decode_index), *position),
    )))
}

fn compute<const N: usize>(diagram: &Diagram) -> Response {
    let started = Instant::now();
    match Layout::<N>::new(diagram) {
        Ok(layout) => Response::Done(pack(&layout, started.elapsed().as_secs_f64() * 1000.0)),
        Err(error) => Response::Failed(format!("cannot lay out: {error:?}")),
    }
}

fn to_bytes(value: &impl Serialize) -> Vec<u8> {
    rmp_serde::to_vec_named(value).expect("encoding to memory cannot fail")
}

// The side that asks.

/// The bytes that ask for the first `N` dimensions of the diagram to be laid out.
#[must_use]
pub fn request<const N: usize>(diagram: &Diagram, delay_ms: u32) -> Vec<u8> {
    let mut store = Store::default();
    let diagram = store.pack_diagram(diagram);
    to_bytes(&Request {
        store,
        diagram,
        dimension: N as u8,
        delay_ms,
    })
}

/// Makes sense of the answer to a request, returning the layout and how long it took to make where
/// it was made, in milliseconds.
pub fn response<const N: usize>(bytes: &[u8]) -> Result<(Layout<N>, f64), String> {
    match rmp_serde::from_slice(bytes).map_err(|e| e.to_string())? {
        Response::Failed(message) => Err(message),
        Response::Done(wire) => {
            let layout = unpack::<N>(&wire).ok_or("the layout was garbled")?;
            Ok((layout, wire.worked_ms))
        }
    }
}

// The side that works.

/// Does the layout that the request describes, and gives the bytes to answer with.
///
/// `pause` is told how many milliseconds the layout is to be delayed for (to have something slow
/// to test with), and does that.
pub fn run(request: &[u8], pause: &mut dyn FnMut(u32)) -> Vec<u8> {
    let response = match rmp_serde::from_slice::<Request>(request) {
        Err(error) => Response::Failed(error.to_string()),
        Ok(Request {
            mut store,
            diagram,
            dimension,
            delay_ms,
        }) => match store.unpack_diagram(diagram) {
            None => Response::Failed("the message was garbled".to_owned()),
            Some(diagram) => {
                pause(delay_ms);
                match usize::from(dimension) {
                    0 => compute::<0>(&diagram),
                    1 => compute::<1>(&diagram),
                    2 => compute::<2>(&diagram),
                    3 => compute::<3>(&diagram),
                    4 => compute::<4>(&diagram),
                    n => Response::Failed(format!("{n} is more than {MAX_DIMENSION} dimensions")),
                }
            }
        },
    };
    homotopy_core::collect_garbage();
    to_bytes(&response)
}
