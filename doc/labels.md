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
