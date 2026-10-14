//! Choosing a sheet: which of the cells hidden behind a position of the projected view the person
//! means.
//!
//! The 2D view of a diagram of more dimensions shows, at each position, the front-most of the
//! cells that its remaining dimensions have there (see [`Depths`]). The others, the *sheets* of
//! the position, are behind it. A click is at a position, and so says nothing of which sheet it
//! is for; a [`Choice`] adds that, as more coordinates of the point: the search for what to attach
//! (see [`AttachOptionsJob`](crate::proof::AttachOptionsJob)) then only looks at attachments that
//! reach that sheet, and not all that reach the position.
//!
//! Nothing changes unless a choice is made: the default is the front-most sheet, which is not
//! added to the point (a click with no choice made is the click it always was). A [`Stack`] is the
//! sheets to choose from at a place under the pointer, and what the person can do to it: step
//! to the next sheet, or *drill*: fix the sheet and choose the one inside it for the next
//! coordinate too.

use homotopy_core::{
    projection::{Depths, Sheet},
    Height, SliceIndex,
};

/// A sheet chosen at a position, and the sheets inside it for the coordinates after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice<const N: usize> {
    pub position: [SliceIndex; N],
    /// The singular heights chosen, one for each of the coordinates after the position's, as
    /// many as have been fixed: the first is a sheet of the position, the next a sheet inside
    /// that. Never empty.
    pub path: Vec<usize>,
}

impl<const N: usize> Choice<N> {
    /// The point that this is a choice of: the position and then the heights chosen.
    #[must_use]
    pub fn point(&self) -> Vec<SliceIndex> {
        let mut point = self.position.to_vec();
        point.extend(
            self.path
                .iter()
                .map(|&z| SliceIndex::Interior(Height::Singular(z))),
        );
        point
    }

    /// The choice without its innermost coordinate (nothing is chosen if there was only one).
    #[must_use]
    pub fn undrilled(&self) -> Option<Self> {
        let (_, outer) = self.path.split_last()?;
        (!outer.is_empty()).then(|| Self {
            position: self.position,
            path: outer.to_vec(),
        })
    }
}

/// A sheet that can be chosen, and the position it is a sheet of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<const N: usize> {
    pub position: [SliceIndex; N],
    pub sheet: Sheet,
}

/// The sheets to choose from at a place on the diagram, one coordinate's worth.
///
/// They are the sheets of each of the positions under the pointer (shapes can overlap), or, once a
/// sheet is fixed (see [`drill`](Self::drill)), the sheets inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack<const N: usize> {
    pub entries: Vec<Entry<N>>,
    /// The entry that is chosen, or that is the default.
    pub current: usize,
    /// Whether a choice was made, and the current entry is not just the default.
    pub explicit: bool,
    /// The sheets fixed outside of the ones that are listed.
    pub fixed: Vec<usize>,
}

impl<const N: usize> Stack<N> {
    /// The stack at a place that has these positions under it (the one on top first), given the
    /// choice made, if one was. `None` if there are no sheets: a diagram of the dimensions of the
    /// view has none. A choice that is not of one of these positions (or of a sheet they do not
    /// have) is as if none was made.
    #[must_use]
    pub fn new(
        depths: &Depths<N>,
        positions: &[[SliceIndex; N]],
        choice: Option<&Choice<N>>,
    ) -> Option<Self> {
        let outermost = |positions: &[[SliceIndex; N]]| -> Vec<Entry<N>> {
            let mut seen: Vec<[SliceIndex; N]> = vec![];
            let mut entries = vec![];
            for &position in positions {
                if seen.contains(&position) {
                    continue;
                }
                seen.push(position);
                entries.extend(
                    depths
                        .sheets(position, &[])
                        .into_iter()
                        .map(|sheet| Entry { position, sheet }),
                );
            }
            entries
        };

        if let Some(choice) = choice {
            if let Some((&last, fixed)) = choice.path.split_last() {
                let entries = if fixed.is_empty() {
                    outermost(positions)
                } else {
                    depths
                        .sheets(choice.position, fixed)
                        .into_iter()
                        .map(|sheet| Entry {
                            position: choice.position,
                            sheet,
                        })
                        .collect()
                };
                let current = entries
                    .iter()
                    .position(|e| e.position == choice.position && e.sheet.z == last);
                if let Some(current) = current {
                    return Some(Self {
                        entries,
                        current,
                        explicit: true,
                        fixed: fixed.to_vec(),
                    });
                }
            }
        }

        let entries = outermost(positions);
        let first = entries.first()?;
        // The front-most of the sheets of the position on top that has any.
        let current = entries
            .iter()
            .position(|e| e.position == first.position && e.sheet.front)
            .unwrap_or(0);
        Some(Self {
            entries,
            current,
            explicit: false,
            fixed: vec![],
        })
    }

