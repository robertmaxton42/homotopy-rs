# Placeholders and axiom schemas

A **placeholder** is a generator that stands for "any cell of this type". Axioms and theorems
that mention placeholders are **schemas**, such as `∀P. φ(P)`. They can be **instantiated** by
replacing each placeholder with an actual diagram of the same type, without leaving any trace
of the substitution in your proofs.

## Using them

1. Create a generator as usual, and turn on *Placeholder* in its settings. Placeholders are
   drawn in a style that ordinary generators can't have.
2. Use it like any other generator when building further generators. An axiom whose diagram
   mentions a placeholder is a schema. So is anything that depends on one: if `e : P ⇒ P` and
   `A : e ≫ e ⇛ e`, both `e` and `A` are schemas in `P`.
3. To instantiate a schema, open its settings. Put the diagram that should replace each
   placeholder in the workspace, click *Use workspace* in that placeholder's slot, then click
   *Create instance*.
4. The instance, e.g. `A[Q/P]`, is listed under its schema in the signature. The schema shows as
   a collapsed folder, so instances stay out of the way until you open it. An instance is an
   ordinary generator from then on: attach it, invert it, use it in proofs.

**Theorem schemas need no extra machinery.** A theorem whose statement mentions a placeholder
is itself a schema. To prove `∀P. φ(P)`, prove `φ(P)` with `P` a placeholder.

## What the kernel checks

- **Replacement types.** A placeholder `P : S ⇒ T` may be replaced by any diagram `X : σ(S) ⇒
  σ(T)`, where `σ(S)` is `S` with the same substitution applied. So placeholders can depend on
  other placeholders, e.g. `P : Q ⇒ Q` with `Q` a placeholder too.
- **Dependent generators.** A generator in the schema's boundary that mentions a replaced
  placeholder is instantiated along with it. In the example above, `A[X/P]` is built from
  `e[X/P]`.
- **Instances.** An instance is built from the schema's source and target with the
  substitution applied, and typechecked like any other generator. Asking for the same
  instance again reuses the existing one.
- **No retraction.** Nothing ever turns a diagram back into a placeholder. So instantiating
  never identifies two things that weren't already identified. This avoids the problem with
  the old workaround of an isomorphism `P ≅ X` for each `X`.

## How substitution works, and its limits

Substitution rebuilds a diagram one height at a time. Each height must attach a single
generator, possibly reflected. It is replayed with the replacement attached in its place, and
everything to the right of a replaced cell moves along as the cell grows or shrinks.

A height that does several things at once, such as `f ◫ g` with two cells side by side, is:

- **kept as it is**, if nothing in or around it is replaced, so the instance keeps the schema's
  layout where substitution doesn't care;
- **separated first**, if something in it is replaced. `f ◫ g` becomes `f ▷ _ ≫ _ ◁ g` (or the
  other order) by an expansion, which is an invertible homotopy, so the instance is still
  derivable from the schema. In that case the instance's boundary is drawn separated, even
  though the schema's was drawn contracted.

A height that cannot be separated this way is refused with an error. This happens when a
replaced cell is caught up in a genuinely higher-dimensional interchange, such as a braid-like
crossing in the boundary of a 4-cell or higher. Replacing a single cell there by a composite
would need new homotopies.

## Inferring instances when attaching

You usually don't need to instantiate by hand. When you click a point in the workspace, the
attach menu also offers instances of schemas whose placeholders can be filled in so that they
fit there. These are tagged with the substitution, e.g. `A [Q ≫ R/P]`. Choosing one creates the
instance and attaches it in one step, which is undone in one step too.

The search tries replacements made from the cells of the diagram you click on:

- for a 0-cell placeholder, its 0-cells;
- for a placeholder of dimension `k`, composites of up to three of its `k`-cells (and the
  inverses of invertible ones), including identities, with the right source and target.

Placeholders are solved from the lowest dimension up, so a placeholder's type is known when it
is solved. If a schema depends on another generator that mentions a placeholder, the instance
of that generator must already exist for the schema to match: it occurs in the diagram you
click on, after all. The search is bounded, so a very large diagram may not offer every
possibility. Instantiating by hand from the signature still works in that case.

## Not yet

- Inferring replacements longer than three cells, or replacements whose cells are contracted
  into one height (`f ◫ g`) in the diagram you click on.
