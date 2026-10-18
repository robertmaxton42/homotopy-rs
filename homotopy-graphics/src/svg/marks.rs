//! The geometry of the marks that show where sheets meet in the 2D view.
//!
//! See `Depths::meeting_pages` and `Depths::echo_sheet`: the stack of pages at a point where
//! sheets meet (one for each sheet), and the echoes of the wires that go into the sheets behind,
//! which begin at the page of their sheet, run along the wire and fade out.
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

/// One page of a fan, as path data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageMarks {
    /// The upper edge: an arc from one end of the page's width over the top to the other.
    pub rim: String,
    /// The lower edge, which nearly coincides with those of the other pages.
    pub underside: String,
    /// The whole outline, closed: what the page covers.
    pub outline: String,
}

/// The ellipse of the page of this `rank` (0 is the bottom one, the first and smallest) of a fan
/// at `centre`, a vertex of radius `radius`: its centre and radii.
fn ellipse(centre: Point, rank: usize, radius: f32) -> (Point, f32, f32) {
    let (width, height) = (PAGE_WIDTH * radius, PAGE_HEIGHT * radius);
    let grow = PAGE_RISE * radius;
    let ratio = width / height;
    let ry = height + grow * rank as f32;
    let rx = width + grow * ratio.sqrt() * rank as f32;
    // Tangent at the bottom, which is the same for every page.
    (Point::new(centre.x, centre.y + height - ry), rx, ry)
}

/// The page of this `rank` of a fan at `centre`: see [`fan`].
#[must_use]
pub fn page(centre: Point, rank: usize, radius: f32) -> PageMarks {
    let (middle, rx, ry) = ellipse(centre, rank, radius);
    let (left, right, cy) = (middle.x - rx, middle.x + rx, middle.y);
    PageMarks {
        rim: format!("M{left:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 1 {right:.2} {cy:.2}"),
        underside: format!("M{left:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 0 {right:.2} {cy:.2}"),
        outline: format!(
            "M{left:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 1 {right:.2} {cy:.2}A{rx:.2} {ry:.2} 0 0 1 {left:.2} {cy:.2}Z"
        ),
    }
}

/// The stretch of a wire from where it leaves a page of a fan, as points.
///
/// The page is the one of this `rank`, and the stretch goes `distance` further on (the first point
/// is where it leaves, the last where it has gone as far). `None` if it does not leave the page, or
/// has no length.
///
/// The wire is its polyline (see [`polyline`]), followed from the end that is nearer to the
/// vertex at `centre`, which is inside every page. If the wire ends before `distance` the stretch
/// goes to its end.
#[must_use]
pub fn leave_page(
    points: &[Point],
    centre: Point,
    rank: usize,
    radius: f32,
    distance: f32,
) -> Option<Vec<Point>> {
    let (first, last) = (*points.first()?, *points.last()?);
    let mut points = points.to_vec();
    if (first - centre).square_length() > (last - centre).square_length() {
        points.reverse();
    }
    let (middle, rx, ry) = ellipse(centre, rank, radius);
    let outside = |p: Point| {
        let (dx, dy) = ((p.x - middle.x) / rx, (p.y - middle.y) / ry);
        dx * dx + dy * dy > 1.0
    };
    // The first segment that ends outside the page, and where it crosses.
    let mut at = *points.first()?;
    let mut crossing = None;
    for (i, &next) in points.iter().enumerate().skip(1) {
        if outside(next) {
            if outside(at) {
                // It started outside: from here.
                crossing = Some((i - 1, at));
            } else {
                let (mut inside_end, mut outside_end) = (at, next);
                for _ in 0..24 {
                    let mid = inside_end.lerp(outside_end, 0.5);
                    if outside(mid) {
                        outside_end = mid;
                    } else {
                        inside_end = mid;
                    }
                }
                crossing = Some((i - 1, outside_end));
            }
            break;
        }
        at = next;
    }
    let (index, from) = crossing?;
    let mut stretch = vec![from];
    let mut left = distance;
    let mut at = from;
    for &next in &points[index + 1..] {
        let length = (next - at).length();
        if length >= left && length > 0.0 {
            stretch.push(at.lerp(next, left / length));
            return Some(stretch);
        }
        left -= length;
        at = next;
        stretch.push(at);
    }
    (stretch.len() > 1).then_some(stretch)
}

