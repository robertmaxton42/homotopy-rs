//! Finding the ways to attach a generator at a point of the workspace diagram.

use homotopy_core::{
    common::{BoundaryPath, Generator},
    signature::Signature as _,
    typecheck::{typecheck, Mode},
    Boundary, Diagram, DiagramN, Height, SliceIndex,
};

use super::{generators::GeneratorInfo, AttachOption, ProofState, Signature, Structure};
use crate::job::{Job, Progress};

/// Why there is nothing to attach to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoAttachment {
    /// There is no diagram in the workspace.
    NoWorkspace,
    /// The point is on a boundary that the diagram does not have.
    NoBoundary,
}

/// The search for the [`AttachOption`]s at a point of the workspace diagram, a generator at a time.
///
/// This is what happens on every click in the workspace, and the time it takes grows with the
/// size of the signature and the number of schemas that are tried (see
/// [`Signature::infer_instances`]).
#[derive(Debug)]
pub struct AttachOptionsJob {
    signature: Signature,
    generators: Vec<GeneratorInfo>,
    next: usize,
    haystack: Diagram,
    boundary: Boundary,
    boundary_path: Option<BoundaryPath>,
    point: Vec<Height>,
    weak_units: bool,
    matches: Vec<AttachOption>,
}

impl AttachOptionsJob {
    /// The search for the ways to attach generators at a point of the diagram in the workspace
    /// (given relative to what it shows), also by way of weak units if `weak_units`.
    pub fn new(
        proof: &ProofState,
        point: &[SliceIndex],
        weak_units: bool,
    ) -> Result<Self, NoAttachment> {
        let workspace = proof.workspace.as_ref().ok_or(NoAttachment::NoWorkspace)?;

        let point = {
            let mut point_with_path: Vec<SliceIndex> = workspace.path.iter().copied().collect();
            point_with_path.extend(point);
            point_with_path
        };

        let (boundary_path, point) = BoundaryPath::split(&point);

        let haystack = match boundary_path {
            None => workspace.diagram.clone(),
            Some(boundary_path) => DiagramN::try_from(workspace.diagram.clone())
                .ok()
                .and_then(|diagram| diagram.boundary(boundary_path))
                .ok_or(NoAttachment::NoBoundary)?,
        };

        Ok(Self {
            signature: proof.signature.clone(),
            generators: proof.signature.iter().cloned().collect(),
            next: 0,
            haystack,
            boundary: boundary_path.map_or(Boundary::Target, BoundaryPath::boundary),
            boundary_path,
            point,
            weak_units,
            matches: vec![],
        })
    }

    /// Finds the ways to attach the generator.
    fn generator(&mut self, info: &GeneratorInfo) {
        let Self {
            signature,
            haystack,
            boundary,
            boundary_path,
            point,
            weak_units,
            matches,
            ..
        } = self;
        let (boundary, boundary_path, weak_units) = (*boundary, *boundary_path, *weak_units);

        macro_rules! extend {
            ($diagram:expr, $structure:expr) => {
                let needle = $diagram.slice(boundary.flip()).unwrap();
                matches.extend(
                    haystack
                        .embeddings(&needle)
                        .filter(|embedding| needle.contains_point(point, embedding))
                        .map(|embedding| AttachOption {
                            generator: info.generator,
                            diagram: $diagram,
                            label: $structure
                                .map(|structure: Structure| structure.label(&info.name)),
                            boundary_path,
                            embedding: embedding.into_iter().collect(),
                            substitution: None,
                        }),
                );
            };
        }
        let bubble = |mut diagram: DiagramN| {
            while diagram.dimension() < haystack.dimension() + 1 {
                diagram = diagram.bubble().unwrap();
            }
            diagram
        };
        let weak_unit = |mut diagram: Diagram| {
            while diagram.dimension() < haystack.dimension() + 1 {
                diagram = diagram.weak_identity().into();
            }
            DiagramN::try_from(diagram).unwrap()
        };

        match info.generator.dimension.cmp(&(haystack.dimension() + 1)) {
            std::cmp::Ordering::Less => {
                // How many dimensions the cell is raised by.
                let k = haystack.dimension() + 1 - info.generator.dimension;
                if weak_units {
                    extend!(
                        weak_unit(info.diagram.clone()),
                        Some(Structure::Identity(k))
                    );
                }

                if let Diagram::DiagramN(d) = &info.diagram {
                    if info.invertibility.is_invertible() {
                        if weak_units {
                            extend!(
                                weak_unit(d.inverse().into()),
                                Some(Structure::InverseIdentity(k))
                            );
                        }

                        extend!(bubble(d.clone()), Some(Structure::Bubble(k)));
                        extend!(bubble(d.inverse()), Some(Structure::InverseBubble(k)));
                    }
                }
            }
            std::cmp::Ordering::Equal => {
                if let Diagram::DiagramN(d) = &info.diagram {
                    extend!(d.clone(), None);
                    if info.invertibility.is_invertible() {
                        extend!(d.inverse(), Some(Structure::Inverse));
                    }
                    // Instances of schemas, with placeholders inferred from the point.
                    for inferred in
                        signature.infer_instances(info.generator, haystack, boundary, point)
                    {
                        matches.push(AttachOption {
                            generator: info.generator,
                            label: Some(
                                Structure::Instance(
                                    signature.describe_substitution(&inferred.substitution),
                                )
                                .label(&info.name),
                            ),
                            diagram: inferred.diagram,
                            boundary_path,
                            embedding: inferred.embedding.into_iter().collect(),
                            substitution: Some(inferred.substitution),
                        });
                    }
                    extend_with_reflections(
                        signature,
                        matches,
                        info,
                        d,
                        haystack,
                        point,
                        boundary,
                        boundary_path,
                    );
                }
            }
            std::cmp::Ordering::Greater => (),
        }
    }
}

