//! The geometry of the marks that show where sheets meet in the 2D view.
//!
//! See `Depths::echoes` and `Depths::sheets_meeting_at`: the stack of pages at a point where
//! sheets meet, and the echoes of the wires behind, which run along the wire and fade out.
//!
//! All of it is in the coordinates of the picture (y down), and is a few pixels in size whatever
//! the diagram is.

use std::fmt::Write as _;

use euclid::default::Vector2D;
use lyon_path::{iterator::PathIterator as _, Event, Path};

use super::shape::Point;

/// The most pages a fan has: more sheets than this meet at the point, but the fan only says that
/// there are several.
pub const MAX_PAGES: usize = 4;

/// The size of the first page of a fan, in the radius of the vertex it is about: its half width
/// and its half height. The ratio is the flattening of a disc seen from a little above.
const PAGE_WIDTH: f32 = 2.1;
const PAGE_HEIGHT: f32 = 1.2;

/// How much taller each next page is than the one before, in the radius of the vertex (and how
/// much wider, in the same proportion as the first).
const PAGE_RISE: f32 = 0.42;

/// The path as a polyline, from its first point to its last (the first sub-path only).
#[must_use]
pub fn polyline(path: &Path) -> Vec<Point> {
    let mut points = vec![];
    for event in path.iter().flattened(0.25) {
        match event {
            Event::Begin { at } if points.is_empty() => points.push(at),
            Event::Line { to, .. } => points.push(to),
            Event::End { .. } if !points.is_empty() => break,
            _ => {}
        }
    }
    points
}

/// Where the path is when it is `distance` along it.
///
/// It is followed from the end that is nearer to `vertex`, and the result is `(start, reached)`:
/// the other end if the path is shorter. `None` for a path with no length.
#[must_use]
pub fn reach_from(path: &Path, vertex: Point, distance: f32) -> Option<(Point, Point)> {
    let mut points = polyline(path);
    let (first, last) = (*points.first()?, *points.last()?);
    if (first - vertex).square_length() > (last - vertex).square_length() {
        points.reverse();
    }
    let start = points[0];
    let mut left = distance;
    let mut at = start;
    for &next in &points[1..] {
        let length = (next - at).length();
        if length >= left && length > 0.0 {
            return Some((start, at.lerp(next, left / length)));
        }
        left -= length;
        at = next;
    }
    (at != start).then_some((start, at))
}

/// The unit vector at right angles to a wire that goes in `direction`, on the side that is up and
/// to the right (the side the echoes are on, like pages stacked behind). `None` for no direction.
#[must_use]
pub fn echo_side(direction: Vector2D<f32>) -> Option<Vector2D<f32>> {
    let length = direction.length();
    if length <= f32::EPSILON {
        return None;
    }
    let normal = Vector2D::new(-direction.y, direction.x) / length;
    let towards = normal.x - normal.y;
    Some(
        if towards < -1e-3 || (towards.abs() <= 1e-3 && normal.x < 0.0) {
            -normal
        } else {
            normal
        },
    )
}

/// The marks of a fan: the rims of the pages and their undersides, as path data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fan {
    /// The upper edge of each page: an arc from one end of its width over the top to the other.
    pub rims: String,
    /// The whole of each page's outline, of which the lower half is the underside, to be drawn
    /// faintly under the rims: they nearly coincide there.
    pub undersides: String,
}

/// A fan of `pages` pages (at most [`MAX_PAGES`]) at `centre`, a vertex of radius `radius`.
///
/// It is a stack of discs seen from a little above, one for each sheet that meets there: flat
/// ellipses that are each taller and wider than the one before and are all tangent at the bottom,
/// so that their centres rise from the vertex and their lower edges are one curve, and the
/// thickness of the stack shows only on its upper side, as the rims of the pages. A still picture
/// about the vertex, with no backing (the vertex is drawn over its middle).
#[must_use]
pub fn fan(centre: Point, pages: usize, radius: f32) -> Fan {
    let pages = pages.min(MAX_PAGES);
    let mut fan = Fan::default();
    let (width, height) = (PAGE_WIDTH * radius, PAGE_HEIGHT * radius);
    let grow = PAGE_RISE * radius;
    let ratio = width / height;
    for page in 0..pages {
        let ry = height + grow * page as f32;
        let rx = width + grow * ratio.sqrt() * page as f32;
        // Tangent at the bottom, which is the same for every page.
        let cy = centre.y + height - ry;
        let (left, right) = (centre.x - rx, centre.x + rx);
        write!(
            fan.rims,
            "M{left:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 1 {right:.2} {cy:.2}"
        )
        .ok();
        write!(
            fan.undersides,
            "M{left:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 0 {right:.2} {cy:.2}"
        )
        .ok();
    }
    fan
}

#[cfg(test)]
mod tests {
    use lyon_path::builder::NoAttributes;

    use super::*;

    fn line(from: (f32, f32), to: (f32, f32)) -> Path {
        let mut builder = NoAttributes::wrap(Path::builder());
        builder.begin(Point::new(from.0, from.1));
        builder.line_to(Point::new(to.0, to.1));
        builder.end(false);
        builder.build()
    }

