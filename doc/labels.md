# Labels

Names (of generators, symmetries and folders, and the project title) are labels. A label is
text with inline $\LaTeX$ between dollar signs, rendered with KaTeX: `$\alpha_1$ and $f$`.

## Dollar signs

Each label is read on its own, whatever is put around it:

- `$...$` is a formula. A `\` in it escapes the next character, so `$\$$` is a dollar sign.
- A `$` with no partner is a dollar sign: the label `f $x` shows `f $x`, and does not turn
  anything after it into a formula. So do `$$` and `\$` outside formulas: `\$5 and $x$`.
- Everything outside formulas is plain text. Markup in it, such as `<b>`, is shown as written.

Code that puts labels together (such as the names of instances, which contain the names of their
schemas) must `seal` each one first: `homotopy_common::tex::seal` escapes the dollar signs that
are not part of a formula, so that the result reads the same as the label did alone.

## Cells the app names

Generators are named by their authors, so nothing is assumed about their names. The cells that
the app derives from them get default names, as formulas with the conventional symbol, with the
name of the generator put in as it is (text or formula). They are shown in the attach menu in
place of the generator's name:

| cell | label for `f` |
|---|---|
| identity (weak unit) | `\mathrm{id}_{f}`, or `\mathrm{id}^{k}_{f}` if raised `k > 1` dimensions |
| identity on the inverse | `\mathrm{id}_{f^{-1}}` |
| inverse | `f^{-1}` |
| bubble (creates `f` with its reverse) | `\mathrm{bub}_{f}`, `\mathrm{bub}^{k}_{f}` for the `k`-fold one |
| bubble of the inverse | `\mathrm{bub}_{f^{-1}}` |
| dual in the first direction | `f^{*}` |
| reflection in directions `S` | `\mathrm{R}_{S}(f)`, e.g. `\mathrm{R}_{1,3}(f)` |
| reflection by the symmetry `σ` | `f^{σ}`, e.g. `f^{op}` |
| instance of a schema | `A [Q/P]` (the name, then the substitution) |

The cells are in `homotopy-model/src/proof/structure.rs`; how each is written (the symbols, and
the shape of the formula) is in `homotopy-model/src/proof/notation.rs`: see "Notation" below.

## Diagrams that are not generators

A diagram used as an argument, such as a replacement for a placeholder when instantiating a
schema, or the substitution in the name of an instance (`A [Q/P]`), is named by what it is made
of. A generator is named by its own name. Anything else gets a formula, from the first of these
that applies (the code is `homotopy-model/src/proof/describe.rs`; `f` and `g` stand for the names
of the cells, text or formula, put in as they are):

| diagram | label |
|---|---|
| a cell derived from a generator | the labels in the table above: `f^{-1}`, `f^{*}`, `\mathrm{R}_{1,3}(f)`, `f^{op}`, `\mathrm{bub}_{f}` |
| an identity on any diagram | `\mathrm{id}_{f \mathbin{;} g}`; `\mathrm{id}^{k}_{f}` for `k` identities on top of each other |
| a composite, from the source to the target of the diagram | `f \mathbin{;} g \mathbin{;} h` (longer ones are cut short with `\cdots`) |
| a cell with other cells or wires beside it | the name of the cell, `f` (`f^{-1}` if it is the inverse of an invertible one, `f^{*}` for a 1-cell that is only dualisable, `\overline{f}` for other cells that are drawn reversed), with what is beside it as whiskers, see below: `p \triangleright T`, `S \triangleleft p`, `S \triangleleft p \triangleright T` |
| an exchange of two cells, a braid | `\beta_{f,g}`, with `f` the cell that is lowest in the source; `\beta^{-1}_{f,g}` for passing on the other side, when that is known |
| a cell cancelled with its reverse | the counit `\varepsilon_{f}` for `f ; f^{*}` becoming nothing, and `\varepsilon_{f^{*}}` for `f^{*} ; f` |
| a cell created with its reverse | the unit `\eta_{f}` for `f^{*} ; f` created out of nothing, and `\eta_{f^{*}}` for `f ; f^{*}` |
| any other homotopy | `\mathrm{h}` |
| anything else | its type, `s \Rightarrow t` (`\to` for a 1-cell, `\Rrightarrow` in higher dimensions), with its source and target described the same way, to a small depth, and `\ldots` below that |

**Whiskers.** A cell of dimension `n` (2 or more) that has other diagrams beside it is named with
them, in the order of the diagram (the same as `;`, from the source to the target): a whisker that
comes after the cell is on its right, `p \triangleright T`, and one that comes before it is on its
left, `S \triangleleft p`; with both, `S \triangleleft p \triangleright T`. The triangle points at
the whisker. A whisker that is the identity is left out, and one that is a composite is put in
parentheses: `p \triangleright (F \mathbin{;} G)` (a long one is cut short like any composite). The
whiskers are diagrams of dimension `n - 1`: what comes before and after the cell in the top
direction of the slice of its step (for a 2-cell, the 1-cells on its two sides in the string
diagram; for a 5-cell, 4-diagrams, such as a 4-cell `T`). What is beside the cell in lower
directions, inside the step, is not part of the name. For example, in the tangent categories
project the replacement `Tp` for `\dot{\eta}` in `\zeta[Tp]` is `p \triangleright T`: `p` with `T` after
it. A step of a composite has its own whiskers: `p \triangleright b \mathbin{;} a \triangleleft q`.

An exchange is a braid when the cells are attached to nothing, so that nothing but the
exchange says on which side of each other they are (such as two scalars, `id ⇒ id` on one
0-cell); there are two ways for such cells to pass, `\beta` and `\beta^{-1}`. If either cell is
attached to wires, the exchange is `\beta` too, with no sign. The cups that the attach menu calls
`\mathrm{bub}_{f}` are creations of a cell with its reverse: `\mathrm{bub}_{f}` is the same cell
as `\eta_{f^{*}}`, and `\mathrm{bub}_{f^{-1}}` is `\eta_{f}`. The name that the menu uses is kept for
a cup that is exactly the bubble of a generator; beside other cells, it is the unit.

Descriptions are made when a panel is shown, so how much of a diagram they look at is bounded
(48 steps in all), whatever its size. A composite is described by its steps only if every step
can be told apart: one that has several different cells at the same height is described by the
type of the whole composite.

## Tooltips

Hovering over the diagram shows a tooltip with the label of the cell under the pointer, rendered
by KaTeX like the labels everywhere else, and in a muted type where in the diagram it is (such as
`@ S0, S1`: the heights from the outermost direction, regular `R`, singular `S`, or the source `S`
and target `T`). The tooltip follows the pointer and never takes it, and goes away when the pointer
leaves the diagram or a button is pressed.

The label of a cell that is drawn as the generator `f` is `f`. The cell at a vertex of a 2-diagram
(the dot where a 2-cell is, which is what the pointer is over) has its whiskers like any other
cell: `S \triangleleft p \triangleright T`. A wire or a surface is a cell by itself, with nothing
beside it, so it shows no whiskers; neither does a vertex of a projection of a diagram of
higher dimension (the cell there has no single place beside others). The reverse of an invertible `f` is
`f^{-1}`; of a generator that is only dualisable, `f^{*}` (for a 1-cell, where this is the dual in
the only direction there is) or `\overline{f}` (other cells, which may have been reflected in
several directions; the tooltip only knows that the cell is drawn reversed). A wire or surface
always shows its own cell.

A vertex where no cell of the dimension of the diagram is, but things happen at once, is named by
the step of the diagram that it belongs to, as in the table above, also when the diagram is
projected from higher dimensions: where two strands cross, `\beta_{f,g}`
(`\beta^{-1}_{f,g}` for the other way of passing, where that is known), and where a cell is cancelled or
created, the counit `\varepsilon_{f}` or the unit `\eta_{f}`. A homotopy that is not recognised is
`\mathrm{h}(f)` if the vertex is drawn as a fold, and the cell as drawn otherwise.

## Notation

Which symbol is used, and how each formula is put together, is decided in one place,
`homotopy-model/src/proof/notation.rs`: the `Notation` trait has a method for each (identity,
inverse, dual, bubble, reflection, symmetry, composition and its cut-off, grouping, left and right
whiskering, exchange, braid, unit and counit, homotopy, types) with the conventional notation
as its default, and `Standard` is that. The code that makes names decides what is said (that this
cell is `p` whiskered on the right by `T`) and the notation how it is written, so another
convention (for example, a notation in which whiskering is written differently) is another
implementation of the trait that overrides some of the methods, and `notation::current` says which is
in use. The tables above are those of `Standard`.
