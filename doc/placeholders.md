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
   `A : e ; e ⇛ e`, both `e` and `A` are schemas in `P`.
3. To instantiate a schema, open its settings. Put the diagram that should replace each
   placeholder in the workspace, click *Use workspace* in that placeholder's slot, then click
   *Create instance*. Replacements that follow from the ones you choose are filled in for you:
   a cone `α₀ : X₀ ∘ ! ⇒ D₀` can only be a cone over `D₀` with apex `X₀`, so choosing it for `α`
   also fills `X` and `D`. They are shown dimmed and marked *inferred*. Choose another diagram
   for one to override it, or clear it to leave it empty (it stays empty until what it was
   inferred from changes). Nothing you chose yourself is ever replaced.
4. The instance, e.g. `A[Q/P]`, is listed under its schema in the signature. The schema shows as
   a collapsed folder, so instances stay out of the way until you open it. An instance is an
   ordinary generator from then on: attach it, invert it, use it in proofs.

To attach an instance of a particular schema at a point without having every schema searched
for, drag the schema from the signature onto the point, or pick it and click there (see
[attaching a specific generator](attaching.md)).

**Theorem schemas need no extra machinery.** A theorem whose statement mentions a placeholder
is itself a schema. To prove `∀P. φ(P)`, prove `φ(P)` with `P` a placeholder.

## What the kernel checks

- **Replacement types.** A placeholder `P : S ⇒ T` may be replaced by any diagram `X : σ(S) ⇒
  σ(T)`, where `σ(S)` is `S` with the same substitution applied. So placeholders can depend on
  other placeholders, e.g. `P : Q ⇒ Q` with `Q` a placeholder too.
- **Invertible and dualisable placeholders.** A replacement is as invertible as the placeholder
  (see the next section).
- **Dependent generators.** A generator in the schema's boundary that mentions a replaced
  placeholder is instantiated along with it. In the example above, `A[X/P]` is built from
  `e[X/P]`.
- **Instances.** An instance is built from the schema's source and target with the
  substitution applied, and typechecked like any other generator. Asking for the same
  instance again reuses the existing one.
- **No retraction.** Nothing ever turns a diagram back into a placeholder. So instantiating
  never identifies two things that weren't already identified. This avoids the problem with
  the old workaround of an isomorphism `P ≅ X` for each `X`.

## Invertible and dualisable placeholders

A placeholder can be marked *invertible* or *dualisable* in its settings, like any generator
(see [duals](duals.md)). For a placeholder this is a requirement on what replaces it: a schema
that uses `P` as invertible (it attaches it reversed, or states that it is an isomorphism) is
only true for replacements that are. This is how to say that an object exists only when some
isomorphism or equivalence does.

- **Invertible**: the replacement is an invertible diagram, that is, every cell it is made of
  is invertible: a cell of an invertible generator, or the inverse of one. Identities and the
  homotopies between cells (interchangers and the like) are invertible, so a composite of
  invertible cells is.
- **Dualisable (level `k`)**: every cell of the replacement is dualisable to at least level `k`,
  or invertible. Composites of such cells are, as they are in the signature.
- **Directed**: anything.

A diagram's cells here are the occurrences of generators of the diagram's own dimension
(`Diagram::weakest_cell` in `homotopy-core`), and the replacement is as invertible as its weakest
cell. The boundary of the replacement does not count: it is the type of the placeholder, which
the replacement's cells go between. One more kind of cell counts: a cup or a cap of a generator
of one dimension less, which folds back on itself. It is the unit or counit of that generator,
and one that is `k + 1` times dualisable has them `k` times dualisable (invertible ones are
invertible), so the cup of a cell that is only dualisable does not replace an invertible
placeholder. (Folds of generators of lower dimension are not counted: those in the boundary
cannot be told from the new ones.) A placeholder of dimension 0 has no cells to be invertible.

It is checked everywhere a replacement is taken:

- the kernel (`Signature::instantiate`) refuses with "`P` must be invertible, but `X` is not
  (`f` is directed)", naming the placeholder and the weakest cell, and makes nothing;
- the Instantiate panel marks the replacement in red as soon as it is chosen, with the same
  message. A replacement chosen for another placeholder can be what requires it ("`η̇` needs `F`
  to be `f`, but `F` must be invertible (`f` is directed)": `η̇ : F ⇒ G` replaced by `f ⇒ g`
  makes `F` be `f`). Matching reports it the same way, and what is inferred never binds such a
  replacement: when the types leave several ways to cut a diagram, those that give an
  invertible placeholder a directed cell are not among them, and if none is left the panel
  says nothing fits;
