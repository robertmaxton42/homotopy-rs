//! The sheets at a position of the 2-dimensional projection of a diagram (`Depths::sheets`):
//! the singular heights of what the position carries, checked against slicing the diagram
//! directly.

use homotopy_core::{
    examples, projection::Depths, Boundary, Diagram, DiagramN, Height, SliceIndex,
};

/// All the coordinates of the first `n` dimensions of a diagram, with what is left of it there.
fn positions(diagram: &Diagram, n: usize) -> Vec<(Vec<SliceIndex>, Diagram)> {
    if n == 0 {
        return vec![(vec![], diagram.clone())];
    }
    let Diagram::DiagramN(d) = diagram else {
        return vec![];
    };
    let mut all = vec![];
    let size = d.size();
    let indices = [SliceIndex::Boundary(Boundary::Source)]
        .into_iter()
        .chain((0..2 * size + 1).map(|i| SliceIndex::Interior(Height::from(i))))
        .chain([SliceIndex::Boundary(Boundary::Target)]);
    for index in indices {
        let slice = d.slice(index).unwrap();
        for (mut rest, fibre) in positions(&slice, n - 1) {
            rest.insert(0, index);
            all.push((rest, fibre));
        }
    }
    all
}

fn check<const N: usize>(diagram: &DiagramN) -> (usize, usize) {
    let diagram: Diagram = diagram.clone().into();
    let depths = Depths::<N>::new(&diagram).unwrap();
    let (mut positions_with_sheets, mut with_several) = (0, 0);
    for (coord, fibre) in positions(&diagram, N) {
        let coord: [SliceIndex; N] = coord.try_into().unwrap();
        assert_eq!(depths.fibre(coord), Some(&fibre), "{coord:?}");
        let sheets = depths.sheets(coord, &[]);
        let expected = match &fibre {
            Diagram::Diagram0(_) => vec![],
            Diagram::DiagramN(f) => f
                .singular_slices()
                .map(|slice| slice.max_generator())
                .collect(),
        };
        assert_eq!(
            sheets.iter().map(|s| s.generator).collect::<Vec<_>>(),
            expected,
            "{coord:?}"
        );
        for (i, sheet) in sheets.iter().enumerate() {
            assert_eq!(sheet.z, i);
        }
        // The front sheet is the one that the depth analysis picks, if there is one.
        let fronts: Vec<_> = sheets.iter().filter(|s| s.front).collect();
        match depths.node_depth(coord) {
            Some(z) if z < sheets.len() => assert_eq!(fronts.len(), 1, "{coord:?}"),
            _ => assert!(fronts.is_empty(), "{coord:?}"),
        }
        if let Some(front) = fronts.first() {
            assert_eq!(Some(front.z), depths.node_depth(coord));
        }
        // Nested: the sheets of a sheet are those of its slice.
        for sheet in &sheets {
            let Diagram::DiagramN(f) = &fibre else {
                unreachable!()
            };
            let inner = f.slice(Height::Singular(sheet.z)).unwrap();
            let nested = depths.sheets(coord, &[sheet.z]);
            match inner {
                Diagram::Diagram0(_) => assert!(nested.is_empty()),
                Diagram::DiagramN(inner) => {
                    assert_eq!(nested.len(), inner.size(), "{coord:?} within {}", sheet.z);
                    assert!(nested.iter().all(|s| !s.front));
                }
            }
        }
        // Nothing there: no position, or sheets beyond the diagram.
        assert!(depths.sheets(coord, &[usize::MAX / 4]).is_empty());
        positions_with_sheets += usize::from(!sheets.is_empty());
        with_several += usize::from(sheets.len() > 1);
    }
    (positions_with_sheets, with_several)
}

#[test]
fn sheets_are_the_singular_heights_of_the_fibre() {
    let mut several = 0;
    let mut checked = 0;
    for (_, diagram) in [
        examples::associator(),
        examples::crossing(),
        examples::half_braid(),
        examples::bubble(),
        examples::algebraic_snake(),
        examples::lips(),
        examples::pants_unit(),
        examples::two_cups(),
    ] {
        let dimension = diagram.dimension();
        if dimension >= 2 {
            let (with, more) = check::<1>(&diagram);
            checked += with;
            several += more;
        }
        if dimension >= 3 {
            let (with, more) = check::<2>(&diagram);
            checked += with;
            several += more;
        }
    }
    assert!(checked > 0, "some positions have sheets");
    assert!(several > 0, "some positions have several sheets");
}