    /// The entry that is current.
    #[must_use]
    pub fn entry(&self) -> &Entry<N> {
        &self.entries[self.current]
    }

    /// The sheets fixed, and the current one: the heights of the point, as they are chosen.
    #[must_use]
    pub fn path(&self) -> Vec<usize> {
        let mut path = self.fixed.clone();
        path.push(self.entry().sheet.z);
        path
    }

    /// The choice that is made by moving `steps` sheets on (back if negative), round to the
    /// other end of the stack when it runs out.
    #[must_use]
    pub fn step(&self, steps: isize) -> Choice<N> {
        let len = self.entries.len() as isize;
        let next = (self.current as isize + steps).rem_euclid(len) as usize;
        let entry = &self.entries[next];
        let mut path = self.fixed.clone();
        path.push(entry.sheet.z);
        Choice {
            position: entry.position,
            path,
        }
    }

    /// The choice that is made by fixing the current sheet and going one coordinate in. Fixing
    /// the default (nothing has been chosen) is the first step: it chooses that sheet, as it is.
    /// The next chooses the first of the sheets inside it. `None` if there are none (the
    /// diagram has no more coordinates there).
    #[must_use]
    pub fn drill(&self, depths: &Depths<N>) -> Option<Choice<N>> {
        let entry = self.entry();
        let path = self.path();
        if !self.explicit {
            return Some(Choice {
                position: entry.position,
                path,
            });
        }
        let inner = depths.sheets(entry.position, &path);
        let first = inner.first()?;
        let mut path = path;
        path.push(first.z);
        Some(Choice {
            position: entry.position,
            path,
        })
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{examples, Boundary, Diagram, DiagramN};

    use super::*;

    /// A position with several sheets, in a 3-diagram viewed in 2D.
    fn crowded() -> (Depths<2>, [SliceIndex; 2], usize) {
        let (_, associator) = examples::associator();
        let diagram: Diagram = associator.into();
        let depths = Depths::<2>::new(&diagram).unwrap();
        let d = DiagramN::try_from(diagram).unwrap();
        for i in 0..=2 * d.size() {
            let Some(Diagram::DiagramN(slice)) = d.slice(Height::from(i)) else {
                continue;
            };
            for j in 0..=2 * slice.size() {
                let position = [
                    SliceIndex::Interior(Height::from(i)),
                    SliceIndex::Interior(Height::from(j)),
                ];
                let sheets = depths.sheets(position, &[]);
                if sheets.len() > 2 {
                    return (depths, position, sheets.len());
                }
            }
        }
        panic!("no position with several sheets");
    }

    #[test]
    fn the_default_is_the_front_sheet_and_nothing_is_chosen() {
        let (depths, position, len) = crowded();
        let stack = Stack::new(&depths, &[position], None).unwrap();
        assert_eq!(stack.entries.len(), len);
        assert!(!stack.explicit && stack.fixed.is_empty());
        let front = depths.sheets(position, &[]);
        let expected = front.iter().position(|s| s.front).unwrap_or(0);
        assert_eq!(stack.current, expected);
    }

    #[test]
    fn stepping_goes_round_and_a_choice_is_a_longer_point() {
        let (depths, position, len) = crowded();
        let stack = Stack::new(&depths, &[position], None).unwrap();
        let mut choice = stack.step(1);
        assert_eq!(choice.position, position);
        assert_eq!(choice.path, vec![(stack.current + 1) % len]);
        assert_eq!(choice.point().len(), 3);
        assert_eq!(
            choice.point()[2],
            SliceIndex::Interior(Height::Singular(choice.path[0]))
        );
        // Round the whole stack is back where it started, in both directions.
        for direction in [1, -1] {
            let mut stack = stack.clone();
            let start = stack.current;
            for _ in 0..len {
                choice = stack.step(direction);
                stack = Stack::new(&depths, &[position], Some(&choice)).unwrap();
                assert!(stack.explicit);
            }
            assert_eq!(stack.current, start);
        }
    }

    #[test]
    fn drilling_fixes_the_sheet_and_then_goes_inside() {
        let (depths, position, _) = crowded();
        let stack = Stack::new(&depths, &[position], None).unwrap();
        // The first time only fixes what is the default.
        let fixed = stack.drill(&depths).unwrap();
        assert_eq!(fixed.path, vec![stack.entry().sheet.z]);
        let stack = Stack::new(&depths, &[position], Some(&fixed)).unwrap();
        assert!(stack.explicit);
        assert_eq!(
            stack.current,
            stack
                .entries
                .iter()
                .position(|e| e.sheet.z == fixed.path[0])
                .unwrap()
        );
        // The next goes inside it, if it has anything inside: an associator in 2D has one more
        // coordinate, which is a 0-diagram at each sheet, so there is nothing.
        let inner = depths.sheets(position, &fixed.path);
        assert_eq!(stack.drill(&depths).is_some(), !inner.is_empty());
        // Undrilling takes the last coordinate off, and the last one left is no choice.
        let deeper = Choice {
            position,
            path: vec![0, 1],
        };
        assert_eq!(deeper.undrilled().unwrap().path, vec![0]);
        assert_eq!(deeper.undrilled().unwrap().undrilled(), None);
    }

    /// In a 4-diagram the sheets have sheets inside them, which drilling chooses from.
    #[test]
    fn drilling_in_four_dimensions_chooses_inside_the_sheet() {
        let (_, lips) = examples::pants_unit();
        let diagram: Diagram = lips.into();
        let depths = Depths::<2>::new(&diagram).unwrap();
        let d = DiagramN::try_from(diagram).unwrap();
        let mut drilled = 0;
        for i in 0..=2 * d.size() {
            let Some(Diagram::DiagramN(slice)) = d.slice(Height::from(i)) else {
                continue;
            };
            for j in 0..=2 * slice.size() {
                let position = [
                    SliceIndex::Interior(Height::from(i)),
                    SliceIndex::Interior(Height::from(j)),
                ];
                let Some(stack) = Stack::new(&depths, &[position], None) else {
                    continue;
                };
                let fixed = stack.drill(&depths).unwrap();
                let stack = Stack::new(&depths, &[position], Some(&fixed)).unwrap();
                let Some(deeper) = stack.drill(&depths) else {
                    continue;
                };
                drilled += 1;
                assert_eq!(deeper.path.len(), 2);
                assert_eq!(deeper.path[0], fixed.path[0]);
                let inside = Stack::new(&depths, &[position], Some(&deeper)).unwrap();
                assert!(inside.explicit);
                assert_eq!(inside.fixed, fixed.path);
                assert_eq!(
                    inside.entries.len(),
                    depths.sheets(position, &fixed.path).len()
                );
                assert_eq!(inside.entry().sheet.z, deeper.path[1]);
                // Stepping stays inside the sheet that is fixed.
                let next = inside.step(1);
                assert_eq!(next.path[0], fixed.path[0]);
                assert_eq!(next.undrilled().unwrap(), fixed);
            }
        }
        assert!(drilled > 0, "some sheet has sheets inside");
    }

    #[test]
    fn a_choice_that_is_not_here_is_no_choice() {
        let (depths, position, _) = crowded();
        let elsewhere = [
            SliceIndex::Boundary(Boundary::Source),
            SliceIndex::Boundary(Boundary::Source),
        ];
        let choice = Choice {
            position: elsewhere,
            path: vec![0],
        };
        let stack = Stack::new(&depths, &[position], Some(&choice)).unwrap();
        assert!(!stack.explicit);
        let too_far = Choice {
            position,
            path: vec![1000],
        };
        assert!(
            !Stack::new(&depths, &[position], Some(&too_far))
                .unwrap()
                .explicit
        );
    }

    #[test]
    fn the_stack_of_overlapping_positions_has_the_sheets_of_each() {
        let (depths, position, len) = crowded();
        let other = [
            SliceIndex::Boundary(Boundary::Target),
            SliceIndex::Boundary(Boundary::Target),
        ];
        let others = depths.sheets(other, &[]).len();
        let stack = Stack::new(&depths, &[position, other, position], None).unwrap();
        assert_eq!(stack.entries.len(), len + others);
        assert!(stack.entries[..len].iter().all(|e| e.position == position));
        // Choosing a sheet of the second keeps the choice with that position.
        if others > 0 {
            let choice = Choice {
                position: other,
                path: vec![0],
            };
            let stack = Stack::new(&depths, &[position, other], Some(&choice)).unwrap();
            assert_eq!(stack.entry().position, other);
            assert!(stack.explicit);
        }
    }
}