/// Path data for a polyline: `M x y L x y ...` (empty for no points).
#[must_use]
pub fn polyline_path(points: &[Point]) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let command = if i == 0 { 'M' } else { 'L' };
        write!(d, "{command}{:.2} {:.2}", p.x, p.y).ok();
    }
    d
}

/// A fan of `pages` pages (at most [`MAX_PAGES`]) at `centre`, a vertex of radius `radius`.
///
/// It is a stack of discs seen from a little above, one for each sheet that meets there: flat
/// ellipses that are each taller and wider than the one before and are all tangent at the bottom,
/// so that their centres rise from the vertex and their lower edges are one curve, and the
/// thickness of the stack shows only on its upper side, as the rims of the pages. A still picture
/// about the vertex, with no backing (the vertex is drawn over its middle). The first page, the
/// smallest, is the one at the back; each next one is in front of the one before.
#[must_use]
pub fn fan(centre: Point, pages: usize, radius: f32) -> Fan {
    let mut fan = Fan::default();
    for rank in 0..pages.min(MAX_PAGES) {
        let page = page(centre, rank, radius);
        fan.rims.push_str(&page.rim);
        fan.undersides.push_str(&page.underside);
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

    #[test]
    fn a_fan_is_its_pages() {
        let (centre, radius) = (Point::new(20.0, 30.0), 6.0);
        let whole = fan(centre, 3, radius);
        let rims: String = (0..3).map(|rank| page(centre, rank, radius).rim).collect();
        let unders: String = (0..3)
            .map(|rank| page(centre, rank, radius).underside)
            .collect();
        assert_eq!((whole.rims, whole.undersides), (rims, unders));
        // The outline is closed, and is the rim and its other half.
        let one = page(centre, 2, radius);
        assert!(one.outline.ends_with('Z'));
        assert_eq!(one.outline.matches('A').count(), 2);
    }

    /// A wire that goes straight down from the vertex leaves every page at its bottom, which is
    /// the same for all of them; one that goes straight up leaves each at its top, and the top of
    /// each next is higher. The reach is as far again along the wire. One that stays inside does not
    /// leave.
    #[test]
    fn a_wire_leaves_a_page_at_its_outline() {
        let (centre, radius) = (Point::new(50.0, 50.0), 7.0);
        let down = [centre, Point::new(50.0, 250.0)];
        let up = [centre, Point::new(50.0, -150.0)];
        let mut tops = vec![];
        for rank in 0..4 {
            let stretch = leave_page(&down, centre, rank, radius, 36.0).unwrap();
            let (from, to) = (stretch[0], *stretch.last().unwrap());
            assert!(
                (from.y - (50.0 + PAGE_HEIGHT * radius)).abs() < 0.05,
                "{from:?}"
            );
            assert!((to.y - from.y - 36.0).abs() < 0.05);
            let stretch = leave_page(&up, centre, rank, radius, 36.0).unwrap();
            let (from, to) = (stretch[0], *stretch.last().unwrap());
            assert!((from.y - to.y - 36.0).abs() < 0.05);
            tops.push(from.y);
        }
        assert!(tops.windows(2).all(|w| w[1] < w[0] - 1.0), "{tops:?}");
        // A wire given from its far end is followed from the end by the vertex.
        let backwards = [Point::new(50.0, 250.0), centre];
        let stretch = leave_page(&backwards, centre, 1, radius, 36.0).unwrap();
        assert!((stretch[0].y - (50.0 + PAGE_HEIGHT * radius)).abs() < 0.05);
        // The stretch follows the bends, and is cut where the distance runs out.
        let bent = [
            centre,
            Point::new(50.0, 60.0),
            Point::new(90.0, 60.0),
            Point::new(90.0, 160.0),
        ];
        let stretch = leave_page(&bent, centre, 0, radius, 20.0).unwrap();
        let length: f32 = stretch.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        assert!((length - 20.0).abs() < 0.05, "{length}");
        assert_eq!(
            polyline_path(&stretch).matches('L').count(),
            stretch.len() - 1
        );
        assert!(polyline_path(&[]).is_empty());
        // Inside the page throughout.
        let short = [centre, Point::new(51.0, 51.0)];
        assert!(leave_page(&short, centre, 3, radius, 36.0).is_none());
        assert!(leave_page(&[], centre, 0, radius, 36.0).is_none());
    }
}
