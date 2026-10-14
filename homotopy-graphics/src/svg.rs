use std::fmt::Write;

use homotopy_core::{Generator, Orientation};

use crate::style::{GeneratorRepresentation, GeneratorStyle, SignatureStyleData};

pub mod render;
pub mod shape;

/// How translucent placeholders are drawn.
const FILL_OPACITY: f32 = 0.4;
const STROKE_OPACITY: f32 = 0.65;
const WIRE_DASH: &str = "7 5";

/// The id of the pattern with which the surfaces of placeholders are hatched (on top of being
/// translucent), in dark stripes.
///
/// The second is the pattern with light stripes, for colours that dark stripes do not show on (see [`Color::stripes`](crate::style::Color::stripes)). It is up to the
/// renderer of the SVG to define them.
pub const PLACEHOLDER_HATCH_ID: &str = "placeholder-hatch";
pub const PLACEHOLDER_HATCH_LIGHT_ID: &str = "placeholder-hatch-light";

/// The index of the lightening of a generator's colour (the `c` of [`Color::lighten`]) when it is
/// drawn in a diagram of the given dimension as a point, wire or surface.
#[inline]
#[must_use]
pub const fn color_index(
    generator: Generator,
    diagram_dimension: usize,
    representation: GeneratorRepresentation,
) -> usize {
    diagram_dimension.saturating_sub(generator.dimension + representation as usize) % 3
}

macro_rules! write_styles_for {
    (
        @c_r
        $c:expr,
        $orientation:expr,
        $generator:expr,
        $color:expr,
        $placeholder:expr,
        $stylesheet:expr
     ) => {{
        let name = generator_class($generator, $c, $orientation);
        let color = $color.lighten($c, $orientation).hex();
        writeln!($stylesheet, ".{name} {{ fill: {color}; stroke: {color}; }}").unwrap();
        if $placeholder {
            // Placeholders are translucent, with dashed wires (and hatched surfaces, which the
            // renderer draws on top of them using `PLACEHOLDER_HATCH_ID`).
            writeln!(
                $stylesheet,
                ".{name} {{ fill-opacity: {FILL_OPACITY}; stroke-opacity: {STROKE_OPACITY}; }} \
                 .{name}.wire {{ stroke-dasharray: {WIRE_DASH}; }}",
            )
            .unwrap();
        }
    }};
    (
        $generator:expr,
        $styles:expr,
        $stylesheet:expr
    ) => {{
        let style = $styles.generator_style($generator).unwrap();
        let (color, placeholder) = (style.color(), style.is_placeholder());

        for c in 0..3 {
            write_styles_for!(@c_r c, Orientation::Positive, $generator, color, placeholder, $stylesheet);
            write_styles_for!(@c_r c, Orientation::Zero, $generator, color, placeholder, $stylesheet);
            write_styles_for!(@c_r c, Orientation::Negative, $generator, color, placeholder, $stylesheet);
        }
    }};
}

pub fn stylesheet(styles: &impl SignatureStyleData) -> String {
    let mut stylesheet = String::new();

    writeln!(
        stylesheet,
        ".wire {{ fill: none !important; }} .point {{ stroke: none !important; }}",
    )
    .unwrap();

    for generator in styles.generators() {
        write_styles_for!(generator, styles, stylesheet);
    }

    stylesheet
}

#[inline]
#[must_use]
pub fn generator_class_from_diagram_dim(
    generator: Generator,
    orientation: Orientation,
    diagram_dimension: usize,
    representation: GeneratorRepresentation,
) -> String {
    let c = color_index(generator, diagram_dimension, representation);

    format!(
        "{} {}",
        generator_class(generator, c, orientation),
        representation,
    )
}

#[inline]
fn generator_class(generator: Generator, c: usize, orientation: Orientation) -> String {
    format!(
        "generator__{}-{}--{}-{}",
        generator.id,
        generator.dimension,
        c,
        match orientation {
            Orientation::Positive => "pos",
            Orientation::Negative => "neg",
            Orientation::Zero => "zer",
        }
    )
}