- the search for candidates (when attaching an instance, and for the placeholders that matching
  does not reach) leaves out cells that are not invertible enough, which is also less to search;
- an invertible placeholder cannot be made more invertible while an instance was made with a
  replacement that is not as invertible.

The instance of a schema has the invertibility of the schema, as before. That is right because
what makes a schema invertible is its cells, and the placeholders among them are invertible when
they are marked so, which their replacements now are.

## How substitution works, and its limits

Substitution rebuilds a diagram one height at a time. Each height must attach a single
generator, possibly reflected. It is replayed with the replacement attached in its place, and
everything to the right of a replaced cell moves along as the cell grows or shrinks.

A height that does several things at once, such as `f ◫ g` with two cells side by side, is:

- **kept as it is**, if nothing in or around it is replaced, so the instance keeps the schema's
  layout where substitution doesn't care;
- **separated first**, if something in it is replaced. `f ◫ g` becomes `f ▷ _ ; _ ◁ g` (or the
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
fit there. These are tagged with the substitution, e.g. `A [Q ; R/P]`. Choosing one creates the
instance and attaches it in one step, which is undone in one step too.

An invertible schema is offered in both directions: as itself where the diagram has what it
starts from, and as its inverse (tagged `A [σ]⁻¹`) where the diagram has what it ends at.
(Instances you made by hand attach in both ways too: the inverse of `A[T/Ẋ]` attaches wherever
`A[T/Ẋ]` ends.)

The search tries replacements made from the cells of the diagram you click on, after reading
what it can off the diagram itself:

- **Instances in the diagram say what they were made with.** If the cells a schema attaches
  along include one that depends on placeholders (`η̇₍₂₎`, `ζ[η̇]`), the diagram has to have an
  instance of it, and the instance records its replacements (`(Tp)₍₂₎` is
  `η̇₍₂₎[T;T/Ḟ, T/Ġ, T◁p/η̇]`). These replacements are read off it, matched like the types of
  replacements in the Instantiate panel, and are not searched for: they need not be made of
  cells of the diagram, or of three cells (`p` is a 5-cell, the diagram may be a 4-diagram).
  A schema whose cell has no instance in the diagram is done at once: there is nothing to find.
  The placeholders that are left are searched for as below, with their types known.
- for a 0-cell placeholder, its 0-cells;
- for a placeholder of dimension `k`, composites of up to three of its `k`-cells (and the
  inverses of invertible ones), including identities, with the right source and target; for
  one of a higher dimension than the diagram, the cells that instances in the diagram were made with.

Placeholders are solved from the lowest dimension up, so a placeholder's type is known when it
is solved. If a schema depends on another generator that mentions a placeholder, the instance
of that generator must already exist for the schema to match: it occurs in the diagram you
click on, after all. The search is bounded, so a very large diagram may not offer every
possibility. Instantiating by hand from the signature still works in that case.

## Filling in replacements in the Instantiate panel

When you choose a replacement, the panel looks for the other placeholders that it determines.
Each replacement chosen has a type in terms of other placeholders (the type of `α` is `X ∘ ! ⇒ D`),
so replacing `α` with `α₀` is only well typed for certain replacements of `X` and `D`. These are
found by matching the types, not by trying replacements:

- a type that is a placeholder is matched with the diagram chosen (`η : F ⇒ G` replaced by
  `p : T ⇒ U` makes `F` be `T` and `G` be `U`), and then the type of that placeholder is matched
  in turn with the boundary of what it was matched with, and so on down, until nothing new is
  bound. A long chain of placeholders, each the type of the next, is filled in from the top;
- otherwise the boundaries are matched, down the dimensions (`h : μ ; η' ⇛ η ; ν`, replaced by
  a cell from `p ; z` to `l ; l`, matches the boundaries of `μ ; η'` with those of `p ; z`,
  so `F` and `G'` are the ends of `p ; z` and `l ; l`);
- a composite of whole cells, like `μ ; η'`, is matched with the diagram cut at some heights. If
  there is only one way to cut it the pieces replace `μ` and `η'`. Otherwise each way is tried:
  a diagram `p ; z` can be `μ ; η'` in three ways (`μ` or `η'` may be an identity);
- a cell that depends on placeholders (the limit `L` of a diagram `D`), matched with an
  instance of it (`L[D₀]`), binds what the instance was made with (`D := D₀`). The inverse of
  an invertible cell is matched like the cell. If the cell is itself an instance made with
  placeholders (`(Ẋη̇)₍₂₎`, see below), what it was made with is matched with what the instance of
  the same schema was made with (`Ḟ ; Ẋ` with `T ; T`, and so on).

A placeholder is filled in if every way that makes the chosen replacements well typed agrees on it.
The matching is bounded, and when it has to stop early only what it has forced so far is filled
in, and the panel says so. Placeholders that the types of the replacements chosen do not mention
are left empty. The search is redone each time a replacement is chosen or cleared, so inferred
replacements never go out of date; what was inferred is only given up when it no longer follows,
and the panel says why if nothing fits.

**Ambiguity.** When the types leave several ways to fill in the rest, nothing of them is filled
in, and the panel lists the ways as *candidates*, each with a *Use* button, the ones without
identities among the cells that the diagram is cut into first. Using one fills in its
placeholders, marked *inferred* like the others: they can be overridden or cleared, and what
follows from them is filled in. Nothing is ever picked for you: a square may well have an
identity in it. Under the list the panel says where the choice is: "`μ ; η'` can be cut 3 ways
to make `p ; ζ[p]`; choose `μ` or `η'` to decide". Choosing one of those placeholders yourself
decides the cut, and then the others follow.

**Checking as you go.** A replacement is checked when it is chosen, against the replacements
chosen before it, on the types alone and without searching: its dimension first, and then its
boundaries against what the others require. If it does not fit, it is kept, in red, with what is
wrong and what it clashes with ("`Ḟ` conflicts with `η̇`: `η̇` needs `Ḟ` to be `T`, not `Ġ`"; "`η̇`
conflicts with `ḣ`: `ḣ := k`: its target is `l ; l`, but the type requires `p ; ν̇` (`η̇ ; ν̇` with
`η̇ := p`) there"). A message names the replacement and says where in it the trouble is (its
source, its target, the target of its source, ...) and what the type requires there, with the
replacements chosen so far put in ("`ḣ := w`: its target is `D₀ ; t`, but the type of `ḣ` needs
`D[a ; t]` (`(Ẋη̇)₍₂₎` with `Ẋ := t`, `η̇ := a`)"). Composites in a message are written like those
in the names of diagrams (`f ; g`, see `doc/labels.md`), each as one formula whatever it is made
of, with a part that is a composite itself in parentheses. It is kept because it
is what you asked for, so you can see what is wrong with it and fix it or what it clashes with;
it takes no part in the inference, and *Create instance* waits until it is fixed or cleared. Of
two replacements that clash, the one chosen first counts.

**It can be cancelled.** What follows from a replacement is worked out as a job (see `CLAUDE.md`):
a step at a time, behind the busy indicator if it takes long, and `Esc` or a click on the
indicator cancels it. The replacements chosen are kept and nothing is inferred; the panel says
so and offers to infer again.

The placeholders a cell can be instantiated at include those that the types of the placeholders it
depends on mention, so a cell that depends on `α` is also instantiated at `X` and `D`.

## Instances that still depend on placeholders

An instance can be made with replacements that are themselves made of placeholders, such as
`(Ẋη̇)₍₂₎ = η̇₍₂₎[Ḟ ; Ẋ/Ḟ, Ġ ; Ẋ/Ġ, η̇ ▷ Ẋ/η̇]`: the cell `η̇₍₂₎` at the images of a cell `η̇` under
a functor `Ẋ`. Its diagram mentions none of those placeholders, but it is still a different cell
for every replacement of `Ẋ` and `η̇`, so it depends on the placeholders that the replacements it
was made with mention (its [`parameters`](../homotopy-model/src/proof/instances.rs)). Replacing
them gives the instance of the schema made with the replacements substituted: `(Ẋη̇)₍₂₎` at
`Ẋ := T`, `η̇ := p` is `η̇₍₂₎[T ; T/Ḟ, T/Ġ, p ▷ T/η̇]`, which is `(Tp)₍₂₎` if that exists. Without
this a schema whose placeholder has `(Ẋη̇)₍₂₎` in its type, like `ḣ : Ẋ(η̇₍₂₎) ≅ (Ẋη̇)₍₂₎`, would
treat it as a fixed cell that no replacement could have in its boundary.

The types of the placeholders are substituted in the same way: the generators in `ḣ`'s type
(not only in the diagram of the schema) that depend on placeholders are instantiated along with
them, and the replacement for `ḣ` must have exactly their instances in its boundary.

## Cells that depend on placeholders they don't mention

A generator is a schema in the placeholders its diagram mentions. Universal properties also
need cells that depend on a placeholder without mentioning it. For example, the mediating map
`u_α : X → L` of a limit depends on the cone `α`, but its type does not mention `α`. Without
that dependency there would be one `u` shared by every cone. In logical terms, an existential
under a universal needs a "Skolem function" `u(α)`.

For this, a generator's settings have a **Depends on** list: tick the placeholders it depends
on. Instances for different replacements of those placeholders are then different generators.
The dependency is inherited: any generator whose diagram mentions `u` also depends on `α`, so
instantiating it at a cone also instantiates `u` at that cone.

### Example: limits as right Kan extensions

Work one level up, with categories as 0-cells, functors as 1-cells and natural transformations
as 2-cells. Write `1` for the terminal category and `! : J → 1`. Then everything is globular:

| generator | type | |
|---|---|---|
| `D` | `J → C` | placeholder: the diagram |
| `L` | `1 → C` | depends on `D`: its limit |
| `π` | `! ; L ⇒ D` | the limit cone |
| `X` | `1 → C` | placeholder: an apex |
| `α` | `! ; X ⇒ D` | placeholder: a cone |
| `u` | `X ⇒ L` | depends on `α`: the mediating map |
| `β` | `α ⇛ (! ◁ u) ; π` | the factorisation |

Uniqueness is one more schema with placeholders `v : X ⇒ L` and `γ : α ⇛ (! ◁ v) ; π`: an
invertible cell `v ⇛ u` (compatible with `γ` and `β` if you want it strict).

To say "`C` has all limits of shape `J`", state these as axioms. To say that one particular
`L` is a limit of one particular `D`, make `D` an ordinary generator instead of a placeholder.

To use the axioms, click a cone `α₀ : ! ; X₀ ⇒ D₀` in your diagram. The attach menu offers
`β [D₀/D, X₀/X, α₀/α]`. Choosing it creates `L[D₀]`, `π[D₀]` and the mediating map `u[…α₀…]`,
and factors the cone through the limit. `L[D₀]` is shared by all cones over `D₀`, while each
cone gets its own mediating map. This example is a test in
`homotopy-model/src/proof/instances.rs`.

The same pattern gives internal homs of a closed (not necessarily compact) monoidal category:
a placeholder `f : A ⊗ X → B`, a currying `λf : X → [A, B]` that depends on `f`, and
`f ⇛ ev ∘ (A ⊗ λf)`. For compact closed categories, use duals instead (see
`doc/duals.md`).

## Deleting generators

Deleting a generator deletes every generator that depends on it, and those in turn
(`Signature::removal_nodes`, which is the one definition of this; the preview in the signature
panel is made from it). A generator goes with another if

- its diagram mentions it (or one of its instances),
- it is an instance made with a substitution that mentions it, or
- it lists it under **Depends on** and it is a **placeholder**. The list says "there always exists
  a cell for each replacement of this placeholder" (the mediating map `u_α` of a limit), which is
  no longer so when the placeholder is gone.

Deleting a schema deletes its instances, as before. Deleting an **instance** deletes what
mentions it, but not what lists it under *Depends on*: the list is made of placeholders (the
settings only offer those, and for anything else the entry is not looked at), and an instance
that is deleted is just a replacement that was never needed, whereas the generator that
depends on it still has all the replacements it had. (This is tied to the "there always exists"
meaning. *Templates*, which are not made yet, would differ: an instance of a template is a cell
the dependent generator promises, so deleting one would delete its dependents too.) Entries of
the list that name generators that are gone are dropped, so that a new generator that is given
the same number does not inherit them. The workspace, the selected boundary and the stash lose
what mentions anything that goes. Undoing brings everything back.

To see what a deletion would delete before doing it, open the settings of a generator or folder
and hover over (or focus) its delete button: the rows that would go are tinted and struck out,
a closed folder, or a schema whose instances are closed, that holds some says how many
("−2 inside"), and a line under the row says "Deletes 4 more".

## Not yet

- Inferring replacements longer than three cells, or replacements whose cells are contracted
  into one height (`f ◫ g`) in the diagram you click on.
