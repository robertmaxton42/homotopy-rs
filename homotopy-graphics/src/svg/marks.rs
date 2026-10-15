//! The geometry of the marks that show where sheets meet in the 2D view.
//!
//! See `Depths::echoes` and `Depths::sheets_meeting_at`: the fan of pages at a point where sheets
//! meet, and the echoes of the wires behind, which run along the wire and fade out.
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

/// How far each page of a fan reaches either side of its direction, in degrees, and how much
/// further out each is than the one before (as a part of the radius of the first).
const FAN_SPAN: f32 = 38.0;
const PAGE_GAP: f32 = 0.34;

/// Where the middle of a fan points, in degrees (y is down: up and to the right). The echoes are
/// on the same side of their wires.
const FAN_DIRECTION: f32 = -45.0;

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

/// The path data of a fan of `pages` pages (at most [`MAX_PAGES`]) at `centre`.
///
/// They are thin arcs about it, on the side that is up and to the right, the first at `radius` from it and each next a
/// little further out, like the edge of a stack of pages. It is a still picture, with no curl or
/// arrow to it.
#[must_use]
pub fn fan(centre: Point, pages: usize, radius: f32) -> String {
    let pages = pages.min(MAX_PAGES);
    let mut d = String::new();
    for page in 0..pages {
        let r = radius * (1.0 + PAGE_GAP * page as f32);
        let at = |degrees: f32| {
            let angle = degrees.to_radians();
            Point::new(centre.x + r * angle.cos(), centre.y + r * angle.sin())
        };
        let (from, to) = (at(FAN_DIRECTION - FAN_SPAN), at(FAN_DIRECTION + FAN_SPAN));
        write!(
            d,
            "M{:.2} {:.2}A{r:.2} {r:.2} 0 0 1 {:.2} {:.2}",
            from.x, from.y, to.x, to.y
        )
        .ok();
    }
    d
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

    #[test]
    fn a_fan_has_a_page_for_each_sheet_up_to_four() {
        let pages = |n| fan(Point::new(10.0, 10.0), n, 12.0).matches('M').count();
        assert_eq!(
            (pages(0), pages(1), pages(3), pages(4), pages(18)),
            (0, 1, 3, 4, 4)
        );
        // The pages are within the radius of the last of the centre, and not within the first's.
        let d = fan(Point::new(0.0, 0.0), 4, 10.0);
        for number in d.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')) {
            if let Ok(v) = number.parse::<f32>() {
                assert!(v.abs() <= 10.0 * (1.0 + 3.0 * PAGE_GAP) + 1e-3, "{d}");
            }
        }
    }
}
