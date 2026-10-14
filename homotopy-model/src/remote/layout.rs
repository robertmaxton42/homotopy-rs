//! Laying a diagram out, or making the geometry of its 3D view, in another instance of this code
//! (the layout worker, or a thread of its own).
//!
//! Unlike the jobs of the parent module this needs no proof: a request is a diagram (through a
//! [`Store`], like every diagram that crosses), how many of its dimensions are to be laid out and
//! what is wanted. The answers are plain data with no diagrams in them (the [`Layout`], which is
//! the positions of points, or the [`CubicalGeometry`] made from it, which is what the 3D view
//! draws), and are written flat, as little-endian numbers, because they can be large and the
//! page has to read them in.

use homotopy_common::idx::Idx;
use homotopy_core::{
    common::{Boundary, Generator, Height, Orientation, SliceIndex},
    layout::Layout,
    serialize::{Key, Store},
    Diagram, Diagram0,
};
use homotopy_graphics::{
    geom::{CubicalGeometry, CurveData, VertData},
    parity::Parity,
};
use instant::Instant;
use serde::{Deserialize, Serialize};
use ultraviolet::Vec4;

/// The most dimensions that are laid out (those of a four-dimensional view).
const MAX_DIMENSION: usize = 4;

/// What is asked for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum What {
    Layout,
    /// The geometry of the view, which includes its layout; with the time as a dimension if
    /// animated.
    Geometry {
        animated: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Request {
    store: Store,
    diagram: Key<Diagram>,
    dimension: u8,
    what: What,
    /// For debugging: how long to take at least, in milliseconds.
    delay_ms: u32,
}

// The answers, which begin with one of these.
const FAILED: u8 = 0;
const LAYOUT: u8 = 1;
const GEOMETRY: u8 = 2;

const GARBLED: &str = "the answer was garbled";

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn usize(&mut self, value: usize) {
        self.u32(u32::try_from(value).unwrap_or(u32::MAX));
    }

    fn f32(&mut self, value: f32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const K: usize>(&mut self) -> Option<[u8; K]> {
        let (head, rest) = self.0.split_first_chunk::<K>()?;
        self.0 = rest;
        Some(*head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    fn usize(&mut self) -> Option<usize> {
        self.u32().map(|value| value as usize)
    }

    fn f32(&mut self) -> Option<f32> {
        self.take().map(f32::from_le_bytes)
    }

    fn f64(&mut self) -> Option<f64> {
        self.take().map(f64::from_le_bytes)
    }

    /// A length that is not more than there is room for, `size` bytes at least for each.
    fn count(&mut self, size: usize) -> Option<usize> {
        let count = self.usize()?;
        (count.checked_mul(size)? <= self.0.len()).then_some(count)
    }
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

const fn encode_orientation(orientation: Orientation) -> u8 {
    match orientation {
        Orientation::Negative => 0,
        Orientation::Zero => 1,
        Orientation::Positive => 2,
    }
}

const fn decode_orientation(code: u8) -> Option<Orientation> {
    match code {
        0 => Some(Orientation::Negative),
        1 => Some(Orientation::Zero),
        2 => Some(Orientation::Positive),
        _ => None,
    }
}

fn put_generator(out: &mut Writer, diagram: Diagram0) {
    out.usize(diagram.generator.dimension);
    out.usize(diagram.generator.id);
    out.u8(encode_orientation(diagram.orientation));
}

fn get_generator(input: &mut Reader) -> Option<Diagram0> {
    let dimension = input.usize()?;
    let id = input.usize()?;
    Some(Diagram0 {
        generator: Generator { id, dimension },
        orientation: decode_orientation(input.u8()?)?,
    })
}

fn put_parity(out: &mut Writer, parity: Parity) {
    out.u8(u8::from(parity.is_even()));
}

fn get_parity(input: &mut Reader) -> Option<Parity> {
    Some(Parity::from(input.u8()? != 0))
}

// The layout.

fn pack_layout<const N: usize>(layout: &Layout<N>, worked_ms: f64) -> Vec<u8> {
    let mut out = Writer::default();
    out.0.reserve(layout.len() * N * 8 + 16);
    out.u8(LAYOUT);
    out.u8(N as u8);
    out.f64(worked_ms);
    out.usize(layout.len());
    for (point, _) in layout.points() {
        for index in point {
            out.u32(encode_index(*index));
        }
    }
    for (_, position) in layout.points() {
        for value in position {
            out.f32(*value);
        }
    }
    out.0
}

fn unpack_layout<const N: usize>(mut input: Reader) -> Option<(Layout<N>, f64)> {
    if usize::from(input.u8()?) != N {
        return None;
    }
    let worked_ms = input.f64()?;
    let count = input.usize()?;
    if N == 0 {
        // A single point with no coordinates.
        let point = ([SliceIndex::Boundary(Boundary::Source); N], [0.0; N]);
        return (count == 1).then(|| (Layout::from_points([point]), worked_ms));
    }
    if count.checked_mul(N * 8)? != input.0.len() {
        return None;
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        let mut key = [SliceIndex::Boundary(Boundary::Source); N];
        for index in &mut key {
            *index = decode_index(input.u32()?);
        }
        points.push(key);
    }
    let mut layout = Vec::with_capacity(count);
    for key in points {
        let mut position = [0.0; N];
        for value in &mut position {
            *value = input.f32()?;
        }
        layout.push((key, position));
    }
    Some((Layout::from_points(layout), worked_ms))
}

// The geometry.

fn pack_geometry(geometry: &CubicalGeometry, animated: bool, worked_ms: f64) -> Vec<u8> {
    let mut out = Writer::default();
    out.u8(GEOMETRY);
    out.u8(u8::from(animated));
    out.f64(worked_ms);

    out.usize(geometry.verts.len());
    for vert in geometry.verts.values() {
        let VertData {
            position,
            boundary,
            generator,
            k,
        } = vert;
        for value in [position.x, position.y, position.z, position.w] {
            out.f32(value);
        }
        out.u8(boundary.iter().fold(0, |mask, b| mask << 1 | u8::from(*b)));
        put_generator(&mut out, *generator);
        out.usize(*k);
    }

    out.usize(geometry.points.len());
    for point in geometry.points.values() {
        out.usize(point.index());
    }
    out.usize(geometry.lines.len());
    for (line, parity) in geometry.lines.values() {
        for vert in line {
            out.usize(vert.index());
        }
        put_parity(&mut out, *parity);
    }
    out.usize(geometry.areas.len());
    for (area, parity) in geometry.areas.values() {
        for vert in area {
            out.usize(vert.index());
        }
        put_parity(&mut out, *parity);
    }
    out.usize(geometry.volumes.len());
    for (volume, parity) in geometry.volumes.values() {
        for vert in volume {
            out.usize(vert.index());
        }
        put_parity(&mut out, *parity);
    }
    out.usize(geometry.curves.len());
    for curve in geometry.curves.values() {
        put_generator(&mut out, curve.generator);
        out.usize(curve.verts.len());
        out.usize(curve.parities.len());
        for vert in &curve.verts {
            out.usize(vert.index());
        }
        for parity in &curve.parities {
            put_parity(&mut out, *parity);
        }
    }
    out.0
}

fn unpack_geometry(mut input: Reader) -> Option<(CubicalGeometry, f64)> {
    use homotopy_graphics::geom::Vert;

    let _animated = input.u8()?;
    let worked_ms = input.f64()?;
    let mut geometry = CubicalGeometry::default();

    for _ in 0..input.count(4 * 4 + 1 + 9 + 4)? {
        let position = Vec4::new(input.f32()?, input.f32()?, input.f32()?, input.f32()?);
        let mask = input.u8()?;
        let boundary = [mask & 8 != 0, mask & 4 != 0, mask & 2 != 0, mask & 1 != 0];
        let generator = get_generator(&mut input)?;
        let k = input.usize()?;
        // `usize::MAX` is written as `u32::MAX`.
        let k = if k == u32::MAX as usize {
            usize::MAX
        } else {
            k
        };
        geometry.mk_vert(VertData {
            position,
            boundary,
            generator,
            k,
        });
    }
    let verts = geometry.verts.len();
    let vert = |input: &mut Reader| -> Option<Vert> {
        let index = input.usize()?;
        (index < verts).then(|| Vert::new(index))
    };

    for _ in 0..input.count(4)? {
        let point = vert(&mut input)?;
        geometry.mk_point(point);
    }
    for _ in 0..input.count(9)? {
        let line = [vert(&mut input)?, vert(&mut input)?];
        geometry.mk_line(line, get_parity(&mut input)?);
    }
    for _ in 0..input.count(17)? {
        let area = [
            vert(&mut input)?,
            vert(&mut input)?,
            vert(&mut input)?,
            vert(&mut input)?,
        ];
        geometry.mk_area(area, get_parity(&mut input)?);
    }
    for _ in 0..input.count(33)? {
        let mut volume = [Vert::new(0); 8];
        for v in &mut volume {
            *v = vert(&mut input)?;
        }
        geometry.mk_volume(volume, get_parity(&mut input)?);
    }
    for _ in 0..input.count(9 + 8)? {
        let generator = get_generator(&mut input)?;
        let length = input.count(4)?;
        let parities = input.count(1)?;
        let verts = (0..length)
            .map(|_| vert(&mut input))
            .collect::<Option<Vec<_>>>()?;
        let parities = (0..parities)
            .map(|_| get_parity(&mut input))
            .collect::<Option<Vec<_>>>()?;
        geometry.curves.push(CurveData {
            verts,
            parities,
            generator,
        });
    }
    input.0.is_empty().then_some((geometry, worked_ms))
}

fn failed(message: &str) -> Vec<u8> {
    let mut out = vec![FAILED];
    out.extend_from_slice(message.as_bytes());
    out
}

fn to_bytes(value: &impl Serialize) -> Vec<u8> {
    rmp_serde::to_vec_named(value).expect("encoding to memory cannot fail")
}

fn compute_layout<const N: usize>(diagram: &Diagram) -> Vec<u8> {
    let started = Instant::now();
    match Layout::<N>::new(diagram) {
        Ok(layout) => pack_layout(&layout, started.elapsed().as_secs_f64() * 1000.0),
        Err(error) => failed(&format!("cannot lay out: {error:?}")),
    }
}

fn compute_geometry<const N: usize>(diagram: &Diagram, animated: bool) -> Vec<u8> {
    let started = Instant::now();
    match CubicalGeometry::new::<N>(diagram, animated) {
        Ok(geometry) => pack_geometry(
            &geometry,
            animated,
            started.elapsed().as_secs_f64() * 1000.0,
        ),
        Err(error) => failed(&format!("cannot make the geometry: {error:?}")),
    }
}

// The side that asks.

fn request(diagram: &Diagram, dimension: usize, what: What, delay_ms: u32) -> Vec<u8> {
    let mut store = Store::default();
    let diagram = store.pack_diagram(diagram);
    to_bytes(&Request {
        store,
        diagram,
        dimension: dimension as u8,
        what,
        delay_ms,
    })
}

/// The bytes that ask for the first `N` dimensions of the diagram to be laid out.
#[must_use]
pub fn request_layout<const N: usize>(diagram: &Diagram, delay_ms: u32) -> Vec<u8> {
    request(diagram, N, What::Layout, delay_ms)
}

/// The bytes that ask for the geometry of the view of the first `N` dimensions of the diagram (as
/// [`CubicalGeometry::new`] makes it).
#[must_use]
pub fn request_geometry<const N: usize>(
    diagram: &Diagram,
    animated: bool,
    delay_ms: u32,
) -> Vec<u8> {
    request(diagram, N, What::Geometry { animated }, delay_ms)
}

fn answer(bytes: &[u8], kind: u8) -> Result<Reader<'_>, String> {
    match bytes.split_first() {
        Some((&FAILED, message)) => Err(String::from_utf8_lossy(message).into_owned()),
        Some((tag, rest)) if *tag == kind => Ok(Reader(rest)),
        _ => Err(GARBLED.to_owned()),
    }
}

/// Makes sense of the answer to a request for a layout, returning the layout and how long it took
/// to make where it was made, in milliseconds.
pub fn response_layout<const N: usize>(bytes: &[u8]) -> Result<(Layout<N>, f64), String> {
    unpack_layout(answer(bytes, LAYOUT)?).ok_or_else(|| GARBLED.to_owned())
}

/// Makes sense of the answer to a request for geometry, returning it and how long it took to make
/// where it was made, in milliseconds.
pub fn response_geometry(bytes: &[u8]) -> Result<(CubicalGeometry, f64), String> {
    unpack_geometry(answer(bytes, GEOMETRY)?).ok_or_else(|| GARBLED.to_owned())
}

// The side that works.

/// Does what the request asks for, and gives the bytes to answer with.
///
/// `pause` is told how many milliseconds the work is to be delayed for (to have something slow to
/// test with), and does that.
pub fn run(request: &[u8], pause: &mut dyn FnMut(u32)) -> Vec<u8> {
    let response = match rmp_serde::from_slice::<Request>(request) {
        Err(error) => failed(&error.to_string()),
        Ok(Request {
            mut store,
            diagram,
            dimension,
            what,
            delay_ms,
        }) => match store.unpack_diagram(diagram) {
            None => failed("the message was garbled"),
            Some(diagram) => {
                pause(delay_ms);
                match (usize::from(dimension), what) {
                    (0, What::Layout) => compute_layout::<0>(&diagram),
                    (1, What::Layout) => compute_layout::<1>(&diagram),
                    (2, What::Layout) => compute_layout::<2>(&diagram),
                    (3, What::Layout) => compute_layout::<3>(&diagram),
                    (4, What::Layout) => compute_layout::<4>(&diagram),
                    (0, What::Geometry { animated }) => compute_geometry::<0>(&diagram, animated),
                    (1, What::Geometry { animated }) => compute_geometry::<1>(&diagram, animated),
                    (2, What::Geometry { animated }) => compute_geometry::<2>(&diagram, animated),
                    (3, What::Geometry { animated }) => compute_geometry::<3>(&diagram, animated),
                    (4, What::Geometry { animated }) => compute_geometry::<4>(&diagram, animated),
                    (n, _) => failed(&format!("{n} is more than {MAX_DIMENSION} dimensions")),
                }
            }
        },
    };
    homotopy_core::collect_garbage();
    response
}
