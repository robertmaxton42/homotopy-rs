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

The conventions are in `homotopy-model/src/proof/structure.rs`. Tooltips over the diagram are
plain text, which cannot show formulas, so they still say *(inverse)* and *(homotopy)*.
