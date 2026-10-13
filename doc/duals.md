# Duals, reflections and symmetries

This note explains how homotopy.io handles dual, adjoint and inverse cells, what a
`k`-dualisable generator is, and how to get dual cells and dual theorems (such as
`F^op : C^op ⇒ D^op`) without adding them to the signature by hand.

## Directions

An `n`-diagram has `n` directions. We number them **from the inside**: `x₁` is the
innermost direction (left to right in a 2D picture) and `xₙ` the outermost one (the
direction in which the diagram's top-dimensional cells compose, bottom to top in 2D).

A `k`-cell in an `n`-diagram is a stratum of codimension `k`. It extends along the outer
directions `xₖ₊₁, …, xₙ` and sits at a point of its *normal space* `x₁, …, xₖ`. For example,
in a 2D diagram a 1-cell `C` is a vertical wire (normal direction `x₁`) and a 2-cell
`F : C ⇒ D` is a vertex (normal directions `x₁, x₂`).

Each occurrence of a generator carries an orientation:

- `+` or `−`: the parity of the number of its normal directions that are reflected compared
  to the generator's own diagram;
- `0`: a *fold*, where the cell turns back on itself, such as the turning point of a cup or
  cap.

## One dualisation operator per direction

Write `Rⱼ` for reflection in direction `xⱼ`. It acts on every cell of dimension `≥ j`
simultaneously: a `k`-cell with `k ≥ j` is reflected in one of its normal directions, so its
orientation flips. Cells of dimension `< j` merely extend along `xⱼ` and are unaffected. The
operators are involutions and commute with each other, so together they form a group
`(ℤ/2)ⁿ`. A `k`-cell only sees the restriction of a reflection to `x₁, …, xₖ`.

For `F : C ⇒ D` between 1-cells and `η : F ⇛ G` between such 2-cells:

| reflection | `C` | `F` | `η` | reading |
|---|---|---|---|---|
| `R₁` | `C*` | `C* ⇒ D*` | `F* ⇛ G*` | dual in the first direction |
| `R₂` | `C` | `D ⇒ C` | `F̄ ⇛ Ḡ` | reverse (inverse/adjoint) of `F` |
| `R₃` | `C` | `F` | `G ⇛ F` | reverse of `η` |
| `R₁R₂` | `C*` | `D* ⇒ C*` | | rotation by 180°: the mate of `F` |
| `R₁R₃` | `C*` | `C* ⇒ D*` | `G* ⇛ F*` | `op` on **Cat**, which reverses 2-cells |

Reflections in an even number of a cell's normal directions are rotations, and leave the
cell positively oriented. Reflections in an odd number reverse its orientation.

In the app, the attach menu offers each reflection of a generator that typechecks, labelled
by the reflected directions (e.g. *dual in x₁, x₂*) or by the name of the symmetry that
gives it (see below). Its inverse `Rₙ` is labelled *inverse*. **Theorems are generators
too**, so a theorem's dual versions are offered in the same way, without adding anything to
the signature. The *Invert* action (`v`) reflects the whole workspace in its outermost
direction.

## What typechecks

Every generator has one of three levels of invertibility.

**Invertible.** Any reflection and any fold is allowed, and so is any singular
behaviour of the cell's stratum. This is a weak equivalence.

**Dualisable(k)** (shown as `k + 1` in the settings). Any reflection and any fold is
allowed. But in diagrams of dimension greater than `dim g + k + 1`, the stratum of `g` must
be a manifold near each of its points. Concretely, `g` admits `k + 1` levels of
creation and annihilation, and only manifold (isotopy) moves above that:

| a 1-cell `C` that is… | cup/cap (2D) | bubble² (3D) | bubble³ (4D) | bubble⁴ (5D) |
|---|---|---|---|---|
| directed | ✗ | ✗ | ✗ | ✗ |
| `Dualisable(0)` | ✓ | ✗ | ✗ | ✗ |
| `Dualisable(1)` | ✓ | ✓ | ✗ | ✗ |
| `Dualisable(2)` | ✓ | ✓ | ✓ | ✗ |
| invertible | ✓ | ✓ | ✓ | ✓ |

Here bubbleⁱ is the `i`-fold iterated bubble (create a cell together with its reverse) of
`C`. So `Dualisable(k)` means **`(k + 1)`-times dualisable**: `C` has a dual, the unit
and counit of that duality have adjoints, their units and counits have adjoints, and so on,
`k + 1` levels deep. For example, creating or removing a circle of `C` (bubble²) is a
further adjunction one level up, so it needs `k ≥ 1`. Beyond these levels, an invertible cell
can still undergo non-manifold moves, but a dualisable one cannot. (This table is pinned
down by the test `dualisable_k_allows_k_plus_one_levels_of_bubbles` in
`homotopy-core/tests/reflect.rs`.)

**Directed.** Only positive orientation is allowed. That rules out the inverse and any other
orientation-reversing reflection, but allows *rotations*. Note, though, that a rotation of
`g` reverses some of the cells in `g`'s boundary, and those must be allowed to appear
reversed in their own right. So the mate `D* ⇒ C*` of a directed `F : C ⇒ D` typechecks
exactly when `C` and `D` are invertible or dualisable. This matches the mathematics: mates
are built from the cups and caps of `C` and `D`.

## Symmetries: duals for free

Some theories have orientation-reversing duals of *directed* cells for free. In **Cat**,
every functor `F : C → D` has `F^op : C^op → D^op`, although `F` has no inverse, and `C` is
not dualisable in the cartesian monoidal structure. Declaring a **symmetry** asserts that the
whole theory is invariant under a reflection, so these cells need no separate generators.

A symmetry is a set `S` of directions, such as `op = {1}`. A directed `k`-cell may then
appear reflected by `S` restricted to `x₁, …, xₖ`, composed with any rotation that is allowed
anyway, i.e. one that only reverses invertible or dualisable boundary cells. With 1-cells as
categories, 2-cells as functors and 3-cells as natural transformations:

- `op = {1}` gives `F^op : C^op ⇒ D^op` and `η^op : F^op ⇛ G^op` (covariant on 2-cells).
- `op = {1, 3}` gives `F^op : C^op ⇒ D^op` and `η^op : G^op ⇛ F^op`. This is the `op` of
  **Cat**, a 2-functor `Cat → Cat^co`.
- Neither gives `D ⇒ C`, or the mate `D^op ⇒ C^op` when `C` and `D` are merely symmetric.

Declaring a symmetry is an **axiom about your theory**. It asserts that every generator,
including every axiom you add, has a reflected counterpart. It is sound exactly when the
models you have in mind carry that symmetry. Also, a symmetry in direction `j` and a dual in
direction `j` produce the same thing (the cell reflected in `xⱼ`). So in a theory where `C`
is dualisable and `{1}` is a symmetry, `C^op` *is* `C*`. That is right for profunctors,
where `C^op` is the dual of `C`, but it is an extra identification in general.

Symmetries are declared in the signature drawer (marked experimental). Removing a symmetry
is refused if a generator or the workspace would then fail to typecheck.

## Internal homs

In a compact closed (rigid) monoidal category, the internal hom is `[A, B] ≅ A* ⊗ B`, so duals
make it structural:

- **Currying** `f : A ⊗ X → B` to `X → A* ⊗ B` is bending the `A` wire round with a cup.
- **Evaluation** `A ⊗ A* ⊗ B → B` is a cap.
- **Names and mates** of morphisms are rotations, which the attach menu offers.

To set this up, use one 0-cell, objects as wires and morphisms as vertices, and make the
objects **Dualisable** (level 1 is enough). That gives cups and caps in 2D, and the snake
equations as allowed moves in 3D. Examples are Rel, finite-dimensional vector spaces,
profunctors and cobordisms.

In a closed category that is not compact, such as Set or **Cat**, `[A, B]` is not
`A* ⊗ B`. Making `A` dualisable would assert structure that isn't there. homotopy.io's duals
are also two-sided, which is stronger than one adjunction `- ⊗ A ⊣ [A, -]`. In that case:

- state currying as a schema whose currying cell depends on the morphism being curried (see
  `doc/placeholders.md`, "Cells that depend on placeholders they don't mention"), or
- work one level up, with the closed structure as an ordinary adjunction of functors given by
  its unit, counit and snake axioms.

## Caveats

- Typechecking is local: it checks the neighbourhood of every point of a diagram against
  the generator it is labelled with, up to the allowed reflections. For invertible and
  dualisable generators, these neighbourhoods are compared without orientations.
- Attach options are checked when the menu is built. Replayed action logs are trusted.
- Dualisable generators are experimental. The manifold check can be slow in high dimensions.
