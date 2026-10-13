use serde::{Deserialize, Serialize};

use crate::{
    diagram::{globularity, NewDiagramError},
    Diagram, Diagram0, DiagramN, Generator,
};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Serialize, Deserialize)]
pub enum Invertibility {
    Directed,
    Dualisable(usize),
    Invertible,
}

impl std::fmt::Display for Invertibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directed => write!(f, "directed"),
            Self::Dualisable(n) => write!(f, "{n}-dualisable"),
            Self::Invertible => write!(f, "invertible"),
        }
    }
}

impl From<bool> for Invertibility {
    fn from(invertible: bool) -> Self {
        if invertible {
            Self::Invertible
        } else {
            Self::Directed
        }
    }
}

impl Invertibility {
    #[must_use]
    pub const fn is_invertible(self) -> bool {
        !matches!(self, Self::Directed)
    }
}

/// A reflection symmetry of a whole theory, such as `op` on **Cat**.
///
/// Declaring a symmetry asserts that reflecting any diagram in the given directions yields
/// another valid diagram, so that every generator `g` may also appear reflected in the
/// directions of the symmetry that lie within its own dimension. Directions are numbered from
/// the innermost direction `1`, as in [`DiagramN::reflections`].
///
/// For example, if 1-cells are categories and 2-cells functors, the symmetry `{1}` sends
/// `F : C → D` to `F^op : C^op → D^op` and a natural transformation `α : F ⇒ G` to
/// `α^op : F^op ⇒ G^op`, while `{1, 3}` sends `α` to `α^op : G^op ⇒ F^op` instead.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct Symmetry {
    pub name: String,
    pub directions: Vec<usize>,
}

impl Symmetry {
    /// The directions of the symmetry as a bit mask, with bit `j - 1` for direction `j`.
    #[must_use]
    pub fn mask(&self) -> u64 {
        self.directions
            .iter()
            .filter(|&&j| (1..=64).contains(&j))
            .fold(0, |mask, j| mask | 1 << (j - 1))
    }
}

pub trait GeneratorInfo {
    fn diagram(&self) -> &Diagram;
    fn invertibility(&self) -> Invertibility;
}

pub trait Signature {
    type Info: GeneratorInfo;
    fn generators(&self) -> impl Iterator<Item = Generator>;
    fn generator_info(&self, g: Generator) -> Option<&Self::Info>;

    /// The reflection symmetries of the theory (see [`Symmetry`]).
    fn symmetries(&self) -> &[Symmetry] {
        &[]
    }

    fn globular_pairs(&self, generator: Generator) -> Vec<Generator> {
        let diagram = self.generator_info(generator).unwrap().diagram();
        self.generators()
            .filter(|&g| {
                g != generator && globularity(self.generator_info(g).unwrap().diagram(), diagram)
            })
            .collect()
    }
}

/// Helper struct for building signatures in tests and benchmarks.
#[derive(Clone, Debug, Default)]
pub struct SignatureBuilder(Vec<GeneratorData>);

#[derive(Clone, Debug)]
pub struct GeneratorData(Generator, Diagram);

impl GeneratorInfo for GeneratorData {
    fn diagram(&self) -> &Diagram {
        &self.1
    }

    fn invertibility(&self) -> Invertibility {
        (self.0.dimension > 0).into()
    }
}

impl Signature for SignatureBuilder {
    type Info = GeneratorData;

    fn generators(&self) -> impl Iterator<Item = Generator> {
        self.0.iter().map(|gd| gd.0)
    }

    fn generator_info(&self, g: Generator) -> Option<&GeneratorData> {
        self.0.get(g.id)
    }
}

impl SignatureBuilder {
    pub fn add_zero(&mut self) -> Diagram0 {
        let generator = Generator::new(self.0.len(), 0);
        let diagram = Diagram0::from(generator);
        self.0.push(GeneratorData(generator, diagram.into()));
        diagram
    }

    pub fn add(
        &mut self,
        source: impl Into<Diagram>,
        target: impl Into<Diagram>,
    ) -> Result<DiagramN, NewDiagramError> {
        let source: Diagram = source.into();
        let target: Diagram = target.into();
        let generator = Generator::new(self.0.len(), source.dimension() + 1);
        let diagram = DiagramN::from_generator(generator, source, target)?;
        self.0
            .push(GeneratorData(generator, diagram.clone().into()));
        Ok(diagram)
    }
}
