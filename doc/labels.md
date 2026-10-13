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

The conventions are in `homotopy-model/src/proof/structure.rs`.

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
| a cell with other cells or wires beside it | the name of the cell, `f` (`f^{-1}` if it is the inverse of an invertible one, `f^{*}` for a 1-cell that is only dualisable, `\overline{f}` for other cells that are drawn reversed) |
| an exchange of two cells | `\mathrm{ex}_{f,g}`, with `f` the cell that is lowest in the source |
| a braid | `\beta_{f,g}`, or `\beta^{-1}_{f,g}` for passing on the other side |
| a cell cancelled with its reverse | the counit `\varepsilon_{f}` for `f ; f^{*}` becoming nothing, and `\varepsilon_{f^{*}}` for `f^{*} ; f` |
| a cell created with its reverse | the unit `\eta_{f}` for `f^{*} ; f` created out of nothing, and `\eta_{f^{*}}` for `f ; f^{*}` |
| any other homotopy | `\mathrm{h}` |
| anything else | its type, `s \Rightarrow t` (`\to` for a 1-cell, `\Rrightarrow` in higher dimensions), with its source and target described the same way, to a small depth, and `\ldots` below that |

An exchange is a braid when the cells are attached to nothing, so that nothing but the
exchange says on which side of each other they are (such as two scalars, `id ⇒ id` on one
0-cell); there are two ways for such cells to pass, `\beta` and `\beta^{-1}`. If either cell is
attached to wires, the exchange is `\mathrm{ex}`. The cups that the attach menu calls
`\mathrm{bub}_{f}` are creations of a cell with its reverse: `\mathrm{bub}_{f}` is the same cell
as `\eta_{f^{*}}`, and `\mathrm{bub}_{f^{-1}}` is `\eta_{f}`. The name that the menu uses is kept for
a cup that is exactly the bubble of a generator; beside other cells, it is the unit.

Descriptions are made when a panel is shown, so how much of a diagram they look at is bounded
(48 steps in all), whatever its size. A composite is described by its steps only if every step
can be told apart: one that has several different cells at the same height is described by the
type of the whole composite.