impl Job for AttachOptionsJob {
    type Output = Vec<AttachOption>;

    fn label(&self) -> &'static str {
        "Finding what can be attached"
    }

    fn progress(&self) -> Option<Progress> {
        Some(Progress {
            done: self.next,
            total: self.generators.len(),
        })
    }

    fn step(&mut self) -> Option<Self::Output> {
        let Some(info) = self.generators.get(self.next).cloned() else {
            return Some(std::mem::take(&mut self.matches));
        };
        self.next += 1;
        self.generator(&info);
        (self.next >= self.generators.len()).then(|| std::mem::take(&mut self.matches))
    }
}

/// Offers the well-typed reflections of a generator other than its inverse: duals, mates,
/// adjoints and so on. These are only valid when some of the generators involved are
/// invertible or dualisable, or when the signature has symmetries, and are typechecked only
/// once they are known to match. Reflections by a symmetry are tagged with its name.
#[allow(clippy::too_many_arguments)]
fn extend_with_reflections(
    signature: &Signature,
    matches: &mut Vec<AttachOption>,
    info: &GeneratorInfo,
    diagram: &DiagramN,
    haystack: &Diagram,
    point: &[Height],
    boundary: Boundary,
    boundary_path: Option<BoundaryPath>,
) {
    let any_invertible = Diagram::from(diagram.clone())
        .generators()
        .keys()
        .any(|g: &Generator| {
            signature
                .generator_info(*g)
                .is_some_and(|info| info.invertibility.is_invertible())
        });
    if !any_invertible && signature.symmetries().is_empty() {
        return;
    }

    let n = diagram.dimension();
    for (directions, reflected) in diagram.reflections() {
        if info.invertibility.is_invertible() && directions == [n] {
            // Already offered as the inverse.
            continue;
        }
        let needle = reflected.slice(boundary.flip()).unwrap();
        let embeddings: Vec<_> = haystack
            .embeddings(&needle)
            .filter(|embedding| needle.contains_point(point, embedding))
            .collect();
        if embeddings.is_empty()
            || typecheck(&reflected.clone().into(), signature, Mode::default(), true).is_err()
        {
            continue;
        }
        let mask = directions.iter().fold(0_u64, |mask, j| mask | 1 << (j - 1));
        let structure = signature
            .symmetries()
            .iter()
            .find(|symmetry| symmetry.mask() & ((1 << n) - 1) == mask)
            .map_or_else(
                || Structure::Reflection(directions.clone()),
                |symmetry| Structure::Symmetry(symmetry.name.clone()),
            );
        let label = structure.label(&info.name);
        matches.extend(embeddings.into_iter().map(|embedding| AttachOption {
            generator: info.generator,
            diagram: reflected.clone(),
            label: Some(label.clone()),
            boundary_path,
            embedding: embedding.into_iter().collect(),
            substitution: None,
        }));
    }
}
