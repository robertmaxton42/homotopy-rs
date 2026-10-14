use std::{fmt, str::FromStr};

use homotopy_core::{signature::Signature, Generator, Orientation};
use palette::{convert::FromColor, Hsl, Srgb};
use serde::{Deserialize, Serialize};

pub trait GeneratorStyle {
    fn color(&self) -> Color;
    fn shape(&self) -> VertexShape;
    fn label(&self) -> Option<String>;

    /// Whether the generator is a placeholder. Placeholders are drawn in a way that ordinary
    /// generators cannot imitate (hatched and translucent) so that they are never mistaken for
    /// genuine generators.
    fn is_placeholder(&self) -> bool {
        false
    }
}

pub trait SignatureStyleData: Signature {
    type Style: GeneratorStyle;

    fn generator_style(&self, g: Generator) -> Option<&Self::Style>;
}

/// The relative luminance (WCAG, 0 to 1) below which a colour counts as dark.
///
/// Dark stripes are not seen on a dark colour, so the stripes of placeholders are drawn light on
/// it. That is black and the darkest colours (a luminance of 0.05 is a grey of about `#3f3f3f`); the colours of
/// the palette other than black stay as they were (the next darkest has 0.13).
pub const LIGHT_STRIPES_BELOW: f32 = 0.05;

/// What the stripes of a placeholder are drawn in, which is what its colour does not hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stripes {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub(crate) Srgb<u8>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VertexShape {
    #[default]
    Circle, // circle / sphere
    Square, // square / cube
}

impl Color {
    const MIN_LIGHTNESS_WRAP: f32 = 0.25;
    const MAX_LIGHTNESS_WRAP: f32 = 0.90;

    #[must_use]
    pub fn hex(&self) -> String {
        let (r, g, b) = self.clone().into_components::<u8>();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    // Used for UI to make sure we always maintain sufficient contrast for legibility.
    #[must_use]
    pub fn is_light(&self) -> bool {
        palette::color_difference::Wcag21RelativeContrast::relative_contrast(
            palette::Srgb::new(1., 1., 1.),
            self.0.into_format::<f32>(),
        ) < 1.5
    }

    // Get color in lightened form. We wrap colors if needed so that they are sufficiently
    // distinguishable for end users.
    //
    // Colors are distinguished as pairs (C, R) where:
    //      C = max(0, D - N - K)
    //      R = -1 | 0 | 1          // orientation of generator
    //      D = [usize]             // diagram dimension
    //      N = [usize]             // generator dimension
    //      K = 0 | 1 | 2           // point | wire | surface (in visible diagram)
    //
    // See: https://github.com/homotopy-io/homotopy-rs/issues/550
    //
    // We treat three kinds of color differently (depending on initial lightness):
    //      lightness < MIN_LIGHTNESS_WRAP
    //          => lighten colours (zero then inverse)
    //      lightness > MAX_LIGHTNESS_WRAP
    //          => lighten colours (zero then inverse) wrapping such that min lightness is never
    //             below MIN_LIGHTNESS_WRAP
    //      otherwise
    //          => lighten colours (zero then inverse) wrapping before reaching MAX_LIGHTNESS_WRAP
    //             and respecting MIN_LIGHTNESS_WRAP
    //
    // The constants used are fairly arbitrary and subject to tweaking.
    #[inline]
    #[must_use]
    pub fn lighten(&self, c: usize, orientation: Orientation) -> Self {
        let mut hsl: Hsl = FromColor::from_color(self.0.into_format::<f32>());
        let (min_lightness, max_lightness) = if hsl.lightness < Self::MIN_LIGHTNESS_WRAP {
            (0., 1.)
        } else if hsl.lightness > Self::MAX_LIGHTNESS_WRAP {
            (Self::MIN_LIGHTNESS_WRAP, 1.)
        } else {
            (Self::MIN_LIGHTNESS_WRAP, Self::MAX_LIGHTNESS_WRAP)
        };
        let r = match orientation {
            Orientation::Positive => 1,
            Orientation::Zero => 0,
            Orientation::Negative => -1,
        };
        let offset = 3 * (1 - r) + (c as isize);
        let o = 0.08 * offset as f32;
        hsl.lightness = (hsl.lightness + o - min_lightness - 0.01)
            % (max_lightness - min_lightness)
            + min_lightness
            + 0.01;
        let srgb: Srgb<f32> = FromColor::from_color(hsl);
        Self(srgb.into_format())
    }

    /// The WCAG relative luminance of the colour: 0 for black, 1 for white.
    #[must_use]
    pub fn relative_luminance(&self) -> f32 {
        let (r, g, b) = self.clone().into_linear_f32_components();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// The colour that the stripes of a placeholder of this colour are drawn in (see
    /// [`Stripes`]): light on a colour that is dark enough for dark stripes not to show.
    #[must_use]
    pub fn stripes(&self) -> Stripes {
        if self.relative_luminance() < LIGHT_STRIPES_BELOW {
            Stripes::Light
        } else {
            Stripes::Dark
        }
    }

    #[must_use]
    pub fn into_components<T>(self) -> (T, T, T)
    where
        T: palette::stimulus::FromStimulus<u8>,
    {
        self.0.into_format().into_components()
    }

    #[must_use]
    pub fn into_linear_f32_components(self) -> (f32, f32, f32) {
        self.0.into_format::<f32>().into_linear().into_components()
    }
}

// Convert from hex string (#rrggbb) to `Color`
impl FromStr for Color {
    type Err = palette::rgb::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        palette::Srgb::<u8>::from_str(s)
            .map(palette::Srgb::into_format)
            .map(Self)
    }
}

// Reverse of `FromStr` (#rrggbb)
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hex())
    }
}

impl Default for Color {
    fn default() -> Self {
        Self(palette::Srgb::new(0, 0, 0))
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub enum GeneratorRepresentation {
    Point = 0,
    Wire = 1,
    Surface = 2,
}

impl std::fmt::Display for GeneratorRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Point => "point",
                Self::Wire => "wire",
                Self::Surface => "surface",
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(hex: &str) -> Color {
        hex.parse().unwrap()
    }

    #[test]
    fn luminance_is_wcag() {
        assert!(color("#000000").relative_luminance().abs() < 1e-6);
        assert!((color("#ffffff").relative_luminance() - 1.0).abs() < 1e-4);
        // Pure green is the brightest primary: 0.7152.
        assert!((color("#00ff00").relative_luminance() - 0.7152).abs() < 1e-4);
        assert!((color("#808080").relative_luminance() - 0.2159).abs() < 1e-3);
    }

    #[test]
    fn stripes_are_light_only_on_dark_colours() {
        assert_eq!(color("#000000").stripes(), Stripes::Light);
        assert_eq!(color("#101010").stripes(), Stripes::Light);
        assert_eq!(color("#000033").stripes(), Stripes::Light);
        // The palette's other colours, including the darkest of them, keep dark stripes.
        for hex in [
            "#2980b9", "#c0392b", "#f39c12", "#8e44ad", "#27ae60", "#f1c40f", "#f6f5f4",
        ] {
            assert_eq!(color(hex).stripes(), Stripes::Dark, "{hex}");
        }
        assert_eq!(color("#808080").stripes(), Stripes::Dark);
        // The threshold is a grey of about #3f3f3f.
        assert_eq!(color("#3a3a3a").stripes(), Stripes::Light);
        assert_eq!(color("#454545").stripes(), Stripes::Dark);
    }
}