    #[test]
    fn a_path_is_followed_from_the_end_by_the_vertex() {
        let path = line((0.0, 0.0), (100.0, 0.0));
        let (start, reached) = reach_from(&path, Point::new(100.0, 0.0), 30.0).unwrap();
        assert_eq!(start.x, 100.0);
        assert!((reached.x - 70.0).abs() < 1e-3, "{reached:?}");
        let (start, reached) = reach_from(&path, Point::new(-5.0, 0.0), 30.0).unwrap();
        assert_eq!(start.x, 0.0);
        assert!((reached.x - 30.0).abs() < 1e-3, "{reached:?}");
        // Shorter than the distance: all the way to the other end.
        let (_, reached) = reach_from(&path, Point::new(0.0, 0.0), 300.0).unwrap();
        assert_eq!(reached.x, 100.0);
        // No length, no reach.
        assert!(reach_from(&line((1.0, 1.0), (1.0, 1.0)), Point::new(1.0, 1.0), 5.0).is_none());
    }

    #[test]
    fn a_path_is_followed_round_its_bends() {
        let mut builder = NoAttributes::wrap(Path::builder());
        builder.begin(Point::new(0.0, 0.0));
        builder.line_to(Point::new(10.0, 0.0));
        builder.line_to(Point::new(10.0, 10.0));
        builder.end(false);
        let (_, reached) = reach_from(&builder.build(), Point::new(0.0, 0.0), 15.0).unwrap();
        assert!(
            (reached - Point::new(10.0, 5.0)).length() < 1e-4,
            "{reached:?}"
        );
    }

    #[test]
    fn echoes_are_on_the_up_and_right_side_of_a_wire() {
        for (direction, expected) in [
            ((0.0_f32, 1.0_f32), (1.0_f32, 0.0_f32)),
            ((0.0, -1.0), (1.0, 0.0)),
            ((1.0, 0.0), (0.0, -1.0)),
            ((-1.0, 0.0), (0.0, -1.0)),
        ] {
            let side = echo_side(Vector2D::new(direction.0, direction.1) * 7.0_f32).unwrap();
            assert!(
                (side - Vector2D::new(expected.0, expected.1)).length() < 1e-5,
                "{direction:?}"
            );
        }
        // Whichever way along the wire, the same side, and at right angles to it.
        let d = Vector2D::new(3.0, -2.0);
        assert_eq!(echo_side(d).unwrap(), echo_side(-d).unwrap());
        assert!(echo_side(d).unwrap().dot(d).abs() < 1e-4);
        assert!(echo_side(Vector2D::zero()).is_none());
    }

    fn numbers(d: &str) -> Vec<f32> {
        d.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter_map(|number| number.parse().ok())
            .collect()
    }

    #[test]
    fn a_fan_has_a_page_for_each_sheet_up_to_four() {
        let pages = |n| {
            fan(Point::new(10.0, 10.0), n, 9.0)
                .rims
                .matches('M')
                .count()
        };
        assert_eq!(
            (pages(0), pages(1), pages(3), pages(4), pages(18)),
            (0, 1, 3, 4, 4)
        );
        assert!(fan(Point::new(0.0, 0.0), 0, 9.0).undersides.is_empty());
    }

    /// The pages are all tangent at the bottom, so that they are one curve there, and each is
    /// higher than the one before at the top, so that the thickness shows above.
    #[test]
    fn the_pages_of_a_fan_meet_at_the_bottom_and_part_at_the_top() {
        let (centre, radius) = (Point::new(50.0, 50.0), 9.0);
        let fan = fan(centre, 4, radius);
        // Each page is `M left cy A rx ry 0 0 1 right cy`: its ends are at the height of its
        // centre, which rises, and it has a radius in y that grows by as much again.
        let pages: Vec<_> = fan.rims.split('M').skip(1).map(numbers).collect();
        assert_eq!(pages.len(), 4);
        let (mut bottoms, mut tops) = (vec![], vec![]);
        for page in &pages {
            let (left, cy, rx, ry, right) = (page[0], page[1], page[2], page[3], page[7]);
            assert!(((left + right) / 2.0 - centre.x).abs() < 1e-2, "centred");
            assert!((right - left - 2.0 * rx).abs() < 2e-2, "{page:?}");
            assert!(ry < rx, "flat: {page:?}");
            bottoms.push(cy + ry);
            tops.push(cy - ry);
        }
        for bottom in &bottoms {
            assert!((bottom - bottoms[0]).abs() < 2e-2, "{bottoms:?}");
        }
        for pair in tops.windows(2) {
            assert!(pair[1] < pair[0] - 1.0, "each page is higher: {tops:?}");
        }
        // The first page is about the vertex: its centre is not above it.
        assert!((pages[0][1] - centre.y).abs() < 1e-2);
        // The underside is the same ellipses, the other way round.
        assert_eq!(fan.undersides.matches("0 0 0").count(), 4);
        assert_eq!(fan.rims.matches("0 0 1").count(), 4);
    }
}
