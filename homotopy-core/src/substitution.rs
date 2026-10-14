//! Substituting diagrams for generators.
//!
//! A diagram is rebuilt height by height: each height must be a single generator attached at
//! some embedding (possibly reflected), which is replayed with the generator's replacement
//! attached instead, at the corresponding embedding into the already substituted slice.

use thiserror::Error;

use crate::{
    common::{Boundary, Direction, Generator, Height},
    diagram::{AttachmentError, Diagram, DiagramN},
    rewrite::Cospan,
    signature::{GeneratorInfo, Signature},
};

#[derive(Debug, Error)]
pub enum SubstitutionError {
    #[error(
        "height {height} of a {dimension}-diagram is not a single generator (it may contract \
         several cells into one height, or be a homotopy); separate it into single steps first"
    )]
    NotAtomic { height: usize, dimension: usize },

    #[error("a 0-cell can only be replaced by a 0-cell")]
    ZeroCell,

    #[error("a generator can only be replaced by a diagram of the same dimension")]
    Dimension,

    #[error("the diagram mentions a generator that is not in the signature")]
    UnknownGenerator,

    #[error(transparent)]
    Attachment(#[from] AttachmentError),
}

/// Replaces generators in a diagram.
///
/// `replacement(g)` is the diagram replacing (the diagram of) the generator `g`, or `None` to
/// leave `g` alone. A replacement must have the same dimension as its generator, and the same
/// source and target after substitution.
pub fn substitute(
    diagram: &Diagram,
    signature: &impl Signature,
    replacement: &impl Fn(Generator) -> Option<Diagram>,
) -> Result<Diagram, SubstitutionError> {
    match diagram {
        Diagram::Diagram0(d) => match replacement(d.generator) {
            None => Ok(diagram.clone()),
            Some(Diagram::Diagram0(r)) => Ok(r.orientation_transform(d.orientation).into()),
            Some(Diagram::DiagramN(_)) => Err(SubstitutionError::ZeroCell),
        },
        Diagram::DiagramN(d) => substitute_n(d, signature, replacement).map(Into::into),
    }
}

fn substitute_n(
    diagram: &DiagramN,
    signature: &impl Signature,
    replacement: &impl Fn(Generator) -> Option<Diagram>,
) -> Result<DiagramN, SubstitutionError> {
    let n = diagram.dimension();
    let mentions_replaced = |d: &Diagram| d.generators().keys().any(|&g| replacement(g).is_some());
    let mut diagram = diagram.clone();
    let mut result = DiagramN::new(
        substitute(&diagram.source(), signature, replacement)?,
        vec![],
    );

    let mut height = 0;
    while height < diagram.size() {
        let cospan = diagram.cospans()[height].clone();
        let regular = diagram.slice(Height::Regular(height)).unwrap();

        if let Some((generator, mask, reflected, embedding)) =
            attachment(&cospan, &regular, n, signature)?
        {
            // Replay the attachment with the generator replaced.
            let needle = match replacement(generator) {
                None => reflected,
                Some(Diagram::DiagramN(r)) if r.dimension() == n => {
                    r.reflect(&Direction::from_mask(mask, n), 0)
                }
                Some(_) => return Err(SubstitutionError::Dimension),
            };
            let embedding = shift_embedding(&embedding, &regular, replacement);
            result = result.attach(&needle, Boundary::Target, &embedding)?;
            height += 1;
        } else if !mentions_replaced(&regular)
            && !mentions_replaced(&diagram.slice(Height::Singular(height)).unwrap())
            && result.target() == regular
        {
            // Nothing is replaced around this height, so it can stay as it is.
            let mut cospans = result.cospans().to_vec();
            cospans.push(cospan);
            result = DiagramN::new(result.source(), cospans);
            height += 1;
        } else {
            // Separate the cells in this height into consecutive heights and try again.
            diagram =
                separate(&diagram, height, signature).ok_or(SubstitutionError::NotAtomic {
                    height,
                    dimension: n,
                })?;
        }
    }

    Ok(result)
}

/// If the step at a height attaches a single generator, which reflection of its diagram is
/// attached where.
#[allow(clippy::type_complexity)]
fn attachment(
    cospan: &Cospan,
    regular: &Diagram,
    n: usize,
    signature: &impl Signature,
) -> Result<Option<(Generator, u64, DiagramN, Vec<usize>)>, SubstitutionError> {
    let Some(point) = cospan
        .max_generator()
        .filter(|point| point.generator.dimension == n)
    else {
        return Ok(None);
    };
    let Diagram::DiagramN(generator) = signature
        .generator_info(point.generator)
        .ok_or(SubstitutionError::UnknownGenerator)?
        .diagram()
        .clone()
    else {
        return Ok(None);
    };

    // The reflections of the generator are only computed (there are `2ⁿ - 1` of them, each of
    // which is a diagram to rebuild) if the generator itself does not fit, which is rare.
    let reflections = std::iter::once(()).flat_map(|()| {
        generator.reflections().into_iter().map(|(directions, d)| {
            let mask = directions.iter().fold(0, |mask, j| mask | 1 << (j - 1));
            (mask, d)
        })
    });
    Ok(std::iter::once((0, generator.clone()))
        .chain(reflections)
        .filter(|(_, d)| d.max_generator().orientation == point.orientation)
        .find_map(|(mask, d)| {
            let embedding = regular
                .embeddings(&d.source())
                .find(|e| d.cospans()[0].pad(e) == *cospan)?;
            Some((point.generator, mask, d, embedding))
        }))
}

/// Splits the step at a height that does several things at once into two steps, by an
/// expansion, if possible.
fn separate(diagram: &DiagramN, height: usize, signature: &impl Signature) -> Option<DiagramN> {
    let size = diagram.slice(Height::Singular(height))?.size()?;
    (0..size)
        .flat_map(|j| [Direction::Forward, Direction::Backward].map(|d| (j, d)))
        .find_map(|(j, direction)| {
            let expanded = diagram
                .clone()
                .identity()
                .expand(
                    Boundary::Target.into(),
                    &mut [],
                    [Height::Singular(height), Height::Singular(j)],
                    direction,
                    signature,
                )
                .ok()?;
            DiagramN::try_from(expanded.target())
                .ok()
                .filter(|d| d.size() == diagram.size() + 1)
        })
}

/// The embedding into the substituted slice corresponding to an embedding into `slice`: at
/// each depth, every replaced generator occurring at a lower height of the selected slice of
/// its own dimension grows or shrinks to the size of its replacement.
fn shift_embedding(
    embedding: &[usize],
    slice: &Diagram,
    replacement: &impl Fn(Generator) -> Option<Diagram>,
) -> Vec<usize> {
    let mut shifted = Vec::with_capacity(embedding.len());
    let mut slice = slice.clone();
    for &offset in embedding {
        let Diagram::DiagramN(nested) = slice else {
            break;
        };
        let k = nested.dimension();
        let mut shift = 0_isize;
        for height in 0..offset {
            let point = nested
                .slice(Height::Singular(height))
                .unwrap()
                .max_generator();
            if point.generator.dimension == k {
                if let Some(Diagram::DiagramN(r)) = replacement(point.generator) {
                    shift += r.size() as isize - 1;
                }
            }
        }
        shifted.push((offset as isize + shift) as usize);
        slice = nested.slice(Height::Regular(offset)).unwrap();
    }
    shifted
}
