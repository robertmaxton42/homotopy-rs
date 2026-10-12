use homotopy_core::{common::Generator, signature::Invertibility, Diagram};
use homotopy_graphics::style::{Color, GeneratorStyle, VertexShape};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // Independent flags.
pub struct GeneratorInfo {
    pub generator: Generator,
    pub oriented: bool,
    pub invertibility: Invertibility,
    pub single_preview: bool,
    pub color: Color,
    pub shape: VertexShape,
    pub diagram: Diagram,
    pub name: String,
    /// Whether this generator is a placeholder, which generators mentioning it (axiom schemas)
    /// can be instantiated at (see [`Signature::instantiate`](super::Signature::instantiate)).
    pub placeholder: bool,
    /// If this generator is an instance of an axiom schema, the schema and substitution.
    pub instance: Option<Instance>,
    /// Whether the instances of this generator are shown in the signature.
    pub instances_open: bool,
}

/// An instance `A[σ]` of a generator `A` mentioning placeholders: the substitution `σ` maps each
/// instantiated placeholder to the diagram it is replaced with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Instance {
    pub schema: Generator,
    pub substitution: Vec<(Generator, Diagram)>,
}

impl GeneratorStyle for GeneratorInfo {
    fn label(&self) -> Option<String> {
        // TODO(thud): Decide whether to show a label
        // Some(self.name.clone())
        None
    }

    fn shape(&self) -> VertexShape {
        self.shape.clone()
    }

    fn color(&self) -> Color {
        self.color.clone()
    }
}

impl homotopy_core::signature::GeneratorInfo for GeneratorInfo {
    fn diagram(&self) -> &Diagram {
        &self.diagram
    }

    fn invertibility(&self) -> Invertibility {
        self.invertibility
    }
}
